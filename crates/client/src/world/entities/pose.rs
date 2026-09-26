//! What pose an entity is in, and how it got there.
//!
//! The largest single subject in this directory and the most self-contained:
//! [`animate`] and [`animate_attachment`] drive the skeletons,
//! [`wanted_animation`] and its neighbours decide *which* sequence the server's
//! answers add up to, and [`fallbacks`] is the table of what to play when a
//! model does not carry it.
//!
//! **Most of this is a game rule wearing renderer clothing.** `wanted_animation`
//! needs no mesh, no material and no window — only a `WorldEntity` — so it
//! would belong in `vale-assets` beside `dress`. What
//! keeps it here is that `WorldEntity` is a component of this crate; moving the
//! rule means moving that type first. Worth doing, not done.

use super::*;
use bevy::utils::Parallel;

/// Pose every animated entity and write its joints.
///
/// **The placement is read off the entity's own `Transform`, not its
/// `GlobalTransform`, and that is the whole difference between a helm sitting on
/// a head and a helm sliding off it.** An entity is spawned at the root of the
/// hierarchy and `place_entities` writes this frame's interpolated position onto
/// its `Transform` earlier in `Update`; its `GlobalTransform` is still last
/// frame's, because propagation runs in `PostUpdate`. A joint is a *world*
/// matrix written here, so composing it against the `GlobalTransform` poses the
/// body where the entity was one frame ago — while an attached model, which is
/// written as a local `Transform` and composed by that same `PostUpdate`
/// propagation, lands where the entity is *now*. The two halves of one character
/// then differ by exactly one frame of travel: 0.13 yards at a run, forward, and
/// varying with the frame time. It reads as the pauldrons and the helm being
/// loosely attached to the body, and it is entirely a question of which frame
/// each half was composed against.
pub(crate) fn animate(
    time: Res<Time>,
    displays: Res<DisplayCache>,
    // `Option`, because the headless harnesses (`--audit`, the entity tests)
    // build worlds with no render plugins and no tuning resource — absent
    // means "everything on", which is also the resource's own default.
    tuning: Option<Res<crate::render::tuning::WorldTuning>>,
    facing: Res<crate::world::facing::BodyFacing>,
    // The terrain and the buildings, for the one question a *drawn* pose asks
    // of them: which way the ground under this unit leans. See
    // [`crate::world::entities::conform`].
    ground: super::conform::Ground,
    // **The world camera by name, and not the first `Camera3d`.** A rig this
    // frustum cannot see is not posed, and a process holds several 3D cameras:
    // the unit-frame portraits, the paper doll, and whatever pictures a host
    // draws. Which one a query yields first is the order their archetypes were
    // made in, and that moves — the fog switch removes `DistanceFog` from the
    // world camera, which puts it in a newer archetype than every picture
    // camera. With the first camera taken, the rigs were then culled against a
    // portrait's frustum: the body stayed where it was last posed, and
    // anything re-hung on it sat at the root, until the process ended.
    camera: Query<
        (&GlobalTransform, &bevy::camera::primitives::Frustum),
        (With<crate::world::camera::WorldCamera>, Without<Joint>),
    >,
    mut entities: Query<
        (
            &WorldEntity,
            &Sheath,
            &Transform,
            &EntityModel,
            &mut Playback,
            // The smoothed ground stance — see
            // [`crate::world::entities::conform`], which is what tilts a horse
            // to the hill it is on.
            &mut super::conform::Stance,
            // **What the unit is riding, when it is riding anything.** It is
            // posed here rather than in a pass of its own because its pose is
            // an *input* to the rider's: the saddle is where the character
            // goes, and the two have to be composed in one frame or the body
            // rides a horse that is one frame behind it.
            Option<&mut Mount>,
            // Where this frame's joint matrices go — see [`Posed`].
            &mut Posed,
        ),
        (Without<Joint>, Without<AttachedTo>),
    >,
    // The joints of what a rig carries: its mount, its attachments, its
    // effects. **Not** the rig's own, which are the query under this and are
    // written in parallel — the two are disjoint by the `Bone` filter, which
    // is what lets both take the `GlobalTransform` mutably.
    mut joints: Query<&mut GlobalTransform, (With<Joint>, Without<Bone>)>,
    // The rig's own joints, each reading one row of its parent's [`Posed`].
    mut bones: Query<(&Bone, &ChildOf, &mut GlobalTransform), (With<Joint>, With<Bone>)>,
    mut attached: Query<&mut Transform, With<AttachedTo>>,
    // The animated colour of the few batches that have one — an effect's fade.
    // Written here rather than in a pass of its own because the clock and the
    // sequence window are exactly what this system already has, and because a
    // rig the camera cannot see is skipped for the same reason its pose is.
    mut tags: Query<&mut bevy::mesh::MeshTag, With<EntityPart>>,
    // One write list per thread, kept across frames for its capacity.
    mut writes: Local<Parallel<RigWrites>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Animate);
    let now = time.elapsed_secs();
    // Global-sequence tracks run on wall-clock time rather than on the playing
    // animation — blinking eyes, a turning mill wheel.
    let now_ms = (now * 1000.0) as u32;
    let camera = camera.iter().next().map(|(placement, frustum)| (*placement, frustum.clone()));

    // Both halves of the emote system go through one lookup — see
    // `vale_assets::tables::dbc::Emotes`, where the split is explained.
    let emote_animation = |id: u32| displays.tables()?.emote_animation(id);
    // …and the cast's two poses through another, three tables deep. A chain
    // with no spell tables answers with an empty pair, which puts every cast
    // back on the generic wind-up and release.
    let cast_animation = |id: u32| {
        displays
            .tables()
            .and_then(|tables| tables.cast_animation(id))
            .unwrap_or_default()
    };
    // …and the shortest hop of the three: a `SpellVisualKit` id straight to the
    // pose that kit holds, for the two packets that carry one and no spell. See
    // `vale_protocol::play::sound`.
    let kit_animation = |kit: u32| displays.tables().and_then(|tables| tables.kit_pose(kit));

    // Asked once for the whole pass rather than per entity: it is two `Arc`
    // clones and a map id, and it is `None` for every frame before there is a
    // world — the login screen's plinth stands on no ground at all.
    let ground = ground.of();
    let dt = time.delta_secs();
    let simulate = tuning.as_deref().is_none_or(|t| t.entities);
    // The pool the parallel pass below runs on. Initialised here as well as by
    // `TaskPoolPlugin`, because the headless harnesses build no such plugin
    // and `par_iter_mut` asks for the pool by name.
    bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::default);

    // **Every rig is posed in parallel, and nothing is written until all of
    // them are.** A pose is a function of the entity's own components and of
    // read-only state — the clock, the camera, the tables, the ground — so
    // the entities are independent of one another, and this pass is the
    // per-frame CPU that scales with the crowd: it was 1.5 ms of a frame
    // standing in Stratholme with nobody else there. What a pose *produces*
    // is writes onto other entities — its joints, its tints, its mount's and
    // its attachments' — and a `Query` cannot be written from inside a
    // parallel iteration, so each thread collects its writes into a
    // [`RigWrites`] of its own and the pass applies them serially afterwards.
    // The apply is a `get_mut` per joint and nothing else; the sampling,
    // the matrix composition and the attachment re-hangs are all inside the
    // parallel half.
    let shared: &Parallel<RigWrites> = &writes;
    entities.par_iter_mut().for_each(
        |(world, sheath, placement, model, mut play, mut stance, mut mount, mut posed)| {
            // Resolved before the state is chosen, because a held emote *is* the
            // state. Skipped entirely for the zero the field almost always holds.
            let held = (world.emote_state != 0)
                .then(|| emote_animation(world.emote_state))
                .flatten();
            let sheath = sheath.state();
            // **The pose an aura holds this unit in**, re-asked every frame because
            // it is a condition rather than an event — nothing on the wire announces
            // a stun *starting*, the slot is simply occupied from one update block
            // to the next. Free for the great majority of units, which carry no
            // aura at all, and a handful of hash lookups for the rest.
            //
            // The first slot that states one wins, in the server's own slot order.
            // Two stuns at once are the same pose, and nothing in the chain ranks
            // one aura above another.
            play.aura = displays.tables().and_then(|tables| {
                world.auras.iter().find_map(|aura| tables.aura_pose(aura.spell))
            });
            // …and the one stun that has no pose to hold: see [`Playback::frozen`]
            // for what this draws and which half of it is measured. A corpse is
            // excluded because the aura outlives the unit on the wire by a tick,
            // which is the same trap `state_or_held` names.
            play.freeze =
                play.aura.is_none() && world.stunned() && !world.dead && !world.feigning;
            let state = wanted_animation(world, sheath, held);
            // Both hops are closures rather than values because each is only taken
            // when its own counter actually moved, which is a handful of times a
            // minute in a city and never in the wild.
            play.note_actions(
                world,
                sheath,
                emote_animation,
                kit_animation,
                cast_animation,
                state,
                now,
            );
            let elapsed = play.advance(state, now);
            // …recorded for the sound cues before any gate below, so a rig
            // the camera cannot see still laughs.
            play.note_window(elapsed);
            // **The entities switch subtracts the pose too, not only the draw.**
            // With the layer off, `tuning::switch` hides the meshes — but the pose
            // sampling, the joint writes and the attachment re-hangs below are the
            // per-entity CPU the subtraction exists to price, and they are exactly
            // what "frame rate dips with players nearby even with entities hidden"
            // was made of. The clocks above have already advanced, so flipping the
            // layer back on resumes mid-stride — the same contract as the frustum
            // gate under this.
            if !simulate {
                return;
            }
            // **A rig the camera cannot see is not posed.** The pose is most of
            // this system's cost — sampling three tracks per bone, then a
            // `GlobalTransform` per joint, which is also what keeps every skin
            // uploading every frame — and a city keeps most of its units off
            // screen (the Trade District framing draws ~10 of 63 animated). The
            // clocks above have already advanced, so a rig entering the frustum
            // resumes mid-stride rather than restarting, and it is posed the same
            // frame the test passes, so no frame of bind pose is ever drawn. The
            // sphere is the model's own declared box ([`EntityModel::cull_radius`])
            // at the placement's scale — the volume the part meshes are culled by,
            // so a drawable part never rides a stale pose — and the far plane is
            // skipped, which errs toward posing.
            if let Some((_, frustum)) = &camera {
                // **The larger of the two rigs**, because the mount is drawn from
                // this pose too and a horse is bigger than the gnome on it. Both
                // radii are in their own model yards, so the mount's is taken at
                // the ratio [`Mount::scale`] is composed at — which is exactly the
                // world radius its own parts are culled by.
                let radius = mount.as_ref().map_or(model.cull_radius, |m| {
                    model.cull_radius.max(m.world_radius(placement.scale.x))
                });
                let sphere = bevy::camera::primitives::Sphere {
                    center: placement.translation.into(),
                    radius: radius * placement.scale.max_element(),
                };
                if !frustum.intersects_sphere(&sphere, false) {
                    return;
                }
            }
            shared.scope(|writes| {
                pose_one(
                    world,
                    placement,
                    model,
                    &mut play,
                    &mut stance,
                    mount.as_deref_mut(),
                    &facing,
                    camera.as_ref(),
                    ground.as_ref(),
                    elapsed,
                    now,
                    now_ms,
                    dt,
                    &mut posed,
                    writes,
                );
            });
        },
    );

    // **The rig's own joints read their pose off the rig, in parallel.** Each
    // carries its bone index ([`Bone`]) and is a child of the entity, so the
    // write is one lookup of the parent's [`Posed`] — which the pass above
    // left there — and a copy, over every joint at once. The joints of the
    // things a rig *carries* — its mount, its attachments, its effects — are
    // on no such index; they are the [`RigWrites`] lists, applied serially
    // below, and they are a small fraction of the joints in view.
    bones.par_iter_mut().for_each(|(bone, child_of, mut transform)| {
        if let Ok(rig) = entities.get(child_of.parent()) {
            if let Some(world) = rig.7 .0.get(bone.0 as usize) {
                *transform = *world;
            }
        }
    });
    for local in writes.iter_mut() {
        local.apply_with_roots(&mut joints, &mut tags, &mut attached);
    }
}

/// A rig's own joint: which bone of its parent's [`Posed`] it draws.
///
/// On the joints `spawn_models` makes for an entity's own skeleton and on no
/// other — a mount's, an attachment's or an effect's joints hang under a
/// different root and are written from the [`RigWrites`] lists. The index is
/// the bone's, with the model's identity joint one past the last bone, which
/// is the order [`EntityModel::joints`] keeps.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct Bone(pub u32);

/// The world matrix of every joint of one rig, as [`animate`] decided it
/// this frame: the bones in order, then the identity joint. Written in the
/// parallel half of the pass and read by the rig's [`Bone`]s in the other.
///
/// Required by [`EntityModel`], so a rig cannot be spawned without the slot
/// its joints read from. Empty until the rig is first posed.
#[derive(Component, Default)]
pub(crate) struct Posed(pub(crate) Vec<GlobalTransform>);

/// The joints, the tints and the attachment roots one rig's pose decided —
/// collected on the thread that posed it, written once every rig is posed.
///
/// A `Query` cannot be written from inside `par_iter_mut`, and the joints of
/// one rig are other entities. So the parallel half of [`animate`] fills one
/// of these per thread and [`Self::apply`] is the serial half: a `get_mut` per
/// entry and nothing else. A tint is compared at apply time, because that is
/// where the current byte can be read — a `DerefMut` on `MeshTag` is an
/// instance re-upload, and an effect that has finished fading holds one byte
/// for as long as it is worn.
///
/// The two callers outside the pass — the glue screen's character and a
/// missile — have their queries to hand and apply at once.
#[derive(Default)]
pub(crate) struct RigWrites {
    joints: Vec<(Entity, GlobalTransform)>,
    tags: Vec<(Entity, u32)>,
    /// The buffer a pose is sampled into, kept for its capacity: one per
    /// thread rather than one allocation per rig per frame, which under the
    /// system allocator's lock was a measurable part of the parallel half.
    /// Taken with `mem::take` around a pose and put back after, because the
    /// pose is read while the joints are pushed.
    scratch: Vec<[f32; 12]>,
    /// An attachment's root, as a local `Transform` in its wearer's space —
    /// see [`animate`] for why it is local while a joint is a world matrix.
    roots: Vec<(Entity, Transform)>,
}

impl RigWrites {
    pub(crate) fn joint(&mut self, joint: Entity, world: Affine3A) {
        self.joints.push((joint, GlobalTransform::from(world)));
    }

