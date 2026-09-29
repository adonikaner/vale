//! The doodad pass: every M2 the terrain tiles place.
//!
//! A tile is ~1,400 `MDDF` placements of ~130 distinct models, and the 3x3
//! around the character is ~12,000 of them. `models.rs` turns a path into a list
//! of (mesh, material) draws; this turns a placement into entities carrying
//! them.
//!
//! Each batch of each placement is one entity, spawned only while the player
//! is within that placement's draw range. Bevy batches draws by mesh and
//! material and culls each placement individually, so no per-model instance
//! grouping is needed. (The WebGL renderer grouped instances by model across
//! every loaded tile and rebuilt the buffers when the tile set changed,
//! because one draw call per placement was tens of thousands a frame.)
//!
//! Spawning happens in two stages. [`resolve_doodads`] turns a placement whose
//! model has loaded into a [`Resident`]: hull placed, transform composed,
//! draws held. [`stream_doodads`] spawns and despawns the entities as the
//! player crosses each resident's own draw range. The reason is the CPU-floor
//! measurement: the per-frame render cost scales with spawned meshes, not
//! visible ones, so a `VisibilityRange` alone leaves a city's six thousand
//! mugs paying the binned-phase rebuild from three hundred yards away. The
//! `VisibilityRange` still decides which pixels are drawn, and streaming
//! spawns a margin before it, so objects appear and disappear at the same
//! distance as when everything was spawned.
//!
//! ## A doodad belongs to the tile its origin is on
//!
//! An object touching two tiles is listed in both tiles' `MDDF` with the same
//! `unique_id`. Drawing it twice doubles the cost of every tree along every
//! seam and z-fights on its leaves. Each placement is assigned to the tile
//! that contains its own origin. This needs no shared state and survives a
//! tile being dropped: a doodad is a child of exactly one tile entity, and
//! despawning that tile despawns it. (The WebGL renderer deduplicated with a
//! set of ids rebuilt whenever the loaded tile set changed.)
//!
//! The cost is a ~50-yard band at the outer edge of the loaded world where an
//! overhanging object whose origin is on an unloaded tile is not drawn. The
//! terrain stops at the same band.
//!
//! ## Animated doodads
//!
//! Most doodads are static. 17 of a Darkshire tile's 128 models carry a
//! skeleton, and 38 of the 300 in Stormwind's `MODD` sets do: the bellows, the
//! gryphon roosts, a windmill, and the torches whose flame is a billboarded
//! bone.
//!
//! A placement within draw range takes the model's skinned build and gets a
//! rig: a root at the placement, a joint per bone, and [`pose_scenery`]
//! writing `placement × bone` onto them each frame. `vale_assets::look::scenery`
//! decides which clip it plays and at what phase; that rule is unit-tested
//! with no renderer running.
//!
//! Every rigged placement is posed while it is spawned. There is no second,
//! shorter posing range; the note above [`ResidentDoodads`] says why.
//!
//! ## Doodad collision
//!
//! Each placement whose model carries a `BoundingTriangles` hull also goes into
//! [`Solids`], so a tree blocks a stride and a crate can be stood on. It uses
//! the same [`Collider`] and the same two queries as a `MODF` building — see
//! [`vale_assets::world::collision`] — and the hull is stored with the model in
//! [`crate::render::models::ModelAssets`], so a placement adds only the
//! transform.
//!
//! The difference from buildings is the count. A tile has a dozen buildings
//! and up to four thousand solid doodads once a city's furniture is counted.
//! Two operations that were cheap at forty hulls are not at four thousand, so
//! the dedup that recognises a repeated placement does not scan, and the wall
//! query rejects a hull by its XY box before ray-testing it. With both, `vale collision Azeroth 31 48` measures a floor-plus-stride
//! query pair at 67 us against Stormwind's 3,817 hulls and 418,692 triangles.
//! The mover queries twenty times a second, so no distance cull is applied.

use crate::axes;
use crate::render::models::{Lookup, Materials, ModelCache, ModelDraw, RoomLight};
use crate::world::session::Solids;
use crate::render::terrain::TerrainTile;
use vale_assets::world::adt::PlacedModel;
use vale_assets::world::collision::{Collider, Solid, SolidId};
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;
use std::sync::Arc;

/// How many placements to resolve — model looked up, hull placed, resident
/// recorded — in one frame.
///
/// A tile's models arrive in bursts and its 1,400 placements would otherwise be
/// one frame's work each time one finishes loading. Spread out, the world fills
/// in over a second or so instead of hitching.
const RESOLVE_BUDGET: usize = 400;

/// How many residents to spawn in one frame once they come into range.
///
/// The first pass over a freshly arrived city tile wants thousands at once;
/// at this budget they fill in over a few frames instead of one.
const STREAM_BUDGET: usize = 400;

/// How far the player moves before the resident lists are re-walked.
///
/// The walk is cheap — a distance per resident over the 3x3 — but it does not
/// need to happen per frame: at a run (7.6 y/s) this is about once a second,
/// and [`STREAM_MARGIN`] is what absorbs the movement in between.
const STREAM_STEP: f32 = 8.0;

/// How far beyond its own draw range a doodad is spawned, and half of how far
/// beyond it one is despawned.
///
/// The margin spawns the entity before the camera reaches the
/// [`VisibilityRange`] cutoff. The range still decides which pixels are drawn,
/// so objects appear at the same distance as when every placement was
/// spawned. The wider despawn edge is hysteresis: a player moving back and
/// forth on one spot does not repeatedly spawn and despawn the boundary ring.
const STREAM_MARGIN: f32 = 40.0;

/// The bounding radius above which the 1.12.1 client never distance-fades a
/// doodad. Trees and huts are drawn until the far clip removes them.
const NEVER_FADE_RADIUS: f32 = 7.0;

/// Where a doodad past [`NEVER_FADE_RADIUS`] is finally cut off anyway.
///
/// The 1.12.1 client draws these to its far clip. This client has no far
/// clip, so the ceiling replaces it. The ceiling lies past the edge of the
/// loaded 3x3, so the end of the terrain hides a tree before the range does.
const RANGE_CEILING: f32 = 800.0;

