//! The art a spell puts on a unit: the wind-up, the impact and the aura.
//!
//! [`hang_model`] is the shared mechanism — it is what an *item* attachment uses
//! too, which is why it is `pub(crate)` and lives here rather than in
//! [`super::attach`] — and [`spell_effects`] is the policy over it: three
//! [`super::EffectSet`]s whose only difference is what sets their `until`.

use super::*;

/// The **floor** on how long a release or an impact stays on, for a model that
/// states no clip of its own.
///
/// Nothing in the tables says. `SpellVisualKit` names the models and the sound
/// and stops; what ends an effect is the **M2's own one-shot clip**, which is
/// what [`super::AttachedPart::clip_secs`] answers and what
/// [`super::EffectSet::want_until_played`] raises this to as each model lands.
/// This is left as the floor for the models that cannot say — one whose take
/// loops, and one with no skeleton at all.
///
/// **It used to be the whole answer and that was the bug.** A flat one and a
/// half seconds cuts `Spells\IceArmor_Low_Head.m2` off at half its 3,000 ms
/// clip and `Spells\ArcaneIntellect_Impact_Base.m2` at 1,500 of its 1,900 —
/// in both cases part-way down the transparency track that is the effect's own
/// fade — so the buff art vanished mid-dissolve instead of fading out. Both
/// numbers are `vale model`'s.
const RELEASE_SECS: f32 = 1.5;

/// The longest a **held** effect stays on with no release to end it.
///
/// A wind-up ends when `SMSG_SPELL_GO` arrives, and the two are separate packets
/// with nothing guaranteeing the second — an interrupted cast simply stops.
/// Without a ceiling a caster who is silenced mid-fireball would carry the glow
/// for the rest of the session. The cast bar's own length plus a beat is the
/// natural bound, and this covers a cast whose bar the server never stated.
const HOLD_GRACE_SECS: f32 = 1.0;

