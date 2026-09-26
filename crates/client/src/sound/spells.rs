//! **A cast's sounds** — the wind-up, the release, the missile and the
//! impact, off the same counters the pose and the effects read.
//!
//! The ids come from [`vale_assets::tables::sound::SoundBank::cast_sounds`], which
//! re-walks the spell chain for the columns `assets::spell` reads past: each
//! kit's own sound and the missile's. 13,909 spells carry at least one.
//!
//! The edges are per-entity, primed on first sight, exactly as
//! [`super::combat`] — and the impact is hung off **`casts_landed`**, the
//! server's own release counter, for the same reason the impact *art* is: what
//! a cast hit is not the presser's to predict. The channel kit's sound is not
//! played yet; a channel's own edge is not distinguished here, and the gap is
//! stated in `sound/mod.rs`'s terms rather than guessed at.

use super::mixer::{Place, Voices};
use crate::world::session::{EntityIndex, WorldEntity};
use bevy::prelude::*;

#[derive(Component)]
pub struct HeardCasts {
    begun: u32,
    released: u32,
    landed: u32,
    cancelled: u32,
    /// **The wind-up's own voice, held so that it can be taken off.**
    ///
    /// A precast is the only one of the five that is tied to a *state* rather
    /// than to an instant: it belongs to the bar that is running, so it ends
    /// when the bar does. Several of them are seconds long — the charge-up
    /// hums and rising whines — and a cast that releases early, is interrupted
    /// or is cancelled left the file playing on to its own end, which is the
    /// "spell sound FX linger after the cast is complete" report. The release,
    /// the missile and the impact are genuinely instants and are not held.
    precast: Option<Entity>,
}

pub struct SpellsPlugin;

impl Plugin for SpellsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, listen.in_set(super::SoundSet));
    }
}

fn listen(
    mut voices: Voices,
    mut commands: Commands,
    game: Res<crate::assets::GameAssets>,
    index: Res<EntityIndex>,
    mut casters: Query<(Entity, &WorldEntity, &Transform, Option<&mut HeardCasts>)>,
    positions: Query<&Transform, With<WorldEntity>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Sound);
    let bank = game.sounds();
    for (id, world, transform, heard) in &mut casters {
        let Some(mut heard) = heard else {
            commands.entity(id).insert(HeardCasts {
                begun: world.casts_begun,
                released: world.casts_released,
                landed: world.casts_landed,
                cancelled: world.casts_cancelled,
                precast: None,
            });
            continue;
        };
        let Some(sounds) = bank.cast_sounds(world.last_spell).copied() else {
            heard.begun = world.casts_begun;
            heard.released = world.casts_released;
            heard.landed = world.casts_landed;
            heard.cancelled = world.casts_cancelled;
            continue;
        };
        let at = Place::At(transform.translation);

        // **Every end of a wind-up takes its voice off**, and there are three
        // of them: the release, the cancellation (interrupted, refused, moved)
        // and a *new* cast beginning on top. Each is a counter moving, which
        // is the same shape every other consumer in this client reads.
        let ended = world.casts_released != heard.released
            || world.casts_cancelled != heard.cancelled
            || world.casts_begun != heard.begun;
        if ended {
            if let Some(voice) = heard.precast.take() {
                commands.entity(voice).try_despawn();
            }
        }
        heard.cancelled = world.casts_cancelled;

        if world.casts_begun != heard.begun {
            heard.begun = world.casts_begun;
            if sounds.precast != 0 {
                heard.precast = voices.play(&bank, sounds.precast, at);
            }
        }
        if world.casts_released != heard.released {
            heard.released = world.casts_released;
            if sounds.cast != 0 {
                voices.play(&bank, sounds.cast, at);
            }
            if sounds.missile != 0 {
                // At the caster, where the throw is: the flight itself is not
                // voiced, which is the missile pass's refinement to claim.
                voices.play(&bank, sounds.missile, at);
            }
        }
        if world.casts_landed != heard.landed {
            heard.landed = world.casts_landed;
            if sounds.impact != 0 {
                // At the victim when the world still holds it, else where the
                // caster is — the same fallback the impact art takes.
                let place = index
                    .0
                    .get(&world.last_spell_target)
                    .and_then(|&e| positions.get(e).ok())
                    .map(|t| Place::At(t.translation))
                    .unwrap_or(at);
                voices.play(&bank, sounds.impact, place);
            }
        }
    }
}
