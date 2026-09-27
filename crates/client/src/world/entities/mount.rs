//! What a unit is **riding**, and where that puts the rider.
//!
//! `UNIT_FIELD_MOUNTDISPLAYID` is the wire's whole statement about mounting: a
//! `CreatureDisplayInfo` id, resolved through the same hop every other model in
//! the world takes. Everything else here is the client's, because nothing in
//! any packet or any file says how a rider and a mount are joined.
//!
//! ## Three things, and only the third is new
//!
//! **The model** is an ordinary creature, loaded through the ordinary cache and
//! hung through the ordinary [`hang_model`] — the same call a pauldron and a
//! spell glow take.
//!
//! **The gait** is what makes it more than an attachment. An [`AttachedPart`]
//! plays one fixed sequence on its own clock, which is right for a helm and
//! wrong for a horse: a mount walks, runs, swims and stands, off the *rider's*
//! motion, and it needs the whole of [`Playback`]'s clock — the cross-fade, the
//! fallback chain, and the `speed / move_speed` rate scaling that is what stops
//! a galloping horse's feet skating. So it carries a real `Playback`, driven by
//! the same `WorldEntity` the rider's is.
//!
//! **The seat** is [`attach::SADDLE`] — attachment point 0 of the mount's own
//! model, measured, and the only thing in any file that says where a rider
//! goes. It reaches the rider as **one affine**: `pose::animate` composes every
//! joint, every attached model, every glow and every ground decal through a
//! single `world_from_entity`, so lifting the character onto the horse is that
//! matrix and nothing else. Getting it anywhere but there is how the body ends
//! up on the saddle with its pauldrons still on the ground.
//!
//! ## What is approximated, and it is stated rather than hidden
//!
//! The seat takes the saddle bone's **translation** and not its rotation. A
//! mount's spine pitches as it gallops and the reference hangs the rider off
//! the full attachment frame, so a rider here stays level where the reference's
//! leans. Translation is the half that is measurable from the file with no
//! reference client to compare against, and getting the rotation wrong tips the
//! character over — which is the plausibly-wrong failure this project counts as
//! worse than an approximation that is written down.
//!
//! The rider keeps its own blob shadow at ground level, which is where the
//! mount is standing, and the mount is given none of its own. And the mouse
//! pick box is still the *rider's*, at the rider's ground position — so a
//! mounted player is clicked where the horse is rather than where the body is.
//!
//! ## …and the camera is deliberately not on the posed seat
//!
//! [`ride`] returns the seat as the animal is standing *this frame*, which is
//! what the rider has to be drawn on. The camera takes [`Mount::seat`] instead —
//! the same point in the mount's **bind** pose — because a gallop pitches the
//! spine every stride and a focus hung off the drawn saddle takes the whole
//! world up and down with the hooves. See [`Mount::camera_anchor`], which is
//! both halves of where a third-person rig looks at a rider.

use super::*;