    pub(crate) fn tag(&mut self, part: Entity, wanted: u32) {
        self.tags.push((part, wanted));
    }

    fn root(&mut self, root: Entity, local: Transform) {
        self.roots.push((root, local));
    }

    /// Write everything collected and empty the lists, keeping their
    /// capacity for the next frame.
    pub(crate) fn apply<F: bevy::ecs::query::QueryFilter>(
        &mut self,
        joints: &mut Query<&mut GlobalTransform, F>,
        tags: &mut Query<&mut bevy::mesh::MeshTag, With<EntityPart>>,
    ) {
        for (joint, world) in self.joints.drain(..) {
            if let Ok(mut transform) = joints.get_mut(joint) {
                *transform = world;
            }
        }
        for (part, wanted) in self.tags.drain(..) {
            if let Ok(mut tag) = tags.get_mut(part) {
                if tag.0 != wanted {
                    tag.0 = wanted;
                }
            }
        }
        debug_assert!(self.roots.is_empty(), "attachment roots need `apply_with_roots`");
        self.roots.clear();
    }

    /// [`Self::apply`], plus the attachment roots — the pass's own form.
    fn apply_with_roots<F: bevy::ecs::query::QueryFilter>(
        &mut self,
        joints: &mut Query<&mut GlobalTransform, F>,
        tags: &mut Query<&mut bevy::mesh::MeshTag, With<EntityPart>>,
        roots: &mut Query<&mut Transform, With<AttachedTo>>,
    ) {
        for (root, local) in self.roots.drain(..) {
            if let Ok(mut transform) = roots.get_mut(root) {
                *transform = local;
            }
        }
        self.apply(joints, tags);
    }
}

/// Pose one rig that the frustum test has passed: the entity's own skeleton,
/// the animal under it and everything hanging off it.
///
/// The body of [`animate`]'s loop, as a function so that the parallel
/// iteration's closure is the gate and this is the work. Everything it reads
/// is either the entity's own or shared read-only; everything it writes goes
/// into `writes`.
#[allow(clippy::too_many_arguments)]
fn pose_one(
    world: &WorldEntity,
    placement: &Transform,
    model: &EntityModel,
    play: &mut Playback,
    stance: &mut super::conform::Stance,
    mount: Option<&mut Mount>,
    facing: &crate::world::facing::BodyFacing,
    camera: Option<&(GlobalTransform, bevy::camera::primitives::Frustum)>,
    ground: Option<&(crate::world::session::Standing, u32)>,
    elapsed: u32,
    now: f32,
    now_ms: u32,
    dt: f32,
    posed: &mut Posed,
    writes: &mut RigWrites,
) {
    // Billboarded bones face the camera, and "the camera" has to be
    // expressed in the *model's* own space — which for an entity is the
    // world turned by minus its facing, that being the only rotation a
    // placement has.
    let bone_camera = camera.map(|(camera, _)| {
        let facing = placement.rotation.to_euler(EulerRot::YXZ).0;
        (
            into_model_space(axes::to_wow(camera.right().into()), facing),
            into_model_space(axes::to_wow(camera.up().into()), facing),
        )
    });
    let mut pose = std::mem::take(&mut writes.scratch);
    let layers = PoseLayers {
            blend: play.fade.as_ref().map(|fade| Blend {
                // Wrapped for a loop — the outgoing one keeps running while it
                // fades, and a creature that has been standing for a minute
                // must fade out of the phase its idle is *at* — and held at
                // its last frame for a one-shot that ran out. See
                // [`Playback::fade_phase`] and [`Fade::held`].
                sequence: fade.sequence,
                elapsed_ms: play.fade_phase(fade, now),
                weight: 1.0 - (now - fade.from) / FADE_SECS,
            }),
            // **The other half of a strafe.** The placement above has
            // already turned the whole model into the slide; this turns the
            // spine and the head back out of it, so the character keeps
            // looking where it is aiming while its hips and legs run along
            // the line of travel. Zero — and therefore free — for
            // everything that faces where it is going. See
            // `crate::world::facing`.
            twist: facing.of(world.guid).map(|body| BodyTwist { gap: body.gap }),
            // **The upper body over the lower**, and it is the whole of how
            // the game swings, emotes and casts on the move: the torso
            // plays its own sequence on the `SpineLow` subtree while the
            // legs keep the gait. `None` for a standing character, whose
            // one-shot took the base track instead — see [`route_oneshot`].
            overlay: play.overlay_at(now),
        };
    play.skeleton.pose_into(
        play.sequence,
        elapsed,
        now_ms,
        bone_camera,
        layers,
        &mut pose,
        &mut play.hints,
    );

    // **The mount, and the one affine it changes.** Posed first, because
    // where it puts the rider is a result of its own pose; the answer is a
    // translation in the rider's local frame, which is composed into
    // `world_from_entity` below and therefore into the rider's joints, its
    // attached models, its glows and its ground decals at once. Identity
    // for everything that is not riding, which is nearly everything.
    // **…and the hill it is standing on**, which is the *animal's* decision
    // when there is one: `HumanMale.m2` never leans and `Horse.m2` always
    // does, so a mounted character tilts because its saddle does. Identity
    // for everything that does not lean, which is every character model in
    // the game and all of the scenery. See `super::conform`.
    let leaning = super::conform::leaning(model.conform, mount.as_deref().map(|m| m.conform));
    let conform = match ground {
        Some((standing, map_id)) => super::conform::settle(
            stance,
            leaning,
            standing,
            *map_id,
            world,
            placement,
            dt,
        ),
        None => Affine3A::IDENTITY,
    };

    let seat = match mount {
        Some(mount) => mount::ride(
            mount,
            world,
            placement,
            conform,
            bone_camera,
            now,
            now_ms,
            writes,
        ),
        // With no animal under it the unit's own conform *is* the seat: one
        // affine either way, which is what keeps the composition below —
        // and the attachment loop, which already carries `seat` — from
        // having to know which case it is in.
        None => conform,
    };

    // The joint matrices *replace* the mesh's world matrix in Bevy's
    // skinning shader, so each one is the entity's placement times the
    // bone's own pose — not the pose alone. They go onto the rig's own
    // [`Posed`], in bone order, for its [`Bone`]s to read; the identity
    // joint, which the weightless vertices ride on, comes last — the model's
    // own space, unposed, and therefore `world_from_entity` rather than the
    // placement, so that a rider's unweighted geometry is lifted onto the
    // saddle with the rest of it.
    let world_from_entity = placement.compute_affine() * seat;
    posed.0.clear();
    posed.0.extend(
        pose.iter()
            .map(|bone| GlobalTransform::from(world_from_entity * axes::pose_to_bevy_affine(bone))),
    );
    posed.0.push(GlobalTransform::from(world_from_entity));

    // The entity's own fade, on the animation it is playing. Written only
    // when the byte moved: a `DerefMut` on `MeshTag` re-uploads that
    // mesh's instance data, and most tinted batches hold one value for
    // minutes at a time.
    if let Some(tints) = &model.tints {
        let window = play.skeleton.sequences.get(play.sequence);
        for one in &model.tinted {
            writes.tag(
                one.part,
                crate::render::models::tint_tag(tints.sample_in(one.tint, window, elapsed, now_ms)),
            );
        }
    }

    // The attached models ride the same pose. An attachment point is a
    // position in its **bone's** frame, so the placement is
    // `bone * T(offset)` — and the root is written as a `Transform` in the
    // wearer's local space rather than as a world matrix, because its
    // parts are drawn and Bevy composes a drawn thing's parent for itself.
    for one in model
        .attached
        .iter()
        .chain(&model.cast.parts)
        .chain(&model.impact.parts)
        .chain(&model.state.parts)
        .chain(&model.milestone.parts)
        .chain(&model.loot.parts)
        // **The two sets a packet or a host fills were not in this chain**, so
        // a model `SMSG_PLAY_SPELL_VISUAL` hung stayed at its root's initial
        // transform — the wearer's origin at scale — instead of riding its
        // bone. Every set with parts is posed here.
        .chain(&model.pushed.parts)
        .chain(&model.hung.parts)
    {
        let Some(local) = one.local(&pose) else {
            continue;
        };
        // **The seat has to be in here too**, and it is the half that is
        // easy to miss: an attachment's root is a *child* of the entity, so
        // Bevy composes it against the entity's own `Transform` — which is
        // still on the ground under the horse. The joints above are world
        // matrices and carry the seat by construction; these are local and
        // do not, so a mounted character would sit on the saddle with its
        // pauldrons and its helm left standing in the mud.
        let local = Mat4::from(seat) * local;
        // **A thing on the floor does not turn with the thing standing on
        // it.** The root is a child of the wearer, so Bevy composes the
        // wearer's whole transform onto it — including the facing, which
        // dragged a rooted player's roots round with them as they spun on
        // the spot and made the effect read as painted on the character
        // rather than on the ground.
        //
        // Taking the rotation back out on the left cancels it: the wearer
        // is `T·R·S`, so `T·R·S · R⁻¹·local` is `T·S·local` **while the
        // scale is uniform**, which every unit's is (`Vec3::splat`). Stated
        // because a non-uniform one would shear instead, and the failure
        // would be subtle.
        let local = if one.grounded {
            Mat4::from_quat(placement.rotation.inverse()) * local
        } else {
            local
        };
        writes.root(one.root(), Transform::from_matrix(local));
        animate_attachment(
            one,
            placement.compute_affine() * Affine3A::from_mat4(local),
            camera.map(|(placement, _)| placement),
            now,
            now_ms,
            writes,
        );
    }
    writes.scratch = pose;
}

/// Pose one attached model on its **own** clock, and fade it on its own tracks.
///
/// `world_from_attach` is the frame the attachment sits in, already composed —
/// the wearer's placement times the carrying bone times the offset for a
/// pauldron or a spell glow, and the projectile's own placement for a missile,
/// which hangs off nothing. That is the whole of what this needs from the
/// caller, which is why it is a free function: a missile is an attached model
/// with no wearer, and duplicating this for it would have been two copies of
/// the billboard derivation.
///
/// **The camera basis is re-derived by inverting that frame** rather than by
/// undoing the wearer's facing. The wearer's shortcut does not survive the
/// second hop — the carrying bone's rotation is between the two — and a torch
/// glow is a quad on a bone flagged 0x8 in the *torch's* own skeleton.
pub(crate) fn animate_attachment(
    one: &AttachedPart,
    world_from_attach: Affine3A,
    camera: Option<&GlobalTransform>,
    now: f32,
    now_ms: u32,
    writes: &mut RigWrites,
) {
    let raw = ((now - one.since) * 1000.0) as u32;
    // Loop or hold: `pose` clamps, so a non-looping effect holds its last
    // frame — which for a one-shot like Arcane Explosion's dome is the
    // collapsed one, not the fully-grown one.
    let elapsed = match (one.loops, &one.skeleton) {
        (true, Some(skeleton)) => skeleton.phase(one.sequence, raw),
        _ => raw,
    };

    // **The fade, and it is the half that ends the effect.** The geometry
    // animates and the emitters run; what says it is *over* is `M2Color` and
    // the transparency block — Arcane Explosion's dome runs 0 -> 0.91 -> 0 over
    // its own 767 ms. Before the skeleton test below, because an effect can be
    // a rigid quad that does nothing but fade.
    if let Some(tints) = &one.tints {
        let window = one
            .skeleton
            .as_ref()
            .and_then(|s| s.sequences.get(one.sequence));
        for tinted in &one.tinted {
            // Only on a change, which [`RigWrites::apply`] decides — the same
            // instance re-upload the wearer's own tint loop avoids, and an
            // effect that has finished fading holds its last byte for as long
            // as it is worn.
            writes.tag(
                tinted.part,
                crate::render::models::tint_tag(
                    tints.sample_in(tinted.tint, window, elapsed, now_ms),
                ),
            );
        }
    }

    // The attachment's **own** pose. Skipped for the rigid majority (no
    // skeleton) at the cost of one branch.
    let (Some(skeleton), false) = (&one.skeleton, one.joints.is_empty()) else {
        return;
    };
    let attach_camera = camera.map(|camera| {
        let inv = world_from_attach.inverse();
        (
            axes::to_wow(inv.transform_vector3(camera.right().into()).normalize()),
            axes::to_wow(inv.transform_vector3(camera.up().into()).normalize()),
        )
    });
    // The wearer's pose is out of the scratch while this runs (see
    // `pose_one`), so an attachment's is a buffer of its own.
    let attach_pose =
        skeleton.pose(one.sequence, elapsed, now_ms, attach_camera, PoseLayers::default());
    for (bone, &joint) in attach_pose.iter().zip(one.joints.iter()) {
        writes.joint(joint, world_from_attach * axes::pose_to_bevy_affine(bone));
    }
    // The identity joint, as on the wearer: the attachment's own space,
    // unposed.
    if let Some(&last) = one.joints.last() {
        writes.joint(last, world_from_attach);
    }
}