/// Hang one loaded model on a wearer's bone: the shared shape of a pauldron
/// and a spell glow.
///
/// The attachment gets a **root** child carrying its frame (written by
/// [`animate`] as `wearer bone × offset × scale`), and under that root the
/// drawn parts and — when the model has a skeleton — its **own** joints, posed
/// on its own clock. That second skeleton is not an indulgence: a torch's glow
/// plane rides a billboarded bone of the *torch's* skeleton, and a spell
/// effect's whole appearance is its own bone animation — frozen in bind pose,
/// the glow is a flat card and Arcane Explosion's dome stands at its full
/// 9-yard authored extent for its whole lifetime, which reads as the world
/// washed purple.
///
/// The model's particle emitters are spawned here too, anchored to its own
/// joints — for most spell effects (508 of the 743 readable ones carry
/// emitters, `vale particles`) they *are* the visible effect, and they were
/// never registered before this existed: Ice Armor drew sixteen vertices and
/// no snow. The emitters are owned by the root, so `retire_emitters` takes
/// them when the root goes.
pub(crate) fn hang_model(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    wearer: Entity,
    attached: &ModelAssets,
    bone: usize,
    offset: [f32; 3],
    scale: f32,
    // **Whether this model is standing on the floor**, which is the one thing
    // that decides whether its flat quads are drawn as geometry or draped over
    // the ground — see [`crate::render::decals`] for the rule and for the honest
    // statement of what the real client does. True for an effect hung at
    // `attach::BASE` and for a persistent area, which *is* a placement on the
    // ground; false for a helm, a pauldron and a missile in flight, none of
    // which has a floor to lie on.
    grounded: bool,
    // `room` is what the **wearer** is standing in, and every drawn part below
    // carries its colour as a `MeshTag` — a helm on a head the tavern has
    // darkened is lit by the tavern. `None` out of doors, and `None` for a
    // spell effect, which is an unlit glow.
    room: Option<RoomLight>,
    // …and the wearer's sun scale, on the same terms — see `SunScale`. The
    // neutral 1.0 for anything with no wearer to take it from.
    sun: f32,
    now: f32,
) -> AttachedPart {
    // No `EntityPart` on the root: it draws nothing, and the HUD's batch
    // count is a count of drawn things. `AttachedTo` alone carries both of
    // its duties — `animate` writes its `Transform` by it, and
    // `rebuild_changed_models` despawns it by it.
    // **The scale is on the root from the start**, not written in by
    // [`animate`] a frame later. For a wearer's attachment that is only a
    // cosmetic saving — one frame of a helm at the wrong size — but a *missile*
    // has no wearer and `animate` never visits it, so an unskinned projectile
    // would be drawn at scale 1 for its whole flight. See `missiles.rs`.
    let root = commands
        .spawn((
            AttachedTo,
            Transform::from_scale(Vec3::splat(scale)),
            Visibility::default(),
            ChildOf(wearer),
        ))
        .id();
    let joints: Vec<Entity> = if attached.skeleton.is_some() {
        (0..attached.joint_count)
            .map(|_| {
                commands
                    .spawn((Joint, GlobalTransform::default(), ChildOf(root)))
                    .id()
            })
            .collect()
    } else {
        Vec::new()
    };
    // The sequence an attached model plays: its own Stand (id 0), or whatever
    // it has — an effect model's single animation is not obliged to call
    // itself Stand.
    let (sequence, loops) = attached
        .skeleton
        .as_ref()
        .and_then(|s| {
            let index = s.best_sequence(&[anim::STAND])?;
            // A sequence loops when bit 0 of its flags is clear — a read of
            // the 5875 loader, the same rule `ParticleClip` applies.
            Some((index, s.sequences[index].flags & 1 == 0))
        })
        .unwrap_or((0, true));
    let mut tinted = Vec::new();
    for draw in &attached.draws {
        // **A flat ground quad is drawn by the projector instead**, when this
        // model is on the floor. It is a *root* entity rather than a child —
        // its mesh is in world space — so it is not despawned by the root going
        // and is retired by `render::decals::retire_decals` instead.
        if let (true, Some(quad)) = (grounded, draw.ground) {
            let decal = crate::render::decals::spawn_ground_decal(
                commands,
                meshes,
                root,
                joints.get(quad.bone as usize).copied(),
                &quad,
                draw.material.clone(),
            );
            // The same tag the mesh child would have carried: this is what makes
            // the spiral *fade out* rather than sit on the ground for ever.
            if let (Some(tint), Some(tints)) = (draw.tint, &attached.tints) {
                let window = attached
                    .skeleton
                    .as_ref()
                    .and_then(|s| s.sequences.get(sequence));
                commands
                    .entity(decal)
                    .insert(bevy::mesh::MeshTag(crate::render::models::tint_tag(
                        tints.sample_in(tint, window, 0, 0),
                    )));
                tinted.push(Tinted { part: decal, tint });
            }
            continue;
        }
        let mut part = commands.spawn((
            EntityPart,
            Mesh3d(draw.mesh.clone()),
            MeshMaterial3d(draw.material.clone()),
            Transform::default(),
            ChildOf(root),
        ));
        // **A tinted batch must be given a tag before it is ever drawn.** Its
        // material reads the word as `0xAARRGGBB`, so the default zero is a
        // fully transparent black — an effect that spends its first frame
        // invisible, which is the frame the eye is on it. Sampled at the
        // clock's own zero, which is this frame.
        //
        // **Otherwise it is the wearer's room**, on exactly the terms the
        // wearer's own batches take it (see `spawn_model`): the room-lit
        // material says only *which branch*, and the colour is per instance.
        // Inserted unconditionally so a later change of room is a tag rewrite
        // rather than an archetype move, and zero is the identity.
        match (draw.tint, &attached.tints) {
            (Some(tint), Some(tints)) => {
                let window = attached
                    .skeleton
                    .as_ref()
                    .and_then(|s| s.sequences.get(sequence));
                part.insert(bevy::mesh::MeshTag(crate::render::models::tint_tag(
                    tints.sample_in(tint, window, 0, 0),
                )));
                tinted.push(Tinted {
                    part: part.id(),
                    tint,
                });
            }
            _ => {
                part.insert(bevy::mesh::MeshTag(crate::render::models::instance_tag(
                    room, sun,
                )));
            }
        }
        // The model's own declared box, for the same reason an entity part
        // carries one: a skinned mesh's bind-pose box does not cover what the
        // animation reaches.
        if let Some(bounds) = attached.bounds {
            part.insert(bounds);
        }
        // **The batch's own bones, not the model's whole skeleton** — see
        // `models::skin_for`, which is the one place that decides it.
        if let Some(joints) = crate::render::models::skin_for(draw, &joints) {
            part.insert(SkinnedMesh {
                inverse_bindposes: attached.inverse_bindposes.clone(),
                joints,
            });
        }
    }
    if let Some(set) = &attached.particles {
        crate::render::particles::spawn_emitters(
            commands,
            meshes,
            set,
            root,
            // The emitter rides a bone of the *attached* model's skeleton;
            // one it does not have falls back to the attachment's frame.
            |_, def| match joints.get(def.bone as usize) {
                Some(joint) => crate::render::particles::Anchor::Joint(*joint),
                None => crate::render::particles::Anchor::Owner,
            },
            scale,
            None,
        );
    }
    // **The trails, and this is where a weapon gets its streak.** A ribbon
    // rides a bone of the *attached* model's own skeleton exactly as its
    // emitters do — an enchant glow's trail follows the blade because the
    // blade is a bone of the sword, not of the arm holding it.
    if let Some(set) = &attached.ribbons {
        crate::render::ribbons::spawn_ribbons(
            commands,
            meshes,
            set,
            root,
            |_, def| match joints.get(def.bone as usize) {
                Some(joint) => crate::render::particles::Anchor::Joint(*joint),
                None => crate::render::particles::Anchor::Owner,
            },
            scale,
        );
    }
    AttachedPart {
        bone,
        offset,
        grounded,
        scale,
        root,
        joints,
        skeleton: attached.skeleton.clone(),
        sequence,
        loops,
        since: now,
        tinted,
        tints: attached.tints.clone(),
    }
}

/// **What this unit had done, the last time the effects were looked at.**
///
/// Five counters rather than a tuple because there are five of them and three
/// are `u32`s that mean nothing alike; a `seen.3` in the middle of the policy
/// below is exactly the shape that lets a comparison drift onto the wrong field.
/// See `WorldEntity`'s own docs for what each one counts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct CastCounters {
    pub begun: u32,
    pub released: u32,
    pub cancelled: u32,
    pub delayed: u32,
    /// …of which this many were a **channel** beginning, which is the one begin
    /// that arrives *after* the release it belongs to.
    pub channelled: u32,
}

/// **The spells released since the caster was last looked at, oldest first.**
///
/// The ring is four deep and the counter says how many actually happened, so a
/// poll that saw six releases gets the last four — the depth is a renderer's
/// choice and is stated as one on [`vale_protocol::state::objects::Entity`].
///
/// The one-release case, which is nearly all of them, is a slice of length one
/// and no allocation either way.
fn released_spells(
    world: &WorldEntity,
    seen: u32,
) -> impl Iterator<Item = u32> + '_ {
    let count = world
        .casts_released
        .wrapping_sub(seen)
        .min(world.recent_spells.len() as u32) as usize;
    world.recent_spells[world.recent_spells.len() - count..]
        .iter()
        .copied()
}

