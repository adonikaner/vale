//! The art a spell puts on a unit: the wind-up, the impact and the aura.
//!
//! [`hang_model`] is the shared mechanism. Item attachments use it too, so it
//! is `pub(crate)`; it lives here rather than in [`super::attach`].
//! [`spell_effects`] is the policy over it: [`super::EffectSet`]s that differ
//! only in what sets their `until`.

use super::*;

/// The minimum time a release or an impact stays on, for a model that states
/// no clip of its own.
///
/// The tables do not state a duration. `SpellVisualKit` names the models and
/// the sound only. What ends an effect is the M2's own one-shot clip, which
/// [`super::AttachedPart::clip_secs`] returns and
/// [`super::EffectSet::want_until_played`] raises this to as each model loads.
/// This value remains the minimum for models that state no clip: one whose
/// take loops, and one with no skeleton.
///
/// Used alone, a flat 1.5 seconds cuts `Spells\IceArmor_Low_Head.m2` off at
/// half its 3,000 ms clip and `Spells\ArcaneIntellect_Impact_Base.m2` at
/// 1,500 of its 1,900 ms, both part-way through the transparency track that is
/// the effect's own fade, so the buff art vanished instead of fading out. Both
/// clip lengths are from `vale model`.
const RELEASE_SECS: f32 = 1.5;

/// How long a held effect outlasts its cast bar when no release ends it.
///
/// A wind-up ends when `SMSG_SPELL_GO` arrives. That is a separate packet and
/// is not guaranteed: an interrupted cast just stops. Without a limit, a caster
/// silenced mid-Fireball would keep the glow for the rest of the session. The
/// limit is the cast bar's length plus this grace, which also covers a cast
/// whose bar length the server never stated.
const HOLD_GRACE_SECS: f32 = 1.0;

