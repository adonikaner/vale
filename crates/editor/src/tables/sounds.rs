//! Playing a `SoundEntries` row: the audition a panel asks for, and the play
//! and stop buttons beside a sound column.

use vale_client::assets::GameAssets;
use bevy::prelude::*;
use super::browser::Browser;

/// Play the sounds the panels queued, through the client's own voices; see
/// [`Browser::audition`].
pub(super) fn audition(
    browser: Option<ResMut<Browser>>,
    assets: Res<GameAssets>,
    mut voices: vale_client::sound::mixer::Voices,
) {
    let Some(mut browser) = browser else { return };
    if browser.audition.is_empty() {
        return;
    }
    let bank = assets.sounds();
    for id in browser.audition.drain(..) {
        voices.play(&bank, id, vale_client::sound::mixer::Place::Flat);
    }
}

/// One sound a panel can play and then stop: a row of `SoundEntries`,
/// `ZoneMusic`, `ZoneIntroMusicTable` or `SoundAmbience`, and for a row with
/// a day and a night sound, which of the two.
///
/// A kit's sound is short and [`Browser::audition`] plays it to its end. A
/// zone's music is a track of minutes and its ambience is a loop, so these
/// are played one at a time and can be stopped: a second play replaces the
/// first, and the button that started one stops it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heard {
    pub table: &'static str,
    pub id: u32,
    pub night: bool,
}

/// A request to [`listen`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Listen {
    /// Play the `SoundEntries` row `entry` on `channel`, as `heard`. The
    /// ambience channel loops, as the game loops it; the other two play once.
    Play {
        heard: Heard,
        entry: u32,
        channel: vale_client::sound::mixer::Channel,
    },
    Stop,
}

/// How long a stopped sound takes to fade, which is short enough to be a stop
/// and long enough not to click.
const STOP_SECS: f32 = 0.25;

/// Play or stop what a panel asked for in [`Browser::listen`], through the
/// client's mixer and at its channel volumes.
///
/// The sound stops by itself when the browser's tools are left or a
/// playtest begins, since a looping ambience has no other end.
pub(super) fn listen(
    browser: Option<ResMut<Browser>>,
    tool: Res<crate::tools::Tool>,
    popout: Res<crate::popout::Popout>,
    playtest: Res<crate::playtest::Playtest>,
    assets: Res<GameAssets>,
    mut voices: vale_client::sound::mixer::Voices,
    alive: Query<(), With<bevy::audio::AudioPlayer>>,
    // The voice and the volume it plays at, which a fade starts from.
    mut playing: Local<Option<(Entity, f32)>>,
) {
    use vale_client::sound::mixer::{Channel, Place};
    let Some(mut browser) = browser else { return };
    if let Some((entity, _)) = *playing {
        if alive.get(entity).is_err() {
            *playing = None;
            browser.hearing = None;
        }
    }
    let away = popout.browser_tool(*tool).table().is_none() || !playtest.editing();
    let ask = match away {
        true => {
            browser.listen = None;
            playing.is_some().then_some(Listen::Stop)
        }
        false => browser.listen.take(),
    };
    let Some(ask) = ask else { return };
    if let Some((entity, volume)) = playing.take() {
        voices.fade_out(entity, volume, STOP_SECS);
    }
    browser.hearing = None;
    let Listen::Play {
        heard,
        entry,
        channel,
    } = ask
    else {
        return;
    };
    let bank = assets.sounds();
    let Some((path, volume)) = voices.pick(&bank, entry) else {
        return;
    };
    let settings = match channel {
        Channel::Ambience => bevy::audio::PlaybackSettings::LOOP,
        _ => bevy::audio::PlaybackSettings::DESPAWN,
    };
    if let Some(entity) = voices.play_file(&path, volume, channel, Place::Flat, settings) {
        *playing = Some((entity, volume * voices.gain(channel)));
        browser.hearing = Some(heard);
    }
}
