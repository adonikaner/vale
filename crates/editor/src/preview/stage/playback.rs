//! Playing the timeline: each step a change to the counters the client's
//! passes watch, and a loop of pushed kits.

use crate::session::EditSession;
use vale_client::world::entity::{AuraSlot, WorldEntity};
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use super::{CASTER_GUID, HOLD_SECS, IMPACT_SECS, PUSHED, STAGE_LAYER, Stage, StageArea, StageUnit, Step, TARGET_GUID, Timeline};
use super::scene::stage_area;

/// Run the timeline: begin, release, land, wear, and round again.
///
/// Every step is a change to a counter — except the state, which is an
/// aura, because that is what the client watches for it. The steps are taken
/// whether or not the clock is running, so a seek while paused still lands
/// where it was asked to.
pub(crate) fn play(
    mut commands: Commands,
    time: Res<Time>,
    mut stage: ResMut<Stage>,
    session: Option<Res<EditSession>>,
    mut units: Query<(&StageUnit, &mut WorldEntity, &Transform)>,
    areas: Query<Entity, With<StageArea>>,
    modelled: Query<(), (With<StageUnit>, With<vale_client::world::entities::EntityModel>)>,
) {
    let Some(session) = session else {
        return;
    };
    let ready = stage.actors_ready(
        time.delta_secs(),
        units.iter().count(),
        modelled.iter().count(),
    );
    let Some(spell) = stage.showing else {
        if stage.pushing.is_some() {
            play_pushes(&time, &mut stage, &mut units, ready);
        }
        return;
    };
    if stage.staged != Some(spell) {
        return;
    }

    let timing = Timeline::of(&session, spell);
    stage.cast_secs = timing.cast_secs;
    stage.channel_secs = timing.channel_secs;
    stage.flight_secs = timing.flight();
    stage.missile_speed = timing.missile_speed;
    stage.self_cast = timing.self_cast;
    stage.area = timing.area;
    stage.whole_secs = timing.whole();
    // The clock and the steps wait for the actors' models.
    if !ready {
        return;
    }

    // A pause holds the phase rather than freezing the frame. The clips
    // the effect pass hangs run on the world's clock, not the stage's, so a
    // paused stage plays its one-shots out and then shows two actors standing:
    // a capture of Fireball taken with `--seek 1.9` and no re-seek has the
    // playhead in the impact and nothing on the target. Re-seeking to the same
    // point every [`HOLD_SECS`] re-hangs them, so what is on screen while
    // paused is the first second and a half of that phase, repeated.
    if stage.playing {
        stage.held = 0.0;
    } else {
        stage.held += time.delta_secs();
        if stage.held >= HOLD_SECS && stage.at < stage.whole_secs {
            let at = stage.at;
            stage.seek(at);
        }
    }
    // A pending seek pauses where it lands. It is `--seek`'s. The stage's
    // own first `restart`, which runs in `arrange` after the flag is read,
    // sets the clock playing, so without the pause a capture asked for at
    // 1.9 s is taken wherever the loop has got to since.
    if let Some(secs) = stage.pending_seek.take() {
        stage.seek(secs);
        stage.playing = false;
    }
    if stage.reset {
        stage.reset = false;
        // The aura comes off between loops, or the state art of the last
        // pass is still up during the next one's wind-up; and the area goes
        // with it.
        for (_, mut unit, _) in units.iter_mut() {
            unit.auras.clear();
        }
        for area in &areas {
            commands.entity(area).despawn();
        }
    }

    if stage.playing {
        stage.at += time.delta_secs();
    }

    let landed = timing.landed();
    let wanted = if stage.at >= landed + IMPACT_SECS {
        Step::Worn
    } else if stage.at >= landed {
        Step::Landed
    } else if stage.at >= timing.cast_secs {
        Step::Released
    } else {
        Step::Begun
    };

    while stage.done < wanted {
        let next = match stage.done {
            Step::Nothing => Step::Begun,
            Step::Begun => Step::Released,
            Step::Released => Step::Landed,
            Step::Landed => Step::Worn,
            Step::Worn => break,
        };
        step(&mut commands, &mut units, spell, next, &timing);
        stage.done = next;
        // A channel's area comes down when the channel stops. `step` puts
        // it down at the release, and vmangos destroys the `DynamicObject`
        // when the channel ends, so an area still standing through the impact
        // and the worn state would show three seconds of a Blizzard nobody is
        // casting. An instant area spell's area is left up for the rest of the
        // loop, because for that spell the impact is when the area arrives.
        if next == Step::Landed && timing.channel_secs > 0.0 {
            for area in &areas {
                commands.entity(area).despawn();
            }
        }
    }

    if stage.at >= stage.whole_secs {
        match stage.looping {
            true => stage.restart(),
            false => {
                stage.playing = false;
                stage.at = stage.whole_secs;
            }
        }
    }
}

