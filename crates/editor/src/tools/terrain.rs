//! The height brush: raise, lower, flatten, smooth.
//!
//! ## The edit loop, in five steps
//!
//! The tiles are opened by [`super::open_tiles`], which is the directory's and
//! not this tool's. What is left is:
//!
//! 1. [`stroke`] runs the brush while the left button is down, applies the edits
//!    to the open tile, and folds them into one entry on the history.
//! 2. [`live_ground`] writes the changed chunks' vertices straight into the
//!    meshes already on the GPU, in the frame the mouse moved.
//! 3. [`live_foliage`] puts the grass back on those chunks once the button comes
//!    up, which is the one thing standing on the ground that a vertex patch
//!    cannot carry with it.
//! 4. [`publish`] writes the edited tile's bytes into the overlay when the
//!    button is released, which is what makes the archives answer with them.
//! 5. [`remesh`] forgets a tile the live path could not reach, so the streamer
//!    reads it again.
//!
//! Every one of them stands down for a playtest — see [`crate::playtest`].
//!
//! ## The ground moves while the button is held
//!
//! Rebuilding a tile is the client's own streamer reading the file, meshing 256
//! chunks and decoding eleven tilesets: a third of a second in a debug build,
//! which cannot happen sixty times a second. So the ground is not rebuilt at all
//! while a stroke is held. [`live_ground`] rewrites the **positions and normals
//! of the chunks the stroke touched**, in place, in the buffers the draw groups
//! already hold — four chunks at 320 vertices each rather than a tile at 81,840.
//!
//! Three things make that possible and all three are stated where they are
//! decided rather than here: `vale_edit::adt::mesh` for where a chunk's
//! vertices sit in a tile's buffer,
//! `vale_client::render::terrain::GroundSources` for where each of a draw
//! group's vertices came from, and
//! `vale_client::render::terrain::LiveEdits` for the two costs of keeping
//! either.
//!
//! [`remesh`] is the fallback and not the normal path. A tile whose draw groups
//! carry no sources — the client's own build, or a tile that arrived before the
//! switch was set — is read again instead.
//!
//! ## …and what a patched vertex does not carry with it
//!
//! A live patch moves the ground's own positions and normals and nothing else,
//! so anything the tile *built from* those heights is left where it was. The
//! visible one is the foliage: the tufts are a merged mesh, standing at the
//! heights the chunk had when the mesh was made, and an undone raise left a
//! chunk's grass hanging in the air where the hill had been. [`live_foliage`] is
//! the answer for that population and it is the only one that needed a live
//! answer, because it is the only one the terrain's own heights place.
//!
//! Three others read the heights and are deliberately left to a re-read:
//!
//! * an `MDDF` doodad stands at the **absolute** height in the file, which is
//!   what the reference does — raising the ground under a tree buries it, here
//!   and in any other editor, until the tree is moved;
//! * `recompute_normals` stops at the tile's edge, so a stroke across a border
//!   leaves a shading seam.
//!
//! A third used to be listed here and is believed to be gone: the draw group's
//! `Aabb`, baked when the mesh was built, so ground raised far above its
//! original range could be frustum-culled early. Bevy 0.19's `calculate_bounds`
//! carries a second query — `Or<(AssetChanged<Mesh3d>, Changed<Mesh3d>)>` —
//! that recomputes an existing `Aabb` when the mesh asset is mutated, and
//! [`live_ground`] mutates it through `Assets::get_mut`, which is what raises
//! `Modified`. **That is read off Bevy's source and not measured here**, which
//! is the difference between the two halves of any rendering claim; it is
//! written down because it is one of the candidates for a reported artefact
//! and it is the one this weakens.
//!
//! All three are fixed by the tile being read again, which is what
//! [`crate::playtest`] does on its way into a login: what a playtest walks into
//! is built from the bytes rather than patched into place.

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::Tool;
use crate::tools::{Wheel, CORE};
use vale_client::render::foliage::TileFoliagePlans;
use vale_client::render::terrain::{GroundSources, LoadedTiles, TerrainTile};
use vale_edit::ops::Edit;
use vale_edit::ops::{Brush, Falloff, Mode, Shape};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

