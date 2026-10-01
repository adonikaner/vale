//! Entity animation: which sequence an entity plays, and posing its skeleton.
//!
//! This is the largest module in this directory and the most self-contained.
//! [`animate`] and [`animate_attachment`] drive the skeletons.
//! [`wanted_animation`] and the functions near it decide which sequence the
//! server's state calls for. [`fallbacks`] is the table of what to play when a
//! model does not have that sequence.
//!
//! Most of the sequence choice is a game rule, not rendering.
//! `wanted_animation` needs no mesh, no material and no window, only a
//! `WorldEntity`, so it could live in `vale-assets` beside `dress`. It stays
//! here because `WorldEntity` is a component of this crate; moving the rule
//! requires moving that type first. That move has not been made.

use super::*;
use bevy::utils::Parallel;

/// Pose every animated entity and write its joints.
///
/// The placement is read from the entity's own `Transform`, not its
/// `GlobalTransform`, so that attached models stay on the body. An entity is
/// spawned at the root of the hierarchy, and `place_entities` writes this
/// frame's interpolated position to its `Transform` earlier in `Update`. Its
/// `GlobalTransform` is still last frame's, because propagation runs in
/// `PostUpdate`. A joint is a world matrix written here, so composing it
/// against the `GlobalTransform` would pose the body where the entity was one
/// frame ago. An attached model is written as a local `Transform` and composed
/// by the same `PostUpdate` propagation, so it lands where the entity is now.
/// The body and its attachments would then differ by one frame of travel:
/// 0.13 yards forward at a run, varying with the frame time. The pauldrons
/// and the helm would appear loosely attached to the body.
pub(crate) fn animate(
    time: Res<Time>,
    displays: Res<DisplayCache>,
    // `Option`, because the headless harnesses (`--audit`, the entity tests)
    // build worlds with no render plugins and no tuning resource. Absent
    // means "everything on", which is also the resource's default.
    tuning: Option<Res<crate::render::tuning::WorldTuning>>,
    facing: Res<crate::world::facing::BodyFacing>,
    // The terrain and the buildings, used only to find the slope of the
    // ground under a drawn unit. See [`crate::world::entities::conform`].
    ground: super::conform::Ground,
    // The world camera, selected by its marker, not the first `Camera3d`. A
    // rig outside this frustum is not posed, and a process holds several 3D
    // cameras: the unit-frame portraits, the paper doll, and any pictures a
    // host draws. A query yields cameras in the order their archetypes were
    // created, and that order changes: the fog switch removes `DistanceFog`
    // from the world camera, which moves it to a newer archetype than every
    // picture camera. When the first camera was taken, the rigs were culled
    // against a portrait's frustum: the body stayed where it was last posed,
    // and anything re-hung on it stayed at the root, until the process ended.
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
            // The smoothed ground stance, which tilts a horse to the slope it
            // stands on. See [`crate::world::entities::conform`].
            &mut super::conform::Stance,
            // The unit's mount, if it is riding. It is posed here rather than
            // in a separate pass because its pose is an input to the rider's:
            // the saddle sets where the character goes, and the two must be
            // composed in the same frame or the body rides a horse that is one
            // frame behind it.
            Option<&mut Mount>,
            // Where this frame's joint matrices go; see [`Posed`].
            &mut Posed,
        ),
        (Without<Joint>, Without<AttachedTo>),
    >,
    // The joints of what a rig carries: its mount, its attachments, its
    // effects. Not the rig's own joints, which are the next query and are
    // written in parallel. The `Bone` filter makes the two queries disjoint,
    // so both can take `GlobalTransform` mutably.
    mut joints: Query<&mut GlobalTransform, (With<Joint>, Without<Bone>)>,
    // The rig's own joints, each reading one row of its parent's [`Posed`].
    mut bones: Query<(&Bone, &ChildOf, &mut GlobalTransform), (With<Joint>, With<Bone>)>,
    mut attached: Query<&mut Transform, With<AttachedTo>>,
    // The animated colour of the few batches that have one, such as an
    // effect's fade. Written here rather than in a separate pass because this
    // system already has the clock and the sequence window, and because a rig
    // the camera cannot see is skipped for colour as it is for its pose.
    mut tags: Query<&mut bevy::mesh::MeshTag, With<EntityPart>>,
    // One write list per thread, kept across frames for its capacity.
    mut writes: Local<Parallel<RigWrites>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Animate);
    let now = time.elapsed_secs();
    // Global-sequence tracks run on wall-clock time rather than on the playing
    // animation: blinking eyes, a turning mill wheel.
    let now_ms = (now * 1000.0) as u32;
    let camera = camera.iter().next().map(|(placement, frustum)| (*placement, frustum.clone()));

    // Both parts of the emote system use one lookup; see
    // `vale_assets::tables::dbc::Emotes`, where the split is explained.
    let emote_animation = |id: u32| displays.tables()?.emote_animation(id);
    // A cast's two poses use another lookup, three tables deep. Without the
    // spell tables it returns an empty pair, which puts every cast on the
    // generic wind-up and release.
    let cast_animation = |id: u32| {
        displays
            .tables()
            .and_then(|tables| tables.cast_animation(id))
            .unwrap_or_default()
    };
    // The shortest lookup of the three: a `SpellVisualKit` id directly to the
    // pose that kit holds, for the two packets that carry a kit and no spell.
    // See `vale_protocol::play::sound`.
    let kit_animation = |kit: u32| displays.tables().and_then(|tables| tables.kit_pose(kit));

    // Read once for the whole pass rather than per entity: it is two `Arc`
    // clones and a map id. It is `None` on every frame before a world is
    // loaded; the login screen's plinth stands on no ground.
    let ground = ground.of();
    let dt = time.delta_secs();
    let simulate = tuning.as_deref().is_none_or(|t| t.entities);
    // The pool the parallel pass below runs on. Initialised here as well as by
    // `TaskPoolPlugin`, because the headless harnesses build no such plugin
    // and `par_iter_mut` asks for the pool by name.
    bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::default);

    // Every rig is posed in parallel, and nothing is written until all of
    // them are posed. A pose depends only on the entity's own components and
    // on read-only state (the clock, the camera, the tables, the ground), so
    // the entities are independent of one another. This pass is the per-frame
    // CPU cost that grows with the number of units: it took 1.5 ms of a frame
    // standing alone in Stratholme. A pose produces writes to other entities
    // (its joints, its tints, its mount's and its attachments'), and a `Query`
    // cannot be written from inside a parallel iteration. So each thread
    // collects its writes into its own [`RigWrites`], and the pass applies
    // them serially afterwards. The apply is one `get_mut` per joint; the
    // sampling, the matrix composition and the attachment re-hangs all happen
    // in the parallel half.
    let shared: &Parallel<RigWrites> = &writes;
    entities.par_iter_mut().for_each(
        |(world, sheath, placement, model, mut play, mut stance, mut mount, mut posed)| {
            // Resolved before the state is chosen, because a held emote is the
            // state. Skipped for zero, which the field almost always holds.
            let held = (world.emote_state != 0)
                .then(|| emote_animation(world.emote_state))
                .flatten();
            let sheath = sheath.state();
            // The pose an aura holds this unit in. It is recomputed every frame
            // because it is a condition, not an event: no packet announces a
            // stun starting; the aura slot is occupied from one update block
            // to the next. It costs nothing for most units, which carry no
            // aura, and a few hash lookups for the rest.
            //
            // The first slot with a pose is used, in the server's slot order.
            // Two stuns at once are the same pose, and none of the tables ranks
            // one aura above another.
            play.aura = displays.tables().and_then(|tables| {
                world.auras.iter().find_map(|aura| tables.aura_pose(aura.spell))
            });
            // A stun that has no pose to hold freezes the unit instead; see
            // [`Playback::frozen`] for what this draws and which part of it is
            // measured. A corpse is excluded because the aura stays on the
            // unit in the update fields for one tick after death, the same
            // problem `state_or_held` describes.
            play.freeze =
                play.aura.is_none() && world.stunned() && !world.dead && !world.feigning;
            let state = wanted_animation(world, sheath, held);
            // The lookups are passed as closures rather than values because
            // each is needed only when its own counter changed, which happens
            // a few times a minute in a city and never in the wild.
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
            // Recorded for the sound cues before either gate below, so a rig
            // the camera cannot see still plays its sounds.
            play.note_window(elapsed);
            // The entities switch skips the pose as well as the draw. With the
            // layer off, `tuning::switch` hides the meshes, but the pose
            // sampling, the joint writes and the attachment re-hangs below are
            // the per-entity CPU cost the switch exists to measure. They were
            // the cause of "frame rate dips with players nearby even with
            // entities hidden". The clocks above have already advanced, so
            // turning the layer back on resumes mid-stride, as with the
            // frustum test below.
            if !simulate {
                return;
            }
            // A rig the camera cannot see is not posed. The pose is most of
            // this system's cost: sampling three tracks per bone, then a
            // `GlobalTransform` per joint, which also makes every skin upload
            // every frame. A city keeps most of its units off screen (the Trade
            // District view draws about 10 of 63 animated units). The clocks
            // above have already advanced, so a rig entering the frustum
            // resumes mid-stride rather than restarting. It is posed in the
            // same frame the test passes, so no frame of bind pose is drawn.
            // The sphere is the model's declared box
            // ([`EntityModel::cull_radius`]) at the placement's scale, the same
            // volume the part meshes are culled by, so a drawable part never
            // uses a stale pose. The far plane is not tested, which errs
            // toward posing.
            if let Some((_, frustum)) = &camera {
                // The larger of the two rigs, because the mount is drawn from
                // this pose too and a horse is larger than the gnome on it.
                // Each radius is in its own model's yards, so the mount's is
                // scaled by the ratio [`Mount::scale`] is composed at, which
                // gives the world radius its own parts are culled by.
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

    // The rig's own joints copy their pose from the rig, in parallel. Each
    // carries its bone index ([`Bone`]) and is a child of the entity, so the
    // write is one lookup in the parent's [`Posed`], filled by the pass above,
    // and a copy. The joints of what a rig carries (its mount, its
    // attachments, its effects) have no such index. They are in the
    // [`RigWrites`] lists, applied serially below, and they are a small
    // fraction of the joints in view.
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
/// Only the joints `spawn_models` makes for an entity's own skeleton carry
/// this. A mount's, an attachment's or an effect's joints hang under a
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

/// The joints, the tints and the attachment roots one rig's pose produced,
/// collected on the thread that posed it and written once every rig is posed.
///
/// A `Query` cannot be written from inside `par_iter_mut`, and the joints of
/// one rig are other entities. So the parallel half of [`animate`] fills one
/// of these per thread, and [`Self::apply`] is the serial half: one `get_mut`
/// per entry. A tint is compared at apply time, because only there can the
/// current byte be read. A `DerefMut` on `MeshTag` causes an instance
/// re-upload, and an effect that has finished fading keeps one byte for as
/// long as it is worn.
///
/// The two callers outside the pass, the glue screen's character and a
/// missile, have their queries available and apply immediately.
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
    /// An attachment's root, as a local `Transform` in its wearer's space.
    /// See [`animate`] for why it is local while a joint is a world matrix.
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

    /// The attachment roots collected and not yet applied, for a test of the
    /// frames [`animate_attachment`] writes.
    #[cfg(test)]
    pub(crate) fn pending_roots(&self) -> &[(Entity, Transform)] {
        &self.roots
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

    /// [`Self::apply`], plus the attachment roots. [`animate`] uses this form.
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
/// The body of [`animate`]'s loop. It is a separate function so that the
/// parallel iteration's closure holds the tests and this holds the work.
/// Everything it reads is either the entity's own or shared read-only;
/// everything it writes goes into `writes`.
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
    // Billboarded bones face the camera, so the camera's axes must be
    // expressed in the model's own space. For an entity that is the world
    // turned by minus its facing, since facing is the only rotation a
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
                // Wrapped for a loop, and held at its last frame for a
                // one-shot that ran out. The outgoing loop keeps running
                // while it fades, so a creature that has been standing for a
                // minute fades out of the phase its idle has reached. See
                // [`Playback::fade_phase`] and [`Fade::held`].
                sequence: fade.sequence,
                elapsed_ms: play.fade_phase(fade, now),
                weight: 1.0 - (now - fade.from) / FADE_SECS,
            }),
            // The upper-body correction for a strafe. The placement above
            // has already turned the whole model toward the direction of
            // travel; this turns the spine and the head back, so the
            // character keeps looking where it is aiming while its hips and
            // legs follow the line of travel. It is zero, and costs nothing,
            // for everything that faces where it is going. See
            // `crate::world::facing`.
            twist: facing.of(world.guid).map(|body| BodyTwist { gap: body.gap }),
            // The upper body's own sequence over the lower body's. This is
            // how the game swings, emotes and casts while moving: the torso
            // plays its own sequence on the `SpineLow` subtree while the
            // legs keep the gait. `None` for a standing character, whose
            // one-shot uses the base track instead; see [`route_oneshot`].
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

    // The mount, and the one affine it changes. The mount is posed first,
    // because where it puts the rider depends on its own pose. The result is
    // a translation in the rider's local frame, which is composed into
    // `world_from_entity` below and so into the rider's joints, its attached
    // models, its glows and its ground decals together. It is the identity
    // for everything that is not riding, which is nearly everything.
    // The slope the unit stands on also goes into this affine, and the mount
    // decides whether to lean when there is one: `HumanMale.m2` never leans
    // and `Horse.m2` always does, so a mounted character tilts because its
    // saddle does. It is the identity for everything that does not lean,
    // which is every character model and all of the scenery. See
    // `super::conform`.
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
        // With no mount, the unit's own conform is the seat. It is one affine
        // either way, so the composition below and the attachment loop,
        // which already uses `seat`, do not need to know which case applies.
        None => conform,
    };

    // The joint matrices replace the mesh's world matrix in Bevy's skinning
    // shader, so each one is the entity's placement times the bone's own
    // pose, not the pose alone. They go into the rig's own [`Posed`], in
    // bone order, for its [`Bone`]s to read. The identity joint, which the
    // unweighted vertices follow, comes last. It is the model's own space,
    // unposed, and so it is `world_from_entity` rather than the placement,
    // so that a rider's unweighted geometry is lifted onto the saddle with
    // the rest of it.
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

    // The attached models follow the same pose. An attachment point is a
    // position in its bone's frame, so the placement is `bone * T(offset)`.
    // The root is written as a `Transform` in the wearer's local space
    // rather than as a world matrix, because its parts are drawn and Bevy
    // composes a drawn entity's parent transform itself.
    for one in model
        .attached
        .iter()
        .chain(&model.cast.parts)
        .chain(&model.impact.parts)
        .chain(&model.state.parts)
        .chain(&model.milestone.parts)
        .chain(&model.loot.parts)
        // The two sets a packet or a host fills. When they were missing from
        // this chain, a model hung by `SMSG_PLAY_SPELL_VISUAL` stayed at its
        // root's initial transform (the wearer's origin, at scale) instead
        // of following its bone. Every set with parts is posed here.
        .chain(&model.pushed.parts)
        .chain(&model.hung.parts)
    {
        let Some(local) = one.local(&pose) else {
            continue;
        };
        // The seat must be applied here too. An attachment's root is a child
        // of the entity, so Bevy composes it against the entity's own
        // `Transform`, which is still on the ground under the horse. The
        // joints above are world matrices and include the seat; these are
        // local and do not, so without this a mounted character would sit on
        // the saddle with its pauldrons and its helm left on the ground.
        let local = Mat4::from(seat) * local;
        // A grounded effect does not turn with the unit standing on it. The
        // root is a child of the wearer, so Bevy composes the wearer's whole
        // transform onto it, including the facing. The roots effect under a
        // rooted player then turned with the player as it turned on the spot,
        // so the effect looked painted on the character rather than on the
        // ground.
        //
        // Multiplying by the inverse rotation on the left cancels it: the
        // wearer is `T·R·S`, so `T·R·S · R⁻¹·local` is `T·S·local` while the
        // scale is uniform, which every unit's is (`Vec3::splat`). A
        // non-uniform scale would shear instead, and the error would be hard
        // to spot.
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

/// Pose one attached model on its own clock, and fade it on its own tracks.
///
/// `world_from_attach` is the frame the attachment sits in, already composed:
/// the wearer's placement times the carrying bone times the offset for a
/// pauldron or a spell glow, and the projectile's own placement for a missile,
/// which is attached to nothing. That frame is all this needs from the caller,
/// so it is a free function: a missile is an attached model with no wearer,
/// and a separate copy for missiles would duplicate the billboard derivation.
///
/// The camera basis is derived again by inverting that frame, rather than by
/// undoing the wearer's facing. Undoing the facing does not work through the
/// second level, because the carrying bone's rotation lies between the two,
/// and a torch glow is a quad on a bone flagged 0x8 in the torch's own
/// skeleton.
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
    // frame. For a one-shot like Arcane Explosion's dome that is the
    // collapsed frame, not the fully grown one.
    let elapsed = match (one.loops, &one.skeleton) {
        (true, Some(skeleton)) => skeleton.phase(one.sequence, raw),
        _ => raw,
    };

    // The fade, which is what ends the effect visually. The geometry animates
    // and the emitters run; `M2Color` and the transparency block make it
    // disappear. Arcane Explosion's dome runs 0 -> 0.91 -> 0 over its own
    // 767 ms. This comes before the skeleton test below, because an effect
    // can be a rigid quad that only fades.
    if let Some(tints) = &one.tints {
        let window = one
            .skeleton
            .as_ref()
            .and_then(|s| s.sequences.get(one.sequence));
        for tinted in &one.tinted {
            // Written only on a change, which [`RigWrites::apply`] decides,
            // to avoid the same instance re-upload the wearer's own tint loop
            // avoids. An effect that has finished fading holds its last byte
            // for as long as it is worn.
            writes.tag(
                tinted.part,
                crate::render::models::tint_tag(
                    tints.sample_in(tinted.tint, window, elapsed, now_ms),
                ),
            );
        }
    }

    // The attachment's own pose. Skipped for most attachments, which are
    // rigid (no skeleton), at the cost of one branch.
    let attach_pose = match (&one.skeleton, one.joints.is_empty()) {
        (Some(skeleton), false) => {
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
            // The identity joint, as on the wearer: the attachment's own
            // space, unposed.
            if let Some(&last) = one.joints.last() {
                writes.joint(last, world_from_attach);
            }
            Some(attach_pose)
        }
        _ => None,
    };

    // The models hung on this model's own points, such as an item visual's
    // glows on a weapon. Each rides this model's bone as this model rides the
    // wearer's, and its root is a child of this one, so it is written in this
    // model's space. A model with no skeleton is in its bind pose, where a
    // point is already in model space.
    for nested in &one.nested {
        let local = match &attach_pose {
            Some(pose) => nested.local(pose),
            None => Some(nested.unposed()),
        };
        let Some(local) = local else {
            continue;
        };
        writes.root(nested.root(), Transform::from_matrix(local));
        animate_attachment(
            nested,
            world_from_attach * Affine3A::from_mat4(local),
            camera,
            now,
            now_ms,
            writes,
        );
    }
}

/// Which animation an entity should be playing, from the state the snapshot
/// carries.
///
/// Every input here is state the server sends, and none of it is an
/// animation id; the protocol has none. Death is health reaching zero, a swing
/// is one `SMSG_ATTACKERSTATEUPDATE`, a combat stance is
/// `UNIT_FLAG_IN_COMBAT`, and sitting is a byte of `UNIT_FIELD_BYTES_1`.
/// Turning those into a pose is the client's job alone, so a wrong choice here
/// produces no error anywhere.
///
/// The order of the tests is a priority, and it follows the game's rules: a
/// corpse does not flinch, and a creature running at the player does not stand
/// in its ready pose.
///
/// `sheath` is the unit's committed sheath state ([`super::Sheath`]). Here it
/// decides only which ready stance a unit in combat holds. It is an argument
/// rather than a field of `WorldEntity` because the client decides it, not the
/// server; see [`super::sheath`].
pub(super) fn wanted_animation(world: &WorldEntity, sheath: u8, emote_state: Option<u16>) -> u16 {
    // A game object follows different rules from a unit. It does not die,
    // fight, sit or walk; it stands open or shut, and `GAMEOBJECT_STATE` is
    // the one field that says which. Its model has no `Stand` either
    // (`Chest02.m2` carries 146..149 and nothing else), so falling through to
    // the unit rules below resolved to no sequence and left every chest and
    // door drawn in its bind pose. This test comes first because none of the
    // rules below apply to a game object.
    //
    // The test is on the object's kind, not on whether the field arrived.
    // Testing the field caused the "some doors show both states at once"
    // report. `Object::_SetCreateBits` omits a field whose value is zero, so a
    // game object standing open (`GO_STATE_ACTIVE` is 0) sends no state, and
    // testing for `Some` sent exactly those objects to the unit rules, where a
    // door matches nothing and is drawn in its bind pose. `DEADMINEDOOR01.m2`
    // has 28 bones whose bind pose is neither state and extends 3.06 yards
    // past the model's own box, so an unposed door shows both leaves splayed,
    // open and shut at once. See
    // `vale_assets::world::collision::game_object_is_solid`, which applies the
    // same rule to the same field and treats its absence the same way.
    if world.kind == ObjectType::GameObject {
        return if world.object_state.unwrap_or(0) == GO_STATE_READY {
            anim::CLOSED
        } else {
            anim::OPENED
        };
    }
    // A corpse holds the last frame of Death. See `anim::DEAD`, which is in
    // the DBC and in none of the 411 models, and `Playback::advance`, which
    // stops the clock rather than looping.
    //
    // A feigning body is drawn as a corpse too, as in the 1.12.1 client: the
    // client treats a unit as dead for display when its health is zero, or
    // `UNIT_DYNFLAG_DEAD` is set, or the object is a corpse. So Feign Death
    // needs no clip, no aura pose and no state of its own. It is the death
    // clip, held on its last frame by the same rule, and when the flag clears
    // the body fades back into whatever it was doing.
    //
    // Nothing else in this client reads [`WorldEntity::feigning`]: the health
    // did not change, so the target frame, the release box and the corpse
    // marker all still show the unit as alive, as they do in 1.12.
    if world.dead || world.feigning {
        return anim::DEATH;
    }
    // A rider plays one sequence and the mount plays the rest. `Mount` (91) is
    // a held pose, not a gait: the character sits still while the horse walks,
    // runs, swims and jumps under it. So this takes precedence over the water
    // and the air as well as the gait, each of which decides the sequence for
    // an unmounted unit. `AnimationData.dbc` agrees: the row's policy column
    // is `STOW_HANDS_BUSY`, so the weapons are put away for the whole ride.
    //
    // It also takes precedence over the emote and the stand state, both of
    // which the server can report on a unit that is still mounted. It comes
    // after death, which dismounts.
    if world.mounted {
        return anim::MOUNT;
    }
    // Swimming takes precedence over the gait, and it is a separate set of
    // animations rather than a variant of one. A character in deep water plays
    // Swim while moving and SwimIdle while not; a swimmer who stops does not
    // stand upright in mid-water.
    //
    // The water has its own order of precedence, which is why this reads flags
    // rather than a direction. The 1.12.1 client orders it turn > strafe >
    // backward > forward, so a turning swimmer treads water whatever else is
    // held, and a diagonal strafe plays the sideways stroke where the same
    // keys on the ground run forward.
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
    // Flying takes precedence over both, on the same terms: the 1.12.1 client
    // tests the spline's `Flying` flag and plays 135 before any of the ground
    // tests below. It is a spline flag rather than a unit state, so it arrives
    // here as a movement flag the ride sets; see
    // [`vale_protocol::state::movement::Mover::ride`] and
    // `Entity::move_flags`, which set and carry it.
    //
    // Not gated on `moving`, unlike the ground gait: a taxi flight is driven
    // by a spline, and a unit on a spline never hovers still.
    if world.move_flags & move_flags::FLYING != 0 {
        return anim::FLY;
    }
    // Being in the air takes precedence over the gait, for the same reason as
    // swimming: it is a different pose, not a variant of running, and a
    // character crossing a gap in its run cycle looks as if it runs on air.
    // Whether the unit pushed off chooses between the two arcs:
    // `MSG_MOVE_JUMP` carries a non-zero `zspeed` (negative, because the
    // packet's axis is positive downward), and a step off a ledge carries
    // zero.
    if world.airborne {
        return if world.jumping { anim::JUMP } else { anim::FALL };
    }
    if world.moving && world.speed >= MOVING_FLOOR {
        // Moving backward is one sequence at any speed. The packet carries a
        // separate `run_back` speed, but the models carry only
        // `Walkbackwards`, so there is nothing to choose between; see
        // [`anim::WALK_BACKWARDS`]. On the ground, backward takes precedence
        // over a strafe, the opposite of the water's order above.
        if world.move_flags & move_flags::BACKWARD != 0 {
            return anim::WALK_BACKWARDS;
        }
        // A stealthed unit creeps at any speed, and this takes precedence
        // over the Sprint/Run/Walk choice. The 1.12.1 client checks
        // `UNIT_FIELD_BYTES_1`'s vis byte for `UNIT_VIS_FLAGS_CREEP` after the
        // backward test and before the speed tests, and plays 119 whatever
        // the unit's speed. `AnimationData.dbc` has no `StealthRun`.
        //
        // The client tests backward movement first, so a stealthed character
        // backing away plays `Walkbackwards`.
        if world.creeping() {
            return anim::STEALTH_WALK;
        }
        // The full-speed run, chosen by speed and not by spell. Charge,
        // Sprint and any other haste past 11.0 y/s reach it the same way; see
        // [`anim::SPRINT`], which has the order of tests and the constant.
        // It comes before the Run/Walk choice for the same reason `creeping`
        // does: the client tests it separately, not as a third case of the
        // speed comparison below.
        if world.speed >= anim::SPRINT_SPEED {
            return anim::SPRINT;
        }
        // A strafe plays the forward gait. There is no sideways gait on the
        // ground: `RunLeft`/`RunRight` are rows in `AnimationData.dbc` that
        // 0 of 411 models carry, and the Shuffles below are the turn-in-place
        // foot shuffle. A strafe looks like a strafe because the drawn body is
        // turned toward the direction of travel while the legs run; see
        // `movement::strafe_body_offset` and `crate::world::facing`, which
        // implement the strafe.
        return if world.speed >= RUN_SPEED { anim::RUN } else { anim::WALK };
    }
    // A stealthed unit that has stopped holds the crouch. The 1.12.1 client's
    // idle choice tests for creeping in the same place as this function:
    // after the water, before every other idle pose.
    //
    // It comes before the turn-in-place shuffles. The client's idle order is
    // `swimming -> creep -> hover -> Stand` and never plays a shuffle, so the
    // only choice here is which side of the shuffles it goes on, and a
    // stealthed character turning on the spot is still stealthed.
    if world.creeping() {
        return anim::STEALTH_STAND;
    }
    // Turning on the spot plays the foot shuffle. It is reached only when not
    // moving, since a turn while travelling curves the path and keeps the
    // gait. It follows the body's actual rotation rather than the turn keys,
    // so a mouse turn shuffles too; `crate::world::facing` sets the flags.
    if world.move_flags & move_flags::TURN_LEFT != 0 {
        return anim::SHUFFLE_LEFT;
    }
    if world.move_flags & move_flags::TURN_RIGHT != 0 {
        return anim::SHUFFLE_RIGHT;
    }
    // Crouched over a body while looting. This is the only pose in this
    // function that no packet states: there is no loot row in `Emotes.dbc`
    // and no unit field for it, so it is played only while this client has
    // the loot window open. See [`WorldEntity::looting`], which is only ever
    // true for the local player, and `anim::LOOT`, where the clip is measured.
    //
    // Its position in the order is deliberate on both sides. It comes after
    // locomotion, the water and the air, because none of those can apply: any
    // movement packet carrying `MOVEFLAG_MASK_MOVING` makes vmangos release
    // the body on the server, so a looting character is standing still. If
    // the window stays open past a step, standing up looks better than
    // reaching into the ground while running. It comes before the stand state
    // and the held emote, because both are poses the server asked for earlier
    // and looting is what the player is doing now: an innkeeper's stool does
    // not take precedence over the corpse in front of him.
    if world.looting {
        return anim::LOOT;
    }
    // Stationary. The server uses the stand state to seat an innkeeper on a
    // stool or lay a guard down at his post; drawn standing, they would
    // appear to float through the furniture.
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
    // A held emote, the part of the emote system that is a field rather than
    // a packet. `UNIT_NPC_EMOTESTATE` is an `Emotes.dbc` id the unit keeps: a
    // dancing player, an innkeeper permanently at work, a guard leaning on a
    // rail. It takes precedence over the combat stance because the server is
    // asking for a specific pose and would clear the field to get the default
    // one. Locomotion takes precedence over it for the same reason as over
    // the stand state.
    if let Some(emote) = emote_state {
        return emote;
    }
    // Attacking, not merely in combat. The 1.12.1 client shows the
    // weapon-class Ready idle while the unit's auto-attack target guid is set
    // (from `SMSG_ATTACKSTART` until `SMSG_ATTACKSTOP`), not while
    // `UNIT_FLAG_IN_COMBAT` is set and not by the sheath state. Of those three
    // only the target guid means the unit is swinging at something: a mage
    // being hit carries the combat flag for the whole fight and never raises
    // a weapon, and a player who has drawn aggro across a room is in combat
    // before anything is within reach.
    if world.attacking {
        // The ready stance is where the drawn weapon shows between blows, and
        // a two-hander held in the unarmed guard looks like a character
        // carrying a plank. `WeaponAnim` chooses the family from the item's
        // class.
        return drawn_weapon(world, sheath).ready();
    }
    anim::STAND
}

/// Where a one-shot plays: over the whole body, or over the legs.
///
/// See [`route_oneshot`], which is where the choice is made and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Route {
    /// On the base track, bone 0: the clip replaces the whole pose, legs
    /// included. A standing character plays one-shots this way.
    FullBody,
    /// On the masked overlay rooted at `SpineLow`: the torso plays the clip and
    /// the legs keep the gait underneath it.
    Masked,
}