/// Which animation an entity should be playing, from the state the snapshot
/// carries.
///
/// **Every input here is something the server states, and none of it is an
/// animation id** — the protocol has no such thing. Death is health reaching
/// zero, a swing is one `SMSG_ATTACKERSTATEUPDATE`, a combat stance is
/// `UNIT_FLAG_IN_COMBAT`, and sitting is a byte of `UNIT_FIELD_BYTES_1`. Turning
/// those into a pose is entirely the client's job, which is why nothing about
/// getting it wrong produces an error.
///
/// The order is a priority, and it is the order the game's own rules imply: a
/// corpse does not flinch, and a creature running at you does not stand in its
/// ready pose.
///
/// `sheath` is the unit's **committed** sheath state ([`super::Sheath`]) and it
/// decides one thing here: which ready stance a unit in combat holds. It is an
/// argument rather than a field of `WorldEntity` because it is the client's own
/// answer rather than the server's — see [`super::sheath`].
pub(super) fn wanted_animation(world: &WorldEntity, sheath: u8, emote_state: Option<u16>) -> u16 {
    // **A game object answers a different question from a unit, and it is the
    // only question it answers.** It does not die, fight, sit or walk; what it
    // does is stand open or stand shut, and `GAMEOBJECT_STATE` is the one field
    // that says which. Its model has no `Stand` either — `Chest02.m2` carries
    // 146..149 and nothing else — so falling through to the unit rules below
    // resolved to no sequence at all and left every chest and door in the world
    // drawn in its bind pose. First, because none of the rules below apply.
    //
    // **The test is the object's *kind*, not whether the field arrived**, and
    // that distinction is the whole of the "some doors show both states at
    // once" report. `Object::_SetCreateBits` omits a field whose value is zero,
    // so a game object standing **open** — `GO_STATE_ACTIVE` is 0 — states
    // nothing at all, and gating on `Some` dropped exactly those through to the
    // unit rules, where a door matches nothing and is drawn in its bind pose.
    // `DEADMINEDOOR01.m2` is 28 bones whose bind pose is neither state and
    // leaves the model's own box by 3.06 yards, so an unposed door is both
    // leaves splayed — open and shut at the same time, which is what was seen.
    // See `vale_assets::world::collision::game_object_is_solid`, which is the same
    // rule for the same field and reads the absence the same way.
    if world.kind == ObjectType::GameObject {
        return if world.object_state.unwrap_or(0) == GO_STATE_READY {
            anim::CLOSED
        } else {
            anim::OPENED
        };
    }
    // A corpse holds the last frame of Death — see `anim::DEAD`, which is in
    // the DBC and in none of the 411 models, and `Playback::advance`, which
    // stops the clock rather than looping.
    //
    // **…and a feigning body is drawn as one**, which is the reference's own
    // conjunction rather than an approximation of it: the client answers "is
    // this dead" with health-is-zero **or** `UNIT_DYNFLAG_DEAD` **or** the
    // object is a corpse, and uses that answer only for display. So Feign Death needs no clip, no aura pose and no state of its own
    // — it is the death clip, held on its last frame by the same rule, and the
    // moment the flag clears the body fades back up into whatever it was doing.
    //
    // Nothing else in this client reads [`WorldEntity::feigning`]: the health
    // never moved, so the target frame, the release box and the corpse marker
    // all go on saying the unit is alive, which is what they say in 1.12 too.
    if world.dead || world.feigning {
        return anim::DEATH;
    }
    // **A rider does one thing, and the mount does the rest.** `Mount` (91) is
    // a held pose, not a gait: the character sits still while the horse walks,
    // runs, swims and jumps underneath it — which is why this outranks the
    // water and the air as well as the gait, where for an unmounted unit each
    // of those is the answer. `AnimationData.dbc` says the same thing about it
    // from the other side: its policy column is `STOW_HANDS_BUSY`, so the
    // weapons go away for the whole ride.
    //
    // Above the emote and the stand state as well, both of which the server can
    // report on a unit that is still mounted; below death, which dismounts.
    if world.mounted {
        return anim::MOUNT;
    }
    // **Swimming outranks the gait, and it is a different *set* of animations
    // rather than a variant of one.** A character in deep water plays Swim
    // while moving and SwimIdle while not — the second matters as much as the
    // first, since a swimmer who stops does not stand to attention in mid-water.
    //
    // **And the water's precedence is its own**, which is the reason this reads
    // flags rather than a direction: the client's swim cascade is
    // **turn > strafe > backward > forward**, so a turning swimmer treads water
    // whatever else is held, and a strafe *diagonal* side-strokes where the
    // same keys on the ground run forwards.
    if world.swimming {
        if !(world.moving && world.speed >= MOVING_FLOOR) {
            return anim::SWIM_IDLE;
        }
        let f = world.move_flags;
        return if f & (move_flags::TURN_LEFT | move_flags::TURN_RIGHT) != 0 {
            anim::SWIM_IDLE
        } else if f & move_flags::STRAFE_LEFT != 0 {
            anim::SWIM_LEFT
        } else if f & move_flags::STRAFE_RIGHT != 0 {
            anim::SWIM_RIGHT
        } else if f & move_flags::BACKWARD != 0 {
            anim::SWIM_BACKWARDS
        } else {
            anim::SWIM
        };
    }
    // **…and flying outranks both**, on the same terms and out of the same
    // cascade: the client tests the *spline's* `Flying` flag and picks 135
    // before it reaches any of the ground tests below. It is a spline flag
    // rather than a unit state, which is why it arrives here as a movement flag
    // the ride sets — see [`vale_protocol::state::movement::Mover::ride`] and
    // `Entity::move_flags`, which are the two ends of it.
    //
    // **Not gated on `moving`**, unlike the ground gait: a taxi flight is
    // driven by a spline and there is no such thing as hovering still on one.
    if world.move_flags & move_flags::FLYING != 0 {
        return anim::FLY;
    }
    // **In the air outranks the gait**, and for the same reason swimming does:
    // it is not a variant of running, it is a different pose entirely, and a
    // character crossing a gap in its run cycle reads as one running on air.
    // Which of the two arcs this is comes from whether anything pushed off —
    // `MSG_MOVE_JUMP` carries a non-zero `zspeed` (negative: the wire is
    // down-positive) where a step off a ledge carries zero.
    if world.airborne {
        return if world.jumping { anim::JUMP } else { anim::FALL };
    }
    if world.moving && world.speed >= MOVING_FLOOR {
        // **Reversing is one sequence at any speed.** The wire carries a
        // separate `run_back` speed and the art carries only `Walkbackwards`,
        // so there is nothing to pick between — see [`anim::WALK_BACKWARDS`].
        // On the ground backward beats a strafe, which is the opposite of the
        // water's order above.
        if world.move_flags & move_flags::BACKWARD != 0 {
            return anim::WALK_BACKWARDS;
        }
        // **A stealthed body creeps, at any speed, and it outranks the
        // Sprint/Run/Walk split entirely.** The client reads
        // `UNIT_FIELD_BYTES_1`'s vis byte for `UNIT_VIS_FLAGS_CREEP` here —
        // between the reverse gait above and the speed tests below — and picks
        // 119 without ever asking how fast the unit is going. There is no
        // `StealthRun` in `AnimationData.dbc` to ask for.
        //
        // **Below the reverse gait and not above it**, which is the reference's
        // own order rather than a choice: a stealthed character backing away
        // plays `Walkbackwards`, because that is the arm that already returned.
        if world.creeping() {
            return anim::STEALTH_WALK;
        }
        // **The all-out run, and it is a speed and not a spell.** Charge,
        // Sprint and any other haste past 11.0 y/s reach it the same way — see
        // [`anim::SPRINT`], which has the cascade and the constant.
        // Above the Run/Walk split for the same reason `creeping` is: it is a
        // separate arm of the reference's own cascade, not a third case of the
        // speed comparison below.
        if world.speed >= anim::SPRINT_SPEED {
            return anim::SPRINT;
        }
        // **And a strafe is the forward gait.** There is no sideways gait on
        // the ground: `RunLeft`/`RunRight` are rows in `AnimationData.dbc` that
        // 0 of 411 models carry, and the Shuffles below are the turn-in-place
        // foot-shuffle, not this. What makes a strafe *look* like one is that
        // the drawn body is turned into the slide while the legs run — see
        // `movement::strafe_body_offset` and `crate::world::facing`, which is
        // where the whole of the strafe lives.
        return if world.speed >= RUN_SPEED { anim::RUN } else { anim::WALK };
    }
    // **…and a stealthed body that has stopped holds the crouch.** The idle
    // cascade is its own function in the reference and has the
    // creep test in the same place this one does: after the water, before
    // everything else it would otherwise stand for.
    //
    // Above the turn-in-place shuffles, which is where the reference puts it —
    // its cascade is `swimming -> creep -> hover -> Stand` and reaches no
    // shuffle at all, so the only question here is which side of them it falls
    // on, and a stealthed character turning on the spot is still stealthed.
    if world.creeping() {
        return anim::STEALTH_STAND;
    }
    // **Turning on the spot: the foot-shuffle.** Reached only when not moving —
    // a turn while travelling curves the run path and keeps the gait — and
    // driven by the body's *actual* rotation rather than by the turn keys, so
    // that a mouse turn shuffles too. `crate::world::facing` is what states it.
    if world.move_flags & move_flags::TURN_LEFT != 0 {
        return anim::SHUFFLE_LEFT;
    }
    if world.move_flags & move_flags::TURN_RIGHT != 0 {
        return anim::SHUFFLE_RIGHT;
    }
    // **Crouched over a body.** The one entry in this whole cascade that no
    // packet states: there is no loot row in `Emotes.dbc` and no unit field for
    // it, so this is played off a window this client knows is open and nothing
    // else. See [`WorldEntity::looting`], which is only ever true for the local
    // player, and `anim::LOOT`, which is where the clip is measured.
    //
    // **Where it sits is the whole of its precedence and each side of it is
    // deliberate.** Below locomotion, the water and the air, because all three
    // of those are moot: any movement packet carrying `MOVEFLAG_MASK_MOVING`
    // makes vmangos release the body server-side, so a looting character is a
    // standing still one — and if the window somehow outlives a step, a
    // character reaching into the ground while running is worse than one that
    // simply stands up. Above the stand state and the held emote, because both
    // of those are poses the server asked for *earlier* and this is what the
    // player is doing *now*: an innkeeper's stool does not outrank the corpse
    // in front of him.
    if world.looting {
        return anim::LOOT;
    }
    // Stationary. The stand state is what the server uses to seat an innkeeper
    // on a stool or lay a guard down at his post, and drawn standing they read
    // as models floating through the furniture.
    match world.stand_state {
        STAND_STATE_SIT => return anim::SIT_GROUND,
        STAND_STATE_SIT_CHAIR => return anim::SIT_CHAIR_MED,
        STAND_STATE_SLEEP => return anim::SLEEP,
        STAND_STATE_SIT_LOW_CHAIR => return anim::SIT_CHAIR_LOW,
        STAND_STATE_SIT_MEDIUM_CHAIR => return anim::SIT_CHAIR_MED,
        STAND_STATE_SIT_HIGH_CHAIR => return anim::SIT_CHAIR_HIGH,
        STAND_STATE_KNEEL => return anim::KNEEL,
        _ => {}
    }
    // **A held emote, which is the half of the emote system that is not a
    // packet.** `UNIT_NPC_EMOTESTATE` is an `Emotes.dbc` id the unit keeps: a
    // dancing player, an innkeeper permanently at work, a guard leaning on a
    // rail. It outranks the combat stance because the server is asking for a
    // specific pose and would clear the field if it wanted the default one, and
    // it loses to locomotion for the same reason the stand state does.
    if let Some(emote) = emote_state {
        return emote;
    }
    // **Engaged, not merely in combat.** The client gates the
    // weapon-class Ready idle on the unit's *auto-attack target guid* being set
    // — `SMSG_ATTACKSTART` until `SMSG_ATTACKSTOP` — rather than on
    // `UNIT_FLAG_IN_COMBAT` and rather than on the sheath state. Those are
    // three different questions and only this one
    // means "is swinging at something": a mage being beaten on carries the
    // combat flag for the whole fight and never raises a weapon, and a player
    // who has picked up aggro across a room is in combat before there is
    // anything within reach to guard against.
    if world.attacking {
        // **The ready stance is the one place the drawn weapon shows between
        // blows**, and a two-hander held in the unarmed guard reads as a
        // character carrying a plank. `WeaponAnim` decides which family, off
        // the item's own class.
        return drawn_weapon(world, sheath).ready();
    }
    anim::STAND
}

/// Where a one-shot plays: over the whole body, or over the legs.
///
/// See [`route_oneshot`], which is where the choice is made and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Route {
    /// On the base track, bone 0 — the clip replaces the whole pose, legs
    /// included. What a standing character does.
    FullBody,
    /// On the masked overlay rooted at `SpineLow`: the torso plays the clip and
    /// the legs keep the gait underneath it.
    Masked,
}

/// Whether a one-shot plays over the legs or replaces them — **decided per
/// play, by live state, and never by the animation id alone.**
///
/// This is the whole of how 1.12 casts a spell at a run, waves while walking or
/// swings mid-jump, and it is the one rule in this area that could not be
/// guessed at: the same `Attack1H` is full-body from a standing character and
/// masked from a running one, so nothing about the id says which. It follows
/// the 1.12.1 client's own rule.
///
/// The lower body is **committed** — and the play therefore masks — when any
/// direction bit is set (which folds in the two keyboard turn keys, unlike the
/// cast-cancel mask beside it in [`Playback::state_or_held`]), or the unit is
/// swimming, or it is in a stand state other than 0, or a *combat* id is thrown
/// while airborne. Otherwise it is standing idle and the clip takes the body.
///
/// The id gates only **which** tests apply: [`class_a`] admits an id to the
/// block at all, [`combat_id`] admits it to the airborne test, and
/// [`forced_full_body`] takes two families back out whatever the state.
pub(super) fn route_oneshot(id: u16, flags: u32, stand_state: u8, airborne: bool) -> Route {
    if forced_full_body(id) || !class_a(id) {
        return Route::FullBody;
    }
    let committed = flags & COMMITTED_LOWER != 0
        || stand_state != 0
        || (combat_id(id) && airborne);
    if committed {
        Route::Masked
    } else {
        Route::FullBody
    }
}

/// The client's movement-flags `& 0x20003f`: every direction bit — the two
/// turn keys included — plus swimming.
///
/// **Not the same mask as the one that cancels a cast**, which is the direction
/// bits and swim and *nothing else*; the client tests two different masks in
/// two places and reading one as the other cancels a cast every time the
/// mouse moves. See [`Playback::state_or_held`] for the other one.
const COMMITTED_LOWER: u32 = move_flags::FORWARD
    | move_flags::BACKWARD
    | move_flags::STRAFE_LEFT
    | move_flags::STRAFE_RIGHT
    | move_flags::TURN_LEFT
    | move_flags::TURN_RIGHT
    | move_flags::SWIMMING;

/// The **maskable-eligible** id set: an id outside it is always
/// full-body.
///
/// The load-bearing memberships are exact; the patchy
/// interior of the wide ranges is inferred and is not load-bearing here,
/// because everything this client routes through it is a swing, a flinch, a
/// cast release or an emote and all of those sit squarely inside a range. What
/// the set *excludes* is what matters at the edges, and it excludes exactly the
/// two families that must never mask: the jump band 37..45, and a game object's
/// 146..149.
fn class_a(id: u16) -> bool {
    matches!(id,
        2 | 8..=10 | 14..=36 | 46..=49 | 51..=90 | 105..=113 | 117..=118 | 122..=138
            | 185..=186 | 195)
}

