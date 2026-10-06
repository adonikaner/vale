//! The ground foliage: the grass, flowers and small rocks the terrain plants,
//! as opposed to the doodads the `MDDF` places.
//!
//! The planting rule is [`vale_assets::world::foliage`] and the table under it
//! is [`vale_assets::tables::foliage`]. This module holds only the part that
//! needs a renderer. Which cell grows what, and which parts follow the 1.12.1
//! client and which are this project's own scatter, is documented in those two
//! modules.
//!
//! ## Why foliage is not drawn by the doodad pass
//!
//! At the default `frillDensity` a tile grows 14,768 tufts
//! (`vale foliage Azeroth 34 51`), against about 1,400 `MDDF` placements, and
//! the 3x3 around the character about 130,000. At the maximum `frillDensity`
//! of 256 it is up to sixteen times that. This is an order of magnitude past
//! the population [`crate::render::doodads`] is built for. Its per-frame render
//! cost scales with the number of spawned meshes, and spawning even the few
//! thousand tufts within sight would cost more in ECS, transform propagation
//! and visibility than the triangles do.
//!
//! So foliage is merged, not instanced. A chunk's tufts become one mesh per
//! material: they are transformed on the CPU and concatenated into one vertex
//! buffer, and the world holds one entity per chunk per material instead of
//! one per tuft. `vale foliage <Map> <x> <y>` reports the distinct textures
//! over the models a chunk plants, which is the number of meshes per chunk: 1
//! on `Azeroth_34_51` (Duskwood), up to 2 on `Azeroth_32_48` and up to 3 on
//! `Kalimdor_39_30`. A chunk of grass is one to three draws, and the ~20 chunks
//! in range are ~40, sharing a few materials that merge again under
//! multidraw.
//!
//! The merge is affordable because a detail doodad is small: all 411 models
//! that exist are a single batch of 4–8 triangles (`vale foliage`). A chunk
//! grows at most 128 tufts at the default `frillDensity` and density 8, which
//! is at most 1,024 triangles; 300 tufts are ~2,700 vertices, 70 KB of buffer,
//! built in tens of microseconds. A tree could not be merged this way.
//!
//! ## Geometry is built from a plan, on demand
//!
//! A merged mesh has to be rebuilt whenever its set of tufts changes, so the
//! plan ([`ChunkFoliage`], about 1.5 KB) is what is kept from the tile parse.
//! The geometry is built when a chunk comes within [`FOLIAGE_RANGE`] and
//! dropped when it leaves. A 3x3 of tiles holds at most about 3.5 MB of plans,
//! where its tufts merged would be about 30 MB.
//!
//! The scatter is a hash of the chunk's world position, so a chunk that is
//! built, dropped and built again comes back identical. A sequence would not
//! guarantee that, and grass that rearranged itself each time it came back
//! into range would look wrong.
//!
//! ## The range
//!
//! [`FOLIAGE_RANGE`] is 50 yards, the 1.12.1 client's cutoff for a small
//! doodad: `draw_range` in [`crate::render::doodads`] carries the client's
//! fade bands by bounding radius, and 50 yards is the end of the 0.5-yard
//! band. The biggest model here, `DskGra05` at 1.03, would take the 126-yard
//! band. Applying the 0.5-yard band to every model is this client's choice,
//! not a stated rule of the 1.12.1 client, and it is the cheaper direction.
//!
//! ## Two things the 1.12.1 client does not do
//!
//! Both are marked as deviations where they appear:
//!
//! * A tuft is lit by the ground's normal, not by its own. The detail models
//!   carry flat face normals (see [`FoliageGeometry`]), so lit with those each
//!   tuft's brightness would depend on its yaw. Each vertex takes the `MCNR`
//!   normal at its cell's centre instead.
//! * The grass sways. 5875 does not sway it; `wind.wgsl` states this and holds
//!   the maths. The cost is one `sin` per foliage vertex, and it is the reason
//!   this crate owns the M2 vertex stage.
//!
//! The sun scale is not a deviation. A tuft takes a doodad's per-instance sun
//! scale, as the 1.12.1 client gives one: `NEUTRAL` (1.0) on lit ground and
//! `SHADOWED_GROUND` (0.5) over the ground's `MCSH` shadow. With the ground's
//! normal and `NEUTRAL`, a lit tuft and the ground under it are lit by the
//! same calculation.
//!
//! ## Chunk granularity
//!
//! A chunk is built whole when its centre comes within `FOLIAGE_RANGE` plus its
//! own half-diagonal, so the near corner of a chunk can be drawn from 74 yards
//! where the far corner would have been dropped at 50. Foliage therefore
//! appears in 33-yard blocks rather than tuft by tuft. Splitting a chunk to
//! tighten that would multiply the draw calls by four, which is not worth it
//! for a change at the edge of visibility.

use crate::axes;
use crate::render::models::{instance_tag, sun_scale, Lookup, M2Material, ModelCache};
use crate::render::terrain::TerrainTile;
use vale_assets::tables::foliage::GroundEffects;
use vale_assets::world::adt::{Adt, CHUNK_SIZE};
use vale_assets::world::foliage::{
    ChunkFoliage, FoliageInstance, DEFAULT_FRILL_DENSITY, FRILL_DENSITY_RANGE,
};
use vale_assets::world::m2::M2;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use std::sync::Arc;