/// How large the height brush may be, in yards.
///
/// **The upper bound is the one thing here that is not arbitrary.** A stroke can
/// only reach tiles this session has *open*, which is [`crate::OPEN_BLOCK`] yards
/// across, so a brush wider than that paints a circle with pieces missing
/// rather than a bigger circle. Half of it is the honest cap: a brush centred
/// anywhere in the middle tile still lands entirely on open ground.
///
/// It was 200, which was a guess, and a guess in the direction of refusing work
/// the tool can do; then 800, which was half the 3x3 the editor used to stream.
/// **It is derived from [`crate::REACH`] now** rather than written down, so the
/// two cannot drift — and a run that narrows the block with `--reach` keeps this
/// cap, which is the harmless direction: a brush past the open ground paints the
/// part of its circle that is open and nothing else.
pub const RADIUS: std::ops::RangeInclusive<f32> = 0.5..=(crate::OPEN_BLOCK / 2.0);

/// …and how hard, in yards a second for raise and lower.
///
/// **No upper bound worth the name.** A height is an `f32` in the file and a
/// stroke moves it by `strength * weight * seconds`, so the only thing a large
/// number does is move the ground a long way in one frame — which is what
/// somebody asking for a large number wants. The old ceiling of 60 was a guess.
pub const STRENGTH: std::ops::RangeInclusive<f32> = 0.0..=100_000.0;

/// …and for the two modes that converge rather than climb.
///
/// A fraction of the remaining distance a second, and `Brush::moved` clamps one
/// step to 1.0 of it — so anything past about 60 at sixty frames a second is
/// already "all of it in one frame" and the range is wide only so that nothing
/// is refused.
pub const RATE: std::ops::RangeInclusive<f32> = 0.0..=1_000.0;

/// The terrain tool's settings.
#[derive(Resource, Debug, Clone)]
pub struct Terrain {
    pub brush: Brush,
    /// What [`Mode::Flatten`] flattens toward when the stroke begins: the height
    /// under the pointer at that moment, so a flatten is "make it like here".
    pub flatten_to_cursor: bool,
}

impl Default for Terrain {
    fn default() -> Terrain {
        Terrain {
            brush: Brush::default(),
            flatten_to_cursor: true,
        }
    }
}

/// Whether a stroke is being held.
///
/// It does not name a tile. A brush is a circle in the world and a tile is a
/// 533-yard square, so a stroke near a border edits both sides of it; which
/// tiles those are is asked again every frame, because the pointer moves.
#[derive(Resource, Default)]
struct Held(bool);

pub struct TerrainToolPlugin;

