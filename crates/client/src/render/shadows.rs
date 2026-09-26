//! The blob every unit stands on — **all of them, as one mesh and one draw**.
//!
//! 1.12 casts nothing at runtime, so this *is* the game's unit shadow rather
//! than an approximation of one: `Textures\ShadowBlob.blp`, a modulate-blended
//! quad lifted clear of the ground it would z-fight with. See
//! [`crate::render::models::SHADOW_BLOB`] for the client's own word on that and
//! for what measured the blend mode.
//!
//! ## Why it is a pass and not a child entity
//!
//! It was a child entity — one per unit, carrying the shared unit quad scaled
//! by that unit's footprint — and that is the obvious shape: the blob follows
//! its wearer for free, it is culled for free, and every one of them names the
//! same interned material, so they are one batch *set*.
//!
//! **One batch set is not one draw call in a sorted phase.** A modulate blend
//! is `AlphaMode::Blend`, which puts the quad in `Transparent3d`, which is
//! sorted back to front every frame and merges only *adjacent* runs of the same
//! set. A hundred blobs at a hundred depths, interleaved with the water, the
//! particles and every other translucent thing in the world, are never
//! adjacent — so they were a hundred draw calls, and this client's draw calls
//! are expensive: measured at a Northshire framing with a `bevy/trace_chrome`
//! capture, the transparent pass's **CPU encode** was 7.5 ms of a 25.9 ms frame
//! against 0.33 ms of GPU, and `--without blobshadows` took **123 of the 238
//! blended calls** and 2.7 ms of a 17.4 ms frame with it. Nothing about that is
//! visible in a GPU span, which is why it stood for so long.
//!
//! So the quads are built into one world-space mesh here. It is the same
//! decision `render::particles` already takes for an emitter's pool and for the
//! same reason: **when the phase cannot batch for you, batch before you reach
//! it.**
//!
//! ## Three consequences, and each is stated rather than hidden
//!
//! * **The field sorts as one item, at the world origin.** Bevy sorts a
//!   transparent item by its instance translation, so a field anchored at the
//!   origin is thousands of yards from any camera in the world and is therefore
//!   drawn *first* among translucent things. That is the safe end and it is
//!   deliberate: a blob lies **on** the ground, and the transparent phase is
//!   depth-*tested*, so anything that should have been drawn before it is
//!   behind the ground and is rejected by the depth test anyway. Order among
//!   the blobs themselves cannot matter at all — a modulate blend is a
//!   multiply, and a multiply commutes.
//! * **Culling is this pass's own.** A child entity got `VisibilityRange` and
//!   Bevy's frustum test for free; one mesh gets neither, so the distance cut
//!   and the frustum test are done here, per quad, while the mesh is built.
//! * **The mesh is rebuilt every frame.** A hundred quads is four hundred
//!   vertices — 15 KB — against a hundred draw calls at tens of microseconds
//!   each. It is not close.
//!
//! ## …and it drapes, which is the reference's own behaviour and was not this
//!
//! A blob was **one flat horizontal quad lifted `shadowBias` clear of the
//! ground**, which is what [`crate::render::decals`]' own module note said it
//! was and flagged as the obvious thing to fix. On level ground that is exact.
//! On a hillside it is a plane through a slope: the uphill half is buried and
//! correctly hidden by the depth test, and the downhill half hangs in the air
//! with a hard straight edge where the terrain falls away from it — which is a
//! shadow that both *clips through the ground* and looks far bigger than the
//! creature standing on it, because the part you can see is the part that has
//! left the ground behind. Both halves of that report are this one quad.
//!
//! The reference projects: it builds a projector, gathers
//! the receiving surfaces and clips to them, and skips the draw
//! outright when nothing received it. So the blob is sampled onto the ground
//! here too — the same heightfield sample `render::decals` takes, for the
//! reasons that module's note gives, and at a **coarser grid**: a blob is
//! 2–3 yards across against an `MCNK` cell's 4.17, so [`GRID`] of 4 already
//! samples the terrain finer than the terrain samples itself.
//!
//! Two things are deliberately not the decals module's:
//!
//! * **It stays one mesh and one draw.** That is the whole argument above, and
//!   a hundred `GroundDecal` entities would give it all back.
//! * **A blob with a node that finds no surface is not drawn**, which is the
//!   reference's own rule. It was the unit's feet plane for a round, on
//!   the argument that the owner is standing on something by construction —
//!   and then a character jumped, and the blob went up with them and hung in
//!   the air, because the ground had dropped past the slab. The ground under
//!   a jump is where the shadow belongs, so a surface *below* the feet is taken
//!   out to [`DROP`], and a blob whose ground is missing or further than that
//!   draws nothing rather than something in the air. The one exception is an
//!   app with no session, where no tile can ever arrive: there the feet plane
//!   stands, which is what the mesh tests are written against.

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::{Frustum, Sphere};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::axes;
use crate::render::models::{shadow_bias, Materials, ModelCache};
use crate::render::nothing;
use crate::render::tuning::WorldTuning;
use crate::world::entities::{EntityModel, EntitySet};
use crate::world::session::{ActiveSession, Session, Solids};