/// Whether a one-shot plays over the legs or replaces them. It is decided for
/// each play from the unit's current state, never by the animation id alone.
///
/// This is how 1.12 casts a spell at a run, waves while walking or swings
/// mid-jump, and it cannot be inferred from the ids: the same `Attack1H` is
/// full-body from a standing character and masked from a running one. It
/// follows the 1.12.1 client's rule.
///
/// The lower body is committed, and the play is therefore masked, when any
/// direction bit is set (including the two keyboard turn keys, unlike the
/// cast-cancel mask in [`Playback::state_or_held`]), or the unit is swimming,
/// or it is in a stand state other than 0, or a combat id is played while
/// airborne. Otherwise the unit is standing idle and the clip takes the whole
/// body.
///
/// The id decides only which tests apply: [`class_a`] decides whether an id
/// can be masked at all, [`combat_id`] admits it to the airborne test, and
/// [`forced_full_body`] excludes two families whatever the state.
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

/// Movement flags `& 0x20003f`: every direction bit, the two turn keys
/// included, plus swimming.
///
/// This is not the mask that cancels a cast, which is the direction bits and
/// swimming only. The client uses the two masks for different decisions, and
/// using this one to cancel a cast would cancel it every time the mouse
/// turned the character. See [`Playback::state_or_held`] for the other mask.
const COMMITTED_LOWER: u32 = move_flags::FORWARD
    | move_flags::BACKWARD
    | move_flags::STRAFE_LEFT
    | move_flags::STRAFE_RIGHT
    | move_flags::TURN_LEFT
    | move_flags::TURN_RIGHT
    | move_flags::SWIMMING;

