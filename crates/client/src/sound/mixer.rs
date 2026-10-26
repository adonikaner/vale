//! The single path every sound is played through.
//!
//! [`Voices`] is the system parameter every other module in this directory
//! plays through. It resolves a `SoundEntries` id to one of its files (the
//! weighted pick is [`vale_assets::tables::sound::SoundEntry::pick`], fed by
//! this module's roll), reads the file out of the archives once, caches the
//! decoded handle, and spawns a Bevy audio entity: spatial at a position, or
//! flat for the interface and the two channels.
//!
//! ## What a distance means here
//!
//! A `SoundEntries` row carries two distances: inside `min_distance` the sound
//! is at full volume, and beyond `cutoff_distance` it does not play at all.
//! The cutoff is applied exactly: a voice past it is never spawned. This is
//! also the cost gate, because a sound beyond the cutoff has no entity, no
//! decode and no mixing. The curve between the two distances is rodio's
//! inverse model under [`SPATIAL_SCALE`]. It approximates the client's curve,
//! and `sound/mod.rs` states it as an approximation.

use vale_assets::tables::sound::{SoundBank, SoundEntry};
use bevy::audio::{
    AudioPlayer, AudioSource, DefaultSpatialScale, PlaybackSettings, SpatialListener,
    SpatialScale, Volume,
};
use bevy::ecs::system::SystemParam;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

/// World units per rodio unit. At 1/6, a sound is clearly audible at its
/// typical 8-yard `min_distance` and near-silent approaching the 45-yard
/// cutoff. The value was chosen to make those two ends sound right, not
/// measured from the client: the client's attenuation curve is defined by the
/// fixed min/cutoff pair, which rodio does not have.
const SPATIAL_SCALE: f32 = 1.0 / 6.0;

/// The three channels the player has a slider for. They are the three
/// channels the sound options panel has.
///
/// Each channel is a pair of CVars, an enable and a volume, and every channel
/// is also multiplied by `MasterVolume`. The names and the split are the ones
/// `SoundOptionsFrame.lua` uses; see [`Channel::gain`].
///
/// `MusicVolume` defaults to 0.4 and `AmbienceVolume` to 0.6, which are the
/// values the client registers.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Everything that is not one of the other two: a swing, a footfall, a
    /// button click, the game's own error chimes.
    Effects,
    /// The music channel, played by [`super::music`], one voice at a time.
    Music,
    /// The ambience bed under a place, played by [`super::ambience`].
    Ambience,
}

impl Channel {
    /// The factor this channel multiplies a voice by, or `0.0` when the
    /// channel is off.
    ///
    /// `MasterSoundEffects` gates the effects channel and the ambience channel,
    /// and not music. This follows the shipped panel:
    /// `SoundOptionsFrame_UpdateDependencies` greys out the ambience and
    /// error-speech boxes when it is unticked and leaves music alone, so a
    /// player can turn every sound effect off and keep the music.
    pub fn gain(self, cvars: &crate::settings::cvars::CVars) -> f32 {
        let master = cvars.number("MasterVolume");
        let (enabled, volume) = match self {
            Channel::Effects => (cvars.flag("MasterSoundEffects"), "SoundVolume"),
            Channel::Music => (cvars.flag("EnableMusic"), "MusicVolume"),
            Channel::Ambience => (
                cvars.flag("EnableAmbience") && cvars.flag("MasterSoundEffects"),
                "AmbienceVolume",
            ),
        };
        match enabled {
            true => master * cvars.number(volume),
            false => 0.0,
        }
    }
}

/// The volume of a voice before the channel and master gains are applied:
/// its entry's or its track's own figure.
///
/// Kept on the voice so [`apply_gains`] can recompute the sink volume when a
/// slider moves. The sink holds the product, and the product cannot be
/// divided back out once the gain that produced it has changed.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct BaseVolume(pub(crate) f32);

/// The decoded-file cache and the pick roll.
#[derive(Resource, Default)]
pub(crate) struct Mixer {
    /// Archive path -> decoded handle. An `AudioSource` is the file's bytes;
    /// rodio decodes per play, so one handle serves any number of voices.
    cache: HashMap<String, Handle<AudioSource>>,
    /// Feeds [`SoundEntry::pick`]. A cheap LCG rather than a real RNG: the
    /// pick only has to not repeat itself audibly, and determinism keeps the
    /// one consumer of randomness in this crate testable.
    roll: u32,
    /// Files that failed to read or decode, so each warns once.
    missing: bevy::platform::collections::HashSet<String>,
}

