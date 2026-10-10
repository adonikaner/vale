//! **The music channel** — one voice, four claimants, in priority order: the
//! interface's `PlayMusic`, the glue screens' theme, a zone's intro fanfare,
//! and the zone's own playlist.
//!
//! ## The playlist rule
//!
//! `ZoneMusic.dbc` is not a loop: it is a track, then a silence drawn from the
//! row's own `min..max` (three to five minutes for most zones), then another
//! track — which is why the world is quiet most of the time and a track
//! arriving feels like weather. Day and night have separate tracks and
//! separate silences, indexed by [`vale_assets::tables::sound::day_index`].
//!
//! ## The glue theme
//!
//! `Sound\Music\GlueScreenMusic\wow_main_theme.mp3` — not a guess: it is
//! `CurrentGlueMusic` in the game's own `Interface\GlueXML\GlueParent.lua`,
//! line 2. The glue screens are that directory now, so the theme does not come
//! from the constant below any more: `SetGlueScreen` ends in
//! `PlayGlueMusic(CurrentGlueMusic)`, which is `PlayMusic` under another name —
//! [`Claim::Lua`], not [`Claim::Glue`]. The constant is the fallback for a
//! screen the interface has not told us about.
//!
//! The theme is the login screen's, so it is played only while the login
//! screen may show its scene: [`crate::render::glue::GlueScenes`], which is on
//! in the client. A host that draws the world with no login screen behind it
//! turns that off, and this channel is then silent outside the world.
//!
//! ## …and a claim belongs to the interface that made it
//!
//! `PlayMusic` outranks everything until `StopMusic`, and **nothing calls
//! `StopMusic` on the way into the world**: `GlueParent.lua` has no arm for it,
//! because in the real client the whole process is torn down between the login
//! screen and the game. Here the process lives on and only the Lua state is
//! replaced ([`crate::lua::host::LuaHost`] builds a fresh one at both edges of a
//! session), so the login theme held the channel for the rest of the session —
//! looping over the world and keeping every zone track off the air, which is
//! reported exactly that way.
//!
//! [`MusicState::interface_changed`] is the rule that fixes it, and it is the
//! same one `glue::glue::Told` already carries one directory over: **a record of
//! what the interface has been told is only about the interface it was told
//! to.** When the loaded directory changes, the override goes with the state
//! that asked for it.
//!
//! ## The intro fanfare
//!
//! `ZoneIntroMusicTable` — "Valley of Heroes", the Orgrimmar horns — plays on
//! entering the area that names it, at most once per its own `min_delay`
//! minutes, and takes the channel the way a track does.
//!
//! ## **Music fades out and never fades in**, which is not what it sounds like
//!
//! The ambience cross-fades (see [`super::ambience`]); this channel does not,
//! and the asymmetry is the client's own. Every track here starts at its
//! entry's full volume — both start paths `SetVolume` outright — and what
//! fades is only the track being *replaced*:
//!
//! ```text
//! the zone's music id changed        Fade(4.0) the zone track
//! PlayMusic took the channel         Fade(4.0) the zone track
//! an intro replaced an intro         Fade(3.0)
//! an intro replaced the zone track   Fade(3.0)
//! ```
//!
//! So a crossing sounds like the old track receding under a new one that is
//! already at full strength, which is the reference. Cutting instead — this
//! client until now — is what "all sounds cut off completely to play something
//! else" was reporting.

use super::mixer::{Channel, Place, Voices};
use vale_assets::tables::sound::day_index;
use bevy::audio::{AudioPlayer, PlaybackSettings};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

/// See the module comment — GlueParent.lua:2.
const GLUE_MUSIC: &str = "Sound\\Music\\GlueScreenMusic\\wow_main_theme.mp3";

/// How long after entering a zone (or logging in) the first track waits —
/// 6000 ms, the client's branch when no `ZoneMusic` row is current. It was a
/// guessed 5.0 here until the client's own value was known.
const FIRST_TRACK_SECS: f32 = 6.0;

/// A track handing the channel to another track.
const HANDOVER_SECS: f32 = 4.0;

/// …and to an intro fanfare, which is more of a hurry.
const INTRO_HANDOVER_SECS: f32 = 3.0;

/// What the channel is currently playing, and for whom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Claim {
    Glue,
    Zone,
    Intro,
    /// **The server said so** — `SMSG_PLAY_MUSIC`, which is the one music
    /// claim with no table behind it. Above the zone playlist and its intro,
    /// below the interface's own `PlayMusic`: a scripted track is the point of
    /// the scene it belongs to, and an addon asking explicitly still wins.
    Pushed,
    /// The interface's `PlayMusic`, which outranks everything until
    /// `StopMusic` — see [`super::interface`].
    Lua,
}