/// The mount a unit is on: a second animated model, its own clock, and the
/// seat it puts the rider in.
///
/// A component on the **rider**, not an entity of its own, because every
/// question about it is a question about the rider — where it is, which way it
/// faces, how fast it is going, and whether it is still there. The drawn half
/// hangs off [`Self::root`], which is a child of the rider so that a unit
/// streaming out of the world takes its horse with it.
#[derive(Component)]
pub struct Mount {
    /// The `UNIT_FIELD_MOUNTDISPLAYID` this was built for. A change of mount is
    /// a rebuild, exactly as a change of `DISPLAYID` is for the rider.
    display_id: u32,
    /// The one child of the rider that everything drawn hangs under.
    ///
    /// Despawning it takes the mount's batches, its joints, its emitters and
    /// its trails — the same contract [`AttachedPart::root`] has, which is what
    /// it was built by.
    root: Entity,
    /// The mount's **own** joints, in bone order with the identity joint last.
    joints: Vec<Entity>,
    /// `modelScale * displayScale` for the mount, out of its own
    /// `CreatureDisplayInfo` row.
    ///
    /// **The mount's, not the rider's.** The rider's `Transform` carries the
    /// rider's scale (a tauren is 1.35), and a kodo does not get bigger for
    /// being sat on by one — so every frame this composes is built at the ratio
    /// of the two and the product comes out as the mount's own.
    scale: f32,
    /// Attachment point 0 of the mount's model — `(bone, offset in that bone's
    /// frame)`. `None` for a display id that resolves to a model with no such
    /// point, which is what a creature nobody rides looks like: the rider then
    /// sits at the mount's own origin, on the floor between its feet, which is
    /// visibly wrong rather than plausibly wrong.
    saddle: Option<(usize, [f32; 3])>,
    /// What it is playing, on the rider's motion — see the module doc.
    play: Playback,
    /// A sphere in model yards covering the mount's own declared box, on the
    /// same terms as [`EntityModel::cull_radius`]: the pose gate in
    /// [`super::pose::animate`] takes the larger of the two, because a horse is
    /// bigger than the gnome on it and a rig whose parts the camera can draw
    /// must never ride a pose the gate declined to compute.
    cull_radius: f32,
    /// **Whether the animal leans with the hill it is on** — the *mount's* flag
    /// and never the rider's, which is the whole of why a mounted character
    /// tilts at all. See [`vale_assets::look::conform`]: `Horse.m2` reads 1 and
    /// `HumanMale.m2` reads 0, and the rider is carried by the saddle rather
    /// than conforming on its own.
    pub(super) conform: vale_assets::look::conform::Conform,
    /// The drawn parts whose colour is animated, and the model's tracks —
    /// almost always empty, as on any other creature. See [`Tinted`].
    tinted: Vec<Tinted>,
    tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
    /// **How far above the ground the saddle is, in world yards** — the mount's
    /// own bind-pose point 0, times the mount's own scale.
    ///
    /// **Static, taken at the build and never rewritten**, which is the whole
    /// difference between this and the seat [`ride`] composes each frame. An
    /// `M2Attachment`'s recorded position is already a bind-pose model-space
    /// point (see `spawn::anchor_height`), so this is the saddle where the
    /// animal is *standing* — and it is the camera's, because a camera hung off
    /// the posed saddle rises and falls with every stride of the gallop and
    /// takes the whole world with it. That is the "being mounted causes the
    /// camera to bounce up and down" report, and nothing else was moving.
    ///
    /// Zero for a model with no point 0, which is a creature nobody rides.
    pub seat: f32,
    /// **The animal's own drawn triangles and its own header sphere**, for the
    /// mouse pick — see [`vale_assets::look::pick`] and
    /// [`crate::interface::target::hover`].
    ///
    /// Held here because the mount is not a `WorldEntity` of its own: it is a
    /// second rig on the rider's entity, so nothing in the pick's query would
    /// otherwise see it, and a click on a horse's flank would go straight
    /// through to whatever is behind it. That is not hypothetical — it is what
    /// the box this pick replaces got right by accident, being tall enough to
    /// cover both.
    pick: Arc<vale_assets::look::pick::PickMesh>,
    model_sphere: vale_assets::look::pick::Sphere,
    /// …and where that rig stood the last time [`ride`] posed it: the same
    /// `world_from_mount` the joints were composed against.
    ///
    /// Written where it is computed rather than recomposed by the reader,
    /// because it is the rider's placement *times the ground conform times the
    /// scale ratio* — three terms, one of which is smoothed per frame — and a
    /// second composition of it is a second answer to where the horse is.
    placed: Mat4,
}