impl Mixer {
    fn next_roll(&mut self) -> u32 {
        self.roll = self.roll.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        self.roll >> 8
    }

    /// Drop what is kept for `path`, compared without regard to ASCII case, so
    /// the next play reads the file again and warns again if it does not read.
    fn forget(&mut self, path: &str) {
        self.cache.retain(|kept, _| !kept.eq_ignore_ascii_case(path));
        self.missing.retain(|kept| !kept.eq_ignore_ascii_case(path));
    }
}

/// Forget the files a [`crate::assets::PathsChanged`] names. A sound written
/// at a path already played would otherwise go on playing as it was.
fn forget_changed_sounds(mut changed: MessageReader<crate::assets::PathsChanged>, mut mixer: ResMut<Mixer>) {
    for change in changed.read() {
        if change.all {
            mixer.cache.clear();
            mixer.missing.clear();
            continue;
        }
        for path in &change.paths {
            mixer.forget(path);
        }
    }
}

/// Where a voice is: in the world at a position, or flat on the output. The
/// interface's clicks and both channels are flat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    At(Vec3),
    /// On a unit: parented to its entity, so the voice moves with it. Without
    /// the parent, a `/silly` told while walking stayed at the position where
    /// it started. The position is the unit's current one, used only for the
    /// audibility test.
    On(Entity, Vec3),
    Flat,
}

/// A voice moving toward a target volume, matching the 1.12.1 client's sound
/// fade.
///
/// A fade has a target volume and a rate rather than an end time
/// (`target / (seconds * 1000)` per millisecond). A fade under 0.1 s is not a
/// ramp: the volume is set at once. [`Self::release`] adds the client's
/// fade-out-and-free: the voice fades to zero and is freed when it gets there,
/// so a fading-out voice outlives the code that let go of it. Here the voice
/// is despawned on arrival, which gives the same lifetime in Bevy.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct Fade {
    target: f32,
    /// Volume per second, always positive; the direction is the target.
    rate: f32,
    /// Despawn on arrival, as the client frees a released sound when its fade
    /// ends.
    release: bool,
}

/// Below this duration, `Fade` sets the volume at once rather than ramping,
/// as the client does.
const CUT_BELOW_SECS: f32 = 0.1;

impl Fade {
    /// Fade to `target` over `seconds`, staying alive at the end.
    pub fn to(target: f32, current: f32, seconds: f32) -> Fade {
        Fade {
            target,
            rate: (target - current).abs() / seconds.max(f32::EPSILON),
            release: false,
        }
    }

    /// Fade to silence over `seconds` and then despawn the voice.
    pub fn release(current: f32, seconds: f32) -> Fade {
        Fade {
            target: 0.0,
            rate: current / seconds.max(f32::EPSILON),
            release: true,
        }
    }

    /// Whether `seconds` is long enough to be a ramp at all.
    pub(crate) fn is_a_ramp(seconds: f32) -> bool {
        seconds > CUT_BELOW_SECS
    }

    /// Advance one step of `dt` seconds. Returns the new volume and whether it
    /// has reached the target.
    fn step(&self, current: f32, dt: f32) -> (f32, bool) {
        let moved = if self.target > current {
            (current + self.rate * dt).min(self.target)
        } else {
            (current - self.rate * dt).max(self.target)
        };
        (moved, (moved - self.target).abs() < f32::EPSILON)
    }
}

/// The system parameter every play in this directory goes through, so the
/// cache, the roll, the cutoff gate and the missing-file warning exist in one
/// place.
#[derive(SystemParam)]
pub struct Voices<'w, 's> {
    commands: Commands<'w, 's>,
    mixer: ResMut<'w, Mixer>,
    audio: ResMut<'w, Assets<AudioSource>>,
    game: Res<'w, crate::assets::GameAssets>,
    listener: Query<'w, 's, &'static GlobalTransform, With<SpatialListener>>,
    /// The player's slider settings; see [`Channel::gain`]. Read here rather
    /// than at each call site so that the four modules that play sounds do not
    /// each have to know which channel they are on and what the master volume
    /// is.
    cvars: Res<'w, crate::settings::cvars::CVars>,
}

