//! **The noise a message makes** — the sound column of the client's own
//! message table, drained.
//!
//! Thirty of the game's 343 messages name a sound, and the name is a
//! `SoundEntries` row rather than an id: `igQuestFailed` for the eleven ways a
//! quest can be refused, `igPlayerInviteDecline` for the seven refusals of an
//! invitation, `TaxiNodeDiscovered` for a flight path. So the resolution is the
//! same by-name lookup [`super::interface`] makes for `PlaySound()` and nothing
//! new — see [`vale_assets::interface::messages`], which carries the table.
//!
//! **Flat rather than placed**, like [`super::player`]'s two: every one of them
//! is an event about the person reading the screen — an invitation declined, a
//! quest failed, a path discovered — and none of them happens anywhere in the
//! world.
//!
//! One system and no state, because the decision was made two crates away: what
//! is left here is the mixer door, which is the whole of what `sound/` owes.
//!
//! ## What is not here
//!
//! **The speech column.** 52 of the messages name one of 45
//! error-speech lines — the character's own voice saying "My bags are full" —
//! and it is *exclusive* with the sound rather than additional to it, so
//! nothing in this file is competing with it. It needs a table inside the
//! client (indexed by race-and-gender then by line) and the
//! `EnableErrorSpeech` CVar, and neither is read yet.

use super::mixer::{Place, Voices};
use crate::interface::messages::MessageSound;
use bevy::prelude::*;

pub struct MessagesPlugin;

impl Plugin for MessagesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, play.in_set(super::SoundSet));
    }
}

fn play(
    mut asked: MessageReader<MessageSound>,
    mut voices: Voices,
    game: Res<crate::assets::GameAssets>,
) {
    // Collected first so the bank is only opened when there is something to
    // play — it takes a lock, and this runs every frame.
    let names: Vec<&'static str> = asked.read().map(|MessageSound(name)| *name).collect();
    if names.is_empty() {
        return;
    }
    let bank = game.sounds();
    for name in names {
        match bank.entry_named(name) {
            Some(entry) => {
                voices.play(&bank, entry.id, Place::Flat);
            }
            // Not reachable from the shipped table — `vale messages` checks
            // all thirty names against `SoundEntries` and they all resolve — so
            // this firing means the table and the archives have parted company.
            None => warn!("message sound {name:?}: no SoundEntries row by that name"),
        }
    }
}
