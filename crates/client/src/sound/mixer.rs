//! **The one door a sound goes out through.**
//!
//! [`Voices`] is the system parameter every other module in this directory
//! plays through: it resolves a `SoundEntries` id to one of its files (the
//! weighted pick is [`vale_assets::tables::sound::SoundEntry::pick`]'s, fed by this
//! module's roll), reads the file out of the archives once and caches the
//! decoded handle, and spawns a Bevy audio entity — spatial at a position, or
//! flat for the interface and the two channels.
//!
//! ## What a distance means here
//!
//! A `SoundEntries` row carries two distances: inside `min_distance` the sound
//! is at full volume, beyond `cutoff_distance` it does not play at all. The
//! **cutoff is honoured exactly** — a voice past it is never spawned, which is
//! also the cost gate: no entity, no decode, no mixing for the fight on the
//! other side of the hill. The curve *between* the two is rodio's inverse
//! model under [`SPATIAL_SCALE`], which is an approximation of the client's
//! and is stated as one in `sound/mod.rs`.

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
/// cutoff — chosen to *land the ends right*, not measured off the client,
/// whose attenuation curve is the fixed min/cutoff pair rodio does not have.
const SPATIAL_SCALE: f32 = 1.0 / 6.0;

/// **The three channels the player has a slider for**, which are the three the
/// sound options panel has and no more.
///
/// Each is a pair of CVars — an enable and a volume — and everything is
/// multiplied by `MasterVolume` on top. The names and the split are
/// `SoundOptionsFrame.lua`'s own; see [`Channel::gain`].
///
/// This replaced three `const f32`s that were, in this file's own words,
/// "placeholders wearing the CVars' names". Two of the three turned out to be
/// **exactly** the values the client registers — `MusicVolume` really is 0.4
/// and `AmbienceVolume` really is 0.6 — which is a pleasant way to find out that the guess
/// was right and a better way to stop guessing.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Everything that is not one of the other two: a swing, a footfall, a
    /// button, the game's own error chimes.
    Effects,
    /// The music channel — one voice at a time, [`super::music`]'s.
    Music,
    /// …and the bed under a place, [`super::ambience`]'s.
    Ambience,
}

impl Channel {
    /// **What this channel multiplies a voice by**, or `0.0` when it is off.
    ///
    /// `MasterSoundEffects` gates the effects channel and *not* the other two,
    /// which is the shipped panel's own arrangement: `SoundOptionsFrame_UpdateDependencies`
    /// greys out the ambience and error-speech boxes when it is unticked and
    /// leaves music alone, so a player can turn every sound effect off and keep
    /// the score.
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

/// **What a voice would be at full volume** — its entry's or its track's own
/// figure, before the channel and the master are applied.
///
/// Kept on the voice so [`apply_gains`] can recompute it when a slider moves:
/// the sink holds the product, and the product cannot be un-multiplied once
/// the gain that made it has changed.
#[derive(Component, Debug, Clone, Copy)]
pub struct BaseVolume(pub f32);

/// The decoded-file cache and the pick roll.
#[derive(Resource, Default)]
pub struct Mixer {
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
}

/// Where a voice is: in the world at a position, or flat on the output —
/// which is what the interface's clicks and both channels are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    At(Vec3),
    /// **On a unit**: parented to its entity, so the voice moves with it —
    /// a `/silly` told while walking used to stay where the joke began.
    /// The position is where it is *now*, for the audibility test alone.
    On(Entity, Vec3),
    Flat,
}

/// **A voice on its way somewhere** — the client's own sound fade.
///
/// The reference's shape: a target volume, a *rate* rather than an end time
/// (`target / (seconds * 1000)` per millisecond), and a **cut for anything
/// under 0.1 s** so a "fade" that short is not a ramp at all. The one thing
/// added here is [`Self::release`], which is the client's fade-out-and-free —
/// it fades to zero and then marks the handle "free me when you get there",
/// so a fading-out
/// voice outlives the code that let go of it. This client despawns instead,
/// which is the same lifetime through Bevy's own door.
#[derive(Component, Debug, Clone, Copy)]
pub struct Fade {
    target: f32,
    /// Volume per second, always positive; the direction is the target.
    rate: f32,
    /// Despawn on arrival — the reference's free-on-arrival flag.
    release: bool,
}

/// Below this, `Fade` is a set rather than a ramp — the client's own guard.
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

    /// …and the other one: fade to silence over `seconds` and then let go.
    pub fn release(current: f32, seconds: f32) -> Fade {
        Fade {
            target: 0.0,
            rate: current / seconds.max(f32::EPSILON),
            release: true,
        }
    }

    /// Whether `seconds` is long enough to be a ramp at all.
    pub fn is_a_ramp(seconds: f32) -> bool {
        seconds > CUT_BELOW_SECS
    }

    /// One step. Answers the new volume and whether it has arrived.
    fn step(&self, current: f32, dt: f32) -> (f32, bool) {
        let moved = if self.target > current {
            (current + self.rate * dt).min(self.target)
        } else {
            (current - self.rate * dt).max(self.target)
        };
        (moved, (moved - self.target).abs() < f32::EPSILON)
    }
}