/// Put a unit's spell effects on, and take them off again — all three sets.
///
/// **This is the visible half of the spell chain**, and it is separate from the
/// *pose* half in `Playback` on purpose: they are answered by different columns
/// of the same kit and either can exist without the other — Fireball's wind-up
/// is a pose with no models on the precast kit, and plenty of buffs are models
/// with no pose at all. Sharing one code path would have made "no animation"
/// silently mean "no effect".
///
/// The three questions, and each is asked of a different thing:
///
/// * **what this unit is doing** — its own cast counters, held or released.
/// * **what was done to it** — a spell landing on it, out of [`PendingImpacts`].
/// * **what is true of it** — the `stateKit`s of the auras it is carrying.
///
/// The lifetime is the whole of the work here. A cast is two counters
/// (`casts_begun`, `casts_released`) and a spell id, exactly as a swing is; what
/// the effects need on top is an *end*, which no packet supplies. See
/// [`RELEASE_SECS`], [`HOLD_GRACE_SECS`] — and, for the aura state, nothing,
/// because that one ends when the server stops saying the aura is there.
pub(super) fn spell_effects(
    mut commands: Commands,
    time: Res<Time>,
    displays: Res<DisplayCache>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    mut impacts: ResMut<PendingImpacts>,
    mut entities: Query<(Entity, &WorldEntity, &mut EntityModel)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Effects);
    let now = time.elapsed_secs();
    // Drained whole, and what this pass queues below stays for the next frame.
    // That is deliberate rather than incidental: the missile pass and this one
    // are both `.after(place_entities)` with no order between them, and a
    // *self*-cast queues its own victim from inside this very loop. Either way
    // the cost of missing the frame is 16 ms on an effect that lasts 1,500.
    let landed = std::mem::take(&mut impacts.0);

    for (entity, world, mut model) in &mut entities {
        // Field-by-field, because the sets are borrowed mutably while the
        // wearer's own attachment points are read.
        let EntityModel {
            cast,
            impact,
            state,
            milestone,
            loot,
            pushed,
            hung,
            state_auras,
            points,
            effects_for,
            landed_for,
            pushed_for,
            ..
        } = &mut *model;

        // --- …and what the server said outright, which names no spell at all ---
        //
        // `SMSG_PLAY_SPELL_VISUAL` and `SMSG_PLAY_SPELL_IMPACT` carry a
        // `SpellVisualKit` id and a guid. Every other visual in this file is
        // reached spell-first, so this is the one place `kit_effects` is asked —
        // see `vale_protocol::play::sound` for the two bodies.
        //
        // **First look adopts and acts on nothing**, exactly as the cast block
        // below does: a unit that streams into view while somebody is eating
        // beside it must not be handed their dinner.
        let seen_pushed = *pushed_for;
        *pushed_for = Some((world.spell_visuals, world.spell_impacts));
        if let Some((visuals, impacts)) = seen_pushed {
            if visuals != world.spell_visuals || impacts != world.spell_impacts {
                // The **visual** when both moved in one poll, on the same
                // argument the pose arm uses: what the unit did outranks what
                // was done to it.
                let kit = match visuals != world.spell_visuals {
                    true => world.last_spell_visual,
                    false => world.last_spell_impact,
                };
                if let Some(models) = displays
                    .tables()
                    .and_then(|tables| tables.kit_effects(kit).cloned())
                {
                    // **Restarted rather than layered**, which matters more here
                    // than anywhere else in this file: vmangos re-sends kit 406
                    // on every regeneration tick a character spends eating, so a
                    // set that added would grow a plate of food per tick.
                    pushed.clear(&mut commands);
                    pushed.want_until_played(models, now + RELEASE_SECS, points);
                }
            }
        }

        // --- what this unit is doing: the cast ---
        let seen = *effects_for;
        *effects_for = Some(CastCounters {
            begun: world.casts_begun,
            released: world.casts_released,
            cancelled: world.casts_cancelled,
            delayed: world.casts_delayed,
            channelled: world.casts_channelled,
        });
        // First look adopts the counters and acts on nothing: anything the
        // caster did before it was modelled is history, and a creature that
        // walks into view mid-cast must not be handed the glow of a cast that
        // began before it existed.
        if let Some(seen) = seen {
            let released = world.casts_released != seen.released;
            let begun = world.casts_begun != seen.begun;
            // **A channel is a release and a begin in that order**, inside one
            // poll — `SendSpellGo` then `SendChannelStart` — so the begin is the
            // later of the two and the one to obey. Without this the release
            // below wins by being written second, and an Evocation draws the
            // release it has not got instead of the channel art it has.
            let channelled = world.casts_channelled != seen.channelled;
            // **A refused, interrupted or cancelled cast takes its own art off
            // and puts nothing in its place**, which is the whole difference
            // between this and the two below. It is tested first because it is
            // the one that ends rather than starts; see
            // `WorldEntity::casts_cancelled`, and note that for an *instant* the
            // release art is already up, since this client draws both halves at
            // the press.
            if world.casts_cancelled != seen.cancelled {
                cast.clear(&mut commands);
            } else if begun || released {
                cast.clear(&mut commands);
                // **The release wins when both counters moved in one step**,
                // which is what an instant spell looks like: `SMSG_SPELL_GO`
                // with no `SMSG_SPELL_START` before it, or both inside one
                // 25 ms tick — **unless the begin was a channel starting**,
                // which is the one case where the begin is the later packet.
                if released && !channelled {
                    // **Every release in the batch, oldest first** — a poll can
                    // hold more than one and the first is not always the
                    // interesting one. Charge is the measurement: it releases
                    // 100 and then the Charge Stun (7922) it triggers, and
                    // reading only `last_spell` gave the stun, which states no
                    // caster models at all — so the red trail and the dust
                    // cloud were never asked for.
                    //
                    // **A release with nothing to draw does not take down what
                    // is up.** There is no such teardown in the reference: a
                    // cast hangs its own kit's models and a kit with none hangs
                    // nothing. Without this the stun would still wipe the trail
                    // one line after it was armed.
                    for spell in released_spells(world, seen.released) {
                        let Some(effects) = displays
                            .tables()
                            .and_then(|tables| tables.cast_effects(spell).cloned())
                            .filter(|e| !e.release.is_empty())
                        else {
                            continue;
                        };
                        // **The release plays itself out** — see
                        // [`RELEASE_SECS`], which is now only the floor.
                        cast.want_until_played(effects.release, now + RELEASE_SECS, points);
                    }
                } else if let Some(effects) = displays
                    .tables()
                    .and_then(|tables| tables.cast_effects(world.last_spell).cloned())
                {
                    // **The channel hangs the channel kit's models**, which is
                    // a different set from the wind-up's for the 45 visuals
                    // that state both — see
                    // `vale_assets::tables::spell::CastEffects::channel`.
                    // A channel with no models of its own keeps the wind-up's,
                    // which is what a spell stating only `channelKit` already
                    // has in `hold`.
                    let models = match channelled && !effects.channel.is_empty() {
                        true => effects.channel,
                        false => effects.hold,
                    };
                    cast.want(
                        models,
                        now + world.cast_time_ms as f32 / 1000.0 + HOLD_GRACE_SECS,
                        points,
                    );
                }
            } else if world.casts_delayed != seen.delayed {
                // **A pushback is not a new cast**, so it is `else if`: it
                // arms nothing and clears nothing, it moves the deadline of
                // what is already up. `SMSG_SPELL_DELAYED` is the only packet
                // that restates a cast's length after `SMSG_SPELL_START` has
                // stated it, and the art on the caster's hands was armed off
                // that one statement — so without this a Fireball knocked back
                // twice loses its glow a second before it is thrown.
                cast.until += world.last_cast_delay_ms as f32 / 1000.0;
            }
        }

        // --- …and what it hit, which is a different counter ---
        //
        // **The impact is the server's answer and what it *hit* is a different
        // question from what the caster did**, which is why this is not folded
        // into the block above even though both are about a release.
        //
        // The split was forced by a prediction that is now gone: `casts_released`
        // used to move at the press, when the hit list did not exist, so an
        // implicitly-aimed spell decided "nobody" a frame before `SMSG_SPELL_GO`
        // said otherwise — Cone of Cold lost its whole visible half that way and
        // Arcane Explosion its flash. Both counters move on the same packet now,
        // and the split is kept because the questions really are two: one draws
        // the caster, this one draws the burst on each victim.
        //
        // `casts_landed` is the wire's own release and nothing else writes it.
        // For every unit but ourselves the two counters move together, so this
        // costs one `u32` compare a frame and changes nothing about anybody else.
        let seen_landed = *landed_for;
        *landed_for = Some(world.casts_landed);
        if seen_landed.is_some_and(|seen| seen != world.casts_landed) {
            // **A spell with no projectile lands the instant it is released**, so
            // its impact is queued here rather than by the missile pass. That is
            // most of the game — 8,751 spells state an impact kit and 898 throw
            // anything — and it includes every self-cast buff, where the caster
            // is also the victim: Ice Armor's `IceArmor_Low_Head` is an impact on
            // the mage's own head, which is what the retail client draws and this
            // client drew as nothing at all.
            let throws = displays
                .tables()
                .is_some_and(|tables| tables.cast_missile(world.last_spell).is_some());
            if !throws {
                // **A cast that named nothing landed on its caster — but only if
                // the spell targets its caster.** A self-buff's hit list usually
                // carries the caster's own guid and sometimes carries nothing at
                // all, and the two must not be different answers: without a
                // fallback, exactly the buffs whose impact is the visible half
                // (Frost Armor's head, Dampen Magic's ring on the ground) resolve
                // to no victim and draw nothing.
                //
                // An **area** spell that caught nobody names nothing too, and it
                // is not the same case: with the fallback unconditional, every
                // Arcane Explosion cast into empty air burst
                // `ArcaneExplosion_Impact_Chest` on the mage's own chest.
                // `Spell.dbc`'s own implicit targets are what tell the two apart
                // — see `SpellVisuals::is_self_cast`.
                //
                // **And the list is walked whole.** `SMSG_SPELL_GO` names every
                // guid the cast connected with and the impact kit is owed to
                // each of them — an Arcane Explosion catching five creatures
                // flashes on five. Reading only the head of the list gave four
                // of the five nothing while every count reported success; see
                // `Entity::last_spell_targets`, which is the field this is.
                if world.last_spell_targets.is_empty() {
                    if displays
                        .tables()
                        .is_some_and(|t| t.is_self_cast(world.last_spell))
                    {
                        impacts.land(world.guid, world.last_spell);
                    }
                } else {
                    for victim in &world.last_spell_targets {
                        if *victim != 0 {
                            impacts.land(*victim, world.last_spell);
                        }
                    }
                }
            }
        }

        // --- what was done to it: the impact ---
        for (_, spell) in landed.iter().filter(|(guid, _)| *guid == world.guid) {
            impact.clear(&mut commands);
            let effects = displays
                .tables()
                .and_then(|tables| tables.cast_effects(*spell).cloned());
            if let Some(effects) = effects {
                // …and so does the burst, which is where the fade that was
                // being cut off actually lives: a self-cast buff's whole
                // visible half is its impact kit on its own caster.
                impact.want_until_played(effects.impact, now + RELEASE_SECS, points);
            }
        }

        // --- what is true of it: the auras it is carrying ---
        //
        // **Rebuilt whole when the set changes rather than diffed.** A unit
        // carries a handful of auras and the set changes seconds apart, so the
        // saving from a diff is nothing; what it would cost is the bug where a
        // buff refreshed in place re-hangs its glow over the one already on the
        // bone.
        //
        // **What is compared is the auras that state a visual, not the slots.**
        // Most auras are invisible and a unit in combat gains and loses them
        // constantly, so comparing the raw slot list would clear and rebuild
        // every glow on the unit — restarting its clock — each time a debuff
        // ticked on. Filtering first costs one hash lookup per slot and makes
        // "the set changed" mean "what is drawn changed".
        let visible: Vec<u32> = displays.tables().map_or_else(Vec::new, |tables| {
            world
                .auras
                .iter()
                .map(|aura| aura.spell)
                .filter(|spell| tables.aura_effects(*spell).is_some())
                .collect()
        });
        if *state_auras != visible {
            *state_auras = visible;
            state.clear(&mut commands);
            if let Some(tables) = displays.tables() {
                for spell in &*state_auras {
                    if let Some(models) = tables.aura_effects(*spell) {
                        // No clock: an aura's glow comes off when the aura does.
                        state.want(models.clone(), f32::INFINITY, points);
                    }
                }
            }
        }

        // --- expiry, and then whatever has finished loading ---
        for set in [
            &mut *cast,
            &mut *impact,
            &mut *state,
            &mut *milestone,
            &mut *loot,
            &mut *pushed,
            &mut *hung,
        ] {
            if now >= set.until && !set.is_empty() {
                set.clear(&mut commands);
            }
            set.spawn(
                &mut commands,
                &mut cache,
                &mut materials,
                &mut meshes,
                entity,
                points,
                now,
            );
        }
    }

}

