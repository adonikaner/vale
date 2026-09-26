//! Particle emitters: the torch flames, chimney smoke, spell dust and wisp
//! trails the models ask for.
//!
//! `m2.rs` parses the 504-byte `M2ParticleOld` records (`vale particles`
//! surveys all 2,626 of them); this module is the half that runs — a CPU
//! simulation per placed emitter and a dynamic quad mesh per frame, drawn
//! through the same `M2Material` as everything else in the world.
//!
//! **The update laws here are the 5875 client's own, not invented ones.**
//! The shape kernels, the integrator (age, `pos += v·dt`, gravity on the up
//! axis, `v -= min(dt·drag, 1)·v`, the sphere kill-outbound tail test), the
//! emission LOD (full rate inside 50 yards, linear falloff to a 25% floor),
//! the quad writer (head, tail, the `< 7.7e-4` degenerate fallback), and
//! the over-life two-segment ramp all follow the client's own particle
//! behaviour exactly. What this
//! module *does not yet do* is also named, per emitter flag, in
//! [`retired deviations`](#deviations) below.
//!
//! ## Where an emitter lives
//!
//! One entity per placed emitter, at the **root** of the hierarchy: its mesh
//! is rebuilt in world space every frame, so a parent's transform must not
//! compose into it — the entity's own `Transform` carries only the anchor
//! (the emitter's world position), which is what the transparent phase sorts
//! by. Root entities do not despawn with a tile or an entity, so every
//! emitter records its `owner` and [`retire_emitters`] despawns the orphans —
//! the same shape as `retire_colliders`, for the same reason.
//!
//! ## Where an emitter is *drawn* — the additive ones share a draw
//!
//! An **additive** quad emitter (blend 3 or 4 — in the shipped data 2,179 of
//! the game's 2,626, all of them at 4, since nothing authors 3) has no draw
//! of its own: [`merge_fields`] builds every visible one
//! into a single world-space mesh **per interned material**, the same
//! decision `render::shadows` took for the blobs and for the same reason —
//! the sorted transparent phase merges only *adjacent* same-set runs, so
//! fifty flames at fifty depths were fifty draw calls at ~30 µs of CPU
//! encode each. Additive blends write `dst + f(src)` and addition commutes,
//! so order **within** the field cannot matter; see [`merged`] for why the
//! alpha-blended minority stays per-emitter, and the field doc for where the
//! one field sorts against everything else.
//!
//! ## The clock
//!
//! An emitter's rate and its ON/OFF gate are keyed tracks on the model's
//! animation timeline. The client runs them on sequence 0's window, wrapped
//! if that sequence loops — which is what makes a campfire pulse and a wisp's
//! trail come and go without any animation playing. The other eight
//! properties are constant in every model the game ships, so they are sampled once, at their first key.
//!
//! ## Deviations
//!
//! Named so the next round knows where to look, with the emitter counts from
//! `vale particles`:
//!
//! * **model-space clouds (flag 0x10, 701 emitters) are birth-baked** like
//!   everything else — visible only on an emitter that moves or turns while
//!   its particles live.
//! * **velocity inherit (0x40, 85) and follow-emitter (0x4000, 73)** are not
//!   applied; both need the emitter's own frame-to-frame motion.
//! * **spline emitters (type 3, 23 of 2,626)** are born at the chain's first
//!   control point rather than along it.
//! * **ground snap (0x2000, 41), twinkle and the recursion model (child
//!   emitters, 19 emitters over 6 files)** are not implemented.
//! * **geometry models are drawn** — see [`model_particles`], which is where
//!   Cone of Cold's and Evocation's slabs came from — but a *rigged* one is
//!   drawn in its bind pose. All 13 files in that population are static.
//! * **lit particles (flag 0x1) are drawn unlit.** The client's emitter
//!   creation clears the material's *unlit* bit when the flag is
//!   set, so fixed-function lighting scales those clouds with the day —
//!   Cone of Cold's clouds carry it, and at night they should dim. Every
//!   particle here keeps the over-life ramp as its whole light.

use crate::axes;
use crate::render::models::{M2Material, M2Params, Materials, SceneLighting, M2_ALPHA_KEY};
use crate::render::nothing;
use vale_assets::world::m2::{particle_flags, M2Particle, M2Track};
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::{Aabb, Frustum, Sphere};
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers, ViewVisibility};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use std::sync::Arc;

/// The per-emitter particle cap: a backstop, far above any steady-state pool
/// the shipped rates produce.
const MAX_PARTICLES: usize = 1024;

/// The emission distance LOD: full rate inside this range…
const LOD_FULL_RATE: f32 = 50.0;
/// …falling linearly by this much per yard, to a floor of [`LOD_FLOOR`].
const LOD_FALLOFF: f32 = 0.02;
const LOD_FLOOR: f32 = 0.25;

/// The emitter clock: sequence 0's window on the model's shared timeline,
/// and the global-sequence durations for the tracks that ignore it.
#[derive(Clone, Debug)]
pub struct ParticleClip {
    pub start: u32,
    pub end: u32,
    /// Wrap the clock onto the window, or hold its end. From a read of the
    /// 5875 loader: a sequence loops when bit 0 of its flags is clear.
    pub loops: bool,
    pub global_sequences: Arc<[u32]>,
}

/// One emitter definition and the material its quads draw through.
pub struct ParticleEmitter {
    pub def: M2Particle,
    pub material: Handle<M2Material>,
}

/// Every emitter of one model, shared by all its dressings and placements.
pub struct ParticleSet {
    pub emitters: Vec<ParticleEmitter>,
    pub clip: Option<ParticleClip>,
}

/// Build a model's emitter set, interning one material per emitter.
///
/// The texture is the model's **own** — `vale particles` counts zero
/// client-supplied slots over every emitter in the game — so the set is a
/// property of the file, like the collision hull. A slot the archive lacks
/// keeps the magenta placeholder, which under additive blend glows magenta:
/// the same honest signal it is everywhere else.
pub fn build_set(
    defs: &[M2Particle],
    clip: Option<ParticleClip>,
    kinds: &[u32],
    own: &[Handle<Image>],
    missing: &Handle<Image>,
    materials: &mut Materials,
) -> Option<Arc<ParticleSet>> {
    if defs.is_empty() {
        return None;
    }
    let emitters = defs
        .iter()
        .map(|def| {
            let slot = def.texture as usize;
            let texture = own
                .get(slot)
                .filter(|_| kinds.get(slot) == Some(&0))
                .cloned()
                .unwrap_or_else(|| missing.clone());
            // The per-blend fog policy — the client's own table (fog modes
            // air/air/air/black/black/white/grey for blends 0..6): an additive
            // draw fogs toward black, everything else toward the air. Flag 0x8
            // opts an emitter *into* fog — inverted from the wiki's
            // reading — and the clear bit rides the same `ambient.w` switch
            // the shadow blob uses.
            let mode = match def.blend {
                3 | 4 => 2.0,
                _ => 1.0,
            };
            // **`blendingType` is the batch blend enum, checked rather than
            // assumed.** The client's emitter build switches on it and writes
            // a material blend:
            //
            // ```text
            // 1 -> 1  and sets material bit 2   4 -> 3
            // 2 -> 2                            5 -> 4
            // 3 -> 0  (falls to the default)    6 -> 5
            // ```
            //
            // — which is the batch blend table `[0,1,2,10,3,4,5]` term for
            // term everywhere the shipped data goes (the census reads
            // 1: 26, 2: 420, 4: 2179, 5: 1 over all 2,626 emitters, and
            // **nothing at all at 3**, so the one entry the two tables disagree
            // on has no data behind it). Passing `def.blend` straight into the
            // shared material is therefore right, and now measured.
            let unfogged = def.flags & particle_flags::FOGGED == 0;
            let material = materials.intern(M2Material {
                params: M2Params {
                    ambient: Vec4::new(0.0, 0.0, 0.0, if unfogged { 1.0 } else { 0.0 }),
                    alpha_cutoff: crate::render::models::alpha_cut(def.blend, M2_ALPHA_KEY),
                    // Fire does not take the sun; the particle branch in
                    // `m2.wgsl` is its own lighting (the over-life colour).
                    unlit: 1.0,
                    vertex_lit: 0.0,
                    liquid: 0.0,
                    liquid_close: Vec4::ZERO,
                    liquid_far: Vec4::ZERO,
                    uv_row0: Vec4::ZERO,
                    uv_row1: Vec4::ZERO,
                    // Unlit, so no scene states anything about it — see
                    // `SceneLighting`, whose whole population is two screens.
                    // Solid: `body.x` is the opacity and 1.0 is "as authored".
            body: Vec4::X,
                    // A particle quad has no environment map — see
                    // `M2Material::overlay_a`.
                    overlay: Vec4::ZERO,
                    scene_ambient: SceneLighting::NONE.ambient,
                    scene_lamps: SceneLighting::NONE.lamps,
                    particle: Vec4::new(mode, 0.0, 0.0, 0.0),
                },
                // **No layers, and the base's own handle in both slots** —
                // see `M2Material::overlay_a`: a binding cannot be empty, and
                // bevy ref-counts bindless resources by id, so naming a handle
                // the material already holds takes no extra slot in the slab.
                overlay_a: texture.clone(),
                overlay_b: texture.clone(),
                texture,
                blend: def.blend,
                two_sided: true,
                no_depth_write: false,
                // Only the ground foliage sways; see `M2Material::wind`.
                wind: false,
            });
            ParticleEmitter {
                def: def.clone(),
                material,
            }
        })
        .collect();
    Some(Arc::new(ParticleSet { emitters, clip }))
}

/// Where an emitter's frame comes from each frame.
pub enum Anchor {
    /// A doodad: the placement, composed once. In the bind pose every bone
    /// matrix is the identity, so the emitter's bone-local position is a
    /// model-space position and the placement is the whole frame.
    Fixed(Transform),
    /// An entity's emitter riding a bone: the joint's `GlobalTransform` is
    /// already `placement × bone pose`, exactly what an attachment composes.
    Joint(Entity),
    /// An entity's emitter whose bone did not resolve: the entity itself.
    Owner,
}

/// One live particle. World space, Bevy axes.
struct Particle {
    pos: Vec3,
    vel: Vec3,
    age: f32,
    life: f32,
    /// Per-particle randomness that must stay fixed for its whole life — the
    /// spin-negate bit, in the reference a hash of the pool slot's pointer.
    seed: u32,
    /// **The instance orientation of a [model particle](model_particles)**, and
    /// the identity for every quad in the world.
    ///
    /// Seeded at birth from the emitter's own frame — the same +90° about local
    /// Z the shape kernels take — and tumbled per step by [`Self::angvel`]
    /// (a Rodrigues half-angle delta right-multiplied in the body
    /// frame). A quad's rotation is a scalar in its billboard plane and lives
    /// in `spin` instead; there is no overlap.
    quat: Quat,
    /// Radians/second about each body axis, drawn once at birth from the
    /// emitter's `tumble_min`/`tumble_max`. Zero for a quad.
    angvel: Vec3,
}

