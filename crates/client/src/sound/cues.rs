//! **The sounds a model's own animation asks for** — `$CSD` events, which
//! are how a character laughs, cries, kisses and claps.
//!
//! An M2 carries events on its timeline (`vale_assets::world::m2::M2Event`),
//! and `$CSD`'s data word is a `SoundEntries` id: `HumanMale.m2` has
//! `HumanMaleEmoteLaugh` (6923) at the first moment of `EmoteLaugh`,
//! `HumanMaleEmoteCry` at `EmoteCry`'s, `ClapSounds` four times through
//! `EmoteApplaud`. No table maps an emote to a race and a gender; the race's
//! model carries its own voice at its own moment, which is why `/lol` had no
//! table to be found in (`EmotesTextSound.dbc` is the twenty-seven spoken
//! ones — `/charge`, `/silly` — and nothing else).
//!
//! [`fire`] runs once a frame over every posed rig: the pose pass records
//! the window of the sequence each rig advanced through this frame
//! ([`Playback::window`]), the model's cues are pre-sorted per sequence
//! ([`vale_assets::world::m2::SoundCues`], built once when the model
//! loads), and every cue inside the window plays **on the unit** —
//! [`Place::On`], a child of the unit's entity — so a voice moves with the
//! one making it rather than staying where the emote began, which was the
//! other report. A rig the camera cannot see is still posed as far as its
//! clock and still fires: a laugh behind you is heard.
//!
//! What is deliberately not here: the other thirty-odd identifiers
//! (`$CAH` the attack hit, `$FSD` the footstep, `$BTH` the breath, `$CSS`,
//! `$CPP` …), which the client switches on and which this
//! client answers elsewhere or not at all. The footfall has its own pass
//! off the stride rather than off `$FSD`; whether the two agree is a
//! measurement not yet made.

use bevy::prelude::*;

use super::mixer::{Place, Voices};
use crate::world::entities::{EntityModel, Playback};
use crate::world::session::WorldEntity;

pub struct CuesPlugin;

impl Plugin for CuesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, fire.in_set(super::SoundSet));
    }
}

fn fire(
    mut voices: Voices,
    session: Res<crate::world::session::Session>,
    game: Res<crate::assets::GameAssets>,
    rigs: Query<(Entity, &WorldEntity, &EntityModel, &Playback, &Transform)>,
) {
    if session.active.is_none() {
        return;
    }
    let bank = game.sounds();
    for (entity, _, model, playback, transform) in &rigs {
        if model.cues.is_empty() {
            continue;
        }
        let Some((sequence, from, to, fresh)) = playback.window() else {
            continue;
        };
        for sound in model.cues.in_window(sequence, from, to, fresh) {
            voices.play(&bank, sound, Place::On(entity, transform.translation));
        }
    }
}