impl Plugin for TerrainToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Terrain>()
            .init_resource::<Held>()
            .add_systems(
                Update,
                (
                    // **After the pick**, which is what a stroke is aimed by:
                    // running first would paint at the position the pointer had
                    // on the previous frame, which while the camera is moving is
                    // a stroke that trails the cursor.
                    (
                        stroke,
                        super::shortcuts,
                        live_ground,
                        live_foliage,
                        publish,
                        remesh,
                    )
                        .chain()
                        .after(crate::pick::aim),
                    draw_brush,
                ),
            )
            // **In `PostUpdate`, and before visibility is propagated.** The
            // streamer spawns a replacement tile in `Update` and it becomes
            // queryable at the next sync point, so a system in `Update` sees it
            // for the first time a frame *after* it was spawned — one frame with
            // two copies of the same ground in the same place, which speckles.
            // Here it is hidden on the frame it appears, and the `before` is
            // what makes that hiding take effect in the same frame rather than
            // the next.
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
    windows: Query<&Window>,
    state: Res<crate::playtest::Playtest>,
    time: Res<Time>,
) {
    // **The left button is the game's during a playtest**, and so is the wheel:
    // one is 1.12's own steer and the other is its zoom.
    if !state.editing() {
        return;
    }
    // …and a press that landed on a panel is the panel's. See
    // [`crate::ui::over_the_world`], which is where the two answers egui gives
    // and why neither works are written down.
    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);
    // **Three numbers on the wheel, by modifier** — see [`Wheel`], which is the
    // one list and is what `camera::fly` reads to decline the same notch.
    if in_world && scroll.delta.y != 0.0 {
        let step = 1.0 + scroll.delta.y * 0.1;
        match Wheel::held(&keys) {
            Some(Wheel::Radius) => {
                terrain.brush.radius =
                    (terrain.brush.radius * step).clamp(*RADIUS.start(), *RADIUS.end());
            }
            Some(Wheel::Strength) => {
                // **Proportional, off a floor.** The ranges span four decades,
                // so a fixed step is either useless at the top or unusable at
                // the bottom; the floor is what lets it climb away from zero,
                // which multiplying alone cannot.
                let range = match terrain.brush.mode {
                    Mode::Raise | Mode::Lower => STRENGTH,
                    _ => RATE,
                };
                terrain.brush.strength =
                    (terrain.brush.strength.max(0.01) * step).clamp(*range.start(), *range.end());
            }
            // A fraction, so this one is a fixed step: a twentieth of the radius
            // a notch takes it end to end in a turn of the wheel.
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
        session.history.end();
        held.0 = false;
        return;
    }
    if *tool != Tool::Terrain {
        return;
    }
    // **The gate is on the press and not on every frame** — a stroke belongs to
    // wherever it started, which is the rule every paint program follows. Asked
    // per frame instead, a stroke that wandered over a panel stopped dead in the
    // middle and started again on the way back.
    if buttons.just_pressed(MouseButton::Left) && !in_world {
        return;
    }
    // The ground under the pointer, which is also what says the pointer is over
    // a tile this session has open at all.
    let Some(at) = cursor.ground else { return };

    if buttons.just_pressed(MouseButton::Left) {
        // What a flatten flattens toward is decided once, when the button goes
        // down: taking it from the pointer every frame would flatten toward
        // whatever the brush had just made, which is a stroke that never
        // converges.
        if terrain.flatten_to_cursor {
            if let Mode::Flatten { .. } = terrain.brush.mode {
                terrain.brush.mode = Mode::Flatten { to: at.z };
            }
        }
        let label = match terrain.brush.mode {
            Mode::Raise => "Raise terrain",
            Mode::Lower => "Lower terrain",
            Mode::Flatten { .. } => "Flatten terrain",
            Mode::Smooth => "Smooth terrain",
            Mode::Noise => "Roughen terrain",
        };
        session.history.begin(label);
        held.0 = true;
    }
    if !buttons.pressed(MouseButton::Left) || !held.0 {
        return;
    }

    // **Every tile the circle reaches, not the one the stroke began on.** A
    // brush that stopped at the tile border left a step in the ground exactly
    // there: one side raised and the other not, with nothing about either file
    // wrong. The four corners of the brush's own bounding box name the tiles,
    // which is at most four and usually one.
    let brush = terrain.brush;
    let seconds = time.delta_secs();
    // **Where a smooth is pulling everything, decided once across every tile the
    // brush reaches**, before any of them move. Taken per tile — or, as it was,
    // per chunk — each piece converges on its own height and the boundary
    // between two pieces becomes a step. See `Brush::weighed`.
    //
    // In its own pass because the tiles are borrowed immutably to read it and
    // mutably to stroke it, and because a mean taken while the ground is already
    // moving is a mean of two different grounds.
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
            // Not open yet. `open_tiles` is working through the 3x3 a tile a
            // frame and will have it shortly; skipping is a few frames of the
            // stroke not reaching that side, where opening it here would be a
            // two-megabyte parse in the middle of a drag.
            continue;
        };
        let edits = brush.stroke(tile, [at.x, at.y], seconds, level);
        if edits.is_empty() {
            continue;
        }
        let chunks: Vec<usize> = edits.iter().filter_map(Edit::chunk).collect();
        session.history.record(&key, edits);
        for chunk in chunks {
            session.touched(coord, chunk);
        }
    }
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
        // The new vertices, as one run per changed chunk. **A run and not a
        // map**: this is compared against every vertex of every draw group of
        // the tile, which is up to 81,840 of them on every frame of a stroke,
        // and a stroke touches at most four chunks. Four integer comparisons
        // beat a hash lookup by enough to matter at that count.
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
/// **Once the stroke is over, not every frame of it.** A tuft's mesh is merged
/// from the heights it was built with, so there is nothing in it to patch the
/// way [`live_ground`] patches the terrain's vertices: the chunk's whole lawn
/// has to be built again. That is a few thousand vertices per chunk per
/// material, which is affordable once a stroke and not sixty times a second —
/// and doing it during the stroke would also mean throwing away the mesh being
/// looked at on every frame, which is a flicker rather than an animation.
///
/// What it fixes was reported as *"the foliage floats where the old ground
/// was"*, and the undo case is the one that made it obvious: an undone raise
/// puts the ground back and left the grass standing in the air at the height the
/// raise had reached. Everything that reads the ground **outside** the tile's
/// own mesh has the same problem — see [`remesh`] for the ones this does not
/// cover.
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
            // The heights and the **recomputed** normals — see
            // `vale_edit::adt::heights::recompute_normals`, which the stroke
            // has already run. A lawn regrounded onto the old shading is lit as
            // the hill was before it was raised.
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
    // **And the shading, which is the third attribute a vertex has.** It rides
    // this path rather than one of its own because `MCCV` is one length or
    // absent — see `vale_edit::adt::colours` — so a colour stroke moves a
    // vertex exactly as a height stroke does and the mesh keeps its shape, its
    // indices and its length. Absent in a client build, where the attribute is
    // never uploaded; present in every editor one, because `LiveEdits` asks for
    // it whether or not the tile has any yet.
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
/// **Every tile in the bounding box's range, not only the four corners.** The
/// corners are enough while a brush is smaller than a tile — which it was, when
/// the ceiling was 200 yards against a tile's 533 — and stop being enough the
/// moment it is not: a 600-yard brush spans three tiles on an axis and its four
/// corners name none of the middle ones, so it would paint a ring and leave a
/// cross of untouched ground through it.
///
/// The box rather than the circle because a corner of the box outside the circle
/// is at worst one extra tile whose vertices all weigh zero.
fn tiles_under(brush: &Brush, at: Vec3) -> Vec<(u32, u32)> {
    tiles_in_range(brush.radius, at)
}