/// The root a [`Mount`]'s drawn half hangs from.
///
/// Its own marker rather than [`AttachedTo`] — which is what [`hang_model`]
/// gives it and which this replaces — because the two are despawned by
/// different things. An `AttachedTo` root is a pauldron or a glow and belongs
/// to the *dressing*, so `rebuild_changed_models` takes it down whenever the
/// wearer's model stops describing them; a mount belongs to the unit, survives
/// a re-dressing and a shapeshift alike, and comes off only when the server
/// says the unit is no longer riding.
#[derive(Component)]
pub(super) struct MountRoot;

/// A mount display id that will not read, so the lookup is not retried every
/// frame.
///
/// Carries the id rather than being a bare marker, so that a rider who swaps
/// one mount for another is tried again — the same shape as [`NoModel`], with
/// the addition that the thing it is refusing can change under it.
#[derive(Component)]
pub(super) struct NoMount(u32);

/// Give every riding unit its mount, and take it off every unit that has
/// dismounted.
///
/// **Before [`super::pose::animate`]**, which is what poses it: the mount's own
/// joints and the seat it puts the rider in are one composition and belong in
/// the frame the rider's joints are written in, not a frame behind it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn spawn_mounts(
    mut commands: Commands,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    mut displays: ResMut<DisplayCache>,
    assets: Res<GameAssets>,
    time: Res<Time>,
    entities: Query<(
        Entity,
        &WorldEntity,
        Option<&Indoors>,
        Option<&SunScale>,
        Option<&Mount>,
        Option<&NoMount>,
    )>,
) {
    let now = time.elapsed_secs();
    for (entity, world, indoors, shade, mount, refused) in &entities {
        let wanted = world.mount_display_id;
        // Still riding the same thing: nothing to do, which is every frame of
        // every ride after the first.
        if let (Some(mount), Some(wanted)) = (mount, wanted) {
            if mount.display_id == wanted {
                continue;
            }
        }
        // Dismounted, or swapped one mount for another. The root takes the
        // whole drawn half with it.
        if let Some(mount) = mount {
            commands.entity(mount.root).despawn();
            commands.entity(entity).remove::<Mount>();
        }
        let Some(wanted) = wanted else {
            // A refusal is about a display id, so dismounting clears it.
            if refused.is_some() {
                commands.entity(entity).remove::<NoMount>();
            }
            continue;
        };
        if refused.is_some_and(|r| r.0 == wanted) {
            continue;
        }
        // The same `CreatureDisplayInfo` hop every model in the world takes —
        // a mount is a creature and there is no second table for it.
        let Some(display) = displays.resolve(&assets, ObjectType::Unit, wanted) else {
            commands.entity(entity).insert(NoMount(wanted));
            continue;
        };
        // A transport is a `.wmo` and cannot be a mount, but the field is a
        // display id and nothing stops one naming a building. Refused rather
        // than handed to a cache that only reads M2s.
        if display.path.to_ascii_lowercase().ends_with(".wmo") {
            commands.entity(entity).insert(NoMount(wanted));
            continue;
        }
        let room = indoors.and_then(|i| i.0);
        // A creature with its own baked skins and nothing worn — the same call
        // `spawn_models` makes for any NPC, minus the hair a character-model
        // NPC needs.
        let ready = cache.dressed(
            &display.path,
            &display.skins,
            None,
            vale_assets::world::m2::Dress::Creature,
            room,
            &mut meshes,
            &mut materials,
        );
        let model = match ready {
            // The loader thread has it; it lands in a frame or two, exactly as
            // a pauldron does.
            Lookup::Loading => continue,
            Lookup::Failed => {
                commands.entity(entity).insert(NoMount(wanted));
                continue;
            }
            Lookup::Ready(model) => model,
        };
        // **No skeleton, no mount.** Every mount in 1.12 is an animated
        // creature; one without a skeleton would have no joints to write, so
        // its parts would be drawn at the *rider's* transform and scale —
        // a horse the size of the gnome on it, standing inside them. Refusing
        // is the visible failure rather than the plausible one.
        let Some(skeleton) = model.skeleton.clone() else {
            let path = &display.path;
            warn!("mount display {wanted} ({path}) has no skeleton");
            commands.entity(entity).insert(NoMount(wanted));
            continue;
        };
        let part = hang_model(
            &mut commands,
            &mut meshes,
            entity,
            &model,
            // The bone and the offset an attachment would ride: neither is used
            // here, because a mount hangs off nothing — it *is* the frame. The
            // seat runs the other way, from the mount to the rider.
            0,
            [0.0; 3],
            // The mount's own scale is composed into the frame this writes each
            // joint against (see [`ride`]), never onto the root: the root's
            // scale would be multiplied by the rider's, which is the wrong
            // product for every rider whose scale is not 1.
            1.0,
            // A mount stands on the ground; it is not a flat effect lying on
            // it, which is what this switch is about.
            false,
            room,
            // A mount stands where its rider stands, so it takes the rider's
            // scale — see `SunScale`.
            shade.map_or(crate::render::models::sun_scale::NEUTRAL, |s| s.now()),
            now,
        );
        // **The one divergence from an attachment, and it is about lifetime.**
        // `hang_model` marks its root `AttachedTo`, which is what
        // `rebuild_changed_models` despawns when a wearer's dressing changes —
        // and a mount must survive that. Swapped here rather than passed in
        // because it is the only caller of eleven that wants a different
        // answer. Both commands land at the same sync point, in this order, so
        // nothing ever observes the root wearing the wrong marker.
        commands
            .entity(part.root)
            .remove::<AttachedTo>()
            .insert(MountRoot);
        let saddle = model
            .attachments
            .iter()
            .find(|p| p.id == attach::SADDLE)
            .map(|p| (p.bone as usize, p.position));
        commands
            .entity(entity)
            .insert(Mount {
                display_id: wanted,
                root: part.root,
                joints: part.joints,
                scale: display.scale,
                seat: saddle.map_or(0.0, |(_, p)| p[2] * display.scale),
                saddle,
                play: Playback::new(skeleton, now),
                cull_radius: model
                    .bounds
                    .map(|b| Vec3::from(b.center).length() + Vec3::from(b.half_extents).length())
                    .unwrap_or(f32::MAX),
                conform: model.conform,
                pick: Arc::clone(&model.pick),
                model_sphere: model.model_sphere,
                // Identity until the first `ride`, which is the same frame:
                // `spawn_mounts` runs inside the entity chain and `ride` is
                // further down it.
                placed: Mat4::IDENTITY,
                tinted: part.tinted,
                tints: part.tints,
            })
            .remove::<NoMount>();
    }
}