/// How far away a doodad is still drawn, using the 1.12.1 client's fade bands.
///
/// The client fades every world doodad by a band chosen only from its
/// bounding-sphere radius:
/// radius ≤ 0.5 fades over 40→50 yards, ≤ 2.5 over 100→125, ≤ 7.0 over
/// 150→200, and anything bigger never fades. The distance is measured to the
/// surface (centre distance minus radius), and a doodad whose fade reaches
/// zero is culled, not drawn transparent. A mug is therefore not drawn past
/// 50 yards. The previous rule, `radius × 60, floor 150`, gave ranges about
/// 3x longer. The spawned population sets the CPU floor (see the module doc),
/// so under that rule every candle in a city paid the render world's
/// per-frame walk from three times the distance the client draws it.
///
/// Two deviations, both of which draw more, never less:
/// * The cutoff is abrupt at the band's far end. The client feathers across
///   the band (`fade = 1 − (d − start)/width` into the per-vertex diffuse
///   alpha). This code draws fully opaque everything the client draws at
///   all; only the partial transparency is missing. The feather needs a fade
///   channel in the instance tag and a blend-mode variant per material, and
///   is not implemented.
/// * The radius is the AABB half-diagonal at the placement's scale (see the
///   caller). It is an upper bound on the scaled bounding-sphere radius the
///   client uses, so a borderline model falls into the longer band.
///
/// Buildings get no range. A WMO is a landmark seen from the next zone, its
/// batches are mostly opaque and merge under multidraw, and GPU occlusion
/// culling already handles its interiors.
fn draw_range(radius: f32) -> f32 {
    let range = if radius > NEVER_FADE_RADIUS {
        RANGE_CEILING
    } else {
        let (start, width) = if radius <= 0.5 {
            (40.0, 10.0)
        } else if radius <= 2.5 {
            (100.0, 25.0)
        } else {
            (150.0, 50.0)
        };
        // The client culls where the fade reaches zero: `d = centre − radius`
        // past `start + width`, i.e. a centre distance of `start + width + radius`.
        start + width + radius
    };
    match park_probe() {
        Some(cap) => range.min(cap),
        None => range,
    }
}

/// `VALE_PARK_BEYOND=<yards>`: a measurement probe, not a setting.
///
/// Clamps every doodad's draw range, so the streaming in this file parks
/// (despawns) everything past the cap. It measures, with one interleaved A/B,
/// whether the spawned population costs frame time. The sweeps that scale
/// with it (`check_visibility`, the `Aabb` and `Changed` walks) rank in every
/// trace, but the wireframe sweep also ranked and removing it measured no
/// change, so their wall-clock cost cannot be inferred from self-times. Two
/// runs that differ in this variable remove about half the spawned batches at
/// once. No `--without` switch can do that: those switches write
/// `Visibility`, and a hidden entity is still swept.
///
/// It visibly empties the middle distance. A probe may do that and a setting
/// may not, so it is not in `WorldTuning`.
fn park_probe() -> Option<f32> {
    static CAP: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("VALE_PARK_BEYOND")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
    })
}

/// The placements a tile owns that have not been spawned yet.
///
/// Attached by the terrain pass when the tile's ADT is parsed, and drained as
/// each model becomes available. Empty is the steady state.
///
/// There are two lists because there are two populations. [`Self::outdoor`]
/// is the tile's own `MDDF` and is what `vale models` lists. [`Self::indoor`]
/// is the `MODD` furniture the WMO pass hands over as each building arrives,
/// which appears in no `MDDF`. Their sum is larger than the tile lists and
/// cannot be checked against anything, so the two are kept and reported
/// separately. (The HUD used to report the sum.)
#[derive(Component, Default)]
pub struct PendingDoodads {
    pub outdoor: Vec<PlacedDoodad>,
    pub indoor: Vec<PlacedDoodad>,
}

/// A placement and what lights it.
///
/// The light is the only difference between an indoor and an outdoor
/// placement. It is carried per placement rather than looked up because it
/// belongs to the placement: a `MODD` names its own baked colour, so two
/// copies of one barrel in two rooms of one building are lit differently.
/// `None` is the outdoor case, lit by the sun, which covers every `MDDF`
/// placement.
pub struct PlacedDoodad {
    pub placement: PlacedModel,
    pub light: Option<RoomLight>,
    /// The per-instance multiplier on the sun term: 2.5 or 0.5 for an `MDDF`
    /// placement, chosen by the `MCSH` bit under its origin, and 1.0 for a
    /// WMO's own props. See [`crate::render::models::sun_scale`].
    pub sun_scale: f32,
}

impl PlacedDoodad {
    /// An `MDDF` placement, lit by the sun and scaled by whether its origin
    /// lies in the ground's baked shadow. The tile loader decides that with
    /// one [`vale_assets::world::adt::Adt::shadowed_at`] call per placement.
    pub fn on_ground(placement: PlacedModel, shadowed: bool) -> PlacedDoodad {
        PlacedDoodad {
            placement,
            light: None,
            sun_scale: if shadowed {
                crate::render::models::sun_scale::SHADOWED_GROUND
            } else {
                crate::render::models::sun_scale::LIT_GROUND
            },
        }
    }
}

/// One drawn batch of one placement.
///
/// Not one placement: a model is one mesh per `M2Batch` and each is its own
/// entity, so a tree with a trunk and a canopy is two of these.
#[derive(Component)]
pub struct Doodad {
    /// `MDDF`'s id, which is what `vale models <Map> <x> <y>` prints.
    pub unique_id: u32,
}

/// A placement whose model has loaded: everything needed to spawn its batches,
/// held so that they are spawned only within range of the player.
///
/// The per-frame render cost scales with spawned meshes, not visible ones:
/// the binned phases are rebuilt and the preprocessing buffers rewritten for
/// the whole spawned population. A mug three hundred yards away with a
/// `VisibilityRange` still pays the queue and rebuild cost although it can
/// never cover a pixel. A resident holds a placement as plain data, which no
/// render system walks, until the player is within the distance at which its
/// range would first draw it.
pub struct Resident {
    transform: Transform,
    /// The dressed model's draws: handle pairs shared with the cache.
    draws: Vec<ModelDraw>,
    /// The lights this placement carries, in world space — see
    /// [`crate::render::lamps::StaticLamps`]. `None` for everything that is not
    /// a lamp, which is almost everything.
    lamps: Option<crate::render::lamps::StaticLamps>,
    /// The model's particle emitters, shared with the cache. A brazier's
    /// flame is spawned and despawned with the brazier.
    particles: Option<std::sync::Arc<crate::render::particles::ParticleSet>>,
    /// The model's ribbon trails, streamed with the same placement.
    ribbons: Option<std::sync::Arc<crate::render::ribbons::RibbonSet>>,
    /// The packed room light and sun scale, or 0 — see `models::instance_tag`.
    tag: u32,
    /// `draw_range` of the model's own radius at this placement's scale: the
    /// `VisibilityRange` cutoff, and now also the spawn distance.
    range: f32,
    unique_id: u32,
    /// The batch entities while in range; empty while parked.
    spawned: Vec<Entity>,
    /// The animation rig for an animated placement; `None` for almost every
    /// placement. See [`SceneryRig`].
    rig: Option<SceneryRig>,
}

