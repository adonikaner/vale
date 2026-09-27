//! **What the world sounds like** — the first subsystem in this client that
//! reaches the speakers.
//!
//! ```text
//! mixer.rs      the one door a sound goes out through: decode, cache, place
//! music.rs      the music channel: glue theme, zone tracks, intros, PlayMusic
//! ambience.rs   the loop under everything, per place, cross-faded on change
//! footsteps.rs  the stride's two footfalls, on what the ground is made of
//! cues.rs       …and the sounds a model's own animation asks for: the laugh at the laugh
//! combat.rs     the swing, the impact, the wound and the death
//! spells.rs     a cast's five sounds, off the same counters the pose reads
//! items.rs      …and what an item sounds like changing hands at a vendor
//! player.rs     what happens to *you*: the level-up chime the UI never plays
//! messages.rs   …and what the game's own 343 messages sound like, by name
//! interface.rs  the four Lua verbs, drained — PlaySound is a name lookup
//! pushed.rs     …and the three the *server* asks for outright, which are the
//!               only sounds in here that nothing else on the wire implies
//! npc.rs        …and what an NPC says when a window opens on it and when it
//!               closes: NPCSounds' hello and goodbye, off the "npc" token
//! ```
//!
//! ## What the player has the sliders at
//!
//! Nothing in here decides a volume any more. The three channels
//! ([`mixer::Channel`]) are the three the game's own sound options panel has,
//! and each reads its pair of CVars out of [`crate::settings::cvars::CVars`] —
//! which is the *interface's* store, written by the panel itself through
//! `SetCVar`. So "turn the music down" is `SoundOptionsFrameSlider3` moving,
//! and this directory finds out the same way an addon would.
//!
//! Two more of that panel's eleven settings land here rather than on a volume:
//! **`SoundZoneMusicNoDelay`** ("Loop Music") drops the between-track silence
//! in [`music`], and **`SoundListenerAtCharacter`** puts the ears on the
//! character rather than on the camera in [`mixer::attach_listener`]. The other
//! two the panel writes and nothing here reads are `EnableErrorSpeech` (there
//! is no error speech yet) and `EmoteSounds`.
//!
//! **Every decision here is a table lookup and the tables live in
//! [`vale_assets::tables::sound`].** Which entry a footstep resolves to, which file
//! of an entry's ten plays, what a zone's playlist is — none of that needs a
//! speaker, so all of it is decided in `assets` where `vale sound` can
//! check the same copy this directory plays. What stays here is the client's
//! half: *when* to ask (a counter moved, a stride wrapped, an area changed)
//! and how a buffer reaches Bevy's audio output.
//!
//! Nothing in here depends on [`crate::render`] except two types that are
//! world state wearing render clothing — [`crate::render::sky::WorldClock`]
//! (the hour, for day/night) and [`crate::render::axes`] (the coordinate
//! frame) — which is what keeps this directory the crate split its own module
//! doc promised while it was empty.
//!
//! ## Stated approximations
//!
//! Each of these is a deliberate simplification, not an oversight:
//!
//! * **Footfall timing is cadence, not the model's own events.** An M2 carries
//!   animation events and the real client fires footsteps off them; this
//!   client fires at the stride's half-cycle points, which is right on average
//!   and wrong per-frame. The refinement is parsing the M2 event track.
//! * **Attenuation is rodio's, not the entry's two distances.** A
//!   `SoundEntries` row states full-volume-inside and nothing-beyond; the
//!   mixer honours the cutoff (a sound past it is not started) and leaves the
//!   curve between to the spatial scale in [`mixer`].
//! * **The day/night switch hours are an interpretation** — see
//!   [`vale_assets::tables::sound::is_night`].
//! * **A weapon impact plays material slot 0 of ten** — which material a
//!   victim is is not modelled.
//! * **The glue theme's handover into the world is this client's own.** Every
//!   other music handover is the client's own duration; that one has no
//!   reference behaviour at all, because the real client tears the process
//!   down between the login screen and the game. It takes the ordinary
//!   four-second handover rather than a third rule invented for it.

pub mod ambience;
pub mod combat;
pub mod cues;
pub mod footsteps;
pub mod interface;
pub mod pushed;
pub mod items;
pub mod messages;
pub mod mixer;
pub mod music;
pub mod npc;
pub mod player;
pub mod spells;

use bevy::prelude::*;

/// Every sound system, in one set: they all read what `GameSet` and the session
/// wrote this frame, so they run after both.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SoundSet;

/// The directory's passes. One line in `lib.rs`, like its four siblings.
pub struct SoundPlugins;

impl Plugin for SoundPlugins {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            mixer::MixerPlugin,
            music::MusicPlugin,
            ambience::AmbiencePlugin,
            footsteps::FootstepsPlugin,
            cues::CuesPlugin,
            combat::CombatPlugin,
            spells::SpellsPlugin,
            player::PlayerPlugin,
            messages::MessagesPlugin,
            interface::InterfacePlugin,
            pushed::PushedPlugin,
            items::ItemSoundPlugin,
            npc::NpcSoundPlugin,
        ));
        // After the session has placed this frame's entities and after `GameSet`
        // has tracked the area — a footstep at last frame's position is a
        // footstep behind the character, and music for last frame's zone
        // arrives a frame late for no reason.
        app.configure_sets(
            Update,
            SoundSet
                .after(crate::world::session::place_entities)
                .after(crate::interface::GameSet),
        );
    }
}