/// **The level-up glow**, and the one visual in the game that no table names.
///
/// `SpellVisualEffectName.dbc` row **21** is called `HARDCODED Unit Level Up`
/// and points at `Spells\LevelUp\LevelUp.m2` — the name is the file saying so:
/// no spell, no `SpellVisual`, no kit reaches it. The client plays it off
/// `SMSG_LEVELUP_INFO` and nothing else, which is why this is a system of its
/// own rather than a row in the chain [`spell_effects`] walks.
///
/// Measured, so the numbers here are the model's rather than a guess:
/// `vale model 'Spells\LevelUp\LevelUp.m2'` reads 4 batches over three
/// textures, all additive and unlit, five particle emitters, and **one
/// sequence, `0..1867 ms`, one-shot** — with every batch's transparency track
/// fading out over exactly that span. So the clip is what ends it and
/// [`LEVELUP_SECS`] is that length with a beat for the emitters' last
/// particles.
///
/// Hung from [`attach::BASE`], the caster's feet, which is where every
/// painted-on-the-ground effect in the game is anchored — the model's own box
/// runs from −1.4 to +16.3 in z about that point, a ring on the floor and a
/// column above it.
pub(super) fn level_up(
    mut commands: Commands,
    time: Res<Time>,
    mut levelled: MessageReader<crate::interface::events::PlayerLevelUp>,
    mut player: Query<&mut EntityModel, With<crate::world::session::LocalPlayer>>,
) {
    // Read whatever arrived even with no player to put it on, or the glow lands
    // on the *next* character to log in.
    let levels = levelled.read().count();
    let Ok(mut model) = player.single_mut() else {
        return;
    };
    let now = time.elapsed_secs();
    if levels > 0 {
        let EntityModel { milestone, points, .. } = &mut *model;
        // Restarted rather than layered on a double level-up, which is the same
        // policy every other set here takes for a second event inside one clip.
        milestone.clear(&mut commands);
        milestone.want(
            vec![vale_assets::tables::spell::KitEffect {
                point: attach::BASE,
                path: LEVELUP_MODEL.to_string(),
                scale: 1.0,
            }],
            now + LEVELUP_SECS,
            points,
        );
    }
    // The set's own spawn and expiry are `spell_effects`', which walks all four
    // — so this system only ever *asks*.
}