impl Voices<'_, '_> {
    /// Play a `SoundEntries` id. Returns the voice entity, or `None` for an
    /// id the bank does not have, a file that will not read, or a position
    /// past the entry's own cutoff.
    pub fn play(&mut self, bank: &SoundBank, entry_id: u32, place: Place) -> Option<Entity> {
        // A channel the player has turned off is not started, rather than
        // started silent. An effect is a one-shot, so a later re-enable has
        // nothing to catch up to, and skipping it is also the cost gate: no
        // entity, no decode, no mixing. The music and ambience channels do the
        // opposite, because a loop turned back on has to be running already.
        if self.gain(Channel::Effects) <= 0.0 {
            return None;
        }
        let entry = bank.entry(entry_id)?;
        if let Place::At(position) | Place::On(_, position) = place {
            if !self.audible(entry, position) {
                return None;
            }
        }
        let roll = self.mixer.next_roll();
        let path = entry.pick(roll)?;
        self.spawn(&path, entry.volume, Channel::Effects, place, PlaybackSettings::DESPAWN)
    }

    /// The factor a channel multiplies by now, for the two callers that need
    /// the number rather than only playing a sound; see [`Channel::gain`].
    pub fn gain(&self, channel: Channel) -> f32 {
        channel.gain(&self.cvars)
    }

    /// Resolve an entry to one of its files without playing it, for the two
    /// channels, which carry their own volume and looping. The roll is spent
    /// here so the pick stays weighted.
    pub fn pick(&mut self, bank: &SoundBank, entry_id: u32) -> Option<(String, f32)> {
        let entry = bank.entry(entry_id)?;
        let roll = self.mixer.next_roll();
        Some((entry.pick(roll)?, entry.volume))
    }

    /// Play an archive path directly. This is `PlaySoundFile`'s contract, and
    /// the music channel's, which carries its own settings.
    pub fn play_file(
        &mut self,
        path: &str,
        volume: f32,
        channel: Channel,
        place: Place,
        settings: PlaybackSettings,
    ) -> Option<Entity> {
        self.spawn(path, volume, channel, place, settings)
    }

    /// Play an archive path faded up from silence. The client does this with
    /// an ambience bed: it sets the volume to 0 and then fades to the bed's
    /// volume over 5 s.
    ///
    /// The voice is spawned at zero rather than at its volume, so the first
    /// frame is silent rather than a click. The client uses the same order,
    /// and it is the order that makes the incoming bed of a cross-fade start
    /// from silence.
    pub(crate) fn play_file_fading_in(
        &mut self,
        path: &str,
        volume: f32,
        channel: Channel,
        place: Place,
        settings: PlaybackSettings,
        seconds: f32,
    ) -> Option<Entity> {
        if !Fade::is_a_ramp(seconds) {
            return self.spawn(path, volume, channel, place, settings);
        }
        let entity = self.spawn(path, 0.0, channel, place, settings)?;
        // The ramp is in effective volume, so its target is the base volume
        // times the channel gain: a bed faded up while the ambience slider is
        // at half arrives at half volume, not at full.
        let target = volume * self.gain(channel);
        self.commands
            .entity(entity)
            .insert(Fade::to(target, 0.0, seconds));
        Some(entity)
    }

    /// Fade a voice out and then despawn it, instead of cutting it off. For a
    /// duration at or under the cut threshold this despawns the voice at once,
    /// and for a voice that has already been despawned it does nothing.
    pub fn fade_out(&mut self, entity: Entity, volume: f32, seconds: f32) {
        let Ok(mut voice) = self.commands.get_entity(entity) else {
            return;
        };
        if !Fade::is_a_ramp(seconds) {
            voice.try_despawn();
            return;
        }
        voice.insert(Fade::release(volume, seconds));
    }

    /// Whether a position is inside the entry's own cutoff distance from the
    /// listener. With no listener yet (the first frames of a session) every
    /// position counts as audible. This errs toward sound, since the camera
    /// arrives within a frame.
    fn audible(&self, entry: &SoundEntry, position: Vec3) -> bool {
        let Ok(listener) = self.listener.single() else {
            return true;
        };
        let cutoff = if entry.cutoff_distance > 0.0 {
            entry.cutoff_distance
        } else {
            f32::MAX
        };
        listener.translation().distance(position) <= cutoff
    }