/// Everything an animated doodad needs to be posed, resolved once at load.
///
/// Held on the [`Resident`] rather than looked up per frame because all three
/// parts are fixed once the model has loaded: which clip this placement plays
/// (`vale_assets::look::scenery`), the skeleton to sample it on, and the
/// skinned build of the geometry attached to that skeleton.
#[derive(Clone)]
struct SceneryRig {
    skeleton: std::sync::Arc<vale_assets::world::m2::M2Skeleton>,
    /// This placement's random seed. `vale_assets::look::scenery::seed` says
    /// why it cannot be the `MODD` id.
    seed: u32,
    /// `bones + 1`. The extra joint is the identity joint that weightless
    /// vertices are bound to, as for an entity's rig.
    joint_count: usize,
    inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    /// The skinned build's draws. A different build of the same model from
    /// [`Resident::draws`], which stays the unskinned one — see
    /// [`crate::render::models::ModelCache::lookup_scenery`].
    draws: Vec<ModelDraw>,
    /// The model's declared box as a sphere around the origin, computed as
    /// `EntityModel::cull_radius` is. [`pose_scenery`]'s frustum test uses
    /// this volume, so a rig is never skipped while any of its parts could be
    /// drawn. A model with no box gets a radius that always poses.
    cull_radius: f32,
}

/// Every rigged placement is posed from the frame it is spawned.
///
/// There is no separate posing range. A sixty-yard range was tried, because a
/// posed doodad is a skinned draw and this renderer is bound by per-draw-call
/// encoding. It needed a second distance, a hysteresis band, and a despawn and
/// respawn when the player crossed it, and it saved no measurable time. It
/// also left distant animated scenery still: the Great Forge did not move until
/// the player was within sixty yards. It was removed.
///
/// The draw range decides whether a placement is posed: it is posed only while
/// spawned, and spawned only while the player is within its draw range. Beyond
/// that there is no rig, no joints and no skinned draw. Within it, the frustum
/// decides: Bevy's culling skips drawing a rig the camera cannot see, and
/// [`pose_scenery`] skips the pose on the same sphere. The clip's clock still
/// advances, so a rig that comes into view resumes mid-cycle.
///
/// If a limit is ever needed, it should be a count (the nearest N) rather than
/// a distance, because animated doodads cluster in small areas such as a
/// courtyard. No measurement so far shows a limit is needed.

/// The loaded placements of one tile, spawned and despawned by
/// [`stream_doodads`] as the player moves.
///
/// A component of the tile, so despawning the tile drops the whole list with
/// it. A parked resident needs no despawn, and a spawned one is a child of the
/// same tile.
#[derive(Component, Default)]
pub struct ResidentDoodads {
    list: Vec<Resident>,
    /// Set when residents are added, so the next stream pass runs even if the
    /// player has not moved.
    dirty: bool,
}

impl ResidentDoodads {
    /// Moves a placement that may already be spawned.
    ///
    /// A resident holds the transform its batches are spawned with. A caller
    /// that writes only the spawned entities moves the placement until it is
    /// parked and spawned again, when it returns to the file's position. This
    /// method updates the resident so the move persists.
    ///
    /// Returns `false` for a `unique_id` this tile has no resident for, which
    /// is the normal result for eight of the nine tiles.
    ///
    /// Nothing in this crate calls it: a placement's transform comes from the
    /// file, and files do not change during a session. It exists for a host
    /// that changes them.
    pub fn reposition(&mut self, unique_id: u32, transform: Transform) -> bool {
        let Some(resident) = self
            .list
            .iter_mut()
            .find(|resident| resident.unique_id == unique_id)
        else {
            return false;
        };
        resident.transform = transform;
        true
    }

    /// The resident's current transform, for a caller that moves a placement
    /// relative to its last position.
    pub fn placement_of(&self, unique_id: u32) -> Option<Transform> {
        self.list
            .iter()
            .find(|resident| resident.unique_id == unique_id)
            .map(|resident| resident.transform)
    }
}

/// Counts of what a tile has drawn, used to check the seam rule.
///
/// `vale models <Map> <x> <y>` lists every placement the tile's `MDDF` names,
/// including those overhanging from a neighbour. The renderer claims only
/// those whose origin is on this tile, so the claimed count should be slightly
/// under the listed count and never over. A count of batches is nearly double
/// and cannot be used for this check. (The HUD used to show the batch count.)
#[derive(Component, Default)]
pub struct TileDoodads {
    /// `MDDF` placements this tile claimed; the number to compare.
    pub placements: usize,
    /// `MODD` furniture handed over by the WMO pass, which is in no `MDDF`.
    pub furniture: usize,
    /// Entities spawned for both, one per batch.
    pub batches: usize,
    /// How many of those placements were also solid, with a hull placed into
    /// [`Solids`]. Always well under `placements + furniture`: 93 of
    /// Darkshire's 128 models carry a hull but only 931 of its 1,391
    /// placements do, and Stormwind's tile has 80 of 219 outdoors and 3,732
    /// of 6,057 indoors. `vale collision <Map> <x> <y>` prints both sides.
    pub solid: usize,
    /// How many of this tile's placements carry a rig, and how many of those
    /// are posed this frame. See [`SceneryRig`].
    ///
    /// Two numbers, because they separate two failures that look the same on
    /// screen. `rigged` at zero means the animated build never resolved and
    /// nothing can move. `rigged` high with `posed` at zero means the range is
    /// wrong or the player is not near any rigged placement. The first failure
    /// once went undetected on screen and by 2,271 tests; these counts expose
    /// it.
    pub rigged: usize,
    pub posed: usize,
}

pub struct DoodadPlugin;