/// One placed emitter's live state.
#[derive(Component)]
pub struct Emitter {
    set: Arc<ParticleSet>,
    index: usize,
    /// Whose existence this emitter is tied to — the tile or the world
    /// entity. Gone means [`retire_emitters`] despawns this too.
    owner: Entity,
    anchor: Anchor,
    /// The placement's uniform scale, for flag 0x20 (`scale_by_instance`).
    scale: f32,
    /// How far a quad can stick out past its particle's *centre*, in yards —
    /// the margin the pool's bounds are grown by to bound the drawn cloud.
    /// Computed once at spawn from the def's own extremes (largest over-life
    /// size, plus the longest tail a particle at terminal speed can trail),
    /// because a bound recomputed per frame would cost what it saves.
    pad: f32,
    pool: Vec<Particle>,
    /// Fractional births owed — the emission accumulator.
    acc: f32,
    /// Seconds since this emitter spawned: the clip clock's input.
    age: f32,
    /// The gate's state last frame, for the burst flag's rising edge.
    gate_was_on: bool,
    rng: u32,
    /// Where the emitter stood this frame — written by [`simulate`] and read by
    /// [`model_particles::draw`], which runs after it and needs the anchor its
    /// child instances are positioned relative to.
    origin: Vec3,
    /// The emitter's live frame this frame — written by [`simulate`] beside
    /// `origin`, read by [`merge_fields`], which runs after the pose is settled
    /// and needs it for an XY-quad's plane.
    frame: bevy::math::Affine3A,
    /// The draw range the per-emitter path expresses as a `VisibilityRange`
    /// component, held here as well because a [`merged`] emitter has no draw of
    /// its own for that component to cull — [`merge_fields`] applies it by
    /// hand, the way `shadows::field` applies its distance cut.
    range: Option<f32>,
    /// A pooled instance per live particle, for a **geometry-model emitter**
    /// and empty for every other one. See [`model_particles`].
    instances: Vec<ModelInstance>,
    /// The most particles this emitter has ever had alive at once, which is
    /// what [`Emitter::output`] normalises against. See it for why the
    /// reference is the emitter's own high-water mark rather than an analytic
    /// steady state: `emission_rate` and `lifespan` are both **tracks**, so
    /// "how many should be alive" is itself a per-frame sample and a moving
    /// target, while this is monotonic and costs a compare.
    peak: usize,
    /// …and how much of that it is delivering now, smoothed —
    /// [`Emitter::output`]'s value, advanced once a frame by [`step`].
    output: f32,
}

/// One geometry-model particle's drawn body: the model's batches as child
/// entities of the emitter, reused frame to frame rather than respawned.
struct ModelInstance {
    parts: Vec<Entity>,
}

impl Emitter {
    fn def(&self) -> &M2Particle {
        &self.set.emitters[self.index].def
    }

    /// **This emitter's own definition**, for the one pass outside this file
    /// that has to ask what kind of emitter it is: [`crate::render::lamps`],
    /// which turns an additive one into a light. Read-only, and it is the
    /// definition rather than an answer because the *rule* about what glows
    /// belongs to that module and not to this one.
    pub fn definition(&self) -> &M2Particle {
        self.def()
    }

    /// **Whose this emitter is** — the entity its life is tied to. Read-only,
    /// for a host that has to find every root-level effect a unit of its own
    /// spawned: an emitter lives at the world root, so walking the unit's
    /// children never reaches it, and this is the only join back.
    pub fn owner(&self) -> Entity {
        self.owner
    }

    /// Where this emitter stood when [`simulate`] last ran.
    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    /// The placement scale its particle *sizes* take. See
    /// [`Self::instance_scale`], which this is the public name of.
    /// **How much light this emitter is putting out right now**, 0 to 1, as a
    /// smoothed fraction of the most it has ever put out.
    ///
    /// It exists because [`crate::render::lamps::LampLight::of`] reads the
    /// emitter's **definition** — the authored colour keys and sizes — and the
    /// definition does not change. So a spell effect used to light the world at
    /// full strength for exactly as long as its *entity* existed and not one
    /// frame of that at the brightness it was actually drawing: a box function
    /// with a fade at each end, over a visual that has its own rise and fall.
    ///
    /// On a one-shot that is nearly invisible, because the fade is most of the
    /// effect's life. On a held or channelled one — `CycloneWater_State` for
    /// Evocation, `Magic_PreCast_Hand` for a cast bar — the light snaps to full,
    /// sits flat for the whole channel and snaps off, and the fade is a
    /// twentieth of it at either end. That is the whole of *"only some spell
    /// effects have the fade"*: they all have it, and on the long ones there is
    /// nothing else to see, so it reads as on/off.
    ///
    /// It is worse than cosmetic on the emitters the file **gates**:
    /// `Magic_PreCast_Hand`'s second emitter carries `gated by animation`, so it
    /// spends part of its life emitting nothing at all while lighting the ground
    /// as though it were.
    ///
    /// Live particle count is the honest measure — it is what is on screen —
    /// and it is smoothed because the count is noisy frame to frame and an
    /// unsmoothed one would make every campfire in the world flicker at
    /// whatever rate its emission accumulator happens to beat at.
    pub fn output(&self) -> f32 {
        self.output
    }

    pub fn placement_scale(&self) -> f32 {
        self.instance_scale()
    }

    /// The placement scale this emitter's *sizes* take — the instance's own
    /// when flag 0x20 is set and 1 otherwise, which is the same split
    /// [`fill_quads`] makes. An instance-scaled prop otherwise scales only its
    /// particle **positions**.
    fn instance_scale(&self) -> f32 {
        if self.def().flags & particle_flags::SCALE_BY_INSTANCE != 0 {
            self.scale
        } else {
            1.0
        }
    }
}

/// Whether this emitter's quads are drawn through a per-material field
/// rather than through a mesh of its own — see [`merge_fields`].
///
/// **Additive only (blend 3 and 4), and quads only.** An additive blend is
/// `dst + f(src)` whatever its source factor, and addition commutes, so any
/// pile of them in one draw is pixel-identical to the same pile drawn in any
/// sorted order. Blend 2 is `mix(dst, src, a)`, which does not commute — an
/// alpha-blended puff in front of another must be drawn after it — so the 420
/// emitters authored at 2 keep their own draw and the phase's own sort. Blend
/// 5's multiply commutes too, but it is **one** emitter in the whole game and
/// a field per material would not merge anything. A geometry-model emitter
/// has no quads at all: its particles are child instances
/// ([`model_particles`]), and its entity is only their anchor.
fn merged(def: &M2Particle) -> bool {
    def.geometry_model.is_none() && matches!(def.blend, 3 | 4)
}

/// Spawn the emitter entities for one placement of a model.
///
/// Root entities — see the module comment — so the caller keeps the returned
/// ids if it despawns things itself (the doodad stream does), and
/// [`retire_emitters`] covers whoever does not.
pub fn spawn_emitters(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    set: &Arc<ParticleSet>,
    owner: Entity,
    anchors: impl Fn(usize, &M2Particle) -> Anchor,
    scale: f32,
    range: Option<f32>,
) -> Vec<Entity> {
    // Perf-bisect kill-switch: a run with
    // `VALE_NO_PARTICLES` set spawns no emitters at all, so two otherwise
    // identical runs subtract to exactly the particle system's whole cost.
    if std::env::var_os("VALE_NO_PARTICLES").is_some() {
        return Vec::new();
    }
    set.emitters
        .iter()
        .enumerate()
        .map(|(index, emitter)| {
            let anchor = anchors(index, &emitter.def);
            let translation = match &anchor {
                Anchor::Fixed(placement) => {
                    placement.transform_point(axes::to_bevy(emitter.def.position))
                }
                _ => Vec3::ZERO,
            };
            // Seeded from the placement and the emitter's slot — two
            // campfires must not flicker in unison, and a deterministic seed
            // costs no shared state.
            let rng = (translation.x.to_bits() ^ translation.z.to_bits().rotate_left(16))
                .wrapping_add((index as u32).wrapping_mul(0x9E37_79B9))
                | 1;
            let size_scale = if emitter.def.flags & particle_flags::SCALE_BY_INSTANCE != 0 {
                scale
            } else {
                1.0
            };
            let frame = match &anchor {
                Anchor::Fixed(placement) => placement.compute_affine(),
                _ => bevy::math::Affine3A::IDENTITY,
            };
            let field_drawn = merged(&emitter.def);
            let mut spawned = commands.spawn((
                Emitter {
                    set: Arc::clone(set),
                    index,
                    owner,
                    anchor,
                    scale,
                    pad: cloud_pad(&emitter.def, size_scale),
                    pool: Vec::new(),
                    acc: 0.0,
                    age: 0.0,
                    gate_was_on: false,
                    rng,
                    origin: translation,
                    frame,
                    range,
                    instances: Vec::new(),
                    peak: 0,
                    output: 0.0,
                },
                Transform::from_translation(translation),
                // Hidden until the first particle is born, so the empty mesh is
                // never queued — and a [`merged`] emitter stays hidden for ever,
                // because its quads are drawn by its material's field. That is
                // the *drawing* half only — it does not keep a mesh out of the
                // allocator, which is why the mesh below is a degenerate quad
                // rather than nothing; see [`nothing_drawn`].
                Visibility::Hidden,
                // The cull volume. The mesh changes shape every frame, so
                // Bevy's mesh-derived box would describe last frame's cloud
                // and is never computed (the entity arrives with its own);
                // `simulate` grows this one from the pool's own bounds. What
                // it buys is not the pixel — the `VisibilityRange` already
                // decides that — it is the **mesh rewrite**: an emitter out of
                // frustum keeps simulating but stops paying geometry, asset
                // extraction and a sorted-phase draw every frame, which
                // measured as most of the particle system's whole cost. A
                // merged emitter keeps it for the same test done by hand —
                // [`merge_fields`] reads it against the frustum.
                Aabb::from_min_max(Vec3::ZERO, Vec3::ZERO),
            ));
            // A merged emitter carries no mesh, no material and no
            // `VisibilityRange` of its own: it is simulation state and an
            // anchor, and giving it a mesh anyway would put a degenerate quad
            // per flame into the allocator for a draw that never happens.
            if !field_drawn {
                spawned.insert((
                    Mesh3d(meshes.add(particle_mesh())),
                    MeshMaterial3d(emitter.material.clone()),
                ));
                if let Some(range) = range {
                    spawned
                        .insert(bevy::camera::visibility::VisibilityRange::abrupt(0.0, range));
                }
            }
            spawned.id()
        })
        .collect()
}

/// How far past a particle's centre its quads can reach, from the def's own
/// extremes: the largest over-life size this emitter can draw (times the
/// instance scale it will draw it at), plus the longest tail a particle can
/// trail — emission speed at full variation, plus what gravity adds over a
/// whole lifespan, for `tail_time` seconds. Deliberately generous: the box is
/// a cull volume, and the cost of too big is a flame drawn a frame early at
/// the frustum's edge where too small is one missing in plain view.
fn cloud_pad(def: &M2Particle, size_scale: f32) -> f32 {
    let size = def.scales.iter().fold(0.0f32, |a, &b| a.max(b.abs())) * size_scale;
    let speed = first(&def.emission_speed, 0.0).abs()
        * (1.0 + first(&def.speed_variation, 0.0).abs());
    let fall = first(&def.gravity, 0.0).abs() * first(&def.lifespan, 0.0).max(0.05);
    // ×1.5 covers a spun quad's diagonal (√2, rounded up).
    size * 1.5 + (speed + fall) * def.tail_time.max(0.0)
}