/// **Which landing a character who has just arrived owes** — the client's own
/// rule, read off the **movement flags** rather than off the
/// resolved gait.
///
/// [`anim::JUMP_LAND_RUN`] carries 6.9 y/s of travel on `HumanMale` where
/// [`anim::JUMP_END`] carries none, so the choice is the whole difference
/// between a body that keeps going and one that stops dead under a character
/// still sliding forward. The client decides it in four tests, in this order:
///
/// ```text
/// no direction held (flags & 0xf == 0)  -> JumpEnd (39)
/// MOVEFLAG_BACKWARD                     -> no clip
/// MOVEFLAG_WALK_MODE                    -> no clip
/// otherwise                             -> JumpLandRun (187)
/// ```
///
/// **"No clip" is a real answer and not a gap.** Walking, or backpedalling, the
/// reference calls its movement-animation update instead of
/// playing anything — the gait simply resumes — and `None` here is what makes
/// [`Playback::note_flight`] do the same. A `Some` that the model cannot play
/// takes the same road, because the client resolves it through
/// `AnimationData.dbc`'s fallback column and that column reads **5 (`Run`)**
/// for `JumpLandRun` and **0 (`Stand`)** for `JumpEnd`: on a model with no
/// landing clip the landing *is* the gait.
///
/// That last case is every mount in the game. `Creature\Horse\Horse.m2`,
/// `Ram`, `Wolf`, `Tiger`, `MechaStrider` and `UndeadHorse` each carry
/// `JumpStart`, `Jump`, `JumpEnd` and `Fall` and **none of them carries 187**
/// (`vale anim <model>` lists what each has). An earlier version of this
/// rule chose the clip off the gait and fired it whether or not the model had
/// it, and a fire that resolves to nothing changes nothing — so a mount that
/// landed running went on playing its take-off clip on the ground for whatever
/// was left of the 833 ms, which is "the mount floats briefly on a quick
/// landing" exactly.
///
/// The three strafes travel and take the running landing; a shuffle is turning
/// on the spot and has no direction bit, so it takes the standing one.
pub(super) fn landing_for(move_flags: u32) -> Option<u16> {
    const DIRECTIONS: u32 = move_flags::FORWARD
        | move_flags::BACKWARD
        | move_flags::STRAFE_LEFT
        | move_flags::STRAFE_RIGHT;
    if move_flags & DIRECTIONS == 0 {
        return Some(anim::JUMP_END);
    }
    if move_flags & (move_flags::BACKWARD | move_flags::WALK_MODE) != 0 {
        return None;
    }
    Some(anim::JUMP_LAND_RUN)
}

/// The **combat** id set — the only ids whose airborne test can
/// mask, which is what makes a swing thrown mid-jump play over the arc while an
/// emote in mid-air does not.
fn combat_id(id: u16) -> bool {
    matches!(id, 10 | 16..=24 | 30 | 36 | 57..=59 | 85..=88 | 95 | 117 | 118)
}

/// The two families forced onto the base track whatever the state: the death
/// class and the sit transitions. A corpse does not
/// fall over from the waist up.
fn forced_full_body(id: u16) -> bool {
    matches!(id, 1 | 6 | 131 | 132 | 57 | 58 | 118)
}

/// Which family of attack and parry animations this unit's **drawn** weapon
/// belongs to.
///
/// Unarmed when nothing is out: a sheathed sword is on the wearer's back and
/// the swing that goes with it is the fist. Which slot counts depends on the
/// sheath state for the same reason the *models* do — see
/// `vale_assets::look::dress`, where the same three cases decide where each weapon
/// hangs.
///
/// **`sheath` is the client's committed state, not `WorldEntity::sheath_state`**
/// — see [`super::Sheath`]. Reading the wire's byte here is what made the local
/// player punch everything in the game with a sword on his back: the field is an
/// echo of a packet this client had never sent.
pub(super) fn drawn_weapon(world: &WorldEntity, sheath: u8) -> WeaponAnim {
    match sheath {
        SHEATH_STATE_MELEE => WeaponAnim::of(&world.weapons[0]),
        SHEATH_STATE_RANGED => WeaponAnim::of(&world.weapons[2]),
        _ => WeaponAnim::Unarmed,
    }
}

/// **The shot a ranged weapon attack plays**, off the third slot and nothing
/// else.
///
/// Deliberately *not* through [`drawn_weapon`]: that asks "what is in the
/// hands", and a ranged spell is fired from the ranged slot whether or not the
/// sheath state has caught up with it yet — a shot pressed on the frame the bow
/// is still being drawn must not throw a sword swing. The slot is the question
/// and the sheath is a consequence of it (see `game::action`, which asks for
/// `SHEATH_RANGED` on the same press).
///
/// `None` for an empty slot and for a **wand**, which is the honest answer
/// rather than a gap: `WeaponAnim::of` puts a wand in `Unarmed` because the
/// character models carry no wand sequence at all, so the alternative is a
/// caster punching the air once a second for the length of a fight. What the
/// reference plays there is not established.
fn ranged_shot(world: &WorldEntity) -> Option<u16> {
    WeaponAnim::of(&world.weapons[2]).ranged_attack()
}

/// The **off** hand, on the same terms — which is a separate question, and the
/// separation is the fix.
///
/// This used to be answered by not asking: any left-handed blow played
/// `AttackOff` (87). The client keys the off-hand swing on the off-hand
/// *item*, and the partition is not the main hand's — a
/// dagger stabs (`AttackOffPierce`, 88), any other weapon swings 87, and an
/// empty hand, a shield or an off-hand tome punches (`AttackUnarmedOff`, 117).
/// A rogue with a dagger in each hand played one animation twice.
fn off_hand(world: &WorldEntity, sheath: u8) -> WeaponAnim {
    match sheath {
        SHEATH_STATE_MELEE => WeaponAnim::of(&world.weapons[1]),
        _ => WeaponAnim::Unarmed,
    }
}

/// Which swing this blow was, off the two `HitInfo` bits the server sets on it.
///
/// **The weapon decides the family and the packet decides the variant**, and
/// they are separate questions: [`drawn_weapon`] answers "what is in the hands"
/// from the wardrobe, and this answers "which hand, and how hard" from the
/// swing's own `SMSG_ATTACKERSTATEUPDATE` — the two bits this client parsed and
/// then read straight over. Every critical in the game looked like every other
/// blow.
///
/// **`HITINFO_CRITICALHIT` is not read here, and that is a retraction.** This
/// function used to answer `CombatCritical` (10) for a critical swing, on the
/// strength of the id's name. The client puts that id somewhere else entirely:
/// it belongs to the **victim's** wound chooser, which takes a `critical`
/// argument and answers 10 `CombatCritical` for a critical and 9 `CombatWound`
/// or 8 `StandWound` otherwise — so 10 sits beside `CombatWound` and
/// `StandWound` in the same chooser and is selected by the same flag. It is the *big flinch*, not the big swing.
///
/// Playing it on the attacker was a visible bug rather than a subtle one: at a
/// twenty-odd percent crit rate, a fifth of every character's blows played a
/// **being-hit** animation instead of a swing, which reads exactly as "my
/// attack animation keeps being interrupted by a hit reaction". See
/// [`reaction`], which is where the id went.
///
/// Every character model carries 87, 88 and 10 (16 of 16, `vale anim`);
/// creatures carry none of them, and their chains in [`fallbacks`] end at the
/// unarmed swing, which is a wolf's bite.
pub(super) fn swing(world: &WorldEntity, sheath: u8) -> u16 {
    use vale_protocol::play::action::hit_info;
    if world.last_swing_info & hit_info::LEFT_SWING != 0 {
        return off_hand(world, sheath).off_attack();
    }
    drawn_weapon(world, sheath).attack()
}

/// `UNIT_FIELD_BYTES_2` byte 0's three values, from the one place that defines
/// them — the same constants `vale_assets::look::dress` hangs the models from, and
/// `vale_assets::look::sheath` decides.
pub(super) use vale_assets::look::sheath::{MELEE as SHEATH_STATE_MELEE, RANGED as SHEATH_STATE_RANGED};

/// Which reaction a `VictimState` calls for on the unit that received the blow.
///
/// **Three of the nine states have a pose of their own** and the rest mean the
/// blow landed, which is the flinch. A parry is made with whatever is in the
/// hands, so it goes through the weapon family; a dodge is the whole body and a
/// block is the shield, and neither depends on what is held.
///
/// **…and a blow that landed has two flinches, chosen by the critical bit.**
/// The client's own wound chooser has a whole shape of three answers off one
/// argument: 10 `CombatCritical` for a critical; otherwise 9 `CombatWound` in
/// a combat stance and 8 `StandWound` when not.
///
/// The 9-against-8 branch is the same
/// "is this unit fighting" question the stance already asks, and it is answered
/// here by the model's own fallback chain (`COMBAT_WOUND` falls back to
/// `STAND_WOUND`) rather than by a second test.
///
/// **10 earns its place twice over.** It is the file's own answer for a
/// critical, and it is in the client's combat set (10, 16..24,
/// 30, 36, 57..59, 85..88, 95, 117..118) — so a critical taken mid-swing goes
/// through the combat fast path and *speeds the swing up and parks* instead of
/// cutting it off, where an ordinary `CombatWound` replaces it. That asymmetry
/// is the client's and not a preference.
pub(super) fn reaction(world: &WorldEntity, sheath: u8) -> u16 {
    use vale_protocol::play::action::hit_info;
    match world.last_victim_state {
        victim_state::DODGE => anim::DODGE,
        victim_state::PARRY => drawn_weapon(world, sheath).parry(),
        victim_state::BLOCKS => anim::SHIELD_BLOCK,
        _ if world.last_blow_info & hit_info::CRITICAL_HIT != 0 => anim::COMBAT_CRITICAL,
        _ => anim::COMBAT_WOUND,
    }
}

/// `UnitStandStateType` (`UnitDefines.h`), the values `UNIT_FIELD_BYTES_1`'s
/// low byte takes. 7 (`DEAD`) is not here: death is decided by health, which the
/// server always writes, where the stand state is only written when it wants a
/// particular pose.
/// `GOState` (`GameObjectDefines.h`): the reset state — a door shut, a chest
/// with its lid down. 0 is `GO_STATE_ACTIVE` and 2 the alternative used state,
/// and both of those are "open".
pub(super) const GO_STATE_READY: u8 = 1;

pub(super) const STAND_STATE_SIT: u8 = 1;
pub(super) const STAND_STATE_SIT_CHAIR: u8 = 2;
pub(super) const STAND_STATE_SLEEP: u8 = 3;
pub(super) const STAND_STATE_SIT_LOW_CHAIR: u8 = 4;
pub(super) const STAND_STATE_SIT_MEDIUM_CHAIR: u8 = 5;
pub(super) const STAND_STATE_SIT_HIGH_CHAIR: u8 = 6;
pub(super) const STAND_STATE_KNEEL: u8 = 8;