impl Plugin for DoodadPlugin {
    fn build(&self, app: &mut App) {
        // Runs after the WMO pass, with the order declared explicitly. A
        // building hands its interior `MODD` furniture to this pass through
        // the tile's [`PendingDoodads`], and both systems take that component
        // mutably, so Bevy never runs them at the same time and one of them
        // goes first. Without `.after`, that order depends on the conflicting
        // access: if one system stopped taking `PendingDoodads` mutably, the
        // two would run in parallel in whatever order the scheduler picks.
        //
        // Nothing would fail. Furniture handed over after this pass has run is
        // spawned on the next frame instead, one frame late. The same class of
        // one-frame ordering bug has occurred twice before (`camera::place`
        // against `follow_player`, and a joint against its attachment). In both,
        // a value existed in two versions for one stage of the frame, nothing
        // was logged, and the symptom looked like a different bug.
        app.add_systems(
            Update,
            (
                resolve_doodads.after(crate::render::wmos::spawn_wmos),
                stream_doodads.after(resolve_doodads),
                // Doodads stream, so a placement that comes into range while
                // the switch is off is spawned visible. Hiding only when the
                // switch is clicked would let newly spawned doodads appear as
                // the character walks. [`crate::render::tuning::switch`] has a
                // second `Added` pass that hides them.
                crate::render::tuning::switch::<Doodad>(|tuning| tuning.doodads),
                // Runs after the spawn, with the order declared explicitly. A
                // rig spawned this frame has identity joints until something
                // writes them, and a skinned mesh whose joints are all identity
                // draws the model collapsed to its origin for that frame. See
                // [`pose_scenery`].
                pose_scenery.after(stream_doodads),
            ),
        );
    }
}

/// Resolves placements whose model is ready, up to the frame's budget: places
/// the hull and records the resident. [`stream_doodads`] does the spawning.
fn resolve_doodads(
    mut cache: ResMut<ModelCache>,
    // A doodad in a room is a second dressing of one model (same meshes,
    // different materials), so this pass needs the material store, as the
    // entity pass does. An outdoor placement never uses it.
    mut materials: Materials,
    mut tiles: Query<(
        &TerrainTile,
        &mut PendingDoodads,
        &mut TileDoodads,
        &mut ResidentDoodads,
    )>,
    focus: Res<crate::render::focus::WorldFocus>,
    solids: Res<Solids>,
    // Used only by the model cache to build a dressing's merged meshes; see
    // `models::loader::MergeSource`.
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Doodads);
    // No map means nothing is being drawn, and a hull keyed to the wrong map
    // becomes an invisible wall elsewhere. Drawing does not need the map, but
    // waiting costs at most a frame: the tiles exist only because something
    // set the focus.
    if !focus.present {
        return;
    }
    let map_id = focus.map_id;
    let mut budget = RESOLVE_BUDGET;
    for (coord, mut pending, mut drawn, mut residents) in &mut tiles {
        if budget == 0 {
            return;
        }
        let coord = coord.coord;
        let pending = &mut *pending;
        let drawn = &mut *drawn;
        let residents = &mut *residents;
        for indoor in [false, true] {
            let (list, claimed) = if indoor {
                (&mut pending.indoor, &mut drawn.furniture)
            } else {
                (&mut pending.outdoor, &mut drawn.placements)
            };
            list.retain(|doodad| {
                if budget == 0 {
                    return true;
                }
                let placement = &doodad.placement;
                let model = match cache.lookup_in_room(
                    &placement.path,
                    doodad.light,
                    &mut meshes,
                    &mut materials,
                ) {
                    Lookup::Ready(model) => model,
                    // Still reading it: keep it and try again next frame.
                    Lookup::Loading => return true,
                    // Reported once by the loader; nothing per placement.
                    Lookup::Failed => return false,
                };
                // The animation rig, resolved before anything below has a side
                // effect.
                //
                // The skinned build is a separate cache entry, and nothing else
                // in the client requests one for a doodad, so the first call
                // always returns `Loading`: it files the request and answers
                // next frame. Treating `Loading` as "no rig" and resolving the
                // placement anyway gives every doodad `rig: None` permanently,
                // because the placement leaves the pending list that frame and
                // is never reconsidered. An earlier version did this, and no
                // doodad animated while every test passed.
                //
                // The placement is therefore held pending, as for the
                // unskinned build. An animated placement resolves one frame
                // later.
                let rig = match model
                    .skeleton
                    .as_ref()
                    .filter(|skeleton| vale_assets::look::scenery::animates(skeleton))
                    .map(|skeleton| {
                        // Seeded from the id and the position. A `MODD` spawn
                        // carries its building's id, so ten gryphon roosts in
                        // one hall share it; seeding from the id alone made
                        // them move in unison. See `look::scenery::seed`.
                        let at = Mat4::from_cols_array(&placement.matrix).w_axis;
                        let seed = vale_assets::look::scenery::seed(
                            placement.unique_id,
                            [at.x, at.y, at.z],
                        );
                        (Arc::clone(skeleton), seed)
                    }) {
                    None => None,
                    Some((skeleton, seed)) => match cache.lookup_scenery(
                        &placement.path,
                        doodad.light,
                        true,
                        &mut meshes,
                        &mut materials,
                    ) {
                        Lookup::Ready(posed) => Some(SceneryRig {
                            skeleton,
                            seed,
                            joint_count: posed.joint_count,
                            inverse_bindposes: posed.inverse_bindposes.clone(),
                            draws: posed.draws.clone(),
                            // The same expression `world::entities::spawn` uses
                            // for `EntityModel::cull_radius`. The two frustum
                            // tests must use the same volume.
                            cull_radius: model
                                .bounds
                                .map(|b| {
                                    Vec3::from(b.center).length()
                                        + Vec3::from(b.half_extents).length()
                                })
                                .unwrap_or(f32::MAX),
                        }),
                        // Still reading it: keep the placement and try again,
                        // before it has changed anything.
                        Lookup::Loading => return true,
                        // The unskinned build loaded, so only the skinned build
                        // failed. The placement is drawn without animation.
                        Lookup::Failed => None,
                    },
                };
                let rig_resolved = rig.is_some();
                budget -= 1;

                // Collision is placed before the draw checks. A model can carry
                // a hull and no visible batch, and it must still block a
                // stride, so this cannot follow the `draws.is_empty()` return
                // below. Hulls do not stream: collision does not depend on what
                // is on screen, so the hull is placed once, here, and lasts as
                // long as the tile.
                //
                // Not every model has a hull. 93 of Darkshire's 128 carry one,
                // and 16 of Stormwind's 22. The rest are grass, birds and flames,
                // which the player walks through.
                if !model.collision.is_empty() {
                    // A `MODD` spawn has no id of its own and carries its
                    // building's, so indoor placements need the ordinal — see
                    // [`SolidId`]. `drawn.solid` is a per-tile counter, which is
                    // sufficient as the ordinal.
                    let id = if indoor {
                        SolidId::spawn(placement.unique_id, drawn.solid as u32)
                    } else {
                        SolidId::placement(placement.unique_id)
                    };
                    let collider = Collider::place(&model.collision, &placement.matrix);
                    solids
                        .0
                        .insert(map_id, coord, id, Solid::Doodad, Arc::new(collider));
                    drawn.solid += 1;
                }

                // A model with no visible batches can still have emitters (a
                // placed glow is one), so only a model with neither stops
                // here.
                if model.draws.is_empty() && model.particles.is_none() && model.ribbons.is_none() {
                    return false;
                }

                // `placement.matrix` is composed in the file's own frame,
                // including the 180° term. Conjugating it carries that
                // derivation into Bevy's frame without re-deriving it here. It
                // is a rotation and a uniform scale, so the decomposition into
                // a `Transform` is exact.
                let transform = Transform::from_matrix(axes::to_bevy_affine(
                    Mat4::from_cols_array(&placement.matrix),
                ));

                *claimed += 1;
                drawn.batches += model.draws.len();

                // The model's own declared radius at this placement's scale.
                // The cull uses the same volume, so the two agree on the
                // model's size.
                let radius = model
                    .bounds
                    .map(|b| Vec3::from(b.half_extents).length() * placement.scale)
                    .unwrap_or(f32::MAX);

                // The placement's baked colour and sun scale are stored on
                // the instance: the room-lit material selects the shader
                // branch, and that branch reads the tag. See `RoomLight` and
                // `sun_scale`. An absent tag reads as zero, which the shader
                // treats as "no room light, neutral sun", so only instances
                // that differ from that carry the component.
                residents.list.push(Resident {
                    transform,
                    rig,
                    // The lights this placement emits, resolved once here
                    // rather than every frame: static scenery does not move,
                    // so a lamppost's glow keeps the same position for as long
                    // as the tile is loaded. `None` for almost every model;
                    // see `render::lamps`.
                    lamps: crate::render::lamps::StaticLamps::of(&model.glows, &transform),
                    draws: model.draws.clone(),
                    particles: model.particles.clone(),
                    ribbons: model.ribbons.clone(),
                    tag: crate::render::models::instance_tag(doodad.light, doodad.sun_scale),
                    range: draw_range(radius),
                    unique_id: placement.unique_id,
                    spawned: Vec::new(),
                });
                if rig_resolved {
                    drawn.rigged += 1;
                }
                residents.dirty = true;
                false
            });
        }
    }
}