/// Cells per side of the grid one blob is sampled onto, so `(GRID + 1)²` height
/// queries per blob per frame.
///
/// Four rather than [`crate::render::decals`]' eight, and the argument is that
/// module's own turned round: the grid is chosen against the *ground's*
/// resolution, an `MCNK` cell is 4.17 yards across, and a blob is 2–3 yards
/// wide where an effect quad runs 2–12. Four cells over three yards is a sample
/// every 0.75 yards, which is already finer than the terrain's own vertices, so
/// a finer grid interpolates something exact and charges four times as many
/// queries for it.
const GRID: usize = 4;

/// How far from the unit's feet a sampled surface may be and still be taken as
/// the thing it is standing on, as a multiple of the blob's radius.
///
/// `render::decals`' own slab, for the same reason: a blob at
/// the foot of a cliff must not climb it. It bounds the surface *above* the
/// feet only; below them [`DROP`] applies. Past either the blob is not drawn —
/// see the module note.
const SLAB: f32 = 2.0;

/// How far *below* the feet a surface may be and still receive the blob, in
/// yards.
///
/// A jump lifts the feet about a yard and a half off the ground the shadow
/// belongs on, so the slab's two radii would lose it at the top of every
/// jump; a fall off a cliff should lose it, because a shadow on ground forty
/// yards down is not what a blob is. Six yards is past any jump and any step
/// a character can make and short of any drop that hurts.
const DROP: f32 = 6.0;

/// How far a blob may travel before its drape is asked for again, in yards.
///
/// **The one approximation in this pass, and it is sized against the ground's
/// own resolution.** A blob that has not moved at all reuses its heights
/// exactly; one that is running has to re-ask, and at 7 yards a second and 100
/// frames a second that is a fresh 25-node query every frame for a sample point
/// that moved seven centimetres. An `MCNK` cell is 4.17 yards across and the
/// grid samples every 0.75 yards, so a quarter of a yard is a third of the
/// finest spacing the terrain can express: the height taken is the height at a
/// point up to `DRAPE_STEP` away, which on the steepest ground a character can
/// stand on (about 50°) is under 30 cm and on the ground a crowd is actually on
/// is a centimetre or two. The slab test still bounds it, and the blob's own
/// quad follows the unit exactly — it is only the *conform* that is quantised.
///
/// It is worth an approximation here where it would not be elsewhere because
/// the drape was the most expensive system in the client and this is what makes
/// it proportional to distance travelled rather than to frame rate: at 100 fps
/// a running unit re-drapes 28 times a second instead of 100.
const DRAPE_STEP: f32 = 0.25;

/// How far away a unit's shadow is still drawn, in yards.
///
/// The real client has a `shadowLOD` CVar and this is what it is for; the number
/// is this client's, and it is the same 150 yards the F10 shadow cascade covers
/// so that the two agree about where "near the camera" ends.
const SHADOW_DISTANCE: f32 = 150.0;

/// The one entity every blob in the world is drawn by.
///
/// Its own marker so the HUD can find it and so nothing mistakes the field for
/// a piece of somebody's model — it is a root entity, not a child of any unit,
/// which is also why `render::selection`'s highlight walk can no longer reach
/// it and no longer has to say so.
#[derive(Component)]
pub struct ShadowBlob;

/// The field: the entity, the mesh it draws, and how many blobs went into it.
#[derive(Resource, Default)]
pub struct ShadowField {
    /// `None` until the texture is off the loader thread — see
    /// [`ModelCache::shadow_material`].
    entity: Option<Entity>,
    mesh: Option<Handle<Mesh>>,
    /// The corner positions last written into the mesh, for the skip below.
    ///
    /// The positions alone determine the whole mesh — the UVs and normals are
    /// per-quad constants and the indices are a function of the count — so a
    /// frame whose corners come out identical (every unit in reach standing
    /// still, which is most frames at most framings) writes nothing at all.
    /// Without this the field re-uploaded ~15 KB and marked the mesh asset
    /// changed every frame, which is a mesh-allocator pass and a
    /// specialization check downstream, for a picture that had not moved.
    written: Vec<[f32; 3]>,
    /// **How many units are standing on a shadow this frame**, for the HUD.
    ///
    /// It is a count worth keeping and it means something slightly different
    /// from what it used to. It was "how many units have been *given* a blob",
    /// which caught the shadow being a third thing that can be absent on its
    /// own (the texture arrives off the loader thread, later than the models);
    /// it is now "how many are in this frame's field", which folds the distance
    /// cut and the frustum in. A count under the *drawn* model count is still
    /// the failure it exists to show — a zero while models are on screen is
    /// still the texture never arriving.
    pub drawn: usize,
}