/// The fallback chain for an animation id: what to play when the model has no
/// sequence for what was asked.
///
/// **A model that has none of what was asked for plays whatever it does have**,
/// and the chain is what decides which. It matters more than it looks: 42 of the
/// game's 405 animated models have no Run, and a critter has no attack at all.
/// The last entry of every chain is Stand, which every animated model has.
pub(super) fn fallbacks(wanted: u16) -> &'static [u16] {
    match wanted {
        anim::RUN => &[anim::RUN, anim::WALK, anim::STAND],
        anim::WALK => &[anim::WALK, anim::RUN, anim::STAND],
        // **Reversing, and the chain is the file's own.** `AnimationData.dbc`
        // carries a fallback column and it says `Walkbackwards -> Walk ->
        // Stand`; the `Run` in the middle is this client's, on the same
        // reasoning as the two chains above — 42 of the game's 405 animated
        // models have no Walk, and one of those reversing should move rather
        // than stand.
        anim::WALK_BACKWARDS => &[
            anim::WALK_BACKWARDS,
            anim::WALK,
            anim::RUN,
            anim::STAND,
        ],
        // **The two stealth clips, and their chains are the file's own.**
        // `AnimationData.dbc` row 119's fallback column reads 4 (`Walk`) and
        // row 120's reads 0 (`Stand`), which is exactly right: a creature with
        // no crouch that the server has hidden should walk and stand, not
        // freeze. The `Run` after `Walk` is this client's, on the same
        // reasoning the gait chains above use — 42 of the 405 animated models
        // have no `Walk`, and one of those creeping should still move.
        //
        // These chains are what make the rule safe to apply to *every* unit
        // with the creep bit rather than to characters only, and the split is
        // total: **all sixteen character models carry both clips and 0 of the
        // 411 creature models carry either** (`vale anim`, whose population
        // line prints all three of these). So a prowling cat, which is the
        // commonest creeping creature in the game, takes the fallback and goes
        // on walking — which is what 5875 does, not a gap.
        anim::STEALTH_WALK => &[
            anim::STEALTH_WALK,
            anim::WALK,
            anim::RUN,
            anim::STAND,
        ],
        anim::STEALTH_STAND => &[anim::STEALTH_STAND, anim::STAND],
        // **The all-out run**, whose fallback column reads 5 (`Run`) — so a
        // model with no `Sprint` runs, which is **every one of the 411 creature
        // models** and therefore every mount. `Walk` and `Stand` after it are
        // this client's, for the reason the `Run` chain itself has them.
        anim::SPRINT => &[anim::SPRINT, anim::RUN, anim::WALK, anim::STAND],
        // **The turn-in-place foot-shuffle**, carried by 104 of the 411 models —
        // the humanoids, which are also the only things a player ever watches
        // turn on the spot. It falls back to Stand and to *nothing else*: the
        // character is not travelling, so substituting a gait would run it on
        // the spot, which is the one thing worse than not shuffling.
        anim::SHUFFLE_LEFT => &[anim::SHUFFLE_LEFT, anim::STAND],
        anim::SHUFFLE_RIGHT => &[anim::SHUFFLE_RIGHT, anim::STAND],
        // **The air.** `Jump` is the mid-flight loop and `Fall` is the one for
        // an arc nobody jumped into; each falls back to the other, because a
        // model that has one and not the other is better off playing the wrong
        // airborne pose than standing to attention over a cliff.
        //
        // The *ends* of the arc — `JumpStart`, `JumpEnd` and `JumpLandRun` —
        // are one-shots fired by [`Playback::note_flight`] and deliberately
        // have **no chain at all**, which is what the `_ => &[]` arm at the
        // bottom means: played if the model has them, dropped otherwise. That
        // is not laziness, it is the only safe form. A one-shot is held for the
        // length of the sequence it resolved to, so substituting Stand for a
        // missing `JumpEnd` would freeze a landing wolf upright for the 2.6
        // seconds of its idle loop while it ran away underneath.
        anim::JUMP => &[anim::JUMP, anim::JUMP_START, anim::FALL, anim::STAND],
        anim::FALL => &[anim::FALL, anim::JUMP, anim::STAND],
        // **The rider's pose, and the chain is the file's own.**
        // `AnimationData.dbc` row 91's fallback column reads 0, `Stand`, and
        // there is nothing sensible between the two: a model with no `Mount`
        // is not a thing that rides, and every seated pose in the table is
        // authored for a chair rather than for a saddle. All eighteen
        // character models carry it, so the fallback is for a *creature* the
        // server has mounted — a kodo rider's kodo — and standing upright on
        // the saddle is the honest failure.
        anim::MOUNT => &[anim::MOUNT, anim::STAND],
        // **The loot crouch, and there is nothing between it and standing.**
        // All sixteen character models carry it and only a player ever loots,
        // so this chain is for a shape that cannot happen rather than for one
        // that does — and every seated pose in the table is authored for a
        // chair rather than for a body on the ground, so substituting one would
        // be worse than not crouching.
        anim::LOOT => &[anim::LOOT, anim::STAND],
        // Death has no substitute worth playing — a creature that dies into its
        // idle loop reads as one that did not die — so a model without it simply
        // holds Stand, which is what it did before any of this.
        anim::DEATH => &[anim::DEATH, anim::STAND],

        // **The swings, one chain per weapon family.** Each falls back through
        // the families a model is likeliest to have instead, and every one ends
        // at the unarmed swing before Stand: a critter has no attack at all, and
        // a wolf has only its bite, which is `ATTACK_UNARMED`.
        anim::ATTACK_UNARMED => &[
            anim::ATTACK_UNARMED,
            anim::ATTACK_1H,
            anim::ATTACK_2H,
            anim::STAND,
        ],
        anim::ATTACK_1H => &[anim::ATTACK_1H, anim::ATTACK_UNARMED, anim::STAND],
        // **The stab**, which the file itself falls back to the swing
        // (`AnimationData.dbc` row 85 -> 17). Every character model carries it;
        // no creature does, and a creature the server says stabbed had better
        // do whatever it can.
        anim::ATTACK_1H_PIERCE => &[
            anim::ATTACK_1H_PIERCE,
            anim::ATTACK_1H,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        anim::ATTACK_2H => &[
            anim::ATTACK_2H,
            anim::ATTACK_2HL,
            anim::ATTACK_1H,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        anim::ATTACK_2HL => &[
            anim::ATTACK_2HL,
            anim::ATTACK_2H,
            anim::ATTACK_1H,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        // A ranged attack falls through to the melee ones rather than to Stand:
        // a creature with a bow model and no bow animation still has to look
        // like it is doing something when it shoots.
        anim::ATTACK_BOW => &[anim::ATTACK_BOW, anim::ATTACK_RIFLE, anim::ATTACK_UNARMED, anim::STAND],
        anim::ATTACK_RIFLE => &[anim::ATTACK_RIFLE, anim::ATTACK_BOW, anim::ATTACK_UNARMED, anim::STAND],
        anim::ATTACK_THROWN => &[anim::ATTACK_THROWN, anim::ATTACK_UNARMED, anim::STAND],
        // **The left hand's three, and the critical** — the swings the *packet*
        // and the off-hand item choose rather than the main hand. All of them
        // fall through the one-handed swing to the unarmed one: every character
        // model has 87 and 88 and no creature model has any, so what these
        // chains actually cover is a creature the server says swung with its
        // left, and the honest answer there is the ordinary attack it does have.
        // None falls through the *two*-handed swings, because a model with a 2H
        // attack and no `AttackOff` is a creature with no off hand at all.
        anim::ATTACK_OFF => &[
            anim::ATTACK_OFF,
            anim::ATTACK_1H,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        anim::ATTACK_OFF_PIERCE => &[
            anim::ATTACK_OFF_PIERCE,
            anim::ATTACK_OFF,
            anim::ATTACK_1H,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        // …and the empty left hand, whose first step is the file's own
        // (row 117 -> 87). It matters more than its siblings because
        // `HumanMale.m2` genuinely lacks it: a sword-and-board fighter's
        // off-hand blow lands on `AttackOff` for every human in the game.
        anim::ATTACK_UNARMED_OFF => &[
            anim::ATTACK_UNARMED_OFF,
            anim::ATTACK_OFF,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        // **The critical flinch falls back through the ordinary one**, not
        // through the swings: 10, 9 and 8 are the three answers of one chooser
        // and a model that lacks the big flinch still has the
        // small one. This chain used to end at `ATTACK_1H`, which was the other
        // half of reading 10 as the attacker's blow — see [`swing`].
        anim::COMBAT_CRITICAL => &[
            anim::COMBAT_CRITICAL,
            anim::COMBAT_WOUND,
            anim::STAND_WOUND,
            anim::STAND,
        ],

        // The flinch: 9 is the version for a unit already in a combat stance and
        // 8 the standing one; models carry one, the other, or neither.
        anim::COMBAT_WOUND => &[anim::COMBAT_WOUND, anim::STAND_WOUND, anim::STAND],
        // **The three defensive reactions.** Each falls back to the flinch
        // rather than to Stand: a creature with no dodge that dodges should
        // still visibly react, because the alternative is a wolf standing
        // motionless while the combat log fills with misses.
        anim::DODGE => &[anim::DODGE, anim::COMBAT_WOUND, anim::STAND_WOUND, anim::STAND],
        anim::SHIELD_BLOCK => &[anim::SHIELD_BLOCK, anim::PARRY_1H, anim::COMBAT_WOUND, anim::STAND],
        anim::PARRY_UNARMED => &[anim::PARRY_UNARMED, anim::PARRY_1H, anim::COMBAT_WOUND, anim::STAND],
        anim::PARRY_1H => &[anim::PARRY_1H, anim::PARRY_UNARMED, anim::COMBAT_WOUND, anim::STAND],
        anim::PARRY_2H => &[anim::PARRY_2H, anim::PARRY_2HL, anim::PARRY_1H, anim::COMBAT_WOUND, anim::STAND],
        anim::PARRY_2HL => &[anim::PARRY_2HL, anim::PARRY_2H, anim::PARRY_1H, anim::COMBAT_WOUND, anim::STAND],

        // The ready stances, one per family, each falling back to the unarmed
        // guard every humanoid model carries.
        anim::READY_UNARMED => &[
            anim::READY_UNARMED,
            anim::READY_1H,
            anim::READY_2H,
            anim::STAND,
        ],
        anim::READY_1H => &[anim::READY_1H, anim::READY_UNARMED, anim::STAND],
        anim::READY_2H => &[anim::READY_2H, anim::READY_2HL, anim::READY_1H, anim::READY_UNARMED, anim::STAND],
        anim::READY_2HL => &[anim::READY_2HL, anim::READY_2H, anim::READY_1H, anim::READY_UNARMED, anim::STAND],
        anim::READY_BOW => &[anim::READY_BOW, anim::READY_RIFLE, anim::READY_UNARMED, anim::STAND],
        anim::READY_RIFLE => &[anim::READY_RIFLE, anim::READY_BOW, anim::READY_UNARMED, anim::STAND],
        anim::READY_THROWN => &[anim::READY_THROWN, anim::READY_UNARMED, anim::STAND],

        // **The cast, and the chain is the whole point of it.** `SpellCast`
        // (32) and `SpellPrecast` (31) are what `AnimationData` calls them —
        // and **no character model carries
        // either**: `HumanMale.m2` has 51..54, the directed/omni pairs, and
        // nothing at 31 or 32. A client that asked for 32 and stopped would
        // animate no player's cast at all while looking entirely correct.
        anim::SPELL_CAST => &[
            anim::SPELL_CAST,
            anim::SPELL_CAST_OMNI,
            anim::SPELL_CAST_DIRECTED,
            anim::SPELL,
            anim::STAND,
        ],
        anim::SPELL_PRECAST => &[
            anim::SPELL_PRECAST,
            anim::READY_SPELL_OMNI,
            anim::READY_SPELL_DIRECTED,
            anim::CHANNEL_CAST_OMNI,
            anim::CHANNEL_CAST_DIRECTED,
            anim::SPELL,
            anim::STAND,
        ],
        // **The four the spell tables actually name**, which is what a cast
        // resolves to whenever the chain has them: `SpellVisual` gives a
        // fireball the *directed* pair and a heal the *omni* one, and a client
        // that asked for the generic id would play the same pose for both.
        //
        // They keep chains of their own because a **creature** casting a
        // player's spell is ordinary — a Defias mage throws the same fireball —
        // and a creature model carries at most `Spell` (2) and often nothing.
        // Each therefore prefers its own pair, then the generic id, before
        // giving up.
        anim::READY_SPELL_DIRECTED => &[
            anim::READY_SPELL_DIRECTED,
            anim::READY_SPELL_OMNI,
            anim::SPELL_PRECAST,
            anim::SPELL,
            anim::STAND,
        ],
        anim::READY_SPELL_OMNI => &[
            anim::READY_SPELL_OMNI,
            anim::READY_SPELL_DIRECTED,
            anim::SPELL_PRECAST,
            anim::SPELL,
            anim::STAND,
        ],
        anim::SPELL_CAST_DIRECTED => &[
            anim::SPELL_CAST_DIRECTED,
            anim::SPELL_CAST_OMNI,
            anim::SPELL_CAST,
            anim::SPELL,
            anim::STAND,
        ],
        anim::SPELL_CAST_OMNI => &[
            anim::SPELL_CAST_OMNI,
            anim::SPELL_CAST_DIRECTED,
            anim::SPELL_CAST,
            anim::SPELL,
            anim::STAND,
        ],
        // A channel is held the way a wind-up is, so it falls back onto the
        // wind-up rather than onto the release: a caster whose model has no
        // channel animation should stand ready, not throw the spell repeatedly.
        anim::CHANNEL_CAST_DIRECTED => &[
            anim::CHANNEL_CAST_DIRECTED,
            anim::CHANNEL_CAST_OMNI,
            anim::READY_SPELL_DIRECTED,
            anim::SPELL,
            anim::STAND,
        ],
        anim::CHANNEL_CAST_OMNI => &[
            anim::CHANNEL_CAST_OMNI,
            anim::CHANNEL_CAST_DIRECTED,
            anim::READY_SPELL_OMNI,
            anim::SPELL,
            anim::STAND,
        ],

        anim::SWIM => &[anim::SWIM, anim::SWIM_IDLE, anim::RUN, anim::STAND],
        // A swimmer at rest. Falls back to Swim rather than to Stand, because a
        // creature treading water reads better than one standing upright in it.
        anim::SWIM_IDLE => &[anim::SWIM_IDLE, anim::SWIM, anim::STAND],
        // **The three other strokes**, each through Swim. `AnimationData.dbc`
        // states that chain for `SwimBackwards` outright and sends the two side
        // strokes straight to Stand; going through Swim instead is this
        // client's, and it is the same trade the gaits take — a creature with
        // one stroke swimming sideways should swim.
        anim::SWIM_BACKWARDS => &[
            anim::SWIM_BACKWARDS,
            anim::SWIM,
            anim::SWIM_IDLE,
            anim::STAND,
        ],
        anim::SWIM_LEFT => &[anim::SWIM_LEFT, anim::SWIM, anim::SWIM_IDLE, anim::STAND],
        anim::SWIM_RIGHT => &[anim::SWIM_RIGHT, anim::SWIM, anim::SWIM_IDLE, anim::STAND],
        // The seated poses. A model with no sitting animation stands, which is
        // wrong and visible rather than wrong and silent.
        anim::SIT_CHAIR_MED => &[
            anim::SIT_CHAIR_MED,
            anim::SIT_CHAIR_LOW,
            anim::SIT_GROUND,
            anim::STAND,
        ],
        anim::SIT_CHAIR_LOW => &[anim::SIT_CHAIR_LOW, anim::SIT_GROUND, anim::STAND],
        anim::SIT_CHAIR_HIGH => &[
            anim::SIT_CHAIR_HIGH,
            anim::SIT_CHAIR_MED,
            anim::SIT_GROUND,
            anim::STAND,
        ],
        anim::SIT_GROUND => &[anim::SIT_GROUND, anim::STAND],
        // **The two held states of a game object.** Each ends at Stand, which
        // is what carries a game object whose model is a plain prop — a waving
        // banner, a brazier — through to its own idle. The *file's* chain runs
        // the other way (`Closed -> Close -> Open`, and `Open -> Close` back
        // again, a genuine cycle) which is right for a client walking it a step
        // at a time and wrong as a list to search: an open chest with no
        // `Opened` would end up playing the closing animation and holding its
        // last frame, which is a shut chest.
        anim::CLOSED => &[anim::CLOSED, anim::STAND],
        anim::OPENED => &[anim::OPENED, anim::CLOSED, anim::STAND],
        anim::SLEEP => &[anim::SLEEP, anim::SIT_GROUND, anim::STAND],
        anim::KNEEL => &[anim::KNEEL, anim::SIT_GROUND, anim::STAND],
        // **No chain.** An id this table says nothing about is played if the
        // model has it and dropped if not — see [`Playback::resolve`]. That is
        // the 78 emotes, which are an arbitrary spread of `AnimationData` ids
        // and which a substitution would turn into a character standing still.
        _ => &[],
    }
}

/// A world-space direction in the model's own space: the world turned by minus
/// the entity's facing.
pub(super) fn into_model_space(v: [f32; 3], facing: f32) -> [f32; 3] {
    let (s, c) = facing.sin_cos();
    [c * v[0] + s * v[1], -s * v[0] + c * v[1], v[2]]
}

impl Playback {
    /// A clock that has not played anything yet, on `skeleton`.
    ///
    /// The `usize::MAX` sequence and the `u16::MAX` id are what make the first
    /// [`Self::advance`] a *first* play — it takes up whatever the state says
    /// with nothing to fade out of. The counters are `None` for the reason
    /// [`Self::seen`] gives: the first poll records and never fires, so a
    /// creature that walks into view mid-fight does not replay the blows landed
    /// before anyone was looking.
    ///
    /// A constructor rather than a struct literal because there are two callers
    /// now — an entity's own rig and the mount underneath it — and twenty
    /// fields' worth of defaults is exactly the shape that drifts.
    pub(super) fn new(skeleton: Arc<M2Skeleton>, now: f32) -> Playback {
        Playback {
            hints: Vec::new(),
            window: None,
            wanted: u16::MAX,
            sequence: usize::MAX,
            since: now,
            fade: None,
            oneshot: None,
            seen: None,
            casting: None,
            aura: None,
            frozen: None,
            freeze: false,
            overlay: None,
            speed: 0.0,
            base_rate: 1.0,
            deferred: None,
            skeleton,
            played: None,
        }
    }

    /// **Record the window this frame advanced through** — see
    /// [`Playback::window`]. Called by `animate` right after
    /// [`Self::advance`], before the frustum gate, so a rig behind the
    /// camera still fires its cues.
    pub(super) fn note_window(&mut self, elapsed: u32) {
        self.window = Some(match self.window {
            Some((sequence, _, before, _)) if sequence == self.sequence => {
                (sequence, before, elapsed, false)
            }
            _ => (self.sequence, 0, elapsed, true),
        });
    }

    /// The base sequence's window this frame: `(sequence, from, to, fresh)`
    /// — see [`vale_assets::world::m2::SoundCues::in_window`]. `None`
    /// before the first frame.
    pub fn window(&self) -> Option<(usize, u32, u32, bool)> {
        self.window
    }

    /// **Which of the model's sequences the base track is playing**, or `None`
    /// before the first [`Self::advance`].
    ///
    /// The one reader is the mouse pick, which needs the sequence's own
    /// bounding sphere — the reference reads exactly this, off its animator,
    /// and it is the difference between hovering a wisp and hovering
    /// the twelve-yard cube of dust its file declares. See
    /// [`super::EntityModel::pick_sphere`].
    ///
    /// **The base track and deliberately not the overlay**, which is what the
    /// reference has: a swing played over a run does not restate the box, and a
    /// masked clip has no box of its own to state.
    pub fn clip(&self) -> Option<&vale_assets::world::m2::M2Sequence> {
        self.skeleton.sequences.get(self.sequence)
    }

    /// Take up the animation `wanted` and return how long the playing one has
    /// been running, in milliseconds.
    ///
    /// **Only a change of *sequence* restarts the clock.** 42 of the game's 405
    /// animated models have no Run, and one with neither gait resolves Run and
    /// Stand onto the same idle loop — restarting that every time the creature
    /// starts or stops moving is the animation visibly resetting for no reason,
    /// which is exactly what "the skinning is broken" looks like.
    pub(super) fn advance(&mut self, state: u16, now: f32) -> u32 {
        // **The masked track expires first**, and the order is what lets a
        // moving caster's wind-up retake the torso the frame after a swing over
        // it finishes: `state_or_held` below only takes a slot that is free.
        if self.overlay.as_ref().is_some_and(|m| now >= m.until) {
            self.overlay = None;
        }
        // **Death is the one state that is not queued behind anything.**
        // [`Self::fire`] already refuses to *start* a clip on a corpse — a
        // creature that flinches at the blow that killed it would stand back up
        // to do it — and this is the same rule for a clip that was already
        // running when the blow landed. Without it a mob killed mid-swing waits
        // out the rest of its swing, and its two-second attack animation is two
        // seconds of a body that is already dead standing up and fighting,
        // which is what "death animations are often delayed" is.
        //
        // All three, because a one-shot can be on either track and a parked
        // swing would otherwise be played by the consumer at the end of
        // [`Self::note_actions`] the frame after the corpse hit the floor.
        if state == anim::DEATH {
            self.oneshot = None;
            self.overlay = None;
            self.deferred = None;
        }
        // A one-shot outranks the state while it lasts, and expires on its own
        // clock rather than on a change of state: a creature that starts running
        // half way through its swing finishes the swing.
        let mut ran_out = None;
        let wanted = match self.oneshot.as_ref() {
            Some(shot) if now < shot.until => shot.wanted,
            Some(shot) => {
                ran_out = Some(shot.until);
                self.oneshot = None;
                self.state_or_held(state, now)
            }
            None => self.state_or_held(state, now),
        };

        if self.wanted != wanted {
            self.wanted = wanted;
            // A change of state **is** a `PlayAnimation`, whether or not the
            // model resolves it to a different sequence — so the sheath
            // reconcile sees it. See [`Playback::played`].
            self.played = Some(wanted);
            self.take_up(wanted, now);
            // **What is fading out is the one-shot that just ran out**, and it
            // is sampled where it stopped — see [`Fade::held`].
            if let (Some(ended), Some(fade)) = (ran_out, self.fade.as_mut()) {
                if fade.from == now {
                    fade.held = Some(ended);
                }
            }
        }
        // A fade that has run its course is dropped rather than left to go
        // negative — `pose` would ignore it, but it would also keep sampling a
        // second animation for the rest of the session.
        if self.fade.as_ref().is_some_and(|f| now - f.from >= FADE_SECS) {
            self.fade = None;
        }

        let elapsed = self.clock_at(self.sequence, now - self.since, self.base_rate);
        // **A corpse holds the last frame of Death.** Id 6 (`Dead`) is in
        // `AnimationData.dbc` and in none of the game's 411 creature models, so
        // there is nothing to play afterwards — and letting the clock loop plays
        // Death again, which is a body repeatedly falling over.
        //
        // `M2Skeleton::pose` clamps to the window, so the hold is simply the
        // clock left alone; everything else is a gait or an idle and wraps.
        if self.wanted == anim::DEATH {
            return elapsed;
        }
        // **…and the loot crouch holds its last frame for the same reason**,
        // which is a missing clip rather than a missing state. `Loot` is the
        // *down* half of a triple whose hold (`LootHold`, 188) and return
        // (`LootUp`, 189) are in `AnimationData.dbc` and in none of the sixteen
        // character models; the clip is a 500 ms descent that never comes back
        // up, so its last frame is the crouch with the arm out. Letting the
        // clock wrap plays the descent again, which is a character bobbing at a
        // corpse. See `anim::LOOT` for the nine-phase measurement.
        if self.wanted == anim::LOOT {
            return elapsed;
        }
        let phase = self.skeleton.phase(self.sequence, elapsed);
        // **A stun with no pose of its own holds the clock**, which is Ice Block
        // — see [`Playback::frozen`], which says what is measured here and what
        // is not. Latched at the phase the freeze began on rather than
        // recomputed, so the picture cannot creep; dropped the moment the flag
        // clears, and the ordinary loop resumes from wherever it would have
        // been, because `since` was never touched.
        if self.freeze {
            return *self.frozen.get_or_insert(phase);
        }
        self.frozen = None;
        phase
    }

    /// A held pose over the state, from whichever of the two things can state
    /// one — and the state itself when neither does.
    ///
    /// **Two claimants on one slot, and they are different kinds of claim.** A
    /// cast's wind-up is a *moment* with a length `SMSG_SPELL_START` states; an
    /// aura's pose is a *condition* that ends when the server stops saying the
    /// aura is there. The cast wins where both apply, because it is the thing
    /// that just happened — in practice they barely meet, since the server
    /// interrupts a stunned caster.
    fn state_or_held(&mut self, state: u16, now: f32) -> u16 {
        // A corpse holds nothing, and neither does a spent cast bar. The timer
        // is what stops an interrupted cast leaving a character frozen in the
        // wind-up for the rest of the session.
        if let Some(cast) = self.casting.as_ref() {
            if now >= cast.until || state == anim::DEATH {
                self.casting = None;
                self.drop_held();
            }
        }
        let hold = match self.casting.as_ref() {
            Some(cast) => Some(cast.hold),
            // **A corpse is not stunned either.** The aura outlives the unit on
            // the wire — vmangos clears it a tick later — and a body cowering on
            // the floor is exactly the "plausibly wrong" this file is about.
            None if state != anim::DEATH => self.aura,
            None => None,
        };
        let Some(hold) = hold else {
            // Nothing claims the slot, so nothing may be left holding it. A
            // no-op today (every site that clears `casting` already drops the
            // held overlay beside it) and the guard that keeps it one once a
            // second claimant can come and go without a counter moving.
            self.drop_held();
            return state;
        };
        match self.hold_over(state, hold, now) {
            Some(wanted) => wanted,
            None => {
                // **Nowhere to put it.** A model with no `SpineLow` cannot mask,
                // so the choice is the whole body or nothing — and a creature
                // that walks away mid-cast has been interrupted, which is what
                // this did for every model before the overlay existed. An
                // *aura* is not interrupted by walking, so only the cast is
                // dropped; the pose simply does not draw while the legs are
                // busy, and retakes the body the moment they stop.
                self.casting = None;
                state
            }
        }
    }

    /// Put `hold` on, over the whole body or over the torso, and answer what the
    /// **base** track should play — `None` when there was nowhere to put it.
    ///
    /// **A held pose is pinned to the whole body only while the unit is standing
    /// still.** Once it moves, the pose does not stop — it moves to the torso,
    /// as a *held loop* on the masked overlay, and the legs run out from under
    /// it. That is the client's own split (a movement-flags `& 0x20000f`
    /// gate).
    ///
    /// **The foot-shuffle is deliberately not in the moving list**, and the
    /// client says so in two different masks: the one that unpins a
    /// cast is the direction bits plus swim and *nothing else*, where the one
    /// that routes a one-shot ([`route_oneshot`]) folds the turn keys in.
    /// Reading the second as the first makes a cast flap between the two routes
    /// at mouse-event cadence.
    fn hold_over(&mut self, state: u16, hold: u16, now: f32) -> Option<u16> {
        // Every gait, not a sample of them: the list used to name Run, Walk,
        // Swim and SwimIdle, so the moment reversing and side-stroking became
        // sequences of their own a caster who backed away kept their hands up.
        let moving = matches!(
            state,
            anim::RUN
                | anim::WALK
                | anim::WALK_BACKWARDS
                | anim::SWIM
                | anim::SWIM_IDLE
                | anim::SWIM_LEFT
                | anim::SWIM_RIGHT
                | anim::SWIM_BACKWARDS
        );
        if !moving {
            // Standing: the pin, and the torso is released back to it.
            self.drop_held();
            return Some(hold);
        }
        // Masked onto the torso: the legs keep the state.
        self.hold_masked(hold, now).then_some(state)
    }

    /// Take the masked track with a **held** wind-up, if it is free to take.
    ///
    /// False when the model cannot mask at all — see [`Self::state_or_held`],
    /// which is what then drops the cast. A masked *one-shot* in the slot wins
    /// while it plays and this answers true anyway: the swing is the cast's own
    /// arm doing something else, and the hold retakes the subtree the frame the
    /// shot ends.
    fn hold_masked(&mut self, hold: u16, now: f32) -> bool {
        if self.skeleton.key_bone(key_bone::SPINE_LOW).is_none() {
            return false;
        }
        match &self.overlay {
            // Already held, and by this same spell.
            Some(m) if m.looping && m.wanted == hold => return true,
            // A one-shot is playing over it: leave it alone and come back.
            Some(m) if !m.looping => return true,
            _ => {}
        }
        // Taking the torso is a play like any other, so the reconcile sees it —
        // which is what stows a moving caster's weapon.
        self.played = Some(hold);
        let Some(sequence) = self.resolve(hold) else {
            return false;
        };
        self.overlay = Some(Masked {
            wanted: hold,
            sequence,
            rate: 1.0,
            since: now,
            until: f32::INFINITY,
            looping: true,
        });
        true
    }

    /// Release a held wind-up from the torso, leaving a one-shot alone.
    fn drop_held(&mut self) {
        if self.overlay.as_ref().is_some_and(|m| m.looping) {
            self.overlay = None;
        }
    }

    /// What the upper body is playing this frame, if anything.
    ///
    /// A held wind-up wraps its clock and a one-shot holds its last frame,
    /// which is the same split [`Self::advance`] makes for the base track and
    /// for the same reason.
    pub(super) fn overlay_at(&self, now: f32) -> Option<Overlay> {
        let masked = self.overlay.as_ref()?;
        let raw = self.clock_at(masked.sequence, now - masked.since, masked.rate);
        Some(Overlay {
            sequence: masked.sequence,
            elapsed_ms: if masked.looping {
                self.skeleton.phase(masked.sequence, raw)
            } else {
                raw
            },
        })
    }

    /// Which of the model's sequences an animation id resolves to.
    ///
    /// **An id with no chain of its own is played or dropped, never
    /// substituted.** That is the emotes: `Emotes.dbc` names 78 of them and a
    /// model carries the handful its race was animated for, so most of them
    /// resolve to nothing on most models. Falling back to Stand there would
    /// make a `/train` at a tauren interrupt whatever it was doing in order to
    /// stand still, which is worse than ignoring the emote.
    /// `find_sequence` rather than `best_sequence` for the no-chain case, and
    /// the difference is the whole point: `best_sequence` never fails — it ends
    /// with "whatever this model does have", which is right for a *state* (a
    /// creature with one idle loop beats a statue) and wrong for an emote, where
    /// the honest answer to "can you dance?" is sometimes no.
    fn resolve(&self, wanted: u16) -> Option<usize> {
        match fallbacks(wanted) {
            [] => self.skeleton.find_sequence(wanted),
            chain => self.skeleton.best_sequence(chain),
        }
    }

    fn take_up(&mut self, wanted: u16, now: f32) {
        // A model that has none of what was asked for plays whatever it does
        // have: a creature with one idle loop beats a statue. See `fallbacks`.
        let Some(sequence) = self.resolve(wanted) else {
            return;
        };
        if sequence == self.sequence {
            return;
        }
        let first = self.sequence == usize::MAX;
        // Nothing to fade out of on the very first pose.
        self.fade = (!first).then_some(Fade {
            sequence: self.sequence,
            since: self.since,
            from: now,
            held: None,
        });
        self.sequence = sequence;
        self.since = now;
        self.base_rate = 1.0;
        // **A body that was already dead the first time this client saw it does
        // not fall over on arrival.** Death is the one state whose clip is a
        // *transition into* the state rather than the state itself — everything
        // else here is a loop, so starting it at the beginning is free. A
        // creature that died out of sight streams in mid-corpse, and playing the
        // clip from the top makes it stand up and topple over as the tile loads,
        // which is exactly the kind of plausibly-wrong this file is about.
        //
        // The clock is wound back past the sequence's own length rather than
        // clamped, because `advance` deliberately leaves a corpse's clock alone
        // and `M2Skeleton::pose` clamps to the window — so this *is* the last
        // frame, by the same mechanism that holds it there afterwards.
        if first && wanted == anim::DEATH {
            let clip = &self.skeleton.sequences[sequence];
            self.since = now - clip.end.saturating_sub(clip.start) as f32 / 1000.0;
        }
    }

    /// Fire a one-shot: a swing, a flinch, an emote, a spell's release.
    ///
    /// **Which body it plays on is decided here and now**, off the state the
    /// unit is in at this moment — see [`route_oneshot`]. A standing character
    /// takes the clip through the whole body; one that is already running,
    /// swimming, seated or mid-jump takes it on the torso alone and keeps
    /// moving. The id has no say in it beyond which tests apply.
    ///
    /// Ignored on a corpse. A dead creature that flinches at the blow that
    /// killed it stands back up to do it.
    fn fire(&mut self, wanted: u16, world: &WorldEntity, state: u16, now: f32) {
        if state == anim::DEATH {
            return;
        }
        // **The combat fast-path, before anything else** — the client's own
        // head-of-function order. A swing thrown while another swing is still
        // playing does not cut it off; it speeds that one up and waits.
        if self.fast_path(wanted, now) {
            return;
        }
        // A normal arm clears the cache (the client resets it on every
        // non-fast-path play), so a parked swing is dropped by whatever
        // *outranked* it rather than played after it.
        self.deferred = None;
        if self.route(wanted, world) == Route::Masked {
            self.fire_masked(wanted, now);
            return;
        }
        self.fire_full_body(wanted, now);
    }

    /// **A combat clip requested while another combat clip is playing is not
    /// armed** — the client's combat fast-path.
    ///
    /// What happens instead is two things, and both of them are the answer to
    /// "fast attacks often do not play at all":
    ///
    /// * **the clip that is running has its rate set to 2x**, re-timing what is
    ///   left of it without moving the pose — so a swing half way through
    ///   finishes in half the time rather than being cut off mid-arc;
    /// * **the request parks** in a one-slot cache and plays the moment nothing
    ///   is live (see the consumer at the end of [`Self::note_actions`]).
    ///
    /// So a dagger swinging every 1.4 s against a 1.0 s clip loses nothing: the
    /// first swing compresses and the second follows it. Without this the second
    /// swing replaced the first outright, which is a blow that never reads as
    /// one — and a third arriving mid-clip replaced *that*, which is how a fast
    /// weapon ends up looking as though it is not swinging at all.
    ///
    /// **Both ids have to be combat ids**, which is [`combat_id`] — the client's
    /// own set. A flinch over a swing is deliberately not this case:
    /// being hit has to read immediately, and it takes the ordinary replacing
    /// path.
    ///
    /// The rate is *set* to 2 rather than doubled, because the client's 2.0 is
    /// an absolute speed: a third request against an already-doubled
    /// clip parks without shortening it a second time.
    fn fast_path(&mut self, wanted: u16, now: f32) -> bool {
        /// The client's fast-path rate.
        const FAST: f32 = 2.0;
        if !combat_id(wanted) {
            return false;
        }
        // Whichever track holds the live one-shot — the masked slot first,
        // because a swing thrown at a run is on it and the base track is then
        // carrying the *gait*, which is not a one-shot at all.
        if let Some(live) = self.overlay.as_ref().filter(|m| !m.looping && now < m.until) {
            if !combat_id(live.wanted) {
                return false;
            }
            let (wanted_live, sequence) = (live.wanted, live.sequence);
            let (rate, since, until) = (live.rate, live.since, live.until);
            self.overlay = Some(Masked {
                wanted: wanted_live,
                sequence,
                rate: FAST,
                // Pose-continuous: the phase at this instant is unchanged and
                // everything after it advances at the new rate.
                since: now - (now - since) * rate / FAST,
                until: now + (until - now) * rate / FAST,
                looping: false,
            });
            self.deferred = Some(wanted);
            return true;
        }
        if let Some(live) = self.oneshot.as_ref().filter(|s| now < s.until) {
            if !combat_id(live.wanted) {
                return false;
            }
            let (wanted_live, until) = (live.wanted, live.until);
            let rate = self.base_rate;
            self.since = now - (now - self.since) * rate / FAST;
            self.base_rate = FAST;
            self.oneshot = Some(OneShot {
                wanted: wanted_live,
                until: now + (until - now) * rate / FAST,
            });
            self.deferred = Some(wanted);
            return true;
        }
        false
    }

    /// Is either track still playing a one-shot? A held wind-up is not one — it
    /// is a loop with no end, and a parked swing goes over it.
    fn oneshot_live(&self, now: f32) -> bool {
        self.oneshot.as_ref().is_some_and(|s| now < s.until)
            || self
                .overlay
                .as_ref()
                .is_some_and(|m| !m.looping && now < m.until)
    }

    /// [`route_oneshot`], plus the one thing it cannot know: whether this
    /// model's skeleton **has** a `SpineLow` to mask at.
    ///
    /// A wolf has none — it has a `Head` and no spine key bone — and the
    /// client's fallback for that case is the whole body, which is also what
    /// this client did for every model before the overlay existed. So a running
    /// wolf's bite still reads.
    fn route(&self, wanted: u16, world: &WorldEntity) -> Route {
        let route = route_oneshot(wanted, world.move_flags, world.stand_state, world.airborne);
        if route == Route::Masked && self.skeleton.key_bone(key_bone::SPINE_LOW).is_some() {
            Route::Masked
        } else {
            Route::FullBody
        }
    }

    /// **A one-shot that started on the whole body moves to the torso the
    /// moment the legs commit.**
    ///
    /// [`Self::fire`] decides the route once, from the state at the instant of
    /// the play, and that is right for the *choice* — but the state it read
    /// does not stay true. A character that swings, or releases a cast, while
    /// standing still and then runs used to keep the standing clip on the base
    /// track for its whole length: the legs never took the gait up, so the
    /// character slid across the ground in the pose it was struck in. Reported
    /// exactly that way, and it is the mirror of a rule this file already has
    /// on the other side — [`Self::state_or_held`] re-asks the same question
    /// every frame for a held *wind-up*, which is why a cast begun standing
    /// and then walked out of already does the right thing.
    ///
    /// The move is **pose-continuous**: the clip keeps its clock, its rate and
    /// its end, so nothing about the upper body changes on the frame it
    /// happens. What changes is that the base track is released, and the next
    /// [`Self::advance`] fades it into the gait.
    ///
    /// **The reverse is deliberately not done.** A masked clip whose owner
    /// stops moving is left on the torso: the reference's own behaviour there
    /// is not established, and the visible difference is only what the legs do
    /// for the tail of one clip — where getting *this* direction wrong is a
    /// character sliding.
    fn rehome_oneshot(&mut self, world: &WorldEntity, now: f32) {
        let Some(shot) = self.oneshot.as_ref().filter(|s| now < s.until) else {
            return;
        };
        let (wanted, until) = (shot.wanted, shot.until);
        if self.route(wanted, world) != Route::Masked {
            return;
        }
        // A newer one-shot already owns the subtree — a swing thrown after the
        // run began. The older full-body clip is dropped rather than made to
        // fight it for the same bones; it was going to be replaced anyway.
        if self.overlay.as_ref().is_some_and(|m| !m.looping && now < m.until) {
            self.oneshot = None;
            return;
        }
        let Some(sequence) = self.resolve(wanted) else {
            return;
        };
        self.overlay = Some(Masked {
            wanted,
            sequence,
            // The base track's own clock, carried across whole — including the
            // 2x the combat fast path may have set on it.
            rate: self.base_rate,
            since: self.since,
            until,
            looping: false,
        });
        self.oneshot = None;
    }

    /// Play a one-shot on the masked track: the torso only.
    ///
    /// Replaces whatever was there — a second swing, or a held wind-up the arm
    /// is interrupting — because the slot is one subtree and the newest play
    /// owns it. The wind-up retakes it when this ends, since `state_or_held`
    /// asks again every frame.
    fn fire_masked(&mut self, wanted: u16, now: f32) {
        // Before the resolve, and deliberately: the reconcile tests the id that
        // was **asked for**, so a model with no such clip still reconciles.
        self.played = Some(wanted);
        let Some(sequence) = self.resolve(wanted) else {
            return;
        };
        let length = self.length_ms(sequence).unwrap_or(1000);
        self.overlay = Some(Masked {
            wanted,
            sequence,
            rate: 1.0,
            since: now,
            until: now + length as f32 / 1000.0,
            looping: false,
        });
    }

    /// Play a one-shot on the base track: the whole body.
    ///
    /// The duration comes from the sequence the model *resolved* to rather than
    /// from the id asked for, because the fallback chain may have landed
    /// somewhere else entirely — and a shot held for the length of an animation
    /// that is not playing either cuts off or freezes.
    fn fire_full_body(&mut self, wanted: u16, now: f32) {
        // As [`Self::fire_masked`]: the request, before the resolve.
        self.played = Some(wanted);
        let Some(sequence) = self.resolve(wanted) else {
            return;
        };
        // Restart the clock even when the sequence is the one already playing:
        // two swings in a row are two swings, not one held pose. That is the
        // case `take_up` deliberately declines, so this does not go through it.
        if sequence != self.sequence {
            self.fade = (self.sequence != usize::MAX).then_some(Fade {
                sequence: self.sequence,
                since: self.since,
                from: now,
                // A one-shot that had already run out when this one replaced
                // it stops where it stopped; one still playing keeps its clock.
                held: self.oneshot.as_ref().filter(|s| now >= s.until).map(|s| s.until),
            });
        }
        self.sequence = sequence;
        self.since = now;
        self.base_rate = 1.0;
        self.wanted = wanted;
        let length = self.length_ms(sequence).unwrap_or(1000);
        self.oneshot = Some(OneShot {
            wanted,
            until: now + length as f32 / 1000.0,
        });
    }

    /// Turn every moved counter into whatever it asks for.
    ///
    /// **The first poll only records, it never fires.** An entity that walks
    /// into view mid-fight arrives with a swing count of forty; playing forty
    /// swings — or even one — for blows that landed before anyone was looking is
    /// worse than playing none.
    ///
    /// **The order is a priority**, and it is what a single-animation skeleton
    /// forces: a model plays one sequence, so when two things happened in one
    /// poll only one of them is seen. A release outranks a swing because a spell
    /// landing is the more conspicuous event; a swing outranks a flinch because
    /// two units hitting each other simultaneously should each look like they
    /// are attacking rather than each look like they are being hit; and an emote
    /// comes last because it is the only one of the five a player chose to do
    /// and therefore the only one they will notice missing — but it is also the
    /// one that has no business interrupting a fight.
    pub(super) fn note_actions(
        &mut self,
        world: &WorldEntity,
        sheath: u8,
        emote_anim: impl FnOnce(u32) -> Option<u16>,
        // **The pose a `SpellVisualKit` holds, asked by kit id** — the only
        // resolver here that does not begin at a spell. See
        // [`vale_protocol::play::sound`]: two packets carry a kit and no
        // spell, and for the two that matter — food (406) and drink (438) — the
        // pose *is* the visible half, `animID 61`, `EmoteEat`.
        kit_anim: impl Fn(u32) -> Option<u16>,
        cast_anim: impl FnOnce(u32) -> CastAnimation,
        state: u16,
        now: f32,
    ) {
        // **Before the first-look guard**, because this is not an event: it is
        // how fast the legs should be turning over, and a rig that has only
        // ever been looked at once still has to play its gait at the right
        // rate. See [`Self::speed`].
        self.speed = world.speed;
        // …and before the first-look guard for the same reason: this is not an
        // event either, it is the answer to "are the legs still free?", asked
        // every frame because the answer changes under a clip that is already
        // playing. See [`Self::rehome_oneshot`].
        self.rehome_oneshot(world, now);
        let seen = Counters::of(world);
        let Some(last) = self.seen.replace(seen) else {
            return;
        };
        // **The two ends of the arc, and they go first so that they lose.**
        // Everything below fires afterwards and overwrites, which is the right
        // way round: a swing or a release is a discrete thing the server stated
        // exactly once and is gone, where the *flight* still has `Jump` or
        // `Fall` underneath it — losing the flourish at the take-off costs a
        // third of a second of ornament, losing the swing costs the event.
        //
        // **…except on a rider, where the whole arc belongs to the animal.**
        // [`wanted_animation`] puts `Mount` (91) above the air, so the middle of
        // the arc is not drawn here at all — and an end with no middle is a
        // character who sits still all the way up and then stands out of the
        // saddle to absorb the landing, which is the "jumping while mounted
        // causes this upon landing" report exactly. It was only ever the
        // landing, and that is the same fact from the other side: the take-off
        // below is gated on `state == JUMP`, which a mounted unit never is, and
        // the landing was gated on nothing. The mount fires its own two, through
        // [`Self::note_flight_only`].
        if !world.mounted {
            self.note_flight(seen, last, world, state, now);
        }
        // …and a door swinging, which is the same shape: two held poses with a
        // one-shot on the way between them.
        if seen.object_state != last.object_state && last.object_state != u8::MAX {
            self.fire(
                if seen.object_state == GO_STATE_READY {
                    anim::CLOSE
                } else {
                    anim::OPEN
                },
                world,
                state,
                now,
            );
        }
        // **What a cast looks like is a property of the spell**, and both
        // halves of it are resolved from one hop so the wind-up and the release
        // cannot disagree about which spell they are describing.
        let cast_moved =
            seen.casts_begun != last.casts_begun || seen.casts_released != last.casts_released;
        let spell = if cast_moved {
            cast_anim(world.last_spell)
        } else {
            CastAnimation::default()
        };

        // A cast beginning is a *state*, not a shot, so it is taken up outside
        // the priority order below — a caster who is also being hit should
        // flinch and then go back to their wind-up.
        //
        // **A spell that states no wind-up has none**, which is the same rule
        // the release below has always followed and which this half did not:
        // `hold.unwrap_or(SPELL_PRECAST)` invented one for every spell the chain
        // said nothing about, and `SpellPrecast` (31) is carried by no character
        // model, so it fell through its own chain to `ReadySpellOmni` (52),
        // which every one of them carries. That is **9,467 of the game's 22,360
        // spells** — `vale spell` counts 12,893 reaching an animation — every
        // proc, every aura application and every silent utility spell raising
        // its hands to cast.
        //
        // `SpellVisualKit`'s `animID` is the only thing in the game that says
        // what a cast looks like, and a spell with no visual, or a kit whose
        // `animID` is zero, is saying nothing on purpose.
        if seen.casts_begun != last.casts_begun && state != anim::DEATH {
            // **A channel's begin holds the channel's own pose**, which is a
            // different kit from the wind-up's and not a fallback for it.
            // `apply_channel_start` bumps this counter a second time and
            // restates the bar as the channel's length, so this arm runs twice
            // for a channelled spell: once for the wind-up and once for the
            // channel. Reading `hold` both times stood Blizzard's caster in
            // `ReadySpellOmni` for four seconds — 61 of the game's 323
            // channelled spells, measured in
            // `vale_assets::tables::spell::CastAnimation::channel`.
            let channelling = seen.casts_channelled != last.casts_channelled;
            let held = match channelling {
                true => spell.channel.or(spell.hold),
                false => spell.hold,
            };
            self.casting = held.map(|hold| Casting {
                hold,
                // A zero-length wind-up is an instant spell whose `START`
                // arrived anyway; the release is a hair behind it and clears
                // this.
                until: now + world.cast_time_ms as f32 / 1000.0,
            });
            // A cast replaces whatever was held, and one with no wind-up of its
            // own replaces it with nothing — or the *previous* spell's pose
            // stays on the torso for the length of this one's bar.
            if self.casting.is_none() {
                self.drop_held();
            }
        }

        // **A pushback is the wind-up lasting longer**, and it is the one thing
        // that happens to a cast in progress without ending it. `Casting::until`
        // was armed off `SMSG_SPELL_START`'s `m_timer` — the only statement of
        // the cast's length the wire makes — so a Fireball knocked back by two
        // blows dropped its held pose a second before it was thrown, and the
        // caster stood empty-handed while the bar was still running. It goes
        // *before* the cancellation below so that a cast pushed back and then
        // interrupted in the same poll still ends.
        if seen.casts_delayed != last.casts_delayed {
            if let Some(casting) = &mut self.casting {
                casting.until += world.last_cast_delay_ms as f32 / 1000.0;
            }
        }

        // **A cast taken off is the wind-up ending with nothing after it.**
        // Outside the priority chain below rather than an arm of it, because it
        // fires *nothing*: it is a stop, so it must not spend the slot a swing
        // or a flinch arriving in the same poll would take. See
        // `WorldEntity::casts_cancelled` for the three packets that move it.
        // Without it the pose was held to `Casting::until` — which for a
        // silenced Fireball is the rest of its bar with nothing at the end.
        if seen.casts_cancelled != last.casts_cancelled {
            self.casting = None;
            self.drop_held();
        }

        // **…and a channel's begin is *later* than its release**, so it survives
        // it. vmangos sends `SendSpellGo` and then `SendChannelStart`, both
        // inside one poll, and this block is written after the one that arms the
        // wind-up — so without the guard it cancelled the channel pose one line
        // after it was set and played the release instead. Evocation has no
        // release at all (`SpellVisual` gives it a channel kit and nothing
        // else), so what that came to on screen was a mage standing in its idle
        // loop for eight seconds. See `WorldEntity::casts_channelled`.
        if seen.casts_released != last.casts_released
            && seen.casts_channelled == last.casts_channelled
        {
            self.casting = None;
            // The wind-up leaves the torso with the cast it belonged to, or a
            // moving caster holds it for ever.
            self.drop_held();
            // **A spell that states no release has none**, and playing the
            // generic one anyway would throw a fireball at the end of a
            // channel: `SpellVisual` gives Arcane Missiles a channel kit and
            // nothing else.
            //
            // **Including a spell the chain said nothing at all about** (this
            // round), which used to be the one exception: `is_empty()` bought a
            // generic `SpellCast` for the 9,467 spells that reach no animation,
            // on the reasoning that the tables were not read yet. They are, and
            // "nothing" is an answer rather than a gap — see the wind-up above,
            // which is the same retraction and is the visible half of it.
            if let Some(release) = spell.release {
                self.fire(release, world, state, now);
            } else if let Some(shot) = spell.ranged_shot.then(|| ranged_shot(world)).flatten() {
                // **…except for the one release the chain does not carry.**
                // A ranged weapon attack's one-shot is the *wielder's* weapon
                // rather than the spell's art, and the two spells a player
                // repeats — Auto Shot (75) and the wand's Shoot (5019) — read
                // `SpellVisual = 0`, so the sentence above ("a spell that
                // states no release has none") would leave an archer standing
                // still through a whole volley. That is the report.
                //
                // Ordered under `release` rather than over it, because a ranged
                // *ability* that does name a kit — Aimed Shot, Multi-Shot —
                // means it: the file is more specific than the weapon.
                self.fire(shot, world, state, now);
            }
        } else if seen.swings != last.swings {
            self.fire(swing(world, sheath), world, state, now);
        } else if seen.blows != last.blows {
            self.fire(reaction(world, sheath), world, state, now);
        } else if seen.emotes != last.emotes {
            // **The one hop the protocol allows.** `SMSG_EMOTE` carries an
            // `Emotes.dbc` id and that row's third column is the animation, so
            // this is the only place in the client where the server comes close
            // to naming a pose. An emote with no animation, or a chain with no
            // `Emotes.dbc`, resolves to nothing and the entity carries on.
            if let Some(id) = emote_anim(world.last_emote) {
                self.fire(id, world, state, now);
            }
        } else if seen.spell_visuals != last.spell_visuals
            || seen.spell_impacts != last.spell_impacts
        {
            // **…and the *other* hop the protocol allows**, which is one hop
            // shorter: `SMSG_PLAY_SPELL_VISUAL` carries a `SpellVisualKit` id,
            // and that row's `animID` column is the pose outright.
            //
            // **This is what eating looks like.** vmangos sends kit 406 (food)
            // or 438 (drink) on every regeneration tick a character spends
            // sitting with either, and both read `animID 61` — `EmoteEat`. It
            // re-fires per tick rather than being held, which is the server's
            // own cadence: nothing on the wire says when eating *stops*, so a
            // held pose would have to invent its own end.
            //
            // Last in the chain, under the emote, because it is the least
            // specific statement of the five: a swing, a flinch, a release and
            // an emote each name what happened, and this names a row of art.
            // The **visual** wins over the **impact** in the same poll for the
            // same reason a swing wins over a flinch — it is what the unit did
            // rather than what was done to it.
            let kit = match seen.spell_visuals != last.spell_visuals {
                true => world.last_spell_visual,
                false => world.last_spell_impact,
            };
            if let Some(id) = kit_anim(kit) {
                self.fire(id, world, state, now);
            }
        }

        // **The parked swing, once nothing is live** — the consumer half of
        // [`Self::fast_path`], as the client drains it at the base
        // recompute. A poll that fired anything has already cleared the cache
        // through `fire`; one that took the fast path leaves the clip it sped up
        // still running, so this waits for it.
        if let Some(parked) = self.deferred.filter(|_| !self.oneshot_live(now)) {
            self.deferred = None;
            self.fire(parked, world, state, now);
        }
    }

    /// The take-off and the landing, which are the two **edges** of being off
    /// the ground rather than events the server counts.
    ///
    /// The arc's middle is a state and is drawn by [`wanted_animation`] —
    /// `Jump` for an arc somebody pushed off into, `Fall` for a step off a
    /// ledge. What is missing from a state is its ends: 1.12 authors
    /// `JumpStart` (37) as the crouch-and-launch, `JumpEnd` (39) as the
    /// absorb-and-straighten, and `JumpLandRun` (187) as the version of the
    /// second for a character who is still running when they arrive — measured
    /// on `HumanMale`, 6.9 y/s of travel in `JumpLandRun` against none at all
    /// in `JumpEnd`, which is the whole difference between the two.
    ///
    /// **The take-off is only for a jump, never for a fall.** A character who
    /// walks off a cliff pushed off nothing, and `MSG_MOVE_JUMP`'s own
    /// `zspeed` is what says which this was (see `WorldEntity::jumping`). The
    /// landing has no such condition — everything that comes down lands.
    ///
    /// **…and neither end belongs to a rider.** The caller decides that, because
    /// the *mount's* own [`Playback`] reaches this through
    /// [`Self::note_flight_only`] with the same (mounted) `WorldEntity` — see
    /// [`Self::note_actions`], where the condition and its argument are.
    ///
    /// Rides the same first-look guard the counters do, because it is called
    /// from inside it: an entity that streams into view already in mid-air must
    /// not be handed a take-off it did not make, and one that streams in on the
    /// ground must not land.
    ///
    /// All three are outside [`class_a`], so all three are full-body whatever
    /// the state — which is right: the crouch and the absorb are the legs.
    fn note_flight(
        &mut self,
        seen: Counters,
        last: Counters,
        world: &WorldEntity,
        state: u16,
        now: f32,
    ) {
        if seen.airborne == last.airborne {
            return;
        }
        if seen.airborne {
            // Only a jump has a start. `state` is [`anim::JUMP`] exactly when
            // the mover says something pushed off.
            if state == anim::JUMP {
                self.fire(anim::JUMP_START, world, state, now);
            }
            return;
        }
        // Down again. Which landing depends on which keys are held — see
        // [`landing_for`] — and **a landing the model cannot play, or one the
        // rule says not to play, still ends the take-off.** The reference
        // reaches the gait either way (directly, or through the
        // fallback column), and leaving the one-shot to run out on its own is
        // the mount hanging in its launch pose on the ground.
        match landing_for(world.move_flags) {
            Some(landing) if self.resolve(landing).is_some() => {
                self.fire(landing, world, state, now);
            }
            Some(landing) => {
                // The request, for the sheath reconcile, as [`Self::fire`]
                // records it — then the gait.
                self.played = Some(landing);
                self.oneshot = None;
            }
            None => self.oneshot = None,
        }
    }

    /// **The two ends of the air arc and nothing else** — the mount's whole
    /// share of [`Self::note_actions`].
    ///
    /// A mount has no counters of its own: nothing on the wire ever describes
    /// one, so it has no swings, no casts, no emotes and no death, and
    /// [`super::mount::gait`] is deliberately a smaller set than a unit's for
    /// exactly that reason. What it *does* have is a jump, because the rider's
    /// is the mount's — and `Creature\Horse\Horse.m2` carries all four clips
    /// (`JumpStart` 37, `Jump` 38, `JumpEnd` 39, `Fall` 40) of which only the
    /// two states were ever reached.
    ///
    /// The rider fires neither (see [`Self::note_flight`]): `Mount` outranks the
    /// air on a rider, so the arc belongs to the animal underneath, ends
    /// included.
    ///
    /// **It shares [`Self::seen`] with `note_actions` and nothing calls both on
    /// the same `Playback`.** A mount's is written only here; a unit's only
    /// there. Calling both would make the first-look guard of one consume the
    /// other's.
    pub(super) fn note_flight_only(&mut self, world: &WorldEntity, state: u16, now: f32) {
        let seen = Counters::of(world);
        let Some(last) = self.seen.replace(seen) else {
            return;
        };
        self.note_flight(seen, last, world, state, now);
    }

    /// How long one of the model's sequences runs for, **on the wall clock** —
    /// which for a rate-scaled gait is not its authored length.
    ///
    /// By index rather than "the one playing", because there are two tracks now
    /// and a masked shot's length is not the base's.
    fn length_ms(&self, sequence: usize) -> Option<u32> {
        let seq = self.skeleton.sequences.get(sequence)?;
        let authored = seq.end.saturating_sub(seq.start) as f32;
        Some((authored / self.skeleton.playback_rate(sequence, self.speed)) as u32)
    }

    /// Milliseconds into a sequence after `elapsed` seconds of wall clock.
    ///
    /// **This is where the playback rate is applied**, and it is applied to the
    /// whole span rather than accumulated frame by frame — see
    /// [`M2Skeleton::playback_rate`] for what the rate is. The difference shows
    /// only when a unit's *speed* changes while one clip keeps playing: the
    /// phase jumps once, because the elapsed span is re-scaled behind it. A
    /// walk-to-run change swaps the sequence and restarts the clock anyway, so
    /// what is left is a haste effect landing mid-stride — one frame's
    /// discontinuity at a moment the game is already announcing, against a
    /// running total this would otherwise have to carry. **Stated as an
    /// approximation, not measured off the client**, which accumulates.
    pub(super) fn clock(&self, sequence: usize, elapsed: f32) -> u32 {
        self.clock_at(sequence, elapsed, 1.0)
    }

    /// **Where the fading clip is sampled** — its wrapped clock for a loop, and
    /// its last frame for a one-shot that ran out. See [`Fade::held`], which
    /// carries the measurement.
    ///
    /// Clamped to the window's last millisecond rather than passed through
    /// [`M2Skeleton::phase`], because the phase of an elapsed span equal to the
    /// length is zero — the frame the report was about.
    pub(super) fn fade_phase(&self, fade: &Fade, now: f32) -> u32 {
        match fade.held {
            Some(ended) => {
                let length = self
                    .skeleton
                    .sequences
                    .get(fade.sequence)
                    .map_or(1, |s| s.end.saturating_sub(s.start).max(1));
                self.clock(fade.sequence, ended - fade.since).min(length - 1)
            }
            None => self
                .skeleton
                .phase(fade.sequence, self.clock(fade.sequence, now - fade.since)),
        }
    }

    /// …times **this play's own** multiplier, which is 1 for everything but a
    /// combat clip the fast-path has re-timed. See [`Playback::fast_path`].
    fn clock_at(&self, sequence: usize, elapsed: f32, rate: f32) -> u32 {
        (elapsed * 1000.0 * rate * self.skeleton.playback_rate(sequence, self.speed)).max(0.0)
            as u32
    }

    /// The animation a play was started with since this was last asked, if any.
    ///
    /// **Taking rather than reading**, which is the whole contract: the sheath
    /// reconcile must fire once per play and never once per frame. See
    /// [`Playback::played`] and [`super::sheath::reconcile`].
    pub(super) fn take_played(&mut self) -> Option<u16> {
        self.played.take()
    }

    /// Where in its stride the base track is — `(animation id, fraction 0..1
    /// of the cycle)` while the legs are playing a travelling ground gait,
    /// `None` otherwise.
    ///
    /// For the footstep system (`crate::sound::footsteps`), which watches the
    /// fraction wrap past the two footfalls. The `move_speed` guard drops a
    /// model whose gait *resolved to an idle loop* (42 of 405 have no Run) —
    /// an idle loop ticking footsteps is worse than silent feet. The clock
    /// arithmetic is [`Playback::advance`]'s, read-only.
    pub(crate) fn stride(&self, now: f32) -> Option<(u16, f32)> {
        if !matches!(self.wanted, anim::WALK | anim::RUN | anim::WALK_BACKWARDS) {
            return None;
        }
        let seq = self.skeleton.sequences.get(self.sequence)?;
        if seq.move_speed <= 0.0 {
            return None;
        }
        let length = seq.end.saturating_sub(seq.start).max(1);
        let elapsed = self.clock_at(self.sequence, now - self.since, self.base_rate);
        let phase = self.skeleton.phase(self.sequence, elapsed);
        Some((self.wanted, phase as f32 / length as f32))
    }
}