#[derive(Resource, Default)]
pub struct MusicState {
    /// The voice, whose claim it is, and **the volume it was started at** —
    /// which a fade-out needs and the sink cannot be asked for from here.
    playing: Option<(Entity, Claim, f32)>,
    /// `PlayMusic`'s path, held until `StopMusic`. Written by
    /// [`super::interface`], read here — one decider, like the world clock.
    pub override_path: Option<String>,
    /// **Which interface directory asked for it** — see the module comment.
    /// `None` before anything has, and the value that makes the override
    /// survive the state that made it a bug rather than a feature.
    override_dir: Option<crate::lua::host::Directory>,
    /// When the zone playlist may next start a track.
    silence_until: f32,
    /// The `ZoneMusic` id the state above was computed for.
    music_id: u32,
    /// **A `SoundEntries` id the server pushed**, held until its voice ends.
    ///
    /// Set by [`super::pushed`] and read here, exactly as `override_path` is
    /// set by [`super::interface`] — one decider for the channel.
    pub pushed: Option<u32>,
    /// …and which id the current [`Claim::Pushed`] voice is of.
    ///
    /// **A re-push of the track already playing is a no-op, and that is the
    /// server's own usage rather than an optimisation**: the Deeprun Tram's
    /// jukebox script re-sends `SMSG_PLAY_MUSIC` every five seconds with the
    /// same id (vmangos' `go_scripts.cpp` says so, off a sniff). Restarting on
    /// each would stutter the track five seconds in, for ever.
    pushed_playing: u32,
    /// Intro id -> when it last played, for the once-per-delay rule.
    intros: HashMap<u32, f32>,
    /// The silence draw's own randomness — see [`super::mixer::Mixer`] for
    /// why an LCG is enough.
    seed: u32,
}

impl MusicState {
    /// **The interface that made a `PlayMusic` claim has gone**, so the claim
    /// goes with it. Answers whether anything was dropped, which is what the
    /// caller logs.
    ///
    /// Called every frame with whichever directory is loaded, including `None`
    /// for the frame between a teardown and the next load — that frame is the
    /// point of the check, since it is exactly when the state that could have
    /// said `StopMusic` stops existing.
    pub fn interface_changed(&mut self, directory: Option<crate::lua::host::Directory>) -> bool {
        if self.override_dir == directory {
            return false;
        }
        self.override_dir = directory;
        self.override_path.take().is_some()
    }

    fn draw(&mut self, min: u32, max: u32) -> f32 {
        self.seed = self.seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let span = max.saturating_sub(min).max(1);
        ((min + (self.seed >> 8) % span) as f32) / 1000.0
    }
}

pub struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MusicState>()
            .add_systems(Update, run_music.in_set(super::SoundSet));
    }
}