/// A joint of an animated doodad's rig. It is the equivalent of the marker an
/// entity's joints carry, for the same reason: [`pose_scenery`] writes
/// `GlobalTransform` directly onto them, so queries must be able to find them
/// and to exclude them from other queries.
///
/// The index is the joint's position in its rig's [`PosedDoodad::posed`]
/// list: the bone index, or one past the last bone for the identity joint.
#[derive(Component)]
pub struct SceneryJoint(u32);

/// The root of one animated placement: the placement's transform, the joints
/// under it, and what it is playing.
///
/// A component rather than a resource keyed by entity, so that despawning the
/// root, which happens when a placement streams out of range, removes the
/// whole rig and leaves nothing to reconcile.
#[derive(Component)]
pub struct PosedDoodad {
    skeleton: std::sync::Arc<vale_assets::world::m2::M2Skeleton>,
    /// The clip currently playing. [`pose_scenery`] picks a new one at random
    /// each time it ends, so a gryphon roost idles, stretches, and idles again
    /// instead of looping one animation.
    clip: vale_assets::look::scenery::Clip,
    /// When it started, on the same clock `pose_scenery` reads.
    started: f32,
    /// The random value the next pick is made from. Advanced per pick so a
    /// placement's successive clips are as unrelated as two placements' first.
    roll: u32,
    /// [`SceneryRig::cull_radius`], copied here for [`pose_scenery`]'s
    /// frustum test.
    cull_radius: f32,
    /// This placement's key hints for `M2Skeleton::pose_into`, kept between
    /// frames so that each track sample starts its key search where the last
    /// one ended.
    hints: Vec<u32>,
    /// The world matrix of every joint as [`pose_scenery`] last posed it: the
    /// bones in order, then the identity joint. Each [`SceneryJoint`] reads its
    /// own row.
    posed: Vec<GlobalTransform>,
    /// Whether [`pose_scenery`] posed this rig this frame. False when the
    /// frustum test skipped it, in which case its joints are not written.
    fresh: bool,
}

/// Builds one placement's rig: a root at the placement, and a joint per bone.
///
/// Returns the root, which the caller parents the skinned batches to and tracks
/// for the despawn.
fn spawn_posed(
    commands: &mut Commands,
    tile: Entity,
    resident: &mut Resident,
    rig: &SceneryRig,
    joints_out: &mut Vec<Entity>,
    now: f32,
) -> Entity {
    let root = commands.spawn((resident.transform, Visibility::default(), ChildOf(tile))).id();
    joints_out.clear();
    joints_out.extend((0..rig.joint_count).map(|index| {
        // No `Transform`: `pose_scenery` writes the `GlobalTransform` itself,
        // and transform propagation would overwrite it from a `Transform`
        // every frame. An entity's joints are built the same way.
        commands
            .spawn((SceneryJoint(index as u32), GlobalTransform::default(), ChildOf(root)))
            .id()
    }));
    // Starts on a clip picked from its own seed, part-way through. Both keep
    // the placements in a courtyard out of step: the clip differs per
    // placement, and so does how far through it each one starts.
    let clip = vale_assets::look::scenery::pick(&rig.skeleton, rig.seed)
        .expect("a rig is only built for a model that animates");
    let into = (rig.seed % clip.duration_ms.max(1)) as f32 / 1000.0;
    commands.entity(root).insert(PosedDoodad {
        skeleton: std::sync::Arc::clone(&rig.skeleton),
        clip,
        started: now - into,
        roll: vale_assets::look::scenery::next_roll(rig.seed),
        cull_radius: rig.cull_radius,
        hints: Vec::new(),
        posed: Vec::with_capacity(joints_out.len()),
        fresh: false,
    });
    resident.spawned.push(root);
    root
}

