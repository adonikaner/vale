//! The doodad pass: every M2 the terrain tiles place.
//!
//! A tile is ~1,400 `MDDF` placements of ~130 distinct models, and the 3x3
//! around the character is ~12,000 of them. `models.rs` turns a path into a list
//! of (mesh, material) draws; this turns a placement into entities carrying
//! them.
//!
//! **One entity per placement per batch — but only while the player is near
//! enough to ever see it.** The WebGL renderer had to group instances by model
//! across every loaded tile and rebuild the buffers whenever the tile set
//! changed, because one draw call per placement was tens of thousands a frame.
//! Bevy batches by mesh and material for free — and, unlike the hand-built
//! buffers, culls each placement individually.
//!
//! The *spawning* is by distance now, in two stages: [`resolve_doodads`] turns
//! a placement whose model has loaded into a [`Resident`] — hull placed,
//! transform composed, draws in hand — and [`stream_doodads`] spawns and
//! despawns the entities as the player crosses each resident's own draw range.
//! The reason is the CPU-floor measurement: the
//! per-frame render cost scales with **spawned** meshes, not visible ones, so
//! a `VisibilityRange` alone leaves a city's six thousand mugs paying the
//! binned-phase rebuild from three hundred yards away. The range still decides
//! the pixel — streaming spawns a margin *before* it — so nothing pops at a
//! different distance than it did when everything was spawned.
//!
//! ## A doodad belongs to the tile its origin is on
//!
//! An object touching two tiles is listed in **both** tiles' `MDDF` with the
//! same `unique_id`, and drawing it twice doubles the cost of every tree along
//! every seam and z-fights on its leaves. The WebGL renderer deduplicated with a
//! set of ids rebuilt whenever the loaded tile set changed; here each placement
//! is simply assigned to the tile containing its own origin, which needs no
//! shared state and survives a tile being dropped — a doodad is a child of
//! exactly one tile entity, and despawning that tile takes it with it.
//!
//! The cost is a ~50-yard band at the outer edge of the loaded world where an
//! overhanging object whose origin is on an unloaded tile is not drawn. That is
//! the same band the terrain stops at.
//!
//! ## …and the few that move, move
//!
//! Most of the world is still, and this pass drew all of it that way for a long
//! time. But 17 of a Darkshire tile's 128 models carry a skeleton and 38 of the
//! 300 in Stormwind's `MODD` sets do — the bellows, the gryphon roosts, a
//! windmill, and the torches whose flame is a billboarded bone.
//!
//! A placement near enough to matter takes the model's **skinned** build and
//! gets a rig: a root at the placement, a joint per bone, and
//! [`pose_scenery`] writing `placement × bone` onto them each frame. Which clip
//! it plays and at what phase is `vale_assets::look::scenery`'s, which is a
//! rule and is unit-tested with no renderer running.
//!
//! **Everything with a rig is posed while it is drawn.** There was a second,
//! shorter range here; it is gone, and the note where it stood says why.
//!
//! ## …and a doodad is solid, on the same terms a building is
//!
//! Each placement whose model carries a `BoundingTriangles` hull also goes into
//! [`Solids`], so a tree stops a stride and a crate is stood on. It is the same
//! [`Collider`] and the same two queries a `MODF` building has used since
//! collision existed — see [`vale_assets::world::collision`] — and the hull rides
//! along with the model in [`crate::render::models::ModelAssets`], so a placement costs
//! only the transform.
//!
//! **The population is the whole difference, and it was measured before it was
//! trusted.** A tile is a dozen buildings and up to four thousand solid
//! doodads once a city's furniture is counted, so two things that were free at
//! forty hulls are not at four thousand: the dedup that recognises a repeated
//! placement had to stop being a scan, and the wall query had to learn to
//! reject a hull by its XY box before ray-testing it. With both,
//! `vale collision Azeroth 31 48` puts a floor-plus-stride pair at **67 us
//! against Stormwind's 3,817 hulls and 418,692 triangles** — the mover asks
//! twenty times a second, so the answer is that no distance cull is needed and
//! none is here.

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

/// How many residents to *spawn* in one frame once they come into range.
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
/// The margin is what keeps the entity in the world *before* the camera
/// reaches the [`VisibilityRange`] cutoff — the range still decides the pixel,
/// so popping distance is unchanged from the spawn-everything design — and the
/// wider despawn edge is hysteresis, so a player pacing on one spot does not
/// churn the boundary ring.
const STREAM_MARGIN: f32 = 40.0;

/// The bounding radius above which the real client never distance-fades a
/// doodad — trees and huts are drawn until the far clip drops them.
const NEVER_FADE_RADIUS: f32 = 7.0;

/// Where a doodad past [`NEVER_FADE_RADIUS`] is finally cut off anyway.
///
/// The real client draws these to its far clip; this client has no far-clip
/// wall, so the ceiling stands in for it — past the edge of the loaded 3x3,
/// where the terrain ends the tree before the range does.
const RANGE_CEILING: f32 = 800.0;