/// Hang one loaded model on a wearer's bone: the shared shape of a pauldron
/// and a spell glow.
///
/// The attachment gets a root child carrying its frame (written by [`animate`]
/// as `wearer bone × offset × scale`). Under that root are the drawn parts
/// and, when the model has a skeleton, its own joints, posed on its own clock.
/// The second skeleton is needed: a torch's glow plane rides a billboarded
/// bone of the torch's skeleton, and a spell effect's appearance is its own
/// bone animation. In bind pose the glow is a flat card, and Arcane
/// Explosion's dome stays at its full 9-yard authored size for its whole
/// lifetime, tinting the whole view purple.
///
/// The model's particle emitters are spawned here too, anchored to its own
/// joints. For most spell effects (508 of the 743 readable ones carry
/// emitters, `vale particles`) the emitters are the visible effect; without
/// them Ice Armor draws sixteen vertices and no snow. The root owns the
/// emitters, so `retire_emitters` removes them when the root is despawned.
pub(crate) fn hang_model(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    wearer: Entity,
    attached: &ModelAssets,
    bone: usize,
    offset: [f32; 3],
    scale: f32,
    // Whether this model stands on the floor. This alone decides whether its
    // flat quads are drawn as geometry or projected onto the ground; see
    // [`crate::render::decals`] for the rule and for what the 1.12.1 client is
    // known to do. True for an effect hung at `attach::BASE` and for a
    // persistent area, which is itself a placement on the ground. False for a
    // helm, a pauldron and a missile in flight, which have no floor to lie on.
    grounded: bool,
    // `room` is the room the wearer stands in. Every drawn part below carries
    // its colour as a `MeshTag`, so a helm on a head the tavern has darkened is
    // lit by the tavern. `None` outdoors, and `None` for a spell effect, which
    // is an unlit glow.
    room: Option<RoomLight>,
    // The wearer's sun scale, on the same terms; see `SunScale`. The neutral
    // 1.0 for anything with no wearer.
    sun: f32,
    now: f32,
) -> AttachedPart {
    // No `EntityPart` on the root: it draws nothing, and the HUD's batch
    // count counts drawn things. `AttachedTo` alone serves both purposes:
    // `animate` finds the root by it to write its `Transform`, and
    // `rebuild_changed_models` finds it by it to despawn it.
    // The scale is set on the root at spawn, not written by [`animate`] a
    // frame later. For a wearer's attachment that only avoids one frame of a
    // helm at the wrong size. A missile has no wearer and `animate` never
    // visits it, so an unskinned projectile would otherwise be drawn at scale
    // 1 for its whole flight. See `missiles.rs`.
    let root = commands
        .spawn((
            AttachedTo,
            Transform::from_scale(Vec3::splat(scale)),
            Visibility::default(),
            ChildOf(wearer),
        ))
        .id();
    // A weapon's trail points, for a melee ability's kit to draw between.
    if let Some(points) = attached.trail {
        commands.entity(root).insert(super::procedural::Blade(points));
    }
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
    // The sequence an attached model plays: its own Stand (id 0), or else
    // whatever it has, since an effect model's single animation need not be
    // a Stand.
    let (sequence, loops) = attached
        .skeleton
        .as_ref()
        .and_then(|s| {
            let index = s.best_sequence(&[anim::STAND])?;
            // A sequence loops when bit 0 of its flags is clear, as in the
            // 5875 client; `ParticleClip` applies the same rule.
            Some((index, s.sequences[index].flags & 1 == 0))
        })
        .unwrap_or((0, true));
    let mut tinted = Vec::new();
    for draw in &attached.draws {
        // When this model is on the floor, a flat ground quad is drawn by the
        // decal projector instead. The decal is a root entity rather than a
        // child, because its mesh is in world space, so despawning the
        // attachment root does not remove it; `render::decals::retire_decals`
        // does.
        if let (true, Some(quad)) = (grounded, draw.ground) {
            let decal = crate::render::decals::spawn_ground_decal(
                commands,
                meshes,
                root,
                joints.get(quad.bone as usize).copied(),
                &quad,
                draw.material.clone(),
            );
            // The same tag the mesh child would have carried. It makes the
            // spiral fade out rather than stay on the ground indefinitely.
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
        // A tinted batch must get a tag before it is first drawn. Its material
        // reads the tag as `0xAARRGGBB`, so the default zero is fully
        // transparent black, and the effect would be invisible on its first
        // frame. The tint is sampled at the clock's zero, which is this frame.
        //
        // An untinted batch gets the wearer's room tag, as the wearer's own
        // batches do (see `spawn_model`): the room-lit material selects only
        // indoor or outdoor lighting, and the colour is per instance. The tag
        // is always inserted, so a later change of room is a tag rewrite
        // rather than an archetype move; zero is the identity.
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
        // The batch's own bones, not the model's whole skeleton; see
        // `models::skin_for`, which decides this for every caller.
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
            // The emitter rides a bone of the attached model's skeleton;
            // one it does not have falls back to the attachment's frame.
            |_, def| match joints.get(def.bone as usize) {
                Some(joint) => crate::render::particles::Anchor::Joint(*joint),
                None => crate::render::particles::Anchor::Owner,
            },
            scale,
            None,
        );
    }
    // The ribbon trails, which give a weapon its streak. A ribbon rides a bone
    // of the attached model's own skeleton, as its emitters do: an enchant
    // glow's trail follows the blade because the blade is a bone of the
    // sword, not of the arm holding it.
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

/// The unit's cast counters as of the last time its effects were checked.
///
/// Named fields rather than a tuple because the five counters are all `u32`s
/// with unrelated meanings, and a `seen.3` in the policy below makes it easy
/// to compare the wrong field. See `WorldEntity`'s docs for what each counts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct CastCounters {
    pub begun: u32,
    pub released: u32,
    pub cancelled: u32,
    pub delayed: u32,
    /// How many of the begins were a channel starting, the one begin that
    /// arrives after the release it belongs to.
    pub channelled: u32,
}

/// The spells released since the caster was last checked, oldest first.
///
/// The ring holds four and the counter says how many releases happened, so a
/// poll that saw six releases gets the last four. The depth is a renderer
/// choice, documented on [`vale_protocol::state::objects::Entity`].
///
/// The usual single release is a slice of length one; neither case allocates.
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