pub struct ParticlePlugin;

impl Plugin for ParticlePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ParticleFields>().add_systems(
            Update,
            (
                // **Stated, not inherited**: the instance pass reads the pool
                // and the anchor `simulate` writes, so a frame-late run would
                // draw every shard of every cast one frame behind the cloud it
                // belongs to.
                model_particles::draw.after(simulate),
                // …and the field pass reads the pool, the origin, the frame
                // and the `Aabb` `simulate` writes, so a frame-late run would
                // draw every additive cloud one frame behind its emitter.
                merge_fields.after(simulate),
                retire_emitters,
                // **After the joints are posed and the camera is placed, and
                // stated rather than inherited** — the rule every ordering in
                // this project follows. A frame-late joint hangs the flame a
                // stride behind a running torchbearer; a frame-late camera
                // basis skews every billboard by one frame of orbit.
                simulate
                    .after(retire_emitters)
                    .after(crate::world::camera::place)
                    .after(crate::world::entities::animate)
                    .after(crate::render::doodads::stream_doodads),
            ),
        );
    }
}

/// Despawn emitters whose owner is gone.
///
/// A root entity does not despawn with the tile or the world entity it
/// belongs to, so the tie is by liveness — the same every-frame comparison
/// `retire_colliders` makes, for the same reason: an orphan here is a flame
/// burning in an empty field.
fn retire_emitters(
    mut commands: Commands,
    emitters: Query<(Entity, &Emitter)>,
    owners: Query<()>,
) {
    for (entity, emitter) in &emitters {
        if owners.get(emitter.owner).is_err() {
            commands.entity(entity).despawn();
        }
    }
}

/// Step every emitter; rewrite the quad mesh of the ones that were drawn.
///
/// The split is the system's whole economy: the *pool* advances for every
/// emitter every frame (a campfire seen again is mid-burn, not relit), but
/// the mesh — the geometry build, the asset re-extraction and the allocator
/// churn it drags behind it — is rebuilt only for emitters whose draw
/// survived last frame's cull. `ViewVisibility` is written in `PostUpdate`,
/// so the read is one frame stale: an emitter crossing into the frustum
/// shows one frame of its last-written cloud, which at the distances a
/// frustum edge sits at is not findable by eye. Measured before the gate,
/// 237 Northshire emitters cost 4.4 ms of a 14.0 ms frame with the GPU flat —
/// most of it for flames behind the camera.
// The query tuple is one `Option` past clippy's complexity bar, and a type
// alias for one system's own parameter would name nothing anything else uses.
#[allow(clippy::type_complexity)]
pub(crate) fn simulate(
    time: Res<Time>,
    // **The switch, folded in rather than applied over the top.** This system
    // owns every emitter's `Visibility` — see the `wanted` write below — so a
    // switchboard writing `Hidden` from outside would be overwritten on the
    // next frame a pool was non-empty. It is also the *whole* subtraction here:
    // the loop below is the pass's cost, and turning it off stops the
    // simulation as well as the draw.
    //
    // What it is **not** is `VALE_NO_PARTICLES`, which spawns no emitters at
    // all and is still the honest way to price the pass — this leaves the
    // entities, their meshes and their materials exactly where they were. See
    // [`crate::render::tuning`].
    tuning: Res<crate::render::tuning::WorldTuning>,
    mut meshes: ResMut<Assets<Mesh>>,
    camera: Query<&GlobalTransform, With<crate::world::camera::WorldCamera>>,
    frames: Query<&GlobalTransform, Without<Emitter>>,
    mut emitters: Query<(
        &mut Emitter,
        // Absent on a [`merged`] emitter, whose quads are a field's — see
        // `spawn_emitters`.
        Option<&Mesh3d>,
        &mut Transform,
        &mut Visibility,
        &ViewVisibility,
        &mut Aabb,
    )>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Particles);
    if !tuning.particles {
        // One pass to put them away, on the frame the switch moved, and then
        // nothing at all — including for emitters that stream in afterwards,
        // which is what the `is_changed` guard would have missed.
        for (_, _, _, mut visibility, _, _) in &mut emitters {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
        }
        return;
    }
    // The reference clamps its step too: a hitch must not teleport the smoke.
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    let now_ms = (time.elapsed_secs_f64() * 1000.0) as u32;
    let Ok(cam) = camera.single() else {
        return;
    };
    let cam_pos = cam.translation();
    let rotation = cam.rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);

    for (mut emitter, mesh3d, mut transform, mut visibility, seen, mut aabb) in &mut emitters {
        let emitter = &mut *emitter;
        // The emitter's live frame. A joint that vanished mid-life (a rebuild
        // in progress) holds the pool where it was for a frame.
        let frame = match &emitter.anchor {
            Anchor::Fixed(placement) => placement.compute_affine(),
            Anchor::Joint(joint) => match frames.get(*joint) {
                Ok(gt) => gt.affine(),
                Err(_) => continue,
            },
            Anchor::Owner => match frames.get(emitter.owner) {
                Ok(gt) => gt.affine(),
                Err(_) => continue,
            },
        };
        let origin = frame.transform_point3(axes::to_bevy(emitter.def().position));

        emitter.age += dt;
        emitter.origin = origin;
        emitter.frame = frame;
        step(emitter, dt, now_ms, frame, origin, cam_pos);

        let field_drawn = merged(emitter.def());

        // The anchor is what the transparent phase sorts this draw by.
        // Written only when it moves — a doodad's never does — so a thousand
        // parked flames do not dirty transform propagation every frame.
        if transform.translation != origin {
            transform.translation = origin;
        }

        // A merged emitter's own entity is never shown: its material's field
        // draws for it, and showing this one too would draw the cloud twice.
        let wanted = if emitter.pool.is_empty() || field_drawn {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
        if emitter.pool.is_empty() {
            continue;
        }

        // The cull volume, from the pool's own bounds — kept fresh whether or
        // not the draw was culled, or a cloud that drifted while off screen
        // would come back wearing the box it left with. Written only on a
        // quarter-yard move, so a steady flame does not dirty the render
        // world's change detection every frame just by flickering.
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &emitter.pool {
            lo = lo.min(p.pos);
            hi = hi.max(p.pos);
        }
        let pad = Vec3::splat(emitter.pad);
        let grown = Aabb::from_min_max(lo - pad - origin, hi + pad - origin);
        if (Vec3::from(aabb.center) - Vec3::from(grown.center)).length_squared() > 0.0625
            || (Vec3::from(aabb.half_extents) - Vec3::from(grown.half_extents)).length_squared()
                > 0.0625
        {
            *aabb = grown;
        }

        // **A merged emitter has no mesh of its own to rewrite** — its quads
        // are built into its material's field by [`merge_fields`], which runs
        // after this and reads the pool, origin, frame and `Aabb` the loop
        // above just wrote. **A geometry-model emitter has no quads at all**:
        // its particles are drawn as child instances by
        // [`model_particles::draw`], on the same terms.
        if field_drawn || emitter.def().geometry_model.is_some() {
            continue;
        }

        // The gate this system exists to hold: an emitter whose draw was
        // culled last frame — by the frustum or by its `VisibilityRange` —
        // skips the rewrite entirely. The pool above already advanced.
        if !seen.get() {
            continue;
        }
        if let Some(mut mesh) = mesh3d.and_then(|m| meshes.get_mut(&m.0)) {
            let mut quads = QuadBuffers::default();
            fill_quads(emitter, origin, right, up, frame, &mut quads);
            write_quads(quads, &mut mesh);
        }
    }
}

/// Advance one emitter by `dt`: age and integrate the pool, then birth what
/// the rate owes.
fn step(
    emitter: &mut Emitter,
    dt: f32,
    now_ms: u32,
    frame: bevy::math::Affine3A,
    origin: Vec3,
    cam_pos: Vec3,
) {
    let set = Arc::clone(&emitter.set);
    let def = &set.emitters[emitter.index].def;
    let clip = &set.clip;

    // The client's integrator: age/kill, position step, gravity
    // on the up axis, then drag — and the sphere kill-outbound tail test
    // against the pre-gravity, pre-drag step velocity, in that byte order.
    let gravity = first(&def.gravity, 0.0);
    let kill_outbound =
        def.emitter_type == 2 && def.flags & particle_flags::KILL_OUTBOUND != 0;
    emitter.pool.retain_mut(|p| {
        p.age += dt;
        if p.age >= p.life {
            return false;
        }
        // The model-particle tumble: a Rodrigues half-angle delta
        // right-multiplied in the **body** frame, skipped below the
        // reference's own 1e-4 threshold. A quad carries zero here and pays
        // one length check.
        let theta = p.angvel.length();
        if theta > 1e-4 {
            p.quat = (p.quat * Quat::from_axis_angle(p.angvel / theta, theta * dt)).normalize();
        }
        let step_vel = p.vel;
        p.pos += p.vel * dt;
        p.pos.y -= 0.5 * gravity * dt * dt;
        p.vel.y -= gravity * dt;
        if def.drag != 0.0 {
            p.vel -= (dt * def.drag).min(1.0) * p.vel;
        }
        if kill_outbound && step_vel.dot(p.pos - origin) > 0.0 {
            return false;
        }
        true
    });

    // The clip clock, and the two tracks that follow it per frame.
    let clip_ms = clip_time(clip, emitter.age);
    let gate_on = gate(&def.enabled, clip, clip_ms, now_ms);
    let rate = if gate_on {
        sample(&def.emission_rate, clip, clip_ms, now_ms).max(0.0)
    } else {
        0.0
    };

    // The emission LOD: full rate inside 50 yards, a linear
    // falloff past it, a 25% floor, never zero.
    let lod = (1.0 - (origin.distance(cam_pos) - LOD_FULL_RATE) * LOD_FALLOFF)
        .clamp(LOD_FLOOR, 1.0);

    let gate_open = rate > 0.0;
    if def.flags & particle_flags::BURST != 0 {
        // One shot on the gate's rising edge, not a steady rate.
        if gate_open && !emitter.gate_was_on {
            emitter.acc = (rate * lod).trunc();
        }
    } else if gate_open {
        emitter.acc += rate * lod * dt;
    } else {
        // A closed gate zeroes the owed fraction; live particles finish.
        emitter.acc = 0.0;
    }
    emitter.gate_was_on = gate_open;

    let speed = first(&def.emission_speed, 0.0);
    let variation = first(&def.speed_variation, 0.0);
    let life = first(&def.lifespan, 0.0).max(0.05);
    // A model particle is born wearing the emitter's own basis, and the
    // orientation is the *whole* of what a body has that a billboard does not
    // — so it is derived once per step rather than per birth.
    let model_particle = def.geometry_model.is_some();
    let birth_quat = model_particle.then(|| {
        // The same +90° about local Z every kernel result takes:
        // WoW's local +Z is Bevy's +Y, so the turn rides as a
        // yaw appended to the frame's rotation.
        let (_, rotation, _) = frame.to_scale_rotation_translation();
        rotation * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
    });
    while emitter.acc >= 1.0 && emitter.pool.len() < MAX_PARTICLES {
        emitter.acc -= 1.0;
        let (local, dir) = birth_kernel(def, &mut emitter.rng);
        let pos = frame.transform_point3(axes::to_bevy(add3(def.position, local)));
        let vel = frame
            .transform_vector3(axes::to_bevy(dir))
            .normalize_or_zero()
            * (speed * (1.0 + variation * s11(&mut emitter.rng)));
        let seed = xorshift(&mut emitter.rng);
        let angvel = match model_particle {
            true => tumble(def, &mut emitter.rng),
            false => Vec3::ZERO,
        };
        emitter.pool.push(Particle {
            pos,
            vel,
            age: 0.0,
            life,
            seed,
            quat: birth_quat.unwrap_or(Quat::IDENTITY),
            angvel,
        });
    }

    // **What this emitter is putting out**, for whoever is lighting the world
    // off it — see [`Emitter::output`]. After the retain and the births, so it
    // is this frame's count and not last frame's.
    emitter.peak = emitter.peak.max(emitter.pool.len());
    let now = match emitter.peak {
        0 => 0.0,
        peak => emitter.pool.len() as f32 / peak as f32,
    };
    // A quarter-second time constant, frame-rate independent. Short enough that
    // a light still follows its effect and long enough to take the emission
    // accumulator's own beat out of it.
    const OUTPUT_SECONDS: f32 = 0.25;
    let blend = (dt / OUTPUT_SECONDS).clamp(0.0, 1.0);
    emitter.output += (now - emitter.output) * blend;
}