/// How far away a doodad is still drawn — **the 1.12 client's own law**, not a
/// heuristic.
///
/// The client fades every world doodad by a band chosen purely from its
/// bounding-sphere radius:
/// radius ≤ 0.5 fades over 40→50 yards, ≤ 2.5 over 100→125, ≤ 7.0 over
/// 150→200, and anything bigger never fades at all. The distance is measured
/// to the *surface* (centre distance minus radius), and a fade that reaches
/// zero is culled outright — not drawn transparent. So a mug is **gone at
/// 50 yards** in the real client, which is the population rule the previous
/// `radius × 60, floor 150` guess missed by 3x: the spawned population is the
/// measured CPU floor (see the module doc), and the floor of that guess kept
/// every candle in a city paying the render world's per-frame walk from three
/// times the distance the game itself would have drawn it.
///
/// Two deviations, both conservative (drawing more, never less):
/// * **the cutoff is abrupt at the band's far end**, where the real client
///   feathers across the band (`fade = 1 − (d − start)/width` into the
///   per-vertex diffuse alpha). Everything the real client draws at all, this
///   draws fully opaque; what is skipped is only the partial transparency.
///   The feather wants a fade channel in the instance tag and a blend-mode
///   twin per material, and is not done here.
/// * **the radius is the AABB half-diagonal at the placement's scale** (see
///   the call site), an upper bound on the bounding-sphere radius the
///   reference scales — a borderline model lands in the longer band.
///
/// Buildings deliberately get **no** range: a WMO is the landmark seen from
/// the next zone over, its batches are mostly opaque and merge under
/// multidraw, and its interiors are what GPU occlusion culling already
/// handles.
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
        // The reference culls where the fade hits zero: `d = centre − radius`
        // past `start + width`, i.e. a centre distance of `start + width + radius`.
        start + width + radius
    };
    match park_probe() {
        Some(cap) => range.min(cap),
        None => range,
    }
}

/// **`VALE_PARK_BEYOND=<yards>` — a measurement probe, never a setting.**
///
/// Clamps every doodad's draw range, which parks (despawns) everything past
/// the cap through the streaming this file already does. It exists to answer
/// one question with one interleaved A/B: *does the spawned population cost
/// the frame anything?* — the sweeps that scale with it (`check_visibility`,
/// the `Aabb` and `Changed` walks) rank in every trace, but the wireframe
/// sweep ranked too and removing it measured null, so the family's cost on
/// the wall cannot be assumed from its self-times. Two runs differing in this
/// variable subtract ~half the spawned batches at once, which is the
/// subtraction no `--without` switch can make: those write `Visibility`, and
/// a hidden entity is still swept.
///
/// It visibly empties the middle distance. That is what a probe may do and a
/// setting may not, which is why it is not in `WorldTuning`.
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
/// **Two lists, because they are two populations.** [`Self::outdoor`] is the
/// tile's own `MDDF` and is what `vale models` lists; [`Self::indoor`] is the
/// `MODD` furniture the WMO pass hands over as each building arrives, which
/// appears in no `MDDF` at all. Summing them gives a number that is larger than
/// the tile lists and cannot be checked against anything — which is what the HUD
/// reported until the entity pass went looking.
#[derive(Component, Default)]
pub struct PendingDoodads {
    pub outdoor: Vec<PlacedDoodad>,
    pub indoor: Vec<PlacedDoodad>,
}

/// A placement and what lights it.
///
/// **The light is the only thing that separates a chair from a tree**, and it is
/// carried per placement rather than looked up because it belongs to the
/// *spawn*: a `MODD` names its own baked colour, so two copies of one barrel in
/// two rooms of one building are lit differently. `None` is the outdoor case —
/// the sun — which is every `MDDF` placement in the world.
pub struct PlacedDoodad {
    pub placement: PlacedModel,
    pub light: Option<RoomLight>,
    /// The per-instance multiplier on the sun term — 2.5 or 0.5 for an `MDDF`
    /// placement by the `MCSH` bit under its origin, 1.0 for a WMO's own
    /// props. See [`crate::render::models::sun_scale`].
    pub sun_scale: f32,
}

