//! Entity animation: which sequence an entity plays, and posing its skeleton.
//!
//! This is the largest module in this directory and the most self-contained.
//! [`animate`] and [`animate_attachment`] drive the skeletons.
//! [`wanted_animation`] and the functions near it decide which sequence the
//! server's state calls for. [`fallbacks`] (in `vale_assets::look::pose`) is the
//! table of what to play when a model does not have that sequence.
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
        debug_assert!(self.roots.is_empty(), "attachment roots need `apply_roots` first");
        self.roots.clear();
    }

    /// Write the attachment roots collected and empty that list. A caller
    /// whose attachments carry nested models calls this before
    /// [`Self::apply`]: [`animate_attachment`] collects a root for each nested
    /// model.
    pub(crate) fn apply_roots<F: bevy::ecs::query::QueryFilter>(
        &mut self,
        roots: &mut Query<&mut Transform, F>,
    ) {
        for (root, local) in self.roots.drain(..) {
            if let Ok(mut transform) = roots.get_mut(root) {
                *transform = local;
            }
        }
    }

    /// [`Self::apply`], plus the attachment roots. [`animate`] uses this form.
    fn apply_with_roots<F: bevy::ecs::query::QueryFilter>(
        &mut self,
        joints: &mut Query<&mut GlobalTransform, F>,
        tags: &mut Query<&mut bevy::mesh::MeshTag, With<EntityPart>>,
        roots: &mut Query<&mut Transform, With<AttachedTo>>,
    ) {
        self.apply_roots(roots);
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
    ground: Option<&(crate::world::standing::Standing, u32)>,
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

/// Whether a landing one-shot gives way to `state`: a standing landing to any
/// travelling gait, and a running landing to any state that does not travel.
/// See [`Playback::advance`].
pub(super) fn landing_yields(landing: u16, state: u16) -> bool {
    let travels = matches!(
        state,
        anim::WALK
            | anim::RUN
            | anim::SPRINT
            | anim::WALK_BACKWARDS
            | anim::RUN_LEFT
            | anim::RUN_RIGHT
            | anim::STEALTH_WALK
    );
    match landing {
        anim::JUMP_END => travels,
        anim::JUMP_LAND_RUN => !travels,
        _ => false,
    }
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
///
/// A melee blow is made with the melee weapon whether or not it was drawn, so
/// a stowed unit asks for the armed swing, not `AttackUnarmed`. The armed
/// swing carries `DRAW_MELEE`, and the reconcile after the play draws the
/// weapon. Asking for `AttackUnarmed` instead made the first blow from a
/// stowed weapon a punch, and its `STOW_EMPTY_HANDS` kept the weapon stowed.
/// A ranged-drawn unit keeps what it holds.
pub(super) fn swing(world: &WorldEntity, sheath: u8) -> u16 {
    use vale_protocol::play::action::hit_info;
    let sheath = if sheath == SHEATH_STATE_RANGED { sheath } else { SHEATH_STATE_MELEE };
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

pub(super) use vale_assets::look::pose::fallbacks;

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
        //
        // The two landings are the exception, because each is a statement
        // about the legs: `JumpEnd` is a landing standing still and
        // `JumpLandRun` one that runs on. A landing that no longer matches what
        // the legs are doing gives way to the gait at once. Otherwise a unit
        // that landed still and set off again slid along in the landing's
        // crouch for the rest of the clip, its legs doing nothing.
        let mut ran_out = None;
        if self
            .oneshot
            .as_ref()
            .is_some_and(|shot| now < shot.until && landing_yields(shot.wanted, state))
        {
            self.oneshot = None;
            ran_out = Some(now);
        }
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
        // A game object's scripted one-shot (`SMSG_GAMEOBJECT_CUSTOM_ANIM`,
        // `SMSG_GAMEOBJECT_DESPAWN_ANIM`): a fishing bobber's splash when a
        // fish bites, a trap firing. Before the state edge below, so that a
        // state change in the same poll replaces it.
        if seen.object_anims != last.object_anims {
            let id = world.object_anim.and_then(|anim| match anim {
                vale_protocol::play::object::ObjectAnim::Custom(n) => {
                    vale_assets::look::object::custom_anim(n)
                }
                vale_protocol::play::object::ObjectAnim::Despawn => {
                    Some(vale_assets::look::object::DESPAWN_ANIM)
                }
            });
            if let Some(id) = id {
                self.fire(id, world, state, now);
            }
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
    /// otherwise. `Sprint` is one of the gaits: a mount at 60% or 100% speed
    /// asks for it and plays its `Run` in its place, and leaving it out made
    /// every mounted footfall silent.
    ///
    /// Used by the footstep system (`crate::sound::footsteps`), which watches
    /// the fraction pass the two footfalls. The `move_speed` test excludes a
    /// model whose gait resolved to an idle loop (42 of 405 have no Run),
    /// because footsteps from an idle loop are worse than silent feet. The
    /// clock arithmetic is [`Playback::advance`]'s, without changing state.
    pub(crate) fn stride(&self, now: f32) -> Option<(u16, f32)> {
        if !matches!(self.wanted, anim::WALK | anim::RUN | anim::SPRINT | anim::WALK_BACKWARDS) {
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

#[cfg(test)]
mod tests {
    use crate::world::entities::*;
    use crate::world::entities::test_support::*;
    use vale_assets::world::m2::{M2Bone, M2Skeleton};

    /// A `Playback` over a boneless skeleton. Every test below that is not about the
    /// masked track uses it.
    ///
    /// A boneless skeleton has no `SpineLow`. Without that bone the 1.12.1 client
    /// does not split the body, so every one-shot plays on the full body. Real
    /// creatures are in this case: a wolf has a `Head` and no spine key bone. See
    /// [`playing_with_spine`] for a skeleton that can split.
    fn playing(ids: &[u16]) -> Playback {
        Playback {
            hints: Vec::new(),
            window: None,
            wanted: u16::MAX,
            sequence: usize::MAX,
            since: 0.0,
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
            skeleton: skeleton(ids),
            played: None,
        }
    }

    /// The same, over a skeleton that can mask: two bones, the first of them
    /// `SpineLow`.
    fn playing_with_spine(ids: &[u16]) -> Playback {
        let mut skeleton = (*skeleton(ids)).clone();
        skeleton.bones = vec![bone(key_bone::SPINE_LOW, -1), bone(-1, 0)];
        Playback {
            hints: Vec::new(),
            window: None,
            skeleton: Arc::new(skeleton),
            ..playing(ids)
        }
    }

    fn bone(key: i16, parent: i16) -> M2Bone {
        M2Bone {
            key_bone: key,
            flags: 0,
            parent,
            pivot: [0.0; 3],
            translation: None,
            rotation: None,
            scale: None,
        }
    }

    /// A two-handed sword: the family whose swing is `Attack2H`.
    fn a_greatsword() -> Weapon {
        Weapon {
            display_id: 30606,
            class: 2,
            subclass: 8, // SWORD2
            inventory_type: 17,
            sheath: 1,
            material: 1,
            enchantments: [0; 7],
        }
    }

    /// A dagger — subclass 15, the one one-hander that stabs.
    fn a_dagger() -> Weapon {
        Weapon {
            display_id: 6768,
            class: 2,
            subclass: 15, // DAGGER
            inventory_type: 13,
            sheath: 3,
            material: 1,
            enchantments: [0; 7],
        }
    }

    /// A fist weapon — subclass 13, held one-handed and thrown as a punch.
    fn a_fist_weapon() -> Weapon {
        Weapon {
            display_id: 12163,
            class: 2,
            subclass: 13, // FIST
            inventory_type: 13,
            sheath: 3,
            material: 1,
            enchantments: [0; 7],
        }
    }

    fn a_fishing_pole() -> Weapon {
        Weapon {
            display_id: 8371,
            class: 2,
            subclass: 20, // FISHING_POLE
            inventory_type: 17,
            sheath: 1,
            material: 2, // WOOD
            enchantments: [0; 7],
        }
    }

    /// Not a weapon at all: `ITEM_CLASS_ARMOR` / `INVTYPE_SHIELD`.
    fn a_shield() -> Weapon {
        Weapon {
            display_id: 18730,
            class: 4,
            subclass: 6,
            inventory_type: 14,
            sheath: 4,
            material: 1,
            enchantments: [0; 7],
        }
    }

    /// [`Playback::note_actions`] with no emote table behind it and a spell that
    /// states both halves of its cast.
    ///
    /// The emote lookup needs a DBC and these tests open no archive, so it returns
    /// `None`. The cast lookup does not return a default: there is no generic
    /// fallback, so `CastAnimation::default()` means the spell has no cast
    /// animation. That is a fact about the spell, not about the pose code under
    /// test, so the fixture's spell names the generic pair
    /// (`SPELL_PRECAST`, `SPELL_CAST`) explicitly. The one test about a spell with
    /// no visual passes its own closure.
    fn note(play: &mut Playback, world: &WorldEntity, state: u16, now: f32) {
        note_sheathed(play, world, vale_assets::look::sheath::UNARMED, state, now);
    }

    /// The same with a sheath state passed in, for the tests about which swing a
    /// drawn weapon plays.
    fn note_sheathed(play: &mut Playback, world: &WorldEntity, sheath: u8, state: u16, now: f32) {
        play.note_actions(
            world,
            sheath,
            |_| None,
            |_| None,
            |_| CastAnimation {
                hold: Some(anim::SPELL_PRECAST),
                release: Some(anim::SPELL_CAST),
                ..CastAnimation::default()
            },
            state,
            now,
        );
    }

    /// The weapon decides the swing family and the packet decides which hand.
    ///
    /// `HITINFO_LEFTSWING` was once parsed and carried but not read, so a rogue's
    /// off-hand swing was drawn as the ordinary main-hand blow. This test pins the
    /// fix.
    ///
    /// `HITINFO_CRITICALHIT` is not read here. `CombatCritical` (10) belongs to the
    /// victim's wound animation (10, 9 or 8, chosen off one flag), not to the
    /// attacker's swing. When the attacker played it, a fifth of every character's
    /// blows showed a being-hit animation. See [`swing`] and [`reaction`].
    #[test]
    fn the_swing_is_chosen_by_the_hand_and_never_by_the_crit() {
        use vale_protocol::play::action::hit_info;

        let mut armed = moving(0.0);
        armed.weapons[0] = a_greatsword();
        let out = SHEATH_STATE_MELEE;

        // No bits set: the weapon's own family.
        assert_eq!(swing(&armed, out), anim::ATTACK_2H);

        // A critical is still the weapon's own swing. The victim shows the
        // critical.
        armed.last_swing_info = hit_info::CRITICAL_HIT;
        assert_eq!(swing(&armed, out), anim::ATTACK_2H);

        // A two-hander has no off hand at all, so a left swing is a punch.
        armed.last_swing_info = hit_info::LEFT_SWING;
        assert_eq!(swing(&armed, out), anim::ATTACK_UNARMED_OFF);

        // Both bits set: the hand decides.
        armed.last_swing_info = hit_info::LEFT_SWING | hit_info::CRITICAL_HIT;
        assert_eq!(swing(&armed, out), anim::ATTACK_UNARMED_OFF);

        // Bits other than these two change nothing. The server sets
        // `AFFECTS_VICTIM` on nearly every blow, so testing the mask by equality
        // would make every ordinary swing a critical.
        armed.last_swing_info = hit_info::AFFECTS_VICTIM | hit_info::MISS;
        assert_eq!(swing(&armed, out), anim::ATTACK_2H);

        // An unarmed creature that the server says swung left still bites. The
        // fallback chain, not the choice, covers a model without the off-hand
        // sequence.
        let creature = moving(0.0);
        assert_eq!(swing(&creature, out), anim::ATTACK_UNARMED);
        for id in [
            anim::ATTACK_OFF,
            anim::ATTACK_OFF_PIERCE,
            anim::ATTACK_UNARMED_OFF,
        ] {
            assert_eq!(
                fallbacks(id).last(),
                Some(&anim::STAND),
                "every chain ends at Stand"
            );
            assert!(
                fallbacks(id).contains(&anim::ATTACK_UNARMED),
                "a creature with no off hand has to fall through to its bite"
            );
        }
    }

    /// A critical is the victim's animation, and it is the one flinch that does not
    /// cut a swing off.
    ///
    /// Both halves match the 1.12.1 client. It plays 10 (`CombatCritical`) rather
    /// than 9 (`CombatWound`) when the blow carries the critical flag. It treats 10
    /// as a combat animation and 9 as not one, so a critical taken mid-swing goes
    /// through the combat fast path and an ordinary hit does not.
    #[test]
    fn a_critical_blow_is_the_big_flinch_and_falls_back_through_the_small_one() {
        use vale_protocol::play::action::hit_info;
        use vale_protocol::play::action::victim_state;

        let mut hit = moving(0.0);
        let out = SHEATH_STATE_MELEE;
        hit.last_victim_state = victim_state::NORMAL;
        assert_eq!(reaction(&hit, out), anim::COMBAT_WOUND);

        hit.last_blow_info = hit_info::AFFECTS_VICTIM | hit_info::CRITICAL_HIT;
        assert_eq!(reaction(&hit, out), anim::COMBAT_CRITICAL);

        // A dodge, a parry and a block keep their own animations. The critical bit
        // is read only when the blow landed.
        hit.last_victim_state = victim_state::DODGE;
        assert_eq!(reaction(&hit, out), anim::DODGE);
        hit.last_victim_state = victim_state::BLOCKS;
        assert_eq!(reaction(&hit, out), anim::SHIELD_BLOCK);

        // The fallback chain is the wound family's, not the swings'. A model with
        // no `CombatCritical` falls back to `CombatWound`.
        let chain = fallbacks(anim::COMBAT_CRITICAL);
        assert!(chain.contains(&anim::COMBAT_WOUND), "{chain:?}");
        assert!(!chain.contains(&anim::ATTACK_1H), "{chain:?}");
        assert_eq!(chain.last(), Some(&anim::STAND));
    }

    /// The off-hand swing is chosen by the off-hand item, separately from the main
    /// hand. Before this, every left swing played `AttackOff`.
    ///
    /// The expected values match the 1.12.1 client's swing for each
    /// `(class, subclass)`. The main case is the rogue with two daggers, which
    /// plays the same stab from each hand.
    #[test]
    fn the_off_hand_swing_follows_the_off_hand_item() {
        use vale_protocol::play::action::hit_info;
        let out = SHEATH_STATE_MELEE;
        let mut rogue = moving(0.0);
        rogue.last_swing_info = hit_info::LEFT_SWING;
        rogue.weapons[0] = a_dagger();

        // Empty left hand: the punch.
        assert_eq!(swing(&rogue, out), anim::ATTACK_UNARMED_OFF);
        // A dagger in it: the stab.
        rogue.weapons[1] = a_dagger();
        assert_eq!(swing(&rogue, out), anim::ATTACK_OFF_PIERCE);
        // Any other weapon: the ordinary off-hand swing.
        rogue.weapons[1] = a_mace();
        assert_eq!(swing(&rogue, out), anim::ATTACK_OFF);
        // A shield is not a weapon, so a left-handed blow from a sword-and-shield
        // fighter is a punch rather than a shield swing.
        rogue.weapons[1] = a_shield();
        assert_eq!(swing(&rogue, out), anim::ATTACK_UNARMED_OFF);

        // The main-hand dagger stabs rather than swings.
        rogue.last_swing_info = 0;
        assert_eq!(swing(&rogue, out), anim::ATTACK_1H_PIERCE);
    }

    /// A fist weapon swings as a punch, and a fishing pole has its own ready stance
    /// and swing. These are the two weapon kinds whose ready stance and swing do not
    /// come from the same weapon family in the 1.12.1 client.
    #[test]
    fn the_ready_stance_and_the_swing_can_disagree() {
        use vale_assets::tables::item::WeaponAnim;
        let fist = WeaponAnim::of(&a_fist_weapon());
        assert_eq!(fist, WeaponAnim::Fist);
        assert_eq!(fist.attack(), anim::ATTACK_UNARMED, "a fist throws a punch");
        assert_eq!(
            fist.ready(),
            anim::READY_1H,
            "…held in the one-handed guard"
        );

        let pole = WeaponAnim::of(&a_fishing_pole());
        assert_eq!(pole.attack(), anim::ATTACK_2HL, "swung like a polearm");
        assert_eq!(pole.ready(), anim::READY_UNARMED, "…and held like nothing");
    }

    /// The three gaits, chosen from the speed the server sends rather than from the
    /// difference of two drawn positions.
    #[test]
    fn the_gait_comes_off_the_stated_speed() {
        assert_eq!(wanted_animation(&moving(0.0), 0, None), anim::STAND);
        assert_eq!(wanted_animation(&moving(2.5), 0, None), anim::WALK);
        assert_eq!(wanted_animation(&moving(7.6), 0, None), anim::RUN);
        // A creature the server turns on the spot has a spline whose destination
        // it already stands on. It plays Stand, not a shuffle.
        assert_eq!(wanted_animation(&moving(0.02), 0, None), anim::STAND);
    }

    /// Feign Death is drawn as death, but the unit is not dead.
    ///
    /// The two states are one bit apart on the wire, and the 1.12.1 client draws
    /// either one as death, so the pose treats them alike. Nothing else follows the
    /// pose. The health did not change, so every other reader in this client (the
    /// release box, the target frame, the corpse marker) sees a feigning hunter as
    /// alive. That is why `feigning` and `dead` are separate fields.
    #[test]
    fn a_feigning_body_plays_the_death_clip_and_stays_alive() {
        let mut feigning = moving(0.0);
        feigning.feigning = true;
        assert_eq!(wanted_animation(&feigning, 0, None), anim::DEATH);
        assert!(!feigning.dead, "the health never moved");

        // Feigning outranks every lower state, as death does. A hunter who feigns
        // mid-run lies on the floor.
        let mut running = moving(7.6);
        running.feigning = true;
        assert_eq!(wanted_animation(&running, 0, None), anim::DEATH);

        // When the flag clears, the unit returns to its gait through an ordinary
        // change of state. No state is kept from the feign.
        let mut up = moving(7.6);
        up.feigning = false;
        assert_eq!(wanted_animation(&up, 0, None), anim::RUN);
    }

    /// Stealth is the creep bit in the fourth byte of `UNIT_FIELD_BYTES_1`, and it
    /// selects two animations: `StealthWalk` when moving and `StealthStand` when
    /// idle. No spell id, aura or visual kit is involved.
    ///
    /// Before this, rogue stealth played the ordinary idle and walk, because the
    /// fourth byte of `UNIT_FIELD_BYTES_1` was not read.
    ///
    /// The test also checks precedence, because in the 1.12.1 client the states on
    /// either side of the creep state can occur while stealthed. A stealthed
    /// character backing away plays `Walkbackwards` (backward outranks creep). A
    /// stealthed swimmer swims (water outranks creep, moving or idle). A stealthed
    /// character turning on the spot holds the crouch rather than shuffling.
    #[test]
    fn a_stealthed_unit_creeps_and_crouches() {
        use vale_protocol::state::objects::UNIT_VIS_FLAGS_CREEP;

        let mut creeping = moving(2.0);
        creeping.vis_flags = UNIT_VIS_FLAGS_CREEP;
        assert_eq!(wanted_animation(&creeping, 0, None), anim::STEALTH_WALK);

        // The creep holds at any speed. `AnimationData.dbc` has no `StealthRun`,
        // and the 1.12.1 client ignores speed while stealthed, so creep outranks
        // both the Run/Walk split and the Sprint threshold.
        let mut hurrying = moving(20.0);
        hurrying.vis_flags = UNIT_VIS_FLAGS_CREEP;
        assert_eq!(wanted_animation(&hurrying, 0, None), anim::STEALTH_WALK);

        // Standing still: the idle stealth animation.
        let mut still = moving(0.0);
        still.vis_flags = UNIT_VIS_FLAGS_CREEP;
        still.moving = false;
        assert_eq!(wanted_animation(&still, 0, None), anim::STEALTH_STAND);

        // The crouch holds through a turn on the spot, which ranks immediately
        // below creep.
        let mut turning = still.clone();
        turning.move_flags = move_flags::TURN_LEFT;
        assert_eq!(wanted_animation(&turning, 0, None), anim::STEALTH_STAND);

        // Creep ranks below the reverse gait, as in the 1.12.1 client.
        let mut backing = creeping.clone();
        backing.move_flags = move_flags::BACKWARD;
        assert_eq!(wanted_animation(&backing, 0, None), anim::WALK_BACKWARDS);

        // Creep also ranks below the water, which outranks both the moving and the
        // idle creep.
        let mut swimming = creeping.clone();
        swimming.swimming = true;
        assert_eq!(wanted_animation(&swimming, 0, None), anim::SWIM);

        // Only the creep bit counts. Ghost and untrackable share the byte and
        // neither selects a pose.
        let mut ghost = moving(2.0);
        ghost.vis_flags = vale_protocol::state::objects::UNIT_VIS_FLAGS_GHOST
            | vale_protocol::state::objects::UNIT_VIS_FLAGS_UNTRACKABLE;
        assert_eq!(wanted_animation(&ghost, 0, None), anim::WALK);
    }

    /// The all-out run is chosen by speed, not by spell, so one rule covers Charge,
    /// Sprint and every other speed increase.
    ///
    /// The 1.12.1 client's threshold is 11.0 y/s. Three speeds bound it: the 1.12
    /// base run is 7.0, Aspect of the Cheetah's +30% is 9.1 and does not sprint,
    /// and Sprint's +70% is 11.9 and does.
    #[test]
    fn past_eleven_yards_a_second_the_gait_is_the_all_out_run() {
        assert_eq!(wanted_animation(&moving(7.0), 0, None), anim::RUN);
        assert_eq!(wanted_animation(&moving(9.1), 0, None), anim::RUN);
        assert_eq!(wanted_animation(&moving(11.9), 0, None), anim::SPRINT);
        // The comparison is `>=`: in the 1.12.1 client a speed of exactly 11.0
        // sprints.
        assert_eq!(wanted_animation(&moving(anim::SPRINT_SPEED), 0, None), anim::SPRINT);

        // Moving backward plays `WalkBackwards` at any speed. Backward outranks
        // sprint, as it outranks creep.
        let mut fast_backwards = moving(20.0);
        fast_backwards.move_flags = move_flags::BACKWARD;
        assert_eq!(wanted_animation(&fast_backwards, 0, None), anim::WALK_BACKWARDS);
    }

    /// A unit on a flying spline plays `Fly`, for both the rider and the mount.
    ///
    /// At a taxi's 32 y/s, `Run` (authored at 6.9 y/s) plays at 4.6x through
    /// [`vale_assets::world::m2::M2Skeleton::playback_rate`], and the gryphon's wings
    /// blur. The gryphon's `Fly` is authored at 30 y/s, so it plays at 1.07x. The
    /// fix is to choose `Fly`, not to cap the rate.
    ///
    /// The rider and the mount are asserted separately because two functions
    /// choose them ([`crate::world::entities::pose::wanted_animation`] and [`crate::world::entities::mount::gait`]).
    /// The mount's was the one drawn wrong.
    #[test]
    fn a_flying_spline_plays_fly_rather_than_a_run_at_four_times_the_rate() {
        let mut flying = moving(32.0);
        flying.move_flags |= move_flags::FLYING;
        assert_eq!(wanted_animation(&flying, 0, None), anim::FLY);
        assert_eq!(crate::world::entities::mount::gait(&flying), anim::FLY);

        // `Fly` outranks the ground gait and does not need `moving`. A unit on a
        // spline does not hover still, and the flags carry FORWARD throughout.
        let mut hovering = moving(0.0);
        hovering.move_flags |= move_flags::FLYING;
        assert_eq!(wanted_animation(&hovering, 0, None), anim::FLY);

        // Without the flag the same speed gets the ground gait, which is `Sprint`
        // because 32 y/s is above the 11.0 threshold. Both functions agree,
        // because a galloping mount uses the same threshold. Neither returns `Fly`.
        assert_eq!(wanted_animation(&moving(32.0), 0, None), anim::SPRINT);
        assert_eq!(crate::world::entities::mount::gait(&moving(32.0)), anim::SPRINT);
    }

    /// The direction of travel is the third input to a gait, and the ground and the
    /// water rank directions differently.
    ///
    /// `wanted_animation` once read only `moving` and `speed`, so every direction
    /// of travel resolved to Walk or Run. A character backing out of a fight ran
    /// forwards while sliding backwards, and a side-stroking swimmer did the crawl
    /// sideways.
    ///
    /// The two surfaces rank directions differently, so this reads movement flags
    /// and not a single direction. On the ground, backward outranks a strafe. In
    /// the water, a strafe outranks backward and a turn outranks both. A four-way
    /// enum applied the ground's order in both places, and a side-stroking swimmer
    /// holding S did the backstroke.
    ///
    /// The ground has no sideways gait. `RunLeft`/`RunRight` are rows that 0 of 411
    /// models carry, and the Shuffles are the turn-in-place foot-shuffle tested
    /// below, not a strafe. A strafing character runs; `crate::world::facing`
    /// turns the drawn body toward the direction of the slide.
    #[test]
    fn the_gait_cascade_is_the_clients_own_and_differs_between_ground_and_water() {
        use vale_protocol::state::movement::move_flags;
        let ground = |flags, speed| {
            let mut e = moving(speed);
            e.move_flags = flags;
            wanted_animation(&e, 0, None)
        };
        let water = |flags, speed| {
            let mut e = moving(speed);
            e.move_flags = flags;
            e.swimming = true;
            wanted_animation(&e, 0, None)
        };

        assert_eq!(ground(move_flags::FORWARD, 7.6), anim::RUN);
        assert_eq!(ground(move_flags::FORWARD, 2.5), anim::WALK);
        // One sequence at any speed: the wire has a `run_back` speed but no model
        // has a `RunBackwards`.
        assert_eq!(ground(move_flags::BACKWARD, 7.6), anim::WALK_BACKWARDS);
        assert_eq!(ground(move_flags::BACKWARD, 2.5), anim::WALK_BACKWARDS);
        // A strafe runs. A strafing backpedal walks backwards, because backward
        // outranks a strafe on the ground.
        assert_eq!(ground(move_flags::STRAFE_LEFT, 7.6), anim::RUN);
        assert_eq!(ground(move_flags::STRAFE_RIGHT, 2.5), anim::WALK);
        assert_eq!(
            ground(move_flags::BACKWARD | move_flags::STRAFE_LEFT, 7.6),
            anim::WALK_BACKWARDS
        );

        assert_eq!(water(move_flags::FORWARD, 4.7), anim::SWIM);
        assert_eq!(water(move_flags::BACKWARD, 2.5), anim::SWIM_BACKWARDS);
        assert_eq!(water(move_flags::STRAFE_LEFT, 2.5), anim::SWIM_LEFT);
        assert_eq!(water(move_flags::STRAFE_RIGHT, 2.5), anim::SWIM_RIGHT);
        // Two cases where the water's order differs from the ground's. A strafing
        // backpedal side-strokes, and a turning swimmer treads water whatever else
        // is held.
        assert_eq!(
            water(move_flags::BACKWARD | move_flags::STRAFE_LEFT, 2.5),
            anim::SWIM_LEFT
        );
        assert_eq!(
            water(move_flags::FORWARD | move_flags::TURN_LEFT, 4.7),
            anim::SWIM_IDLE
        );
        // A swimmer who stops treads water whichever way they were going.
        assert_eq!(water(move_flags::BACKWARD, 0.0), anim::SWIM_IDLE);
    }

    /// The foot-shuffle is turning on the spot, not strafing. `ShuffleLeft` (11)
    /// and `ShuffleRight` (12) were once used for the strafe. They are the small
    /// step a standing character takes as its body turns, and the 1.12.1 client
    /// plays them only when the unit is not otherwise moving. A turn while
    /// travelling curves the run path and keeps the gait.
    #[test]
    fn the_shuffle_is_the_turn_in_place_and_only_when_standing() {
        use vale_protocol::state::movement::move_flags;
        let standing = |flags| {
            let mut e = moving(0.0);
            e.move_flags = flags;
            wanted_animation(&e, 0, None)
        };
        assert_eq!(standing(move_flags::TURN_LEFT), anim::SHUFFLE_LEFT);
        assert_eq!(standing(move_flags::TURN_RIGHT), anim::SHUFFLE_RIGHT);
        assert_eq!(standing(move_flags::NONE), anim::STAND);

        // Turning while running is still running.
        let mut runner = moving(7.6);
        runner.move_flags = move_flags::FORWARD | move_flags::TURN_LEFT;
        assert_eq!(wanted_animation(&runner, 0, None), anim::RUN);

        // A model with no shuffle plays Stand rather than a gait, because it is not
        // travelling.
        assert_eq!(
            fallbacks(anim::SHUFFLE_LEFT),
            &[anim::SHUFFLE_LEFT, anim::STAND]
        );
    }

    /// The take-off and landing of a jump are one-shots, and a model without them
    /// plays nothing in their place.
    ///
    /// A one-shot is held for the length of the sequence it resolved to. If
    /// `JumpEnd` fell back to Stand, as every other chain in [`fallbacks`] does, a
    /// landing wolf would hold its idle for the 2.6 s of that idle while it ran on.
    /// So `JumpStart`, `JumpEnd` and `JumpLandRun` have no fallback chain.
    #[test]
    fn the_jump_has_a_take_off_and_a_landing_and_neither_is_substituted() {
        let air = |jumping| {
            let mut e = moving(0.0);
            e.airborne = true;
            e.jumping = jumping;
            e
        };

        // The middle of the arc is a state: `Jump` for a jump, `Fall` for a fall.
        assert_eq!(wanted_animation(&air(true), 0, None), anim::JUMP);
        assert_eq!(wanted_animation(&air(false), 0, None), anim::FALL);

        let full = [
            anim::STAND,
            anim::RUN,
            anim::JUMP,
            anim::JUMP_START,
            anim::JUMP_END,
            anim::JUMP_LAND_RUN,
        ];
        let mut play = playing(&full);
        let ground = moving(0.0);

        // The first reading only records. An entity that streams into view already
        // in mid-air was not seen jumping, so the first reading fires nothing.
        note(&mut play, &air(true), anim::JUMP, 0.0);
        assert_eq!(play.oneshot.as_ref().map(|s| s.wanted), None);

        // A jump from the ground. The entity lands first, so the jump is an edge
        // rather than the same state read twice.
        note(&mut play, &ground, anim::STAND, 0.05);
        play.oneshot = None;
        note(&mut play, &air(true), anim::JUMP, 0.1);
        assert_eq!(
            play.oneshot.as_ref().map(|s| s.wanted),
            Some(anim::JUMP_START)
        );

        // The landing. No key is held, so it is the standing landing rather than
        // the running one. The landing reads the keys (`landing_for`), and
        // `moving` holds `FORWARD` even at speed 0, so this uses `standing()`.
        note(&mut play, &standing(), anim::STAND, 1.0);
        assert_eq!(
            play.oneshot.as_ref().map(|s| s.wanted),
            Some(anim::JUMP_END)
        );

        // Landing while running plays `JumpLandRun`.
        let mut running = playing(&full);
        note(&mut running, &air(true), anim::JUMP, 0.0);
        note(&mut running, &ground, anim::RUN, 0.5);
        assert_eq!(
            running.oneshot.as_ref().map(|s| s.wanted),
            Some(anim::JUMP_LAND_RUN)
        );

        // A step off a ledge has no take-off. Only a jump carries the
        // `MSG_MOVE_JUMP` `zspeed` that marks a push off the ground.
        let mut stepped = playing(&full);
        note(&mut stepped, &ground, anim::STAND, 0.0);
        note(&mut stepped, &air(false), anim::FALL, 0.1);
        assert_eq!(stepped.oneshot.as_ref().map(|s| s.wanted), None);

        // A model carrying none of the three fires none of them, rather than
        // holding a substituted idle.
        let mut wolf = playing(&[anim::STAND, anim::RUN]);
        note(&mut wolf, &air(true), anim::JUMP, 0.0);
        note(&mut wolf, &air(true), anim::JUMP, 0.1);
        assert!(
            wolf.oneshot.is_none(),
            "a model without JumpStart fired one anyway"
        );
        note(&mut wolf, &ground, anim::RUN, 0.5);
        assert!(
            wolf.oneshot.is_none(),
            "a landing was substituted onto the idle"
        );
    }

    /// A whole jump through the pose code, frame by frame. This covers the
    /// animation of the arc;
    /// `crate::world::predict::tests::a_jump_is_drawn_as_one_arc_from_frame_to_frame`
    /// covers the position.
    ///
    /// The reported bug was "the character jitters or resets mid-air, and then
    /// replays the second half of the jump after landing". A single-frame check
    /// cannot detect either, because both concern the sequence of frames. This
    /// test runs at 60 Hz across an 820 ms arc and records which clip plays on
    /// every frame.
    #[test]
    fn a_jump_plays_one_take_off_and_one_landing() {
        const FRAME: f32 = 1.0 / 60.0;
        // The measured lengths on `HumanMale`: JumpStart 833 ms, Jump 1000,
        // JumpEnd 1000, Fall 1000, JumpLandRun 833. `skeleton` gives every id a
        // 1000 ms window, so the take-off here is longer than `HumanMale`'s. That
        // is the harder case, because the one-shot outlives the arc.
        let full = [
            anim::STAND,
            anim::RUN,
            anim::JUMP,
            anim::JUMP_START,
            anim::JUMP_END,
            anim::FALL,
            anim::JUMP_LAND_RUN,
        ];
        let mut play = playing(&full);
        // `standing()` rather than `moving(0.0)`: the landing is chosen from the
        // held keys (`landing_for`), and `moving` holds `FORWARD` even at 0.
        let ground = standing();
        let air = |jumping| {
            let mut e = standing();
            e.airborne = true;
            e.jumping = jumping;
            e
        };

        // Standing on the ground, so the take-off below is an edge.
        note(&mut play, &ground, anim::STAND, 0.0);
        play.advance(anim::STAND, 0.0);

        let mut clips: Vec<(f32, u16)> = Vec::new();
        let airborne_until = 0.82;
        let mut frame = 1;
        loop {
            let now = frame as f32 * FRAME;
            if now > 2.0 {
                break;
            }
            let up = now < airborne_until;
            let world = if up { air(true) } else { ground.clone() };
            let state = if up { anim::JUMP } else { anim::STAND };
            note(&mut play, &world, state, now);
            play.advance(state, now);
            clips.push((now, play.wanted));
            frame += 1;
        }

        // One take-off and one landing. A clip is counted each time it starts, so
        // a second run of the same id is a second entry.
        let mut runs: Vec<u16> = Vec::new();
        for (_, id) in &clips {
            if runs.last() != Some(id) {
                runs.push(*id);
            }
        }
        assert_eq!(
            runs.iter().filter(|id| **id == anim::JUMP_START).count(),
            1,
            "the take-off played more than once: {runs:?}"
        );
        assert_eq!(
            runs.iter().filter(|id| **id == anim::JUMP_END).count(),
            1,
            "the landing played more than once: {runs:?}"
        );
        // The landing ends within a second of standing, rather than playing on
        // after its arc.
        assert_eq!(
            clips.last().map(|(_, id)| *id),
            Some(anim::STAND),
            "still playing {runs:?} a second after landing"
        );
        // No clip returns to an earlier stage of the arc. The ids must appear in
        // the arc's order. A repeat of a stage already left is the "replays the
        // second half" symptom.
        let order = [anim::STAND, anim::JUMP_START, anim::JUMP, anim::JUMP_END, anim::STAND];
        let mut at = 0;
        for id in &runs {
            let Some(next) = order[at..].iter().position(|want| want == id) else {
                panic!("the arc went backwards to {id}: {runs:?}");
            };
            at += next;
        }
    }

    /// The landing animation is chosen from the held keys, in the four cases the
    /// 1.12.1 client distinguishes.
    ///
    /// The landing was once chosen from the resolved gait, using a list that
    /// missed `SPRINT`, so an epic mount (14 y/s, above `SPRINT_SPEED`'s 11) landed
    /// standing and slid. The rule now matches the 1.12.1 client and reads the
    /// movement flags, so there is no list to go out of date. The sweep below checks
    /// every combination of held keys against the four cases.
    #[test]
    fn the_landing_is_chosen_off_the_held_keys() {
        use crate::world::entities::pose::landing_for;
        use vale_protocol::state::movement::move_flags as f;
        // Nothing held: the standing landing.
        assert_eq!(landing_for(0), Some(anim::JUMP_END));
        assert_eq!(landing_for(f::TURN_LEFT), Some(anim::JUMP_END), "turning is not a direction");
        // Forward, or a strafe: the running one.
        assert_eq!(landing_for(f::FORWARD), Some(anim::JUMP_LAND_RUN));
        assert_eq!(landing_for(f::STRAFE_LEFT), Some(anim::JUMP_LAND_RUN));
        assert_eq!(landing_for(f::STRAFE_RIGHT | f::FORWARD), Some(anim::JUMP_LAND_RUN));
        // Backpedalling, or walking: no landing clip; the gait plays.
        assert_eq!(landing_for(f::BACKWARD), None);
        assert_eq!(landing_for(f::FORWARD | f::WALK_MODE), None);

        // Sweep every combination of the four direction bits, walk mode and the two
        // turns, against the 1.12.1 client's rule: no direction bit gives
        // `JumpEnd`, backward or walk mode gives no clip, and anything else gives
        // `JumpLandRun`.
        for bits in 0..128u32 {
            let flags = (bits & 0xf)
                | if bits & 0x10 != 0 { f::WALK_MODE } else { 0 }
                | if bits & 0x20 != 0 { f::TURN_LEFT } else { 0 }
                | if bits & 0x40 != 0 { f::TURN_RIGHT } else { 0 };
            let expected = if flags & 0xf == 0 {
                Some(anim::JUMP_END)
            } else if flags & (f::BACKWARD | f::WALK_MODE) != 0 {
                None
            } else {
                Some(anim::JUMP_LAND_RUN)
            };
            assert_eq!(landing_for(flags), expected, "flags {flags:#x}");
        }
    }

    /// A finished one-shot fades out from its last frame, not its first.
    ///
    /// `jump_pose_continuity` measured the fault. The fade sampled the finished
    /// clip at its wrapped clock, which is zero. At the end of `JumpStart`,
    /// mid-air, the pose blended from the crouch on the ground, a 2-yard bone move
    /// in one frame. At the end of `JumpLandRun`, on the ground, it blended from
    /// the airborne first frame. This wrap caused both "resets mid-air" and
    /// "replays the second half after landing".
    #[test]
    fn a_finished_one_shot_fades_out_of_its_last_frame() {
        const FRAME: f32 = 1.0 / 60.0;
        let full = [anim::STAND, anim::RUN, anim::JUMP, anim::JUMP_START, anim::JUMP_END, anim::JUMP_LAND_RUN];
        let mut play = playing(&full);
        let mut air = standing();
        air.airborne = true;
        air.jumping = true;
        note(&mut play, &standing(), anim::STAND, 0.0);
        play.advance(anim::STAND, 0.0);
        note(&mut play, &air, anim::JUMP, FRAME);
        play.advance(anim::JUMP, FRAME);
        let until = play.oneshot.as_ref().expect("the take-off fired").until;
        // The take-off ends while still in the air. The loop takes over, and the
        // take-off fades out from its end.
        let after = until + FRAME;
        note(&mut play, &air, anim::JUMP, after);
        play.advance(anim::JUMP, after);
        let fade = play.fade.as_ref().expect("a fade into the loop");
        assert_eq!(fade.held, Some(until), "the finished one-shot is not held");
        let length = {
            let seq = &play.skeleton.sequences[fade.sequence];
            seq.end - seq.start
        };
        assert_eq!(play.fade_phase(fade, after), length - 1, "sampled at its last frame");
        assert_ne!(
            play.skeleton.phase(fade.sequence, play.clock(fade.sequence, after - fade.since)),
            length - 1,
            "the wrapped clock would have said something else, which is the bug"
        );
        // A loop keeps its own clock, so a run fading into a stand fades from
        // mid-stride.
        let mut play = playing(&full);
        note(&mut play, &moving(7.0), anim::RUN, 0.0);
        play.advance(anim::RUN, 0.0);
        note(&mut play, &standing(), anim::STAND, 0.3);
        play.advance(anim::STAND, 0.3);
        let fade = play.fade.as_ref().expect("a fade into the stand");
        assert_eq!(fade.held, None);
        assert_eq!(play.fade_phase(fade, 0.3), 300, "a loop's own clock, unwrapped inside its length");
    }

    /// A model with no running landing goes straight to its gait on touchdown. Every
    /// mount is such a model: `Horse`, `Ram`, `Wolf`, `Tiger`, `MechaStrider` and
    /// `UndeadHorse` carry `JumpStart`/`Jump`/`JumpEnd`/`Fall` and none carries
    /// `JumpLandRun` (187).
    ///
    /// The reported bug was "on a quick landing the mount floats briefly". An arc
    /// shorter than the 833 ms take-off clip landed while the one-shot was still
    /// playing, the landing resolved to nothing, and the horse kept playing its
    /// launch pose on the ground for the rest of the clip. The 1.12.1 client
    /// resolves 187 through the fallback column of `AnimationData.dbc`, which reads
    /// 5 (`Run`), so the landing is the gait, at once.
    #[test]
    fn a_mount_with_no_running_landing_goes_straight_to_its_gait() {
        const FRAME: f32 = 1.0 / 60.0;
        let mount = [anim::STAND, anim::RUN, anim::JUMP_START, anim::JUMP, anim::JUMP_END, anim::FALL];
        let mut play = playing(&mount);
        let mut running = moving(7.0);
        running.mounted = true;
        let mut air = running.clone();
        air.airborne = true;
        air.jumping = true;

        // On the ground and running, then a take-off.
        play.note_flight_only(&running, anim::RUN, 0.0);
        play.advance(anim::RUN, 0.0);
        play.note_flight_only(&air, anim::JUMP, FRAME);
        play.advance(anim::JUMP, FRAME);
        assert_eq!(play.wanted, anim::JUMP_START, "the take-off fired");

        // A short arc: down again at 400 ms, well inside the take-off clip's 1000 ms
        // (`skeleton` gives every id a second).
        let landed = 0.4;
        play.note_flight_only(&running, anim::RUN, landed);
        play.advance(anim::RUN, landed);
        assert_eq!(
            play.wanted,
            anim::RUN,
            "the take-off clip outlived the arc: the mount is floating"
        );
        // A model that has the clip plays it.
        let character = [anim::STAND, anim::RUN, anim::JUMP_START, anim::JUMP, anim::JUMP_END, anim::JUMP_LAND_RUN];
        let mut play = playing(&character);
        let ground = moving(7.0);
        let mut air = ground.clone();
        air.airborne = true;
        air.jumping = true;
        note(&mut play, &ground, anim::RUN, 0.0);
        play.advance(anim::RUN, 0.0);
        note(&mut play, &air, anim::JUMP, FRAME);
        play.advance(anim::JUMP, FRAME);
        note(&mut play, &ground, anim::RUN, landed);
        play.advance(anim::RUN, landed);
        assert_eq!(play.wanted, anim::JUMP_LAND_RUN);
    }

    /// A rider plays neither the take-off nor the landing, because it does not play
    /// the middle of the jump either. The mount plays all three.
    ///
    /// `wanted_animation` ranks `Mount` (91) above the airborne states, so a
    /// mounted character sits still for the whole jump and the animal underneath
    /// plays it. The landing had no condition, so a mounted jump ended with the
    /// rider playing a full-body `JumpEnd`, standing up out of the saddle. The
    /// take-off was already correct because it requires `state == JUMP`, which a
    /// mounted unit never has.
    ///
    /// The mount's own [`Playback`] reaches the take-off and landing through
    /// `note_flight_only`, with the same mounted `WorldEntity`. That is why the
    /// mounted check is made by the caller rather than inside `note_flight`.
    #[test]
    fn a_rider_fires_no_landing_and_its_mount_fires_both_ends() {
        let full = [
            anim::STAND,
            anim::RUN,
            anim::MOUNT,
            anim::JUMP,
            anim::JUMP_START,
            anim::JUMP_END,
            anim::JUMP_LAND_RUN,
        ];
        // `standing()`: no keys held, so the landing is `JumpEnd`. See
        // `landing_for`, which reads the movement flags.
        let mounted = |airborne| {
            let mut e = standing();
            e.mounted = true;
            e.airborne = airborne;
            e.jumping = airborne;
            e
        };

        // The rider: `Mount` is the state for the whole jump.
        assert_eq!(wanted_animation(&mounted(true), 0, None), anim::MOUNT);
        assert_eq!(wanted_animation(&mounted(false), 0, None), anim::MOUNT);

        let mut rider = playing(&full);
        note(&mut rider, &mounted(false), anim::MOUNT, 0.0);
        note(&mut rider, &mounted(true), anim::MOUNT, 0.1);
        assert!(rider.oneshot.is_none(), "a rider does not push off");
        note(&mut rider, &mounted(false), anim::MOUNT, 1.0);
        assert!(
            rider.oneshot.is_none(),
            "a rider stood out of the saddle to land"
        );

        // The mount: the same two edges, with the gait as the state. Both ends
        // fire, because the mount's model plays the middle of the jump.
        let mut horse = playing(&full);
        horse.note_flight_only(&mounted(false), anim::STAND, 0.0);
        horse.note_flight_only(&mounted(true), anim::JUMP, 0.1);
        assert_eq!(
            horse.oneshot.as_ref().map(|s| s.wanted),
            Some(anim::JUMP_START)
        );
        horse.note_flight_only(&mounted(false), anim::STAND, 1.0);
        assert_eq!(
            horse.oneshot.as_ref().map(|s| s.wanted),
            Some(anim::JUMP_END)
        );

        // An unmounted character still plays both ends.
        let mut afoot = playing(&full);
        let air = |airborne| {
            let mut e = standing();
            e.airborne = airborne;
            e.jumping = airborne;
            e
        };
        note(&mut afoot, &air(false), anim::STAND, 0.0);
        note(&mut afoot, &air(true), anim::JUMP, 0.1);
        assert_eq!(
            afoot.oneshot.as_ref().map(|s| s.wanted),
            Some(anim::JUMP_START)
        );
        note(&mut afoot, &air(false), anim::STAND, 1.0);
        assert_eq!(
            afoot.oneshot.as_ref().map(|s| s.wanted),
            Some(anim::JUMP_END)
        );
    }

    /// A game object has four animations and none of them is Stand.
    ///
    /// `Chest02.m2` carries exactly `Close` (146), `Closed` (147), `Open` (148) and
    /// `Opened` (149). The gait code once asked for Stand, the model had none, the
    /// id had no fallback chain, and `take_up` substituted nothing, so the sequence
    /// stayed unset and every chest and door was drawn in its bind pose.
    /// `GAMEOBJECT_STATE` is the only field the server sends about a game object's
    /// appearance. Zero is the open state, so an omitted field reads as open.
    #[test]
    fn a_game_object_stands_open_or_shut_rather_than_standing() {
        let object = |state: u8| {
            let mut e = moving(0.0);
            e.kind = ObjectType::GameObject;
            e.object_state = Some(state);
            e
        };

        assert_eq!(wanted_animation(&object(1), 0, None), anim::CLOSED);
        // 0 is `GO_STATE_ACTIVE` and 2 the alternative used state; both are open.
        assert_eq!(wanted_animation(&object(0), 0, None), anim::OPENED);
        assert_eq!(wanted_animation(&object(2), 0, None), anim::OPENED);

        // A game object with no state field is open. Every case above passes a
        // `Some`; this one passes `None`. `Object::_SetCreateBits` omits a field
        // whose value is zero, so an open door sends no state. While the check
        // required `Some`, such objects fell through to the unit rules, matched no
        // sequence, and were drawn in their bind pose. `DEADMINEDOOR01.m2` has 28
        // bones whose bind pose is neither state, so it showed both leaves half
        // open.
        let mut silent = object(0);
        silent.object_state = None;
        assert_eq!(wanted_animation(&silent, 0, None), anim::OPENED);
        // A game object with no state field still outranks the unit rules. An
        // unset sequence looks like the model's art rather than a fault, so this
        // failure was hard to see.
        let mut silent_moving = silent.clone();
        silent_moving.moving = true;
        silent_moving.speed = 7.0;
        assert_eq!(wanted_animation(&silent_moving, 0, None), anim::OPENED);
        // The game-object state outranks every unit rule, because none of those
        // rules applies to a game object.
        let mut in_combat = object(1);
        in_combat.in_combat = true;
        in_combat.stand_state = STAND_STATE_SIT;
        assert_eq!(wanted_animation(&in_combat, 0, None), anim::CLOSED);

        // A unit is unaffected: no state, and the ordinary rules.
        assert_eq!(wanted_animation(&moving(0.0), 0, None), anim::STAND);

        // The transition between the two states is a one-shot, like the take-off
        // and landing of a jump.
        let chest = [anim::CLOSE, anim::CLOSED, anim::OPEN, anim::OPENED];
        let mut play = playing(&chest);
        note(&mut play, &object(1), anim::CLOSED, 0.0);
        assert!(play.oneshot.is_none(), "the first look fired an animation");
        note(&mut play, &object(0), anim::OPENED, 0.5);
        assert_eq!(play.oneshot.as_ref().map(|s| s.wanted), Some(anim::OPEN));
        play.oneshot = None;
        note(&mut play, &object(1), anim::CLOSED, 3.0);
        assert_eq!(play.oneshot.as_ref().map(|s| s.wanted), Some(anim::CLOSE));

        // A custom animation from the server is a one-shot on the object's own
        // model: the bobber's `Custom0` (153) when a fish bites.
        let bobber_anims = [anim::STAND, vale_assets::look::object::CUSTOM0_ANIM];
        let mut bobber = playing(&bobber_anims);
        let mut float = object(1);
        note(&mut bobber, &float, anim::STAND, 0.0);
        float.object_anims = 1;
        float.object_anim = Some(vale_protocol::play::object::ObjectAnim::Custom(0));
        note(&mut bobber, &float, anim::STAND, 1.0);
        assert_eq!(
            bobber.oneshot.as_ref().map(|s| s.wanted),
            Some(vale_assets::look::object::CUSTOM0_ANIM)
        );

        // A plain prop model, such as a banner or a brazier, has none of the four
        // and falls back to its own idle rather than to nothing.
        let mut banner = playing(&[anim::STAND]);
        banner.advance(wanted_animation(&object(1), 0, None), 0.0);
        assert_eq!(banner.skeleton.sequences[banner.sequence].id, anim::STAND);
    }

    /// A corpse plays Death whatever else it was doing, and its clock is the one
    /// clock this client does not wrap.
    ///
    /// `AnimationData.dbc` has id 6 (`Dead`), but no 1.12 creature model carries a
    /// sequence for it, so nothing plays after Death. A wrapped clock plays Death
    /// again, and the body falls over repeatedly. `M2Skeleton::pose` clamps to the
    /// window, so an unwrapped clock holds the last frame; every other animation
    /// wraps its clock.
    ///
    /// The assertion checks whether the clock wrapped, not a millisecond count.
    /// This test once required `min(elapsed, length)` (500 on the fixture) and
    /// passed while every corpse stood back up, because the sampler wrapped that
    /// number and `500 % 500` is frame zero of Death. A test that checks only the
    /// value one layer passes to the next does not check what the next layer does
    /// with it. See `m2::tests::a_clock_past_the_end_holds_the_last_frame_rather_than_wrapping`.
    #[test]
    fn a_corpse_holds_the_last_frame_of_death() {
        let mut dead = moving(7.0);
        dead.dead = true;
        assert_eq!(
            wanted_animation(&dead, 0, None),
            anim::DEATH,
            "a corpse was still running"
        );

        // The fixture's sequences are 500 ms long. The unit stands first, because
        // only a creature that dies in view plays the clip; a corpse that streamed
        // in already dead does not. See
        // `a_corpse_that_was_already_dead_on_arrival_does_not_fall_over`.
        let mut play = playing(&[anim::STAND, anim::DEATH]);
        play.advance(anim::STAND, 0.0);
        play.advance(anim::DEATH, 0.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::DEATH);
        assert_eq!(play.advance(anim::DEATH, 0.2), 200);
        assert_eq!(
            play.advance(anim::DEATH, 9.0),
            9000,
            "a corpse's clock was wrapped, which restarts Death"
        );

        // An idle's clock is wrapped. If it were held, every standing creature
        // would freeze on the last frame of its idle a second after it stopped
        // moving.
        let mut alive = playing(&[anim::STAND, anim::DEATH]);
        alive.advance(anim::STAND, 0.0);
        assert_eq!(alive.advance(anim::STAND, 9.0), 9000 % 500);
    }

    /// A body that was already dead when first seen does not fall over as the tile
    /// loads.
    ///
    /// This is part of the "dead units that died outside our presence" report. Once
    /// such a corpse is read as dead, playing Death from the start makes every body
    /// in a cleared room stand up and fall as the player walks in. Death is the
    /// only state whose clip is a transition into the state rather than the state
    /// itself, so the first take-up of Death starts at its end.
    #[test]
    fn a_corpse_that_was_already_dead_on_arrival_does_not_fall_over() {
        // Nothing has ever played on this rig: it streamed in as a corpse.
        let mut play = playing(&[anim::STAND, anim::DEATH]);
        // The fixture's sequences are 500 ms; past the end is the last frame,
        // because `M2Skeleton::pose` clamps and a corpse's clock is left alone.
        assert!(
            play.advance(anim::DEATH, 0.0) >= 500,
            "a streamed-in corpse played its death clip from the top"
        );
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::DEATH);
        // It stays on the last frame, because the clock is not wrapped.
        assert!(play.advance(anim::DEATH, 9.0) >= 500);

        // The rule applies to Death only. A creature that streams in running starts
        // its gait at the start of the loop.
        let mut alive = playing(&[anim::STAND, anim::RUN]);
        assert_eq!(alive.advance(anim::RUN, 0.0), 0);
    }

    /// A model with no Death holds its idle rather than substituting something
    /// that reads as alive.
    #[test]
    fn a_model_without_death_falls_back_to_standing() {
        let mut play = playing(&[anim::STAND, anim::RUN]);
        play.advance(anim::DEATH, 0.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
    }

    /// One swing per counter increment, and none on the first poll.
    ///
    /// An entity that walks into view mid-fight arrives with a swing count of
    /// forty. Firing on the first reading would play a swing for a blow that
    /// landed before it was in view. Firing on the count rather than the difference
    /// would play forty.
    #[test]
    fn a_swing_fires_on_the_counter_moving_and_not_on_the_first_sight_of_it() {
        let mut play = playing(&[anim::STAND, anim::ATTACK_UNARMED]);
        let mut world = moving(0.0);
        world.swings_thrown = 40;

        note(&mut play, &world, anim::STAND, 0.0);
        play.advance(anim::STAND, 0.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::STAND,
            "it replayed a fight it had not seen"
        );

        world.swings_thrown = 41;
        note(&mut play, &world, anim::STAND, 1.0);
        let elapsed = play.advance(anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::ATTACK_UNARMED
        );
        assert_eq!(elapsed, 0, "the swing did not start at its own frame zero");

        // The swing runs for the sequence's own length (500 ms in the fixture),
        // and then the state's animation plays again.
        play.advance(anim::STAND, 1.4);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::ATTACK_UNARMED
        );
        play.advance(anim::STAND, 1.6);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
    }

    /// Two swings in a row play as two swings, and the first is not cut off.
    ///
    /// This is the 1.12.1 client's combat fast path. A swing started while another
    /// swing is playing does not replace it: the running swing plays at 2x and the
    /// new one waits. This keeps each swing of a fast weapon visible. Restarting
    /// the clip instead would cut the first blow mid-arc, and a third request would
    /// cut the second.
    #[test]
    fn a_second_swing_speeds_the_first_up_and_waits() {
        let mut play = playing(&[anim::STAND, anim::ATTACK_UNARMED]);
        let mut world = moving(0.0);
        note(&mut play, &world, anim::STAND, 0.0);

        // The fixture's swing is 500 ms. 200 ms in, at 1x.
        world.swings_thrown = 1;
        note(&mut play, &world, anim::STAND, 1.0);
        assert_eq!(play.advance(anim::STAND, 1.2), 200);

        // A second swing lands at 300 ms. The pose does not jump, because the phase
        // at this instant is unchanged. From here it advances twice as fast, so
        // the remaining 200 ms of clip take 100 ms of wall clock.
        world.swings_thrown = 2;
        note(&mut play, &world, anim::STAND, 1.3);
        // Within a millisecond either way, because the clock truncates a float.
        let near = |got: u32, want: u32, what: &str| {
            assert!(got.abs_diff(want) <= 1, "{what}: {got} against {want}");
        };
        near(play.advance(anim::STAND, 1.3), 300, "the pose jumped");
        near(
            play.advance(anim::STAND, 1.35),
            400,
            "the remainder is not 2x",
        );

        // The whole 500 ms clip plays, and it ends at 1.4 rather than 1.5: 300 ms
        // at 1x and the last 200 ms in 100 ms of wall clock.
        near(
            play.advance(anim::STAND, 1.399),
            498,
            "the sped-up clip did not finish",
        );
        assert_eq!(
            play.advance(anim::STAND, 1.4),
            0,
            "the swing outlived its own clip"
        );
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);

        // The waiting swing plays on the next poll, from its own frame zero.
        note(&mut play, &world, anim::STAND, 1.41);
        assert_eq!(
            play.advance(anim::STAND, 1.41),
            0,
            "the parked swing never played"
        );
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::ATTACK_UNARMED
        );
    }

    /// A flinch does not use the fast path, and it cuts a swing off.
    ///
    /// `combat_id` lists the animations the 1.12.1 client treats as combat
    /// animations: the swings, the parries, the block, the dodge and the two spell
    /// casts. `CombatWound` (9) is not one of them. So a blow landing mid-swing
    /// replaces the swing at once. A hit is shown when it happens; a second swing
    /// can wait.
    #[test]
    fn a_flinch_replaces_the_swing_rather_than_parking() {
        let mut play = playing(&[anim::STAND, anim::ATTACK_UNARMED, anim::COMBAT_WOUND]);
        let mut world = moving(0.0);
        note(&mut play, &world, anim::STAND, 0.0);

        world.swings_thrown = 1;
        note(&mut play, &world, anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::ATTACK_UNARMED
        );

        world.blows_taken = 1;
        note(&mut play, &world, anim::STAND, 1.2);
        assert_eq!(
            play.advance(anim::STAND, 1.2),
            0,
            "the flinch waited its turn"
        );
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::COMBAT_WOUND
        );
    }

    /// A corpse does not flinch at the blow that killed it, and does not swing.
    /// Either one-shot would stand the corpse back up.
    #[test]
    fn a_one_shot_is_refused_on_a_corpse() {
        let mut play = playing(&[anim::STAND, anim::DEATH, anim::COMBAT_WOUND]);
        let mut world = moving(0.0);
        world.dead = true;
        note(&mut play, &world, anim::DEATH, 0.0);

        world.blows_taken = 1;
        note(&mut play, &world, anim::DEATH, 1.0);
        play.advance(anim::DEATH, 1.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::DEATH);
    }

    /// A clip that was already running when the unit died is cut off, not played to
    /// its end.
    ///
    /// A one-shot expires on its own clock rather than on a change of state. Before
    /// this rule, a mob killed half way through a two-second swing stayed up
    /// swinging for the rest of it. That was the report "death animations are
    /// often delayed — the dead mob has to finish their current anim before
    /// dying".
    #[test]
    fn death_cuts_off_a_clip_that_is_already_running() {
        let mut play = playing(&[anim::STAND, anim::DEATH, anim::ATTACK_UNARMED]);
        let mut world = moving(0.0);
        note(&mut play, &world, anim::STAND, 0.0);

        // A swing, which is a one-shot held for the length of the clip.
        world.swings_thrown = 1;
        note(&mut play, &world, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::ATTACK_UNARMED
        );
        assert!(
            play.oneshot.is_some(),
            "the fixture's swing is not a one-shot"
        );

        // The killing blow lands mid-swing.
        world.dead = true;
        note(&mut play, &world, anim::DEATH, 1.2);
        play.advance(anim::DEATH, 1.2);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::DEATH,
            "the corpse finished its swing first"
        );
        assert!(play.oneshot.is_none());
        assert!(
            play.deferred.is_none(),
            "a parked swing would play over the corpse"
        );
    }

    /// A gait plays at a rate set by the unit's speed; no other animation does.
    ///
    /// A `Run` authored for 6.9 y/s and played at 1x by a character moving at 7.6
    /// slides its feet against the ground by 10% for as long as it moves, and a
    /// faster one by more. The rate comes from the sequence header's declared
    /// `move_speed`. That field is the authored speed, not root motion.
    #[test]
    fn a_gait_is_played_at_the_speed_the_unit_is_going_and_a_swing_is_not() {
        let mut skeleton = (*skeleton(&[anim::STAND, anim::RUN, anim::ATTACK_UNARMED])).clone();
        skeleton.sequences[1].move_speed = 2.5;
        skeleton.sequences[2].move_speed = 2.5; // declared, but not a gait id
        let mut play = Playback {
            hints: Vec::new(),
            window: None,
            skeleton: Arc::new(skeleton),
            ..playing(&[])
        };

        // Twice the authored speed, so twice the playback: a tenth of a second of
        // wall clock is 200 ms into the clip.
        let mut world = moving(5.0);
        note(&mut play, &world, anim::RUN, 0.0);
        play.advance(anim::RUN, 0.0);
        assert_eq!(play.advance(anim::RUN, 0.1), 200);

        // At the authored speed the rate is 1:1.
        world.speed = 2.5;
        note(&mut play, &world, anim::RUN, 0.0);
        assert_eq!(play.advance(anim::RUN, 0.1), 100);

        // A swing plays at 1x at any speed. `AttackUnarmed` is not a rate-scaled
        // id, so its one-shot runs the sequence's own 500 ms.
        world.speed = 5.0;
        world.swings_thrown = 1;
        note(&mut play, &world, anim::RUN, 1.0);
        assert_eq!(
            play.oneshot.as_ref().expect("a swing").until,
            1.5,
            "the swing was rate-scaled"
        );
    }

    /// A one-shot started while the legs are moving plays on the upper body over
    /// them. The same one-shot started while standing plays on the whole body.
    ///
    /// This fixes "cast animations are interrupted in a way WoW does not". With one
    /// animation track, the client must choose between the swing and the run, and
    /// choosing the swing stops a running character mid-stride. The route is chosen
    /// for each play from the current state; the id does not affect it. That is why
    /// the same `AttackUnarmed` appears twice here with two different routes.
    #[test]
    fn a_swing_masks_over_a_run_and_takes_the_whole_body_standing_still() {
        let ids = &[anim::STAND, anim::RUN, anim::ATTACK_UNARMED];

        // Running: the swing goes to the torso and the base keeps the gait.
        let mut play = playing_with_spine(ids);
        let mut world = moving(7.0);
        note(&mut play, &world, anim::RUN, 0.0);
        world.swings_thrown = 1;
        note(&mut play, &world, anim::RUN, 1.0);
        play.advance(anim::RUN, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::RUN,
            "the swing stopped the legs"
        );
        assert!(play.oneshot.is_none(), "the swing took the base track");
        let overlay = play.overlay.as_ref().expect("the torso is swinging");
        assert_eq!(
            play.skeleton.sequences[overlay.sequence].id,
            anim::ATTACK_UNARMED
        );
        assert!(!overlay.looping, "a swing is not held");

        // The swing ends on its own clock (500 ms in the fixture). The gait keeps
        // its original start time rather than restarting.
        play.advance(anim::RUN, 1.6);
        assert!(play.overlay.is_none(), "the shot outlived its sequence");
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);
        assert_eq!(play.since, 1.0, "the base clock was restarted");

        // Standing, the same swing plays on the whole body. A standing swing
        // lunges, and the clip's leg keys carry the lunge.
        let mut play = playing_with_spine(ids);
        let mut world = standing();
        note(&mut play, &world, anim::STAND, 0.0);
        world.swings_thrown = 1;
        note(&mut play, &world, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert!(play.overlay.is_none(), "a standing swing masked");
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::ATTACK_UNARMED
        );
    }

    /// A swing that began standing still moves to the upper body when the legs
    /// start moving.
    ///
    /// The route is chosen when the clip starts, from the state at that instant,
    /// and that state can change before the clip ends. Without the move, a
    /// character who swung and then ran kept the standing clip on the base track
    /// for its whole length and slid across the ground in it, which was the
    /// reported bug. `state_or_held` already applies the same rule to a held
    /// wind-up, so a cast begun standing was already handled correctly.
    ///
    /// The move keeps the clip's own clock, so the upper body does not change on
    /// the frame it happens.
    #[test]
    fn a_standing_swing_moves_to_the_torso_when_the_legs_start() {
        let ids = &[anim::STAND, anim::RUN, anim::ATTACK_UNARMED];
        let mut play = playing_with_spine(ids);

        // Standing: the swing plays on the whole body.
        let mut world = standing();
        note(&mut play, &world, anim::STAND, 0.0);
        world.swings_thrown = 1;
        note(&mut play, &world, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert!(play.overlay.is_none(), "a standing swing masked");
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::ATTACK_UNARMED
        );
        let clock = play.since;

        // The character starts running a fifth of the way through the 500 ms clip.
        let mut running = moving(7.0);
        running.swings_thrown = 1;
        note(&mut play, &running, anim::RUN, 1.1);
        play.advance(anim::RUN, 1.1);

        assert!(play.oneshot.is_none(), "the base track was not released");
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::RUN,
            "the legs never took the gait up — the character slides"
        );
        let overlay = play.overlay.as_ref().expect("the torso kept the swing");
        assert_eq!(
            play.skeleton.sequences[overlay.sequence].id,
            anim::ATTACK_UNARMED
        );
        assert!(!overlay.looping, "a swing is not a held wind-up");
        assert_eq!(overlay.since, clock, "the clip's clock was restarted");

        // The swing still ends at its original time; the move does not give it a
        // new length.
        play.advance(anim::RUN, 1.6);
        assert!(play.overlay.is_none(), "the shot outlived its own end");
    }

    /// The animation id decides which state tests apply to a one-shot. It does not
    /// by itself decide whether the one-shot can play on the upper body. The cases
    /// below are the ones where a wrong route is visible.
    #[test]
    fn the_id_gates_which_tests_apply_and_not_whether_a_play_can_mask() {
        use vale_protocol::state::movement::move_flags;
        const RUNNING: u32 = move_flags::FORWARD;

        // The jump ids are outside CLASS_A, so the landing plays on the full body
        // even for a character still running when it lands. `JumpLandRun` carries
        // 6.9 y/s of travel for that reason.
        assert_eq!(
            route_oneshot(anim::JUMP_LAND_RUN, RUNNING, 0, false),
            Route::FullBody
        );
        assert_eq!(
            route_oneshot(anim::JUMP_START, RUNNING, 0, false),
            Route::FullBody
        );
        // A chest's lid also plays on the full body; a chest has no upper body.
        assert_eq!(
            route_oneshot(anim::OPEN, RUNNING, 0, false),
            Route::FullBody
        );

        // The airborne test applies to combat ids only. A swing started mid-jump
        // plays on the upper body over the arc; an emote in mid-air does not.
        assert_eq!(route_oneshot(anim::ATTACK_1H, 0, 0, true), Route::Masked);
        assert_eq!(route_oneshot(66, 0, 0, true), Route::FullBody);

        // The two ids that always play on the full body, in any state.
        assert_eq!(
            route_oneshot(anim::DEATH, RUNNING, 0, false),
            Route::FullBody
        );
        assert_eq!(route_oneshot(57, RUNNING, 0, false), Route::FullBody);

        // A seated unit's legs are also occupied. An innkeeper who waves from his
        // stool does not stand up to wave.
        assert_eq!(
            route_oneshot(66, 0, STAND_STATE_SIT_CHAIR, false),
            Route::Masked
        );
        assert_eq!(route_oneshot(66, 0, 0, false), Route::FullBody);

        // A turn on the spot occupies the legs for a one-shot, but not for a cast.
        // The two cases test different sets of movement flags; see
        // `Playback::state_or_held`.
        assert_eq!(
            route_oneshot(anim::ATTACK_1H, move_flags::TURN_LEFT, 0, false),
            Route::Masked
        );
        assert_eq!(
            route_oneshot(anim::ATTACK_1H, move_flags::SWIMMING, 0, false),
            Route::Masked
        );
    }

    /// A caster who runs keeps casting on the upper body.
    ///
    /// The wind-up is a held pose, so it is the only clip on the masked track that
    /// loops: it runs until the cast ends, not until the clip ends. Standing, it
    /// plays on the whole body instead. This matches the 1.12.1 client, which
    /// splits the body on the same movement test this function uses.
    #[test]
    fn a_moving_casters_wind_up_moves_to_the_torso_instead_of_being_dropped() {
        let mut play = playing_with_spine(&[anim::STAND, anim::READY_SPELL_OMNI, anim::RUN]);
        let mut world = standing();
        note(&mut play, &world, anim::STAND, 0.0);

        world.casts_begun = 1;
        world.cast_time_ms = 10_000;
        note(&mut play, &world, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::READY_SPELL_OMNI
        );
        assert!(
            play.overlay.is_none(),
            "a standing cast is pinned to the whole body"
        );

        // The caster runs: the legs play the run and the upper body holds the
        // wind-up.
        play.advance(anim::RUN, 1.5);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);
        assert!(
            play.casting.is_some(),
            "the cast was dropped instead of masked"
        );
        let overlay = play.overlay.as_ref().expect("the torso holds the wind-up");
        assert_eq!(
            play.skeleton.sequences[overlay.sequence].id,
            anim::READY_SPELL_OMNI
        );
        assert!(
            overlay.looping,
            "a held wind-up must wrap, not freeze on its last frame"
        );

        // Stopping moves it back to the whole body and clears the upper-body track.
        play.advance(anim::STAND, 2.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::READY_SPELL_OMNI
        );
        assert!(play.overlay.is_none());

        // The release ends the wind-up on whichever track holds it.
        play.advance(anim::RUN, 2.5);
        assert!(play.overlay.is_some());
        world.casts_released = 1;
        note(&mut play, &world, anim::RUN, 2.6);
        assert!(play.overlay.is_none(), "the wind-up outlived its cast");
    }

    /// A stunned unit plays `Stun`, and holds it for as long as the aura is present
    /// rather than for a stated length.
    ///
    /// The reported bug was "stuns, roots and death do not work … anims do not play
    /// either". The animation comes from `SpellVisualKit`'s `animID` on the state
    /// kit. For Hammer of Justice (349) it is 14 (`Stun`), next to the
    /// `StunSwirl_State_Head` this client already drew. 257 of the 308 spells whose
    /// state kit names an animation name that one.
    ///
    /// Unlike a cast, the pose has no clock. It starts when the aura slot is set
    /// and ends when the slot is cleared.
    #[test]
    fn an_auras_pose_is_held_while_the_aura_is_and_ends_with_it() {
        let mut play = playing_with_spine(&[anim::STAND, anim::STUN, anim::RUN]);
        let world = standing();
        note(&mut play, &world, anim::STAND, 0.0);
        play.advance(anim::STAND, 0.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);

        // The stun lands. No packet announces it and no counter changes. The aura
        // slot is set, and the pose is applied again every frame.
        play.aura = Some(anim::STUN);
        play.advance(anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::STUN,
            "a stunned unit stands in its idle"
        );
        assert!(
            play.overlay.is_none(),
            "a standing hold pins the whole body"
        );

        // The pose has no timer. A minute later it still plays, where a cast's
        // `until` would have expired.
        play.advance(anim::STAND, 61.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STUN);

        // The pose ends when the slot is cleared, not on a packet.
        play.aura = None;
        play.advance(anim::STAND, 62.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
    }

    /// When both a cast and an aura name a pose, the cast's pose plays. The aura's
    /// pose plays again when the cast ends.
    #[test]
    fn a_cast_outranks_an_auras_pose_and_the_aura_is_still_there_after() {
        let mut play =
            playing_with_spine(&[anim::STAND, anim::STUN, anim::READY_SPELL_OMNI, anim::RUN]);
        let mut world = standing();
        note(&mut play, &world, anim::STAND, 0.0);
        play.aura = Some(anim::STUN);

        world.casts_begun = 1;
        world.cast_time_ms = 2_000;
        note(&mut play, &world, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::READY_SPELL_OMNI
        );

        // The cast ends; the aura is still set.
        play.advance(anim::STAND, 3.5);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STUN);
    }

    /// A corpse does not play the stun pose. The aura stays on the wire after the
    /// unit dies (vmangos clears it a tick later), and without this rule the corpse
    /// would play `Stun` on the floor.
    #[test]
    fn a_corpse_drops_the_auras_pose() {
        let mut play = playing_with_spine(&[anim::STAND, anim::STUN, anim::DEATH]);
        let world = standing();
        note(&mut play, &world, anim::STAND, 0.0);
        play.aura = Some(anim::STUN);
        play.advance(anim::STAND, 1.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STUN);

        play.advance(anim::DEATH, 2.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::DEATH);
    }

    /// A stunned unit that is moving keeps its gait on the legs, and the pose plays
    /// on the upper body, as a moving caster's wind-up does. It uses the same slot
    /// and the same rule. A root normally prevents the movement, but a knockback or
    /// a spline can move a unit the server has stopped.
    #[test]
    fn a_moving_units_aura_pose_rides_the_torso() {
        let mut play = playing_with_spine(&[anim::STAND, anim::STUN, anim::RUN]);
        let world = standing();
        note(&mut play, &world, anim::STAND, 0.0);
        play.aura = Some(anim::STUN);

        play.advance(anim::RUN, 1.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);
        let overlay = play
            .overlay
            .as_ref()
            .expect("the torso holds the aura's pose");
        assert_eq!(play.skeleton.sequences[overlay.sequence].id, anim::STUN);
        assert!(
            overlay.looping,
            "a held pose must wrap, not freeze on its last frame"
        );

        // Stopping moves it back to the whole body and clears the upper-body track.
        play.advance(anim::STAND, 2.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STUN);
        assert!(play.overlay.is_none());
    }

    /// A swing started mid-cast takes the upper body, and the wind-up takes it back
    /// when the swing ends. The upper-body subtree plays the newest clip.
    #[test]
    fn a_masked_shot_interrupts_a_held_wind_up_and_the_wind_up_retakes_it() {
        let mut play = playing_with_spine(&[
            anim::STAND,
            anim::READY_SPELL_OMNI,
            anim::RUN,
            anim::ATTACK_UNARMED,
        ]);
        let mut world = moving(7.0);
        note(&mut play, &world, anim::RUN, 0.0);
        world.casts_begun = 1;
        world.cast_time_ms = 10_000;
        note(&mut play, &world, anim::RUN, 1.0);
        play.advance(anim::RUN, 1.0);
        assert!(play.overlay.as_ref().is_some_and(|m| m.looping));

        world.swings_thrown = 1;
        note(&mut play, &world, anim::RUN, 1.1);
        play.advance(anim::RUN, 1.1);
        let overlay = play.overlay.as_ref().expect("the swing took the torso");
        assert_eq!(
            play.skeleton.sequences[overlay.sequence].id,
            anim::ATTACK_UNARMED
        );
        assert!(!overlay.looping);

        // 500 ms later the swing has ended and the wind-up plays again.
        play.advance(anim::RUN, 1.7);
        let overlay = play
            .overlay
            .as_ref()
            .expect("the wind-up did not retake the torso");
        assert_eq!(
            play.skeleton.sequences[overlay.sequence].id,
            anim::READY_SPELL_OMNI
        );
        assert!(overlay.looping);
    }

    /// The stand state is a pose the server sets. An innkeeper on his stool drawn
    /// standing would stand through the furniture.
    #[test]
    fn a_seated_unit_sits() {
        let mut sitting = moving(0.0);
        sitting.stand_state = STAND_STATE_SIT_MEDIUM_CHAIR;
        assert_eq!(wanted_animation(&sitting, 0, None), anim::SIT_CHAIR_MED);

        // The sitting pose applies only while the unit is stationary. If the
        // server leaves the byte set on a unit it then moves, the unit runs rather
        // than sliding along seated.
        sitting.moving = true;
        sitting.speed = 7.0;
        assert_eq!(wanted_animation(&sitting, 0, None), anim::RUN);
    }

    /// A unit that is auto-attacking and standing still plays the ready pose. A unit
    /// that is only in combat does not.
    ///
    /// The 1.12.1 client plays the Ready idle only while the unit has an
    /// auto-attack target, which is set from `SMSG_ATTACKSTART` until
    /// `SMSG_ATTACKSTOP`. Reading `UNIT_FLAG_IN_COMBAT` instead put every unit
    /// involved in a fight into its combat guard: a caster being attacked, a healer
    /// at the back, and anyone who had drawn aggro from across a room.
    #[test]
    fn a_unit_swinging_at_something_stands_ready() {
        let mut fighting = moving(0.0);
        // In combat with no auto-attack target: the idle.
        fighting.in_combat = true;
        assert_eq!(wanted_animation(&fighting, 0, None), anim::STAND);

        fighting.attacking = true;
        assert_eq!(wanted_animation(&fighting, 0, None), anim::READY_UNARMED);

        // The combat flag does not affect the result.
        fighting.in_combat = false;
        assert_eq!(wanted_animation(&fighting, 0, None), anim::READY_UNARMED);

        let mut play = playing(&[anim::STAND, anim::READY_UNARMED]);
        play.advance(anim::READY_UNARMED, 0.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::READY_UNARMED
        );
    }

    /// The ready stance and the swing both follow the drawn weapon, which the sheath
    /// state decides, not the inventory.
    ///
    /// A guard with a greatsword on his back takes the unarmed stance, because the
    /// sword is sheathed. When he draws it he takes the two-handed stance and
    /// swings two-handed. Reading the item alone would put a character in the
    /// two-handed stance with nothing in their hands.
    #[test]
    fn the_stance_and_the_swing_follow_the_drawn_weapon() {
        let mut fighting = moving(0.0);
        fighting.attacking = true;
        fighting.weapons[0] = a_greatsword();

        // Sheathed: the weapon is on his back and the stance is unarmed.
        let away = vale_assets::look::sheath::UNARMED;
        let out = SHEATH_STATE_MELEE;
        assert_eq!(wanted_animation(&fighting, away, None), anim::READY_UNARMED);
        assert_eq!(drawn_weapon(&fighting, away).attack(), anim::ATTACK_UNARMED);

        // Drawn: both change together, because both read the drawn weapon.
        assert_eq!(wanted_animation(&fighting, out, None), anim::READY_2H);
        assert_eq!(drawn_weapon(&fighting, out).attack(), anim::ATTACK_2H);

        // A one-hander is its own family, and a shield in the off hand does not
        // change what the main hand swings.
        fighting.weapons[0] = a_mace();
        fighting.weapons[1] = a_shield();
        assert_eq!(wanted_animation(&fighting, out, None), anim::READY_1H);
        assert_eq!(drawn_weapon(&fighting, out).attack(), anim::ATTACK_1H);
    }

    /// A swing plays the weapon family's own animation, and a model that lacks it
    /// falls back rather than freezing.
    ///
    /// The fixture has only the unarmed swing. Most creature models are the same:
    /// a wolf's bite is `AttackUnarmed` and it has no other swing.
    #[test]
    fn a_weapon_swing_falls_back_to_the_one_the_model_has() {
        let mut play = playing(&[anim::STAND, anim::ATTACK_UNARMED]);
        let mut world = moving(0.0);
        world.weapons[0] = a_greatsword();
        let out = SHEATH_STATE_MELEE;
        note_sheathed(&mut play, &world, out, anim::STAND, 0.0);

        world.swings_thrown = 1;
        note_sheathed(&mut play, &world, out, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::ATTACK_UNARMED,
            "a two-hander asked for Attack2H and the model has only the fist"
        );

        // A model that carries `Attack2H` plays it.
        let mut armed = playing(&[anim::STAND, anim::ATTACK_UNARMED, anim::ATTACK_2H]);
        note_sheathed(&mut armed, &moving(0.0), out, anim::STAND, 0.0);
        note_sheathed(&mut armed, &world, out, anim::STAND, 1.0);
        armed.advance(anim::STAND, 1.0);
        assert_eq!(armed.skeleton.sequences[armed.sequence].id, anim::ATTACK_2H);
    }

    /// A dodge, a parry and a block each have their own reaction, and none of them
    /// is the flinch.
    ///
    /// Before the victim state was read, a blow that a character defended against
    /// produced no animation, because `HITINFO_AFFECTS_VICTIM` is not set for it.
    /// Two duellists who mostly parried stood still. The parry is made with the
    /// weapon, so it is chosen through the same weapon family as the swing.
    #[test]
    fn the_victim_state_chooses_the_reaction() {
        let mut world = moving(0.0);
        world.weapons[0] = a_greatsword();
        let out = SHEATH_STATE_MELEE;

        world.last_victim_state = victim_state::DODGE;
        assert_eq!(reaction(&world, out), anim::DODGE);
        world.last_victim_state = victim_state::BLOCKS;
        assert_eq!(reaction(&world, out), anim::SHIELD_BLOCK);
        world.last_victim_state = victim_state::PARRY;
        assert_eq!(
            reaction(&world, out),
            anim::PARRY_2H,
            "a two-hander parried one-handed"
        );
        world.last_victim_state = victim_state::NORMAL;
        assert_eq!(reaction(&world, out), anim::COMBAT_WOUND);

        // The reaction reaches the model: a dodge fires as a one-shot on the same
        // counter as a flinch.
        let mut play = playing(&[anim::STAND, anim::COMBAT_WOUND, anim::DODGE]);
        world.last_victim_state = victim_state::DODGE;
        note(&mut play, &world, anim::STAND, 0.0);
        world.blows_taken = 1;
        note(&mut play, &world, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::DODGE);
    }

    /// Swimming is a pair of animations, not a variant of the gait.
    ///
    /// A swimmer who stops treads water (`SwimIdle`) rather than playing Stand in
    /// mid-river.
    #[test]
    fn swimming_has_its_own_idle() {
        let mut swimmer = moving(0.0);
        swimmer.swimming = true;
        assert_eq!(wanted_animation(&swimmer, 0, None), anim::SWIM_IDLE);

        swimmer.moving = true;
        swimmer.speed = 4.7;
        assert_eq!(wanted_animation(&swimmer, 0, None), anim::SWIM);

        // Swimming outranks the run. A character moving at running speed through
        // deep water swims, and the server's swimming flag decides which.
        swimmer.speed = 7.6;
        assert_eq!(wanted_animation(&swimmer, 0, None), anim::SWIM);
        swimmer.swimming = false;
        assert_eq!(wanted_animation(&swimmer, 0, None), anim::RUN);
    }

    /// The cast wind-up is a held state that the release ends. The timer only ends
    /// a cast that is never released.
    ///
    /// No character model carries `SpellCast` (32) or `SpellPrecast` (31):
    /// `HumanMale.m2` has the directed/omni pairs at 51..54 and nothing at either
    /// id. The fixture is built the same way, so code that asked for 32 without
    /// falling back would animate nothing.
    #[test]
    fn a_cast_holds_its_wind_up_until_it_is_released() {
        let mut play = playing(&[anim::STAND, anim::READY_SPELL_OMNI, anim::SPELL_CAST_OMNI]);
        let mut world = moving(0.0);
        note(&mut play, &world, anim::STAND, 0.0);

        // The cast begins: a 3.5 s bar, and the wind-up is held.
        world.casts_begun = 1;
        world.cast_time_ms = 3500;
        note(&mut play, &world, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::READY_SPELL_OMNI,
            "SpellPrecast did not fall through to what the model has"
        );
        // Still held two seconds in. A one-shot would have ended.
        play.advance(anim::STAND, 3.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::READY_SPELL_OMNI
        );

        // The release ends it and fires its own animation.
        world.casts_released = 1;
        note(&mut play, &world, anim::STAND, 3.2);
        play.advance(anim::STAND, 3.2);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::SPELL_CAST_OMNI
        );
        assert!(play.casting.is_none());

        // Afterwards the state's animation plays again.
        play.advance(anim::STAND, 4.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
    }

    /// A refused cast ends its wind-up without a release animation.
    ///
    /// This client draws the player's own cast when the key is pressed, so the pose
    /// is already held when `SMSG_CAST_RESULT` refuses it. The only end a wind-up
    /// had was `Casting::until`, the cast bar's length, so a Fireball refused on
    /// the press held its hands up for a second and a half and then lowered them.
    /// `casts_cancelled` is the third way a wind-up ends; see
    /// `WorldEntity::casts_cancelled` for the three packets that increment it.
    #[test]
    fn a_cancelled_cast_ends_the_wind_up_with_nothing_after_it() {
        let mut play = playing(&[anim::STAND, anim::READY_SPELL_OMNI, anim::SPELL_CAST_OMNI]);
        let mut world = moving(0.0);
        note(&mut play, &world, anim::STAND, 0.0);

        world.casts_begun = 1;
        world.cast_time_ms = 3500;
        note(&mut play, &world, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::READY_SPELL_OMNI
        );

        // Refused. The wind-up stops and no release is played. That is the
        // difference from an increment of `casts_released`.
        world.casts_cancelled = 1;
        note(&mut play, &world, anim::STAND, 1.2);
        assert!(
            play.casting.is_none(),
            "the wind-up is not still the answer"
        );
        play.advance(anim::STAND, 1.2);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::STAND,
            "a refused cast goes back to standing rather than throwing anything"
        );
    }

    /// The spell chooses the cast pose, so two spells cast the same way can look
    /// different.
    ///
    /// Without the `SpellVisual` lookup, every cast resolved through one fallback
    /// list to `SpellCastOmni` (both hands over the head), so a fireball, a heal
    /// and opening a chest all looked the same. The two casts here differ only in
    /// the spell id, and the model carries all four sequences, so code that
    /// ignored the id would give both the same result.
    #[test]
    fn the_spell_decides_which_cast_animation_is_played() {
        let model = &[
            anim::STAND,
            anim::READY_SPELL_DIRECTED,
            anim::READY_SPELL_OMNI,
            anim::SPELL_CAST_DIRECTED,
            anim::SPELL_CAST_OMNI,
        ];
        // Fireball is directed and Lesser Heal is omni, per `SpellVisual`; see
        // `vale_assets::tables::spell`.
        const FIREBALL: u32 = 133;
        const LESSER_HEAL: u32 = 2050;
        let visual = |spell: u32| match spell {
            FIREBALL => CastAnimation {
                hold: Some(anim::READY_SPELL_DIRECTED),
                release: Some(anim::SPELL_CAST_DIRECTED),
                ..CastAnimation::default()
            },
            LESSER_HEAL => CastAnimation {
                hold: Some(anim::READY_SPELL_OMNI),
                release: Some(anim::SPELL_CAST_OMNI),
                ..CastAnimation::default()
            },
            _ => CastAnimation::default(),
        };

        let cast = |spell: u32, at: f32| {
            let mut play = playing(model);
            let mut world = moving(0.0);
            play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, at);

            world.casts_begun = 1;
            world.cast_time_ms = 3500;
            world.last_spell = spell;
            play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, at + 1.0);
            play.advance(anim::STAND, at + 1.0);
            let held = play.skeleton.sequences[play.sequence].id;

            world.casts_released = 1;
            play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, at + 2.0);
            play.advance(anim::STAND, at + 2.0);
            (held, play.skeleton.sequences[play.sequence].id)
        };

        assert_eq!(
            cast(FIREBALL, 0.0),
            (anim::READY_SPELL_DIRECTED, anim::SPELL_CAST_DIRECTED),
            "a fireball was not thrown from the shoulder"
        );
        assert_eq!(
            cast(LESSER_HEAL, 0.0),
            (anim::READY_SPELL_OMNI, anim::SPELL_CAST_OMNI),
            "a heal was not raised overhead"
        );
    }

    /// A channel holds the channel kit's pose, not the wind-up's.
    ///
    /// The reported spell was Blizzard: `precastKit` at `ReadySpellOmni` for the
    /// wind-up and `channelKit` at `ChannelCastOmni` for the channel. The two
    /// arrive as two begins, `SMSG_SPELL_START` and then `MSG_CHANNEL_START`, which
    /// increments `casts_begun` again. Reading `hold` both times left the caster
    /// in the wind-up for the whole four seconds.
    ///
    /// The second assertion covers Arcane Missiles, which states a channel kit and
    /// no precast kit. Its `hold` is already the channel pose, and the first begin
    /// must keep that pose.
    #[test]
    fn a_channel_holds_its_own_pose_and_not_the_wind_ups() {
        let model = &[
            anim::STAND,
            anim::READY_SPELL_OMNI,
            anim::CHANNEL_CAST_OMNI,
            anim::SPELL_CAST_OMNI,
        ];
        const BLIZZARD: u32 = 10;
        const ARCANE_MISSILES: u32 = 5143;
        let visual = |spell: u32| match spell {
            BLIZZARD => CastAnimation {
                hold: Some(anim::READY_SPELL_OMNI),
                release: Some(anim::CHANNEL_CAST_OMNI),
                channel: Some(anim::CHANNEL_CAST_OMNI),
                ..CastAnimation::default()
            },
            // No precast kit, so the lookup puts the channel in `hold` too.
            ARCANE_MISSILES => CastAnimation {
                hold: Some(anim::CHANNEL_CAST_OMNI),
                release: None,
                channel: Some(anim::CHANNEL_CAST_OMNI),
                ..CastAnimation::default()
            },
            _ => CastAnimation::default(),
        };

        let channel = |spell: u32| {
            let mut play = playing(model);
            let mut world = moving(0.0);
            play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, 0.0);

            // The wind-up: one begin, with the cast bar's own length.
            world.casts_begun = 1;
            world.cast_time_ms = 1500;
            world.last_spell = spell;
            play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, 1.0);
            play.advance(anim::STAND, 1.0);
            let wind_up = play.skeleton.sequences[play.sequence].id;

            // The release and the channel's begin, in one poll. vmangos sends
            // `SendSpellGo` and then `SendChannelStart` in that order.
            world.casts_released = 1;
            world.casts_channelled = 1;
            world.casts_begun = 2;
            world.cast_time_ms = 8000;
            play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, 2.0);
            play.advance(anim::STAND, 2.0);
            let held = play.skeleton.sequences[play.sequence].id;

            // Still held four seconds in, rather than dropped when the cast bar
            // would have ended.
            play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, 6.0);
            play.advance(anim::STAND, 6.0);
            (wind_up, held, play.skeleton.sequences[play.sequence].id)
        };

        assert_eq!(
            channel(BLIZZARD),
            (
                anim::READY_SPELL_OMNI,
                anim::CHANNEL_CAST_OMNI,
                anim::CHANNEL_CAST_OMNI
            ),
            "Blizzard stood in its wind-up for the whole channel"
        );
        assert_eq!(
            channel(ARCANE_MISSILES),
            (
                anim::CHANNEL_CAST_OMNI,
                anim::CHANNEL_CAST_OMNI,
                anim::CHANNEL_CAST_OMNI
            ),
            "a spell with only a channel kit holds it from the first begin"
        );
    }

    /// A ranged attack is the exception: its release animation comes from the
    /// weapon, not the spell.
    ///
    /// Auto Shot states no visual, so the rule "no release means none" left an
    /// archer standing still through a whole volley. The three assertions cover
    /// three cases: a bow plays the bow shot, a gun in the same slot plays its own
    /// clip rather than the bow's, and a wand plays nothing instead of a punch.
    /// `WeaponAnim` puts the wand in `Unarmed` because the character models carry
    /// no wand sequence.
    #[test]
    fn a_ranged_attack_fires_the_drawn_weapon() {
        let bow =
            Weapon { display_id: 5432, class: 2, subclass: 2, inventory_type: 15, sheath: 3, material: 2, enchantments: [0; 7] };
        let gun = Weapon { subclass: 3, ..bow };
        let wand = Weapon { subclass: 19, ..bow };
        // The spell states only that it is fired from the ranged slot, as Auto
        // Shot's row does.
        let shot = |_| CastAnimation { ranged_shot: true, ..CastAnimation::default() };

        let fire = |ranged: Weapon| {
            let mut play = playing(&[
                anim::STAND,
                anim::ATTACK_BOW,
                anim::ATTACK_RIFLE,
                anim::ATTACK_UNARMED,
            ]);
            let mut world = moving(0.0);
            world.weapons[2] = ranged;
            play.note_actions(&world, vale_assets::look::sheath::RANGED, |_| None, |_| None, shot, anim::STAND, 0.0);

            // One shot: the server's `SMSG_SPELL_GO`. For an auto-repeat spell it
            // is the only thing that increments this counter, because the key
            // press is not predicted.
            world.casts_released = 1;
            play.note_actions(&world, vale_assets::look::sheath::RANGED, |_| None, |_| None, shot, anim::STAND, 1.0);
            play.advance(anim::STAND, 1.0);
            play.skeleton.sequences[play.sequence].id
        };

        assert_eq!(fire(bow), anim::ATTACK_BOW);
        assert_eq!(fire(gun), anim::ATTACK_RIFLE, "a gun is not drawn back like a bow");
        assert_eq!(
            fire(wand),
            anim::STAND,
            "a wand has no sequence, and a punch is worse than nothing"
        );
    }

    /// A spell that states no release plays none. A channelled spell's visual
    /// carries a channel kit and neither of the other two. Playing the generic
    /// release on its `SMSG_SPELL_GO` would play a throw at the end of every
    /// channel.
    #[test]
    fn a_spell_with_a_hold_and_no_release_fires_nothing_at_the_end() {
        let mut play = playing(&[
            anim::STAND,
            anim::CHANNEL_CAST_DIRECTED,
            anim::SPELL_CAST_OMNI,
        ]);
        let channel = |_| CastAnimation {
            hold: Some(anim::CHANNEL_CAST_DIRECTED),
            release: None,
            ..CastAnimation::default()
        };
        let mut world = moving(0.0);
        play.note_actions(&world, 0, |_| None, |_| None, channel, anim::STAND, 0.0);

        world.casts_begun = 1;
        world.cast_time_ms = 3000;
        play.note_actions(&world, 0, |_| None, |_| None, channel, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::CHANNEL_CAST_DIRECTED
        );

        world.casts_released = 1;
        play.note_actions(&world, 0, |_| None, |_| None, channel, anim::STAND, 1.5);
        play.advance(anim::STAND, 1.5);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::STAND,
            "the generic release was played for a spell that has none"
        );
    }

    /// A spell with no cast animation in its visual lookup plays nothing. It once
    /// got the generic pair.
    ///
    /// `SpellVisualKit`'s `animID` is the only data that says what a cast looks
    /// like, and 9,467 of the game's 22,360 spells reach no kit (`vale spell`:
    /// 12,893 do). These are procs, aura applications and silent utility spells.
    /// The old fallback gave each of them `SpellPrecast` (31), which no character
    /// model carries, so it fell back to `ReadySpellOmni` (52), which all of them
    /// carry. Those spells all raised the caster's hands.
    #[test]
    fn a_spell_with_no_visual_plays_no_cast_animation() {
        let mut play = playing(&[anim::STAND, anim::READY_SPELL_OMNI, anim::SPELL_CAST_OMNI]);
        let mut world = moving(0.0);
        let silent = |_| CastAnimation::default();
        play.note_actions(&world, 0, |_| None, |_| None, silent, anim::STAND, 0.0);

        world.casts_begun = 1;
        world.casts_released = 0;
        world.cast_time_ms = 3500;
        world.last_spell = 999_999;
        play.note_actions(&world, 0, |_| None, |_| None, silent, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert!(
            play.casting.is_none(),
            "a wind-up was invented for a spell that states none"
        );
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::STAND,
            "the caster should be standing there"
        );

        // The release also plays nothing, as it does for a spell that names a
        // wind-up and no release.
        world.casts_released = 1;
        play.note_actions(&world, 0, |_| None, |_| None, silent, anim::STAND, 2.0);
        play.advance(anim::STAND, 2.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
    }

    /// A caster on a model that cannot mask, who moves away, has been interrupted.
    /// The server does not say so; the movement it sends is the only signal.
    ///
    /// The fixture is boneless, like a creature: it has no `SpineLow`, so the
    /// wind-up cannot play on the upper body while the legs run. The wind-up must
    /// then take the whole body or stop. Holding it would keep a pose after its
    /// cause ended, like a corpse standing up to flinch. A model that can mask
    /// keeps casting on the upper body instead; see
    /// [`a_moving_casters_wind_up_moves_to_the_torso_instead_of_being_dropped`].
    #[test]
    fn moving_cancels_a_cast_on_a_model_that_cannot_mask() {
        let mut play = playing(&[anim::STAND, anim::READY_SPELL_OMNI, anim::RUN]);
        let mut world = moving(0.0);
        note(&mut play, &world, anim::STAND, 0.0);

        world.casts_begun = 1;
        world.cast_time_ms = 10_000;
        note(&mut play, &world, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::READY_SPELL_OMNI
        );

        play.advance(anim::RUN, 1.5);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);
        assert!(
            play.casting.is_none(),
            "the cast outlived the character running off"
        );
    }

    /// An interrupted cast that is never released still ends, on its own timer.
    #[test]
    fn a_cast_that_is_never_released_expires() {
        let mut play = playing(&[anim::STAND, anim::READY_SPELL_OMNI]);
        let mut world = moving(0.0);
        note(&mut play, &world, anim::STAND, 0.0);
        world.casts_begun = 1;
        world.cast_time_ms = 1000;
        note(&mut play, &world, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert!(play.casting.is_some());
        play.advance(anim::STAND, 2.5);
        assert!(play.casting.is_none());
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
    }

    /// A held emote is a state, and it is not sent as an emote packet.
    ///
    /// vmangos `Unit::HandleEmote` branches on the row's `EmoteType`: 0 is sent as
    /// `SMSG_EMOTE` and any other value is written into `UNIT_NPC_EMOTESTATE` and
    /// left there. Every `ONESHOT_*` row reads 0 and every `STATE_*` row reads 2.
    /// A client that reads only the packet animates `/wave` but not `/dance`, and
    /// leaves every innkeeper standing idle instead of working.
    #[test]
    fn a_held_emote_is_a_state_and_outranks_the_combat_stance() {
        const EMOTE_DANCE: u16 = 69;
        let mut idle = moving(0.0);
        assert_eq!(wanted_animation(&idle, 0, None), anim::STAND);
        assert_eq!(wanted_animation(&idle, 0, Some(EMOTE_DANCE)), EMOTE_DANCE);

        // It outranks the ready stance. The server set a specific pose, and would
        // have cleared the field to get the default.
        idle.in_combat = true;
        assert_eq!(wanted_animation(&idle, 0, Some(EMOTE_DANCE)), EMOTE_DANCE);

        // Moving outranks it, as moving outranks the seated stand states.
        idle.moving = true;
        idle.speed = 7.0;
        assert_eq!(wanted_animation(&idle, 0, Some(EMOTE_DANCE)), anim::RUN);
    }

    /// An emote the model cannot play is ignored, not substituted.
    ///
    /// `Emotes.dbc` names 78 emotes, and a model carries the few its race was
    /// animated for. Falling back to Stand would make `/train` at a creature with
    /// no such animation interrupt its current animation to stand still.
    #[test]
    fn an_emote_plays_its_animation_or_none_at_all() {
        const EMOTE_DANCE: u32 = 10;
        const EMOTE_TRAIN: u32 = 47;
        // `EmoteDance` is 69 and `EmoteTrain` 195. This model has only the first,
        // which is the usual case.
        let table = |id: u32| match id {
            EMOTE_DANCE => Some(69),
            EMOTE_TRAIN => Some(195),
            _ => None,
        };

        let mut play = playing(&[anim::STAND, 69]);
        let mut world = moving(0.0);
        play.note_actions(
            &world,
            0,
            table,
            |_| None,
            |_| CastAnimation::default(),
            anim::STAND,
            0.0,
        );

        world.emotes = 1;
        world.last_emote = EMOTE_DANCE;
        play.note_actions(
            &world,
            0,
            table,
            |_| None,
            |_| CastAnimation::default(),
            anim::STAND,
            1.0,
        );
        play.advance(anim::STAND, 1.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id, 69,
            "the dance did not play"
        );

        // An emote whose animation this model lacks changes nothing. The clock
        // shows it: a substitution would restart the clock at zero.
        world.emotes = 2;
        world.last_emote = EMOTE_TRAIN;
        play.note_actions(
            &world,
            0,
            table,
            |_| None,
            |_| CastAnimation::default(),
            anim::STAND,
            1.2,
        );
        assert_eq!(
            play.skeleton.sequences[play.sequence].id, 69,
            "an emote the model cannot play interrupted the one it could"
        );
    }

    /// A change of wanted animation that resolves to the same sequence does not
    /// restart the clock. A model with no Run resolves Run and Stand to its one
    /// idle loop. Restarting that loop every time the creature stops produces a
    /// visible reset that looks like a broken skinning shader.
    #[test]
    fn only_a_change_of_sequence_restarts_the_animation() {
        // One sequence, Stand, so Run falls back to it.
        let mut play = playing(&[anim::STAND]);
        play.advance(anim::STAND, 0.0);
        assert_eq!(play.sequence, 0);

        // 0.7 s into a 500 ms loop, so the wrapped clock is 200, which a restart
        // could not produce. On a whole number of cycles it would be 0, the same
        // value a restart gives.
        let elapsed = play.advance(anim::RUN, 0.7);
        assert_eq!(play.sequence, 0, "it resolved to a different sequence");
        assert_eq!(play.since, 0.0, "the clock was restarted");
        assert_eq!(elapsed, 200);
        assert!(play.fade.is_none(), "it faded into itself");
    }

    /// A switch to a different sequence restarts the clock and fades out of the
    /// old sequence. The old sequence keeps its own clock, so a stride continues
    /// during the fade.
    #[test]
    fn a_genuine_switch_cross_fades() {
        let mut play = playing(&[anim::STAND, anim::RUN]);
        play.advance(anim::STAND, 0.0);

        let elapsed = play.advance(anim::RUN, 2.0);
        assert_eq!(play.sequence, 1);
        assert_eq!(elapsed, 0, "the new animation starts at its own frame zero");
        let fade = play.fade.as_ref().expect("a fade");
        assert_eq!(fade.sequence, 0);
        assert_eq!(fade.since, 0.0, "the outgoing animation lost its clock");

        // The fade ends 150 ms later, rather than sampling a second animation for
        // the rest of the session.
        play.advance(anim::RUN, 2.0 + FADE_SECS);
        assert!(play.fade.is_none());
    }

    /// Walk falls back to Run before it falls back to Stand. 42 of the 405
    /// animated models have no Run and 43 have no Walk, and a moving creature
    /// should play a moving animation.
    #[test]
    fn a_missing_gait_falls_back_to_the_other_one() {
        let mut play = playing(&[anim::STAND, anim::RUN]);
        play.advance(anim::WALK, 0.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);

        let mut play = playing(&[anim::STAND, anim::WALK]);
        play.advance(anim::RUN, 0.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, anim::WALK);
    }

    /// A joint is a child so that despawning the entity despawns it too. It carries
    /// no `Transform`, so nothing overwrites the affine transform `animate` wrote
    /// into it. The second part assumes that every Bevy propagation query requires
    /// a `Transform`. If that stopped being true, every entity would be posed at
    /// the origin with no error, so this test checks it.
    #[test]
    fn transform_propagation_leaves_a_joint_alone() {
        let mut app = App::new();
        app.add_plugins(bevy::transform::TransformPlugin);

        let entity = app
            .world_mut()
            .spawn(Transform::from_xyz(1.0, 2.0, 3.0))
            .id();
        // What `animate` writes: the entity's placement times the bone's pose,
        // already composed, so a parent must not apply it again.
        let posed = GlobalTransform::from(Transform::from_xyz(9.0, 9.0, 9.0));
        let joint = app.world_mut().spawn((Joint, posed, ChildOf(entity))).id();

        app.update();

        let after = *app
            .world()
            .entity(joint)
            .get::<GlobalTransform>()
            .expect("a joint");
        assert_eq!(
            after.translation(),
            Vec3::splat(9.0),
            "propagation composed the parent's transform into the joint"
        );

        // Despawning the entity despawns the joint, which is why the joint is a
        // child rather than a separate entity.
        app.world_mut().entity_mut(entity).despawn();
        assert!(app.world().get_entity(joint).is_err());
    }

    /// The joints and the attachments of one character must be composed against
    /// the same frame's placement. Different code composes them: a joint is a world
    /// matrix this pass writes, while an attached model is a local `Transform` that
    /// Bevy composes in `PostUpdate`.
    ///
    /// Reading the entity's `GlobalTransform` here, which propagation last wrote a
    /// frame ago, poses the body where the entity was and hangs the helm where it
    /// is. The gap is one frame of travel, 0.13 yards at a run, and on screen it
    /// looks like loose attachments rather than a frame of latency. Nothing warns
    /// and no count changes, so this test checks it: reading the `GlobalTransform`
    /// makes the first assertion read 0 against 10.
    #[test]
    fn a_joint_and_an_attachment_share_one_frames_placement() {
        let mut app = App::new();
        let [entity, bone_joint, identity_joint, pauldron] = one_rig(&mut app, f32::MAX);

        app.update();

        // The character runs: `place_entities` writes this frame's interpolated
        // position to the `Transform`, and the entity's `GlobalTransform` still
        // holds last frame's position.
        let moved = Vec3::new(10.0, 0.0, 0.0);
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Transform>()
            .expect("a placement")
            .translation = moved;

        app.update();

        let at = |e: Entity| {
            app.world()
                .entity(e)
                .get::<GlobalTransform>()
                .expect("a world transform")
                .translation()
        };
        assert_eq!(at(bone_joint), moved, "the body is a frame behind");
        assert_eq!(
            at(identity_joint),
            moved,
            "the weightless vertices are a frame behind"
        );
        assert_eq!(at(pauldron), moved, "the pauldron is not where the body is");
    }

    /// A rig is culled against the world camera only.
    ///
    /// A process holds several 3D cameras (the unit-frame portraits, the paper
    /// doll, any pictures a host draws), and a query yields them in the order their
    /// archetypes were created. The pass took the first. Once a picture camera
    /// sorted ahead of the world camera, every rig was tested against a frustum
    /// aimed at a model on another render layer, failed, and was not posed again.
    /// The body stayed where it was last posed while the entity walked away, and
    /// anything re-hung on it sat at the root. Turning fog off was one trigger,
    /// because removing `DistanceFog` moves the world camera to a newer archetype.
    ///
    /// Both arrangements are run, because this code does not control which camera
    /// a query yields first: a picture camera that cannot see the rig beside a
    /// world camera that can, and the reverse. A pass that takes the first camera
    /// gets one of the two wrong in either order. Each arrangement is run with the
    /// cameras spawned in both orders for the same reason.
    #[test]
    fn a_picture_camera_does_not_decide_what_is_posed() {
        for picture_first in [true, false] {
            assert!(
                posed_with_cameras(picture_first, false, true),
                "the world camera sees the rig and it was not posed"
            );
            assert!(
                !posed_with_cameras(picture_first, true, false),
                "the world camera cannot see the rig and it was posed"
            );
        }
    }

    /// Whether a rig that walks ten yards is posed at its new position, under a
    /// picture camera and a world camera that each see either everything or
    /// nothing near it.
    fn posed_with_cameras(picture_first: bool, picture_sees: bool, world_sees: bool) -> bool {
        use bevy::camera::primitives::Frustum;
        let mut app = App::new();
        // A camera a kilometre up, looking along -Z: the rig is far outside its
        // side planes. The far plane is not the test, because the pass skips it.
        let eye = Transform::from_xyz(0.0, 1000.0, 0.0);
        let clip_from_world = Mat4::perspective_rh(1.0, 1.0, 0.1, 5.0) * eye.to_matrix().inverse();
        let blind = Frustum(bevy::math::primitives::ViewFrustum::from_clip_from_world(
            &clip_from_world,
        ));
        for centre in [Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0)] {
            let sphere = bevy::camera::primitives::Sphere { center: centre.into(), radius: 2.0 };
            assert!(!blind.intersects_sphere(&sphere, false), "the blind camera sees {centre}");
        }
        // A default frustum has no planes and sees everything.
        let frustum = |sees: bool| match sees {
            true => Frustum::default(),
            false => blind,
        };
        let picture = (Camera3d::default(), eye, GlobalTransform::from(eye), frustum(picture_sees));
        let world = (
            crate::world::camera::WorldCamera,
            Camera3d::default(),
            eye,
            GlobalTransform::from(eye),
            frustum(world_sees),
        );
        match picture_first {
            true => {
                app.world_mut().spawn(picture);
                app.world_mut().spawn(world);
            }
            false => {
                app.world_mut().spawn(world);
                app.world_mut().spawn(picture);
            }
        }
        let [entity, bone_joint, _, _] = one_rig(&mut app, 2.0);
        app.update();
        let moved = Vec3::new(10.0, 0.0, 0.0);
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Transform>()
            .expect("a placement")
            .translation = moved;
        app.update();
        let at = app
            .world()
            .entity(bone_joint)
            .get::<GlobalTransform>()
            .expect("a world transform")
            .translation();
        at == moved
    }

    /// One entity with a one-bone skeleton, an identity joint and a pauldron hung
    /// on the bone, in an app that runs [`animate`] and transform propagation.
    /// Returns the entity, its two joints and the pauldron.
    fn one_rig(app: &mut App, cull_radius: f32) -> [Entity; 4] {
        app.add_plugins(bevy::transform::TransformPlugin);
        app.init_resource::<Time>();
        // `animate` reads the display tables for the emote lookup. They are empty
        // here: this test is about where a joint lands, and without `Emotes.dbc`
        // every emote resolves to nothing.
        app.init_resource::<DisplayCache>();
        // `animate` also reads the drawn body heading, for the strafe twist. It is
        // empty here too: this entity is not strafing, so it takes no twist and
        // the pose is the untwisted one this test measures.
        app.init_resource::<crate::world::facing::BodyFacing>();
        app.add_systems(Update, animate);

        let entity = app
            .world_mut()
            .spawn((moving(7.6), Transform::default(), Visibility::default()))
            .id();
        let bone_joint = app
            .world_mut()
            .spawn((Joint, Bone(0), GlobalTransform::default(), ChildOf(entity)))
            .id();
        let identity_joint = app
            .world_mut()
            .spawn((Joint, Bone(1), GlobalTransform::default(), ChildOf(entity)))
            .id();
        // A pauldron: an ordinary drawn child. `animate` overwrites its local
        // transform, and propagation then composes its world transform.
        let pauldron = app
            .world_mut()
            .spawn((
                AttachedTo,
                Transform::default(),
                GlobalTransform::default(),
                ChildOf(entity),
            ))
            .id();
        app.world_mut().entity_mut(entity).insert((
            EntityModel {
                display_id: 0,
                dbc_scale: 1.0,
            conform: vale_assets::look::conform::Conform::Level,
            painted: None,
            faded: None,
                joints: vec![bone_joint, identity_joint],
                look: None,
                wanted: Vec::new(),
                attached: vec![AttachedPart {
                    bone: 0,
                    offset: [0.0; 3],
                    grounded: false,
                    scale: 1.0,
                    root: pauldron,
                    joints: Vec::new(),
                    skeleton: None,
                    sequence: 0,
                    loops: true,
                    since: 0.0,
                    tinted: Vec::new(),
                    tints: None,
                    points: Arc::new(Vec::new()),
                    nested: Vec::new(),
                    pending: Vec::new(),
                }],
                tinted: Vec::new(),
                tints: None,
                cast: EffectSet::default(),
                impact: EffectSet::default(),
                state: EffectSet::default(),
                milestone: EffectSet::default(),
                loot: EffectSet::default(),
                pushed: EffectSet::default(),
                hung: EffectSet::default(),
                pushed_for: None,
                state_auras: Vec::new(),
                effects_for: None,
                landed_for: None,
                points: Arc::new(Vec::new()),
                cues: Arc::new(Default::default()),
                head: 0.0,
            name_anchor: 0.0,
                anchor: 0.0,
                mounted_anchor: None,
                mounted_name: None,
                model_sphere: Default::default(),
                pick: Arc::new(Default::default()),
                room: None,
                sun: crate::render::models::sun_scale::NEUTRAL,
                lamps: Default::default(),
                shadow_radius: 0.5,
                cull_radius,
                roots: Vec::new(),
                hands: ([Weapon::default(); 3], 0),
            },
            Playback {
                hints: Vec::new(),
                window: None,
                wanted: u16::MAX,
                sequence: usize::MAX,
                since: 0.0,
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
                skeleton: one_still_bone(),
                played: None,
            },
            Sheath::seeded(0),
        ));
        [entity, bone_joint, identity_joint, pauldron]
    }

    /// The camera must reach `pose` in the model's own space. Otherwise a
    /// billboarded bone, such as a torch flame or a pair of eyes, faces the
    /// entity's heading instead of the viewer.
    #[test]
    fn the_camera_arrives_in_the_models_own_space() {
        // A camera looking along world north at a creature facing north: in the
        // creature's space the camera is straight ahead.
        let north = [1.0, 0.0, 0.0];
        let unturned = into_model_space(north, 0.0);
        assert!((unturned[0] - 1.0).abs() < 1e-6 && unturned[1].abs() < 1e-6);

        // Turn the creature a quarter turn counter-clockwise (to face west), and
        // the same world direction is on its right, at -90°.
        let turned = into_model_space(north, std::f32::consts::FRAC_PI_2);
        assert!(turned[0].abs() < 1e-6, "{turned:?}");
        assert!((turned[1] + 1.0).abs() < 1e-6, "{turned:?}");
    }

    /// A model hung on a rigid weapon's own point is written at that point in the
    /// weapon's space, not in the wearer's. The weapon's frame is passed in
    /// separately, so the glow's root carries only its offset along the blade.
    #[test]
    fn an_item_visual_hangs_on_the_weapons_own_point() {
        let mut world = World::new();
        let weapon_root = world.spawn_empty().id();
        let glow_root = world.spawn_empty().id();
        let mut weapon = AttachedPart::rigid(weapon_root, 1.0);
        let mut glow = AttachedPart::rigid(glow_root, 1.0);
        // Point 4 of `Axe_2H_Horde_D_01.m2`, at the end of the blade.
        glow.offset = [1.08, 0.0, 0.0];
        weapon.nested.push(glow);

        let mut writes = RigWrites::default();
        animate_attachment(
            &weapon,
            Affine3A::from_translation(Vec3::new(0.0, 1.0, 0.0)),
            None,
            0.0,
            0,
            &mut writes,
        );
        let roots = writes.pending_roots();
        assert_eq!(roots.len(), 1, "only the glow's root is written here");
        assert_eq!(roots[0].0, glow_root);
        assert!(
            roots[0]
                .1
                .translation
                .abs_diff_eq(crate::render::axes::to_bevy([1.08, 0.0, 0.0]), 1e-6),
            "{:?}",
            roots[0].1.translation
        );
    }

    // ---------------------------------------------------------------------------
    // Sheath state: the client-side state, the animation reconcile and the
    // sheath request
    // ---------------------------------------------------------------------------

    /// The sheath reconcile runs once per animation play, never once per frame. An
    /// implementation that reads the currently playing animation every frame gets
    /// this wrong without any error.
    ///
    /// The 1.12.1 client reconciles the sheath state only when an animation starts,
    /// so a frame in which no animation started leaves the weapons alone. Checking
    /// every frame makes a caster's staff oscillate: a moving cast's hold stows the
    /// weapon on the frame it takes the upper body, and the next frame's base gait
    /// plus the engaged draw pull the weapon back out, sixty times a second.
    #[test]
    fn a_play_is_latched_once_and_taken_once() {
        let mut play = playing(&[anim::STAND, anim::RUN]);
        // The first `advance` is a state change and therefore a play.
        play.advance(anim::STAND, 0.0);
        assert_eq!(play.take_played(), Some(anim::STAND));
        assert_eq!(play.take_played(), None, "taken, not read");

        // Ten frames of the same state are not ten plays.
        for frame in 1..10 {
            play.advance(anim::STAND, frame as f32 * 0.016);
        }
        assert_eq!(play.take_played(), None, "standing still is not a play");

        // A change of state is a play.
        play.advance(anim::RUN, 1.0);
        assert_eq!(play.take_played(), Some(anim::RUN));
    }

    /// A one-shot the model cannot play is still a play.
    ///
    /// The 1.12.1 client reconciles on the requested animation id, so a weaponless
    /// creature asked for `Attack2H` reconciles on that row's flags even though its
    /// skeleton resolved the clip to the unarmed swing. Latching after the fallback
    /// instead would hide the reconcile on the models that use fallbacks most.
    #[test]
    fn the_latch_carries_the_id_that_was_asked_for() {
        // A model with no attack sequence at all.
        let mut play = playing(&[anim::STAND]);
        let mut world = moving(0.0);
        world.weapons[0] = a_greatsword();
        let out = SHEATH_STATE_MELEE;
        note_sheathed(&mut play, &world, out, anim::STAND, 0.0);
        play.take_played();

        world.swings_thrown = 1;
        note_sheathed(&mut play, &world, out, anim::STAND, 1.0);
        assert_eq!(
            play.take_played(),
            Some(anim::ATTACK_2H),
            "the request, not whatever the skeleton fell back to"
        );
    }

    // ---------------------------------------------------------------------------
    // Mounts
    // ---------------------------------------------------------------------------

    /// A rider holds one pose. `Mount` (91) is a held pose that outranks the gait,
    /// the water and the air, because the mount underneath walks, swims and jumps.
    /// It does not outrank death, because dying dismounts.
    ///
    /// The test checks swimming and airborne as well as moving, because
    /// `wanted_animation` checks airborne first, then swimming, then moving. A test
    /// that only set `moving` would pass with the mount rule placed anywhere above
    /// the moving check.
    #[test]
    fn a_rider_holds_one_pose_over_the_gait_the_water_and_the_air() {
        let out = SHEATH_STATE_MELEE;

        let mut rider = moving(7.0);
        rider.mounted = true;
        assert_eq!(wanted_animation(&rider, out, None), anim::MOUNT);

        rider.swimming = true;
        assert_eq!(wanted_animation(&rider, out, None), anim::MOUNT);

        rider.swimming = false;
        rider.airborne = true;
        assert_eq!(wanted_animation(&rider, out, None), anim::MOUNT);

        // A held emote and a stand state, both of which the server can report on
        // a unit that is still mounted.
        rider.airborne = false;
        rider.stand_state = STAND_STATE_SIT;
        assert_eq!(
            wanted_animation(&rider, out, Some(anim::STAND)),
            anim::MOUNT
        );

        // Death dismounts, and the pose is Death.
        rider.dead = true;
        assert_eq!(wanted_animation(&rider, out, None), anim::DEATH);

        // Nothing changes for a unit that is not riding.
        let walker = moving(2.0);
        assert_eq!(wanted_animation(&walker, out, None), anim::WALK);
    }

    /// A character with a loot window open crouches over the body, and the clock
    /// holds the crouch rather than replaying the reach.
    ///
    /// The test makes three claims; the first two are measured from the models.
    /// `Loot` (50) is 500 ms at flags `0x1` in all sixteen character models, and
    /// none of them carries `LootHold` (188) or `LootUp` (189). So the clip is the
    /// downward part of a set of three whose other two parts 1.12 does not ship.
    /// Posed at nine phases, `HumanMale`'s skinned height runs
    /// `2.01 2.01 2.01 1.97 1.85 1.67 1.51 1.35 1.25` and does not rise again, so
    /// the last frame is the crouch. Letting the clock wrap would replay the
    /// descent, and the character would bob over the corpse.
    ///
    /// The third claim is the precedence, which is a design choice: above the stand
    /// state and the held emote, below locomotion. See `wanted_animation`.
    #[test]
    fn a_looting_character_holds_the_crouch_it_reached_in() {
        let mut looter = moving(0.0);
        looter.looting = true;
        assert_eq!(wanted_animation(&looter, 0, None), anim::LOOT);

        // Looting outranks a stand state and a held emote.
        looter.stand_state = crate::world::entities::pose::STAND_STATE_SIT_CHAIR;
        assert_eq!(wanted_animation(&looter, 0, None), anim::LOOT);
        assert_eq!(
            // 69 is `EmoteDance`, a held emote.
            wanted_animation(&looter, 0, Some(69)),
            anim::LOOT
        );

        // Looting ranks below locomotion. On a live server this case does not
        // occur, because vmangos releases the body on any movement packet with
        // `MOVEFLAG_MASK_MOVING`. If the window stays open during a step, the
        // character runs rather than crouching while it moves.
        let mut running = moving(7.6);
        running.looting = true;
        assert_eq!(wanted_animation(&running, 0, None), anim::RUN);

        // The hold: a frame long after the clip ends reads past the clip's end
        // rather than wrapping back to its start.
        let mut play = playing(&[anim::STAND, anim::LOOT]);
        let first = play.advance(anim::LOOT, 0.4);
        let later = play.advance(anim::LOOT, 9.0);
        assert!(later > first, "the clock runs on");
        assert!(
            later > 500,
            "…and past the end of the 500 ms clip rather than wrapping — \
             `M2Skeleton::pose` clamps, so the crouch is held"
        );

        // Closing the window returns the character to Stand.
        assert_eq!(wanted_animation(&moving(0.0), 0, None), anim::STAND);
    }

    /// `Loot` falls back to `Stand`, with nothing in between.
    ///
    /// All sixteen character models carry `Loot` and only a player loots, so this
    /// fallback is not expected to be used. Every seated pose in the table is
    /// authored for a chair rather than for a body on the ground, so substituting
    /// one would look worse than not crouching.
    #[test]
    fn a_model_with_no_loot_clip_simply_stands() {
        let mut play = playing(&[anim::STAND]);
        play.advance(anim::LOOT, 0.0);
        assert_eq!(play.clip().map(|c| c.id), Some(anim::STAND));
    }

    /// An effect on the ground keeps its facing while its wearer turns. This tests
    /// the arithmetic of `pose`'s grounded branch.
    ///
    /// The report was a rooted player whose roots turned with them as they spun on
    /// the spot, so the effect looked attached to the character rather than to the
    /// ground. The root is a child of the wearer, so Bevy composes the wearer's
    /// whole transform onto it, including the facing. Multiplying by the inverse
    /// rotation on the left cancels the facing.
    ///
    /// The test asserts the behaviour rather than the implementation: a direction
    /// fixed in the world stays fixed however the wearer turns. It also asserts
    /// the uniform-scale condition, because the cancellation relies on it.
    #[test]
    fn a_grounded_effect_keeps_its_facing_while_its_wearer_turns() {
        use bevy::math::{Mat4, Quat, Vec3};

        // The part's frame in the wearer's space: a yard forward at the feet.
        let local = Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0));
        let north = Vec3::new(0.0, 0.0, 1.0);

        let mut seen = Vec::new();
        for eighth in 0..8 {
            let facing = Quat::from_rotation_y(eighth as f32 * std::f32::consts::FRAC_PI_4);
            // The wearer, uniformly scaled: `T · R · S`, like an entity's placement.
            let wearer = Mat4::from_scale_rotation_translation(
                Vec3::splat(2.0),
                facing,
                Vec3::new(5.0, 0.0, -3.0),
            );
            // `pose`'s grounded branch: the inverse rotation applied on the left.
            let grounded = Mat4::from_quat(facing.inverse()) * local;
            let world = wearer * grounded;
            seen.push(world.transform_vector3(north).normalize());
        }

        // All eight point the same way in the world.
        for (i, direction) in seen.iter().enumerate() {
            assert!(
                direction.abs_diff_eq(north, 1e-5),
                "facing {i} turned the effect to {direction:?}"
            );
        }

        // Without the branch the effect turns with the wearer, which was the bug.
        let turned = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Vec3::ZERO,
        ) * local;
        assert!(
            !turned.transform_vector3(north).normalize().abs_diff_eq(north, 1e-3),
            "the ungrounded path is supposed to turn"
        );
    }

    /// A `SpellVisualKit` sent by the server plays that kit's own pose, and the
    /// first reading plays nothing.
    ///
    /// `SMSG_PLAY_SPELL_VISUAL` needs one lookup fewer than every other pose in
    /// this client: the packet carries a kit id, and that row's `animID` column is
    /// the animation. Kits 406 (food) and 438 (drink) both read `61` (`EmoteEat`),
    /// and vmangos sends one of them on every regeneration tick while a character
    /// sits eating or drinking. That is the whole eating animation.
    ///
    /// The first-reading rule stops a unit that walks into view mid-meal from
    /// playing an eat animation for a tick it was not seen for. Every other counter
    /// here has the same rule.
    #[test]
    fn a_pushed_kit_plays_its_own_pose_and_not_on_the_first_look() {
        const FOOD_KIT: u32 = 406;
        const EAT: u16 = 61;
        let kits = |kit: u32| match kit {
            FOOD_KIT => Some(EAT),
            _ => None,
        };

        let mut play = playing(&[anim::STAND, EAT]);
        let mut world = moving(0.0);
        // Already eating when first seen: the counter is recorded and nothing
        // plays.
        world.spell_visuals = 7;
        world.last_spell_visual = FOOD_KIT;
        play.note_actions(&world, 0, |_| None, kits, |_| CastAnimation::default(), anim::STAND, 0.0);
        play.advance(anim::STAND, 0.0);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id,
            anim::STAND,
            "a unit seen mid-meal was handed somebody else's dinner"
        );

        // The next tick is a real increment and plays the pose.
        world.spell_visuals = 8;
        play.note_actions(&world, 0, |_| None, kits, |_| CastAnimation::default(), anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        assert_eq!(play.skeleton.sequences[play.sequence].id, EAT, "the kit's pose did not play");

        // An impact counter increment names a kit in the same way and resolves
        // through the same table. A kit the table cannot resolve leaves the
        // running clip alone rather than substituting Stand. The check is made
        // within the clip's 500 ms, because after that the pose returns to the
        // state anyway and the assertion would pass for the wrong reason.
        world.spell_impacts = 1;
        world.last_spell_impact = 999;
        play.note_actions(&world, 0, |_| None, kits, |_| CastAnimation::default(), anim::STAND, 1.2);
        play.advance(anim::STAND, 1.2);
        assert_eq!(
            play.skeleton.sequences[play.sequence].id, EAT,
            "a kit the table cannot resolve interrupted the pose that was running"
        );
    }

    /// Benchmark of the pose pass over a crowd, on real skeletons.
    ///
    /// Ignored, because it needs the archives and prints a number rather than
    /// asserting one. Run it with `--ignored --nocapture`. `VALE_BENCH_RIGS` sets
    /// the crowd size (default 120, about a Stratholme street), and
    /// `VALE_GAMEDATA` sets the archive folder when the working directory is not
    /// the install. It builds the same `App` as the placement test above
    /// (`animate` alone, no camera, so every rig is posed) and advances the clock
    /// by hand so the tracks are sampled mid-window rather than at their first
    /// key. It prints the mean time per `app.update()` over the timed frames, which
    /// is the pass's own cost with nothing else in the schedule.
    #[test]
    #[ignore]
    fn bench_animate_over_a_crowd() {
        use std::time::{Duration, Instant};

        let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
            let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            here.join("..").join("..").join("Data").to_string_lossy().into_owned()
        });
        let mut assets = match vale_assets::Assets::open(&root) {
            Ok(assets) => assets,
            Err(e) => {
                eprintln!("no archives at {root}: {e}");
                return;
            }
        };
        let rigs: usize = std::env::var("VALE_BENCH_RIGS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        // A mix of model kinds: two character models and two creatures.
        let paths = [
            r"Character\Human\Male\HumanMale.m2",
            r"Character\Scourge\Male\ScourgeMale.m2",
            r"Creature\Wolf\Wolf.m2",
            r"Creature\Ghoul\Ghoul.m2",
        ];
        let mut skeletons: Vec<(String, Arc<M2Skeleton>)> = Vec::new();
        for path in paths {
            let Ok(bytes) = assets.read(path) else {
                eprintln!("  {path}: not in the archives, skipped");
                continue;
            };
            let m2 = vale_assets::world::m2::M2::parse(&bytes).expect("a model");
            let Some(skeleton) = m2.skeleton else {
                eprintln!("  {path}: no skeleton, skipped");
                continue;
            };
            let still = skeleton
                .bones
                .iter()
                .filter(|b| b.translation.is_none() && b.rotation.is_none() && b.scale.is_none())
                .count();
            eprintln!(
                "  {path}: {} bones, {} with no track, {} sequences",
                skeleton.bones.len(),
                still,
                skeleton.sequences.len()
            );
            skeletons.push((path.to_string(), Arc::new(skeleton)));
        }
        assert!(!skeletons.is_empty(), "nothing to pose");

        let mut app = App::new();
        app.init_resource::<Time>();
        app.init_resource::<DisplayCache>();
        app.init_resource::<crate::world::facing::BodyFacing>();
        app.add_systems(Update, animate);

        let mut joints_total = 0usize;
        for i in 0..rigs {
            let (_, skeleton) = &skeletons[i % skeletons.len()];
            let entity = app
                .world_mut()
                .spawn((
                    WorldEntity { guid: i as u64 + 1, ..standing() },
                    Transform::from_translation(Vec3::new(i as f32 * 2.0, 0.0, 0.0)),
                    Visibility::default(),
                ))
                .id();
            let joints: Vec<Entity> = (0..=skeleton.bones.len())
                .map(|i| {
                    app.world_mut()
                        .spawn((Joint, Bone(i as u32), GlobalTransform::default(), ChildOf(entity)))
                        .id()
                })
                .collect();
            joints_total += joints.len();
            app.world_mut().entity_mut(entity).insert((
                EntityModel {
                    display_id: 0,
                    dbc_scale: 1.0,
                    conform: vale_assets::look::conform::Conform::Level,
                    painted: None,
                    faded: None,
                    joints,
                    look: None,
                    wanted: Vec::new(),
                    attached: Vec::new(),
                    tinted: Vec::new(),
                    tints: None,
                    cast: EffectSet::default(),
                    impact: EffectSet::default(),
                    state: EffectSet::default(),
                    milestone: EffectSet::default(),
                    loot: EffectSet::default(),
                    pushed: EffectSet::default(),
                    hung: EffectSet::default(),
                    pushed_for: None,
                    state_auras: Vec::new(),
                    effects_for: None,
                    landed_for: None,
                    points: Arc::new(Vec::new()),
                    cues: Arc::new(Default::default()),
                    head: 0.0,
                    name_anchor: 0.0,
                    anchor: 0.0,
                    mounted_anchor: None,
                    mounted_name: None,
                    model_sphere: Default::default(),
                    pick: Arc::new(Default::default()),
                    room: None,
                    sun: crate::render::models::sun_scale::NEUTRAL,
                    lamps: Default::default(),
                    shadow_radius: 0.5,
                    cull_radius: f32::MAX,
                    roots: Vec::new(),
                    hands: ([Weapon::default(); 3], 0),
                },
                Playback::new(skeleton.clone(), 0.0),
                Sheath::seeded(0),
            ));
        }

        let step = Duration::from_micros(16_667);
        let tick = |app: &mut App| {
            app.world_mut().resource_mut::<Time>().advance_by(step);
            app.update();
        };
        for _ in 0..30 {
            tick(&mut app);
        }
        let frames = 300;
        let started = Instant::now();
        for _ in 0..frames {
            tick(&mut app);
        }
        let per_frame = started.elapsed().as_secs_f64() * 1e6 / frames as f64;
        eprintln!(
            "animate: {rigs} rigs, {joints_total} joints, {per_frame:.0} us per frame ({:.2} us per rig)",
            per_frame / rigs as f64
        );
    }

    /// A jump, frame by frame, as pose deltas. This is the diagnostic for "the
    /// model jitters or resets mid-air, then replays the second half after
    /// landing".
    ///
    /// It drives a real skeleton through `Playback` at 60 Hz over a real-length arc
    /// (`VALE_ARC_MS`, default 850: the mover's 825 ms rounded to its 25 ms tick),
    /// poses every frame as `animate` does, including the fade, and prints the
    /// largest bone movement between consecutive frames next to the playing clip.
    /// A pop is a frame whose delta is far above its neighbours'; the output shows
    /// the transition where it happens. Run with `--ignored --nocapture`.
    /// `VALE_JUMP_MODEL` selects the model (default `HumanMale`), and
    /// `VALE_JUMP_MOUNTED=1` drives it as a mount through `note_flight_only`.
    #[test]
    #[ignore]
    fn jump_pose_continuity() {
        use vale_assets::world::m2::{Blend, PoseLayers};
        const FRAME: f32 = 1.0 / 60.0;

        let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
            let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            here.join("..").join("..").join("Data").to_string_lossy().into_owned()
        });
        let mut assets = match vale_assets::Assets::open(&root) {
            Ok(assets) => assets,
            Err(e) => {
                eprintln!("no archives at {root}: {e}");
                return;
            }
        };
        let path = std::env::var("VALE_JUMP_MODEL")
            .unwrap_or_else(|_| r"Character\Human\Male\HumanMale.m2".to_string());
        let mounted = std::env::var("VALE_JUMP_MOUNTED").is_ok_and(|v| v == "1");
        let arc_ms: u32 = std::env::var("VALE_ARC_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(850);
        let bytes = assets.read(&path).expect("the model");
        let m2 = vale_assets::world::m2::M2::parse(&bytes).expect("a model");
        let skeleton = Arc::new(m2.skeleton.expect("a skeleton"));
        let mut play = Playback::new(Arc::clone(&skeleton), 0.0);

        let mut running = moving(7.0);
        running.mounted = mounted;
        let mut air = running.clone();
        air.airborne = true;
        air.jumping = true;
        let gait = |world: &WorldEntity| {
            if mounted {
                crate::world::entities::mount::gait(world)
            } else {
                wanted_animation(world, 0, None)
            }
        };

        let take_off = 0.25;
        let land = take_off + arc_ms as f32 / 1000.0;
        let mut previous: Option<Vec<[f32; 12]>> = None;
        let mut rows: Vec<(f32, u16, u16, u32, f32)> = Vec::new();
        for frame in 0..(3.0 / FRAME) as u32 {
            let now = frame as f32 * FRAME;
            let world = if now >= take_off && now < land { &air } else { &running };
            let state = gait(world);
            if mounted {
                play.note_flight_only(world, state, now);
            } else {
                note(&mut play, world, state, now);
            }
            let elapsed = play.advance(state, now);
            let layers = PoseLayers {
                blend: play.fade.as_ref().map(|fade| Blend {
                    sequence: fade.sequence,
                    elapsed_ms: play.fade_phase(fade, now),
                    weight: 1.0 - (now - fade.from) / crate::world::entities::FADE_SECS,
                }),
                twist: None,
                overlay: play.overlay_at(now),
            };
            let pose = skeleton.pose(play.sequence, elapsed, (now * 1000.0) as u32, None, layers);
            let delta = previous
                .as_ref()
                .map(|last| {
                    last.iter()
                        .zip(&pose)
                        .map(|(a, b)| {
                            // The translation column of the 3x4 affine.
                            let d = [a[3] - b[3], a[7] - b[7], a[11] - b[11]];
                            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                        })
                        .fold(0.0f32, f32::max)
                })
                .unwrap_or(0.0);
            previous = Some(pose);
            let id = skeleton.sequences.get(play.sequence).map_or(u16::MAX, |s| s.id);
            rows.push((now, play.wanted, id, elapsed, delta));
        }

        eprintln!("{path}{} — arc {arc_ms} ms, take-off at {take_off:.3}s, landing at {land:.3}s", if mounted { " (mounted)" } else { "" });
        eprintln!("   t      wanted  clip  phase   max bone move (yards/frame)");
        let mut ranked: Vec<usize> = (1..rows.len()).collect();
        ranked.sort_by(|a, b| rows[*b].4.total_cmp(&rows[*a].4));
        let spikes: std::collections::BTreeSet<usize> = ranked.iter().take(8).copied().collect();
        let mut last_clip = u16::MAX;
        for (i, (t, wanted, id, phase, delta)) in rows.iter().enumerate() {
            let transition = *id != last_clip;
            last_clip = *id;
            if transition || spikes.contains(&i) {
                eprintln!(
                    "  {t:5.3}  {wanted:5}  {id:5}  {phase:5}   {delta:.3}{}{}",
                    if transition { "   <- clip change" } else { "" },
                    if spikes.contains(&i) { "   <- spike" } else { "" }
                );
            }
        }
        let typical = {
            let mut all: Vec<f32> = rows.iter().map(|r| r.4).collect();
            all.sort_by(f32::total_cmp);
            all[all.len() / 2]
        };
        eprintln!("  median frame-to-frame move {typical:.3}; the eight largest are marked");
    }

    /// A mount at riding speed asks for `Sprint` and plays its `Run`, and its
    /// stride still has footfalls in it. Without `Sprint` among the gaits the
    /// footstep pass heard nothing from any mount at 60% or 100% speed.
    #[test]
    fn a_galloping_mount_still_has_a_stride() {
        let mut play = playing(&[anim::STAND, anim::RUN]);
        let mut skeleton = (*play.skeleton).clone();
        skeleton.sequences[1].move_speed = 7.0;
        play.skeleton = Arc::new(skeleton);
        play.speed = 14.0;
        play.advance(anim::SPRINT, 0.0);
        let (gait, _) = play.stride(0.1).expect("a stride at a gallop");
        assert_eq!(gait, anim::SPRINT);
    }

    /// A stowed weapon swings as the weapon, not as a fist: the first blow of a
    /// fight from a sheathed character was a punch.
    #[test]
    fn a_stowed_weapon_swings_as_the_weapon() {
        let mut armed = moving(0.0);
        armed.weapons[0] = a_greatsword();
        assert_eq!(swing(&armed, 0), anim::ATTACK_2H);
        // With nothing to hold, it is still a punch.
        assert_eq!(swing(&moving(0.0), 0), anim::ATTACK_UNARMED);
    }

    /// A standing landing gives way to a gait at once, and a running one to
    /// standing still; neither gives way to the other kind of state.
    #[test]
    fn a_landing_gives_way_when_the_legs_change() {
        use crate::world::entities::pose::landing_yields;
        assert!(landing_yields(anim::JUMP_END, anim::RUN));
        assert!(landing_yields(anim::JUMP_END, anim::SPRINT));
        assert!(!landing_yields(anim::JUMP_END, anim::STAND));
        assert!(landing_yields(anim::JUMP_LAND_RUN, anim::STAND));
        assert!(!landing_yields(anim::JUMP_LAND_RUN, anim::RUN));
        // Nothing else yields.
        assert!(!landing_yields(anim::ATTACK_1H, anim::RUN));

        // And in the playback: a landing still, then the run keys pressed.
        let mut play = playing(&[anim::STAND, anim::RUN, anim::JUMP_END]);
        play.oneshot = Some(OneShot { wanted: anim::JUMP_END, until: 1.0 });
        play.advance(anim::STAND, 0.1);
        assert_eq!(play.wanted, anim::JUMP_END, "still standing: the landing plays");
        play.advance(anim::RUN, 0.2);
        assert_eq!(play.wanted, anim::RUN, "moving: the landing gives way");
    }
}