    fn spawn(
        &mut self,
        path: &str,
        volume: f32,
        channel: Channel,
        place: Place,
        settings: PlaybackSettings,
    ) -> Option<Entity> {
        let handle = match self.mixer.cache.get(path) {
            Some(handle) => handle.clone(),
            None => {
                let bytes = self
                    .game
                    .with_archive(|assets| assets.read(path).map_err(|e| e.to_string()))
                    .ok();
                let Some(bytes) = bytes else {
                    if self.mixer.missing.insert(path.to_string()) {
                        warn!("sound file not in the archives: {path}");
                    }
                    return None;
                };
                // Probe the bytes before Bevy sees them. `bevy_audio` unwraps
                // its decoder inside `play_queued_audio_system`, so a file
                // rodio cannot decode panics the whole app, and the game's
                // 20-year-old archives hold such files (`vale sound` lists
                // them). The probe is the same decoder at the same version,
                // so it accepts exactly the files the player accepts.
                if rodio::Decoder::new(std::io::Cursor::new(bytes.clone())).is_err() {
                    if self.mixer.missing.insert(path.to_string()) {
                        warn!("sound file will not decode, not playing it: {path}");
                    }
                    return None;
                }
                let handle = self.audio.add(AudioSource { bytes: bytes.into() });
                self.mixer.cache.insert(path.to_string(), handle.clone());
                handle
            }
        };
        self.voice(handle, volume, channel, place, settings)
    }

    /// Play a sound's bytes that are at no archive path: a file a host has
    /// converted and not yet written. The bytes are probed as a file read
    /// from the archives is, and are not cached.
    pub fn play_bytes(
        &mut self,
        bytes: Vec<u8>,
        volume: f32,
        channel: Channel,
        settings: PlaybackSettings,
    ) -> Option<Entity> {
        if rodio::Decoder::new(std::io::Cursor::new(bytes.clone())).is_err() {
            warn!("the sound's bytes do not decode as WAV or MP3; not playing them");
            return None;
        }
        let handle = self.audio.add(AudioSource { bytes: bytes.into() });
        self.voice(handle, volume, channel, Place::Flat, settings)
    }

    /// A voice for a decoded handle, at `volume` through `channel`.
    fn voice(
        &mut self,
        handle: Handle<AudioSource>,
        volume: f32,
        channel: Channel,
        place: Place,
        settings: PlaybackSettings,
    ) -> Option<Entity> {
        // The sink holds the product of base volume and channel gain. The
        // base and the channel are stored on the entity so [`apply_gains`]
        // can recompute the product when a slider moves.
        let base = volume;
        let volume = base * channel.gain(&self.cvars);
        let settings = settings.with_volume(Volume::Linear(volume));
        let mut voice = self
            .commands
            .spawn((AudioPlayer(handle), channel, BaseVolume(base)));
        match place {
            Place::At(position) => {
                voice.insert((
                    settings.with_spatial(true),
                    Transform::from_translation(position),
                ));
            }
            Place::On(unit, _) => {
                // At the unit's origin, positioned by transform propagation,
                // and despawned with the unit because it is a child entity.
                voice.insert((settings.with_spatial(true), Transform::default(), ChildOf(unit)));
            }
            Place::Flat => {
                voice.insert(settings);
            }
        }
        Some(voice.id())
    }
}

pub(crate) struct MixerPlugin;

impl Plugin for MixerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mixer>()
            .insert_resource(DefaultSpatialScale(SpatialScale::new(SPATIAL_SCALE)))
            // `apply_gains` and `attach_listener` run after `GameSet`, which is
            // where a `SetCVar` becomes a change to `CVars`. Without this
            // ordering, a slider moved in one frame is heard in the next, and
            // Bevy reports nothing about the one-frame lag.
            .add_systems(
                Update,
                (
                    run_fades,
                    (attach_listener, apply_gains).after(crate::interface::GameSet),
                ),
            )
            .add_message::<crate::assets::PathsChanged>()
            .add_systems(Update, forget_changed_sounds.in_set(crate::assets::ForgetChanged));
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report);
    }
}