/// Run a loop of pushed kits: each kit is pushed onto its actor when the
/// clock reaches it, and the loop begins again at its end.
///
/// The spell timeline's rules hold here too. A seek is a restart and a
/// fast-forward, so every push up to the asked-for time is made in one
/// frame, and a paused stage pushes its phase again every [`HOLD_SECS`] so
/// that what is on screen is the phase and not two bodies standing after a
/// one-shot clip has ended.
fn play_pushes(
    time: &Time,
    stage: &mut Stage,
    units: &mut Query<(&StageUnit, &mut WorldEntity, &Transform)>,
    ready: bool,
) {
    if stage.staged != Some(PUSHED) {
        return;
    }
    let Some(pushes) = stage.pushing.clone() else {
        return;
    };
    // A different loop on the same actors starts from nothing.
    if stage.pushed_of != Some(pushes.of) {
        stage.pushed_of = Some(pushes.of);
        stage.restart();
    }
    stage.cast_secs = 0.0;
    stage.channel_secs = 0.0;
    stage.flight_secs = 0.0;
    stage.missile_speed = 0.0;
    stage.area = None;
    stage.self_cast = pushes.alone;
    stage.whole_secs = pushes.whole_secs;
    // The clock and the pushes wait for the actors' models.
    if !ready {
        return;
    }

    if stage.playing {
        stage.held = 0.0;
    } else {
        stage.held += time.delta_secs();
        if stage.held >= HOLD_SECS && stage.at < stage.whole_secs {
            let at = stage.at;
            stage.seek(at);
        }
    }
    if let Some(secs) = stage.pending_seek.take() {
        stage.seek(secs);
        stage.playing = false;
    }
    if stage.reset {
        stage.reset = false;
        stage.pushed = 0;
        // A spell played before this loop may have left its aura on.
        for (_, mut unit, _) in units.iter_mut() {
            if !unit.auras.is_empty() {
                unit.auras.clear();
            }
        }
    }
    if stage.playing {
        stage.at += time.delta_secs();
    }
    while let Some(push) = pushes.pushes.get(stage.pushed).filter(|push| push.at <= stage.at) {
        // With one actor everything lands on it, as a self-cast does.
        let on_target = push.on_target && !pushes.alone;
        for (which, mut unit, _) in units.iter_mut() {
            if which.caster == on_target {
                continue;
            }
            match push.on_target {
                true => {
                    unit.last_spell_impact = push.kit;
                    unit.spell_impacts = unit.spell_impacts.wrapping_add(1);
                }
                false => {
                    unit.last_spell_visual = push.kit;
                    unit.spell_visuals = unit.spell_visuals.wrapping_add(1);
                }
            }
        }
        stage.pushed += 1;
    }
    if stage.at >= stage.whole_secs {
        match stage.looping {
            true => stage.restart(),
            false => {
                stage.playing = false;
                stage.at = stage.whole_secs;
            }
        }
    }
}