pub struct ShadowPlugin;

impl Plugin for ShadowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShadowField>().add_systems(
            Update,
            // **After the whole entity chain**, which is what places the
            // transforms this reads and what gives a unit the `EntityModel`
            // carrying its footprint. Stated rather than inherited: the two
            // systems share no component mutably, so nothing would order them
            // and a blob would simply land a frame behind the unit it is under
            // — which reads as the shadows sliding around behind everybody.
            field.after(EntitySet),
        );
    }
}

/// Rebuild the field from every unit that should be standing on a blob.
#[allow(clippy::too_many_arguments)]
fn field(
    mut commands: Commands,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    mut shadows: ResMut<ShadowField>,
    tuning: Res<WorldTuning>,
    session: Res<Session>,
    solids: Res<Solids>,
    camera: Query<(&GlobalTransform, &Frustum), With<crate::world::camera::WorldCamera>>,
    units: Query<(Entity, &Transform, &EntityModel)>,
    mut scratch: Local<Scratch>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Shadows);
    // **The switch is folded in rather than written over the top**, which is
    // `render::tuning::switch`'s own rule for a pass that decides its own
    // visibility: an empty field is no draw call at all, where hiding the
    // entity would leave the mesh built.
    let wanted = tuning.blob_shadows;
    let camera = camera.iter().next();

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    scratch.blobs.clear();
    if wanted {
        for (unit, placement, model) in &units {
            // The footprint in world yards: the model's own declared
            // half-extent times whatever the entity is scaled by.
            let radius = model.shadow_radius * placement.scale.max_element() * BLOB_HALF;
            if radius <= 0.0 {
                continue;
            }
            // **The feet, with no bias in them.** The bias is added once, in
            // [`drape`], to whatever surface each node lands on — a node that
            // found ground takes the ground's height and a node that found
            // none takes this. Adding it here as well put those two a
            // `shadowBias` apart from each other for one build.
            let centre = placement.translation;
            if let Some((eye, frustum)) = camera {
                if eye.translation().distance(placement.translation) > SHADOW_DISTANCE {
                    continue;
                }
                // The quad's own sphere, so a blob behind the camera is not in
                // the mesh at all. The far plane is skipped for the same reason
                // the pose gate skips it: the distance cut above is the far
                // plane here, and it is the tighter of the two.
                if !frustum.intersects_sphere(
                    &Sphere {
                        center: centre.into(),
                        radius,
                    },
                    false,
                ) {
                    continue;
                }
            }
            scratch.blobs.push(Blob { unit, centre, radius });
        }
    }
    // The memo is per map — see [`Drape`]. A teleport replaces the ground under
    // every entry at once, and the entities themselves are gone with it.
    let map = session.active.as_ref().map_or(0, |a| a.map_id);
    if scratch.map != map {
        scratch.map = map;
        scratch.memo.clear();
    }
    // …and an entry outlives its unit otherwise: a blob that walked out of
    // range, died, or was despawned would sit in the map for the session. The
    // sweep is over the memo rather than over the blobs, so it is proportional
    // to what is being kept and not to what is being drawn.
    if scratch.memo.len() > scratch.blobs.len() {
        let here: bevy::platform::collections::HashSet<Entity> =
            scratch.blobs.iter().map(|b| b.unit).collect();
        scratch.memo.retain(|unit, _| here.contains(unit));
    }
    // **Every node of every blob asked in two calls**, not two calls per blob:
    // the two lookups take a slice, and a hundred blobs at twenty-five nodes is
    // one 2,500-point query rather than two hundred small ones. See
    // [`drape`] for the join and for what a node that finds nothing does.
    drape(
        session.active.as_ref(),
        &solids.0,
        &mut scratch,
        &mut positions,
        &mut uvs,
        &mut normals,
        &mut indices,
    );

    // Read before it is written: whether the *previous* frame put anything in
    // the field is what decides, below, between rewriting an emptied mesh and
    // leaving it alone.
    let blobs = indices.len() / 6;
    let previously = shadows.drawn;
    shadows.drawn = blobs;

    // **The field is built on the first frame there is anything to put in it**,
    // and not before: the texture comes back off the loader thread a frame or
    // two into the session, exactly as a model does, and asking for the
    // material every frame until then is one hash lookup.
    if shadows.entity.is_none() {
        if positions.is_empty() {
            return;
        }
        let Some(material) = cache.shadow_material(&mut materials) else {
            return;
        };
        let mesh = meshes.add(empty_field());
        let id = commands
            .spawn((
                ShadowBlob,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                // At the world origin, which is what sorts the field first
                // among translucent things — see the module doc. Every vertex
                // is already in world space, so this transform is the identity
                // rather than a placement.
                Transform::default(),
                // One draw, always: there is nothing to gain from an `Aabb`
                // over a mesh whose extent changes every frame, and keeping one
                // current would cost more than the test saves.
                NoFrustumCulling,
                // It is a shadow; it does not cast one, whatever F10 says.
                bevy::light::NotShadowCaster,
            ))
            .id();
        shadows.entity = Some(id);
        shadows.mesh = Some(mesh);
    }
    let Some(handle) = shadows.mesh.clone() else {
        return;
    };
    // Nothing to draw: the field holds the degenerate quad rather than empty
    // arrays, for the reason [`crate::render::nothing`] gives — and it is
    // written **once**, on the frame the last blob leaves, because
    // `insert_attribute` marks the asset changed whether or not the value moved,
    // which is the same trap `place_entities` avoids on a `Transform`.
    //
    // The gate is the previous frame's blob count and not the mesh's own vertex
    // count: a single blob is four vertices too, so asking the mesh would leave
    // the last shadow in the world standing where its owner used to be.
    if blobs == 0 {
        if previously != 0 {
            if let Some(mut mesh) = meshes.get_mut(&handle) {
                nothing::nothing_drawn(&mut mesh);
            }
            shadows.written.clear();
        }
        return;
    }
    // Nothing moved: the corners are the ones already in the mesh, so leave
    // the asset untouched — see [`ShadowField::written`]. **Before**
    // `meshes.get_mut`, because `get_mut` alone marks the asset modified and
    // queues the re-upload this skip exists to avoid.
    if positions == shadows.written {
        return;
    }
    let Some(mut mesh) = meshes.get_mut(&handle) else {
        return;
    };
    shadows.written = positions.clone();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
}

