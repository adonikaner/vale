//! **The interface's four sound verbs, drained** — the other end of
//! [`crate::lua::api::sound`]'s queue.
//!
//! `PlaySound("igMainMenuOption")` is a **name lookup into `SoundEntries`** —
//! measured, not assumed: 99 of the interface's `gs*`/`ig*` strings are rows
//! of that table's name column, which is the whole of how the game's own
//! FrameXML clicks and checkbox ticks make noise. `PlaySoundFile` plays a
//! path as it is; `PlayMusic`/`StopMusic` set and clear the music channel's
//! override, and [`super::music`] does the playing — one channel, one
//! decider.

use super::mixer::{Channel, Place, Voices};
use crate::lua::api::sound::SoundRequest;
use bevy::prelude::*;

pub struct InterfacePlugin;

impl Plugin for InterfacePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, drain.in_set(super::SoundSet));
    }
}

fn drain(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut voices: Voices,
    mut music: ResMut<super::music::MusicState>,
    game: Res<crate::assets::GameAssets>,
) {
    let Some(mut host) = host else {
        return;
    };
    // **A music claim belongs to the interface that made it**, and this is the
    // only system holding both the host and the channel. Checked before the
    // early return below, because the frame that matters has no requests on it
    // at all: the login screen says `PlayGlueMusic` once and then the whole Lua
    // state is replaced on the way into the world, with nothing left to say
    // `StopMusic`. See [`super::music`].
    if music.interface_changed(host.directory()) {
        debug!("music: the interface that asked for it is gone — the override is dropped");
    }
    let requests = host.take_sound_requests();
    if requests.is_empty() {
        return;
    }
    let bank = game.sounds();
    for request in requests {
        match request {
            SoundRequest::Named(name) => {
                if let Some(entry) = bank.entry_named(&name) {
                    voices.play(&bank, entry.id, Place::Flat);
                } else {
                    // Once per session would be better; once per call is
                    // honest enough for a name the game's own files spell
                    // right — this fires on addons and typos.
                    debug!("PlaySound({name:?}): no SoundEntries row by that name");
                }
            }
            SoundRequest::File(path) => {
                voices.play_file(
                    &path,
                    1.0,
                    Channel::Effects,
                    Place::Flat,
                    bevy::audio::PlaybackSettings::DESPAWN,
                );
            }
            SoundRequest::Music(path) => music.override_path = Some(path),
            SoundRequest::StopMusic => music.override_path = None,
        }
    }
}