/// Add a unit's spell effects and remove them again, for every effect set.
///
/// This is the model half of the spell visuals. It is separate from the pose
/// half in `Playback` because the two come from different columns of the same
/// kit and either can exist without the other: Fireball's wind-up is a pose
/// with no models on the precast kit, and many buffs are models with no pose.
/// With one code path, "no animation" would also have meant "no effect".
///
/// Each of the three main sets reads a different source:
///
/// * what this unit is doing: its own cast counters, held or released.
/// * what was done to it: a spell landing on it, from [`PendingImpacts`].
/// * what is true of it: the `stateKit`s of the auras it carries.
///
/// Most of the work is the lifetime. A cast is two counters (`casts_begun`,
/// `casts_released`) and a spell id, like a swing. The effects also need an
/// end time, which no packet supplies. See [`RELEASE_SECS`] and
/// [`HOLD_GRACE_SECS`]. The aura state has no end time; it ends when the
/// server stops listing the aura.
pub(super) fn spell_effects(
    mut commands: Commands,
    time: Res<Time>,
    displays: Res<DisplayCache>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    mut impacts: ResMut<PendingImpacts>,
    mut entities: Query<(
        Entity,
        &WorldEntity,
        &mut EntityModel,
        &mut super::procedural::Procedurals,
    )>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Effects);
    let now = time.elapsed_secs();
    // The procedurals' clocks run in milliseconds over sessions of hours, past
    // an `f32`'s whole-millisecond range.
    let now_precise = time.elapsed_secs_f64();
    let spells = displays.tables().and_then(|tables| tables.spells());
    // Drained whole; what this pass queues below waits for the next frame.
    // This is intended: the missile pass and this one are both
    // `.after(place_entities)` with no order between them, and a self-cast
    // queues its own victim from inside this loop. Either way, missing the
    // frame delays an effect that lasts 1,500 ms by 16 ms.
    let landed = std::mem::take(&mut impacts.0);

    for (entity, world, mut model, mut procs) in &mut entities {
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

        // --- kits the server sent directly, with no spell ---
        //
        // `SMSG_PLAY_SPELL_VISUAL` and `SMSG_PLAY_SPELL_IMPACT` carry a
        // `SpellVisualKit` id and a guid. Every other visual in this file is
        // reached from a spell, so this is the only caller of `kit_effects`;
        // see `vale_protocol::play::sound` for the two bodies.
        //
        // The first look records the counters and acts on nothing, as in the
        // cast block below: a unit that streams into view beside a character
        // who is eating must not show that character's eating visual.
        let seen_pushed = *pushed_for;
        *pushed_for = Some((world.spell_visuals, world.spell_impacts));
        if let Some((visuals, impacts)) = seen_pushed {
            if visuals != world.spell_visuals || impacts != world.spell_impacts {
                // The visual wins when both moved in one poll, as in the pose
                // code: what the unit did takes priority over what was done
                // to it.
                let kit = match visuals != world.spell_visuals {
                    true => world.last_spell_visual,
                    false => world.last_spell_impact,
                };
                if let Some(kit) = spells.and_then(|s| s.kit_procedurals(kit)) {
                    procs.play(kit, now_precise);
                }
                if let Some(models) = displays
                    .tables()
                    .and_then(|tables| tables.kit_effects(kit).cloned())
                {
                    // Restarted rather than layered. This matters most here:
                    // vmangos re-sends kit 406 on every regeneration tick while
                    // a character eats, so an additive set would gain a plate of
                    // food per tick.
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
            // A channel is a release and then a begin within one poll
            // (vmangos' `SendSpellGo` then `SendChannelStart`), so the begin is
            // the later event and takes effect. Without this, the release below
            // wins by being written second, and Evocation draws release art it
            // does not have instead of its channel art.
            let channelled = world.casts_channelled != seen.channelled;
            // A refused, interrupted or cancelled cast removes its own art and
            // adds nothing, unlike the two cases below. It is tested first
            // because it ends art rather than starting it; see
            // `WorldEntity::casts_cancelled`. For an instant cast the release
            // art is already up, because this client draws both halves at the
            // key press.
            // The procedurals of each kit that played: the precast kit when
            // the cast began, the cast kit of every release, and the channel
            // kit when a channel began. A later one replaces an earlier one
            // on the unit, so they are played in the order the kits play.
            if world.casts_cancelled == seen.cancelled {
                if let Some(spells) = spells {
                    if begun && !channelled {
                        if let Some(p) = spells.procedurals(world.last_spell) {
                            procs.play(&p.precast, now_precise);
                        }
                    }
                    if released {
                        for spell in released_spells(world, seen.released) {
                            if let Some(p) = spells.procedurals(spell) {
                                procs.play(&p.cast, now_precise);
                            }
                        }
                    }
                    if channelled {
                        if let Some(p) = spells.procedurals(world.last_spell) {
                            procs.play(&p.channel, now_precise);
                        }
                    }
                }
            }
            if world.casts_cancelled != seen.cancelled {
                cast.clear(&mut commands);
            } else if begun || released {
                cast.clear(&mut commands);
                // The release wins when both counters moved in one step, which
                // is how an instant spell arrives: `SMSG_SPELL_GO` with no
                // `SMSG_SPELL_START` before it, or both inside one 25 ms tick.
                // The exception is a channel starting, the one case where the
                // begin is the later packet.
                if released && !channelled {
                    // Every release in the batch, oldest first: a poll can hold
                    // more than one, and the last is not always the one with
                    // art. Charge releases 100 and then the Charge Stun (7922)
                    // it triggers. Reading only `last_spell` gave the stun,
                    // which states no caster models, so the red trail and the
                    // dust cloud were never requested.
                    //
                    // A release with nothing to draw does not remove what is
                    // up. The 1.12.1 client does not remove it either: a cast
                    // hangs its own kit's models, and a kit with none hangs
                    // nothing. Without this, the stun would remove the trail
                    // right after it was armed.
                    for spell in released_spells(world, seen.released) {
                        let Some(effects) = displays
                            .tables()
                            .and_then(|tables| tables.cast_effects(spell).cloned())
                            .filter(|e| !e.release.is_empty())
                        else {
                            continue;
                        };
                        // The release stays until its clip ends; see
                        // [`RELEASE_SECS`], which is the minimum.
                        cast.want_until_played(effects.release, now + RELEASE_SECS, points);
                    }
                } else if let Some(effects) = displays
                    .tables()
                    .and_then(|tables| tables.cast_effects(world.last_spell).cloned())
                {
                    // A channel hangs the channel kit's models, which differ
                    // from the wind-up's for the 45 visuals that state both;
                    // see `vale_assets::tables::spell::CastEffects::channel`.
                    // A channel with no models of its own keeps the wind-up's,
                    // which for a spell stating only `channelKit` are already
                    // in `hold`.
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
                // A pushback is not a new cast, so this is an `else if`: it
                // arms and clears nothing and only moves the deadline of what
                // is already up. `SMSG_SPELL_DELAYED` is the only packet that
                // changes a cast's length after `SMSG_SPELL_START`, and the art
                // on the caster's hands was armed from that length. Without
                // this, a Fireball pushed back twice loses its glow a second
                // before it is thrown.
                cast.until += world.last_cast_delay_ms as f32 / 1000.0;
            }
        }

        // --- what the cast hit, from a separate counter ---
        //
        // The impact depends on what the server says the cast hit, which is
        // separate from what the caster did, so this is not merged into the
        // block above even though both concern a release. The block above
        // draws the caster; this one queues the impact on each victim. The
        // split was made when `casts_released` moved at the key press, before
        // the hit list existed, so implicitly aimed spells found no victim a
        // frame before `SMSG_SPELL_GO` named them (Cone of Cold lost its whole
        // visible half, Arcane Explosion its flash). Both counters now move on
        // the same packet.
        //
        // `casts_landed` moves only on the wire's release, and nothing else
        // writes it. For every unit except the local player it moves together
        // with `casts_released`, so this costs one `u32` comparison a frame.
        let seen_landed = *landed_for;
        *landed_for = Some(world.casts_landed);
        if seen_landed.is_some_and(|seen| seen != world.casts_landed) {
            // A spell with no projectile lands when it is released, so its
            // impact is queued here rather than by the missile pass. That is
            // most spells (8,751 state an impact kit and 898 throw a
            // projectile), including every self-cast buff, where the caster is
            // also the victim: Ice Armor's `IceArmor_Low_Head` is an impact on
            // the mage's own head, which the 1.12.1 client draws.
            let throws = displays
                .tables()
                .is_some_and(|tables| tables.cast_missile(world.last_spell).is_some());
            if !throws {
                // A cast with an empty hit list landed on its caster, but only
                // if the spell targets its caster. A self-buff's hit list
                // usually carries the caster's own guid and sometimes nothing,
                // and both must give the same result. Without the fallback,
                // the buffs whose impact is their visible part (Frost Armor's
                // head, Dampen Magic's ring on the ground) find no victim and
                // draw nothing.
                //
                // An area spell that hit nobody also has an empty list, and
                // must not fall back: with an unconditional fallback, every
                // Arcane Explosion cast into empty air showed
                // `ArcaneExplosion_Impact_Chest` on the mage's own chest.
                // `Spell.dbc`'s implicit targets tell the two cases apart; see
                // `SpellVisuals::is_self_cast`.
                //
                // The whole list is used. `SMSG_SPELL_GO` names every guid the
                // cast hit, and each gets the impact kit: an Arcane Explosion
                // hitting five creatures flashes on all five. Reading only the
                // first entry gave the other four nothing while every counter
                // looked correct. See `Entity::last_spell_targets`, the field
                // read here.
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
            if let Some(p) = spells.and_then(|s| s.procedurals(*spell)) {
                procs.play(&p.impact, now_precise);
            }
            impact.clear(&mut commands);
            let effects = displays
                .tables()
                .and_then(|tables| tables.cast_effects(*spell).cloned());
            if let Some(effects) = effects {
                // The impact also stays until its clip ends. This is where the
                // fade that was being cut off is: a self-cast buff's visible
                // part is its impact kit on its own caster.
                impact.want_until_played(effects.impact, now + RELEASE_SECS, points);
            }
        }

        // --- what is true of it: the auras it is carrying ---
        //
        // Rebuilt whole when the set changes, not diffed. A unit carries a few
        // auras and the set changes seconds apart, so a diff saves nothing, and
        // it would risk a buff refreshed in place hanging a second glow over
        // the one already on the bone.
        //
        // The comparison uses the auras that state a visual, not the slots.
        // Most auras are invisible, and a unit in combat gains and loses them
        // constantly, so comparing the raw slot list would clear and rebuild
        // every glow on the unit, restarting its clock, each time a debuff was
        // applied. Filtering first costs one hash lookup per slot and makes
        // "the set changed" mean "what is drawn changed".
        let visible: Vec<u32> = displays.tables().map_or_else(Vec::new, |tables| {
            world
                .auras
                .iter()
                .map(|aura| aura.spell)
                .filter(|spell| tables.aura_effects(*spell).is_some())
                .collect()
        });
        // A state kit's timed colour and weapon trail play when its aura
        // appears on the unit, including on the first look: a unit that
        // arrives carrying the aura has its kit played then.
        if let Some(spells) = spells {
            let playing: Vec<u32> = world
                .auras
                .iter()
                .map(|aura| aura.spell)
                .filter(|spell| {
                    spells
                        .procedurals(*spell)
                        .is_some_and(|p| p.state.flash.is_some() || p.state.trail.is_some())
                })
                .collect();
            if procs.state_auras != playing {
                for spell in &playing {
                    if !procs.state_auras.contains(spell) {
                        if let Some(p) = spells.procedurals(*spell) {
                            procs.play(&p.state, now_precise);
                        }
                    }
                }
                procs.state_auras = playing;
            }
        }
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

/// The level-up glow, one of the visuals that no spell table reaches.
///
/// `SpellVisualEffectName.dbc` row 21 is named `HARDCODED Unit Level Up` and
/// points at `Spells\LevelUp\LevelUp.m2`. As the name says, no spell,
/// `SpellVisual` or kit reaches it. The client plays it on
/// `SMSG_LEVELUP_INFO` only, so this is a separate system rather than a row in
/// the chain [`spell_effects`] follows.
///
/// The numbers are measured from the model: `vale model
/// 'Spells\LevelUp\LevelUp.m2'` reads 4 batches over three textures, all
/// additive and unlit, five particle emitters, and one sequence, `0..1867 ms`,
/// one-shot, with every batch's transparency track fading out over exactly
/// that span. So the clip ends it, and [`LEVELUP_SECS`] is that length plus a
/// margin for the emitters' last particles.
///
/// Hung from [`attach::BASE`], the caster's feet, where every ground-painted
/// effect in the game is anchored. The model's own box runs from −1.4 to
/// +16.3 in z about that point: a ring on the floor and a column above it.
pub(super) fn level_up(
    mut commands: Commands,
    time: Res<Time>,
    mut levelled: MessageReader<crate::interface::events::PlayerLevelUp>,
    mut player: Query<&mut EntityModel, With<crate::world::session::LocalPlayer>>,
) {
    // Read the messages even with no player to show them on, or the glow
    // appears on the next character to log in.
    let levels = levelled.read().count();
    let Ok(mut model) = player.single_mut() else {
        return;
    };
    let now = time.elapsed_secs();
    if levels > 0 {
        let EntityModel { milestone, points, .. } = &mut *model;
        // Restarted rather than layered on a double level-up, as every other
        // set here does for a second event within one clip.
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
    // `spell_effects` spawns and expires every set, so this system only
    // requests.
}

/// `SpellVisualEffectName.dbc` row 21's model, with the `.mdl` the table names
/// turned into the `.m2` the archive holds.
const LEVELUP_MODEL: &str = "Spells\\LevelUp\\LevelUp.m2";

/// How long the level-up glow stays on: the clip's own 1,867 ms plus a margin
/// for its five emitters' last particles. Unlike [`RELEASE_SECS`], this is not
/// an estimate over many models: there is exactly one model, and it was
/// measured.
const LEVELUP_SECS: f32 = 2.4;

/// The sparkle over a body that still has loot, the second of the two visuals
/// no spell table reaches; see [`level_up`] for the first.
///
/// `SpellVisualEffectName.dbc` row 14 is named `HARDCODED Loot Art` and points
/// at `Particles\LootFX.mdl`. The 1.12.1 client looks up both this row and
/// `HARDCODED Unit Level Up` by name, so the lookup follows the client rather
/// than a convention invented here. No spell, `SpellVisual` or kit reaches
/// either; the client plays them on its own conditions.
///
/// The shape of the effect is measured from the file: `vale model
/// 'Particles\LootFX.m2'` reads 0 render batches, two textures
/// (`Item\ObjectComponents\Weapon\Flare.blp` and `Spells\Star5A.blp`, the
/// golden flare and the star twinkles), four particle emitters, and one
/// sequence, `3333..5733 ms`, flagged to loop. So it is entirely emitters and
/// it holds rather than playing out, which is why this is a state with no
/// clock, where the level-up is a one-shot with one.
///
/// ## The condition: dead and `UNIT_DYNFLAG_LOOTABLE`
///
/// A dead unit carrying `UNIT_DYNFLAG_LOOTABLE` shows the sparkle; no other
/// unit does. There is no tap check, no distance test, no loot-window test and
/// no fade. The flag is per viewer: vmangos clears it for a player with no
/// loot rights ("hide lootable animation for unallowed players"), so it
/// already says whether a right-click would do anything for this player, and
/// further tests would only duplicate the server's decision. When the flag
/// clears, the sparkle is removed at once, which is what looting the last
/// item looks like.
///
/// Hung from [`attach::BASE`], attachment `0x13`, the same point the level-up
/// glow uses. When a model lacks that point, the 1.12.1 client draws the
/// effect at the unit's origin.
///
/// Acts on the edge rather than every frame. `want` appends, so requesting
/// every frame would add a second, third and hundredth copy of the emitters to
/// one corpse. The set's own emptiness records whether it is on, which also
/// lets a rebuilt model (`CarriedEffects` carries this set) keep its copy
/// instead of adding another.
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
        // Without `SpellVisualEffectName` in the archive chain there is no loot
        // art, the documented fallback for every optional table here.
        return;
    };
    let (path, scale) = (path.to_string(), scale);
    for (world, mut model) in &mut units {
        let wanted = world.dead && world.lootable;
        let EntityModel { loot, points, .. } = &mut *model;
        match (wanted, loot.is_empty()) {
            // The rising edge only; see the doc comment for why this is not
            // requested every frame.
            (true, true) => loot.want(
                vec![vale_assets::tables::spell::KitEffect {
                    point: attach::BASE,
                    path: path.clone(),
                    scale,
                }],
                // No clock: the sequence loops and the condition ends it.
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
/// a pauldron and a missile use, so it poses, fades and emits particles with no
/// duplicated code. A Blizzard is almost entirely emitters, so this matters.
#[derive(Component)]
pub struct AreaArt(pub(super) AttachedPart);

/// A `DynamicObject` whose spell names no ground model, or whose model will not
/// read. Marked so the lookup is not retried every frame, as with
/// [`super::NoModel`].
#[derive(Component)]
pub struct NoAreaArt;

/// Draw the persistent areas: a Blizzard, a Flamestrike, a Consecration, a
/// Rain of Fire.
///
/// A persistent area is the one object the server puts in the world with no
/// display id. A `DynamicObject` states a caster, a spell id and a radius and
/// nothing else. Its appearance comes from three columns of the spell's own
/// `SpellVisual` row; see [`vale_assets::tables::spell::SpellVisuals::area`].
/// That rule is measured; it replaced a fallback chain over the caster's five
/// kits.
///
/// The radius is not a scale. `DYNAMICOBJECT_RADIUS` is the area the spell
/// covers; the art is authored at its own size, and `SpellVisualEffectName`
/// has its own scale column. The 1.12.1 client scales the art by the radius
/// only when `DYNAMICOBJECT_BYTES` is neither 1 nor 2, and vmangos writes 1
/// there for every area aura it creates (`DynamicObject::Create`), so no area
/// spell in this game is scaled by its radius. The radius does drive
/// [`rain_impacts`], where it sets the spread of the impacts, not a size.
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
            // The tables have not been read yet. `spawn_models` reads them for
            // its first entity; this system does not force the read.
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
                // No wearer bone: the area is the placement, and the object's
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
                // The falling impacts, for the ten visuals that have them. The
                // seed is the entity's own index so two Blizzards side by side
                // do not fall in step; the particle emitters seed the same way
                // for the same reason.
                if let Some(rain) = rain {
                    commands.entity(entity).insert(AreaRain {
                        path: rain.path,
                        rate: rain.rate,
                        radius,
                        // Starts at 1.0, not 0.0. The first impact shows that
                        // the spell went off; without it, a Blizzard's first
                        // fifth of a second is empty.
                        accumulated: 1.0,
                        rng: (entity.to_bits() as u32).wrapping_mul(0x9E37_79B9) | 1,
                    });
                }
            }
        }
    }
}

/// The falling-impact procedural (`charProc == 9`), held on the
/// `DynamicObject` it belongs to; see [`vale_assets::tables::spell::AreaRain`].
#[derive(Component)]
pub struct AreaRain {
    path: String,
    /// Impacts per second, the kit's own `charParamOne`.
    rate: f32,
    /// `DYNAMICOBJECT_RADIUS`, in yards — how far out an impact may land.
    radius: f32,
    /// The fractional remainder carried between frames. It must be fractional:
    /// a 0.7/s Lightning Cloud strikes about once every one and a half seconds,
    /// and a whole number per frame would give it either none or sixty.
    accumulated: f32,
    rng: u32,
}

/// One falling impact, alive on its own clock.
///
/// An entity of its own rather than a child of the area, because its position
/// is not the area's: it lands somewhere inside the radius, on the ground
/// there. It carries [`AreaArt`], so [`animate_areas`] poses it and runs its
/// emitters with no duplicated code.
#[derive(Component)]
pub struct AreaImpact {
    /// When it is taken off, in `Time::elapsed_secs`.
    until: f32,
}

/// How long an impact lives when its model states no sequence length.
///
/// The 1.12.1 client removes a shard when its model's animation finishes, so
/// the clip is the lifetime, and this is only the fallback for a model that
/// states none. Same reasoning and similar value as [`RELEASE_SECS`].
const IMPACT_FALLBACK_SECS: f32 = 1.5;

/// The longest an impact is held, whatever its clip says.
///
/// `Flamestrike_Impact_Base`'s Stand runs twelve seconds. A model with a clip
/// that long is authored to be the area, not to fall into it. No kit rains
/// one, but the limit keeps a rate and a long clip from producing an
/// unbounded number of impacts if one ever does.
const IMPACT_MAX_SECS: f32 = 6.0;

/// The random delay on an impact's own clock, in seconds: `0..0.51`.
///
/// The 1.12.1 client starts each shard's clock 0 to 510 ms late, in 2 ms
/// steps. It matters most at low rates, where the accumulator alone would
/// start every strike on a frame boundary and the area would strike at a
/// visibly regular rhythm.
const IMPACT_STAGGER_SECS: f32 = 0.51;

/// Spawn the falling impacts: a Blizzard's shards, a Rain of Fire's meteors, a
/// Hurricane's gusts.
///
/// One instance of the kit's model per whole unit of `dt * rate`, each at
/// `centre + radius * r * direction` for a uniform `r`. That concentrates them
/// towards the middle rather than spreading them evenly over the disc, and it
/// matches the 1.12.1 client; it is not a simplification. There is
/// deliberately no `sqrt`, which would make the spread area-uniform.
///
/// The floor is the same one the player walks on, found through
/// `Standing::floor`, which the ground-target pointer also uses, so a
/// Blizzard cast on a bridge lands on the bridge. A column with no floor (a
/// hole, or a tile not yet streamed in) drops that impact rather than placing
/// it at the area's own height, as `ground_under_ray` does.
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
        // Capped per frame, for each area. A tab-out that returns a two-second
        // delta would otherwise spawn thirty shards in one frame, all of which
        // then expire together. The 1.12.1 client has the same exposure. A
        // client that stutters should drop impacts rather than pile them up.
        let mut budget = MAX_IMPACTS_PER_FRAME;
        while rain.accumulated >= 1.0 && budget > 0 {
            rain.accumulated -= 1.0;
            budget -= 1;
            // Coordinates are WoW's here: the offset is picked in the game's own
            // xy and converted once, and the ground query uses the same axes.
            let radius = rain.radius;
            let (dx, dy) = impact_offset(&mut rain.rng, radius);
            let centre = crate::axes::to_wow(placement.translation);
            let (x, y) = (centre[0] + dx, centre[1] + dy);
            use vale_protocol::socket::session::World;
            // Searched from above the area's own height, as the 1.12.1 client
            // searches from 16.7 yards up: the floor under the edge of a
            // Blizzard on a slope can be well above the centre.
            let Some(z) = standing.floor(map_id, x, y, centre[2] + IMPACT_TRACE_UP) else {
                continue;
            };
            let path = rain.path.clone();
            let Lookup::Ready(assets) =
                cache.attached(&path, None, None, SceneLighting::NONE, &mut meshes, &mut materials)
            else {
                // Loading, or unreadable: this impact is not spawned. Marking
                // the area `NoAreaArt` here would also remove its own model, and
                // the two are different files for eight of the ten visuals.
                continue;
            };
            let stagger = crate::render::particles::rand01(&mut rain.rng) * IMPACT_STAGGER_SECS;
            let at = crate::axes::to_bevy([x, y, z]);
            let impact = commands
                .spawn((
                    Transform::from_translation(at),
                    // Written here as well as by Bevy's propagation: this is a
                    // root with no parent, and `render::decals` reads the
                    // carrier's `GlobalTransform` in the same schedule as this
                    // spawn.
                    GlobalTransform::from_translation(at),
                    Visibility::default(),
                ))
                .id();
            // The delay is a late start on the model's own clock rather than a
            // deferred spawn: `animate_attachment` clamps a negative elapsed
            // time to zero, so the shard stays at frame 0 until its start.
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
            // The clip `hang_model` chose, not sequence 0. The two differ for a
            // model whose Stand is not its first sequence, and a lifetime taken
            // from the wrong one either cuts the shard off mid-fall or leaves it
            // standing. The effect sets use the same method:
            // [`super::AttachedPart::clip_secs`].
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
        // A frame that ran out of budget must not carry the backlog forward
        // indefinitely; see `MAX_IMPACTS_PER_FRAME`.
        if budget == 0 {
            rain.accumulated = rain.accumulated.min(1.0);
        }
    }
}

/// Where one impact lands, relative to the area's centre, in WoW xy yards.
///
/// `r` is uniform in `0..radius` and there is no `sqrt`. The 1.12.1 client
/// places an impact at one uniform random fraction of the radius in a uniform
/// random direction, so the impacts concentrate towards the middle of the
/// area: mean radius `R/2`, against the `2R/3` of an area-uniform disc. The
/// difference is visible at a Blizzard's eight yards. An area-uniform version
/// would spread the snow out towards a ring.
pub(super) fn impact_offset(rng: &mut u32, radius: f32) -> (f32, f32) {
    let angle = crate::render::particles::rand01(rng) * std::f32::consts::TAU;
    let reach = crate::render::particles::rand01(rng) * radius;
    (reach * angle.cos(), reach * angle.sin())
}

/// How many impacts one area may spawn in one frame. See [`rain_impacts`].
const MAX_IMPACTS_PER_FRAME: u32 = 4;

/// How far above the area's own height the floor search starts, in yards. The
/// 1.12.1 client uses 16.67.
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
            // `despawn` removes the attachment root and its parts. The ground
            // decals it owns are removed by `render::decals::retire_decals`, as
            // for every other attachment.
            commands.entity(entity).despawn();
        }
    }
}