/// The door. Every play in the directory goes through one of these three
/// methods, so the cache, the roll, the cutoff gate and the missing-file
/// warning exist exactly once.
#[derive(SystemParam)]
pub struct Voices<'w, 's> {
    commands: Commands<'w, 's>,
    mixer: ResMut<'w, Mixer>,
    audio: ResMut<'w, Assets<AudioSource>>,
    game: Res<'w, crate::assets::GameAssets>,
    listener: Query<'w, 's, &'static GlobalTransform, With<SpatialListener>>,
    /// **What the player has the sliders at** — see [`Channel::gain`]. Read
    /// here rather than at each call site so that the four modules that play
    /// something do not each have to remember which channel they are on and
    /// what the master is.
    cvars: Res<'w, crate::settings::cvars::CVars>,
}

impl Voices<'_, '_> {
    /// Play a `SoundEntries` id. Returns the voice entity, or `None` for an
    /// id the bank does not have, a file that will not read, or a position
    /// past the entry's own cutoff.
    pub fn play(&mut self, bank: &SoundBank, entry_id: u32, place: Place) -> Option<Entity> {
        // **A channel the player has turned off is not started**, rather than
        // started silent: an effect is a one-shot, so there is nothing for a
        // later re-enable to catch up to, and this is also the cost gate — no
        // entity, no decode, no mixing. The two *channels* below do the
        // opposite, because a loop turned back on has to already be running.
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

    /// **What a channel is multiplied by right now**, for the two callers that
    /// have to know rather than merely play — see [`Channel::gain`].
    pub fn gain(&self, channel: Channel) -> f32 {
        channel.gain(&self.cvars)
    }

    /// Resolve an entry to one of its files **without playing it** — for the
    /// two channels, which carry their own volume and looping. The roll is
    /// spent here so the pick stays weighted.
    pub fn pick(&mut self, bank: &SoundBank, entry_id: u32) -> Option<(String, f32)> {
        let entry = bank.entry(entry_id)?;
        let roll = self.mixer.next_roll();
        Some((entry.pick(roll)?, entry.volume))
    }

    /// Play an archive path directly — `PlaySoundFile`'s contract, and the
    /// music channel's, which carries its own settings.
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

    /// …and the same thing **faded up from silence**, which is what the client
    /// does with an ambience bed (set 0, then `Fade(5.0, volume)`).
    ///
    /// The voice is spawned at zero rather than at its volume, so the first
    /// frame is silent rather than a click — which is the ordering the
    /// reference uses and the only one that makes a cross-fade a cross-fade.
    pub fn play_file_fading_in(
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
        // **The ramp is in effective volume**, so its target is the base
        // through the channel — a bed faded up while the ambience slider is
        // half way arrives at half way and not at full.
        let target = volume * self.gain(channel);
        self.commands
            .entity(entity)
            .insert(Fade::to(target, 0.0, seconds));
        Some(entity)
    }

    /// **Let a voice go without cutting it off**. Under the cut
    /// threshold, or for a voice that has already gone, this is the despawn it
    /// stands in for.
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

    /// Is a position inside the entry's own cutoff of the listener? No
    /// listener yet (the first frames of a session) counts as audible —
    /// erring toward sound, since the camera arrives within a frame.
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
                // **Probe before Bevy sees the bytes.** `bevy_audio` unwraps
                // its decoder inside `play_queued_audio_system`, so a file
                // rodio cannot eat panics the whole app — and a 20-year-old
                // archive holds such files (`vale sound` lists them). The
                // probe is the same decoder at the same version, so its
                // verdict and the player's cannot disagree.
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
        // **The sink holds the product**; the base and the channel ride along
        // so [`apply_gains`] can recompute it when a slider moves.
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
                // At the unit's own origin, composed by propagation — and
                // despawned with it, which is what a child is for.
                voice.insert((settings.with_spatial(true), Transform::default(), ChildOf(unit)));
            }
            Place::Flat => {
                voice.insert(settings);
            }
        }
        Some(voice.id())
    }
}

pub struct MixerPlugin;

impl Plugin for MixerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mixer>()
            .insert_resource(DefaultSpatialScale(SpatialScale::new(SPATIAL_SCALE)))
            // **`apply_gains` and `attach_listener` after `GameSet`**, which is
            // where a `SetCVar` becomes a change to `CVars`: without the
            // ordering a slider moved this frame is heard next frame, which is
            // a lag nobody would ever find and Bevy would never mention.
            .add_systems(
                Update,
                (
                    run_fades,
                    (attach_listener, apply_gains).after(crate::interface::GameSet),
                ),
            );
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report);
    }
}