/// One model particle's angular velocity, drawn at birth from the emitter's
/// `tumble_min`/`tumble_max` — **and the asymmetry between the axes is the
/// file's, not a slip**.
///
/// Only X honours `min + u·range`; Y and Z multiply a raw `[1, 2)` mantissa by
/// their *range* alone, so their authored minimum is dead. That is the 5875
/// client's own arithmetic,
/// and it is load-bearing rather than cosmetic: Cone of Cold's shards author
/// `[0,0,0]..[0,0,−10]`, which under the X rule would be a uniform −10 and
/// under the real one is −10..−20 rad/s — the difference between a rigid
/// picture turning as one and a churning cloud.
///
/// Flag 0x200 then sign-flips each axis independently.
fn tumble(def: &M2Particle, rng: &mut u32) -> Vec3 {
    let (lo, hi) = (def.tumble_min, def.tumble_max);
    let mut w = [
        lo[0] + rand01(rng) * (hi[0] - lo[0]),
        (1.0 + rand01(rng)) * (hi[1] - lo[1]),
        (1.0 + rand01(rng)) * (hi[2] - lo[2]),
    ];
    if def.flags & particle_flags::TUMBLE_RANDOM_SIGN != 0 {
        for axis in &mut w {
            if xorshift(rng) & 1 == 0 {
                *axis = -*axis;
            }
        }
    }
    axes::to_bevy(w)
}

/// Where and which way one particle is born, in the emitter's own frame —
/// the client's shape kernels, including the +90° turn about local Z that
/// every kernel result takes on the way out.
fn birth_kernel(def: &M2Particle, rng: &mut u32) -> ([f32; 3], [f32; 3]) {
    let vertical = first(&def.vertical_range, 0.0);
    let horizontal = first(&def.horizontal_range, 0.0);
    let length = first(&def.area_length, 0.0);
    let width = first(&def.area_width, 0.0);

    let (pos, mut dir) = match def.emitter_type {
        // Sphere: radius uniform in [length, width], a latitude
        // and longitude band, and the velocity is the same unit shell vector
        // — reusing the pair is what keeps a zero-radius sphere spraying
        // outward instead of collapsing.
        2 => {
            let r = length + rand01(rng) * (width - length).max(0.0);
            let (lat, lon) = (s11(rng) * vertical, s11(rng) * horizontal);
            let (slat, clat) = lat.sin_cos();
            let (slon, clon) = lon.sin_cos();
            let shell = [clat * clon, clat * slon, slat];
            let dir = if def.flags & particle_flags::SPHERE_UP != 0 {
                [0.0, 0.0, 1.0]
            } else {
                shell
            };
            ([r * shell[0], r * shell[1], r * shell[2]], dir)
        }
        // Spline (type 3): born at the chain's head — a named deviation, 23
        // emitters in the whole game.
        3 => (def.spline.first().copied().unwrap_or([0.0; 3]), [0.0, 0.0, 1.0]),
        // Plane: uniform in the ±half rectangle — **length on
        // local X, width on local Y**, in that order: the
        // kernel multiplies its first rand by areaLength into x and its
        // second by areaWidth into y, each times 0.5. Direction a
        // symmetric cone about +Z.
        _ => {
            let pos = [s11(rng) * 0.5 * length, s11(rng) * 0.5 * width, 0.0];
            let (theta, phi) = (s11(rng) * vertical, s11(rng) * horizontal);
            let (st, ct) = theta.sin_cos();
            let (sp, cp) = phi.sin_cos();
            (pos, [st * cp, st * sp, ct])
        }
    };

    // zSource on any shape: radial from a pivot below the emitter.
    let z_source = first(&def.z_source, 0.0);
    if z_source != 0.0 {
        let v = [pos[0], pos[1], pos[2] - z_source];
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        dir = if len > 1e-4 {
            [v[0] / len, v[1] / len, v[2] / len]
        } else {
            [0.0, 0.0, 1.0]
        };
    }

    (rot90(pos), rot90(dir))
}