/// How far a tuft is drawn: the 1.12.1 client's small-prop band. The module
/// doc says which part of that is the client's rule and which is this
/// client's choice.
pub const FOLIAGE_RANGE: f32 = 50.0;

/// Half a chunk's diagonal: how much further out a chunk's centre has to be
/// watched, since the whole chunk is one mesh.
const CHUNK_RADIUS: f32 = CHUNK_SIZE * std::f32::consts::SQRT_2 / 2.0;

/// How far beyond that a chunk is kept once built. This is the hysteresis
/// band: without it a player moving back and forth on one spot rebuilds the
/// boundary ring twice a step. The same idea as [`crate::render::doodads`]'
/// `STREAM_MARGIN`, and larger, because a rebuild here is a whole chunk's
/// geometry rather than a spawn.
const KEEP_MARGIN: f32 = 20.0;

/// How far the player moves before the plans are re-walked. A walk is one
/// distance per plan over the 3x3, a couple of thousand, which is cheap but
/// not cheap enough to do every frame.
const STREAM_STEP: f32 = 8.0;

/// The direction the wind's wave travels across the ground, in Bevy's `xz`.
///
/// It must match `wind.wgsl`'s `WIND_DIRECTION`: this is the axis the baked
/// phase is measured along, and that is the axis the shader leans the tips
/// down. The two are separate because one is baked into a vertex buffer once
/// and the other is read every frame; a test checks that they agree.
pub const WIND_DIRECTION: Vec2 = Vec2::new(0.7071, 0.7071);

/// Radians of phase per yard along [`WIND_DIRECTION`], so the gust is a wave
/// about 20 yards long: a few tufts lean together, rather than the whole field
/// moving at once.
pub const WIND_WAVE: f32 = 0.3;

/// How many chunks are built in one frame.
///
/// A chunk is at most ~1,150 vertices of transform-and-concatenate at the
/// default `frillDensity` (about nine a tuft), tens of microseconds; two a
/// frame fills the ring in well under a second while
/// leaving the frame that a tile arrives on to the tile.
const BUILD_BUDGET: usize = 2;

/// Marker for every merged foliage mesh, so the tuning switch can find them,
/// carrying the mesh's triangle count.
///
/// The count is on the entity because it cannot be read from the asset: these
/// meshes are `RenderAssetUsages::RENDER_WORLD`-only like every other world
/// mesh in this crate, and `Mesh::indices` panics once such a mesh has been
/// extracted rather than answering `None`. A HUD line that read the count from
/// the mesh panicked when `F4` was pressed.
///
/// The count is recorded at the merge, which built the index buffer, so
/// nothing reads a mesh back.
#[derive(Component)]
pub struct Foliage {
    pub triangles: usize,
}

/// One foliage model's geometry, kept on the CPU so it can be merged.
///
/// The renderer's own mesh handles cannot be used: [`crate::render::models`]
/// builds its meshes `RENDER_WORLD`-only, so the vertices are gone from the
/// main world by the time anything could read them back. This is the same
/// geometry read a second time, which is cheap, because the expensive part of
/// a model is its texture, and that comes from [`ModelCache`] like every other
/// doodad's.
///
/// The batches are in [`crate::render::models::loader::batch_draw`]'s order,
/// produced by the same function, so `batches[i]` is the geometry of
/// `ModelAssets::draws[i]` and takes its material. Two lists built by two
/// filters could differ; two lists built by one cannot.
pub struct FoliageProto {
    /// The archive path, which is also [`ModelCache`]'s key for the material.
    pub path: String,
    pub batches: Vec<FoliageGeometry>,
}

/// One batch of one foliage model, in Bevy's axes.
///
/// The model's own normals are not kept, which is a deviation from the 1.12.1
/// client. A tuft is two crossed cards whose files carry face normals, so the
/// normals lie flat in the ground plane: `DskGra05`'s are exactly `(0,-1,0)`
/// and `(1,0,0)`, and every grass model's `normal.z` is within 0.04 of zero.
/// Only the rock models have usable normals. Used as they are, with the
/// batches drawn two-sided as the file says, each card is lit on at most one
/// side and a tuft's brightness depends on its hashed yaw, which draws a field
/// of randomly bright and dark clumps.
///
/// So every foliage vertex takes the ground's shading normal under its tuft
/// instead ([`FoliageInstance::normal`], the `MCNR` sample at its cell's
/// centre), and the grass shades with the slope it grows on.
pub struct FoliageGeometry {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    /// How much of the wind each vertex takes: 0 at the tuft's base, 1 at its
    /// tip, baked once per model rather than once per tuft.
    ///
    /// The square of the vertex's height up the model, which is the cheapest
    /// curve that looks like a stalk bending rather than a card shearing. A
    /// detail model's origin is its base (every one of their bounding boxes
    /// starts within a few centimetres of zero), so the height is the vertex's
    /// own `up` and nothing has to be measured per instance.
    sway: Vec<f32>,
}

/// What one tile hands over: the plans, and the models they name.
///
/// Built on the tile loader thread (see [`read_tile_foliage`]), because that is
/// where the `Adt` and an archive both are, and because parsing even a few M2s
/// on the main thread causes a frame hitch.
#[derive(Default)]
pub struct TileFoliage {
    /// The plans, each with the index of the `MCNK` it came from.
    ///
    /// The index is carried, not derived: a chunk that grows nothing has no
    /// plan, so the plans are a subsequence of the tile's 256 and their
    /// positions in this list do not identify the chunk. The one caller that
    /// needs the index is a host editing the ground under a built tile; see
    /// [`TileFoliagePlans::reground`].
    pub chunks: Vec<(usize, ChunkFoliage)>,
    /// The distinct models this tile's plans name, by
    /// [`GroundEffects::models`] index. Repeats across tiles are dropped on
    /// arrival; a zone uses a few models (six on `Azeroth_34_51`).
    pub protos: Vec<(u16, FoliageProto)>,
}