/// Whether a blob that has not moved reuses its last drape — see [`Drape`].
///
/// The third of this round's kill switches, and there for the reason the other
/// two are: a saving nobody can subtract is a saving nobody can check. Set
/// `VALE_NO_DRAPE_MEMO=1` and every blob is asked about every frame, which
/// is what this pass did before.
fn memoising() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VALE_NO_DRAPE_MEMO").is_none())
}

/// The buffers the drape reuses across frames — one allocation for the life of
/// the app rather than four per rebuild, which is `render::decals`' `Scratch`
/// for `render::decals`' reason.
#[derive(Default)]
struct Scratch {
    /// This frame's blobs: whose they are, where the unit's feet are, and how
    /// wide its smudge is. Collected first so the surface under all of them can
    /// be asked for in one call.
    blobs: Vec<Blob>,
    xy: Vec<(f32, f32)>,
    terrain: Vec<Option<f32>>,
    building: Vec<Option<f32>>,
    /// **What each blob's nodes last landed on, and where it was standing when
    /// they did** — see [`Drape`].
    memo: bevy::platform::collections::HashMap<Entity, Drape>,
    /// Which map the memo describes; a change empties it.
    map: u32,
    /// This frame's answers for the blobs the memo refused to keep — a node
    /// with nothing under it yet. Cleared every frame; see [`Drape`].
    pending: bevy::platform::collections::HashMap<Entity, Vec<f32>>,
}

/// One blob to draw this frame.
#[derive(Clone, Copy)]
struct Blob {
    /// Whose it is, which is the memo's key.
    unit: Entity,
    centre: Vec3,
    radius: f32,
}

/// **The surface under one blob's grid, kept until the blob moves.**
///
/// The drape was the most expensive system in this client at 1.3 ms a frame,
/// and every bit of that is the two lookups: a blob is 25 nodes and every node
/// asks the terrain for a height and the collision world for a floor. A hundred
/// blobs is five thousand queries a frame — and in a city nearly every one of
/// them is asked about a unit that has not moved since the last frame and will
/// give the identical answer.
///
/// So the query is gated on the input actually having moved, which is the same
/// rule three other passes in this client keep. The key is the feet and the
/// radius **exactly** — bit-for-bit, as `place_entities` compares a
/// `Transform` — because a blob that moved a millimetre really does sample
/// different ground.
///
/// **An entry with a node that found nothing is not kept.** The one way a
/// cached answer could go stale is a tile or a building arriving under a unit
/// that is standing still, which is every unit for the first half minute of a
/// session; a node that found no surface is exactly the node that would be
/// wrong, so those blobs are re-asked every frame until the ground is there.
/// Going the other way — ground streaming *out* from under a standing unit —
/// keeps the last answer, which is the height the unit is standing at anyway.
struct Drape {
    centre: Vec3,
    radius: f32,
    /// The surface height at each of the `(GRID + 1)²` nodes.
    heights: Vec<f32>,
}

