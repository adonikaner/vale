//! **The loop under everything** — `SoundAmbience.dbc`, per place, day against
//! night.
//!
//! Unlike the music channel this never goes quiet on purpose: an ambience is
//! a looping bed (wind, insects, the city's murmur) that changes only when
//! the place or the hour's half of the pair does.
//!
//! ## The change is a cross-fade
//!
//! The client's rule, and it is three rules rather than one:
//!
//! ```text
//! change(immediate)
//!   wanted = the wanted SoundEntries id         (the day/night pick)
//!   if a bed is playing:
//!       playing == wanted  -> return            ; the same bed: leave it alone
//!       immediate          -> hard stop         ; the cut entry point
//!       otherwise          -> Fade(5.0) + free  ; and forget the handle
//!   start the new voice
//!   if not immediate: SetVolume(0), then Fade(5.0, entry->volume)
//!   else             SetVolume(entry->volume)
//! ```
//!
//! * **Five seconds, both directions, overlapping.** The outgoing bed keeps
//!   playing while it falls and the incoming one rises from silence, which is
//!   what makes a boundary a blend rather than a cut.
//! * **The same bed is left alone.** The wanted id is compared against the
//!   playing one, not the area — so crossing from Elwynn into Goldshire, which
//!   share ambience 35, does not restart the wind.
//! * **A cut is a separate entry point**, not a
//!   shorter fade, and `Fade` itself treats anything under 0.1 s as a set.
//!
//! ## …and where the id comes from
//!
//! [`crate::interface::worldmap::WorldMapState::sounds`] — already resolved through
//! the building, the area and the zone, a column at a time. This module used to
//! do that join itself and did it a *row* at a time, which is why a tavern
//! played the forest outside it and 306 of the game's subzones had no bed at
//! all.

use super::mixer::{Channel, Place, Voices};
use vale_assets::tables::sound::day_index;
use bevy::audio::PlaybackSettings;
use bevy::prelude::*;

/// The cross-fade, in seconds, used in both directions.
const AMBIENCE_FADE_SECS: f32 = 5.0;

#[derive(Resource, Default)]
pub struct AmbienceState {
    /// The looping voice and the `SoundEntries` id it is looping, as the
    /// reference keeps them.
    playing: Option<(Entity, u32)>,
}

pub struct AmbiencePlugin;

impl Plugin for AmbiencePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AmbienceState>()
            .add_systems(Update, run_ambience.in_set(super::SoundSet));
    }
}

fn run_ambience(
    mut state: ResMut<AmbienceState>,
    mut voices: Voices,
    session: Res<crate::world::session::Session>,
    worldmap: Res<crate::interface::worldmap::WorldMapState>,
    clock: Res<crate::render::sky::WorldClock>,
    game: Res<crate::assets::GameAssets>,
    // …and the sky, whose loop outranks the place's. See below.
    status: Res<crate::world::session::WorldStatus>,
) {
    let in_world =
        session.screen() == crate::world::session::Screen::InWorld && session.active.is_some();
    let bank = game.sounds();
    // **The weather's loop is played on this bed, ahead of the place's** —
    // and that is the reference's own routing rather than a convenience.
    // `SMSG_WEATHER`'s handler stores its sound id, writes the wanted entry
    // into the *ambience* slot, raises the weather flag, and falls into the
    // same five-second cross-fade the module note above quotes. So a downpour's loop replaces
    // the wind and the crickets rather than playing over them, and the
    // moment the server says fine weather with sound 0 the place's own bed
    // comes back through the same fade. One channel, one voice, one rule.
    let weather = status.weather.map_or(0, |w| w.sound);
    let wanted = if !in_world {
        0
    } else if weather != 0 {
        weather
    } else {
        let hm = clock.override_half_minutes.unwrap_or(clock.half_minutes);
        bank.ambience(worldmap.sounds.ambience)
            .map(|row| row.sounds[day_index((hm % 2880) / 120)])
            .unwrap_or(0)
    };

    // The bed already playing is the wanted one: nothing happens. Keyed on the
    // *entry* rather than on the area, which is the reference's own comparison
    // and is what stops a subzone crossing restarting the wind.
    if state.playing.map(|(_, id)| id) == Some(wanted).filter(|&id| id != 0) {
        return;
    }
    if let Some((entity, id)) = state.playing.take() {
        // The ramp is in effective volume, so the entry's own figure goes
        // through the channel — see [`Channel::gain`].
        let gain = voices.gain(Channel::Ambience);
        let volume = bank.entry(id).map_or(gain, |entry| entry.volume * gain);
        voices.fade_out(entity, volume, AMBIENCE_FADE_SECS);
    }
    if wanted == 0 {
        return;
    }
    if let Some((path, volume)) = voices.pick(&bank, wanted) {
        if let Some(entity) = voices.play_file_fading_in(
            &path,
            volume,
            Channel::Ambience,
            Place::Flat,
            PlaybackSettings::LOOP,
            AMBIENCE_FADE_SECS,
        ) {
            state.playing = Some((entity, wanted));
        }
    }
}
