//! **Footsteps** — the stride's two footfalls, on what the ground is made of.
//!
//! The chain, every link the game's own (see [`vale_assets::tables::sound`]):
//! display id -> `CreatureSoundData`'s footstep key -> crossed with the
//! terrain under the entity — `MCLY`'s `effectId` -> `GroundEffectTexture`'s
//! sound column — in `FootstepTerrainLookup`. Grass and dirt and stone sound
//! different because the *texture painters* said so, chunk by chunk.
//!
//! **When** a foot lands is this module's approximation and is stated in
//! `sound/mod.rs`: two footfalls per gait cycle, at the half-cycle points,
//! read off the same clock the legs are posed by ([`Playback::stride`]). The
//! real client fires them off the model's own animation events.
//!
//! **And whose feet they are is the other question**, which this used to get
//! wrong for every mounted character in the game. A rider plays `Mount` (91) —
//! not one of the three gaits [`Playback::stride`] answers for — so its clock
//! correctly says the legs are still: they *are*. Both halves of the chain
//! belong to the animal instead, and neither is a fallback for the other's
//! absence: the stride, because that is the rig with feet in it, and the
//! display id, because `CreatureSoundData`'s footstep column is keyed by
//! display and the rider's answers a human's boots. See [`Mount::footfall`].

use super::mixer::{Place, Voices};
use crate::world::entities::{Mount, Playback};
use crate::world::session::WorldEntity;
use bevy::prelude::*;

/// Where in the cycle this entity's last footfall was, so a wrap is an event.
#[derive(Component, Default)]
pub struct Feet {
    /// The half-cycle (0 or 1) the stride was in last frame, or `None` while
    /// the legs are not striding.
    half: Option<u8>,
}

/// Everything [`step`] needs about one entity: what it is, where it stands,
/// what its own legs are doing, what it is *riding*, and where its last footfall
/// was.
///
/// [`Feet`] is optional because it is inserted on first sight rather than at
/// spawn — an entity is not born with a footfall in progress — and the mount is
/// optional because most things are not riding one.
type Walker = (
    Entity,
    &'static WorldEntity,
    &'static Transform,
    &'static Playback,
    Option<&'static Mount>,
    Option<&'static mut Feet>,
);

pub struct FootstepsPlugin;

impl Plugin for FootstepsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, step.in_set(super::SoundSet));
    }
}

fn step(
    mut voices: Voices,
    mut commands: Commands,
    time: Res<Time>,
    session: Res<crate::world::session::Session>,
    game: Res<crate::assets::GameAssets>,
    mut entities: Query<Walker>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let bank = game.sounds();
    let now = time.elapsed_secs();
    for (id, world, transform, playback, mount, feet) in &mut entities {
        // Swimming and airborne feet touch nothing. The splash column of the
        // footstep table exists and is not played yet — a wading state needs
        // the water depth this loop does not know.
        //
        // **The animal's clock and the animal's display, whenever there is
        // one** — see the module note. Not `or_else`: a mounted rider whose
        // horse is standing has no footfall, and falling through to the rider
        // would give it the *rider's*, which is a boot on a horse.
        let walker = (!world.swimming && !world.airborne).then(|| match mount {
            Some(mount) => mount.footfall(now),
            None => match (playback.stride(now), world.display_id) {
                (Some((_, fraction)), Some(display)) => Some((display, fraction)),
                _ => None,
            },
        });
        let Some(mut feet) = feet else {
            commands.entity(id).insert(Feet::default());
            continue;
        };
        let Some((display, fraction)) = walker.flatten() else {
            feet.half = None;
            continue;
        };
        let half = (fraction >= 0.5) as u8;
        let landed = feet.half.is_some_and(|last| last != half);
        feet.half = Some(half);
        if !landed {
            continue;
        }
        // The joins, only paid on a footfall of something audible.
        let Some(key) = bank.unit_sounds(display).map(|s| s.footstep) else {
            continue;
        };
        if key == 0 {
            continue;
        }
        let wow = crate::render::axes::to_wow(transform.translation);
        let terrain = active
            .terrain_ground_effect(active.map_id, wow[0], wow[1])
            .map(|effect| bank.terrain_of_ground_effect(effect))
            .unwrap_or(0);
        if let Some(cell) = bank.footstep(key, terrain) {
            voices.play(&bank, cell.sound, Place::At(transform.translation));
        }
    }
}