/// Sample the ground under every blob and write the conforming mesh.
///
/// **The join is the mover's**: the higher of the terrain and whatever building
/// floor is over the same point, which is what `world::session::Standing` does
/// for the character's own feet. It is repeated rather than shared for the
/// reason `render::decals` gives — a mover asks what it can stand on and this
/// asks what is visible under a thing already standing.
///
/// A blob with a node whose surface is missing, or is above the feet by more
/// than the slab or below them by more than [`DROP`], **is not drawn** — the
/// module note has the argument, and the reference's own rule. With no
/// session at all the feet plane stands in for the ground, so the mesh tests
/// have geometry to check.
#[allow(clippy::too_many_arguments)]
fn drape(
    active: Option<&ActiveSession>,
    solids: &vale_assets::CollisionWorld,
    scratch: &mut Scratch,
    positions: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    normals: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
) {
    let nodes = (GRID + 1) * (GRID + 1);
    // **Which blobs actually need asking.** A blob whose feet and radius are
    // bit-identical to the ones its memo entry was built from samples the same
    // ground and gets the same answer — see [`Drape`].
    scratch.xy.clear();
    scratch.pending.clear();
    let mut asked: Vec<usize> = Vec::new();
    for b in 0..scratch.blobs.len() {
        let blob = scratch.blobs[b];
        let fresh = memoising()
            && scratch.memo.get(&blob.unit).is_some_and(|d| {
                d.radius == blob.radius
                    && (d.centre - blob.centre).abs().max_element() <= DRAPE_STEP
            });
        if fresh {
            continue;
        }
        asked.push(b);
        for j in 0..=GRID {
            for i in 0..=GRID {
                let w = axes::to_wow(node(blob.centre, blob.radius, i, j));
                scratch.xy.push((w[0], w[1]));
            }
        }
    }
    // **One ceiling for the whole batch, and the slab test per node.**
    // `floor_batch` takes a single ceiling, so it is set above every blob being
    // asked about and the per-node test below is what rejects a floor that
    // belongs to some other storey. The two together cannot do worse than the
    // feet plane, which is the whole safety argument for the fallback.
    let ceiling = asked
        .iter()
        .map(|&b| {
            let blob = scratch.blobs[b];
            axes::to_wow(blob.centre)[2] + SLAB * blob.radius
        })
        .fold(f32::MIN, f32::max);
    match active {
        Some(active) if !scratch.xy.is_empty() => {
            active.terrain_heights(active.map_id, &scratch.xy, &mut scratch.terrain);
            solids.floor_batch(active.map_id, &scratch.xy, ceiling, &mut scratch.building);
        }
        // No session: nothing has streamed in, so every node finds nothing —
        // and is asked again next frame, exactly as a not-yet-arrived tile is.
        // The draw below stands the feet plane in for the ground in this one
        // case, so the mesh tests have geometry to check.
        _ => {
            scratch.terrain.clear();
            scratch.terrain.resize(scratch.xy.len(), None);
            scratch.building.clear();
            scratch.building.resize(scratch.xy.len(), None);
        }
    }

    // Record what was asked. A blob with a node that found no surface at all is
    // deliberately **not** kept — see [`Drape`].
    for (slot, &b) in asked.iter().enumerate() {
        let blob = scratch.blobs[b];
        let mut heights = Vec::with_capacity(nodes);
        let mut whole = true;
        for n in 0..nodes {
            let at = slot * nodes + n;
            // The mover's join: a building's floor over the ground it was
            // built on.
            let surface = match (
                scratch.building.get(at).copied().flatten(),
                scratch.terrain.get(at).copied().flatten(),
            ) {
                (Some(f), Some(g)) => Some(f.max(g)),
                (f, g) => f.or(g),
            };
            match surface {
                Some(z) => heights.push(z),
                None => {
                    whole = false;
                    // Marked rather than substituted: a node with nothing under
                    // it takes the whole blob out of the draw below.
                    heights.push(f32::NAN);
                }
            }
        }
        match whole {
            true => {
                scratch.memo.insert(
                    blob.unit,
                    Drape { centre: blob.centre, radius: blob.radius, heights },
                );
            }
            false => {
                scratch.memo.remove(&blob.unit);
                // Kept out of the memo and, carrying its NaN, out of the draw
                // below — but asked again next frame, for the tile that may
                // have arrived by then.
                scratch.pending.insert(blob.unit, heights);
            }
        }
    }

    let bias = shadow_bias();
    // With no session there is no ground to find and never will be; the feet
    // plane stands in. See the module note.
    let feet_plane = active.is_none();
    let mut ys: Vec<f32> = Vec::with_capacity(nodes);
    for blob in &scratch.blobs {
        let slab = SLAB * blob.radius;
        let heights = scratch
            .memo
            .get(&blob.unit)
            .map(|d| d.heights.as_slice())
            .or_else(|| scratch.pending.get(&blob.unit).map(Vec::as_slice));
        // **Every node has to have received the blob or none of it is drawn.**
        // WoW's `z` is Bevy's `y`; a surface may sit up to the slab above the
        // feet (a step, a kerb) and up to `DROP` below them (a jump), and
        // anything else — a missing tile, a floor of another storey, the
        // ground forty yards under a fall — takes the blob out this frame.
        ys.clear();
        for n in 0..nodes {
            match heights.and_then(|h| h.get(n)).copied() {
                Some(z) if z - blob.centre.y <= slab && blob.centre.y - z <= DROP => ys.push(z),
                _ if feet_plane => ys.push(blob.centre.y),
                _ => break,
            }
        }
        if ys.len() < nodes {
            continue;
        }
        let base = positions.len() as u32;
        for j in 0..=GRID {
            for i in 0..=GRID {
                let n = j * (GRID + 1) + i;
                let p = node(blob.centre, blob.radius, i, j);
                positions.push([p.x, ys[n] + bias, p.z]);
                uvs.push([i as f32 / GRID as f32, j as f32 / GRID as f32]);
                normals.push([0.0, 1.0, 0.0]);
            }
        }
        for j in 0..GRID {
            for i in 0..GRID {
                let a = base + (j * (GRID + 1) + i) as u32;
                let (bb, c, d) = (a + 1, a + GRID as u32 + 1, a + GRID as u32 + 2);
                // **The winding the single quad had, generalised.** The old
                // corner order was `(-1,-1) (1,-1) (1,1) (-1,1)` wound
                // `0 3 2 / 0 2 1`; this grid walks `+x` along `i` and `+z` along
                // `j`, which is the same handedness seen from above, so the same
                // reversal is needed. [`a_blob_faces_upward`] is the check, and
                // it is a check because a back-facing blob is culled outright —
                // no error, no warning, and every count unchanged.
                indices.extend([a, c, d, a, d, bb]);
            }
        }
    }
}