/// The ids that may be masked: an id outside this set is always full-body.
///
/// The memberships this client depends on are exact. The irregular interior
/// of the wide ranges is inferred, and no result here depends on it, because
/// everything this client routes through the set is a swing, a flinch, a cast
/// release or an emote, and all of those lie well inside a range. At the
/// edges, what matters is what the set excludes: the two families that must
/// never be masked, the jump band 37..45 and a game object's 146..149.
fn class_a(id: u16) -> bool {
    matches!(id,
        2 | 8..=10 | 14..=36 | 46..=49 | 51..=90 | 105..=113 | 117..=118 | 122..=138
            | 185..=186 | 195)
}

/// Which landing animation a character plays when it lands. This follows the
/// 1.12.1 client's rule, which reads the movement flags rather than the
/// resolved gait.
///
/// [`anim::JUMP_LAND_RUN`] carries 6.9 y/s of travel on `HumanMale` and
/// [`anim::JUMP_END`] carries none, so the choice decides whether the body
/// keeps running or stops dead while the character still slides forward. The
/// client decides it with four tests, in this order:
///
/// ```text
/// no direction held (flags & 0xf == 0)  -> JumpEnd (39)
/// MOVEFLAG_BACKWARD                     -> no clip
/// MOVEFLAG_WALK_MODE                    -> no clip
/// otherwise                             -> JumpLandRun (187)
/// ```
///
/// "No clip" is a deliberate result. When walking or moving backward, the
/// client plays no landing and returns to the gait, and `None` here makes
/// [`Playback::note_flight`] do the same. A `Some` that the model cannot play
/// has the same effect, because the client resolves it through
/// `AnimationData.dbc`'s fallback column, which reads 5 (`Run`) for
/// `JumpLandRun` and 0 (`Stand`) for `JumpEnd`: on a model with no landing
/// clip, the landing is the gait.
///
/// That last case covers every mount. `Creature\Horse\Horse.m2`, `Ram`,
/// `Wolf`, `Tiger`, `MechaStrider` and `UndeadHorse` each carry `JumpStart`,
/// `Jump`, `JumpEnd` and `Fall`, and none of them carries 187 (`vale anim
/// <model>` lists what each has). An earlier version of this rule chose the
/// clip from the gait and played it whether or not the model had it, and
/// playing a clip that resolves to nothing changes nothing. So a mount that
/// landed running kept playing its take-off clip on the ground for the rest
/// of the 833 ms, which caused the report "the mount floats briefly on a
/// quick landing".
///
/// The three strafe cases travel and take the running landing. A shuffle is a
/// turn on the spot with no direction bit, so it takes the standing landing.
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