/// `SpellVisualEffectName.dbc` row 21's model, with the `.mdl` the table names
/// turned into the `.m2` the archive holds.
const LEVELUP_MODEL: &str = "Spells\\LevelUp\\LevelUp.m2";

/// How long the level-up glow stays on — the clip's own 1,867 ms plus a beat
/// for its five emitters' last particles. Unlike [`RELEASE_SECS`] this is not a
/// flat guess over a population: there is exactly one model and it was measured.
const LEVELUP_SECS: f32 = 2.4;

/// **The sparkle over a body with loot still on it**, which is the second of
/// the two visuals no table names — see [`level_up`] for the first.
///
/// `SpellVisualEffectName.dbc` row **14** is called `HARDCODED Loot Art` and
/// points at `Particles\LootFX.mdl`; both the name and its sibling
/// `HARDCODED Unit Level Up` are looked up by the client itself, so the lookup
/// is the client's rather than a convention invented here. No
/// spell, no `SpellVisual` and no kit reaches either — the engine plays them
/// off its own conditions.
///
/// **Measured, so the shape of the effect is the file's rather than a guess**:
/// `vale model 'Particles\LootFX.m2'` reads **0 render batches**, two
/// textures (`Item\ObjectComponents\Weapon\Flare.blp` and `Spells\Star5A.blp` —
/// the golden flare and the star twinkles), four particle emitters, and **one
/// sequence, `3333..5733 ms`, flagged to loop**. So it is entirely emitters and
/// it *holds* rather than playing out, which is why this is a state with no
/// clock where the level-up is a one-shot with one.
///
/// ## The condition is two fields and there is nothing else in it
///
/// A **dead** unit carrying `UNIT_DYNFLAG_LOOTABLE` wears it; anything else
/// does not. There is deliberately no tap check, no distance, no "is my loot
/// window open" and no fade: the flag is **per-viewer** — vmangos strips it for
/// a player with no rights ("hide lootable animation for unallowed players") —
/// so it already answers "would a right-click do anything for *me*", and every
/// further test would be this client second-guessing an answer the server has
/// given. The falling edge takes the sparkle straight off, which is what
/// looting the last item looks like.
///
/// Hung from [`attach::BASE`] — attachment `0x13`, the same point the level-up
/// ding uses, and the effect attach's own last fallback so a model without it
/// lands on the unit's origin.
///
/// **Edge-driven rather than re-asserted.** `want` appends, so asking every
/// frame would pile a second, third and hundredth copy of the emitters onto one
/// corpse; the set's own emptiness is the memo, which also makes a rebuilt
/// model (`CarriedEffects` carries this set) keep the one it has instead of
/// growing another.
pub(super) fn loot_art(
    mut commands: Commands,
    displays: Res<crate::assets::GameAssets>,
    mut units: Query<(&crate::world::session::WorldEntity, &mut EntityModel)>,
) {
    let Ok(tables) = displays.display_tables() else {
        return;
    };
    let Some((path, scale)) = tables
        .spells()
        .and_then(|spells| spells.hardcoded(vale_assets::tables::spell::LOOT_ART))
    else {
        // No `SpellVisualEffectName` in the chain is no loot art, which is the
        // documented degradation every optional table here takes.
        return;
    };
    let (path, scale) = (path.to_string(), scale);
    for (world, mut model) in &mut units {
        let wanted = world.dead && world.lootable;
        let EntityModel { loot, points, .. } = &mut *model;
        match (wanted, loot.is_empty()) {
            // The rising edge, and only it — see the note on why this is not
            // re-asserted every frame.
            (true, true) => loot.want(
                vec![vale_assets::tables::spell::KitEffect {
                    point: attach::BASE,
                    path: path.clone(),
                    scale,
                }],
                // **No clock**: the sequence loops and the condition ends it.
                f32::INFINITY,
                points,
            ),
            (false, false) => loot.clear(&mut commands),
            _ => {}
        }
    }
}