/// Pose the mount for this frame and answer **where it puts the rider**, as an
/// affine in the rider's own local space.
///
/// The product the caller wants is `placement × seat`, which is what
/// `pose::animate` uses as its one `world_from_entity` — so the whole character
/// and everything hanging off it moves together, by construction. Identity
/// would leave the rider on the floor.
///
/// `bone_camera` is the camera's right and up axes expressed in the *rider's*
/// model space, which is also the mount's: the two share a facing and differ
/// only by a uniform scale, and a uniform scale does not turn a direction.
#[allow(clippy::too_many_arguments)]
pub(super) fn ride(
    mount: &mut Mount,
    world: &WorldEntity,
    placement: &Transform,
    // **The lean the animal is standing at**, in the rider's local frame and
    // already decided by the *mount's* own model flag — see `super::conform`.
    // It goes into the mount's own matrix and comes back out in the seat, which
    // is what tips the rider with the horse rather than leaving them sitting
    // level a foot above a tilted saddle.
    conform: Affine3A,
    bone_camera: Option<([f32; 3], [f32; 3])>,
    now: f32,
    now_ms: u32,
    writes: &mut super::pose::RigWrites,
) -> Affine3A {
    // **The rider's speed is the mount's speed**, and it is the whole of what
    // drives the gait — a mount has no motion of its own on the wire and never
    // will, because the server moves the *rider*.
    mount.play.speed = world.speed;
    let state = gait(world);
    // **The jump's two ends, which are the animal's and not the rider's.**
    // `Mount` (91) outranks the air in `wanted_animation`, so a rider sits still
    // for the whole arc and the horse plays it — which means the horse owes the
    // `JumpStart` and the `JumpEnd` as well as the `Jump` in between. Both are in
    // `Creature\Horse\Horse.m2`; neither had ever been reached. Before `advance`,
    // on the same terms `pose::animate` calls `note_actions` before it.
    mount.play.note_flight_only(world, state, now);
    let elapsed = mount.play.advance(state, now);

    // The mount's frame, in the rider's local space: a uniform scale that turns
    // the rider's scale back out and the mount's own in. Composed with the
    // caller's placement it comes to `T · R · S(mount scale)`, which is where
    // a creature of that display id would stand if the server had sent it.
    let rider_scale = placement.scale.x.max(1e-4);
    let mount_local = Affine3A::from_scale(Vec3::splat(mount.scale / rider_scale));
    // …with the lean between the placement and the animal, so the whole rig —
    // the horse's own bones and, through the seat below, the rider and
    // everything hanging off it — turns together.
    let world_from_mount = placement.compute_affine() * conform * mount_local;
    // Kept for the mouse pick, which needs the animal's own frame and must not
    // recompose it — see the field.
    mount.placed = Mat4::from(world_from_mount);

    let pose = mount.play.skeleton.pose(
        mount.play.sequence,
        elapsed,
        now_ms,
        bone_camera,
        PoseLayers {
            blend: mount.play.fade.as_ref().map(|fade| Blend {
                sequence: fade.sequence,
                // Held at its last frame for a one-shot that ran out — the
                // horse's own take-off — see `Playback::fade_phase`.
                elapsed_ms: mount.play.fade_phase(fade, now),
                weight: 1.0 - (now - fade.from) / FADE_SECS,
            }),
            // Neither of the other two layers applies. A strafe turns the
            // *body* into the slide and the mount is the body; the masked
            // upper-body track is what a rider does over the mount's gait and
            // belongs to the rider's own skeleton.
            twist: None,
            overlay: None,
        },
    );
    for (bone, &joint) in pose.iter().zip(mount.joints.iter()) {
        writes.joint(joint, world_from_mount * axes::pose_to_bevy_affine(bone));
    }
    // The identity joint, as on any other rig: the mount's own space, unposed.
    if let Some(&last) = mount.joints.last() {
        writes.joint(last, world_from_mount);
    }
    // The fade, for the handful of mounts that have one. Written only when
    // the byte moved — the same instance re-upload the wearer's tint loop
    // avoids — which `RigWrites::apply` decides.
    if let Some(tints) = &mount.tints {
        let window = mount.play.skeleton.sequences.get(mount.play.sequence);
        for one in &mount.tinted {
            writes.tag(
                one.part,
                crate::render::models::tint_tag(tints.sample_in(one.tint, window, elapsed, now_ms)),
            );
        }
    }

    // The *drawn* seat, which is the posed one: the rider is meant to move with
    // the saddle it is sitting on. Only the camera wants the standing height
    // ([`Mount::seat`]), and it reads that instead.
    //
    // **The lean is on the outside**, because `seat_offset` is a point in the
    // animal's own (already leaning) frame: composing it the other way round
    // would tilt the rider about the ground rather than about the saddle and
    // slide them off the back of a horse on a hill.
    conform * Affine3A::from_translation(seat_offset(mount, &pose, rider_scale))
}