/// **What `EntityModel::shadow_radius` turns out to be a *diameter*, not a
/// radius** — and this is the one number in the blob pass the picture overruled
/// a reading on.
///
/// `spawn::shadow_radius` transcribes the client's own arithmetic and its note
/// records the value as the blob's *half*-width, which [`node`] below then
/// spans either side of the feet. That makes a human's blob 2.85 yards across.
/// Side by side against the 1.12.1 client on the same two characters standing on
/// the same sand, the reference's shadow is **about half that** — it hugs the
/// feet where ours reaches a body's width past them on every side.
///
/// So the arithmetic is kept exactly as it was measured and only what it
/// *means* is corrected, which is the half of a rendering fact this project's
/// own notes say to distrust: a number read out of the client is a
/// measurement, and "this is a half-width" is an interpretation of it. The
/// interpretation was wrong by a factor of two and nothing but a screenshot
/// could have said so.
const BLOB_HALF: f32 = 0.5;

/// One node of one blob's grid, in the horizontal plane — the corner positions
/// of the quad this replaces, subdivided.
///
/// **World-axis aligned rather than turned with the wearer**, which is not a
/// change: the quad it replaces was a child of the entity and inherited its
/// facing, and a rotation about up leaves a radially symmetric texture on a
/// horizontal grid looking exactly the same.
fn node(centre: Vec3, radius: f32, i: usize, j: usize) -> Vec3 {
    let (s, t) = (i as f32 / GRID as f32, j as f32 / GRID as f32);
    Vec3::new(
        centre.x + (s * 2.0 - 1.0) * radius,
        centre.y,
        centre.z + (t * 2.0 - 1.0) * radius,
    )
}