/// The combat ids: the only ids that are masked when airborne. So a swing
/// made mid-jump plays over the jump arc, and an emote in mid-air does not.
fn combat_id(id: u16) -> bool {
    matches!(id, 10 | 16..=24 | 30 | 36 | 57..=59 | 85..=88 | 95 | 117 | 118)
}

/// The two families always played on the base track, whatever the state: the
/// death animations and the sit transitions. A dying unit falls with its whole
/// body, not from the waist up.
fn forced_full_body(id: u16) -> bool {
    matches!(id, 1 | 6 | 131 | 132 | 57 | 58 | 118)
}

/// Which family of attack and parry animations this unit's drawn weapon
/// belongs to.
///
/// Unarmed when no weapon is drawn: a sheathed sword is on the wearer's back,
/// and the swing that goes with it is the fist. Which slot counts depends on
/// the sheath state, as the attached models do; see
/// `vale_assets::look::dress`, where the same three cases decide where each
/// weapon hangs.
///
/// `sheath` is the client's committed state, not `WorldEntity::sheath_state`;
/// see [`super::Sheath`]. Reading the update field here made the local player
/// punch with a sword on his back: the field echoes a packet this client had
/// not sent.
pub(super) fn drawn_weapon(world: &WorldEntity, sheath: u8) -> WeaponAnim {
    match sheath {
        SHEATH_STATE_MELEE => WeaponAnim::of(&world.weapons[0]),
        SHEATH_STATE_RANGED => WeaponAnim::of(&world.weapons[2]),
        _ => WeaponAnim::Unarmed,
    }
}

/// The animation a ranged weapon attack plays, from the third slot only.
///
/// This does not use [`drawn_weapon`], which reports what is in the hands. A
/// ranged spell is fired from the ranged slot whether or not the sheath state
/// has changed yet, and a shot pressed on the frame the bow is still being
/// drawn must not play a sword swing. The slot decides, and the sheath follows
/// from it (see `interface::action`, which requests `SHEATH_RANGED` on the
/// same press).
///
/// `None` for an empty slot and for a wand. `WeaponAnim::of` puts a wand in
/// `Unarmed` because the character models carry no wand sequence, so the
/// alternative would be a caster punching the air once a second for a whole
/// fight. What the 1.12.1 client plays for a wand is not established.
fn ranged_shot(world: &WorldEntity) -> Option<u16> {
    WeaponAnim::of(&world.weapons[2]).ranged_attack()
}

/// The off hand's animation family, chosen separately from the main hand's.
///
/// Every left-handed blow once played `AttackOff` (87), so a rogue with a
/// dagger in each hand played one animation twice. The client chooses the
/// off-hand swing from the off-hand item, and its grouping differs from the
/// main hand's: a dagger stabs (`AttackOffPierce`, 88), any other weapon
/// swings 87, and an empty hand, a shield or an off-hand tome punches
/// (`AttackUnarmedOff`, 117).
fn off_hand(world: &WorldEntity, sheath: u8) -> WeaponAnim {
    match sheath {
        SHEATH_STATE_MELEE => WeaponAnim::of(&world.weapons[1]),
        _ => WeaponAnim::Unarmed,
    }
}

/// Which swing this blow was, off the two `HitInfo` bits the server sets on it.
///
/// The weapon decides the family and the packet decides the variant.
/// [`drawn_weapon`] reports what is in the hands, from the wardrobe; this
/// reports which hand swung, from the swing's `SMSG_ATTACKERSTATEUPDATE`. This
/// client once parsed those bits and did not use them.
///
/// `HITINFO_CRITICALHIT` is not read here. This function once returned
/// `CombatCritical` (10) for a critical swing, based on the id's name. The
/// client uses that id for the victim instead: its wound reaction plays 10
/// `CombatCritical` for a critical and 9 `CombatWound` or 8 `StandWound`
/// otherwise. So 10 is chosen alongside `CombatWound` and `StandWound`, by the
/// same flag. It is the large flinch, not a large swing.
///
/// Playing it on the attacker was a visible bug: at a crit rate of about 20
/// percent, a fifth of every character's blows played a hit reaction instead
/// of a swing, reported as "my attack animation keeps being interrupted by a
/// hit reaction". See [`reaction`], which now plays the id.
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

/// Values of `UNIT_FIELD_BYTES_2` byte 0, from the one place that defines
/// them: the same constants `vale_assets::look::dress` uses to hang the models,
/// and that `vale_assets::look::sheath` decides.
pub(super) use vale_assets::look::sheath::{MELEE as SHEATH_STATE_MELEE, RANGED as SHEATH_STATE_RANGED};