/// The ears ride the camera. Inserted rather than spawned with it so this
/// directory does not edit `world::camera` — the rule about a subject not
/// editing files about every other subject, applied in reverse.
///
/// **The world camera, not every `Camera3d`.** `render::portraits` spawns up to
/// nine more, one per unit-frame face, and `Camera3d` alone gave each of them a
/// listener — ten sets of ears, `bevy_audio` warning about it on every spatial
/// play and picking whichever it found first, which is a session heard from a
/// portrait studio at the origin rather than from the player.
fn attach_listener(
    mut commands: Commands,
    cvars: Res<crate::settings::cvars::CVars>,
    // `WorldCamera` alone: the marker only ever sits on the one `Camera3d`, so
    // the third filter the first draft carried said nothing.
    camera: Query<Entity, With<crate::world::camera::WorldCamera>>,
    player: Query<Entity, With<crate::world::session::LocalPlayer>>,
    listening: Query<Entity, With<SpatialListener>>,
) {
    // **`SoundListenerAtCharacter` is a real setting and its default is on**,
    // which is the sound panel's "Sound Effects At Character" — so the ears
    // belong on the character and only ride the camera when the box is
    // unticked. This client had them on the camera unconditionally, which is
    // the *unset* behaviour of a CVar that ships set.
    //
    // It matters at the two ends of the zoom: at maximum distance the camera is
    // some fifteen yards behind the shoulder, so a fight at the character's
    // feet is mixed as if it were across the road. Falls back to the camera
    // whenever there is no character — the glue screens, and the first frames
    // of a session.
    let wanted = match cvars.flag("SoundListenerAtCharacter") {
        true => player.iter().next().or_else(|| camera.iter().next()),
        false => camera.iter().next(),
    };
    let Some(wanted) = wanted else { return };
    if listening.iter().any(|entity| entity == wanted) {
        return;
    }
    // One listener: whoever was holding it gives it up in the same frame.
    for entity in &listening {
        commands.entity(entity).remove::<SpatialListener>();
    }
    commands.entity(wanted).insert(SpatialListener::default());
}

/// **Run every ramp**, and let go of the ones that asked to be let go of.
///
/// The sink is what holds the volume, so this is the one system in the
/// directory that touches `AudioSink` — a fade cannot be a setting on the
/// player, because Bevy applies `PlaybackSettings` once when the voice starts.
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


/// **Re-apply the channel gains when a slider moves**, which is what makes the
/// sound options panel do something while a sound is playing.
///
/// Only when [`crate::settings::cvars::CVars`] actually changed — a walk of every
/// live voice sixty times a second to write the same number would be the
/// second-largest per-frame cost in this directory, and settings move perhaps
/// twice a session.
///
/// **A fading voice is left alone.** [`Fade`] ramps in effective volume toward
/// a target computed when it started, and two writers on one sink would fight.
/// The stated cost is that a fade begun in the same breath as a slider move
/// ramps at the old rate; [`Fade::step`] clamps at the target either way, so
/// what it costs is a fade a fraction long or short and never a wrong volume.
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
        // 45 rather than 40: `world::desync` had 40 already, and two
        // passes on one slot is two lines in an arbitrary order.
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

    /// **The three channels, and what each is multiplied by** — the values the
    /// sound panel writes and the mixer reads back.
    #[test]
    fn a_channel_is_its_own_volume_under_the_master() {
        let cvars = CVars::default();
        // The shipped defaults: master 1.0, effects 1.0, music 0.4, ambience 0.6.
        assert_eq!(Channel::Effects.gain(&cvars), 1.0);
        assert_eq!(Channel::Music.gain(&cvars), 0.4);
        assert!((Channel::Ambience.gain(&cvars) - 0.6).abs() < 1e-6);
    }

    /// …and **an unticked box is silence on that channel alone**, on the
    /// dependency the shipped panel's own `SoundOptionsFrame_UpdateDependencies`
    /// draws: the master effects switch takes the ambience with it and leaves
    /// the music playing.
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

    /// **The master multiplies every channel**, which is the one slider that
    /// is not a channel of its own.
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

    /// **A fade arrives when its clock says so and not a frame before**, from
    /// either direction, and it does not overshoot on a long frame — which is
    /// the whole of what a rate-based ramp has to get right. Stepped at 60 Hz
    /// because that is what the ramp runs at.
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

    /// **Under a tenth of a second is a cut**, which is the
    /// reference's own guard and the reason `Fade` is never asked to divide by
    /// something near zero.
    #[test]
    fn a_fade_shorter_than_a_tenth_of_a_second_is_not_a_ramp() {
        assert!(!Fade::is_a_ramp(0.0));
        assert!(!Fade::is_a_ramp(0.1));
        assert!(Fade::is_a_ramp(0.11));
        assert!(Fade::is_a_ramp(5.0));
    }
}