/// The plans a tile carries, and what has been built from them.
///
/// A component of the tile entity, so when the tile goes out of range the
/// plans are dropped with the ground they describe, and the built meshes,
/// which are children of the same tile, are dropped with them.
#[derive(Component, Default)]
pub struct TileFoliagePlans {
    plans: Vec<PlannedChunk>,
    /// Set when plans arrive, so the first pass runs even if nobody has moved.
    dirty: bool,
}

impl TileFoliagePlans {
    /// Move one chunk's foliage onto ground that has been edited under it.
    ///
    /// The plan is regrounded (`ChunkFoliage::reground`, which holds the rule)
    /// and the entities already built from the old ground are handed back for
    /// the caller to despawn. A merged mesh is built once from the heights it
    /// was built with, so there is nothing in it to patch the way
    /// `render::terrain`'s vertices are patched. `stream_foliage` builds it
    /// again on its next pass, from the same hash, so the same tufts stand on
    /// the new heights.
    ///
    /// `false` for a chunk with no plan: bare ground, a chunk whose layers name
    /// no ground effect, a whole tile in a cave. That is not a failure; there
    /// is no grass there to move.
    ///
    /// Nothing in this crate calls it, because a chunk's heights do not change
    /// during a session. It is here for a host that changes them.
    pub fn reground(
        &mut self,
        chunk: usize,
        ground: vale_assets::world::adt::ChunkGround,
        normals: &[[f32; 3]],
        dead: &mut Vec<Entity>,
    ) -> bool {
        let Some(planned) = self.plans.iter_mut().find(|planned| planned.chunk == chunk) else {
            return false;
        };
        planned.plan.reground(ground, normals);
        dead.extend(planned.built.drain(..));
        // …so `stream_foliage` looks at this tile again even if the player has
        // not moved, which during an edit is every frame.
        self.dirty = true;
        true
    }
}

struct PlannedChunk {
    /// Which of the tile's 256 map chunks this is — see [`TileFoliage::chunks`].
    chunk: usize,
    plan: ChunkFoliage,
    /// The chunk's centre in Bevy's axes: what the distance is measured to,
    /// and where the merged meshes are positioned.
    centre: Vec3,
    /// The merged meshes while in range; empty while the chunk is only a plan.
    built: Vec<Entity>,
}

/// The CPU geometry of every foliage model seen so far, and the table behind
/// it.
///
/// One entry per model for the whole session: 411 models exist in the archives
/// and a zone uses a few, each a few hundred bytes of vertices, so nothing is
/// evicted.
#[derive(Resource, Default)]
pub struct FoliageModels {
    protos: HashMap<u16, Arc<FoliageProto>>,
    /// The join, once the first tile has brought it. `None` before a world is
    /// entered, and empty on a chain with no ground-effect tables, which draws
    /// bare ground and is not a failure.
    effects: Option<Arc<GroundEffects>>,
}

impl FoliageModels {
    /// Take what a tile brought: its models, and the table if this is the
    /// first.
    pub fn absorb(&mut self, effects: &Arc<GroundEffects>, protos: Vec<(u16, FoliageProto)>) {
        if self.effects.is_none() {
            self.effects = Some(Arc::clone(effects));
        }
        for (index, proto) in protos {
            self.protos.entry(index).or_insert_with(|| Arc::new(proto));
        }
    }
}

pub struct FoliagePlugin;

impl Plugin for FoliagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FoliageModels>().add_systems(
            Update,
            (
                stream_foliage,
                // Foliage streams, so a chunk that came into range while the
                // switch was off would be built visible. The doodad pass needs
                // the same catch-up for the same reason; `tuning::switch`'s
                // second `Added` pass does it.
                crate::render::tuning::switch::<Foliage>(|tuning| tuning.foliage),
            ),
        );
        // …and the HUD line for what it holds: chunks built, meshes and
        // triangles. `ui::report` says why the line is written here and not
        // in `hud.rs`.
        #[cfg(feature = "diagnostics")]
        app.add_systems(
            Update,
            report.after(stream_foliage).run_if(crate::ui::report::watched),
        );
    }
}

/// [`crate::ui::report::HudReport`] slot, beside the terrain's, because
/// foliage comes with the same tiles. See `ui::report`.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(16);

/// The HUD line: how many chunks are planned, how many are built, and what that
/// comes to in meshes and triangles.
///
/// Merged, a built chunk is one to three meshes; spawned per tuft it would be
/// one entity per tuft. The line prints chunks and meshes so the module doc's
/// figures can be checked in the window.
#[cfg(feature = "diagnostics")]
fn report(
    tiles: Query<&TileFoliagePlans>,
    models: Res<FoliageModels>,
    drawn: Query<&Foliage>,
    mut hud: ResMut<crate::ui::report::HudReport>,
) {
    let planned: usize = tiles.iter().map(|t| t.plans.len()).sum();
    let built: usize = tiles
        .iter()
        .flat_map(|t| t.plans.iter())
        .filter(|c| !c.built.is_empty())
        .count();
    // Read from the entities, not the assets. See [`Foliage`]: asking a
    // `RENDER_WORLD`-only mesh for its indices panics once it has been
    // extracted, so this line cannot be built from the meshes it describes.
    let drawn_meshes = drawn.iter().count();
    let triangles: usize = drawn.iter().map(|f| f.triangles).sum();
    hud.set(
            crate::ui::report::Section::Scene,
        SLOT,
        "foliage",
        format!(
            "foliage: {built}/{planned} chunks built, {drawn_meshes} meshes, \
             {triangles} triangles, {} models held",
            models.protos.len()
        ),
    );
}