impl PlacedDoodad {
    /// An `MDDF` placement, which the sun lights — scaled by whether its
    /// origin stands in the ground's own baked shadow, which is the tile
    /// loader's one [`vale_assets::world::adt::Adt::shadowed_at`] call per
    /// placement.
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

/// One drawn *batch* of one placement.
///
/// Not one placement: a model is one mesh per `M2Batch` and each is its own
/// entity, so a tree with a trunk and a canopy is two of these.
#[derive(Component)]
pub struct Doodad {
    /// `MDDF`'s id, which is what `vale models <Map> <x> <y>` prints.
    pub unique_id: u32,
}

/// A placement whose model has loaded: everything needed to spawn its batches,
/// held so that they are only spawned **within range of the player**.
///
/// This is the "next lever" the CPU-floor measurement named. The per-frame
/// render cost scales with *spawned* meshes, not visible ones — the binned
/// phases are rebuilt and the preprocessing buffers rewritten for the whole
/// spawned population — so a `VisibilityRange` on a mug three hundred yards
/// away still pays queue and rebuild for a thing that can never resolve a
/// pixel. A resident is a placement waiting at the same distance its range
/// would first draw it, as plain data no render system walks.
pub struct Resident {
    transform: Transform,
    /// The dressed model's draws — handle pairs, shared with the cache.
    draws: Vec<ModelDraw>,
    /// The lights this placement carries, in world space — see
    /// [`crate::render::lamps::StaticLamps`]. `None` for everything that is not
    /// a lamp, which is almost everything.
    lamps: Option<crate::render::lamps::StaticLamps>,
    /// The model's particle emitters, shared with the cache — a brazier's
    /// flame streams in and out with the brazier.
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
    /// **What this placement is doing, for the scenery that moves** — `None`
    /// for the great majority of the world. See [`SceneryRig`].
    rig: Option<SceneryRig>,
}

/// Everything an animated doodad needs to be posed, resolved once at load.
///
/// Held on the [`Resident`] rather than looked up per frame because all three
/// halves are settled the moment the model is: which clip this placement plays
/// (`vale_assets::look::scenery`), the skeleton to sample it on, and the
/// skinned build of the geometry to hang off it.
#[derive(Clone)]
struct SceneryRig {
    skeleton: std::sync::Arc<vale_assets::world::m2::M2Skeleton>,
    /// This placement's own roll — see `vale_assets::look::scenery::seed`,
    /// which says why it cannot be the `MODD` id.
    seed: u32,
    /// `bones + 1` — the extra one is the identity joint weightless vertices
    /// ride on, exactly as an entity's is.
    joint_count: usize,
    inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    /// The **skinned** build's draws. A different build of the same model from
    /// [`Resident::draws`], which stays the unskinned one — see
    /// [`crate::render::models::ModelCache::lookup_scenery`].
    draws: Vec<ModelDraw>,
    /// The model's declared box as a sphere from the origin, on the same terms
    /// as `EntityModel::cull_radius` — the volume [`pose_scenery`]'s frustum
    /// gate tests, so a rig is never skipped while any of its parts could draw.
    /// A model with no box gets a radius that always poses, not one that never
    /// does.
    cull_radius: f32,
}

/// **Everything with a rig is posed the moment it is drawn.**
///
/// There was a range here — sixty yards — on the argument that a posed doodad is
/// a skinned draw and this renderer is bound by per-draw-call encode. It was
/// removed after being looked at: the machinery it bought (a second distance, a
/// hysteresis band, and a despawn-and-respawn when the player crossed it) paid
/// for a saving nobody could see, and what it cost was the thing the feature is
/// for — the Great Forge standing still until you walked into it.
///
/// The draw range still decides everything: a placement is only posed while it
/// is spawned, and it is only spawned while the player is near enough to see it.
/// Beyond that there is no rig, no joints and no skinned draw. What is left is
/// the frustum: Bevy's culling already skips *drawing* a rig the camera cannot
/// see, and [`pose_scenery`] skips the pose itself on the same sphere — the
/// clip's clock still advances, so a rig turning into view resumes mid-stride.
///
/// If this ever needs a budget it should be a *count* — the nearest N — rather
/// than a distance, because the population that matters is a courtyard rather
/// than a radius. Nothing measured says it does.

/// The loaded placements of one tile, spawned and despawned by
/// [`stream_doodads`] as the player moves.
///
/// A component of the tile so that walking out of range drops the whole list
/// with the ground it stands on — a parked resident needs no despawn, and a
/// spawned one is a child of the same tile.
#[derive(Component, Default)]
pub struct ResidentDoodads {
    list: Vec<Resident>,
    /// Set when residents are added, so the next stream pass runs even if the
    /// player has not moved.
    dirty: bool,
}

impl ResidentDoodads {
    /// **Move a placement that is already on screen.**
    ///
    /// A resident holds the transform its batches were spawned with, so a caller
    /// that writes the drawn entities alone has moved the thing until the
    /// player walks far enough for it to be parked and spawned again — at which
    /// point it comes back where the file put it. This is the other half.
    ///
    /// `false` for a `unique_id` this tile has no resident for, which is the
    /// ordinary answer for all but one tile of the nine.
    ///
    /// **Nothing in this crate calls it**: a placement's transform is the
    /// file's, and files do not change during a session. It is here for a host
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