/// Move the numbers one step of a cast moves. See the module comment for what
/// each step writes and why it is written there.
pub(super) fn step(
    commands: &mut Commands,
    units: &mut Query<(&StageUnit, &mut WorldEntity, &Transform)>,
    spell: u32,
    step: Step,
    timing: &Timeline,
) {
    let victim_guid = match timing.self_cast {
        true => CASTER_GUID,
        false => TARGET_GUID,
    };
    let mut victim_at: Option<Vec3> = None;
    for (which, mut unit, placement) in units.iter_mut() {
        // A self-cast lands on the caster, so the caster is also the victim.
        let victim = match timing.self_cast {
            true => which.caster,
            false => !which.caster,
        };
        if victim {
            victim_at = Some(placement.translation);
        }
        match (step, which.caster, victim) {
            (Step::Begun, true, _) => {
                unit.last_spell = spell;
                unit.cast_time_ms = (timing.cast_secs * 1000.0) as u32;
                unit.casts_begun = unit.casts_begun.wrapping_add(1);
            }
            // The ring and the counter together: the pass reads the spells
            // released since it last looked, which is a window into
            // `recent_spells` rather than a single field. The victim is
            // written to both of its fields: `last_spell_target` is what a
            // missile flies at, and `last_spell_targets` is who the impact
            // lands on. A self-cast has nowhere to fly, so its
            // `last_spell_target` stays 0 and its list names the caster,
            // which is what the server's own hit list carries for a buff.
            (Step::Released, true, _) => {
                let last = unit.recent_spells.len() - 1;
                unit.recent_spells[last] = spell;
                unit.casts_released = unit.casts_released.wrapping_add(1);
                unit.last_spell_target = match timing.self_cast {
                    true => 0,
                    false => victim_guid,
                };
                unit.last_spell_targets = vec![victim_guid];
                // A channel begins in the same step it is released, which
                // is the order vmangos sends the two packets in and the order
                // the pose and effect passes are written against. The cast
                // time restated is the channel's length, which is what holds
                // the channel art up.
                //
                // It is a `begun` as well as a `channelled`.
                // `ObjectManager::apply_channel_start`, which applies the one
                // packet that states a channel, bumps three fields,
                // `casts_begun` among them, and the stage has to bump the same
                // three to agree with a session. Both passes that draw a
                // channel are written against the begin: `pose` arms its held
                // pose there and `effects` hangs its models there. A channel
                // with no begin leaves the wind-up's `Casting::until` to
                // expire a moment later, and the caster stands empty-handed
                // and plays no animation for the whole of the channel.
                // Blizzard is a spell that shows it.
                if timing.channel_secs > 0.0 {
                    unit.cast_time_ms = (timing.channel_secs * 1000.0) as u32;
                    unit.casts_channelled = unit.casts_channelled.wrapping_add(1);
                    unit.casts_begun = unit.casts_begun.wrapping_add(1);
                }
            }
            // The impact is the caster's counter, because it is the
            // caster's `SMSG_SPELL_GO` — see the module comment. The effect
            // pass hangs the impact kit on every guid in the caster's hit
            // list, and skips all of them for a spell that throws a missile,
            // whose arrival lands the impact instead.
            (Step::Landed, true, _) => {
                unit.last_spell = spell;
                unit.casts_landed = unit.casts_landed.wrapping_add(1);
            }
            _ => {}
        }
        // The flinch, on somebody else: a self-buff does not stagger its
        // caster.
        if step == Step::Landed && victim && !timing.self_cast {
            unit.blows_taken = unit.blows_taken.wrapping_add(1);
        }
        // An aura and not a counter. A state kit is worn for as long as
        // the aura is on the unit, so the client watches the aura list rather
        // than anything that counts — see `world::entities::effects`, where
        // the state set is rebuilt when the visible auras change.
        if step == Step::Worn && victim {
            unit.auras = vec![AuraSlot {
                slot: 0,
                spell,
                flags: 0,
                level: 1,
                applications: 1,
            }];
        }
    }
    // The area, at the victim's feet. A `DynamicObject` the server would
    // have put there; the client's own pass hangs the visual's area model on it
    // and runs its rain.
    //
    // A channel's area goes down when the channel starts, not when it ends.
    // vmangos creates the `DynamicObject` in `Spell::EffectPersistentAA`, which
    // for a channelled spell runs as the channel begins, and destroys it when
    // the channel stops. The area lasts as long as the channel, and for
    // Blizzard the falling ice is the area's rain rather than anything the
    // impact hangs. Putting the area down at `Landed` for every spell shows
    // nothing for the four seconds of Blizzard's channel and the whole spell
    // in the one second after it. An instant area spell such as Flamestrike
    // still puts its area down at the impact, which is the same rule: the area
    // appears when the server makes it.
    let when = match timing.channel_secs > 0.0 {
        true => Step::Released,
        false => Step::Landed,
    };
    if let (true, Some(radius), Some(at)) = (step == when, timing.area, victim_at) {
        commands.spawn((
            stage_area(spell, radius),
            Transform::from_translation(at),
            Visibility::default(),
            RenderLayers::from_layers(&[0, STAGE_LAYER]),
            StageArea,
        ));
    }
}