/// Put the `SpatialListener` on the player's character or on the world
/// camera. The component is inserted rather than spawned with the camera so
/// this directory does not edit `world::camera`: a module about one subject
/// does not edit files about another.
///
/// Only the world camera, not every `Camera3d`. `render::portraits` spawns up
/// to nine more, one per unit-frame face, and a filter on `Camera3d` alone gave
/// each of them a listener. With ten listeners, `bevy_audio` warned on every
/// spatial play and used whichever it found first, so the session could be
/// heard from a portrait camera at the origin rather than from the player.
fn attach_listener(
    mut commands: Commands,
    cvars: Res<crate::settings::cvars::CVars>,
    // `WorldCamera` alone: the marker is only on the one world `Camera3d`, so
    // no further filter is needed.
    camera: Query<Entity, With<crate::world::camera::WorldCamera>>,
    player: Query<Entity, With<crate::world::entity::LocalPlayer>>,
    listening: Query<Entity, With<SpatialListener>>,
) {
    // `SoundListenerAtCharacter` is the sound panel's "Sound Effects At
    // Character" box, and it is on by default. The listener therefore goes on
    // the character, and on the camera only when the box is unticked.
    //
    // The difference is largest at maximum zoom, where the camera is about
    // fifteen yards behind the shoulder: with the listener on the camera, a
    // fight at the character's feet is mixed as if it were fifteen yards away.
    // The listener falls back to the camera whenever there is no character:
    // on the glue screens, and in the first frames of a session.
    let wanted = match cvars.flag("SoundListenerAtCharacter") {
        true => player.iter().next().or_else(|| camera.iter().next()),
        false => camera.iter().next(),
    };
    let Some(wanted) = wanted else { return };
    if listening.iter().any(|entity| entity == wanted) {
        return;
    }
    // Only one listener: the entity that held it loses it in the same frame.
    for entity in &listening {
        commands.entity(entity).remove::<SpatialListener>();
    }
    commands.entity(wanted).insert(SpatialListener::default());
}

/// Advance every fade, and despawn the voices whose fade is a release.
///
/// The sink holds the volume, so a fade writes to `AudioSink`. A fade cannot
/// be a setting on the player, because Bevy applies `PlaybackSettings` once
/// when the voice starts.
///
/// A voice whose sink has not arrived yet is skipped rather than despawned:
/// `bevy_audio` inserts `AudioSink` a frame after `AudioPlayer`, so the first
/// frame of every fade has nothing to write to.
fn run_fades(
    mut commands: Commands,
    time: Res<Time>,
    mut fading: Query<(Entity, &Fade, &mut AudioSink)>,
) {
    use bevy::audio::AudioSinkPlayback;
    let dt = time.delta_secs();
    for (entity, fade, mut sink) in &mut fading {
        let current = match sink.volume() {
            Volume::Linear(v) => v,
            other => other.to_linear(),
        };
        let (volume, arrived) = fade.step(current, dt);
        sink.set_volume(Volume::Linear(volume));
        if !arrived {
            continue;
        }
        match fade.release {
            true => commands.entity(entity).try_despawn(),
            false => {
                commands.entity(entity).remove::<Fade>();
            }
        }
    }
}


/// Re-apply the channel gains when a slider moves, so a change on the sound
/// options panel applies to sounds that are already playing.
///
/// Runs only when [`crate::settings::cvars::CVars`] changed. Walking every
/// live voice sixty times a second to write the same number would be the
/// second-largest per-frame cost in this directory, and settings change
/// perhaps twice a session.
///
/// A fading voice is skipped. [`Fade`] ramps in effective volume toward a
/// target computed when it started, and two writers on one sink would
/// conflict. The cost is that a fade begun in the same frame as a slider move
/// ramps at the old rate; [`Fade::step`] clamps at the target either way, so
/// the fade ends slightly early or late and never at a wrong volume.
fn apply_gains(
    cvars: Res<crate::settings::cvars::CVars>,
    mut voices: Query<(&Channel, &BaseVolume, &mut AudioSink), Without<Fade>>,
) {
    use bevy::audio::AudioSinkPlayback;
    if !cvars.is_changed() {
        return;
    }
    for (channel, base, mut sink) in &mut voices {
        sink.set_volume(Volume::Linear(base.0 * channel.gain(&cvars)));
    }
}

/// The HUD line: how many voices are alive, and how many files are cached.
#[cfg(feature = "diagnostics")]
fn report(
    mixer: Res<Mixer>,
    voices: Query<(), With<AudioPlayer>>,
    mut hud: ResMut<crate::ui::report::HudReport>,
) {
    hud.set(
        crate::ui::report::Section::Scene,
        // 45 rather than 40: `world::desync` uses slot 40, and two passes on
        // one slot print two lines in an arbitrary order.
        crate::ui::report::Slot(45),
        "sound",
        format!(
            "sound: {} voices, {} files cached, {} missing",
            voices.iter().count(),
            mixer.cache.len(),
            mixer.missing.len()
        ),
    );
}