/// Which reaction a `VictimState` calls for on the unit that received the blow.
///
/// Three of the nine states have their own pose, and the rest mean the blow
/// landed, which plays the flinch. A parry is made with whatever is in the
/// hands, so it goes through the weapon family. A dodge uses the whole body
/// and a block uses the shield, and neither depends on what is held.
///
/// A blow that landed has two flinches, chosen by the critical bit. The
/// 1.12.1 client's wound reaction has three results from one input:
/// 10 `CombatCritical` for a critical; otherwise 9 `CombatWound` in a combat
/// stance and 8 `StandWound` when not.
///
/// The choice between 9 and 8 depends on whether the unit is fighting, the
/// same condition the stance uses. Here the model's fallback chain makes that
/// choice (`COMBAT_WOUND` falls back to `STAND_WOUND`) rather than a second
/// test.
///
/// 10 is correct for two reasons. `AnimationData.dbc` names it for a
/// critical, and it is in the client's combat set (10, 16..24, 30, 36,
/// 57..59, 85..88, 95, 117..118). So a critical taken mid-swing is treated
/// as a combat animation: it speeds the swing up and holds it, instead of
/// cutting it off as an ordinary `CombatWound` does. The 1.12.1 client
/// behaves this way; it is not a choice made here.
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
/// low byte takes. 7 (`DEAD`) is not here: death is decided by health, which
/// the server always writes, while the stand state is written only when the
/// server wants a particular pose.
/// `GOState` (`GameObjectDefines.h`): the reset state, a door shut or a chest
/// with its lid down. 0 is `GO_STATE_ACTIVE` and 2 the alternative used state,
/// and both of those are open.
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
/// A model without the requested sequence plays the first sequence in the
/// chain that it has. This applies often: 42 of the game's 405 animated models
/// have no Run, and a critter has no attack at all. The last entry of every
/// chain is Stand, which every animated model has.
pub(super) fn fallbacks(wanted: u16) -> &'static [u16] {
    match wanted {
        anim::RUN => &[anim::RUN, anim::WALK, anim::STAND],
        anim::WALK => &[anim::WALK, anim::RUN, anim::STAND],
        // Moving backward. The chain comes from `AnimationData.dbc`'s
        // fallback column, which gives `Walkbackwards -> Walk -> Stand`. The
        // `Run` in the middle is added by this client, for the same reason as
        // in the two chains above: 42 of the game's 405 animated models have
        // no Walk, and one of those moving backward should move rather than
        // stand.
        anim::WALK_BACKWARDS => &[
            anim::WALK_BACKWARDS,
            anim::WALK,
            anim::RUN,
            anim::STAND,
        ],
        // The two stealth clips. Their chains come from `AnimationData.dbc`:
        // row 119's fallback column reads 4 (`Walk`) and row 120's reads 0
        // (`Stand`), so a creature with no crouch that the server has hidden
        // walks and stands rather than freezing. The `Run` after `Walk` is
        // added by this client, for the same reason as in the gait chains
        // above: 42 of the 405 animated models have no `Walk`, and one of
        // those creeping should still move.
        //
        // These chains make the creep rule safe to apply to every unit with
        // the creep bit, not only to characters. All sixteen character models
        // carry both clips and 0 of the 411 creature models carry either
        // (`vale anim`, whose population line prints all three numbers). So a
        // prowling cat, the most common creeping creature, takes the fallback
        // and keeps walking, which is what 5875 does.
        anim::STEALTH_WALK => &[
            anim::STEALTH_WALK,
            anim::WALK,
            anim::RUN,
            anim::STAND,
        ],
        anim::STEALTH_STAND => &[anim::STEALTH_STAND, anim::STAND],
        // The full-speed run. Its fallback column reads 5 (`Run`), so a model
        // with no `Sprint` runs. That is every one of the 411 creature models,
        // and so every mount. `Walk` and `Stand` after it are added by this
        // client, for the same reason the `Run` chain has them.
        anim::SPRINT => &[anim::SPRINT, anim::RUN, anim::WALK, anim::STAND],
        // The turn-in-place foot shuffle, carried by 104 of the 411 models:
        // the humanoids, which are also the only units a player watches turn
        // on the spot. It falls back to Stand only. The character is not
        // travelling, so a gait would make it run on the spot, which looks
        // worse than not shuffling.
        anim::SHUFFLE_LEFT => &[anim::SHUFFLE_LEFT, anim::STAND],
        anim::SHUFFLE_RIGHT => &[anim::SHUFFLE_RIGHT, anim::STAND],
        // In the air. `Jump` is the mid-flight loop and `Fall` is the loop for
        // an arc that did not start with a jump. Each falls back to the other,
        // because a model with only one of them looks better in the wrong
        // airborne pose than standing upright over a cliff.
        //
        // The ends of the arc (`JumpStart`, `JumpEnd` and `JumpLandRun`) are
        // one-shots played by [`Playback::note_flight`] and deliberately have
        // no chain, which is what the `_ => &[]` arm at the bottom means: they
        // are played if the model has them and dropped otherwise. That is the
        // only safe form. A one-shot is held for the length of the sequence it
        // resolved to, so substituting Stand for a missing `JumpEnd` would
        // freeze a landing wolf upright for the 2.6 seconds of its idle loop
        // while it ran away.
        anim::JUMP => &[anim::JUMP, anim::JUMP_START, anim::FALL, anim::STAND],
        anim::FALL => &[anim::FALL, anim::JUMP, anim::STAND],
        // The rider's pose. The chain comes from `AnimationData.dbc`: row 91's
        // fallback column reads 0, `Stand`, and no pose fits between the two.
        // A model with no `Mount` is not built to ride, and every seated pose
        // in the table is made for a chair rather than a saddle. All eighteen
        // character models carry it, so the fallback is for a creature the
        // server has mounted, such as a kodo rider's kodo, and standing upright
        // on the saddle is the least wrong result.
        anim::MOUNT => &[anim::MOUNT, anim::STAND],
        // The loot crouch, which falls back to standing only. All sixteen
        // character models carry it and only a player loots, so this chain
        // covers a case that does not occur. Every seated pose in the table is
        // made for a chair rather than for crouching over a body, so
        // substituting one would look worse than not crouching.
        anim::LOOT => &[anim::LOOT, anim::STAND],
        // Death has no useful substitute: a creature that dies into its idle
        // loop looks as if it did not die. So a model without it holds Stand,
        // as it did before this table existed.
        anim::DEATH => &[anim::DEATH, anim::STAND],

        // The swings, one chain per weapon family. Each falls back through the
        // families a model is most likely to have instead, and every chain ends
        // at the unarmed swing before Stand: a critter has no attack at all, and
        // a wolf has only its bite, which is `ATTACK_UNARMED`.
        anim::ATTACK_UNARMED => &[
            anim::ATTACK_UNARMED,
            anim::ATTACK_1H,
            anim::ATTACK_2H,
            anim::STAND,
        ],
        anim::ATTACK_1H => &[anim::ATTACK_1H, anim::ATTACK_UNARMED, anim::STAND],
        // The stab, which `AnimationData.dbc` falls back to the swing (row
        // 85 -> 17). Every character model carries it and no creature does,
        // so a creature the server says stabbed plays whatever attack it has.
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
        // A ranged attack falls back to the melee ones rather than to Stand:
        // a creature with a bow model and no bow animation must still visibly
        // act when it shoots.
        anim::ATTACK_BOW => &[anim::ATTACK_BOW, anim::ATTACK_RIFLE, anim::ATTACK_UNARMED, anim::STAND],
        anim::ATTACK_RIFLE => &[anim::ATTACK_RIFLE, anim::ATTACK_BOW, anim::ATTACK_UNARMED, anim::STAND],
        anim::ATTACK_THROWN => &[anim::ATTACK_THROWN, anim::ATTACK_UNARMED, anim::STAND],
        // The three off-hand swings and the critical: the animations the
        // packet and the off-hand item choose, rather than the main hand. All
        // of them fall back through the one-handed swing to the unarmed one.
        // Every character model has 87 and 88 and no creature model has any,
        // so these chains cover a creature the server says swung with its left
        // hand, which should play the ordinary attack it has. None falls back
        // through the two-handed swings, because a model with a 2H attack and
        // no `AttackOff` is a creature with no off hand.
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
        // The empty left hand, whose first fallback comes from
        // `AnimationData.dbc` (row 117 -> 87). This chain is used more than
        // the others because `HumanMale.m2` lacks the clip: a sword-and-board
        // fighter's off-hand blow plays `AttackOff` for every human.
        anim::ATTACK_UNARMED_OFF => &[
            anim::ATTACK_UNARMED_OFF,
            anim::ATTACK_OFF,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        // The critical flinch falls back through the ordinary one, not through
        // the swings: 10, 9 and 8 are the three results of one wound reaction,
        // and a model that lacks the large flinch still has the small one.
        // This chain once ended at `ATTACK_1H`, part of the same error as
        // treating 10 as the attacker's blow; see [`swing`].
        anim::COMBAT_CRITICAL => &[
            anim::COMBAT_CRITICAL,
            anim::COMBAT_WOUND,
            anim::STAND_WOUND,
            anim::STAND,
        ],

        // The flinch: 9 is the version for a unit already in a combat stance and
        // 8 the standing one; models carry one, the other, or neither.
        anim::COMBAT_WOUND => &[anim::COMBAT_WOUND, anim::STAND_WOUND, anim::STAND],
        // The three defensive reactions. Each falls back to the flinch rather
        // than to Stand: a creature with no dodge that dodges should still
        // visibly react, rather than stand motionless while the combat log
        // fills with misses.
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

        // The cast. These chains are required: `SpellCast` (32) and
        // `SpellPrecast` (31) are the `AnimationData` names, and no character
        // model carries either. `HumanMale.m2` has 51..54, the directed and
        // omni pairs, and nothing at 31 or 32. Without the chain, a request
        // for 32 would animate no player's cast at all.
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
        // The four ids the spell tables name, which a cast resolves to
        // whenever the tables give one: `SpellVisual` gives a fireball the
        // directed pair and a heal the omni pair. Requesting the generic id
        // would play the same pose for both.
        //
        // They have their own chains because creatures often cast players'
        // spells (a Defias mage casts the same fireball), and a creature model
        // carries at most `Spell` (2) and often nothing. Each therefore tries
        // its own pair, then the generic id, then Stand.
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
        // A channel is held like a wind-up, so it falls back to the wind-up
        // rather than the release: a caster whose model has no channel
        // animation should stand ready, not throw the spell repeatedly.
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
        // creature swimming in place looks better than one standing upright in
        // the water.
        anim::SWIM_IDLE => &[anim::SWIM_IDLE, anim::SWIM, anim::STAND],
        // The three other strokes, each falling back to Swim.
        // `AnimationData.dbc` gives that chain for `SwimBackwards` and sends
        // the two side strokes straight to Stand. Going through Swim for those
        // is this client's choice, for the same reason as the gait chains: a
        // creature with one stroke that swims sideways should swim.
        anim::SWIM_BACKWARDS => &[
            anim::SWIM_BACKWARDS,
            anim::SWIM,
            anim::SWIM_IDLE,
            anim::STAND,
        ],
        anim::SWIM_LEFT => &[anim::SWIM_LEFT, anim::SWIM, anim::SWIM_IDLE, anim::STAND],
        anim::SWIM_RIGHT => &[anim::SWIM_RIGHT, anim::SWIM, anim::SWIM_IDLE, anim::STAND],
        // The seated poses. A model with no sitting animation stands, which is
        // wrong but visibly so.
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
        // The two held states of a game object. Each ends at Stand, so a game
        // object whose model is a plain prop (a waving banner, a brazier)
        // plays its own idle. `AnimationData.dbc`'s chain runs the other way
        // (`Closed -> Close -> Open`, and `Open -> Close`, a cycle). That
        // works when followed one step at a time, but not as a list to search:
        // an open chest with no `Opened` would play the closing animation and
        // hold its last frame, which shows a shut chest.
        anim::CLOSED => &[anim::CLOSED, anim::STAND],
        anim::OPENED => &[anim::OPENED, anim::CLOSED, anim::STAND],
        anim::SLEEP => &[anim::SLEEP, anim::SIT_GROUND, anim::STAND],
        anim::KNEEL => &[anim::KNEEL, anim::SIT_GROUND, anim::STAND],
        // No chain. An id not listed here is played if the model has it and
        // dropped if not; see [`Playback::resolve`]. This covers the 78
        // emotes, which are scattered across `AnimationData` ids, and for
        // which a substitution would show a character standing still.
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
    /// The `usize::MAX` sequence and the `u16::MAX` id make the first
    /// [`Self::advance`] a first play: it starts whatever the state says with
    /// nothing to fade out of. The counters are `None` for the reason
    /// [`Self::seen`] gives: the first poll records and never plays, so a
    /// creature that comes into view mid-fight does not replay the blows
    /// landed before it was visible.
    ///
    /// A constructor rather than a struct literal because there are two
    /// callers, an entity's own rig and the mount under it, and two copies of
    /// twenty fields' defaults would drift apart.
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

    /// Record the window this frame advanced through; see
    /// [`Playback::window`]. Called by `animate` right after
    /// [`Self::advance`], before the frustum test, so a rig behind the camera
    /// still plays its sound cues.
    pub(super) fn note_window(&mut self, elapsed: u32) {
        self.window = Some(match self.window {
            Some((sequence, _, before, _)) if sequence == self.sequence => {
                (sequence, before, elapsed, false)
            }
            _ => (self.sequence, 0, elapsed, true),
        });
    }

    /// The base sequence's window this frame: `(sequence, from, to, fresh)`.
    /// See [`vale_assets::world::m2::SoundCues::in_window`]. `None` before the
    /// first frame.
    pub fn window(&self) -> Option<(usize, u32, u32, bool)> {
        self.window
    }

    /// Which of the model's sequences the base track is playing, or `None`
    /// before the first [`Self::advance`].
    ///
    /// The only reader is the mouse pick, which needs the sequence's own
    /// bounding sphere, as the 1.12.1 client uses for picking. With it, the
    /// pointer hovers a wisp only over the wisp, not over the twelve-yard cube
    /// of dust its model file declares. See
    /// [`super::EntityModel::pick_sphere`].
    ///
    /// This is the base track, not the overlay, as in the 1.12.1 client: a
    /// swing played over a run does not change the box, and a masked clip has
    /// no box of its own.
    pub fn clip(&self) -> Option<&vale_assets::world::m2::M2Sequence> {
        self.skeleton.sequences.get(self.sequence)
    }

    /// Take up the animation `wanted` and return how long the playing one has
    /// been running, in milliseconds.
    ///
    /// Only a change of sequence restarts the clock. 42 of the game's 405
    /// animated models have no Run, and one with neither gait resolves Run and
    /// Stand to the same idle loop. Restarting that loop every time the
    /// creature starts or stops moving makes the animation visibly reset for
    /// no reason, which looks like broken skinning.
    pub(super) fn advance(&mut self, state: u16, now: f32) -> u32 {
        // The masked track expires first. This order lets a moving caster's
        // wind-up take back the torso the frame after a swing over it ends:
        // `state_or_held` below only takes a free slot.
        if self.overlay.as_ref().is_some_and(|m| now >= m.until) {
            self.overlay = None;
        }
        // Death is the one state that does not wait for anything.
        // [`Self::fire`] already refuses to start a clip on a corpse (a
        // creature that flinched at the killing blow would stand back up to do
        // it), and this applies the same rule to a clip that was already
        // running when the blow landed. Without it, a mob killed mid-swing
        // finishes its swing, and its two-second attack animation shows a dead
        // body standing and fighting for two seconds. That was the cause of
        // "death animations are often delayed".
        //
        // All three are cleared, because a one-shot can be on either track,
        // and a deferred swing would otherwise be played at the end of
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
            // A change of state counts as a `PlayAnimation`, whether or not
            // the model resolves it to a different sequence, so the sheath
            // reconcile sees it. See [`Playback::played`].
            self.played = Some(wanted);
            self.take_up(wanted, now);
            // If a one-shot just ran out, the fade is from that one-shot, and
            // it is sampled where it stopped; see [`Fade::held`].
            if let (Some(ended), Some(fade)) = (ran_out, self.fade.as_mut()) {
                if fade.from == now {
                    fade.held = Some(ended);
                }
            }
        }
        // A finished fade is dropped rather than left to go negative. `pose`
        // would ignore it, but would also keep sampling a second animation
        // for the rest of the session.
        if self.fade.as_ref().is_some_and(|f| now - f.from >= FADE_SECS) {
            self.fade = None;
        }

        let elapsed = self.clock_at(self.sequence, now - self.since, self.base_rate);
        // A corpse holds the last frame of Death. Id 6 (`Dead`) is in
        // `AnimationData.dbc` and in none of the game's 411 creature models,
        // so there is nothing to play afterwards, and letting the clock loop
        // would play Death again, showing a body repeatedly falling over.
        //
        // `M2Skeleton::pose` clamps to the window, so holding means leaving
        // the clock unwrapped; every other state is a gait or an idle and
        // wraps.
        if self.wanted == anim::DEATH {
            return elapsed;
        }
        // The loot crouch also holds its last frame, because of a missing
        // clip rather than a missing state. `Loot` is the downward part of a
        // set of three whose hold (`LootHold`, 188) and return (`LootUp`, 189)
        // are in `AnimationData.dbc` and in none of the sixteen character
        // models. The clip is a 500 ms descent that does not come back up, so
        // its last frame is the crouch with the arm out. Letting the clock
        // wrap would play the descent again, showing a character bobbing at a
        // corpse. See `anim::LOOT` for the nine-phase measurement.
        if self.wanted == anim::LOOT {
            return elapsed;
        }
        let phase = self.skeleton.phase(self.sequence, elapsed);
        // A stun with no pose of its own, such as Ice Block, holds the clock;
        // see [`Playback::frozen`], which says what is measured here and what
        // is not. The phase is latched when the freeze begins rather than
        // recomputed, so the pose cannot drift. It is dropped as soon as the
        // flag clears, and the ordinary loop resumes where it would have
        // been, because `since` was not changed.
        if self.freeze {
            return *self.frozen.get_or_insert(phase);
        }
        self.frozen = None;
        phase
    }

    /// A held pose over the state, from a cast or an aura, or the state itself
    /// when neither gives one.
    ///
    /// The two sources are different kinds of hold. A cast's wind-up is an
    /// event with a length `SMSG_SPELL_START` states; an aura's pose is a
    /// condition that ends when the server stops reporting the aura. The cast
    /// takes precedence where both apply, because it is the more recent
    /// event. In practice they rarely coincide, since the server interrupts a
    /// stunned caster.
    fn state_or_held(&mut self, state: u16, now: f32) -> u16 {
        // A corpse holds nothing, and neither does a finished cast bar. The
        // timer stops an interrupted cast from leaving a character frozen in
        // the wind-up for the rest of the session.
        if let Some(cast) = self.casting.as_ref() {
            if now >= cast.until || state == anim::DEATH {
                self.casting = None;
                self.drop_held();
            }
        }
        let hold = match self.casting.as_ref() {
            Some(cast) => Some(cast.hold),
            // A corpse does not show a stun pose either. The aura stays in the
            // update fields after death (vmangos clears it a tick later), and
            // without this test the body would cower on the floor.
            None if state != anim::DEATH => self.aura,
            None => None,
        };
        let Some(hold) = hold else {
            // Nothing claims the slot, so no held pose may remain in it. This
            // currently does nothing (every place that clears `casting` also
            // drops the held overlay), and it keeps that true once a second
            // source can start and stop without a counter changing.
            self.drop_held();
            return state;
        };
        match self.hold_over(state, hold, now) {
            Some(wanted) => wanted,
            None => {
                // The held pose cannot be placed. A model with no `SpineLow`
                // cannot mask, so the choice is the whole body or nothing, and
                // a creature that walks away mid-cast has been interrupted.
                // This is what happened for every model before the overlay
                // existed. Walking does not interrupt an aura, so only the
                // cast is dropped; the aura pose is not drawn while the legs
                // move, and takes the whole body again as soon as they stop.
                self.casting = None;
                state
            }
        }
    }

    /// Apply `hold` over the whole body or over the torso, and return what the
    /// base track should play, or `None` when the hold could not be placed.
    ///
    /// A held pose covers the whole body only while the unit is standing
    /// still. Once the unit moves, the pose continues on the torso as a held
    /// loop on the masked overlay, and the legs play the gait. The 1.12.1
    /// client makes the same split, on movement flags `& 0x20000f`.
    ///
    /// The foot shuffle is deliberately not in the moving list, and the client
    /// uses two different masks for this: the one that moves a cast off the
    /// whole body is the direction bits plus swimming only, while the one that
    /// routes a one-shot ([`route_oneshot`]) includes the turn keys. Using the
    /// second in place of the first makes a cast switch between the two
    /// routes at the rate of mouse events.
    fn hold_over(&mut self, state: u16, hold: u16, now: f32) -> Option<u16> {
        // Every gait, not a subset. The list once named only Run, Walk, Swim
        // and SwimIdle, so once moving backward and the side strokes became
        // separate sequences, a caster who backed away kept their hands up.
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
            // Standing: the hold covers the whole body, and the torso overlay
            // is released.
            self.drop_held();
            return Some(hold);
        }
        // Masked onto the torso: the legs keep the state.
        self.hold_masked(hold, now).then_some(state)
    }

    /// Put a held wind-up on the masked track, if the track is free.
    ///
    /// False when the model cannot mask at all; [`Self::state_or_held`] then
    /// drops the cast. A masked one-shot already in the slot keeps it while it
    /// plays, and this still returns true: the swing uses the same arm as the
    /// cast, and the hold takes the subtree back the frame the one-shot ends.
    fn hold_masked(&mut self, hold: u16, now: f32) -> bool {
        if self.skeleton.key_bone(key_bone::SPINE_LOW).is_none() {
            return false;
        }
        match &self.overlay {
            // Already held, and by this same spell.
            Some(m) if m.looping && m.wanted == hold => return true,
            // A one-shot is playing in the slot: leave it and try again next
            // frame.
            Some(m) if !m.looping => return true,
            _ => {}
        }
        // Taking the torso counts as a play, so the sheath reconcile sees it,
        // which stows a moving caster's weapon.
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
    /// An id with no chain of its own is played or dropped, never substituted.
    /// These are the emotes: `Emotes.dbc` names 78 of them and a model carries
    /// the few its race was animated for, so most of them resolve to nothing
    /// on most models. Falling back to Stand would make a `/train` at a tauren
    /// interrupt whatever it was doing so that it could stand still, which is
    /// worse than ignoring the emote.
    /// The no-chain case uses `find_sequence` rather than `best_sequence`.
    /// `best_sequence` never fails: it ends with whatever the model has, which
    /// is right for a state (a creature with one idle loop looks better than a
    /// statue) and wrong for an emote, which a model may not have.
    fn resolve(&self, wanted: u16) -> Option<usize> {
        match fallbacks(wanted) {
            [] => self.skeleton.find_sequence(wanted),
            chain => self.skeleton.best_sequence(chain),
        }
    }

    fn take_up(&mut self, wanted: u16, now: f32) {
        // A model without the requested sequence plays whatever the chain
        // finds: a creature with one idle loop looks better than a statue.
        // See `fallbacks`.
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
        // A body that was already dead the first time this client saw it does
        // not fall over on arrival. Death is the one state whose clip is a
        // transition into the state rather than the state itself; every other
        // state here is a loop, so starting it at the beginning is harmless. A
        // creature that died out of sight arrives already a corpse, and playing
        // the clip from the start would make it stand up and fall over as the
        // tile loads.
        //
        // The clock is set back by the sequence's own length rather than
        // clamped, because `advance` leaves a corpse's clock unwrapped and
        // `M2Skeleton::pose` clamps to the window. So this gives the last
        // frame, by the same mechanism that holds it there afterwards.
        if first && wanted == anim::DEATH {
            let clip = &self.skeleton.sequences[sequence];
            self.since = now - clip.end.saturating_sub(clip.start) as f32 / 1000.0;
        }
    }

    /// Fire a one-shot: a swing, a flinch, an emote, a spell's release.
    ///
    /// Whether it plays on the whole body or the torso is decided at the
    /// moment of the call, from the unit's current state; see
    /// [`route_oneshot`]. A standing character plays the clip on the whole
    /// body. One that is running, swimming, seated or mid-jump plays it on the
    /// torso only and keeps moving. The id decides only which tests apply.
    ///
    /// Ignored on a corpse. A dead creature that flinched at the killing blow
    /// would stand back up to do it.
    fn fire(&mut self, wanted: u16, world: &WorldEntity, state: u16, now: f32) {
        if state == anim::DEATH {
            return;
        }
        // The combat fast path is tried first, as in the 1.12.1 client. A
        // swing started while another swing is still playing does not cut it
        // off; it speeds that swing up and waits.
        if self.fast_path(wanted, now) {
            return;
        }
        // Any other play clears the one-slot cache (the client clears it on
        // every play that does not take the fast path), so a deferred swing is
        // dropped by the clip that took precedence rather than played after
        // it.
        self.deferred = None;
        if self.route(wanted, world) == Route::Masked {
            self.fire_masked(wanted, now);
            return;
        }
        self.fire_full_body(wanted, now);
    }

    /// The combat fast path: a combat clip requested while another combat clip
    /// is playing is not started.
    ///
    /// Two things happen instead, and together they fix "fast attacks often do
    /// not play at all":
    ///
    /// * The running clip's rate is set to 2x, which re-times the rest of it
    ///   without moving the pose. A swing half way through finishes in half
    ///   the time rather than being cut off mid-arc.
    /// * The request is deferred to a one-slot cache and plays as soon as no
    ///   one-shot is playing (see the end of [`Self::note_actions`]).
    ///
    /// So a dagger swinging every 1.4 s with a 1.0 s clip loses nothing: the
    /// first swing is compressed and the second follows it. Without this, the
    /// second swing replaced the first immediately, so the first blow never
    /// showed, and a third arriving mid-clip replaced the second. A fast
    /// weapon then looked as though it was not swinging at all.
    ///
    /// Both ids must be combat ids, per [`combat_id`], the client's set. A
    /// flinch over a swing is deliberately not this case: a hit reaction must
    /// show immediately, so it replaces the swing by the ordinary path.
    ///
    /// The rate is set to 2 rather than doubled, because the client's 2.0 is
    /// an absolute speed: a third request against a clip already at 2x is
    /// deferred without shortening the clip again.
    fn fast_path(&mut self, wanted: u16, now: f32) -> bool {
        /// The client's fast-path rate.
        const FAST: f32 = 2.0;
        if !combat_id(wanted) {
            return false;
        }
        // Check whichever track holds the playing one-shot, the masked slot
        // first: a swing made at a run is on it, and the base track then
        // carries the gait, which is not a one-shot.
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
                // The pose is continuous: the phase at this instant is
                // unchanged, and everything after it advances at the new rate.
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

    /// Whether either track is still playing a one-shot. A held wind-up is not
    /// one: it is a loop with no end, and a deferred swing plays over it.
    fn oneshot_live(&self, now: f32) -> bool {
        self.oneshot.as_ref().is_some_and(|s| now < s.until)
            || self
                .overlay
                .as_ref()
                .is_some_and(|m| !m.looping && now < m.until)
    }

    /// [`route_oneshot`], plus the one thing it cannot know: whether this
    /// model's skeleton has a `SpineLow` to mask at.
    ///
    /// A wolf has none (it has a `Head` and no spine key bone). The client
    /// plays the whole body in that case, which is also what this client did
    /// for every model before the overlay existed. So a running wolf's bite
    /// still shows.
    fn route(&self, wanted: u16, world: &WorldEntity) -> Route {
        let route = route_oneshot(wanted, world.move_flags, world.stand_state, world.airborne);
        if route == Route::Masked && self.skeleton.key_bone(key_bone::SPINE_LOW).is_some() {
            Route::Masked
        } else {
            Route::FullBody
        }
    }

    /// Move a one-shot that started on the whole body to the torso as soon as
    /// the legs become committed.
    ///
    /// [`Self::fire`] decides the route once, from the state at the moment of
    /// the play. That is right for the choice, but the state can change. A
    /// character that swung, or released a cast, while standing still and
    /// then ran once kept the standing clip on the base track for its whole
    /// length: the legs never started the gait, so the character slid across
    /// the ground in the pose it started in. This mirrors
    /// [`Self::state_or_held`], which re-checks the same condition every frame
    /// for a held wind-up, so a cast begun standing and then walked out of
    /// already behaves correctly.
    ///
    /// The move keeps the pose continuous: the clip keeps its clock, its rate
    /// and its end, so the upper body does not change on that frame. The base
    /// track is released, and the next [`Self::advance`] fades it into the
    /// gait.
    ///
    /// The reverse move is deliberately not made. A masked clip whose unit
    /// stops moving stays on the torso. The 1.12.1 client's behaviour in that
    /// case is not established, and the visible difference is only what the
    /// legs do for the end of one clip, while getting the forward direction
    /// wrong makes a character slide.
    fn rehome_oneshot(&mut self, world: &WorldEntity, now: f32) {
        let Some(shot) = self.oneshot.as_ref().filter(|s| now < s.until) else {
            return;
        };
        let (wanted, until) = (shot.wanted, shot.until);
        if self.route(wanted, world) != Route::Masked {
            return;
        }
        // A newer one-shot already holds the subtree, such as a swing made
        // after the run began. The older full-body clip is dropped rather than
        // sharing the same bones; the newer clip would have replaced it anyway.
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
            // The base track's own clock, carried over unchanged, including
            // the 2x rate the combat fast path may have set on it.
            rate: self.base_rate,
            since: self.since,
            until,
            looping: false,
        });
        self.oneshot = None;
    }

    /// Play a one-shot on the masked track: the torso only.
    ///
    /// Replaces whatever was there (a second swing, or a held wind-up the arm
    /// interrupts), because the slot is one subtree and the newest play takes
    /// it. The wind-up takes it back when this ends, since `state_or_held`
    /// checks again every frame.
    fn fire_masked(&mut self, wanted: u16, now: f32) {
        // Recorded before the resolve, deliberately: the reconcile tests the
        // requested id, so a model with no such clip still reconciles.
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
    /// The duration comes from the sequence the model resolved to rather than
    /// from the requested id, because the fallback chain may have chosen a
    /// different sequence, and a one-shot held for the length of an animation
    /// that is not playing either cuts off or freezes.
    fn fire_full_body(&mut self, wanted: u16, now: f32) {
        // As in [`Self::fire_masked`]: the requested id, recorded before the
        // resolve.
        self.played = Some(wanted);
        let Some(sequence) = self.resolve(wanted) else {
            return;
        };
        // Restart the clock even when the sequence is already playing: two
        // swings in a row are two swings, not one held pose. `take_up`
        // deliberately does not restart in that case, so this does not use it.
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

    /// Play whatever each changed counter calls for.
    ///
    /// The first poll only records; it never plays. An entity that comes into
    /// view mid-fight arrives with a swing count of forty, and playing forty
    /// swings, or even one, for blows that landed before it was visible is
    /// worse than playing none.
    ///
    /// The order is a priority, because a skeleton plays one animation: when
    /// two events happen in one poll, only one is shown. A release takes
    /// precedence over a swing because a spell landing is the more noticeable
    /// event. A swing takes precedence over a flinch because two units hitting
    /// each other at the same time should each look as if they are attacking,
    /// not as if they are being hit. An emote comes last: it is the only one of
    /// the five a player chose, and so the one they will notice missing, but it
    /// should not interrupt a fight.
    pub(super) fn note_actions(
        &mut self,
        world: &WorldEntity,
        sheath: u8,
        emote_anim: impl FnOnce(u32) -> Option<u16>,
        // The pose a `SpellVisualKit` holds, looked up by kit id: the only
        // lookup here that does not start from a spell. See
        // [`vale_protocol::play::sound`]: two packets carry a kit and no
        // spell. For the two kits that matter, food (406) and drink (438), the
        // pose is the visible part: `animID 61`, `EmoteEat`.
        kit_anim: impl Fn(u32) -> Option<u16>,
        cast_anim: impl FnOnce(u32) -> CastAnimation,
        state: u16,
        now: f32,
    ) {
        // Set before the first-poll check, because this is not an event: it is
        // how fast the legs should cycle, and a rig on its first poll still
        // has to play its gait at the right rate. See [`Self::speed`].
        self.speed = world.speed;
        // Also before the first-poll check, for the same reason: this is not
        // an event but a check of whether the legs are still free, made every
        // frame because the answer changes while a clip plays. See
        // [`Self::rehome_oneshot`].
        self.rehome_oneshot(world, now);
        let seen = Counters::of(world);
        let Some(last) = self.seen.replace(seen) else {
            return;
        };
        // The two ends of the jump arc go first, so that anything below
        // replaces them. A swing or a release is a single event the server
        // sends once, while the flight still has `Jump` or `Fall` under it.
        // Losing the take-off costs a third of a second of decoration; losing
        // the swing loses the event.
        //
        // A rider plays neither end; the mount plays the whole arc.
        // [`wanted_animation`] gives `Mount` (91) precedence over the air, so
        // the middle of the arc is not drawn on the rider, and an end with no
        // middle shows a character sitting still all the way up and then
        // standing out of the saddle to absorb the landing. That was the
        // "jumping while mounted causes this upon landing" report. Only the
        // landing showed, because the take-off below requires
        // `state == JUMP`, which a mounted unit never has, and the landing had
        // no condition. The mount plays its own two through
        // [`Self::note_flight_only`].
        if !world.mounted {
            self.note_flight(seen, last, world, state, now);
        }
        // A door opening or closing has the same form: two held poses with a
        // one-shot between them.
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
        // A cast's animations are a property of the spell. The wind-up and the
        // release are resolved by one lookup, so they cannot describe
        // different spells.
        let cast_moved =
            seen.casts_begun != last.casts_begun || seen.casts_released != last.casts_released;
        let spell = if cast_moved {
            cast_anim(world.last_spell)
        } else {
            CastAnimation::default()
        };

        // A cast beginning is a state, not a one-shot, so it is handled outside
        // the priority order below: a caster who is also being hit should
        // flinch and then return to the wind-up.
        //
        // A spell that states no wind-up has none. The release below always
        // followed this rule and the wind-up once did not:
        // `hold.unwrap_or(SPELL_PRECAST)` supplied a wind-up for every spell
        // the tables gave none, and no character model carries `SpellPrecast`
        // (31), so its chain fell through to `ReadySpellOmni` (52), which every
        // character model carries. That affected 9,467 of the game's 22,360
        // spells (`vale spell` counts 12,893 reaching an animation): every
        // proc, every aura application and every silent utility spell raised
        // the caster's hands.
        //
        // `SpellVisualKit`'s `animID` is the only data that says what a cast
        // looks like, and a spell with no visual, or a kit whose `animID` is
        // zero, deliberately has no animation.
        if seen.casts_begun != last.casts_begun && state != anim::DEATH {
            // A channel's begin holds the channel's own pose, which comes from
            // a different kit from the wind-up's and is not a fallback for it.
            // `apply_channel_start` increments this counter a second time and
            // sets the bar to the channel's length, so this block runs twice
            // for a channelled spell: once for the wind-up and once for the
            // channel. Reading `hold` both times held Blizzard's caster in
            // `ReadySpellOmni` for four seconds. This affects 61 of the game's
            // 323 channelled spells, measured in
            // `vale_assets::tables::spell::CastAnimation::channel`.
            let channelling = seen.casts_channelled != last.casts_channelled;
            let held = match channelling {
                true => spell.channel.or(spell.hold),
                false => spell.hold,
            };
            self.casting = held.map(|hold| Casting {
                hold,
                // A zero-length wind-up is an instant spell whose `START`
                // arrived anyway; the release follows immediately and clears
                // this.
                until: now + world.cast_time_ms as f32 / 1000.0,
            });
            // A cast replaces whatever was held, and one with no wind-up of
            // its own replaces it with nothing. Otherwise the previous spell's
            // pose would stay on the torso for the length of this cast bar.
            if self.casting.is_none() {
                self.drop_held();
            }
        }

        // A pushback makes the wind-up last longer; it is the one change to a
        // cast in progress that does not end it. `Casting::until` is set from
        // `SMSG_SPELL_START`'s `m_timer`, the only cast length the packets
        // state. Without this, a Fireball pushed back by two blows dropped its
        // held pose a second before it was thrown, and the caster stood
        // empty-handed while the bar was still running. This comes before the
        // cancellation below, so that a cast pushed back and then interrupted
        // in the same poll still ends.
        if seen.casts_delayed != last.casts_delayed {
            if let Some(casting) = &mut self.casting {
                casting.until += world.last_cast_delay_ms as f32 / 1000.0;
            }
        }

        // A cancelled cast ends the wind-up with nothing after it. This is
        // outside the priority chain below because it plays nothing: it only
        // stops, so it must not use the slot a swing or a flinch arriving in
        // the same poll would take. See `WorldEntity::casts_cancelled` for the
        // three packets that change it. Without it the pose was held until
        // `Casting::until`, which for a silenced Fireball is the rest of its
        // bar with nothing at the end.
        if seen.casts_cancelled != last.casts_cancelled {
            self.casting = None;
            self.drop_held();
        }

        // A channel's begin arrives after its release, so the release must not
        // clear it. vmangos sends `SendSpellGo` and then `SendChannelStart`,
        // both within one poll, and this block runs after the one that sets
        // the wind-up. Without the guard it cancelled the channel pose just
        // after it was set and played the release instead. Evocation has no
        // release (`SpellVisual` gives it a channel kit and nothing else), so
        // the mage stood in its idle loop for eight seconds. See
        // `WorldEntity::casts_channelled`.
        if seen.casts_released != last.casts_released
            && seen.casts_channelled == last.casts_channelled
        {
            self.casting = None;
            // The wind-up leaves the torso with the cast it belonged to;
            // otherwise a moving caster would hold it indefinitely.
            self.drop_held();
            // A spell that states no release has none. Playing the generic
            // release anyway would throw a fireball at the end of a channel:
            // `SpellVisual` gives Arcane Missiles a channel kit and nothing
            // else.
            //
            // This includes a spell the tables give no animation at all. That
            // case was once an exception: `is_empty()` played a generic
            // `SpellCast` for the 9,467 spells that reach no animation, on the
            // assumption that the tables had not been read yet. They are read
            // now, and no animation is a valid result; see the wind-up above,
            // which made the same change and is where it is most visible.
            if let Some(release) = spell.release {
                self.fire(release, world, state, now);
            } else if let Some(shot) = spell.ranged_shot.then(|| ranged_shot(world)).flatten() {
                // The one release the tables do not carry. A ranged weapon
                // attack's one-shot comes from the wielder's weapon rather
                // than the spell's visual, and the two spells a player
                // repeats, Auto Shot (75) and the wand's Shoot (5019), have
                // `SpellVisual = 0`. The rule above ("a spell that states no
                // release has none") would leave an archer standing still
                // through a whole volley, which is what was reported.
                //
                // Tested after `release` rather than before it, because a
                // ranged ability that names a kit (Aimed Shot, Multi-Shot)
                // uses that kit: the spell's data is more specific than the
                // weapon.
                self.fire(shot, world, state, now);
            }
        } else if seen.swings != last.swings {
            self.fire(swing(world, sheath), world, state, now);
        } else if seen.blows != last.blows {
            self.fire(reaction(world, sheath), world, state, now);
        } else if seen.emotes != last.emotes {
            // The emote lookup. `SMSG_EMOTE` carries an `Emotes.dbc` id, and
            // that row's third column is the animation, so this is the closest
            // the server comes to naming a pose. An emote with no animation,
            // or no `Emotes.dbc` loaded, resolves to nothing and the entity
            // continues what it was doing.
            if let Some(id) = emote_anim(world.last_emote) {
                self.fire(id, world, state, now);
            }
        } else if seen.spell_visuals != last.spell_visuals
            || seen.spell_impacts != last.spell_impacts
        {
            // The kit lookup, which is one step shorter:
            // `SMSG_PLAY_SPELL_VISUAL` carries a `SpellVisualKit` id, and that
            // row's `animID` column is the pose.
            //
            // This is how eating is drawn. vmangos sends kit 406 (food) or 438
            // (drink) on every regeneration tick a character spends sitting
            // with either, and both have `animID 61`, `EmoteEat`. It plays
            // again on each tick rather than being held, following the
            // server's timing: no packet says when eating stops, so a held
            // pose would need an invented end.
            //
            // Last in the chain, after the emote, because it is the least
            // specific of the five: a swing, a flinch, a release and an emote
            // each name what happened, and this names a visual. The visual
            // takes precedence over the impact in the same poll for the same
            // reason a swing takes precedence over a flinch: it is what the
            // unit did rather than what was done to it.
            let kit = match seen.spell_visuals != last.spell_visuals {
                true => world.last_spell_visual,
                false => world.last_spell_impact,
            };
            if let Some(id) = kit_anim(kit) {
                self.fire(id, world, state, now);
            }
        }

        // The deferred swing, played once no one-shot is playing. This is the
        // second half of [`Self::fast_path`]; the 1.12.1 client also plays the
        // deferred swing when the current one ends. A poll that played
        // anything has already cleared the cache through `fire`; one that took
        // the fast path leaves the clip it sped up still running, so this
        // waits for it.
        if let Some(parked) = self.deferred.filter(|_| !self.oneshot_live(now)) {
            self.deferred = None;
            self.fire(parked, world, state, now);
        }
    }

    /// The take-off and the landing: the start and end of being off the
    /// ground, detected here rather than counted by the server.
    ///
    /// The middle of the arc is a state, drawn by [`wanted_animation`]: `Jump`
    /// for an arc the unit pushed off into, `Fall` for a step off a ledge. A
    /// state has no ends. 1.12 provides `JumpStart` (37) as the crouch and
    /// launch, `JumpEnd` (39) as the absorb and straighten, and `JumpLandRun`
    /// (187) as the landing for a character still running when it lands.
    /// Measured on `HumanMale`, `JumpLandRun` carries 6.9 y/s of travel and
    /// `JumpEnd` carries none, which is the whole difference between them.
    ///
    /// The take-off plays only for a jump, never for a fall. A character who
    /// walks off a cliff did not push off, and `MSG_MOVE_JUMP`'s `zspeed`
    /// tells the two apart (see `WorldEntity::jumping`). The landing has no
    /// such condition: everything that comes down lands.
    ///
    /// A rider plays neither end. The caller decides that, because the
    /// mount's own [`Playback`] reaches this through
    /// [`Self::note_flight_only`] with the same mounted `WorldEntity`; see
    /// [`Self::note_actions`] for the condition and the reason for it.
    ///
    /// This is covered by the same first-poll check as the counters, because
    /// it is called after that check: an entity that comes into view already
    /// in mid-air must not play a take-off it did not make, and one that
    /// comes into view on the ground must not land.
    ///
    /// All three ids are outside [`class_a`], so all three play on the whole
    /// body whatever the state. That is correct: the crouch and the absorb
    /// are movements of the legs.
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
            // the mover reports that the unit pushed off.
            if state == anim::JUMP {
                self.fire(anim::JUMP_START, world, state, now);
            }
            return;
        }
        // Landed. Which landing plays depends on which keys are held (see
        // [`landing_for`]), and a landing the model cannot play, or one the
        // rule says not to play, still ends the take-off. The 1.12.1 client
        // returns to the gait in both cases (directly, or through the fallback
        // column). Leaving the one-shot to run out would keep the mount in its
        // launch pose on the ground.
        match landing_for(world.move_flags) {
            Some(landing) if self.resolve(landing).is_some() => {
                self.fire(landing, world, state, now);
            }
            Some(landing) => {
                // Record the request for the sheath reconcile, as
                // [`Self::fire`] does, then return to the gait.
                self.played = Some(landing);
                self.oneshot = None;
            }
            None => self.oneshot = None,
        }
    }

    /// The two ends of the jump arc and nothing else: the mount's part of
    /// [`Self::note_actions`].
    ///
    /// A mount has no counters of its own: no packet describes one, so it has
    /// no swings, no casts, no emotes and no death, and
    /// [`super::mount::gait`] is deliberately a smaller set than a unit's for
    /// that reason. It does have a jump, because the rider's jump is the
    /// mount's. `Creature\Horse\Horse.m2` carries all four clips (`JumpStart`
    /// 37, `Jump` 38, `JumpEnd` 39, `Fall` 40), and before this only the two
    /// states were played.
    ///
    /// The rider plays neither end (see [`Self::note_flight`]): `Mount` takes
    /// precedence over the air on a rider, so the mount plays the whole arc,
    /// ends included.
    ///
    /// It shares [`Self::seen`] with `note_actions`, and nothing calls both on
    /// the same `Playback`. A mount's is written only here and a unit's only
    /// there. Calling both would make one's first-poll check consume the
    /// other's.
    pub(super) fn note_flight_only(&mut self, world: &WorldEntity, state: u16, now: f32) {
        let seen = Counters::of(world);
        let Some(last) = self.seen.replace(seen) else {
            return;
        };
        self.note_flight(seen, last, world, state, now);
    }

    /// How long one of the model's sequences runs in wall-clock time, which for
    /// a rate-scaled gait differs from its authored length.
    ///
    /// Takes an index rather than using the playing sequence, because there
    /// are two tracks and a masked one-shot's length is not the base track's.
    fn length_ms(&self, sequence: usize) -> Option<u32> {
        let seq = self.skeleton.sequences.get(sequence)?;
        let authored = seq.end.saturating_sub(seq.start) as f32;
        Some((authored / self.skeleton.playback_rate(sequence, self.speed)) as u32)
    }

    /// Milliseconds into a sequence after `elapsed` seconds of wall clock.
    ///
    /// The playback rate is applied here, to the whole span rather than
    /// accumulated frame by frame; see [`M2Skeleton::playback_rate`] for what
    /// the rate is. The difference shows only when a unit's speed changes while
    /// one clip keeps playing: the phase jumps once, because the whole elapsed
    /// span is rescaled. A walk-to-run change swaps the sequence and restarts
    /// the clock anyway, so the remaining case is a haste effect applied
    /// mid-stride: a one-frame discontinuity at a moment the game already
    /// shows a change, instead of a running total this would have to keep.
    /// This is an approximation: the 1.12.1 client accumulates.
    pub(super) fn clock(&self, sequence: usize, elapsed: f32) -> u32 {
        self.clock_at(sequence, elapsed, 1.0)
    }

    /// Where the fading clip is sampled: its wrapped clock for a loop, and its
    /// last frame for a one-shot that ran out. See [`Fade::held`], which has
    /// the measurement.
    ///
    /// Clamped to the window's last millisecond rather than passed through
    /// [`M2Skeleton::phase`], because the phase of an elapsed span equal to
    /// the length is zero, which showed the first frame instead of the last.
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

    /// [`Self::clock`], times this play's own multiplier, which is 1 for
    /// everything except a combat clip the fast path has re-timed. See
    /// [`Playback::fast_path`].
    fn clock_at(&self, sequence: usize, elapsed: f32, rate: f32) -> u32 {
        (elapsed * 1000.0 * rate * self.skeleton.playback_rate(sequence, self.speed)).max(0.0)
            as u32
    }

    /// The animation a play was started with since this was last asked, if any.
    ///
    /// This takes the value rather than reading it, so the sheath reconcile
    /// runs once per play and never once per frame. See [`Playback::played`]
    /// and [`super::sheath::reconcile`].
    pub(super) fn take_played(&mut self) -> Option<u16> {
        self.played.take()
    }

    /// Where in its stride the base track is: `(animation id, fraction 0..1 of
    /// the cycle)` while the legs play a travelling ground gait, `None`
    /// otherwise.
    ///
    /// Used by the footstep system (`crate::sound::footsteps`), which watches
    /// the fraction pass the two footfalls. The `move_speed` test excludes a
    /// model whose gait resolved to an idle loop (42 of 405 have no Run),
    /// because footsteps from an idle loop are worse than silent feet. The
    /// clock arithmetic is [`Playback::advance`]'s, without changing state.
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