/// Where the saddle is, in the **rider's** local frame — the translation half
/// of [`ride`]'s answer, and the only part of it this client claims to know.
///
/// A `Vec3` and not a matrix on purpose: see the module doc for why the bone's
/// rotation is left out and what that costs.
impl Mount {
    /// **What the mouse pick has to test besides the rider**: the animal's own
    /// broad-phase sphere, the frame it was last posed in, its triangles and
    /// its joints.
    ///
    /// One accessor rather than four, because a caller that had three of them
    /// and the fourth from somewhere else would be composing a horse out of
    /// parts of two frames.
    pub fn pick_target(
        &self,
    ) -> (
        vale_assets::look::pick::Sphere,
        Mat4,
        &Arc<vale_assets::look::pick::PickMesh>,
        &[Entity],
    ) {
        // **The clip it is playing**, on the same rule as the rider's — see
        // `EntityModel::pick_sphere`. A galloping horse's box is wider than a
        // standing one's and the file states both.
        let sphere = vale_assets::look::pick::sequence_sphere(self.model_sphere, self.play.clip());
        (sphere, self.placed, &self.pick, &self.joints)
    }

    /// **Whose feet are on the ground while a character is riding**: the
    /// animal's display id and where in its own stride it is, or `None` when it
    /// is not travelling on the ground.
    ///
    /// Both halves are the mount's and neither is the rider's, which is the
    /// whole of "mounted running has no footstep sounds":
    ///
    /// * **the stride** — a mounted character plays `Mount` (91), which is not
    ///   one of the three travelling gaits [`Playback::stride`] answers for, so
    ///   the rider's clock says the legs are still and it is *right*. They are.
    ///   The animal's clock is the one with feet in it.
    /// * **the display id** — `CreatureSoundData`'s footstep column is keyed by
    ///   display, so asking the rider's gives a human's boots at a horse's
    ///   cadence. A hoof is not a boot and the table says so.
    ///
    /// The two-footfalls-per-cycle approximation `sound/footsteps` is built on
    /// is *more* wrong here than on a biped and stated in that module: a horse
    /// has four feet and a gallop is not a symmetric two-beat.
    pub fn footfall(&self, now: f32) -> Option<(u32, f32)> {
        let (_, fraction) = self.play.stride(now)?;
        Some((self.display_id, fraction))
    }