/// The field's mesh before anything has been put in it: the attributes declared,
/// so the vertex layout the material specialises against is fixed from the
/// first frame, and no triangles that reach the rasteriser.
///
/// "No triangles" is the degenerate quad and not an empty array — see
/// [`crate::render::nothing`].
fn empty_field() -> Mesh {
    // **`default()`, which is `MAIN_WORLD | RENDER_WORLD`, and not the
    // `RENDER_WORLD` every static mesh in this client takes.** A mesh extracted
    // to the render world alone has its vertex data *dropped* from the main
    // one, so the next frame's rebuild panics on the attribute it is trying to
    // replace. The same choice `particles` and `ribbons` make, and for the same
    // reason: this geometry is rewritten every frame.
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    nothing::nothing_drawn(&mut mesh);
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The half of the drape that needs no session and no collision world:
    /// every node keeps the feet plane, which is the flat quad this replaced.
    fn flat_field(blobs: &[(Vec3, f32)]) -> (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[f32; 3]>, Vec<u32>) {
        let mut scratch = Scratch {
            blobs: blobs
                .iter()
                .enumerate()
                // A distinct entity per blob, because the memo is keyed by one
                // — see [`Drape`]. `Entity::from_raw` is the cheapest way to
                // get one in a test with no world.
                .map(|(i, &(centre, radius))| Blob {
                    unit: Entity::from_raw_u32(i as u32).expect("a valid raw entity"),
                    centre,
                    radius,
                })
                .collect(),
            ..Scratch::default()
        };
        let (mut p, mut uv, mut n, mut i) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        drape(
            None,
            &vale_assets::CollisionWorld::default(),
            &mut scratch,
            &mut p,
            &mut uv,
            &mut n,
            &mut i,
        );
        (p, uv, n, i)
    }

    /// **A blob that has not moved is not asked about again**, and one that
    /// has moved further than [`DRAPE_STEP`] is.
    ///
    /// The drape was the most expensive system in this client — 0.51 ms a frame
    /// standing in Elwynn, 0.89 with forty players around — and all of it is
    /// the two lookups behind every one of a blob's 25 nodes. A hundred blobs
    /// is five thousand queries a frame, and in a city nearly every one is
    /// about a unit that has not moved. See [`Drape`].
    #[test]
    fn a_blob_that_has_not_moved_is_not_re_asked() {
        let unit = Entity::from_raw_u32(1).expect("a valid raw entity");
        let mut scratch = Scratch::default();
        let nodes = (GRID + 1) * (GRID + 1);
        fn ask(scratch: &mut Scratch, unit: Entity, centre: Vec3) -> (usize, Vec<[f32; 3]>) {
            scratch.blobs = vec![Blob { unit, centre, radius: 1.0 }];
            let (mut p, mut uv, mut n, mut i) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            drape(
                None,
                &vale_assets::CollisionWorld::default(),
                scratch,
                &mut p,
                &mut uv,
                &mut n,
                &mut i,
            );
            (scratch.xy.len(), p)
        }

        // With no session every node finds nothing, so the entry is refused and
        // the blob is re-asked — which is the streaming-in case the memo is
        // *not* allowed to cache. See [`Drape`].
        let (asked, flat) = ask(&mut scratch, unit, Vec3::ZERO);
        assert_eq!(asked, nodes, "the first frame asks");
        let (asked_again, _) = ask(&mut scratch, unit, Vec3::ZERO);
        assert_eq!(
            asked_again, nodes,
            "a blob whose nodes found nothing is asked again, or a tile that \
             arrives under a standing unit would never be draped"
        );

        // …and with an answer to keep, it is kept until the blob moves past the
        // step. Seeded by hand, because a session and a collision world are
        // exactly what this test exists not to need.
        scratch.memo.insert(
            unit,
            Drape { centre: Vec3::ZERO, radius: 1.0, heights: vec![0.0; nodes] },
        );
        let (asked, _) = ask(&mut scratch, unit, Vec3::ZERO);
        assert_eq!(asked, 0, "an unmoved blob asks nothing");
        let (asked, _) = ask(&mut scratch, unit, Vec3::new(DRAPE_STEP * 0.5, 0.0, 0.0));
        assert_eq!(asked, 0, "and neither does one that has barely moved");
        let (asked, _) = ask(&mut scratch, unit, Vec3::new(DRAPE_STEP * 2.0, 0.0, 0.0));
        assert_eq!(asked, nodes, "past the step it is asked again");

        // The mesh is still a full grid either way — the memo skips the
        // *query*, never a vertex.
        assert_eq!(flat.len(), nodes);
    }

    /// A blob whose owner has gone does not sit in the memo for the session.
    #[test]
    fn the_memo_lets_go_of_a_blob_whose_unit_left() {
        let mut scratch = Scratch::default();
        for i in 0..4u32 {
            scratch.memo.insert(
                Entity::from_raw_u32(i).expect("a valid raw entity"),
                Drape { centre: Vec3::ZERO, radius: 1.0, heights: Vec::new() },
            );
        }
        scratch.blobs = vec![Blob {
            unit: Entity::from_raw_u32(2).expect("a valid raw entity"),
            centre: Vec3::ZERO,
            radius: 1.0,
        }];
        // The sweep `field` runs, transcribed: it is over the memo rather than
        // over the blobs, so it is proportional to what is being kept.
        if scratch.memo.len() > scratch.blobs.len() {
            let here: bevy::platform::collections::HashSet<Entity> =
                scratch.blobs.iter().map(|b| b.unit).collect();
            scratch.memo.retain(|unit, _| here.contains(unit));
        }
        assert_eq!(scratch.memo.len(), 1);
    }

    /// **A blob is a square of the footprint centred on the unit**, and with
    /// nothing under it every node sits at the feet — which is the flat quad it
    /// replaced, subdivided.
    ///
    /// Asserted on what the corners *land on* rather than on the sum, because
    /// the number that matters is the width on the ground: an earlier shape put
    /// `radius * 2.0` into a scale and let the parent's own scale multiply it,
    /// so a tauren's blob was wider than a human's by exactly its scale, and a
    /// version of this that dropped the entity scale would look right on every
    /// human in the game.
    #[test]
    fn a_blob_is_a_square_of_the_footprint_centred_on_the_unit() {
        let (p, _uv, _n, i) = flat_field(&[(Vec3::new(10.0, 3.0, -4.0), 1.5)]);
        assert_eq!(p.len(), (GRID + 1) * (GRID + 1));
        assert_eq!(i.len(), GRID * GRID * 6);
        // Flat: every node at the feet, plus the bias the reference's own
        // `shadowBias` CVar names.
        let y = 3.0 + shadow_bias();
        assert!(p.iter().all(|c| (c[1] - y).abs() < 1e-6), "{p:?}");
        // Square, three yards across, centred.
        let xs: Vec<f32> = p.iter().map(|c| c[0]).collect();
        let zs: Vec<f32> = p.iter().map(|c| c[2]).collect();
        assert_eq!(
            (
                xs.iter().cloned().fold(f32::MAX, f32::min),
                xs.iter().cloned().fold(f32::MIN, f32::max)
            ),
            (8.5, 11.5)
        );
        assert_eq!(
            (
                zs.iter().cloned().fold(f32::MAX, f32::min),
                zs.iter().cloned().fold(f32::MIN, f32::max)
            ),
            (-5.5, -2.5)
        );
    }

    /// **The blob faces the sky**, which is the same winding question a model
    /// batch answers and has the same failure: a grid wound the other way is
    /// back-face culled and the shadow simply is not there, with no error and
    /// every count unchanged. It moved here from `models::tests` with the quad
    /// it is about, and it is the one thing about the merge that could have
    /// been silently inverted — `axes::to_bevy` negates two of three axes, so
    /// the natural corner order faces *down*, which is exactly what took the
    /// whole ground-decal population off the screen in an earlier round.
    #[test]
    fn a_blob_faces_upward() {
        let (p, _uv, _n, i) = flat_field(&[(Vec3::ZERO, 1.0)]);
        for triangle in i.chunks_exact(3) {
            let c: Vec<Vec3> = triangle
                .iter()
                .map(|&v| Vec3::from_array(p[v as usize]))
                .collect();
            let geometric = (c[1] - c[0]).cross(c[2] - c[0]);
            assert!(geometric.y > 0.0, "wound face down: {geometric:?}");
        }
    }

    /// **The texture is laid across the whole grid and not once per cell**,
    /// which is the one thing subdividing the quad could have silently got
    /// wrong: a UV that restarted at each cell would tile `ShadowBlob.blp`
    /// sixteen times under every creature in the world and draw a dark grid
    /// instead of a smudge.
    #[test]
    fn the_texture_spans_the_grid_once() {
        let (_p, uv, _n, _i) = flat_field(&[(Vec3::ZERO, 1.0)]);
        let u: Vec<f32> = uv.iter().map(|c| c[0]).collect();
        assert_eq!(u.iter().cloned().fold(f32::MAX, f32::min), 0.0);
        assert_eq!(u.iter().cloned().fold(f32::MIN, f32::max), 1.0);
        // …and the corner nodes are the corners of the texture, in step with
        // the corner positions above.
        assert_eq!(uv[0], [0.0, 0.0]);
        assert_eq!(uv[GRID], [1.0, 0.0]);
        assert_eq!(uv[uv.len() - 1], [1.0, 1.0]);
    }

    /// **The field never hands Bevy a zero-vertex mesh either**, which for this
    /// pass is about the *emptied* mesh rather than the spawned one: the field
    /// is only ever built on a frame that has blobs, but every blob leaving the
    /// camera's reach used to write empty arrays over it. See
    /// [`crate::render::nothing`] for what that costs.
    #[test]
    fn an_emptied_field_is_the_degenerate_quad() {
        let mesh = empty_field();
        assert_eq!(mesh.count_vertices(), nothing::VERTICES);
        // …and no colour, because the blob material was never specialised
        // against one — the half of the shared writer that is conditional.
        assert!(!mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR));
    }

    /// **Every blob after the first is indexed off its own base**, which is the
    /// one thing a merged mesh can get wrong that a per-entity quad could not:
    /// a second grid written with the first one's indices draws the first one
    /// twice and never draws the second, which on screen is a unit standing on
    /// nothing while another has a shadow that will not move.
    #[test]
    fn a_second_blob_indexes_its_own_corners() {
        let nodes = (GRID + 1) * (GRID + 1);
        let (p, uv, n, i) =
            flat_field(&[(Vec3::ZERO, 1.0), (Vec3::new(50.0, 0.0, 0.0), 1.0)]);
        assert_eq!(p.len(), nodes * 2);
        assert_eq!(uv.len(), nodes * 2);
        assert_eq!(n.len(), nodes * 2);
        assert_eq!(i.len(), GRID * GRID * 12);
        let second = &i[GRID * GRID * 6..];
        assert!(
            second.iter().all(|&v| (nodes as u32..nodes as u32 * 2).contains(&v)),
            "{second:?}"
        );
    }
}