/// The same, for any radius — shared with the texture brush, which asks the same
/// question about the same circle.
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

/// The tile on screen that a fresh read is going to replace, and **which other
/// tiles it has to change places with**.
///
/// It stays drawn and stays exactly as it was; [`swap`] takes it away on the
/// frame every replacement in its batch is whole. See [`remesh`] for why a tile
/// is replaced at all and [`swap`] for why they go in batches.
#[derive(Component)]
pub struct Replacing(pub u64);

/// Hand a stale tile back to the streamer, keeping the old one on screen.
///
/// ## Why not simply despawn it
///
/// That is what this did, and the gap was reported as *"the entire screen
/// flashes and the terrain re-renders, momentarily disappearing"*. It is not a
/// flash of anything: it is a **third of a second with no tile there**, because
/// despawning is instant and reading a tile again is a parse, a mesh of 256
/// chunks and eleven texture decodes. At editing distance one tile is most of
/// the screen.
///
/// The other obvious arrangement is worse. Leaving the old one and letting the
/// new one spawn over it puts two copies of identical ground in the same place,
/// which z-fights along every triangle for as long as it lasts.
///
/// So: the old tile is **marked and left alone**, the new one is spawned
/// **hidden**, and the two change places in one frame. `LoadedTiles::arriving`
/// is what says when the new one is whole — the one thing a host cannot work out
/// from outside, because a tile's atlas, textures and draw groups are spread
/// over several frames by the upload budget.
///
/// A tile whose replacement never arrives — an ADT that will not parse — keeps
/// the old one for ever, which is the right way for this to fail.
fn remesh(
    mut session: Option<ResMut<EditSession>>,
    mut loaded: ResMut<LoadedTiles>,
    tiles: Query<(Entity, &TerrainTile, Option<&Replacing>)>,
    mut commands: Commands,
    // **One batch per pass, not per tile** — see [`swap`]. Everything made stale
    // together is replaced together, which for the one thing that makes two
    // tiles stale at once is the whole point.
    mut batch: Local<u64>,
    // **The revision each in-flight replacement was started at** — see the loop
    // below, and `crate::session::EditSession::revision`.
    mut started: Local<bevy::platform::collections::HashMap<(u32, u32), u64>>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };

    // **A replacement reads the file once, when it is started.** Anything
    // published between then and the frame it lands is a change that read cannot
    // have seen, and nothing downstream corrects it: the live sets — the
    // ground's vertices, a placement's transform, a chunk's blend map — were
    // drained against the *outgoing* copy, which is despawned the moment the
    // incoming one appears. So the tile on screen goes back to whatever the
    // older bytes said and stays there until something else makes it stale.
    //
    // It was reported as a building that stayed where it had been dragged after
    // an undo, with the file already correct. A drag ends by asking for the tile
    // again, that read takes a third of a second — seconds on a city tile — and
    // `Ctrl+Z` inside that window moves the outgoing copy back and then has it
    // replaced by one built before the undo.
    //
    // Asking for another read is the answer that converges, and it is the same
    // answer the one-at-a-time guard below already gives: the file is right, so
    // reading it again is right.
    started.retain(|coord, _| {
        tiles
            .iter()
            .any(|(_, tile, going)| tile.coord == *coord && going.is_some())
    });
    let mut behind: Vec<(u32, u32)> = Vec::new();
    for (coord, had) in started.iter() {
        if *had != session.revision(*coord) {
            behind.push(*coord);
        }
    }
    for coord in behind {
        session.stale.insert(coord);
    }
    // **While the button is still down, and that is new.** It used to wait for
    // the release, because a rebuild was a third of a second with a hole in the
    // world and doing that mid-stroke was unthinkable. Now that a replacement
    // keeps the old tile on screen until the new one is whole, a rebuild costs
    // a task on the compute pool and nothing on screen at all — so there is no
    // reason to make somebody let go of the mouse to see what they painted.
    //
    // It was reported as exactly that: *"when you are painting and hit another
    // tile, the new paint does not show until you let go of the cursor."*
    //
    // What keeps it from queueing a rebuild a frame is the one-at-a-time guard
    // below: a tile that is already being replaced stays stale and is picked up
    // when the one in flight lands, by which point the file holds everything
    // painted since.
    if session.stale.is_empty() {
        return;
    }
    let stale: Vec<(u32, u32)> = session.stale.drain().collect();
    *batch += 1;
    for coord in stale {
        // **One replacement at a time per tile.** A second `reload` while the
        // first is still arriving would leave two outgoing copies and no way to
        // say which the incoming one replaces. It stays stale and is picked up
        // on a later frame, by which point the file holds everything the stroke
        // wrote anyway.
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
        // What the read about to start will see. Taken here rather than when it
        // lands, so the error is on the side of one read too many.
        started.insert(coord, session.revision(coord));
        // **Without the despawn this time**, which is the half `LoadedTiles`'
        // own comment warns about: forgetting a tile without despawning it
        // draws the tile twice. [`swap`] is what pairs with it, one frame later
        // than it used to be.
        loaded.reload(coord);
    }
}