/// Samples every spawned scenery rig and writes its joints. This is the
/// rendering side of [`vale_assets::look::scenery`].
///
/// One `M2Skeleton::pose` per posed placement per frame, the same call an
/// entity makes. The draw range has already limited the rigs to those near
/// the player, and the frustum test below limits them to those the camera
/// can see.
///
/// Two separate clocks are used. The clip runs on the placement's own elapsed
/// time (wall clock plus its phase, wrapped by the clip's length). The
/// model's global sequences run on the wall clock directly, which is what
/// `pose`'s second argument is for. A torch's flicker is a global sequence;
/// giving it the phased clock would invent a per-instance clock the file does
/// not have. See the module note in `look::scenery`.
///
/// The camera is passed so that bones flagged `0x8` are billboarded. Most
/// animated doodads depend on this: a flame is a spherical billboard, and
/// without billboarding every torch is a flat card, seen edge-on from half
/// the possible viewing angles.
pub(crate) fn pose_scenery(
    time: Res<Time>,
    tuning: Res<crate::render::tuning::WorldTuning>,
    // `Without<SceneryJoint>` is on both reading queries, not on the writing
    // one. Bevy proves two queries disjoint from their filters alone, and it
    // cannot know that a camera is not a joint, so a plain `With<WorldCamera>`
    // beside a `&mut GlobalTransform` is error `B0001` at schedule build: a
    // panic on the first frame, not a compile error. The entity pass filters
    // its camera and entity queries the same way; see
    // `world::entities::pose::animate`.
    camera: Query<
        (&GlobalTransform, &bevy::camera::primitives::Frustum),
        (With<crate::world::camera::WorldCamera>, Without<SceneryJoint>),
    >,
    mut rigs: Query<(&mut PosedDoodad, &GlobalTransform), Without<SceneryJoint>>,
    mut joints: Query<(&SceneryJoint, &ChildOf, &mut GlobalTransform), Without<PosedDoodad>>,
    // One pose buffer per thread, kept across frames for its capacity.
    scratch: Local<bevy::utils::Parallel<Vec<[f32; 12]>>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::SceneryPose);
    if !tuning.doodad_animation {
        return;
    }
    let now = time.elapsed_secs();
    let now_ms = (now * 1000.0) as u32;
    // The camera's right and up vectors in world space. `pose` takes them in
    // model space, so each rig transforms them by its own inverse placement
    // below.
    let camera = camera.iter().next().map(|(placement, frustum)| (*placement, frustum.clone()));
    let view = camera.as_ref().map(|(c, _)| c.affine());
    // The pool the parallel passes run on, initialised here as well as by
    // `TaskPoolPlugin` because a headless harness builds no such plugin.
    bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::default);
    let scratch: &bevy::utils::Parallel<Vec<[f32; 12]>> = &scratch;

    // Each rig is posed into its own `posed` list, in parallel: a pose reads
    // only the rig, the clock and the camera. The joints are written by the
    // second pass below, because a joint is another entity and a `Query`
    // cannot be written from inside `par_iter_mut`.
    rigs.par_iter_mut().for_each(|(mut rig, placement)| {
        let rig = &mut *rig;
        rig.fresh = false;
        // A clip that has ended is replaced by a new weighted random pick, not
        // looped, as in the 1.12.1 client. Without it every placement of a
        // model would move in the same cycle. A gryphon roost idles three
        // times in five and otherwise plays one of four other animations, by
        // the file's own weights. See `vale_assets::look::scenery::pick`.
        //
        // A model with one row picks the same row again, which loops it (a
        // torch's flame), at the cost of one weighted pick per clip length.
        let elapsed = now - rig.started;
        let length = rig.clip.duration_ms as f32 / 1000.0;
        if elapsed >= length {
            let roll = rig.roll;
            if let Some(clip) = vale_assets::look::scenery::pick(&rig.skeleton, roll) {
                rig.clip = clip;
            }
            rig.roll = vale_assets::look::scenery::next_roll(roll);
            // From `now` rather than from `started + length`: a rig that has
            // been off screen for a minute would otherwise re-roll once per
            // clip length to catch up, all in one frame.
            rig.started = now;
        }
        let sequence = rig.clip.sequence;
        let elapsed_ms = ((now - rig.started).max(0.0) * 1000.0) as u32;
        let world_from_placement = placement.affine();
        // A rig the camera cannot see is not posed. The test, its guarantee
        // and its sphere are the same as in `world::entities::pose::animate`.
        // The clocks above have already advanced, so a rig entering the
        // frustum resumes mid-cycle and is posed on the frame the test passes.
        // The scale is the largest of the placement's three axis lengths,
        // which is an upper bound however the matrix is composed. The far
        // plane is not tested, which errs toward posing.
        // `VALE_NO_SCENERY_CULL=1` disables the test, so its saving can be
        // measured like every other one.
        if scenery_culling() {
            if let Some((_, frustum)) = &camera {
                let scale = world_from_placement
                    .x_axis
                    .length()
                    .max(world_from_placement.y_axis.length())
                    .max(world_from_placement.z_axis.length());
                let sphere = bevy::camera::primitives::Sphere {
                    center: world_from_placement.translation,
                    radius: rig.cull_radius * scale,
                };
                if !frustum.intersects_sphere(&sphere, false) {
                    return;
                }
            }
        }
        let billboard = view.map(|view| {
            let to_model = world_from_placement.inverse();
            let right = to_model.transform_vector3(view.x_axis.into());
            let up = to_model.transform_vector3(view.y_axis.into());
            (right.to_array(), up.to_array())
        });
        scratch.scope(|pose| {
            rig.skeleton.pose_into(
                sequence,
                elapsed_ms,
                now_ms,
                billboard,
                Default::default(),
                pose,
                &mut rig.hints,
            );
            rig.posed.clear();
            rig.posed.extend(pose.iter().map(|bone| {
                GlobalTransform::from(world_from_placement * axes::pose_to_bevy_affine(bone))
            }));
        });
        // The identity joint that weightless vertices are bound to: the
        // placement itself, unposed. Without it they collapse to the world
        // origin.
        rig.posed.push(GlobalTransform::from(world_from_placement));
        rig.fresh = true;
    });

    // Each joint copies its row of its rig's `posed` list, in parallel. A rig
    // the frustum test skipped is not `fresh`, and its joints are left
    // unwritten so that their skins are not re-uploaded.
    joints.par_iter_mut().for_each(|(joint, child_of, mut transform)| {
        let Ok((rig, _)) = rigs.get(child_of.parent()) else {
            return;
        };
        if !rig.fresh {
            return;
        }
        if let Some(world) = rig.posed.get(joint.0 as usize) {
            *transform = *world;
        }
    });
}