/// Read one tile's foliage: the plans, and the geometry of every model they
/// name.
///
/// Runs on the tile loader thread, with the tile's own archive handle. The
/// models are read here rather than through [`ModelCache`] because this needs
/// the vertices, which the cache does not keep. Their textures still come from
/// the cache, so the expensive part is loaded once for the session and shared
/// with every other doodad.
pub fn read_tile_foliage(
    archive: &mut vale_assets::Assets,
    adt: &Adt,
    effects: &GroundEffects,
) -> TileFoliage {
    let mut out = TileFoliage::default();
    if effects.is_empty() {
        return out;
    }
    let mut wanted: Vec<u16> = Vec::new();
    for (index, chunk) in adt.chunks.iter().enumerate() {
        let Some(plan) = ChunkFoliage::plan(chunk, effects) else {
            continue;
        };
        for &id in &plan.cells {
            let Some(effect) = effects.effect(id) else {
                continue;
            };
            for &model in &effect.models[..usize::from(effect.choices)] {
                if !wanted.contains(&model) {
                    wanted.push(model);
                }
            }
        }
        out.chunks.push((index, plan));
    }

    for model in wanted {
        let path = effects.model(model);
        // A model in no archive (32 of the 443 the table names) is not read.
        // The tufts that would have used it are not drawn, which is the same
        // degradation `ModelCache` reports once for the material.
        let Ok(raw) = archive.read(path) else { continue };
        let Ok(parsed) = M2::parse(&raw) else { continue };
        let batches = parsed
            .batches
            .iter()
            // The same filter the renderer's own build uses, through the same
            // function, so the batch lists cannot come out different lengths
            // and give a batch its neighbour's material. No folded layers: a
            // blade of grass is one batch, and this build merges thousands of
            // them rather than building one model.
            .filter_map(|batch| {
                crate::render::models::loader::batch_draw(&parsed, batch, None, Default::default())
            })
            .map(|draw| {
                // Bevy's up is `+Y`, and a detail model's base is its origin.
                let top = draw
                    .positions
                    .iter()
                    .map(|p| p[1])
                    .fold(0.0f32, f32::max)
                    .max(1e-3);
                let sway = draw
                    .positions
                    .iter()
                    .map(|p| {
                        let t = (p[1] / top).clamp(0.0, 1.0);
                        t * t
                    })
                    .collect();
                FoliageGeometry {
                    positions: draw.positions,
                    uvs: draw.uvs,
                    indices: draw.indices,
                    sway,
                }
            })
            .collect();
        out.protos.push((
            model,
            FoliageProto {
                path: path.to_string(),
                batches,
            },
        ));
    }
    out
}

/// The plans a tile arrived with, as the component the streamer walks.
pub fn plans_of(tile: TileFoliage, models: &mut FoliageModels, effects: &Arc<GroundEffects>) -> TileFoliagePlans {
    let plans = tile
        .chunks
        .into_iter()
        .map(|(chunk, plan)| {
            // The chunk's centre: half a chunk in from its origin along both
            // axes, which run away from it. The height is the origin's, which
            // is within a few yards of the middle and only decides a distance.
            let centre = axes::to_bevy([
                plan.ground.position[0] - CHUNK_SIZE / 2.0,
                plan.ground.position[1] - CHUNK_SIZE / 2.0,
                plan.ground.position[2],
            ]);
            PlannedChunk {
                chunk,
                plan,
                centre,
                built: Vec::new(),
            }
        })
        .collect();
    models.absorb(effects, tile.protos);
    TileFoliagePlans { plans, dirty: true }
}

/// Whether a chunk at `distance²` should have geometry, given whether it has.
///
/// The two edges are apart on purpose: a chunk is built when its centre comes
/// within the range plus its own radius, and kept until a further
/// [`KEEP_MARGIN`], so a player standing on the boundary does not rebuild the
/// ring on every step.
fn wants_built(d2: f32, built: bool) -> bool {
    let edge = FOLIAGE_RANGE + CHUNK_RADIUS + if built { KEEP_MARGIN } else { 0.0 };
    d2 < edge * edge
}

/// `frillDensity` as the 1.12.1 client accepts it: a whole number of cell
/// draws in 1..=256, [`DEFAULT_FRILL_DENSITY`] with no settings at all.
fn frill_density(cvars: Option<&crate::settings::cvars::CVars>) -> u32 {
    cvars.map_or(DEFAULT_FRILL_DENSITY, |c| {
        let value = c.number("frillDensity");
        if value >= 1.0 {
            (value as u32).clamp(*FRILL_DENSITY_RANGE.start(), *FRILL_DENSITY_RANGE.end())
        } else {
            DEFAULT_FRILL_DENSITY
        }
    })
}