    /// …and where it is now, for a caller that wants to move it *relative* to
    /// wherever it was last put.
    pub fn placement_of(&self, unique_id: u32) -> Option<Transform> {
        self.list
            .iter()
            .find(|resident| resident.unique_id == unique_id)
            .map(|resident| resident.transform)
    }
}

/// What a tile has actually drawn, which is the cross-check on the seam rule.
///
/// `vale models <Map> <x> <y>` lists every placement the tile's `MDDF` names,
/// including the ones overhanging from a neighbour; the renderer claims only
/// those whose *origin* is on this tile, so **claimed should always be a little
/// under listed and never over**. Counting batches instead — which is what the
/// HUD did until the entity pass went looking — reads as nearly double and
/// breaks the check without breaking anything on screen.
#[derive(Component, Default)]
pub struct TileDoodads {
    /// `MDDF` placements this tile claimed — the number to compare.
    pub placements: usize,
    /// `MODD` furniture handed over by the WMO pass, which is in no `MDDF`.
    pub furniture: usize,
    /// Entities spawned for both, one per batch.
    pub batches: usize,
    /// How many of those placements were also *solid* — a hull placed into
    /// [`Solids`]. Under `placements + furniture` always, and by a lot: 93 of
    /// Darkshire's 128 models carry a hull but only 931 of its 1,391
    /// placements do, and Stormwind's tile is 80 of 219 outdoors against 3,732
    /// of 6,057 indoors. `vale collision <Map> <x> <y>` prints both sides.
    pub solid: usize,
    /// **How many of this tile's placements carry a rig at all**, and how many
    /// of those are posed *right now* — see [`SceneryRig`].
    ///
    /// Two numbers rather than one, because the two failures they separate look
    /// identical from a chair: `rigged` at zero means the animated build never
    /// resolved and nothing can ever move, and `rigged` high with `posed` at
    /// zero means the range is wrong or the player is nowhere near one. This
    /// pass shipped with the first of those and nothing on screen or in 2,271
    /// tests could tell — which is why the count is here now.
    pub rigged: usize,
    pub posed: usize,
}

pub struct DoodadPlugin;

impl Plugin for DoodadPlugin {
    fn build(&self, app: &mut App) {
        // **After the WMO pass, and stated rather than inherited.** A building
        // hands its interior `MODD` furniture to this pass through the tile's
        // [`PendingDoodads`], and both systems take that component mutably — so
        // Bevy already refuses to run them at the same time and one of them
        // happens to go first. That is an ordering guaranteed by an
        // implementation detail rather than by this schedule: it holds only for
        // as long as both keep the conflicting access, and the day one of them
        // stops taking `PendingDoodads` mutably the two become parallel and the
        // order becomes whatever the scheduler picks that run.
        //
        // Nothing would fail. Furniture handed over after this pass has run is
        // spawned on the *next* frame instead, so the cost is a frame — which is
        // exactly the class of bug this project has paid for twice already
        // (`camera::place` against `follow_player`, and a joint against its
        // attachment). Both were a value that existed in two versions for one
        // stage of the frame, neither logged anything, and both read as
        // something else entirely.
        app.add_systems(
            Update,
            (
                resolve_doodads.after(crate::render::wmos::spawn_wmos),
                stream_doodads.after(resolve_doodads),
                // **This is the population the switch's catch-up pass exists
                // for.** Doodads stream: a placement that comes into range
                // while the switch is off is spawned visible, so hiding only on
                // the click would look like the setting undoing itself as the
                // character walks. See [`crate::render::tuning::switch`], whose
                // second `Added` pass is there for exactly this.
                crate::render::tuning::switch::<Doodad>(|tuning| tuning.doodads),
                // **After the spawn, and stated.** A rig spawned this frame has
                // joints at the identity until something writes them, and a
                // skinned mesh whose joints are all identity draws the model
                // folded into its own origin — one frame of a torch collapsed
                // to a point is exactly the kind of thing nobody sees and
                // nobody can reproduce. See [`pose_scenery`].
                pose_scenery.after(stream_doodads),
            ),
        );
    }
}

/// Resolve whatever placements have a model ready — place the hull, record the
/// resident — up to the frame's budget. Spawning is [`stream_doodads`]' job.
fn resolve_doodads(
    mut cache: ResMut<ModelCache>,
    // A doodad standing in a room is a second dressing of one model — same
    // meshes, different materials — so this pass needs the material store the
    // way the entity pass does. An outdoor placement never touches it.
    mut materials: Materials,
    mut tiles: Query<(
        &TerrainTile,
        &mut PendingDoodads,
        &mut TileDoodads,
        &mut ResidentDoodads,
    )>,
    focus: Res<crate::render::focus::WorldFocus>,
    solids: Res<Solids>,
    // Only so the model cache can build a dressing's merged meshes — see
    // `models::loader::MergeSource`.
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Doodads);
    // No map means nothing is being drawn, and a hull keyed to the wrong map is
    // a wall in a field. The *drawing* does not need one, but it costs a frame
    // at most: the tiles only exist because something set the focus.
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
                // **The scenery that moves**, resolved *before* anything below
                // has a side effect — and that ordering is the whole of a bug
                // this shipped with.
                //
                // The skinned build is a second, separate cache entry, and
                // nothing else in the client ever asks for one of a doodad. So
                // the first call is **always** `Loading`: it files the request
                // and answers next frame. Treating that as "no rig" and
                // resolving the placement anyway — which is what the first
                // draft did, with a comment calling it an acceptable
                // degradation — gave *every doodad in the world* `rig: None`
                // for ever, because the placement leaves the pending list on
                // the same frame and is never reconsidered. Nothing animated,
                // anywhere, and every test still passed.
                //
                // So it is held pending exactly as the unskinned build is. The
                // cost is one extra frame before an animated placement resolves,
                // against a feature that otherwise does not exist.
                let rig = match model
                    .skeleton
                    .as_ref()
                    .filter(|skeleton| vale_assets::look::scenery::animates(skeleton))
                    .map(|skeleton| {
                        // **Seeded from the id *and the position*.** A `MODD`
                        // spawn carries its building's id, so ten gryphon roosts
                        // in one hall share it — which is exactly what made them
                        // move in unison. See `look::scenery::seed`.
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
                            // for `EntityModel::cull_radius`, and it has to be:
                            // the two gates make the same promise.
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
                        // The unskinned build answered, so the model is fine and
                        // it is only the second build that failed. Drawn still,
                        // which is what this client did before any of this.
                        Lookup::Failed => None,
                    },
                };
                let rig_resolved = rig.is_some();
                budget -= 1;