/// Put the replacements on screen and take the old tiles off, in one frame.
///
/// ## Tiles made stale together change over together
///
/// One tile at a time is right for a texture edit, which is about one tile. It is
/// **wrong for anything that moves a thing between two of them**: a placement
/// dragged past a tile border is taken out of one file and put into the other, so
/// both are read again, and the two reads finish whenever they finish. If each
/// swapped on its own the moment it was ready, then between the two moments the
/// tile that gained the placement is drawing it while the tile that lost it is
/// still drawing the old copy — and the thing is **on screen twice**.
///
/// That is not a theoretical window. It was reported of Stormwind, where it is
/// seconds: a city tile is 311 groups and 844,627 vertices, so the read that
/// removes it takes far longer than the read that adds it.
///
/// So the batch is the unit. Everything [`remesh`] made stale in one pass carries
/// the same number, and none of them changes over until all of them are whole.
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
        // **Whole means every group, not merely spawned**: a tile's root exists
        // from its first instalment and its ground arrives over the frames after
        // it, so swapping on the root's appearance would show a tile with two of
        // its thirty draw groups on it.
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
/// Against the **edited** heights rather than the drawn mesh, for the reason
/// [`crate::pick`] marches its ray against them: while a stroke is held those
/// two disagree by the whole of what has been painted, and the ring is the only
/// thing on screen that is up to date.
fn draw_brush(
    mut gizmos: Gizmos,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    terrain: Res<Terrain>,
    cursor: Res<Cursor>,
) {
    if *tool != Tool::Terrain {
        return;
    }
    // `cursor.ground` is `None` for the whole of a playtest — see
    // [`crate::pick::aim`] — so there is no second check for it here.
    let (Some(session), Some(at)) = (session, cursor.ground) else {
        return;
    };
    let colour = match terrain.brush.mode {
        Mode::Raise => Color::srgb(0.4, 0.9, 0.4),
        Mode::Lower => Color::srgb(0.9, 0.5, 0.3),
        Mode::Flatten { .. } => Color::srgb(0.4, 0.7, 1.0),
        Mode::Smooth => Color::srgb(0.9, 0.9, 0.4),
        Mode::Noise => Color::srgb(0.8, 0.5, 0.9),
    };
    // Two rings: the radius, and the half-radius where a smooth falloff still
    // has most of its strength. Both follow the ground rather than lying flat,
    // which on a slope is the difference between a circle and an ellipse in the
    // wrong place.
    rings(
        &mut gizmos,
        &session,
        at,
        terrain.brush.radius,
        terrain.brush.core,
        terrain.brush.shape,
        colour,
    );
}