/// Build the chunks the player has come near, and drop the ones left behind.
fn stream_foliage(
    mut commands: Commands,
    focus: Res<crate::render::focus::WorldFocus>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut cache: ResMut<ModelCache>,
    // …and the pool the swaying copy of each foliage material is interned in.
    // The model cache returns the material the 1.12.1 client would draw a tuft
    // with; this pass uses that material with one pipeline difference. See
    // [`crate::render::models::Materials::with_wind`].
    mut materials: crate::render::models::Materials,
    models: Res<FoliageModels>,
    mut tiles: Query<(Entity, &mut TileFoliagePlans), With<TerrainTile>>,
    mut last: Local<Option<Vec3>>,
    mut scratch: Local<Vec<FoliageInstance>>,
    // `frillDensity`, the number of cell draws per chunk. The registered
    // default is seeded into `CVars`, so a `Config.wtf` without it reads 16.
    cvars: Option<Res<crate::settings::cvars::CVars>>,
    mut built_at: Local<Option<u32>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Foliage);
    if !focus.present {
        return;
    }
    let Some(effects) = models.effects.clone() else {
        return;
    };
    let frill = frill_density(cvars.as_deref());
    // A changed density changes every chunk's lawn, so everything built at
    // the old value is dropped and built again.
    if built_at.is_some_and(|was| was != frill) {
        for (_, mut plans) in &mut tiles {
            for chunk in &mut plans.plans {
                for entity in chunk.built.drain(..) {
                    commands.entity(entity).despawn();
                }
            }
            plans.dirty = true;
        }
    }
    *built_at = Some(frill);
    let player = axes::to_bevy(focus.position.to_array());
    let moved = last.is_none_or(|p| p.distance_squared(player) >= STREAM_STEP * STREAM_STEP);
    if !moved && !tiles.iter().any(|(_, plans)| plans.dirty) {
        return;
    }

    let mut budget = BUILD_BUDGET;
    let mut complete = true;
    for (tile, mut plans) in &mut tiles {
        let plans = &mut *plans;
        for chunk in &mut plans.plans {
            let d2 = chunk.centre.distance_squared(player);
            let want = wants_built(d2, !chunk.built.is_empty());
            if want && chunk.built.is_empty() {
                if budget == 0 {
                    complete = false;
                    continue;
                }
                match merge_chunk(
                    &chunk.plan,
                    chunk.centre,
                    frill,
                    &effects,
                    &models,
                    &mut cache,
                    &mut materials,
                    &mut scratch,
                ) {
                    // A model still coming off the loader: leave the chunk
                    // unbuilt and try again next pass rather than merging half
                    // its lawn, which would be a mesh nothing ever completes.
                    None => complete = false,
                    Some(merged) => {
                        budget -= 1;
                        for group in merged {
                            let entity = commands
                                .spawn((
                                    Foliage {
                                        triangles: group.triangles,
                                    },
                                    Mesh3d(meshes.add(group.mesh)),
                                    MeshMaterial3d(group.material),
                                    Transform::from_translation(chunk.centre),
                                    bevy::mesh::MeshTag(group.tag),
                                    VisibilityRange::abrupt(0.0, FOLIAGE_RANGE + CHUNK_RADIUS),
                                    // A child of the tile, so the ground
                                    // walking out of range takes its grass.
                                    ChildOf(tile),
                                ))
                                .id();
                            chunk.built.push(entity);
                        }
                    }
                }
            } else if !want && !chunk.built.is_empty() {
                for entity in chunk.built.drain(..) {
                    commands.entity(entity).despawn();
                }
            }
        }
        if complete {
            plans.dirty = false;
        }
    }
    if complete {
        *last = Some(player);
    }
}