                // **The solid half, before the drawn one.** A model can carry a
                // hull and no visible batch, and one that does still has to stop
                // a stride — so this cannot sit under the `draws.is_empty()`
                // return below. It also does not stream: what a stride walks
                // into is not a function of what is on screen, so the hull is
                // placed once, here, and lives until the tile does not.
                //
                // Most models have nothing here. 93 of Darkshire's 128 carry a
                // hull, and only 16 of Stormwind's 22 — the rest are the grass,
                // the birds and the flames the game means to be walked through.
                if !model.collision.is_empty() {
                    // A `MODD` spawn has no id of its own and carries its
                    // building's, so indoor placements need the ordinal — see
                    // [`SolidId`]. `drawn.solid` is a per-tile counter, which is
                    // all the ordinal has to be.
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

                // A model with no visible batches can still be all emitters —
                // a placed glow is exactly that — so only a model with
                // neither is done here.
                if model.draws.is_empty() && model.particles.is_none() && model.ribbons.is_none() {
                    return false;
                }

                // `placement.matrix` is composed in the file's own frame, 180°
                // term and all, and conjugating it is what carries that
                // derivation across rather than having it re-eyeballed. It is a
                // rotation and a uniform scale, so the decomposition into a
                // `Transform` is exact.
                let transform = Transform::from_matrix(axes::to_bevy_affine(
                    Mat4::from_cols_array(&placement.matrix),
                ));

                *claimed += 1;
                drawn.batches += model.draws.len();

                // The model's own declared radius, at this placement's scale —
                // the same volume the cull uses, so the two cannot disagree
                // about how big the thing is.
                let radius = model
                    .bounds
                    .map(|b| Vec3::from(b.half_extents).length() * placement.scale)
                    .unwrap_or(f32::MAX);

                // The spawn's own baked colour and its sun scale, on the
                // *instance*: the room-lit material only says which branch
                // to take, and the tag is what that branch reads. See
                // `RoomLight` and `sun_scale` — an absent tag is a zero
                // tag, which the shader reads as "no room light, neutral
                // sun", so only instances that differ from that pay for
                // the component.
                residents.list.push(Resident {
                    transform,
                    rig,
                    // **What this placement stands there lighting**, resolved
                    // once here rather than every frame: static scenery does not
                    // move, so a lamppost's glow is at the same three numbers
                    // for as long as the tile is loaded. `None` for almost
                    // every model in the world — see `render::lamps`.
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

/// **A joint of an animated doodad's rig** — the same marker an entity's joints
/// carry, and for the same reason: [`pose_scenery`] writes `GlobalTransform`
/// straight onto them, so they must be findable and must not be anything else.
#[derive(Component)]
pub struct SceneryJoint;

/// The root of one animated placement: the placement's transform, the joints
/// under it, and what it is playing.
///
/// A component rather than a resource keyed by entity, so that despawning the
/// root — which is what a placement streaming out of range does — takes the
/// whole rig with it and leaves nothing to reconcile.
#[derive(Component)]
pub struct PosedDoodad {
    skeleton: std::sync::Arc<vale_assets::world::m2::M2Skeleton>,
    /// The clip running right now, re-rolled by [`pose_scenery`] each time it
    /// ends — which is what makes a gryphon roost sit still, stretch, and sit
    /// still again rather than looping one flourish for ever.
    clip: vale_assets::look::scenery::Clip,
    /// When it started, on the same clock `pose_scenery` reads.
    started: f32,
    /// The roll the *next* pick will be made from. Advanced per pick so a
    /// placement's successive clips are as unrelated as two placements' first.
    roll: u32,
    /// In bone order, with the identity joint last — exactly an entity's.
    joints: Vec<Entity>,
    /// [`SceneryRig::cull_radius`], carried to where the pose is paid.
    cull_radius: f32,
}

/// Build one placement's rig: a root at the placement, and a joint per bone.
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
    joints_out.extend((0..rig.joint_count).map(|_| {
        // No `Transform`: `pose_scenery` writes the `GlobalTransform` itself,
        // and a `Transform` beside it would be propagated over the top every
        // frame. The same shape an entity's joints have.
        commands
            .spawn((SceneryJoint, GlobalTransform::default(), ChildOf(root)))
            .id()
    }));
    // **Opened on its own roll, part-way in.** Both halves are what stop a
    // courtyard moving as one: the clip differs per placement and so does how
    // far through it the placement already is.
    let clip = vale_assets::look::scenery::pick(&rig.skeleton, rig.seed)
        .expect("a rig is only built for a model that animates");
    let into = (rig.seed % clip.duration_ms.max(1)) as f32 / 1000.0;
    commands.entity(root).insert(PosedDoodad {
        skeleton: std::sync::Arc::clone(&rig.skeleton),
        clip,
        started: now - into,
        roll: vale_assets::look::scenery::next_roll(rig.seed),
        joints: joints_out.clone(),
        cull_radius: rig.cull_radius,
    });
    resident.spawned.push(root);
    root
}

/// **Sample every live scenery rig and write its joints** — the drawn half of
/// [`vale_assets::look::scenery`].
///
/// One `M2Skeleton::pose` per posed placement per frame, which is the same call
/// an entity pays, over whatever is drawn — which the draw range has already
/// cut to what the player is near, and the frustum gate below to what the
/// camera can actually see.
///
/// **Two clocks, and they are not the same one.** The clip runs on the
/// placement's own elapsed time — wall clock plus its phase, wrapped by the
/// clip's length — and the model's *global* sequences run on the wall clock
/// outright, which is what `pose`'s second argument is for. A torch's flicker is
/// a global sequence; giving it the phased clock would be inventing a
/// per-instance clock the file does not have. See the module note in
/// `look::scenery`.
///
/// The camera is passed so that bones flagged `0x8` billboard, which on this
/// population is most of what there is to see: a flame is a spherical billboard
/// and without it every torch in the world is a flat card seen edge-on from
/// half the angles you can stand at.
pub(crate) fn pose_scenery(
    time: Res<Time>,
    tuning: Res<crate::render::tuning::WorldTuning>,
    // **`Without<SceneryJoint>` on both readers, not on the writer.** Bevy
    // proves two queries disjoint from their filters alone, and it cannot know
    // that a camera is not a joint — so a plain `With<WorldCamera>` beside a
    // `&mut GlobalTransform` is `B0001` at schedule build, which is a panic on
    // the first frame rather than a compile error. The entity pass states it the
    // same way on its own camera and entity queries; see
    // `world::entities::pose::animate`.
    camera: Query<
        (&GlobalTransform, &bevy::camera::primitives::Frustum),
        (With<crate::world::camera::WorldCamera>, Without<SceneryJoint>),
    >,
    mut rigs: Query<(&mut PosedDoodad, &GlobalTransform), Without<SceneryJoint>>,
    mut joints: Query<&mut GlobalTransform, (With<SceneryJoint>, Without<PosedDoodad>)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::SceneryPose);
    if !tuning.doodad_animation {
        return;
    }
    let now = time.elapsed_secs();
    let now_ms = (now * 1000.0) as u32;
    // The camera's right and up in **world** space; `pose` wants them in the
    // model's, so each rig turns them by its own inverse placement below.
    let camera = camera.iter().next().map(|(placement, frustum)| (*placement, frustum.clone()));
    let view = camera.as_ref().map(|(c, _)| c.affine());
    for (mut rig, placement) in &mut rigs {
        // **The clip that has run out is re-rolled, not looped**, and that is
        // the difference between the reference's courtyard and a chorus line: a
        // gryphon roost sits still three times in five and takes one of four
        // flourishes otherwise, by the file's own weights. See
        // `vale_assets::look::scenery::pick`.
        //
        // A model with one row re-rolls to the same row, which is a loop — a
        // torch's flame — and costs one weighted pick per clip length.
        let elapsed = now - rig.started;
        let length = rig.clip.duration_ms as f32 / 1000.0;
        if elapsed >= length {
            let roll = rig.roll;
            if let Some(clip) = vale_assets::look::scenery::pick(&rig.skeleton, roll) {
                rig.clip = clip;
            }
            rig.roll = vale_assets::look::scenery::next_roll(roll);
            // From *now* rather than from `started + length`: a rig that has
            // been off screen for a minute would otherwise re-roll once per
            // clip length to catch up, all in one frame.
            rig.started = now;
        }
        let sequence = rig.clip.sequence;
        let elapsed_ms = ((now - rig.started).max(0.0) * 1000.0) as u32;
        let world_from_placement = placement.affine();
        // **A rig the camera cannot see is not posed** — the same gate, the
        // same contract and the same sphere as `world::entities::pose::animate`:
        // the clocks above have already advanced, so a rig entering the frustum
        // resumes mid-stride, and it is posed the same frame the test passes.
        // The scale is the largest of the placement's three axes, which is an
        // upper bound however the matrix is composed; the far plane is skipped,
        // which errs toward posing. `VALE_NO_SCENERY_CULL=1` removes the
        // gate, so its saving can be subtracted like every other one.
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
                    continue;
                }
            }
        }
        let billboard = view.map(|view| {
            let to_model = world_from_placement.inverse();
            let right = to_model.transform_vector3(view.x_axis.into());
            let up = to_model.transform_vector3(view.y_axis.into());
            (right.to_array(), up.to_array())
        });
        let pose = rig.skeleton.pose(sequence, elapsed_ms, now_ms, billboard, Default::default());
        for (bone, &joint) in pose.iter().zip(rig.joints.iter()) {
            if let Ok(mut transform) = joints.get_mut(joint) {
                *transform = GlobalTransform::from(
                    world_from_placement
                        * bevy::math::Affine3A::from_mat4(axes::pose_to_bevy(bone)),
                );
            }
        }
        // The identity joint the weightless vertices ride on — the placement
        // itself, unposed. Without it they collapse to the world origin.
        if let Some(&last) = rig.joints.last() {
            if let Ok(mut transform) = joints.get_mut(last) {
                *transform = GlobalTransform::from(world_from_placement);
            }
        }
    }
}