/// The ground art of one persistent area, hanging off the object that is it.
///
/// The model rides the `DynamicObject` entity through the same [`AttachedPart`]
/// a pauldron and a missile do, so it poses, fades and sprays with no second
/// copy of any of that — and a Blizzard is almost entirely emitters, which is
/// why that matters here rather than being a tidiness argument.
#[derive(Component)]
pub struct AreaArt(pub(super) AttachedPart);

/// A `DynamicObject` whose spell names no ground model, or whose model will not
/// read. Marked so the lookup is not retried every frame, exactly as
/// [`super::NoModel`] is.
#[derive(Component)]
pub struct NoAreaArt;

/// **Draw the persistent areas** — a Blizzard, a Flamestrike, a Consecration, a
/// Rain of Fire.
///
/// The one thing the server puts in the world that has no display id. A
/// `DynamicObject` states a caster, a spell id and a radius and nothing else,
/// and what it looks like is three columns of the spell's own `SpellVisual` row
/// — see [`vale_assets::tables::spell::SpellVisuals::area`], which is a measurement
/// now where it used to be a fallback chain over the caster's five kits.
///
/// **The radius is not a scale**, and that is the one thing here worth stating
/// rather than assuming. `DYNAMICOBJECT_RADIUS` is what the *spell* covers; the
/// art is authored at its own size and `SpellVisualEffectName` carries its own
/// scale column. The client agrees for the case that matters: it tests
/// `DYNAMICOBJECT_BYTES` and takes the scale branch **only** when it is neither
/// 1 nor 2, and vmangos writes 1 into it for every area aura it creates
/// (`DynamicObject::Create`), so no area spell in this game ever reaches the
/// division. What the radius does drive is [`rain_impacts`], where it is a
/// distribution rather than a size.
pub(super) fn persistent_areas(
    mut commands: Commands,
    time: Res<Time>,
    displays: Res<DisplayCache>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    areas: Query<(Entity, &WorldEntity), (Without<AreaArt>, Without<NoAreaArt>)>,
) {
    let now = time.elapsed_secs();
    for (entity, world) in &areas {
        let Some((spell, radius)) = world.area else {
            continue;
        };
        let Some(tables) = displays.tables() else {
            // The tables have not been read yet — that is `spawn_models`'
            // first entity, and this is not the place to force it.
            continue;
        };
        let Some(area) = tables.spell_area(spell) else {
            commands.entity(entity).insert(NoAreaArt);
            continue;
        };
        let (path, scale, rain) = (area.path.clone(), area.scale, area.rain.clone());
        match cache.attached(&path, None, None, SceneLighting::NONE, &mut meshes, &mut materials) {
            Lookup::Loading => {}
            Lookup::Failed => {
                warn!("area art {path} for spell {spell} will not read");
                commands.entity(entity).insert(NoAreaArt);
            }
            Lookup::Ready(assets) => {
                // No wearer bone: the area *is* the placement, and the object's
                // own `Transform` is where the server put it.
                let part = hang_model(
                    &mut commands,
                    &mut meshes,
                    entity,
                    &assets,
                    0,
                    [0.0; 3],
                    scale,
                    true,
                    None,
                    crate::render::models::sun_scale::NEUTRAL,
                    now,
                );
                commands.entity(entity).insert(AreaArt(part));
                // …and the second half, for the ten visuals that have one. The
                // seed is the entity's own index so two Blizzards side by side
                // do not fall in step — the emitters' rule, for the emitters'
                // reason.
                if let Some(rain) = rain {
                    commands.entity(entity).insert(AreaRain {
                        path: rain.path,
                        rate: rain.rate,
                        radius,
                        // **Starts at 1.0, not 0.0.** The first impact is what
                        // says the spell went off; waiting a fifth of a second
                        // for it is a Blizzard whose first frame is empty.
                        accumulated: 1.0,
                        rng: (entity.to_bits() as u32).wrapping_mul(0x9E37_79B9) | 1,
                    });
                }
            }
        }
    }
}

