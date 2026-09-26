//! **The three sounds the server asks for outright** — `SMSG_PLAY_SOUND`,
//! `SMSG_PLAY_MUSIC` and `SMSG_PLAY_OBJECT_SOUND`.
//!
//! Every other file in this directory *derives* a sound: a counter moved, a
//! stride wrapped, an area changed, a message was said. These three are the
//! only ones where the server states the noise itself, and nothing else on the
//! wire implies any of them. A boss line, a gate grinding open, a Deeprun Tram
//! station announcing itself, `Map::PlayDirectSoundToMap` over a whole zone,
//! the outdoor PvP banners and every `SCRIPT_COMMAND_PLAY_SOUND` row in the
//! world database arrive here or are silent.
//!
//! ## The three differ only in where
//!
//! * [`Cue::Direct`] is flat, at the listener. `PlayDirectSound`.
//! * [`Cue::Object`] is placed at the named object, so it attenuates and pans.
//!   `PlayDistanceSound`, and vmangos' own comment on it states the reference's
//!   behaviour: *"ignored by client if unit is not loaded"* — a sound whose
//!   guid this client has never heard of is **dropped**, not played flat. That
//!   is followed here rather than softened, because the alternative is a
//!   distant scripted noise arriving at full volume in the player's ear.
//! * [`Cue::Music`] is the music channel rather than the effects one, so it
//!   replaces the zone's track instead of layering over it. The claim lives in
//!   [`super::music`], which is the one decider for that channel — this module
//!   only writes the id, exactly as [`super::interface`] writes `PlayMusic`'s
//!   path.

use super::mixer::{Place, Voices};
use crate::game::events::SoundPushed;
use crate::world::session::{EntityIndex, WorldEntity};
use vale_protocol::play::sound::Cue;
use bevy::prelude::*;

pub struct PushedPlugin;

impl Plugin for PushedPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (drain, kits).in_set(super::SoundSet));
    }
}

/// The two pushed-kit counters this unit has already been heard for.
#[derive(Component)]
struct HeardKits {
    visuals: u32,
    impacts: u32,
}

/// **…and the sound half of the other two**, which does not come through the
/// queue above at all: `SMSG_PLAY_SPELL_VISUAL` names a `SpellVisualKit`, and
/// that row's field 13 is a `SoundEntries` id.
///
/// So the same packet that puts a plate of food at a character's feet also says
/// what eating sounds like — kit 406 reads sound **45** and kit 438 reads
/// **3373** — and neither is reachable any other way, because every other sound
/// in this directory is looked up spell-first. See
/// [`vale_assets::tables::sound::SoundBank::kit_sound`].
///
/// Per entity and edge-driven, exactly as [`super::spells`] is, and **primed on
/// first sight** for the same reason: a character already eating when they come
/// into view has a counter this client has never read, and acting on it would
/// play a bite for every diner in the inn the moment the door opened.
fn kits(
    mut voices: Voices,
    mut commands: Commands,
    game: Res<crate::assets::GameAssets>,
    mut units: Query<(Entity, &WorldEntity, &Transform, Option<&mut HeardKits>)>,
) {
    let bank = game.sounds();
    for (id, world, transform, heard) in &mut units {
        let Some(mut heard) = heard else {
            commands.entity(id).insert(HeardKits {
                visuals: world.spell_visuals,
                impacts: world.spell_impacts,
            });
            continue;
        };
        let moved_visual = world.spell_visuals != heard.visuals;
        let moved_impact = world.spell_impacts != heard.impacts;
        heard.visuals = world.spell_visuals;
        heard.impacts = world.spell_impacts;
        if !moved_visual && !moved_impact {
            continue;
        }
        // The visual when both moved in one poll, which is the order the pose
        // and the models take: what the unit did outranks what was done to it.
        let kit = match moved_visual {
            true => world.last_spell_visual,
            false => world.last_spell_impact,
        };
        if let Some(sound) = bank.kit_sound(kit) {
            voices.play(&bank, sound, Place::At(transform.translation));
        }
    }
}

fn drain(
    mut cues: MessageReader<SoundPushed>,
    mut voices: Voices,
    mut music: ResMut<super::music::MusicState>,
    game: Res<crate::assets::GameAssets>,
    index: Res<EntityIndex>,
    placed: Query<&GlobalTransform>,
) {
    // Collected first so the reader is drained whether or not the bank is up:
    // an unread message would arrive on whatever frame this system next ran,
    // which for a scripted noise is the wrong scene.
    let cues: Vec<Cue> = cues.read().map(|pushed| pushed.0).collect();
    if cues.is_empty() {
        return;
    }
    let bank = game.sounds();
    for cue in cues {
        match cue {
            Cue::Direct(id) => {
                voices.play(&bank, id, Place::Flat);
            }
            // **Written, not played.** The music channel has one decider and it
            // is not this system; see [`super::music`]'s claim cascade for
            // where a pushed track sits against the interface's `PlayMusic`,
            // the glue theme and the zone playlist.
            Cue::Music(id) => music.pushed = Some(id),
            Cue::Object { sound_id, guid } => {
                // The reference drops one whose unit it does not hold. Both
                // halves of that are real misses: a guid we have never been
                // sent, and one we have been sent but have not yet placed.
                let Some(at) = index
                    .0
                    .get(&guid)
                    .and_then(|entity| placed.get(*entity).ok())
                    .map(|transform| transform.translation())
                else {
                    debug!("SMSG_PLAY_OBJECT_SOUND {sound_id}: object {guid:#x} is not loaded");
                    continue;
                };
                voices.play(&bank, sound_id, Place::At(at));
            }
        }
    }
}