/// The two rings every brush draws: the radius, and the core inside it.
///
/// **The inner ring is the core, and there is no inner ring without one.** It
/// used to sit at half the radius and mark nothing — a preview line at a place
/// the brush does nothing in particular is a line a person reads a meaning into
/// that is not there, and this one was read as *where the pressure is focused*,
/// which is what the core now actually is.
///
/// **One function because the three brushes that take a core must not be able
/// to disagree about whether they draw it.** Each of them used to spell this
/// out for itself and the shading brush did not spell it out at all: its
/// `Alt`+wheel set a core the preview never showed, so the one control on the
/// tool with no feedback was the one that decides where the paint lands. That
/// is the report. Two copies of a rule is how the third comes to be missing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rings(
    gizmos: &mut Gizmos,
    session: &EditSession,
    at: Vec3,
    radius: f32,
    core: f32,
    shape: Shape,
    colour: Color,
) {
    let inner = (core > 0.0).then(|| radius * core);
    for (radius, shade) in [Some(radius), inner]
        .into_iter()
        .zip([1.0, 0.5])
        .filter_map(|(radius, shade)| Some((radius?, shade)))
    {
        ring(gizmos, session, at, radius, shape, colour.with_alpha(shade));
    }
}

/// One ring of 48 segments, each vertex dropped onto the edited ground.
///
/// Shared with the texture brush, which wants the same ring for the same reason
/// and must not grow a second copy of it: two brushes that disagreed about where
/// the pointer is would be two brushes, one of which is wrong.
///
/// **A vertex with no ground under it breaks the ring rather than borrowing a
/// height.** There are three ways to have none — a hole in the terrain, a tile
/// that is not open, and the ground beyond the edge of the map — and the first
/// draft substituted the height under the pointer for all three. During a raise
/// that height is the top of the hill being built, so the ring grew a spike to
/// it wherever it crossed a hole: an artefact that reads exactly like torn
/// ground, on ground that is not torn. A gap in the ring says what is true,
/// which is that there is nothing there to draw on.
pub(crate) fn ring(
    gizmos: &mut Gizmos,
    session: &EditSession,
    at: Vec3,
    radius: f32,
    shape: Shape,
    colour: Color,
) {
    const SEGMENTS: usize = 64;
    /// Yards above the ground, so the line is not buried in the mesh it is
    /// drawn over.
    const LIFT: f32 = 0.15;

    // **The shape's own outline and not a circle**, and taken from the shape
    // rather than drawn here: a preview built from a second reading of the
    // footprint is a preview that says the brush reaches somewhere it does not,
    // and the two would drift the first time a norm was corrected. See
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
    for i in 0..points.len() {
        if let (Some(from), Some(to)) = (points[i], points[(i + 1) % points.len()]) {
            gizmos.line(from, to, colour);
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

/// …and the five falloffs.
///
/// **In order of how much of the radius they keep**, which is what a person is
/// choosing between: `Dome` holds nearly all of it, `Sharp` gives it up at once,
/// and the three in the middle are the ones that were here.
pub const FALLOFFS: [(&str, Falloff); 5] = [
    ("Smooth", Falloff::Smooth),
    ("Linear", Falloff::Linear),
    ("Flat", Falloff::Flat),
    ("Sharp", Falloff::Sharp),
    ("Dome", Falloff::Dome),
];

/// …and the three footprints. See `vale_edit::ops::Shape`, where a shape
/// being a *distance* rather than a mask is argued — it is what lets these three
/// and the five above be two lists rather than fifteen.
pub const SHAPES: [(&str, Shape); 3] = [
    ("Circle", Shape::Circle),
    ("Square", Shape::Square),
    ("Diamond", Shape::Diamond),
];