/// The whole channel, one claim at a time.
#[allow(clippy::too_many_arguments)]
fn run_music(
    mut state: ResMut<MusicState>,
    mut voices: Voices,
    session: Res<crate::world::session::Session>,
    worldmap: Res<crate::interface::worldmap::WorldMapState>,
    clock: Res<crate::render::sky::WorldClock>,
    time: Res<Time>,
    game: Res<crate::assets::GameAssets>,
    cvars: Res<crate::settings::cvars::CVars>,
    alive: Query<(), With<AudioPlayer>>,
    glue: Option<Res<crate::render::glue::GlueScenes>>,
    // A ghost hears the ghost playlist instead of the zone's.
    dying: Option<Res<crate::interface::death::Dying>>,
) {
    let now = time.elapsed_secs();
    // A voice that finished has despawned itself (`PlaybackMode::Despawn`);
    // noticing is what arms the next silence.
    let playing = state
        .playing
        .filter(|(entity, ..)| alive.get(*entity).is_ok());
    if playing.is_none() {
        if let Some((_, Claim::Pushed, _)) = state.playing {
            // A pushed track is a one-shot: when it ends the channel goes back
            // to whatever the place sounds like, and a later push of the same
            // id starts it again rather than being swallowed by the guard.
            state.pushed = None;
            state.pushed_playing = 0;
        }
        if let Some((_, Claim::Zone | Claim::Intro, _)) = state.playing {
            // The track just ended: the playlist goes quiet for the row's own
            // silence. Drawn below once the row is in hand.
            state.silence_until = f32::MAX;
        }
        state.playing = None;
    }

    let in_world =
        session.screen() == crate::world::session::Screen::InWorld && session.active.is_some();

    // --- claim 1: the interface said `PlayMusic(path)` ---
    if let Some(path) = state.override_path.clone() {
        if !matches!(state.playing, Some((_, Claim::Lua, _))) {
            // The incoming track is already at full volume when the
            // outgoing one is handed its four seconds.
            hand_over(&mut state, &mut voices, HANDOVER_SECS);
            // `PlayMusic` loops — GlueParent re-raises its own on an event,
            // and 5875's in-world callers expect it to hold.
            let gain = voices.gain(Channel::Music);
            if let Some(entity) =
                voices.play_file(&path, 1.0, Channel::Music, Place::Flat, PlaybackSettings::LOOP)
            {
                state.playing = Some((entity, Claim::Lua, gain));
            } else {
                state.override_path = None;
            }
        }
        return;
    }
    if matches!(state.playing, Some((_, Claim::Lua, _))) {
        // `StopMusic` — the loop is let go of and the ordinary claims resume.
        hand_over(&mut state, &mut voices, HANDOVER_SECS);
    }

    // --- claim 2: the glue screens ---
    // With no login screen to belong to, the theme is not played and the
    // channel is quiet until the world is entered.
    let glue = glue.is_none_or(|scenes| scenes.0);
    if !in_world && !glue {
        hand_over(&mut state, &mut voices, HANDOVER_SECS);
        return;
    }
    if !in_world {
        if !matches!(state.playing, Some((_, Claim::Glue, _))) {
            hand_over(&mut state, &mut voices, HANDOVER_SECS);
            let gain = voices.gain(Channel::Music);
            if let Some(entity) = voices.play_file(
                GLUE_MUSIC,
                1.0,
                Channel::Music,
                Place::Flat,
                PlaybackSettings::LOOP,
            ) {
                state.playing = Some((entity, Claim::Glue, gain));
            }
        }
        return;
    }
    if matches!(state.playing, Some((_, Claim::Glue, _))) {
        // Entering the world ends the theme. The reference tears the whole
        // process down here, so it has no opinion; this takes the handover
        // every other claim takes rather than inventing a third behaviour.
        hand_over(&mut state, &mut voices, HANDOVER_SECS);
    }

    // --- what this place sounds like: the building, the area, then the zone ---
    // Resolved once, in `interface::worldmap`, which is where the building is known.
    let bank = game.sounds();

    // --- claim 2b: the server said so ---
    //
    // Above the intro and the playlist below, because a scripted track is the
    // point of the scene it arrives with; below the interface's `PlayMusic`
    // above, because that is somebody asking outright. Taken here rather than
    // beside the Lua claim so that it has the bank in hand.
    if let Some(id) = state.pushed {
        if state.pushed_playing != id || !matches!(state.playing, Some((_, Claim::Pushed, _))) {
            if let Some((entity, volume)) = play_track(&mut voices, &bank, id) {
                hand_over(&mut state, &mut voices, HANDOVER_SECS);
                state.playing = Some((entity, Claim::Pushed, volume));
                state.pushed_playing = id;
            } else {
                // A row this client cannot resolve to a file is dropped rather
                // than retried every frame for the rest of the session.
                state.pushed = None;
            }
        }
        return;
    }
    let sounds = worldmap.sounds;
    let hm = clock.override_half_minutes.unwrap_or(clock.half_minutes);
    let day = day_index((hm % 2880) / 120);

    // --- claim 3: the intro fanfare, once per its own delay ---
    //
    // **It takes the channel off a zone track rather than waiting for one to
    // end** — the client builds the intro voice and only then fades whatever is
    // playing, which is why the order here is play, hand over, record. Waiting
    // for silence, which is what this did, meant the Valley of Heroes fanfare
    // simply never played for anyone who walked in while Elwynn's track was on.
    if sounds.intro_music != 0 && !matches!(state.playing, Some((_, Claim::Intro, _))) {
        if let Some(intro) = bank.intro_music(sounds.intro_music) {
            let ready = state
                .intros
                .get(&sounds.intro_music)
                .is_none_or(|last| now - last >= intro.min_delay_minutes as f32 * 60.0);
            if ready {
                state.intros.insert(sounds.intro_music, now);
                if let Some((entity, volume)) = play_track(&mut voices, &bank, intro.sound) {
                    hand_over(&mut state, &mut voices, INTRO_HANDOVER_SECS);
                    state.playing = Some((entity, Claim::Intro, volume));
                    return;
                }
            }
        }
    }

    // --- claim 4: the playlist ---
    // While the player is a ghost the 1.12.1 client plays the ghost row in
    // place of the zone's; the change ends the zone's track as a zone
    // crossing does.
    let zone_music = match dying.is_some_and(|d| d.ghost) {
        true => bank
            .zone_music_named(vale_assets::tables::sound::GHOST_MUSIC)
            .unwrap_or(sounds.zone_music),
        false => sounds.zone_music,
    };
    if state.music_id != zone_music {
        state.music_id = zone_music;
        // **A zone's track belongs to that zone, so crossing out of it ends
        // it.** Without this a track started in Elwynn played on through
        // Westfall and Duskwood to its end — reported exactly that way — and
        // then the *new* zone's silence was armed off a row nobody was
        // standing in any more. It *recedes* rather than stopping, which is
        // the client's four seconds. The intro fanfare is deliberately left
        // alone: it is a one-shot belonging to the area boundary that has just
        // been crossed, so cutting it would silence the thing the crossing is
        // for.
        if matches!(state.playing, Some((_, Claim::Zone, _))) {
            hand_over(&mut state, &mut voices, HANDOVER_SECS);
        }
        // A new zone's first track comes quickly; the between-track silences
        // are the row's own.
        if state.playing.is_none() {
            state.silence_until = now + FIRST_TRACK_SECS;
        }
    }
    if state.playing.is_some() || state.music_id == 0 {
        return;
    }
    let Some(music) = bank.zone_music(state.music_id) else {
        return;
    };
    if state.silence_until == f32::MAX {
        // The track that just ended left the draw to us — see the top.
        //
        // …unless the player has asked for **`SoundZoneMusicNoDelay`**, which
        // is the sound panel's "Loop Music" and is exactly this: no silence
        // between tracks. The name is the behaviour — the zone's own
        // `silence_min`/`silence_max` are the delay it says there is none of.
        state.silence_until = match cvars.flag("SoundZoneMusicNoDelay") {
            true => now,
            false => now + state.draw(music.silence_min[day], music.silence_max[day]),
        };
        return;
    }
    if now < state.silence_until || music.sounds[day] == 0 {
        return;
    }
    if let Some((entity, volume)) = play_track(&mut voices, &bank, music.sounds[day]) {
        state.playing = Some((entity, Claim::Zone, volume));
    } else {
        // A row whose file is missing must not retry every frame.
        state.silence_until = now + 60.0;
    }
}