/// The fixed +90° about local Z every kernel result takes.
/// Dropping it turns every rectangular emitter in the world a quarter turn.
fn rot90(v: [f32; 3]) -> [f32; 3] {
    [-v[1], v[0], v[2]]
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The vertex streams one frame of quads accumulates before they become a
/// mesh — one emitter's on the per-emitter path, **every visible member's**
/// on a field's ([`merge_fields`]), which is the whole reason this is a
/// struct rather than five locals inside the fill.
#[derive(Default)]
struct QuadBuffers {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

/// Move one frame's quads into a mesh — or, when nothing was built, the
/// degenerate quad, for the allocator reason [`nothing_drawn`] states: a live
/// pool does not guarantee live *geometry*, since every particle in it can be
/// sitting on a zero scale key at once, and writing empty vectors through
/// would put a zero-vertex mesh back into the allocator every frame for as
/// long as that lasted.
fn write_quads(quads: QuadBuffers, mesh: &mut Mesh) {
    if quads.positions.is_empty() {
        nothing_drawn(mesh);
        return;
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, quads.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, quads.normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, quads.uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, quads.colors);
    mesh.insert_indices(Indices::U32(quads.indices));
}

/// Build the emitter's quads: one or two per particle, positions relative to
/// the anchor so the drawing entity's transform stays the sort key —
/// **appended**, so a field can fold many emitters into one buffer set.
///
/// `frame` is the emitter's live frame — the placement, the posed joint, or
/// the attachment root — and it is what an XY-quad emitter's plane comes from.
fn fill_quads(
    emitter: &Emitter,
    anchor: Vec3,
    right: Vec3,
    up: Vec3,
    frame: bevy::math::Affine3A,
    out: &mut QuadBuffers,
) {
    let def = emitter.def();
    let quads = emitter.pool.len() * if def.head_tail == 2 { 2 } else { 1 };
    out.positions.reserve(quads * 4);
    out.normals.reserve(quads * 4);
    out.uvs.reserve(quads * 4);
    out.colors.reserve(quads * 4);
    out.indices.reserve(quads * 6);

    let inv_cols = 1.0 / f32::from(def.columns);
    let inv_rows = 1.0 / f32::from(def.rows);
    let size_scale = if def.flags & particle_flags::SCALE_BY_INSTANCE != 0 {
        emitter.scale
    } else {
        1.0
    };
    // The head basis: the camera's, or — for an XY-quad emitter — the
    // emitter's own plane, camera-independent. The XY basis carries the same
    // +90° turn the kernels take, written out as the two axes it maps to.
    // **From the live frame, whatever the anchor** — a joint-anchored XY quad
    // (the glow lying along a sword's blade) used to fall back to the camera
    // basis, which stood every such quad up to face the viewer instead of
    // lying in its bone's own plane.
    let (head_right, head_up) = if def.flags & particle_flags::XY_QUAD != 0 {
        (
            frame
                .transform_vector3(axes::to_bevy([0.0, 1.0, 0.0]))
                .normalize_or_zero(),
            frame
                .transform_vector3(axes::to_bevy([-1.0, 0.0, 0.0]))
                .normalize_or_zero(),
        )
    } else {
        (right, up)
    };

    for p in &emitter.pool {
        let u = (p.age / p.life).clamp(0.0, 1.0);
        let (color, size, cell) = over_life(def, u);
        let half = size * size_scale;
        if half <= 0.0 {
            continue;
        }
        // The atlas cell. v grows downward, like every texture here.
        let cell = u32::from(cell).min(u32::from(def.rows) * u32::from(def.columns) - 1);
        let (cx, cy) = (
            (cell % u32::from(def.columns)) as f32,
            (cell / u32::from(def.columns)) as f32,
        );
        let (u0, u1) = (cx * inv_cols, (cx + 1.0) * inv_cols);
        let (v0, v1) = (cy * inv_rows, (cy + 1.0) * inv_rows);
        let center = p.pos - anchor;

        // The quad spin (`spin·age`): a negative angle
        // is negated on the half of the pool whose slot hash carries bit 5.
        if def.head_tail != 1 {
            let mut angle = def.spin * p.age;
            if angle < 0.0 && p.seed & 0x20 != 0 {
                angle = -angle;
            }
            let (s, c) = angle.sin_cos();
            let r = (head_right * c + head_up * s) * half;
            let u_axis = (head_up * c - head_right * s) * half;
            push_quad(
                &mut out.positions,
                &mut out.uvs,
                &mut out.indices,
                [
                    center - r - u_axis,
                    center + r - u_axis,
                    center + r + u_axis,
                    center - r + u_axis,
                ],
                [[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
            );
            out.normals.extend([[0.0, 1.0, 0.0]; 4]);
            out.colors.extend([color; 4]);
        }
        // The tail quad: a streak `|velocity| × tail_time` long,
        // projected into the view plane, width `2·half` perpendicular to it.
        if def.head_tail >= 1 {
            let t_eff = if def.flags & particle_flags::TAIL_GROWS != 0 {
                def.tail_time.min(p.age)
            } else {
                def.tail_time
            };
            let tail = -p.vel * t_eff;
            let (tr, tu) = (tail.dot(right), tail.dot(up));
            let l2 = tr * tr + tu * tu;
            if l2 < 7.7e-4 {
                // Degenerate: the reference's plain-billboard fallback.
                let r = right * half;
                let u_axis = up * half;
                push_quad(
                    &mut out.positions,
                    &mut out.uvs,
                    &mut out.indices,
                    [
                        center - r - u_axis,
                        center + r - u_axis,
                        center + r + u_axis,
                        center - r + u_axis,
                    ],
                    [[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
                );
            } else {
                let perp = (up * tr - right * tu) * (half / l2.sqrt());
                push_quad(
                    &mut out.positions,
                    &mut out.uvs,
                    &mut out.indices,
                    [
                        center - perp,
                        center + perp,
                        center + tail + perp,
                        center + tail - perp,
                    ],
                    [[u0, v1], [u0, v0], [u1, v0], [u1, v1]],
                );
            }
            out.normals.extend([[0.0, 1.0, 0.0]; 4]);
            out.colors.extend([color; 4]);
        }
    }
}

fn push_quad(
    positions: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    corners: [Vec3; 4],
    quad_uvs: [[f32; 2]; 4],
) {
    let base = positions.len() as u32;
    positions.extend(corners.map(|c| c.to_array()));
    uvs.extend(quad_uvs);
    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// The one entity a material's whole additive population is drawn by — its
/// own marker so the field pass can address its `Transform` and `Visibility`
/// without ever matching an emitter.
#[derive(Component)]
pub struct ParticleField;

/// Every merged field, keyed by the interned material the members share.
///
/// The key is free: [`build_set`] interns every emitter material through
/// [`Materials::intern`], so two torches of one model carry the *same*
/// handle, and equality of ids is equality of pipeline state, texture and
/// blend — everything a draw call is.
///
/// **…and by the layers its members are drawn on.** A field is one mesh and a
/// mesh carries one `RenderLayers`, so members a camera cannot see must not be
/// drawn through a field it can: an emitter on a layer of its own goes into a
/// field of its own. In a session nothing carries the component and every
/// member shares the default, so there is one field per material exactly as
/// before; a host that puts a unit on another layer — a preview drawn into an
/// image — gets that unit's clouds on the same layer as the unit.
#[derive(Resource, Default)]
pub struct ParticleFields {
    fields: HashMap<(AssetId<M2Material>, u64), Field>,
    /// How many emitters were drawn *through* a field this frame — the HUD's
    /// half of the story the per-emitter `ViewVisibility` count can no longer
    /// tell, since a merged emitter's own entity is never visible.
    pub merged: usize,
    /// …and how few draws they became, which is the number this pass exists
    /// to hold down.
    pub drawn: usize,
}

struct Field {
    entity: Entity,
    mesh: Handle<Mesh>,
}

/// The layers an emitter is drawn on, as the bits of a key. Layer 0 alone —
/// what an entity with no component is on — is bit 0, so the default and an
/// explicit `RenderLayers::layer(0)` share a field.
fn layer_bits(layers: Option<&RenderLayers>) -> u64 {
    match layers {
        Some(layers) => layers.iter().fold(0u64, |bits, layer| bits | 1u64 << (layer % 64)),
        None => 1,
    }
}

/// Build every additive emitter's quads into one mesh per material — the
/// same decision [`crate::render::shadows`] takes for the blobs, one module
/// over: **when the phase cannot batch for you, batch before you reach it.**
///
/// ## Where the field sorts, and what that trades
///
/// Bevy sorts a transparent item by its translation, and a merged field has
/// one translation for many clouds — so it is put **at the visible member
/// nearest the camera**. Order *within* the field cannot matter (additive
/// blends commute; see [`merged`]), and against everything else the choice
/// errs late: a member farther away than the nearest one is drawn after
/// translucent geometry that stands in front of it, so an additive cloud
/// behind a waterfall adds *over* the water instead of being filtered by it.
/// That is the mild error — light bleeding through a translucent surface
/// reads as glow — where the shadow field's origin trick would have been the
/// harsh one here: a flame in *front* of the water dimmed by it. The blobs
/// could afford origin-sorting only because they lie on the ground and the
/// depth test rejects everything behind it; a flame is in the open air.
///
/// ## Culling is this pass's own
///
/// A member's draw used to be culled by Bevy — the frustum against its
/// `Aabb`, the `VisibilityRange` against its distance. One mesh gets
/// neither, so both tests are done here per member, from the same `Aabb`
/// [`simulate`] keeps fresh and the same range the spawn recorded. An
/// emitter that fails either is simply not in this frame's field — which is
/// the same economy the `ViewVisibility` gate bought the per-emitter path:
/// its pool advances and its geometry costs nothing.
#[allow(clippy::too_many_arguments)]
fn merge_fields(
    mut commands: Commands,
    tuning: Res<crate::render::tuning::WorldTuning>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut fields: ResMut<ParticleFields>,
    camera: Query<(&GlobalTransform, &Frustum), With<crate::world::camera::WorldCamera>>,
    emitters: Query<(Entity, &Emitter, &Aabb, Option<&RenderLayers>)>,
    mut views: Query<(&mut Transform, &mut Visibility), With<ParticleField>>,
) {
    fields.merged = 0;
    fields.drawn = 0;
    // The switch, folded in on the same terms as `simulate`'s: an empty,
    // hidden field is no draw at all. The fields themselves are kept — the
    // subtraction is the draw and the geometry build, not the bookkeeping.
    if !tuning.particles {
        for (_, mut visibility) in &mut views {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
        }
        return;
    }
    let Ok((cam, frustum)) = camera.single() else {
        return;
    };
    let cam_pos = cam.translation();
    let rotation = cam.rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);

    // Which members each material has this frame, and which of them survive
    // the cull. Membership is counted even for an empty pool, because it is
    // what keeps a field alive between a campfire's pulses — a field is
    // retired only when *no* live emitter names its material at all.
    struct Group {
        material: Handle<M2Material>,
        /// The members' own layers, put on the field when it is born — see
        /// [`ParticleFields`].
        layers: Option<RenderLayers>,
        visible: Vec<Entity>,
        nearest: f32,
        anchor: Vec3,
    }
    let mut groups: HashMap<(AssetId<M2Material>, u64), Group> = HashMap::default();
    for (entity, emitter, aabb, layers) in &emitters {
        if !merged(emitter.def()) {
            continue;
        }
        let material = &emitter.set.emitters[emitter.index].material;
        let group = groups
            .entry((material.id(), layer_bits(layers)))
            .or_insert_with(|| Group {
                material: material.clone(),
                layers: layers.cloned(),
                visible: Vec::new(),
                nearest: f32::MAX,
                anchor: Vec3::ZERO,
            });
        if emitter.pool.is_empty() {
            continue;
        }
        let distance = emitter.origin.distance(cam_pos);
        if emitter.range.is_some_and(|range| distance > range) {
            continue;
        }
        if !frustum.intersects_sphere(
            &Sphere {
                center: (emitter.origin + Vec3::from(aabb.center)).into(),
                radius: Vec3::from(aabb.half_extents).length(),
            },
            false,
        ) {
            continue;
        }
        if distance < group.nearest {
            group.nearest = distance;
            group.anchor = emitter.origin;
        }
        group.visible.push(entity);
    }

    for (id, group) in &groups {
        let mut quads = QuadBuffers::default();
        for &member in &group.visible {
            if let Ok((_, emitter, _, _)) = emitters.get(member) {
                fill_quads(emitter, group.anchor, right, up, emitter.frame, &mut quads);
            }
        }
        // Nothing built — every member culled, gated shut, or sitting on a
        // zero scale key: the field hides rather than writing an empty mesh
        // into the allocator (the same rule as [`nothing_drawn`], one level
        // up, where hiding is available because the field owns a draw).
        if quads.positions.is_empty() {
            if let Some(field) = fields.fields.get(id) {
                if let Ok((_, mut visibility)) = views.get_mut(field.entity) {
                    if *visibility != Visibility::Hidden {
                        *visibility = Visibility::Hidden;
                    }
                }
            }
            continue;
        }
        fields.merged += group.visible.len();
        fields.drawn += 1;
        if let Some(field) = fields.fields.get(id) {
            if let Some(mut mesh) = meshes.get_mut(&field.mesh) {
                write_quads(quads, &mut mesh);
            }
            if let Ok((mut transform, mut visibility)) = views.get_mut(field.entity) {
                if transform.translation != group.anchor {
                    transform.translation = group.anchor;
                }
                if *visibility != Visibility::Inherited {
                    *visibility = Visibility::Inherited;
                }
            }
        } else {
            // The first frame this material has something to draw: the field
            // is born already carrying it, so there is no hidden first frame.
            let mut mesh = particle_mesh();
            write_quads(quads, &mut mesh);
            let handle = meshes.add(mesh);
            let entity = commands
                .spawn((
                    ParticleField,
                    Mesh3d(handle.clone()),
                    MeshMaterial3d(group.material.clone()),
                    Transform::from_translation(group.anchor),
                    // One draw whose extent changes every frame — the frustum
                    // test was already taken per member above.
                    NoFrustumCulling,
                    bevy::light::NotShadowCaster,
                ))
                .id();
            if let Some(layers) = &group.layers {
                commands.entity(entity).insert(layers.clone());
            }
            fields.fields.insert(*id, Field { entity, mesh: handle });
        }
    }

    // A material no live emitter names any more takes its field down with it,
    // which is also what lets `residency` evict the material itself: the
    // field's `MeshMaterial3d` was the last strong handle.
    fields.fields.retain(|id, field| {
        if groups.contains_key(id) {
            return true;
        }
        commands.entity(field.entity).despawn();
        false
    });
}

/// **Model particles: the emitters whose particles are not quads at all.**
///
/// An emitter's record carries an `M2Array<char>` at +0x18 naming another
/// model, and when it is set the client draws each live particle as a small
/// three-dimensional instance of that file —
/// oriented by the particle's own quaternion, scaled by the over-life size
/// ramp, tinted by the over-life colour — and never reads the emitter's own
/// texture slot at all.
///
/// **This is the field whose absence drew Cone of Cold and Evocation as
/// slabs, and it is worth saying why three rounds of work on the quad path
/// could not have found it.** Eight of Cone of Cold's eleven emitters and five
/// of Evocation's name `Spells\ConeofCold_Geo.mdx` and
/// `Spells\CycloneGeo*_Additive.mdx`; the `SPELLS\CLOUDS.BLP` in their texture
/// slot is a 256x256 DXT1 sheet with `alphaDepth = 0` — **no alpha channel at
/// all**, mean luminance 85 of 255, corners at 155, nothing black anywhere in
/// it. There is no blend mode, alpha reference, over-life ramp or size rule
/// that turns that image into a cloud on a billboard, because it was never
/// meant to be drawn. Every measurement of the quad path was true and the
/// question was wrong. `vale particles` counts the population now: **55
/// emitters over 13 distinct files**, all present in the archive.
///
/// ## What this pass does and does not do
///
/// Each instance is a **child of the emitter entity**, so the pool is torn
/// down with the emitter by the engine rather than by a retirement sweep of
/// its own — the emitter's transform is a pure translation (its anchor), which
/// makes a child's local transform exactly `world − anchor`, the same relative
/// frame [`fill_quads`] writes its vertices in.
///
/// The pool is **grown and reused, never respawned per frame**: a particle
/// slot past the live count is hidden rather than despawned, so a 25-per-second
/// emitter does not churn a dozen entities and their bind groups every frame.
///
/// What is deliberately not modelled, on the same terms as the rest of this
/// module's deviations: a **rigged** geometry model is drawn in its bind pose
/// (every file in the 13 is static), the reference's optional per-emitter depth
/// sort is left to the transparent phase's own ordering, and a recursion model
/// — the child-emitter path at +0x20 — is still unread.
mod model_particles {
    use super::*;
    use crate::render::models::{Lookup, ModelCache};
    use bevy::mesh::MeshTag;

    /// A hard cap on one emitter's instance pool, far above any authored
    /// steady state: the largest model emitter in the game pours 25/s over a
    /// 0.9 s life, which is 23 alive.
    const MAX_INSTANCES: usize = 128;

    /// Grow and place every model emitter's instances from the pool
    /// [`super::simulate`] has just advanced.
    pub(super) fn draw(
        mut commands: Commands,
        tuning: Res<crate::render::tuning::WorldTuning>,
        mut cache: ResMut<ModelCache>,
        mut materials: crate::render::models::Materials,
        // Only so the model cache can build a dressing's merged meshes — see
        // `models::loader::MergeSource`.
        mut meshes: ResMut<Assets<Mesh>>,
        mut emitters: Query<(Entity, &mut Emitter)>,
        mut parts: Query<(&mut Transform, &mut Visibility, &mut MeshTag), Without<Emitter>>,
    ) {
        for (entity, mut emitter) in &mut emitters {
            let emitter = &mut *emitter;
            let Some(path) = emitter.def().geometry_model.clone() else {
                continue;
            };
            // The particles switch is the whole subtraction here too — see
            // `simulate`'s note. The pool has already been emptied of new
            // births by then; this puts the bodies away.
            let live = if tuning.particles {
                emitter.pool.len().min(MAX_INSTANCES)
            } else {
                0
            };
            // **The dressing whose `MeshTag` is a colour**, not a room light —
            // see `ModelCache::as_particle`. A model still loading simply
            // draws nothing this frame; the pool goes on simulating.
            let Lookup::Ready(model) = cache.as_particle(&path, &mut meshes, &mut materials) else {
                continue;
            };
            while emitter.instances.len() < live {
                let parts = model
                    .draws
                    .iter()
                    .map(|draw| {
                        commands
                            .spawn((
                                Mesh3d(draw.mesh.clone()),
                                MeshMaterial3d(draw.material.clone()),
                                // Hidden until the loop below places it, so a
                                // freshly grown slot is never drawn at the
                                // origin for one frame.
                                Visibility::Hidden,
                                Transform::default(),
                                MeshTag(0),
                                ChildOf(entity),
                            ))
                            .id()
                    })
                    .collect();
                emitter.instances.push(ModelInstance { parts });
            }
            for (slot, instance) in emitter.instances.iter().enumerate() {
                // A slot past the live count, and a particle sitting on a zero
                // size key, are the same thing to the draw: nothing.
                let placed = emitter.pool.get(slot).filter(|_| slot < live).map(|p| {
                    let u = (p.age / p.life).clamp(0.0, 1.0);
                    let (colour, size, _) = over_life(emitter.def(), u);
                    (p, colour, size * emitter.instance_scale())
                });
                for &part in &instance.parts {
                    let Ok((mut transform, mut visibility, mut tag)) = parts.get_mut(part) else {
                        continue;
                    };
                    match placed {
                        Some((p, colour, size)) if size > 0.0 => {
                            *transform = Transform {
                                translation: p.pos - emitter.origin,
                                rotation: p.quat,
                                scale: Vec3::splat(size),
                            };
                            tag.0 = crate::render::models::tint_tag(colour);
                            if *visibility != Visibility::Inherited {
                                *visibility = Visibility::Inherited;
                            }
                        }
                        _ => {
                            if *visibility != Visibility::Hidden {
                                *visibility = Visibility::Hidden;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The over-life ramp: two linear segments split at `mid_point`, each with
/// its own atlas-cell range — the client's own evaluator.
pub fn over_life(def: &M2Particle, u: f32) -> ([f32; 4], f32, u16) {
    let u = u.clamp(0.0, 1.0);
    let mid = def.mid_point.clamp(1e-3, 1.0);
    let (a, b, t, cells) = if u <= mid {
        (0, 1, u / mid, def.cells[0])
    } else {
        (1, 2, (u - mid) / (1.0 - mid).max(1e-3), def.cells[1])
    };
    let mut color = [0.0f32; 4];
    for (c, out) in color.iter_mut().enumerate() {
        let (ca, cb) = (f32::from(def.colors[a][c]), f32::from(def.colors[b][c]));
        *out = (ca + (cb - ca) * t) / 255.0;
    }
    let size = def.scales[a] + (def.scales[b] - def.scales[a]) * t;
    // The cell walks begin..=end across its segment:
    // `floor(begin + (end - begin + 1) · t)`, clamped onto the range.
    //
    // **…in either direction.** A segment may be authored `[15, 0]` — the
    // second half of `Spells\FarSight_Impact_Base.m2`'s emitter 1 is exactly
    // that, a flipbook played backwards over the sprite's decay — and a walk
    // that assumed `begin <= end` clamped onto an empty range and panicked the
    // frame Eagle Eye's focus first drew (`min > max. min = 15, max = 0`). The
    // step is signed and the clamp is onto the sorted pair.
    let (begin, end) = (i32::from(cells[0]), i32::from(cells[1]));
    let span = ((end - begin).abs() + 1) as f32;
    let step = (span * t).floor() as i32;
    let idx = if end >= begin { begin + step } else { begin - step };
    let cell = idx.clamp(begin.min(end), begin.max(end)) as u16;
    (color, size, cell)
}

/// Seconds of emitter age onto the clip's millisecond clock.
fn clip_time(clip: &Option<ParticleClip>, age: f32) -> u32 {
    let ms = (age * 1000.0) as u32;
    match clip {
        Some(c) => {
            let span = c.end.saturating_sub(c.start).max(1);
            if c.loops {
                ms % span
            } else {
                ms.min(span)
            }
        }
        None => ms,
    }
}

/// A keyed emitter track at the clip clock, or its first key without one.
fn sample(track: &Option<M2Track>, clip: &Option<ParticleClip>, clip_ms: u32, now_ms: u32) -> f32 {
    match (track, clip) {
        (None, _) => 0.0,
        (Some(t), Some(c)) => {
            t.sample(c.start + clip_ms, c.start, c.end, &c.global_sequences, now_ms)[0]
        }
        (Some(t), None) => t.first(),
    }
}

/// The emission gate. No keys means always on (the loader's own default);
/// with keys but no clip to run them on, on if any key is nonzero.
fn gate(track: &Option<M2Track>, clip: &Option<ParticleClip>, clip_ms: u32, now_ms: u32) -> bool {
    match (track, clip) {
        (None, _) => true,
        (Some(t), Some(c)) => {
            t.sample(c.start + clip_ms, c.start, c.end, &c.global_sequences, now_ms)[0] > 0.5
        }
        (Some(t), None) => t.values.iter().any(|&v| v > 0.5),
    }
}

/// The first key of an emitter property track — the "sampled once"
/// convention; see [`M2Track::first`].
fn first(track: &Option<M2Track>, fallback: f32) -> f32 {
    track.as_ref().map(|t| t.first()).unwrap_or(fallback)
}

/// The mesh an emitter starts with. The `COLOR` attribute is present from the
/// start so the pipeline variant never changes shape underneath the material.
///
/// It holds [one zero-area quad](nothing_drawn) rather than nothing at all, for
/// a reason that has nothing to do with what is drawn — see that function.
///
/// **`default()` — `MAIN_WORLD | RENDER_WORLD` — and it is the one mesh in this
/// renderer that cannot be `RENDER_WORLD` alone.** Every other mesh here is
/// built once and never touched again, so it declares `RENDER_WORLD` and
/// `extract_render_asset` *moves* it (`Mesh::take_gpu_data`) instead of cloning
/// it. This one is rewritten every frame it is drawn, and a mesh that has been
/// taken is left as `MeshExtractableData::ExtractedToRenderWorld` — after
/// which `insert_attribute` **panics** (`try_insert_attribute` returns
/// `MeshAccessError::ExtractedToRenderWorld` and the infallible wrapper
/// `expect`s it). So the per-frame clone of the whole cloud is the price of a
/// mesh the CPU may rewrite, and the way out of it is not a usage flag — it is
/// not rewriting a `Mesh` per frame at all.
fn particle_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    nothing_drawn(&mut mesh);
    mesh
}

/// Fill an emitter's mesh with the degenerate quad, declaring the `COLOR` the
/// shared writer will only *refresh*.
///
/// The rule and the whole argument for it are
/// [`crate::render::nothing`]'s — this is the emitter's attribute set laid over
/// it, and the one thing it adds is the colour, which the three other per-frame
/// passes do not all carry.
fn nothing_drawn(mesh: &mut Mesh) {
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; nothing::VERTICES]);
    nothing::nothing_drawn(mesh);
}

// ------------------------------------------------------------------- rng --

pub(crate) fn xorshift(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

/// Uniform in `0..1`.
///
/// `pub(crate)` for one caller outside this file — the persistent areas' own
/// falling-impact procedural, which picks a place inside a radius and a stagger
/// on the same terms an emitter picks a particle. A second xorshift beside this
/// one is exactly the shape of duplication this repo has paid for elsewhere.
pub(crate) fn rand01(state: &mut u32) -> f32 {
    (xorshift(state) >> 8) as f32 / 16_777_216.0
}

/// Uniform in −1..1 — the reference's `S11`, which is what makes every cone
/// and band **symmetric**; a `[0, range)` reading tilts every flame.
fn s11(state: &mut u32) -> f32 {
    rand01(state) * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::m2::M2Track;

    fn plain_def() -> M2Particle {
        M2Particle {
            flags: 0,
            position: [0.0; 3],
            bone: 0,
            texture: 0,
            blend: 3,
            geometry_model: None,
            recursion_model: None,
            emitter_type: 1,
            head_tail: 0,
            rows: 1,
            columns: 1,
            emission_speed: None,
            speed_variation: None,
            vertical_range: None,
            horizontal_range: None,
            gravity: None,
            lifespan: None,
            emission_rate: None,
            area_length: None,
            area_width: None,
            z_source: None,
            mid_point: 0.5,
            colors: [[255, 0, 0, 255], [0, 255, 0, 128], [0, 0, 255, 0]],
            scales: [1.0, 2.0, 4.0],
            cells: [[0, 1], [2, 3]],
            tail_time: 0.0,
            twinkle_speed: 0.0,
            twinkle_percent: 0.0,
            twinkle_scale: [0.0; 2],
            inherit_scale: 0.0,
            drag: 0.0,
            spin: 0.0,
            tumble_min: [0.0; 3],
            tumble_max: [0.0; 3],
            follow_speed1: 0.0,
            follow_scale1: 0.0,
            follow_speed2: 0.0,
            follow_scale2: 0.0,
            spline: Vec::new(),
            enabled: None,
        }
    }

    /// The over-life ramp is two segments split at the midpoint, and each
    /// half owns its own atlas-cell range — the property a single 0..1 lerp
    /// (which is what the reference viewer did) gets wrong on both counts.
    #[test]
    fn the_over_life_ramp_breaks_at_the_midpoint() {
        let def = plain_def();
        // At birth: the first key exactly.
        let (color, size, cell) = over_life(&def, 0.0);
        assert_eq!(color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(size, 1.0);
        assert_eq!(cell, 0);
        // At the midpoint: the middle key, and the first segment's last cell.
        let (color, size, cell) = over_life(&def, 0.5);
        assert_eq!(color, [0.0, 1.0, 0.0, 128.0 / 255.0]);
        assert_eq!(size, 2.0);
        assert_eq!(cell, 1);
        // Past it: the second segment interpolates toward the death key over
        // its own cells.
        let (color, size, cell) = over_life(&def, 0.75);
        assert_eq!(size, 3.0);
        assert!(color[2] > 0.4 && color[3] < 0.3);
        assert_eq!(cell, 3);
        // At death: clamped, not wrapped.
        let (_, size, cell) = over_life(&def, 1.0);
        assert_eq!(size, 4.0);
        assert_eq!(cell, 3);
    }

    /// **A cell range may run backwards**, and the walk follows it rather than
    /// panicking on an empty clamp. `Spells\FarSight_Impact_Base.m2`'s emitter
    /// 1 is `[[0, 15], [15, 0]]` — a flipbook forward over the first half and
    /// back over the second — and the first frame Eagle Eye's focus drew was
    /// `min > max. min = 15, max = 0` in the merge pass.
    #[test]
    fn a_reversed_cell_range_walks_backwards() {
        let mut def = plain_def();
        def.cells = [[0, 15], [15, 0]];
        let (_, _, at_birth) = over_life(&def, 0.0);
        let (_, _, at_mid) = over_life(&def, 0.5);
        let (_, _, past_mid) = over_life(&def, 0.75);
        let (_, _, at_death) = over_life(&def, 1.0);
        assert_eq!(at_birth, 0);
        assert_eq!(at_mid, 15, "the first half ends at its last cell");
        assert!(past_mid < 15 && past_mid > 0, "…and the second walks down: {past_mid}");
        assert_eq!(at_death, 0, "clamped onto the sorted pair, not wrapped");
    }

    /// **No mesh this module produces is ever zero-vertex**, whether it has
    /// been simulated or not and whether its particles have any size or not.
    ///
    /// This is not about what is drawn — a degenerate quad and nothing at all
    /// look identical. It is that Bevy's mesh allocator skips an empty mesh when
    /// allocating and then copies into it anyway, reporting a *use-after-free*
    /// for a mesh that was never allocated; see [`nothing_drawn`]. Visibility
    /// does not gate it, because asset extraction is driven by add/modify
    /// events, so this has to hold at the mesh rather than at the entity.
    #[test]
    fn an_emitter_never_hands_bevy_an_empty_mesh() {
        let starting = particle_mesh();
        assert_eq!(
            starting.count_vertices(),
            nothing::VERTICES,
            "a freshly spawned emitter's mesh is extracted before it ever simulates"
        );
        // The colour is this module's own line rather than the shared writer's,
        // so its length is asserted separately: `count_vertices` takes the
        // shortest attribute array and would not notice a short one.
        assert_eq!(
            starting.attribute(Mesh::ATTRIBUTE_COLOR).map(|c| c.len()),
            Some(nothing::VERTICES)
        );

        // A pool that is alive and entirely invisible: every scale key zero, so
        // `fill_quads` skips every particle it walks.
        let mut def = plain_def();
        def.scales = [0.0; 3];
        let mut emitter = test_emitter(def);
        emitter.pool = vec![Particle {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            age: 0.0,
            life: 1.0,
            seed: 1,
            quat: Quat::IDENTITY,
            angvel: Vec3::ZERO,
        }];
        let mut quads = QuadBuffers::default();
        fill_quads(
            &emitter,
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
            bevy::math::Affine3A::IDENTITY,
            &mut quads,
        );
        let mut mesh = particle_mesh();
        write_quads(quads, &mut mesh);
        assert_eq!(
            mesh.count_vertices(),
            4,
            "a live pool at zero size must not write an empty mesh back"
        );

        // …and the ordinary case still produces real geometry, so the guard
        // above is not quietly swallowing every emitter.
        let mut emitter = test_emitter(plain_def());
        emitter.pool = vec![Particle {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            age: 0.0,
            life: 1.0,
            seed: 1,
            quat: Quat::IDENTITY,
            angvel: Vec3::ZERO,
        }];
        let mut quads = QuadBuffers::default();
        fill_quads(
            &emitter,
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
            bevy::math::Affine3A::IDENTITY,
            &mut quads,
        );
        let mut mesh = particle_mesh();
        write_quads(quads, &mut mesh);
        assert_eq!(mesh.count_vertices(), 4, "one particle is one quad");
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        assert_ne!(positions.get_bytes(), [0u8; 48], "…and it has area");
    }

    /// An XY-quad emitter's quads lie in the **emitter's own plane**, whatever
    /// anchors it — the flag turns billboarding off, and the plane comes from
    /// the live frame rather than from the camera basis handed in. For an
    /// unrotated frame that is the world's ground plane (the model's XY), so
    /// every corner stays at height zero where a billboard against the +X/+Y
    /// camera basis below would stand the quad up. A joint-anchored XY quad
    /// used to take exactly that billboard fallback, which is how a sword's
    /// blade-plane glows ended up facing the camera instead of the blade.
    #[test]
    fn an_xy_quad_lies_in_the_emitters_plane_not_the_cameras() {
        let mut def = plain_def();
        def.flags |= particle_flags::XY_QUAD;
        let mut emitter = test_emitter(def);
        emitter.pool = vec![Particle {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            age: 0.0,
            life: 1.0,
            seed: 1,
            quat: Quat::IDENTITY,
            angvel: Vec3::ZERO,
        }];
        let mut quads = QuadBuffers::default();
        fill_quads(
            &emitter,
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
            bevy::math::Affine3A::IDENTITY,
            &mut quads,
        );
        let mut mesh = particle_mesh();
        write_quads(quads, &mut mesh);
        let corners = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|v| v.as_float3())
            .expect("positions");
        assert_eq!(corners.len(), 4, "one particle is one quad");
        let mut spans = false;
        for c in corners {
            assert!(c[1].abs() < 1e-6, "corner {c:?} left the emitter's plane");
            spans |= c[0].abs() > 1e-3 || c[2].abs() > 1e-3;
        }
        assert!(spans, "…and the quad still has area in that plane");
    }

    /// The cull volume's margin covers the worst quad the def can draw: the
    /// largest over-life size at this instance's scale, and the longest tail
    /// a particle at full speed plus a lifespan of gravity can trail. Too
    /// small is a flame missing in plain view at the frustum's edge, which is
    /// why the bound is taken from the def's extremes and not the pool's.
    #[test]
    fn the_cull_pad_covers_the_biggest_quad_and_the_longest_tail() {
        let mut def = plain_def();
        def.scales = [1.0, 4.0, 2.0];
        assert_eq!(cloud_pad(&def, 1.0), 6.0, "largest size key × 1.5, no tail");
        assert_eq!(cloud_pad(&def, 0.5), 3.0, "the instance scale scales it");
        def.tail_time = 0.5;
        def.emission_speed = Some(scalar_track(2.0));
        def.lifespan = Some(scalar_track(1.0));
        def.gravity = Some(scalar_track(4.0));
        assert_eq!(
            cloud_pad(&def, 1.0),
            9.0,
            "…plus (speed + gravity × lifespan) × tail_time"
        );
    }

    /// **A geometry-model emitter births oriented, tumbling particles and a
    /// quad emitter births neither** — the one branch that decides whether a
    /// spell is a cloud of shards or a wall of slabs.
    ///
    /// The tumble's per-axis asymmetry is the file's own and is what this
    /// pins: only X reads `min + u·range`, while Y and Z multiply a `[1, 2)`
    /// mantissa by their *range*, so Cone of Cold's authored
    /// `[0,0,0]..[0,0,−10]` spins at −10..−20 rad/s rather than at a rigid
    /// −10. Taking the X rule for all three turns the whole cloud as one
    /// picture.
    #[test]
    fn a_geometry_emitter_births_a_tumbling_body_and_a_quad_births_none() {
        let mut quad = test_emitter({
            let mut def = plain_def();
            def.emission_rate = Some(scalar_track(100.0));
            def.lifespan = Some(scalar_track(10.0));
            def
        });
        step(&mut quad, 0.1, 0, bevy::math::Affine3A::IDENTITY, Vec3::ZERO, Vec3::ZERO);
        assert!(!quad.pool.is_empty(), "the rate pours");
        for p in &quad.pool {
            assert_eq!(p.angvel, Vec3::ZERO, "a billboard does not tumble");
            assert_eq!(p.quat, Quat::IDENTITY, "…and carries no body orientation");
        }

        let mut model = test_emitter({
            let mut def = plain_def();
            def.emission_rate = Some(scalar_track(100.0));
            def.lifespan = Some(scalar_track(10.0));
            def.geometry_model = Some("Spells\\ConeofCold_Geo.m2".into());
            // Cone of Cold's own band, verbatim.
            def.tumble_min = [0.0; 3];
            def.tumble_max = [0.0, 0.0, -10.0];
            def
        });
        step(&mut model, 0.1, 0, bevy::math::Affine3A::IDENTITY, Vec3::ZERO, Vec3::ZERO);
        assert!(!model.pool.is_empty());
        let mut distinct = 0;
        for p in &model.pool {
            // WoW's local Z is Bevy's +Y (`axes::to_bevy`), so the whole band
            // lands on that axis and nowhere else.
            assert!(p.angvel.x == 0.0 && p.angvel.z == 0.0, "only Z is authored");
            assert!(
                (-20.0..=-10.0).contains(&p.angvel.y),
                "the Y/Z rule spreads the rate over [1,2)·range, got {}",
                p.angvel.y
            );
            distinct += usize::from(p.angvel.y != model.pool[0].angvel.y);
        }
        assert!(distinct > 0, "…and it is a spread, not one value for the cloud");

        // …and the tumble is integrated: the body has turned after a step.
        let before = model.pool[0].quat;
        step(&mut model, 0.1, 0, bevy::math::Affine3A::IDENTITY, Vec3::ZERO, Vec3::ZERO);
        assert!(
            model.pool[0].quat.angle_between(before) > 1e-3,
            "the Rodrigues delta never ran"
        );
    }

    /// Every kernel result leaves through the +90° turn about local Z — the
    /// client's own emitter frame (and the angle really is
    /// 90°: π times 0.5). A plane emitter's rectangle therefore has its
    /// `area_length` along the emitter's **Y** and its `area_width` along
    /// **−X** after the turn.
    #[test]
    fn the_kernel_turns_a_quarter_turn_about_z() {
        assert_eq!(rot90([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]);
        assert_eq!(rot90([0.0, 1.0, 0.0]), [-1.0, 0.0, 0.0]);
        assert_eq!(rot90([0.0, 0.0, 1.0]), [0.0, 0.0, 1.0]);
    }

    /// A zero-radius sphere still sprays outward: the direction reuses the
    /// shell vector rather than normalising a zero position — the exact
    /// reason the reference reuses its sincos pair.
    #[test]
    fn a_zero_radius_sphere_still_disperses() {
        let mut def = plain_def();
        def.emitter_type = 2;
        def.vertical_range = Some(scalar_track(1.5));
        def.horizontal_range = Some(scalar_track(3.0));
        let mut rng = 12345;
        for _ in 0..32 {
            let (pos, dir) = birth_kernel(&def, &mut rng);
            assert_eq!(pos, [0.0; 3], "zero radii put every birth at the origin");
            let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-4, "the direction is a unit shell vector");
        }
    }

    /// The gate stops **new** emission only, and a closed gate zeroes the
    /// owed fraction rather than banking it for the reopening.
    #[test]
    fn a_closed_gate_zeroes_the_accumulator_and_spares_the_living() {
        let mut emitter = test_emitter({
            let mut def = plain_def();
            def.emission_rate = Some(scalar_track(10.0));
            // A gate that is off at 0 ms and on from 500.
            def.enabled = Some(M2Track {
                interpolation: 0,
                global_sequence: -1,
                times: vec![0, 500],
                values: vec![0.0, 1.0],
                dim: 1,
            });
            def.lifespan = Some(scalar_track(10.0));
            def
        });
        let frame = bevy::math::Affine3A::IDENTITY;
        // While the gate is closed nothing is born.
        step(&mut emitter, 0.1, 0, frame, Vec3::ZERO, Vec3::ZERO);
        assert!(emitter.pool.is_empty(), "the gate is closed at 100 ms");
        assert_eq!(emitter.acc, 0.0, "and the owed fraction is zeroed");
        // Once the clock passes the key, births flow.
        emitter.age = 0.9;
        step(&mut emitter, 0.3, 0, frame, Vec3::ZERO, Vec3::ZERO);
        assert!(!emitter.pool.is_empty(), "the gate opened at 500 ms");
        let living = emitter.pool.len();
        // Closing it again (a clip that wraps back before 500) kills nothing.
        emitter.age = 0.1;
        step(&mut emitter, 0.1, 0, frame, Vec3::ZERO, Vec3::ZERO);
        assert_eq!(emitter.pool.len(), living, "live particles finish their lifespan");
    }

    /// The integrator is the verified one: gravity accelerates downward on
    /// the up axis, and drag — which runs **after** gravity, so an
    /// over-clamped drag damps the fresh gravity too — is
    /// `v -= min(dt·drag, 1)·v`, a clamp and not an exponential.
    #[test]
    fn gravity_pulls_down_and_drag_is_the_clients_clamp() {
        let launch = |def: M2Particle| {
            let mut emitter = test_emitter(def);
            emitter.pool.push(Particle {
                pos: Vec3::ZERO,
                vel: Vec3::new(3.0, 0.0, 0.0),
                age: 0.0,
                life: 10.0,
                seed: 0,
                quat: Quat::IDENTITY,
                angvel: Vec3::ZERO,
            });
            step(
                &mut emitter,
                0.1,
                0,
                bevy::math::Affine3A::IDENTITY,
                Vec3::ZERO,
                Vec3::ZERO,
            );
            (emitter.pool[0].pos, emitter.pool[0].vel)
        };

        let (pos, vel) = launch({
            let mut def = plain_def();
            def.gravity = Some(scalar_track(10.0));
            def.lifespan = Some(scalar_track(10.0));
            def
        });
        assert!(vel.y < 0.0 && pos.y < 0.0, "gravity pulls along -Y (the world's down)");

        let (_, vel) = launch({
            let mut def = plain_def();
            def.drag = 100.0; // dt·drag > 1 clamps to a full stop, not a flip
            def.lifespan = Some(scalar_track(10.0));
            def
        });
        assert_eq!(vel.x, 0.0, "over-clamped drag stops the particle, never reverses it");
    }

    fn scalar_track(v: f32) -> M2Track {
        M2Track {
            interpolation: 0,
            global_sequence: -1,
            times: vec![0],
            values: vec![v],
            dim: 1,
        }
    }

    /// **An emitter's light follows what it is emitting**, which is the whole
    /// of [`Emitter::output`] — see it for the report this came from.
    ///
    /// Three claims, in the order they matter: it is dark before anything has
    /// been emitted, it comes up while particles are alive, and **it goes back
    /// down when they stop**. The third is the one that was broken. A spell
    /// effect's light used to stand at full strength for exactly as long as its
    /// *entity* existed, whatever the emitter was doing — so a channel lit the
    /// ground flat for its whole duration, and an emitter the file marks `gated
    /// by animation` lit it while emitting nothing at all.
    #[test]
    fn an_emitters_light_follows_what_it_is_actually_emitting() {
        let running = |rate: f32| {
            let mut def = plain_def();
            def.emission_rate = Some(scalar_track(rate));
            def.lifespan = Some(scalar_track(0.5));
            def
        };
        let mut e = test_emitter(running(60.0));
        let tick = |e: &mut Emitter| {
            step(e, 1.0 / 60.0, 0, bevy::math::Affine3A::IDENTITY, Vec3::ZERO, Vec3::ZERO);
        };

        // Nothing emitted yet, so nothing lit. A light that is already on
        // before its effect has drawn a single particle is the pop this
        // replaces.
        assert_eq!(e.output(), 0.0);

        for _ in 0..180 {
            tick(&mut e);
        }
        let lit = e.output();
        assert!(!e.pool.is_empty(), "the emitter really is emitting");
        assert!(lit > 0.5, "…and it is lighting the world: {lit}");

        // **Stop it, and the light goes out with the particles.** The emitter
        // is still here and its *definition* — the colour keys and sizes the
        // old reading looked at, and all it looked at — has not changed by one
        // byte that matters to a lamp.
        e.set = Arc::new(ParticleSet {
            emitters: vec![ParticleEmitter {
                def: running(0.0),
                material: Handle::default(),
            }],
            clip: e.set.clip.clone(),
        });
        for _ in 0..180 {
            tick(&mut e);
        }
        assert!(e.pool.is_empty(), "every particle has expired");
        assert!(
            e.output() < lit * 0.25,
            "an emitter with nothing alive is not a lamp: {} against {lit}",
            e.output()
        );
    }

    fn test_emitter(def: M2Particle) -> Emitter {
        Emitter {
            set: Arc::new(ParticleSet {
                emitters: vec![ParticleEmitter {
                    def,
                    material: Handle::default(),
                }],
                clip: Some(ParticleClip {
                    start: 0,
                    end: 1000,
                    loops: true,
                    global_sequences: Arc::from([].as_slice()),
                }),
            }),
            index: 0,
            owner: Entity::PLACEHOLDER,
            anchor: Anchor::Fixed(Transform::IDENTITY),
            scale: 1.0,
            pad: 1.0,
            pool: Vec::new(),
            acc: 0.0,
            age: 0.0,
            gate_was_on: false,
            rng: 0x1234_5679,
            origin: Vec3::ZERO,
            frame: bevy::math::Affine3A::IDENTITY,
            range: None,
            instances: Vec::new(),
            peak: 0,
            output: 0.0,
        }
    }

    /// **The merged set is the additive quads and nothing else.** Blend 3 and
    /// 4 write `dst + f(src)` and addition commutes, so any pile of them in
    /// one draw is order-free; blend 2 is `mix`, which is not, and a
    /// geometry-model emitter has no quads for a field to hold — its blend
    /// belongs to the *instances*. A wrong answer here is invisible in every
    /// count: a blend-2 puff merged anyway still draws, at the wrong depth
    /// against its neighbours.
    #[test]
    fn only_additive_quad_emitters_are_merged() {
        let mut def = plain_def();
        for (blend, expect) in [(0, false), (1, false), (2, false), (3, true), (4, true), (5, false)]
        {
            def.blend = blend;
            assert_eq!(merged(&def), expect, "blend {blend}");
        }
        def.blend = 4;
        def.geometry_model = Some("Spells\\ConeofCold_Geo.m2".into());
        assert!(!merged(&def), "a geometry emitter's quads do not exist to merge");
    }

    /// **A field's quads are relative to the field's anchor, not the
    /// emitter's** — the one arithmetic the merge changes. The drawing
    /// entity's transform carries the anchor, so `vertex + anchor` must land
    /// back on the particle's world position; an implementation that kept
    /// subtracting the emitter's own origin would draw every cloud offset by
    /// the distance between it and the nearest member, which for the nearest
    /// member itself is zero — right where you look first, wrong everywhere
    /// else.
    #[test]
    fn field_quads_are_relative_to_the_fields_anchor() {
        let mut emitter = test_emitter(plain_def());
        emitter.pool = vec![Particle {
            pos: Vec3::new(100.0, 5.0, -40.0),
            vel: Vec3::ZERO,
            age: 0.0,
            life: 1.0,
            seed: 1,
            quat: Quat::IDENTITY,
            angvel: Vec3::ZERO,
        }];
        let anchor = Vec3::new(90.0, 0.0, -40.0);
        let mut quads = QuadBuffers::default();
        fill_quads(&emitter, anchor, Vec3::X, Vec3::Y, bevy::math::Affine3A::IDENTITY, &mut quads);
        assert_eq!(quads.positions.len(), 4);
        let centre = quads
            .positions
            .iter()
            .fold(Vec3::ZERO, |a, p| a + Vec3::from_array(*p))
            / 4.0;
        assert!(
            (centre + anchor - Vec3::new(100.0, 5.0, -40.0)).length() < 1e-4,
            "vertex + anchor must be the particle's world position, got {centre:?}"
        );
    }

    /// **A second emitter's quads index their own corners** — the same
    /// property the shadow field pins, and the one thing an appended buffer
    /// can get wrong that a per-emitter mesh could not: a second cloud
    /// written with the first one's indices draws the first twice and the
    /// second never, which on screen is one campfire burning double and its
    /// neighbour dark.
    #[test]
    fn a_second_emitters_quads_index_their_own_corners() {
        let particle = |x: f32| Particle {
            pos: Vec3::new(x, 0.0, 0.0),
            vel: Vec3::ZERO,
            age: 0.0,
            life: 1.0,
            seed: 1,
            quat: Quat::IDENTITY,
            angvel: Vec3::ZERO,
        };
        let mut first = test_emitter(plain_def());
        first.pool = vec![particle(0.0)];
        let mut second = test_emitter(plain_def());
        second.pool = vec![particle(50.0)];
        let mut quads = QuadBuffers::default();
        let frame = bevy::math::Affine3A::IDENTITY;
        fill_quads(&first, Vec3::ZERO, Vec3::X, Vec3::Y, frame, &mut quads);
        fill_quads(&second, Vec3::ZERO, Vec3::X, Vec3::Y, frame, &mut quads);
        assert_eq!(quads.positions.len(), 8);
        assert_eq!(quads.normals.len(), 8);
        assert_eq!(quads.uvs.len(), 8);
        assert_eq!(quads.colors.len(), 8);
        assert_eq!(quads.indices.len(), 12);
        assert!(
            quads.indices[6..].iter().all(|&v| (4..8).contains(&v)),
            "{:?}",
            &quads.indices[6..]
        );
    }
}