    /// A mount with a saddle and nothing else — no entities, no model, no
    /// archive.
    ///
    /// For the seat arithmetic, which is the load-bearing half of this module
    /// and is about two scales and a point. Building the real thing needs a
    /// loaded M2 and a running `App`; the sum being checked needs neither.
    #[cfg(test)]
    pub(super) fn seated(scale: f32, saddle: Option<(usize, [f32; 3])>) -> Mount {
        Mount {
            display_id: 0,
            root: Entity::PLACEHOLDER,
            joints: Vec::new(),
            scale,
            seat: saddle.map_or(0.0, |(_, p)| p[2] * scale),
            saddle,
            play: Playback::new(
                Arc::new(M2Skeleton::new(Vec::new(), Vec::new(), Vec::new())),
                0.0,
            ),
            cull_radius: 0.0,
            pick: Arc::new(Default::default()),
            model_sphere: Default::default(),
            placed: Mat4::IDENTITY,
            conform: vale_assets::look::conform::Conform::Level,
            tinted: Vec::new(),
            tints: None,
        }
    }

    /// **Where the camera looks at whoever is riding this**, in world yards
    /// above the ground the mount is standing on.
    ///
    /// Two constants and nothing from the frame being drawn: the saddle where
    /// the animal *stands* ([`Self::seat`], already world yards and the mount's
    /// own scale) and the rider's anchor **inside the `Mount` clip**, which is
    /// the rider's and takes the rider's scale.
    ///
    /// **The obvious sum is the bug.** Adding the standing anchor to the saddle
    /// puts the focus most of a yard above the rider's own head, because the
    /// clip folds the body down onto the animal — `HumanMale`'s posed extent
    /// goes from `0.00..2.0` standing to `-0.85..1.06` seated, so a rider on a
    /// horse spans 1.02..2.93 and the naive sum aims at 3.77. That is the
    /// "mounted human on horse has a very high anchor point that is above the
    /// character's head" report, and the test below is written against the
    /// product rather than the arithmetic for exactly that reason.
    ///
    /// A rider whose model has no `Mount` clip keeps its standing anchor, which
    /// is the only other height it has.
    pub(crate) fn camera_anchor(&self, rider: &EntityModel, rider_scale: f32) -> f32 {
        self.seat + rider.mounted_anchor.unwrap_or(rider.anchor) * rider_scale
    }