/// Whether an off-screen rig's pose is skipped — see the gate in
/// [`pose_scenery`].
///
/// A kill switch for the reason every other one exists: a saving nobody can
/// subtract is a saving nobody can check. Set `VALE_NO_SCENERY_CULL=1` and
/// every live rig is posed every frame, which is what this pass did before.
fn scenery_culling() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VALE_NO_SCENERY_CULL").is_none())
}

/// Whether a resident at `distance²` from the player should have entities,
/// given whether it currently does.
///
/// The two edges are deliberately apart. A parked resident spawns a margin
/// *before* its `VisibilityRange` cutoff, so the entity exists by the time the
/// camera could first see it — the range still decides the pixel, so nothing
/// pops sooner or later than it did when everything was spawned. A spawned one
/// is kept to a second margin past that, so a player pacing on one spot does
/// not churn the ring at the spawn distance.
fn wants_spawned(d2: f32, range: f32, spawned: bool) -> bool {
    let edge = if spawned {
        range + 2.0 * STREAM_MARGIN
    } else {
        range + STREAM_MARGIN
    };
    d2 < edge * edge
}

/// Spawn the residents the player has come within range of, and despawn the
/// ones left behind.
///
/// Runs when the player has moved [`STREAM_STEP`] yards or a tile has new
/// residents, and walks every resident of the 3x3 — a distance apiece, tens of
/// microseconds for a city's worth. Spawns are budgeted; a pass cut short by
/// the budget leaves the moved-marker unset so the next frame resumes.
pub(crate) fn stream_doodads(
    mut commands: Commands,
    focus: Res<crate::render::focus::WorldFocus>,
    // **A clock, for the one thing spawned here that has a phase.** A rig opens
    // part-way into its clip so that two placements of one model do not start
    // together — see `vale_assets::look::scenery`, and `spawn_posed`, which
    // is the only reader.
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut tiles: Query<(Entity, &mut ResidentDoodads, &mut TileDoodads)>,
    mut last: Local<Option<Vec3>>,
    // One scratch buffer for the whole pass rather than a `Vec` per rig: the
    // joints are copied straight onto the `PosedDoodad` and the list is reused,
    // so a city's worth of spawns allocates once.
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
                // **Posed if it moves at all**, with no second distance: a
                // placement is only here because it is near enough to draw. See
                // the note on [`spawn_posed`], and the one where the range used
                // to be.
                let posed = resident.rig.clone().map(|rig| {
                    spawn_posed(&mut commands, tile, resident, &rig, &mut rig_joints, now)
                });
                let range = VisibilityRange::abrupt(0.0, resident.range);
                // **A posed placement hangs its batches off its own rig root**
                // and a still one hangs them off the tile, and the two carry
                // different transforms for a reason that is Bevy's rather than
                // this client's: the joint matrices *replace* a skinned mesh's
                // world matrix, so a posed batch's own transform is never read
                // and the placement has to be in the joints instead. Give it the
                // placement as well and nothing breaks, which is exactly why it
                // is worth saying — see `spawn_posed`.
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
                        // A child of the tile, so walking out of range
                        // despawns the doodads with the ground they stand
                        // on — a parked resident goes with the component.
                        // …or of the rig root, which is itself a child of the
                        // tile, so the same walk-out takes a posed one too.
                        ChildOf(parent),
                    ));
                    // The batch's own bones, not the model's whole skeleton —
                    // `models::skin_for` is the one place that decides it, and
                    // this is its fifth caller.
                    if let Some(rig) = posed.as_ref().and(resident.rig.as_ref()) {
                        if let Some(joints) = crate::render::models::skin_for(draw, &rig_joints) {
                            batch.insert(SkinnedMesh {
                                inverse_bindposes: rig.inverse_bindposes.clone(),
                                joints,
                            });
                        }
                    }
                    // **A baked tint wins over the room light**, and it has
                    // to: a batch with one is drawn through a material
                    // specialised as *tinted*, so whatever is in the tag is
                    // read as `0xAARRGGBB` rather than as a room colour and a
                    // sun scale. Every batch this fires on is `unlit`, which is
                    // exactly the population that takes neither of the two
                    // things the room-light tag carries — so nothing is lost by
                    // preferring the tint. See `ModelDraw::baked_tint`, and the
                    // Gadgetzan light cone in `loader`, which is authored at
                    // 16% opacity and was drawn at 100%.
                    match (draw.baked_tint, resident.tag) {
                        (Some(tint), _) => {
                            batch.insert(bevy::mesh::MeshTag(tint));
                        }
                        (None, tag) if tag != 0 => {
                            batch.insert(bevy::mesh::MeshTag(tag));
                        }
                        _ => {}
                    }
                    // **On the first batch only.** The component is a list of
                    // world positions, not a per-mesh property, and a lamppost
                    // drawn in four batches must not be four lamps. It rides a
                    // spawned entity rather than the tile so that walking out of
                    // range takes the light with the geometry.
                    if let (true, Some(lamps)) = (first, resident.lamps.as_ref()) {
                        batch.insert(lamps.clone());
                    }
                    first = false;
                    // **A posed batch is not tracked here.** It is a child of
                    // the rig root, which is, and despawning that takes its
                    // children with it — listing both would despawn each batch
                    // twice.
                    if posed.is_none() {
                        resident.spawned.push(batch.id());
                    }
                }
                // The emitters stream with the batches: a brazier out of
                // range burns for nobody. Root entities — their meshes are
                // world-space — tracked here for the range despawn and tied
                // to the tile for the walk-out one (`retire_emitters`).
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
                // The trails beside them, on the same terms. A doodad is
                // drawn in its bind pose with no joints, so every ribbon on
                // one rides the placement itself.
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
        // **What is actually posed this instant**, recounted rather than
        // incremented: the walk above has just decided it for every resident of
        // this tile, and a counter nudged at three call sites drifts.
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

    /// A placement's matrix has to survive the trip into Bevy's frame as a
    /// `Transform`: it is a rotation and a uniform scale, so the decomposition
    /// is exact and the position it puts the model at is the one
    /// `placement_to_world` computed.
    ///
    /// This is the join between two things that are each checked elsewhere —
    /// `vale wmos` puts the matrix on the game's own `MODF` boxes to 0.00
    /// yards, and `axes` pins the basis — so what is worth testing is that the
    /// join does not quietly lose one of them.
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
        // tree in the world inside out and still look plausible.
        assert!(transform.rotation.is_normalized());
    }

    /// **The draw distance is the client's own size-bucketed law.** The three
    /// bands end at 50, 125 and 200 yards of *surface* distance — so the cull
    /// lands the radius past each — and a model bigger than 7 yards never
    /// distance-fades at all: it takes the ceiling past the loaded 3x3's edge,
    /// where only the terrain ending removes it. A missing bounds volume lands
    /// on the ceiling too, which is the conservative direction: an
    /// unmeasurable doodad is drawn, not dropped.
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

    /// **The streaming edges bracket the draw range, and they are apart.** A
    /// parked resident must exist *before* the camera reaches the
    /// `VisibilityRange` cutoff — an entity spawned exactly at the range would
    /// pop in a frame late every time — and a spawned one must survive past
    /// the spawn edge, or a player pacing on one spot spawns and despawns the
    /// whole boundary ring twice a step.
    #[test]
    fn the_streaming_edges_bracket_the_range_and_do_not_meet() {
        let range = 150.0;
        let d2 = |d: f32| d * d;
        // Inside the range itself: wanted whichever state it is in.
        assert!(wants_spawned(d2(100.0), range, false));
        assert!(wants_spawned(d2(100.0), range, true));
        // Between the range and the spawn edge: still spawned ahead of arrival.
        assert!(wants_spawned(d2(range + STREAM_MARGIN * 0.5), range, false));
        // Between the two edges: a parked resident stays parked and a spawned
        // one stays spawned — the hysteresis band.
        let between = d2(range + STREAM_MARGIN * 1.5);
        assert!(!wants_spawned(between, range, false));
        assert!(wants_spawned(between, range, true));
        // Past both edges: gone either way.
        let beyond = d2(range + STREAM_MARGIN * 2.5);
        assert!(!wants_spawned(beyond, range, false));
        assert!(!wants_spawned(beyond, range, true));
    }

    /// The model's own origin is where the placement says, and a point a yard up
    /// the model's Z stays a yard *up* in the world — the check that would fail
    /// if the vertices and the matrix disagreed about which frame they are in.
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
