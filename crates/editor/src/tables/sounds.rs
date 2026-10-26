//! Playing sounds for the panels: a `SoundEntries` row a kit's sound column
//! names, a row or a file a play button names, and a converted file the
//! Import Sound dialog has not written yet; and stopping what plays.

use std::sync::Arc;

use vale_assets::tables::sound::SoundBank;
use vale_client::assets::GameAssets;
use bevy::prelude::*;
use super::browser::Browser;
use crate::session::EditSession;

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
pub(crate) struct Heard {
    pub(crate) table: &'static str,
    pub(crate) id: u32,
    pub(crate) night: bool,
}

/// A request to [`listen`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Listen {
    /// Play the `SoundEntries` row `entry` on `channel`, as `heard`. The
    /// ambience channel loops, as the game loops it; the other two play once.
    Play {
        heard: Heard,
        entry: u32,
        channel: vale_client::sound::mixer::Channel,
    },
    /// Play the file at archive path `path` once, on `channel`, as the Assets
    /// workspace plays one of the project's sounds.
    File {
        path: String,
        channel: vale_client::sound::mixer::Channel,
    },
    /// Play a sound's bytes once, on `channel` at `volume`: a file the Import
    /// Sound dialog has converted and not yet written. `key` is what
    /// [`Browser::hearing_file`] holds while it plays.
    Bytes {
        key: String,
        bytes: Vec<u8>,
        volume: f32,
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
/// The sound is stopped when the tool shown is neither one with a table nor
/// the Assets workspace, and when a playtest begins, because a looping
/// ambience has no other end.
pub(super) fn listen(
    browser: Option<ResMut<Browser>>,
    tool: Res<crate::tools::Tool>,
    popout: Res<crate::popout::Popout>,
    playtest: Res<crate::playtest::Playtest>,
    assets: Res<GameAssets>,
    mut session: Option<ResMut<EditSession>>,
    mut voices: vale_client::sound::mixer::Voices,
    alive: Query<(), With<bevy::audio::AudioPlayer>>,
    // The voice and the volume it plays at, which a fade starts from.
    mut playing: Local<Option<(Entity, f32)>>,
    // The edited `SoundEntries` table's bank and the revision it was read at.
    mut edited: Local<Option<(u64, Arc<SoundBank>)>>,
) {
    use vale_client::sound::mixer::{Channel, Place};
    let Some(mut browser) = browser else { return };
    if let Some((entity, _)) = *playing {
        if alive.get(entity).is_err() {
            *playing = None;
            browser.hearing = None;
            browser.hearing_file = None;
        }
    }
    let shown = popout.browser_tool(*tool).table().is_some()
        || matches!(*tool, crate::tools::Tool::Assets | crate::tools::Tool::GameFiles);
    let away = !shown || !playtest.editing();
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
    browser.hearing_file = None;
    let once = bevy::audio::PlaybackSettings::DESPAWN;
    let (voice, volume, channel) = match ask {
        Listen::Stop => return,
        Listen::File { path, channel } => {
            let voice = voices.play_file(&path, 1.0, channel, Place::Flat, once);
            if voice.is_some() {
                browser.hearing_file = Some(path);
            }
            (voice, 1.0, channel)
        }
        Listen::Bytes { key, bytes, volume, channel } => {
            let voice = voices.play_bytes(bytes, volume, channel, once);
            if voice.is_some() {
                browser.hearing_file = Some(key);
            }
            (voice, volume, channel)
        }
        Listen::Play { heard, entry, channel } => {
            let bank = bank_of(session.as_deref(), &assets, &mut edited);
            let Some((path, volume)) = voices.pick(&bank, entry) else {
                return;
            };
            let settings = match channel {
                Channel::Ambience => bevy::audio::PlaybackSettings::LOOP,
                _ => once,
            };
            let voice = voices.play_file(&path, volume, channel, Place::Flat, settings);
            if voice.is_some() {
                browser.hearing = Some(heard);
            }
            (voice, volume, channel)
        }
    };
    if let Some(entity) = voice {
        let gain = voices.gain(channel);
        *playing = Some((entity, volume * gain));
        // The game's sound settings decide what is heard, and a channel they
        // switch off plays at no volume. Said on the status line, since the
        // button shows Stop all the same.
        if gain == 0.0 {
            if let Some(session) = session.as_mut() {
                session.status = format!("{} is silent: {}", channel_words(channel), silenced_by(channel));
            }
        }
    }
}

/// What the status line calls a mixer channel.
fn channel_words(channel: vale_client::sound::mixer::Channel) -> &'static str {
    use vale_client::sound::mixer::Channel;
    match channel {
        Channel::Effects => "the sound effects channel",
        Channel::Music => "the music channel",
        Channel::Ambience => "the ambience channel",
    }
}

/// The game settings that switch a channel off or turn it to zero, as
/// `vale_client::sound::mixer::Channel::gain` reads them from
/// `WTF\Config.wtf`.
fn silenced_by(channel: vale_client::sound::mixer::Channel) -> &'static str {
    use vale_client::sound::mixer::Channel;
    match channel {
        Channel::Effects => "MasterSoundEffects is 0, or MasterVolume or SoundVolume is 0, in WTF\\Config.wtf",
        Channel::Music => "EnableMusic is 0, or MasterVolume or MusicVolume is 0, in WTF\\Config.wtf",
        Channel::Ambience => {
            "EnableAmbience or MasterSoundEffects is 0, or MasterVolume or AmbienceVolume is 0, in WTF\\Config.wtf"
        }
    }
}

/// The sound bank a button plays from. While the session has `SoundEntries`
/// open, the bank is built from the open table, cached by
/// `EditSession::table_revision`, so a row just added or changed plays as it
/// is now, saved or not. A save writes the table into the project but does
/// not make `GameAssets` parse it again. With the table not open, the bank is
/// `GameAssets::sounds`.
fn bank_of(
    session: Option<&EditSession>,
    assets: &GameAssets,
    edited: &mut Option<(u64, Arc<SoundBank>)>,
) -> Arc<SoundBank> {
    const TABLE: &str = "SoundEntries";
    let Some(session) = session.filter(|session| session.table(TABLE).is_some()) else {
        return assets.sounds();
    };
    if let Some((at, bank)) = edited.as_ref() {
        if *at == session.table_revision {
            return Arc::clone(bank);
        }
    }
    let bytes = session.table(TABLE).map(|table| table.write());
    let read = SoundBank::load(|table| match table == TABLE {
        true => bytes.clone(),
        false => None,
    });
    match read {
        Ok(bank) => {
            let bank = Arc::new(bank);
            *edited = Some((session.table_revision, Arc::clone(&bank)));
            bank
        }
        Err(_) => assets.sounds(),
    }
}