/// One chunk's tufts, merged into a mesh per material.
///
/// `None` while any model the chunk plants is still loading. The whole chunk
/// waits, because a mesh is built once and a tuft left out of it is never
/// added. A model that has failed to load is skipped instead: that is
/// permanent, and waiting for it would leave the chunk bare.
///
/// The tag carries the per-instance sun scale, so a chunk that is partly in the
/// ground's baked `MCSH` shadow comes out as two meshes per material rather
/// than one. See [`instance_tag`], and [`FoliageInstance::shadowed`], which is
/// the shadow bit sampled under each tuft.
fn merge_chunk(
    plan: &ChunkFoliage,
    centre: Vec3,
    frill_density: u32,
    effects: &GroundEffects,
    models: &FoliageModels,
    cache: &mut ModelCache,
    materials: &mut crate::render::models::Materials,
    scratch: &mut Vec<FoliageInstance>,
) -> Option<Vec<Merged>> {
    scratch.clear();
    plan.grow(effects, frill_density, scratch);
    if scratch.is_empty() {
        // No draw planted anything: every draw landed on a cell that grows
        // nothing or on a hole, or every tuft took an empty column. The chunk
        // has no geometry and must still count as built, or the streamer
        // retries it every pass.
        return Some(Vec::new());
    }

    // The materials, resolved once per distinct model rather than per tuft.
    let mut resolved: HashMap<u16, Option<Arc<crate::render::models::ModelAssets>>> =
        HashMap::default();
    let mut waiting = false;
    for tuft in scratch.iter() {
        resolved.entry(tuft.model).or_insert_with(|| {
            match cache.lookup(effects.model(tuft.model)) {
                Lookup::Ready(assets) => Some(assets),
                Lookup::Loading => {
                    waiting = true;
                    None
                }
                Lookup::Failed => None,
            }
        });
    }
    if waiting {
        return None;
    }

    // The swaying copy of each material, interned once per chunk build and
    // keyed on the still material's id rather than looked up per tuft: a
    // chunk is up to a few hundred tufts over one to three materials, and
    // `with_wind` is a clone and a hash of 64 words.
    let mut swaying: HashMap<AssetId<M2Material>, Handle<M2Material>> = HashMap::default();

    let mut groups: HashMap<(AssetId<M2Material>, bool), Group> = HashMap::default();
    for tuft in scratch.iter() {
        let Some(Some(assets)) = resolved.get(&tuft.model) else {
            continue;
        };
        let Some(proto) = models.protos.get(&tuft.model) else {
            continue;
        };
        // The tuft's own basis: a yaw about the world's up, which is Bevy's
        // `+Y`. The change of basis carries a rotation about WoW's `+Z` to one
        // about `+Y` by the same angle — see `axes::basis`.
        let (sin, cos) = tuft.yaw.sin_cos();
        let offset = axes::to_bevy(tuft.position) - centre;
        // The ground's normal, not the card's; see [`FoliageGeometry`]. It is
        // one normal for the whole tuft, so the basis change happens once here
        // rather than per vertex.
        let normal = axes::to_bevy(tuft.normal).normalize_or(Vec3::Y);
        // …and the phase the wind reads: the tuft's own hashed phase plus a
        // term in how far along the wind it stands. The second term makes the
        // tufts move as a wave crossing the field rather than independently.
        // It is baked, so the shader adds a clock and takes one sine.
        let along = offset.x * WIND_DIRECTION.x + offset.z * WIND_DIRECTION.y;
        let phase = tuft.phase + along * WIND_WAVE;

        for (index, geometry) in proto.batches.iter().enumerate() {
            let Some(draw) = assets.draws.get(index) else {
                continue;
            };
            let material = swaying
                .entry(draw.material.id())
                .or_insert_with(|| {
                    materials
                        .with_wind(&draw.material)
                        // Already swaying, or the handle stopped resolving
                        // between the cache's answer and here: draw it still
                        // rather than not at all.
                        .unwrap_or_else(|| draw.material.clone())
                })
                .clone();
            let group = groups
                .entry((material.id(), tuft.shadowed))
                .or_insert_with(|| Group::new(material.clone()));
            group.push(geometry, tuft.scale, sin, cos, offset, normal, phase);
        }
    }

    Some(
        groups
            .into_iter()
            .map(|((_, shadowed), group)| {
                // A tuft takes a doodad's sun scale, the 1.12.1 client's rule
                // for a doodad: `NEUTRAL` (1.0) on lit ground and
                // `SHADOWED_GROUND` (0.5) over the ground's `MCSH` shadow.
                // `LIT_GROUND` (2.5) is an entity's (a unit, player or game
                // object on lit ground), not a doodad's; on a tuft that takes
                // the ground's normal it draws the grass visibly brighter than
                // the terrain it stands in.
                //
                // At `NEUTRAL` the arithmetic is exactly the ground's: the same
                // normal through the same `daylight` with the same scale, so a
                // tuft and the texel under it are lit by one calculation. In
                // shadow the ground is dimmed by `terrain1.bls`'s flat 0.3/0.7
                // and the tuft by `SHADOWED_GROUND`, a different curve for the
                // same shadow.
                let sun = if shadowed {
                    sun_scale::SHADOWED_GROUND
                } else {
                    sun_scale::NEUTRAL
                };
                let material = group.material.clone();
                let triangles = group.indices.len() / 3;
                Merged {
                    material,
                    tag: instance_tag(None, sun),
                    triangles,
                    mesh: group.into_mesh(),
                }
            })
            .collect(),
    )
}

/// One finished mesh of a chunk's lawn, with everything the spawn needs.
///
/// A named type rather than a `(Handle, u32, usize, Mesh)` tuple, so that the
/// `triangles` field, which exists so nothing has to read the mesh back (see
/// [`Foliage`]), is named.
struct Merged {
    material: Handle<M2Material>,
    /// The packed sun scale — see [`instance_tag`].
    tag: u32,
    triangles: usize,
    mesh: Mesh,
}