    /// The mount's own culling sphere, expressed in the **rider's** model
    /// yards — the units `pose::animate`'s gate compares in, since it scales
    /// the result by the rider's transform. See [`Self::cull_radius`].
    pub(super) fn world_radius(&self, rider_scale: f32) -> f32 {
        self.cull_radius * (self.scale / rider_scale.max(1e-4))
    }
}

pub(super) fn seat_offset(mount: &Mount, pose: &[[f32; 12]], rider_scale: f32) -> Vec3 {
    let Some((bone, offset)) = mount.saddle else {
        return Vec3::ZERO;
    };
    let Some(matrix) = pose.get(bone) else {
        // A model may name a bone it does not have; reading past the pose would
        // give some other bone's matrix rather than failing.
        return Vec3::ZERO;
    };
    // The point in the mount's own model space, then out of it by the same
    // ratio [`ride`] composes the mount's frame at — so the product with the
    // rider's placement is `mount scale × the point`, whatever the rider is.
    let point = axes::pose_to_bevy(matrix).transform_point3(axes::to_bevy(offset));
    point * (mount.scale / rider_scale.max(1e-4))
}

/// Which gait a mount is in, from the rider's motion.
///
/// **A deliberately smaller set than [`super::pose::wanted_animation`]'s**, and
/// the difference is not an oversight: a mount does not die, sit, kneel, emote,
/// hold a stance, cast, flinch or draw a weapon. Everything the server says
/// about a mounted unit that is *not* travel belongs to the rider — the mount
/// underneath goes on walking. Routing a mount through the unit rules would
/// have a horse lie down when its rider is killed and sit in a chair when its
/// rider sits in one, which are both states the server can report while the
/// mount is still attached.
pub(super) fn gait(world: &WorldEntity) -> u16 {
    // **The one the taxi needs**, and it is first here as it is first in
    // [`super::pose::wanted_animation`]: a gryphon on a flight path is flying,
    // whatever the rider's other flags say. Without it the mount plays `Run` at
    // the ride's 32 y/s over `Run`'s authored 6.9 — the "flies super fast"
    // report, which was the mount's clip rather than the rider's.
    if world.move_flags & move_flags::FLYING != 0 {
        return anim::FLY;
    }
    // The air and the water outrank the gait for the same reason they do for a
    // unit: each is a different pose rather than a variant of running.
    if world.airborne {
        return if world.jumping {
            anim::JUMP
        } else {
            anim::FALL
        };
    }
    let travelling = world.moving && world.speed >= MOVING_FLOOR;
    if world.swimming {
        return if travelling {
            anim::SWIM
        } else {
            anim::SWIM_IDLE
        };
    }
    if travelling {
        if world.move_flags & move_flags::BACKWARD != 0 {
            return anim::WALK_BACKWARDS;
        }
        // **A gallop is a gait and therefore is here**, where the states above
        // it are not: the reference has one cascade for a unit and the
        // Sprint arm sits in it beside the Run/Walk split, not
        // among the poses a mount has no business taking. An epic mount is
        // 100% speed — 14 y/s against the threshold's 11 — so this is the
        // ordinary case rather than a corner, and 0 of the 411 creature models
        // carry `Sprint`, so every one of them takes the fallback column to
        // `Run` and looks exactly as it did. See [`anim::SPRINT`].
        if world.speed >= anim::SPRINT_SPEED {
            return anim::SPRINT;
        }
        return if world.speed >= RUN_SPEED {
            anim::RUN
        } else {
            anim::WALK
        };
    }
    anim::STAND
}