/// The falling-impact procedural, held on the `DynamicObject` it belongs to —
/// `BlizzardObject`, see [`vale_assets::tables::spell::AreaRain`].
#[derive(Component)]
pub struct AreaRain {
    path: String,
    /// Impacts per second, the kit's own `charParamOne`.
    rate: f32,
    /// `DYNAMICOBJECT_RADIUS`, in yards — how far out an impact may land.
    radius: f32,
    /// The fractional carry between frames. **Fractional is the whole point**:
    /// a 0.7/s Lightning Cloud strikes once every one and a half seconds and an
    /// integer-per-frame reading would give it either none or sixty.
    accumulated: f32,
    rng: u32,
}

/// One falling impact, alive on its own clock.
///
/// It is an entity of its own rather than a child of the area because its
/// **position is not the area's**: it lands somewhere inside the radius, on
/// whatever the ground there is. Carrying [`AreaArt`] means [`animate_areas`]
/// poses and sprays it with no second copy of any of that.
#[derive(Component)]
pub struct AreaImpact {
    /// When it is taken off, in `Time::elapsed_secs`.
    until: f32,
}

/// How long an impact lives when its model states no sequence length.
///
/// The reference frees a shard on its model's own animation-finished callback,
/// so the clip *is* the lifetime and this is only the fallback for
/// a model that will not tell us one. The same argument and the same order of
/// magnitude as [`RELEASE_SECS`].
const IMPACT_FALLBACK_SECS: f32 = 1.5;

/// The longest an impact is held, whatever its clip says.
///
/// `Flamestrike_Impact_Base`'s Stand runs **twelve seconds**, and a model with a
/// clip that long is authored to *be* the area rather than to fall into it — no
/// kit rains one, but the ceiling is here so that a rate and a clip length
/// cannot multiply into an unbounded population if one ever does.
const IMPACT_MAX_SECS: f32 = 6.0;

/// The stagger on an impact's own clock, in seconds — `0..0.51`.
///
/// The reference's own: a shard's start time is
/// `now + 2 * (int)(rand01() * 255)` milliseconds. It matters most at the low
/// rates, where the accumulator alone would land every strike on a frame
/// boundary and the whole area would tick like a metronome.
const IMPACT_STAGGER_SECS: f32 = 0.51;

/// **Rain the impacts** — a Blizzard's shards, a Rain of Fire's meteors, a
/// Hurricane's gusts.
///
/// One instance of the kit's model per whole unit of `dt * rate`, each at
/// `centre + radius * r * direction` for a **uniform `r`** — which piles them up
/// towards the middle rather than spreading them evenly over the disc, and is
/// the reference's own arithmetic rather than a simplification of it. The `sqrt` that would make it area-uniform is exactly
/// what is *not* there.
///
/// **The floor is the mover's own floor**, through the same `Standing::floor`
/// join the character walks on and the ground-target pointer picks with — so a
/// Blizzard cast on a bridge lands on the bridge. A column with no floor at all
/// (a hole, or a tile that has not streamed in) drops that impact rather than
/// putting it at the area's own height, which is the one-sided behaviour
/// `ground_under_ray` already takes.
#[allow(clippy::too_many_arguments)]
pub(super) fn rain_impacts(
    mut commands: Commands,
    time: Res<Time>,
    session: Res<crate::world::session::Session>,
    solids: Res<crate::world::session::Solids>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    mut areas: Query<(&Transform, &mut AreaRain)>,
) {
    if areas.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let standing = active.standing(&solids);
    let map_id = active.map_id;
    for (placement, mut rain) in &mut areas {
        rain.accumulated += dt * rain.rate;
        // **Capped per frame, not per area.** A tab-out that hands back a
        // two-second delta would otherwise spawn thirty shards on one frame,
        // all of which then die together; the reference has the same loop and
        // the same exposure, and a client that stutters should drop impacts
        // rather than pile them.
        let mut budget = MAX_IMPACTS_PER_FRAME;
        while rain.accumulated >= 1.0 && budget > 0 {
            rain.accumulated -= 1.0;
            budget -= 1;
            // Model space is WoW's here: the offset is picked in the game's own
            // xy and converted once, with the ground answering in its own axes.
            let radius = rain.radius;
            let (dx, dy) = impact_offset(&mut rain.rng, radius);
            let centre = crate::axes::to_wow(placement.translation);
            let (x, y) = (centre[0] + dx, centre[1] + dy);
            use vale_protocol::socket::session::World;
            // From above the area's own height, for the same reason the
            // reference starts its trace 16.7 yards up: the floor under the
            // edge of a Blizzard on a slope can be well over the centre.
            let Some(z) = standing.floor(map_id, x, y, centre[2] + IMPACT_TRACE_UP) else {
                continue;
            };
            let path = rain.path.clone();
            let Lookup::Ready(assets) =
                cache.attached(&path, None, None, SceneLighting::NONE, &mut meshes, &mut materials)
            else {
                // Loading, or unreadable: this impact is simply not spawned.
                // Putting the area on `NoAreaArt` here would take its *model*
                // off too, and the two are different files for eight of the ten.
                continue;
            };
            let stagger = crate::render::particles::rand01(&mut rain.rng) * IMPACT_STAGGER_SECS;
            let at = crate::axes::to_bevy([x, y, z]);
            let impact = commands
                .spawn((
                    Transform::from_translation(at),
                    // Written here as well as by Bevy's own propagation: this is
                    // a root with no parent, and `render::decals` reads the
                    // carrier's `GlobalTransform` in the same schedule this
                    // spawn happens in.
                    GlobalTransform::from_translation(at),
                    Visibility::default(),
                ))
                .id();
            // The stagger is a *late start* on the model's own clock rather
            // than a deferred spawn: `animate_attachment` saturates a negative
            // elapsed to zero, so the shard sits at frame 0 until its moment.
            let part = hang_model(
                &mut commands,
                &mut meshes,
                impact,
                &assets,
                0,
                [0.0; 3],
                1.0,
                true,
                None,
                crate::render::models::sun_scale::NEUTRAL,
                now + stagger,
            );
            // **The clip `hang_model` chose**, not sequence 0 — the two differ
            // for a model whose Stand is not its first take, and a lifetime read
            // off the wrong one either cuts the shard off mid-fall or leaves it
            // standing. The same question the effect sets ask, so the same
            // answer: [`super::AttachedPart::clip_secs`].
            let life = part
                .clip_secs()
                .unwrap_or(IMPACT_FALLBACK_SECS)
                .min(IMPACT_MAX_SECS);
            commands.entity(impact).insert((
                AreaArt(part),
                AreaImpact {
                    until: now + stagger + life,
                },
            ));
        }
        // A frame that ran out of budget must not carry the backlog forward for
        // ever — see `MAX_IMPACTS_PER_FRAME`.
        if budget == 0 {
            rain.accumulated = rain.accumulated.min(1.0);
        }
    }
}