/// One track on the channel: the entry's own volume under the music master,
/// despawning itself at the end — which is what arms the next silence.
///
/// **At full volume from the first sample**, which is the reference's
/// `SetVolume` rather than a fade — see the module comment.
fn play_track(
    voices: &mut Voices,
    bank: &vale_assets::tables::sound::SoundBank,
    entry_id: u32,
) -> Option<(Entity, f32)> {
    let (path, volume) = voices.pick(bank, entry_id)?;
    let effective = volume * voices.gain(Channel::Music);
    let entity =
        voices.play_file(&path, volume, Channel::Music, Place::Flat, PlaybackSettings::DESPAWN)?;
    // The *effective* volume, because it is what a later cross-fade ramps down
    // from — see [`super::mixer::apply_gains`], which owns the other direction.
    Some((entity, effective))
}

/// **Let the channel go without cutting it** — the client's own release,
/// which fades to zero and marks the handle free-when-arrived. The state stops
/// pointing at the voice immediately, so the next claim can start on top of it
/// while it recedes; that overlap is the whole difference from a stop.
fn hand_over(state: &mut MusicState, voices: &mut Voices, seconds: f32) {
    if let Some((entity, _, volume)) = state.playing.take() {
        voices.fade_out(entity, volume, seconds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::host::Directory;

    /// **The login theme does not follow you into the world.** `SetGlueScreen`
    /// ends in `PlayGlueMusic`, which is `PlayMusic`, and nothing in
    /// `GlueParent.lua` ever says `StopMusic` — so the claim has to be dropped
    /// by the thing that knows the state making it has been thrown away.
    #[test]
    fn a_play_music_claim_dies_with_the_interface_that_made_it() {
        let mut state = MusicState::default();
        // The glue loads, and then asks for its theme — `PlayGlueMusic`.
        assert!(!state.interface_changed(Some(Directory::Glue)));
        state.override_path = Some(GLUE_MUSIC.to_string());

        // The same directory, frame after frame: nothing happens.
        assert!(!state.interface_changed(Some(Directory::Glue)));
        assert_eq!(state.override_path.as_deref(), Some(GLUE_MUSIC));

        // The state is torn down and rebuilt — one frame with nothing loaded,
        // which is where the claim goes.
        assert!(state.interface_changed(None), "the theme is dropped");
        assert_eq!(state.override_path, None);
        assert!(!state.interface_changed(Some(Directory::Frame)));
    }

    /// …and the answer is about a *claim* rather than a change: swapping
    /// directories with nothing playing has nothing to report, so a logout does
    /// not log a line about music nobody asked for.
    #[test]
    fn a_directory_change_with_no_claim_is_silent() {
        let mut state = MusicState::default();
        assert!(!state.interface_changed(Some(Directory::Frame)));
        assert!(!state.interface_changed(Some(Directory::Glue)));
        assert_eq!(state.override_path, None);
    }
}