/// One material's share of a chunk, accumulating.
struct Group {
    material: Handle<M2Material>,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    /// The sway weight and phase, which become `UV_1`; see
    /// [`Group::into_mesh`].
    wind: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Group {
    fn new(material: Handle<M2Material>) -> Group {
        Group {
            material,
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            wind: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Append one tuft: the batch's vertices scaled, rotated by its yaw and
    /// moved to its place, and its indices offset past whatever is already
    /// here.
    ///
    /// `normal` is the ground's, one for the whole tuft (the deviation
    /// described on [`FoliageGeometry`]), and `phase` is what the wind reads,
    /// baked here because it depends on where the tuft stands.
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        geometry: &FoliageGeometry,
        scale: f32,
        sin: f32,
        cos: f32,
        offset: Vec3,
        normal: Vec3,
        phase: f32,
    ) {
        let base = self.positions.len() as u32;
        for p in &geometry.positions {
            let p = [p[0] * scale, p[1] * scale, p[2] * scale];
            self.positions.push([
                cos * p[0] + sin * p[2] + offset.x,
                p[1] + offset.y,
                -sin * p[0] + cos * p[2] + offset.z,
            ]);
        }
        let normal = normal.to_array();
        self.normals
            .extend(std::iter::repeat_n(normal, geometry.positions.len()));
        self.uvs.extend_from_slice(&geometry.uvs);
        self.wind
            .extend(geometry.sway.iter().map(|&weight| [weight, phase]));
        self.indices
            .extend(geometry.indices.iter().map(|&i| i + base));
    }

    fn into_mesh(self) -> Mesh {
        // `RENDER_WORLD` like every other world mesh in this crate: nothing
        // reads these vertices back, because the CPU copy that is kept is the
        // proto, not the merged result.
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        // The sway is carried in `UV_1`. Nothing else in this renderer writes
        // a second UV set, so the attribute costs eight bytes a vertex on the
        // foliage and nothing anywhere else. The vertex stage reads it as
        // `(weight, phase)`; see `wind.wgsl`.
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, self.wind);
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blade one yard up and one yard out along the model's own `+X`.
    fn blade() -> FoliageGeometry {
        FoliageGeometry {
            positions: vec![[0.0, 0.0, 0.0], axes::to_bevy([1.0, 0.0, 0.0]).to_array()],
            uvs: vec![[0.0, 0.0], [1.0, 0.0]],
            indices: vec![0, 1, 0],
            sway: vec![0.0, 1.0],
        }
    }

    /// A tuft rotated by its yaw stays the same size and lands where the plan
    /// put it. This fails if the rotation and the change of basis disagree
    /// about which axis is up, which would lay every blade of grass on its
    /// side.
    #[test]
    fn a_tuft_is_turned_about_the_worlds_up_and_not_tipped_over() {
        let geometry = blade();
        let mut group = Group::new(Handle::default());
        let (sin, cos) = std::f32::consts::FRAC_PI_2.sin_cos();
        group.push(&geometry, 1.0, sin, cos, Vec3::new(10.0, 3.0, -4.0), Vec3::Y, 0.0);

        let base = Vec3::from(group.positions[0]);
        let tip = Vec3::from(group.positions[1]);
        assert!(
            (base - Vec3::new(10.0, 3.0, -4.0)).length() < 1e-5,
            "the tuft's own origin moved to {base:?}"
        );
        // A quarter turn about the up axis: the same length, still level.
        assert!(((tip - base).length() - 1.0).abs() < 1e-5, "the blade changed length");
        assert!((tip.y - base.y).abs() < 1e-5, "the blade tipped over");
        // …and it really turned: the model's own +X is not where it started.
        let unturned = axes::to_bevy([1.0, 0.0, 0.0]);
        assert!((tip - base - unturned).length() > 0.5, "the yaw did nothing");
    }

    /// Merging offsets each tuft's indices past the vertices already in the
    /// buffer. Without the offset nothing fails visibly: every tuft's indices
    /// draw the first tuft's triangles again, so the grass looks thin rather
    /// than broken.
    #[test]
    fn merged_indices_are_offset_past_what_is_already_there() {
        let geometry = FoliageGeometry {
            positions: vec![[0.0; 3]; 3],
            uvs: vec![[0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            sway: vec![0.0; 3],
        };
        let mut group = Group::new(Handle::default());
        for _ in 0..3 {
            group.push(&geometry, 1.0, 0.0, 1.0, Vec3::ZERO, Vec3::Y, 0.0);
        }
        assert_eq!(group.positions.len(), 9);
        assert_eq!(group.indices, vec![0, 1, 2, 3, 4, 5, 6, 7, 8]);
    }

    /// Every vertex of a tuft takes the ground's normal, not the card's: the
    /// deviation described on [`FoliageGeometry`]. Without it a field of grass
    /// draws as randomly bright and dark clumps. The wrong version still draws
    /// lit grass, only noisy, so it is tested here.
    #[test]
    fn every_vertex_of_a_tuft_is_lit_by_the_ground_under_it() {
        let geometry = blade();
        let mut group = Group::new(Handle::default());
        // A slope: the ground leaning a quarter of the way over.
        let ground = Vec3::new(0.25, 1.0, -0.1).normalize();
        // Two tufts at different yaws, which under the card's own normals would
        // have come out shaded differently.
        group.push(&geometry, 1.0, 0.0, 1.0, Vec3::ZERO, ground, 0.0);
        group.push(&geometry, 1.0, 1.0, 0.0, Vec3::X, ground, 0.0);
        assert_eq!(group.normals.len(), group.positions.len());
        for normal in &group.normals {
            assert!(
                (Vec3::from(*normal) - ground).length() < 1e-5,
                "a vertex took {normal:?} rather than the ground's {ground:?}"
            );
        }
    }

    /// The sway weight holds the base still and moves the tip, and the phase
    /// is one number for the whole tuft.
    ///
    /// Both are baked into `UV_1` and read by a vertex stage no test can run,
    /// so this checks the buffer: a base vertex that swayed would slide the
    /// grass out of the ground, and a per-vertex phase would tear each card in
    /// half.
    #[test]
    fn the_sway_weight_is_zero_at_the_base_and_one_at_the_tip() {
        let mut group = Group::new(Handle::default());
        group.push(&blade(), 1.0, 0.0, 1.0, Vec3::ZERO, Vec3::Y, 1.5);
        assert_eq!(group.wind, vec![[0.0, 1.5], [1.0, 1.5]]);
    }

    /// The triangle count is recorded at the merge and never read from the
    /// mesh. These meshes are `RENDER_WORLD`-only, and `Mesh::indices` panics
    /// on one that has been extracted instead of answering `None`. A HUD line
    /// that read the count from the mesh panicked when `F4` was pressed; that
    /// kind of bug only fires after the frame the mesh was built on.
    ///
    /// This checks that the number on the component is the number in the
    /// buffer beside it. The two are written in one place, and a difference
    /// between them is the only way the line can be wrong.
    #[test]
    fn the_triangle_count_comes_off_the_buffer_and_not_off_the_asset() {
        let geometry = FoliageGeometry {
            positions: vec![[0.0; 3]; 4],
            uvs: vec![[0.0, 0.0]; 4],
            // Two triangles a tuft.
            indices: vec![0, 1, 2, 0, 2, 3],
            sway: vec![0.0; 4],
        };
        let mut group = Group::new(Handle::default());
        for _ in 0..5 {
            group.push(&geometry, 1.0, 0.0, 1.0, Vec3::ZERO, Vec3::Y, 0.0);
        }
        let triangles = group.indices.len() / 3;
        assert_eq!(triangles, 10, "five tufts of two triangles");
        let merged = Merged {
            material: group.material.clone(),
            tag: 0,
            triangles,
            mesh: group.into_mesh(),
        };
        assert_eq!(
            merged.mesh.indices().map(|i| i.len() / 3),
            Some(merged.triangles),
            "the component would report a different lawn from the one drawn"
        );
    }

    /// The build and keep edges are both beyond the range and do not meet. A
    /// chunk must be built by the time its nearest corner could be seen, and
    /// must be kept past that edge, or a player standing on the boundary
    /// rebuilds the chunk twice a step.
    #[test]
    fn the_build_edge_and_the_keep_edge_are_apart() {
        let d2 = |d: f32| d * d;
        let edge = FOLIAGE_RANGE + CHUNK_RADIUS;
        assert!(wants_built(d2(FOLIAGE_RANGE), false), "in range, unbuilt");
        assert!(wants_built(d2(edge - 1.0), false));
        // Between the two edges: an unbuilt chunk stays unbuilt and a built one
        // stays built. That band is the hysteresis.
        let between = d2(edge + KEEP_MARGIN / 2.0);
        assert!(!wants_built(between, false));
        assert!(wants_built(between, true));
        // Past both: gone either way.
        let beyond = d2(edge + KEEP_MARGIN + 1.0);
        assert!(!wants_built(beyond, false));
        assert!(!wants_built(beyond, true));
    }

    /// The visibility cutoff and the build edge are the same number, so no
    /// chunk is ever built and invisible, or visible and unbuilt: the chunk is
    /// built exactly when it comes within the visibility range.
    #[test]
    fn the_visibility_cutoff_is_the_build_edge() {
        assert!(wants_built((FOLIAGE_RANGE + CHUNK_RADIUS - 0.01).powi(2), false));
        assert!(!wants_built((FOLIAGE_RANGE + CHUNK_RADIUS + 0.01).powi(2), false));
    }

    /// The wind's direction is written in two places and they must agree.
    ///
    /// The phase is baked into the vertex buffer here, along this axis; the
    /// shader leans the tips along the same axis every frame. They cannot be
    /// one constant, because one is Rust and one is WGSL, so this reads the
    /// shipped shader and compares. A mismatch does not crash: the grass's
    /// wave travels across the direction it leans, which looks like a shimmer
    /// rather than wind and has no obvious cause.
    #[test]
    fn the_winds_direction_is_the_same_number_in_the_shader() {
        let wgsl = include_str!("models/shaders/wind.wgsl");
        let line = wgsl
            .lines()
            .find(|l| l.contains("const WIND_DIRECTION"))
            .expect("wind.wgsl states a direction");
        let numbers: Vec<f32> = line
            .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .filter(|s| s.contains('.'))
            .filter_map(|s| s.parse().ok())
            .collect();
        assert_eq!(numbers.len(), 2, "read {numbers:?} out of {line:?}");
        assert!(
            (Vec2::new(numbers[0], numbers[1]) - WIND_DIRECTION).length() < 1e-4,
            "the shader leans along {numbers:?}, the buffer is phased along {WIND_DIRECTION:?}"
        );
        // …and it has to be a unit vector, or the amplitude is not what the
        // shader's comment says it is.
        assert!((WIND_DIRECTION.length() - 1.0).abs() < 1e-3);
    }

    /// A model arriving from a second tile does not replace the copy already
    /// held. The tiles of a zone name the same few models, so re-interning them
    /// would be an allocation per tile crossing for geometry that has not
    /// changed.
    #[test]
    fn a_model_seen_twice_is_kept_once() {
        let mut models = FoliageModels::default();
        let effects = Arc::new(GroundEffects::load(|_| None));
        let proto = |path: &str| FoliageProto {
            path: path.to_string(),
            batches: Vec::new(),
        };
        models.absorb(&effects, vec![(3, proto("first"))]);
        let held = Arc::clone(models.protos.get(&3).expect("just absorbed"));
        models.absorb(&effects, vec![(3, proto("second")), (4, proto("other"))]);
        assert_eq!(models.protos.len(), 2);
        assert!(Arc::ptr_eq(&held, models.protos.get(&3).unwrap()));
        assert_eq!(models.protos[&3].path, "first");
    }
}