/// Whether an off-screen rig's pose is skipped; see the frustum test in
/// [`pose_scenery`].
///
/// A kill switch, like the others, so the saving can be measured by turning it
/// off. With `VALE_NO_SCENERY_CULL=1`, every spawned rig is posed every frame.
fn scenery_culling() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VALE_NO_SCENERY_CULL").is_none())
}

/// Whether a resident at `distance²` from the player should have entities,
/// given whether it currently does.
///
/// The spawn and despawn distances differ. A parked resident spawns a margin
/// before its `VisibilityRange` cutoff, so the entity exists by the time the
/// camera could first see it. The range still decides which pixels are drawn,
/// so objects appear and disappear at the same distance as when everything
/// was spawned. A spawned resident is kept until a second margin past that,
/// so a player moving back and forth on one spot does not repeatedly spawn
/// and despawn the ring at the spawn distance.
fn wants_spawned(d2: f32, range: f32, spawned: bool) -> bool {
    let edge = if spawned {
        range + 2.0 * STREAM_MARGIN
    } else {
        range + STREAM_MARGIN
    };
    d2 < edge * edge
}

/// Spawns the residents the player has come within range of, and despawns
/// those out of range.
///
/// Runs when the player has moved [`STREAM_STEP`] yards or a tile has new
/// residents, and walks every resident of the 3x3: one distance each, tens of
/// microseconds for a city. Spawns are budgeted; a pass cut short by the
/// budget leaves the moved-marker unset so the next frame resumes.
pub(crate) fn stream_doodads(
    mut commands: Commands,
    focus: Res<crate::render::focus::WorldFocus>,
    // The clock, used only for a rig's starting phase. A rig starts part-way
    // into its clip so that two placements of one model are not in step; see
    // `vale_assets::look::scenery`, and `spawn_posed`, its only reader.
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut tiles: Query<(Entity, &mut ResidentDoodads, &mut TileDoodads)>,
    mut last: Local<Option<Vec3>>,
    // One scratch buffer for the whole pass rather than a `Vec` per rig: the
    // joints are copied onto the `PosedDoodad` and the list is reused, so
    // spawning a whole city allocates once.
    mut rig_joints: Local<Vec<Entity>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Doodads);
    if !focus.present {
        return;
    }
    let now = time.elapsed_secs();
    let player = axes::to_bevy(focus.position.to_array());
    let moved =
        last.map_or(true, |p| p.distance_squared(player) >= STREAM_STEP * STREAM_STEP);
    if !moved && !tiles.iter().any(|(_, r, _)| r.dirty) {
        return;
    }

    let mut budget = STREAM_BUDGET;
    let mut complete = true;
    for (tile, mut residents, mut drawn) in &mut tiles {
        let residents = &mut *residents;
        for resident in &mut residents.list {
            let d2 = resident.transform.translation.distance_squared(player);
            let want = wants_spawned(d2, resident.range, !resident.spawned.is_empty());
            if want && resident.spawned.is_empty() {
                if budget == 0 {
                    complete = false;
                    continue;
                }
                budget -= 1;
                // Any rigged placement is posed, with no second distance: a
                // placement reaches this point only when it is within draw
                // range. See [`spawn_posed`] and the note above
                // [`ResidentDoodads`].
                let posed = resident.rig.clone().map(|rig| {
                    spawn_posed(&mut commands, tile, resident, &rig, &mut rig_joints, now)
                });
                let range = VisibilityRange::abrupt(0.0, resident.range);
                // A posed placement parents its batches to its own rig root; a
                // static one parents them to the tile. Their transforms differ
                // because of how Bevy skins: the joint matrices replace a
                // skinned mesh's world matrix, so a posed batch's own transform
                // is never read and the placement must be in the joints. Giving
                // a posed batch the placement transform as well would have no
                // visible effect, so the difference is easy to miss. See
                // `spawn_posed`.
                let (parent, batch_at, draws) = match (&posed, &resident.rig) {
                    (Some(root), Some(rig)) => (*root, Transform::default(), &rig.draws),
                    _ => (tile, resident.transform, &resident.draws),
                };
                let mut first = true;
                for draw in draws {
                    let mut batch = commands.spawn((
                        Doodad {
                            unique_id: resident.unique_id,
                        },
                        Mesh3d(draw.mesh.clone()),
                        MeshMaterial3d(draw.material.clone()),
                        batch_at,
                        range.clone(),
                        // A child of the tile, so despawning the tile
                        // despawns its doodads; a parked resident goes with
                        // the component. For a posed placement the parent is
                        // the rig root, which is itself a child of the tile.
                        ChildOf(parent),
                    ));
                    // The batch's own bones, not the model's whole skeleton.
                    // `models::skin_for` makes that decision for all five
                    // callers, this one included.
                    if let Some(rig) = posed.as_ref().and(resident.rig.as_ref()) {
                        if let Some(joints) = crate::render::models::skin_for(draw, &rig_joints) {
                            batch.insert(SkinnedMesh {
                                inverse_bindposes: rig.inverse_bindposes.clone(),
                                joints,
                            });
                        }
                    }
                    // A baked tint takes precedence over the room light. A
                    // batch with a tint is drawn through a material
                    // specialised as tinted, which reads the tag as
                    // `0xAARRGGBB` rather than as a room colour and a sun
                    // scale. Every batch with a tint is `unlit`, and unlit
                    // batches use neither value the room-light tag carries, so
                    // preferring the tint loses nothing. See
                    // `ModelDraw::baked_tint`, and the Gadgetzan light cone in
                    // `loader`, which is authored at 16% opacity and was drawn
                    // at 100% without the tint.
                    match (draw.baked_tint, resident.tag) {
                        (Some(tint), _) => {
                            batch.insert(bevy::mesh::MeshTag(tint));
                        }
                        (None, tag) if tag != 0 => {
                            batch.insert(bevy::mesh::MeshTag(tag));
                        }
                        _ => {}
                    }
                    // On the first batch only. The component is a list of
                    // world positions, not a per-mesh property, and a lamppost
                    // drawn in four batches must not be four lamps. It is on a
                    // spawned entity rather than the tile so that despawning
                    // out of range removes the light with the geometry.
                    if let (true, Some(lamps)) = (first, resident.lamps.as_ref()) {
                        batch.insert(lamps.clone());
                    }
                    first = false;
                    // A posed batch is not tracked here. It is a child of the
                    // rig root, which is tracked, and despawning the root
                    // despawns its children; listing both would despawn each
                    // batch twice.
                    if posed.is_none() {
                        resident.spawned.push(batch.id());
                    }
                }
                // The emitters stream with the batches, so an out-of-range
                // brazier has no particles. They are root entities (their
                // meshes are in world space), tracked here for the range
                // despawn and tied to the tile for the tile despawn
                // (`retire_emitters`).
                if let Some(set) = &resident.particles {
                    let placement = resident.transform;
                    resident.spawned.extend(crate::render::particles::spawn_emitters(
                        &mut commands,
                        &mut meshes,
                        set,
                        tile,
                        |_, _| crate::render::particles::Anchor::Fixed(placement),
                        resident.transform.scale.x,
                        Some(resident.range),
                    ));
                }
                // The ribbon trails stream the same way. A doodad is drawn
                // in its bind pose with no joints, so every ribbon on one is
                // anchored to the placement itself.
                if let Some(set) = &resident.ribbons {
                    let placement = resident.transform;
                    resident.spawned.extend(crate::render::ribbons::spawn_ribbons(
                        &mut commands,
                        &mut meshes,
                        set,
                        tile,
                        |_, _| crate::render::particles::Anchor::Fixed(placement),
                        resident.transform.scale.x,
                    ));
                }
            } else if !want && !resident.spawned.is_empty() {
                for entity in resident.spawned.drain(..) {
                    commands.entity(entity).despawn();
                }
            }
        }
        residents.dirty = residents.dirty && !complete;
        // The number of rigs posed now, recounted rather than incremented: the
        // walk above has just decided it for every resident of this tile, and
        // a counter updated at three call sites can drift.
        drawn.posed = residents
            .list
            .iter()
            .filter(|r| r.rig.is_some() && !r.spawned.is_empty())
            .count();
    }
    if complete {
        *last = Some(player);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::adt::{placement_matrix, placement_to_world};

    /// A placement's matrix converts into Bevy's frame as a `Transform`
    /// without loss: it is a rotation and a uniform scale, so the
    /// decomposition is exact and the model's position is the one
    /// `placement_to_world` computed.
    ///
    /// The two parts are each checked elsewhere: `vale wmos` matches the
    /// matrix to the game's own `MODF` boxes to 0.00 yards, and `axes` fixes
    /// the basis. This test checks that combining them loses neither.
    #[test]
    fn a_placement_lands_where_the_file_says() {
        let world = placement_to_world([1000.0, 50.0, 2000.0]);
        let matrix = placement_matrix(world, [0.0, 60.0, 0.0], 2.0);
        let transform =
            Transform::from_matrix(axes::to_bevy_affine(Mat4::from_cols_array(&matrix)));

        let expected = axes::to_bevy(world);
        assert!(
            (transform.translation - expected).length() < 1e-3,
            "{:?} != {expected:?}",
            transform.translation
        );
        assert!(
            (transform.scale - Vec3::splat(2.0)).length() < 1e-4,
            "scale {:?} is not the placement's",
            transform.scale
        );
        // A rotation, not a mirror: a negative determinant here would turn every
        // tree inside out, and the result could still look plausible.
        assert!(transform.rotation.is_normalized());
    }

    /// The draw distance follows the 1.12.1 client's size bands. The three
    /// bands end at 50, 125 and 200 yards of surface distance, so the cull is
    /// the radius past each. A model bigger than 7 yards never distance-fades:
    /// it gets the ceiling past the loaded 3x3's edge, where only the end of
    /// the terrain hides it. A missing bounds volume also gets the ceiling, so
    /// a doodad with no size is drawn, not dropped.
    #[test]
    fn a_doodads_draw_distance_is_the_clients_own_law() {
        assert_eq!(draw_range(0.25), 50.25, "a mug ends the small-prop band");
        assert_eq!(draw_range(1.0), 126.0, "a crate ends the mid band");
        assert_eq!(draw_range(5.0), 205.0, "a statue ends the large band");
        assert_eq!(draw_range(8.0), 800.0, "a tree is ended only by the terrain");
        assert_eq!(draw_range(f32::MAX), 800.0, "no bounds is drawn, not dropped");
        // The bucket edges are monotonic: a bigger model is never culled
        // sooner than a smaller one, including across each boundary.
        for (small, large) in [(0.5, 0.51), (2.5, 2.51), (7.0, 7.01)] {
            assert!(
                draw_range(small) <= draw_range(large),
                "bigger is never culled sooner across the {small} boundary"
            );
        }
    }

    /// The spawn and despawn edges both lie outside the draw range, and apart.
    /// A parked resident must exist before the camera reaches the
    /// `VisibilityRange` cutoff; an entity spawned exactly at the range would
    /// appear a frame late every time. A spawned one must remain past the
    /// spawn edge, or a player moving back and forth on one spot spawns and
    /// despawns the whole boundary ring twice a step.
    #[test]
    fn the_streaming_edges_bracket_the_range_and_do_not_meet() {
        let range = 150.0;
        let d2 = |d: f32| d * d;
        // Inside the range itself: wanted whichever state it is in.
        assert!(wants_spawned(d2(100.0), range, false));
        assert!(wants_spawned(d2(100.0), range, true));
        // Between the range and the spawn edge: still spawned ahead of arrival.
        assert!(wants_spawned(d2(range + STREAM_MARGIN * 0.5), range, false));
        // Between the two edges (the hysteresis band): a parked resident stays
        // parked and a spawned one stays spawned.
        let between = d2(range + STREAM_MARGIN * 1.5);
        assert!(!wants_spawned(between, range, false));
        assert!(wants_spawned(between, range, true));
        // Past both edges: gone either way.
        let beyond = d2(range + STREAM_MARGIN * 2.5);
        assert!(!wants_spawned(beyond, range, false));
        assert!(!wants_spawned(beyond, range, true));
    }

    /// The model's origin is at the placement's position, and a point a yard
    /// up the model's Z is a yard up in the world. This fails if the vertices
    /// and the matrix use different frames.
    #[test]
    fn the_models_own_up_stays_up() {
        let world = placement_to_world([1000.0, 50.0, 2000.0]);
        let matrix = placement_matrix(world, [0.0, 0.0, 0.0], 1.0);
        let transform =
            Transform::from_matrix(axes::to_bevy_affine(Mat4::from_cols_array(&matrix)));

        // A vertex one yard up in model space, converted the way `batch_draw`
        // converts one.
        let vertex = axes::to_bevy([0.0, 0.0, 1.0]);
        let placed = transform.transform_point(vertex);
        let lifted = placed - axes::to_bevy(world);
        assert!(
            (lifted - Vec3::Y).length() < 1e-4,
            "a yard up the model came out at {lifted:?}"
        );
    }


}