/// Where one impact lands, relative to the area's centre, in **WoW** xy yards.
///
/// **`r` is uniform in `0..radius` and there is no `sqrt`**, which is the whole
/// of what this function is for. The reference multiplies a single `rand01` by
/// the radius and by a unit direction, so the impacts
/// pile up towards the middle of the area — mean radius `R/2` against the `2R/3`
/// an area-uniform disc gives. That is a *visible* difference at a Blizzard's
/// eight yards and it is the reference's, not a simplification of it: the
/// obvious "correct" version spreads the snow out into a ring.
pub(super) fn impact_offset(rng: &mut u32, radius: f32) -> (f32, f32) {
    let angle = crate::render::particles::rand01(rng) * std::f32::consts::TAU;
    let reach = crate::render::particles::rand01(rng) * radius;
    (reach * angle.cos(), reach * angle.sin())
}

/// How many impacts one area may spawn in one frame. See [`rain_impacts`].
const MAX_IMPACTS_PER_FRAME: u32 = 4;

/// How far above the area's own height the floor is looked for, in yards — the
/// reference's own 16.67.
const IMPACT_TRACE_UP: f32 = 16.666_666;

/// Take an impact off when its clip has run.
pub(super) fn retire_impacts(
    mut commands: Commands,
    time: Res<Time>,
    impacts: Query<(Entity, &AreaImpact)>,
) {
    let now = time.elapsed_secs();
    for (entity, impact) in &impacts {
        if now >= impact.until {
            // `despawn` takes the attachment root and its parts with it; the
            // ground decals it owns are `render::decals::retire_decals`', which
            // is the same division of labour every other attachment takes.
            commands.entity(entity).despawn();
        }
    }
}

/// Drop every impact when the world goes.
///
/// An impact is a **root** entity and so is not swept by the entity teardown,
/// which walks what the server told us about — the same exposure the emitters
/// and the decals have, and the same answer.
pub(super) fn forget_impacts(
    mut commands: Commands,
    mut leaving: MessageReader<crate::interface::events::PlayerLeavingWorld>,
    impacts: Query<Entity, With<AreaImpact>>,
) {
    if leaving.read().next().is_none() {
        return;
    }
    for entity in &impacts {
        commands.entity(entity).despawn();
    }
}

/// Pose, fade and spray every persistent area on its own clock.
///
/// Separate from [`super::animate`] because a `DynamicObject` has no skeleton
/// of its own and therefore no [`super::Playback`] — that system's query would
/// never match it. The frame handed down is the object's placement **times the
/// art's own scale**, which is the root's local transform: an unskinned part
/// gets that scale from Bevy's propagation, but a skinned one's joints replace
/// its world matrix outright, so the scale has to be in the matrix the joints
/// are composed against or a scaled effect draws at 1.
pub(crate) fn animate_areas(
    time: Res<Time>,
    camera: Query<&GlobalTransform, (With<crate::world::camera::WorldCamera>, Without<Joint>)>,
    areas: Query<(&Transform, &AreaArt)>,
    mut joints: Query<&mut GlobalTransform, With<Joint>>,
    mut tags: Query<&mut bevy::mesh::MeshTag, With<EntityPart>>,
) {
    if areas.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    let now_ms = (now * 1000.0) as u32;
    let camera = camera.iter().next().copied();
    let mut writes = super::pose::RigWrites::default();
    for (placement, area) in &areas {
        animate_attachment(
            &area.0,
            placement.compute_affine() * Affine3A::from_scale(Vec3::splat(area.0.scale)),
            camera.as_ref(),
            now,
            now_ms,
            &mut writes,
        );
    }
    writes.apply(&mut joints, &mut tags);
}

/// Spells that have **landed** and are waiting to burst on their victim.
///
/// A queue rather than a call because the two ends are two passes: a projectile
/// arrives in `missiles::fly_missiles` and the burst hangs on a *different*
/// entity, which that pass has no business reaching into. It is also what makes
/// the ordering between the two passes not matter — [`spell_effects`] drains
/// what it finds and anything queued after it has already run simply waits a
/// frame, which is 16 ms on an effect that lasts 1,500.
///
/// **Keyed by guid rather than by `Entity`.** The victim may be rebuilt between
/// the launch and the arrival — new gear, a change of room — and the burst
/// belongs to the unit rather than to the model it was wearing at the time.
#[derive(Resource, Default)]
pub struct PendingImpacts(Vec<(u64, u32)>);

impl PendingImpacts {
    /// A spell arrived: burst it on `victim`.
    pub(crate) fn land(&mut self, victim: u64, spell: u32) {
        self.0.push((victim, spell));
    }

    /// What is queued — for the missile test, which is about what a projectile
    /// hands over rather than about what is done with it.
    #[cfg(test)]
    pub(crate) fn landed(&self) -> &[(u64, u32)] {
        &self.0
    }
}