#[cfg(test)]
mod tests {
    use crate::settings::cvars::CVars;

    /// The three channels and the factor each multiplies by, from the values
    /// the sound panel writes and the mixer reads back.
    #[test]
    fn a_channel_is_its_own_volume_under_the_master() {
        let cvars = CVars::default();
        // The shipped defaults: master 1.0, effects 1.0, music 0.4, ambience 0.6.
        assert_eq!(Channel::Effects.gain(&cvars), 1.0);
        assert_eq!(Channel::Music.gain(&cvars), 0.4);
        assert!((Channel::Ambience.gain(&cvars) - 0.6).abs() < 1e-6);
    }

    /// An unticked box silences its own channel and the channels that depend
    /// on it, following the dependency the shipped panel's
    /// `SoundOptionsFrame_UpdateDependencies` sets up: the master effects
    /// switch silences the ambience with the effects and leaves the music
    /// playing, and `EnableMusic` silences the music alone.
    #[test]
    fn turning_the_effects_off_keeps_the_music() {
        let mut cvars = CVars::default();
        cvars.set_for_test("MasterSoundEffects", "0");
        assert_eq!(Channel::Effects.gain(&cvars), 0.0);
        assert_eq!(Channel::Ambience.gain(&cvars), 0.0, "the panel greys it out with it");
        assert_eq!(Channel::Music.gain(&cvars), 0.4, "…and leaves this alone");

        cvars.set_for_test("MasterSoundEffects", "1");
        cvars.set_for_test("EnableMusic", "0");
        assert_eq!(Channel::Music.gain(&cvars), 0.0);
        assert_eq!(Channel::Effects.gain(&cvars), 1.0);
    }

    /// `MasterVolume` multiplies every channel. It is the one slider that is
    /// not a channel of its own.
    #[test]
    fn the_master_slider_scales_all_three() {
        let mut cvars = CVars::default();
        cvars.set_for_test("MasterVolume", "0.5");
        assert_eq!(Channel::Effects.gain(&cvars), 0.5);
        assert_eq!(Channel::Music.gain(&cvars), 0.2);
        cvars.set_for_test("MasterVolume", "0");
        assert_eq!(Channel::Effects.gain(&cvars), 0.0);
        assert_eq!(Channel::Music.gain(&cvars), 0.0);
    }

    use super::*;

    /// A fade reaches its target when its duration has elapsed and not a frame
    /// before, in either direction, and does not overshoot on a long frame.
    /// Stepped at 60 Hz because that is the rate the ramp runs at.
    #[test]
    fn a_five_second_fade_takes_five_seconds_and_stops_there() {
        let (mut up, mut down) = (0.0f32, 0.8f32);
        let rise = Fade::to(0.8, 0.0, 5.0);
        let fall = Fade::release(0.8, 5.0);
        for _ in 0..(60 * 5) {
            (up, _) = rise.step(up, 1.0 / 60.0);
            (down, _) = fall.step(down, 1.0 / 60.0);
        }
        assert!((up - 0.8).abs() < 1e-3, "risen to volume: {up}");
        assert!(down.abs() < 1e-3, "fallen to silence: {down}");

        // Halfway, and a single frame longer than the whole fade.
        let (half, arrived) = rise.step(0.0, 2.5);
        assert!((half - 0.4).abs() < 1e-6, "{half}");
        assert!(!arrived);
        let (over, arrived) = rise.step(0.0, 50.0);
        assert_eq!(over, 0.8, "clamped at the target rather than sailing past");
        assert!(arrived);
        let (under, arrived) = fall.step(0.8, 50.0);
        assert_eq!(under, 0.0);
        assert!(arrived);
    }

    /// A fade of a tenth of a second or less is a cut, as in the client. This
    /// also means `Fade` is never asked to divide by something near zero.
    #[test]
    fn a_fade_shorter_than_a_tenth_of_a_second_is_not_a_ramp() {
        assert!(!Fade::is_a_ramp(0.0));
        assert!(!Fade::is_a_ramp(0.1));
        assert!(Fade::is_a_ramp(0.11));
        assert!(Fade::is_a_ramp(5.0));
    }
}