/// Drop every impact when the world goes.
///
/// An impact is a root entity, so the entity teardown, which removes only the
/// entities the server described, does not reach it. Emitters and decals have
/// the same issue and are handled the same way.
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

/// Pose, fade and run the emitters of every persistent area on its own clock.
///
/// Separate from [`super::animate`] because a `DynamicObject` has no skeleton
/// of its own and so no [`super::Playback`], which that system's query
/// requires. The frame passed down is the object's placement times the art's
/// own scale, which is the root's local transform. An unskinned part gets that
/// scale from Bevy's propagation, but a skinned part's joints replace its world
/// matrix, so the scale must be in the matrix the joints are composed against,
/// or a scaled effect draws at scale 1.
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

/// Spells that have landed and are waiting to show their impact on the victim.
///
/// A queue rather than a call because the two ends are separate passes: a
/// projectile arrives in `missiles::fly_missiles`, and the impact hangs on a
/// different entity, which that pass does not access. The queue also makes
/// the order of the two passes irrelevant: [`spell_effects`] drains what it
/// finds, and anything queued after it has run waits one frame, 16 ms on an
/// effect that lasts 1,500 ms.
///
/// Keyed by guid rather than `Entity`. The victim's model may be rebuilt
/// between launch and arrival (new gear, a change of room), and the impact
/// belongs to the unit, not to the model it had at launch.
#[derive(Resource, Default)]
pub struct PendingImpacts(Vec<(u64, u32)>);

impl PendingImpacts {
    /// A spell arrived: show its impact on `victim`.
    pub(crate) fn land(&mut self, victim: u64, spell: u32) {
        self.0.push((victim, spell));
    }

    /// What is queued. Used by the missile test, which checks what a
    /// projectile hands over, not what is done with it.
    #[cfg(test)]
    pub(crate) fn landed(&self) -> &[(u64, u32)] {
        &self.0
    }
}

