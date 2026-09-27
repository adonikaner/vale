//! **What an NPC says when you walk up, and when you walk away** —
//! `NPCSounds.dbc`'s `hello` and `goodbye` columns, off the click and off the
//! `"npc"` token's closing edge.
//!
//! ```text
//! CreatureDisplayInfo.dbc field 11  ->  NPCSounds.dbc id  ->  hello / goodbye / pissed / ack
//! ```
//!
//! The reference plays the greeting on the click and the farewell the moment
//! a conversation window closes, and both are the *display*'s rather than the
//! creature's: the column is on `CreatureDisplayInfo`, so two innkeepers with
//! one model say the same thing. See
//! [`vale_assets::tables::sound::SoundBank::npc_sounds`], which resolves
//! the row, and `vale sound`, which checks every one of the 156 against
//! the archives.
//!
//! **The greeting is the click's, and the farewell is the window's.** Every
//! click on a unit greets — a guard clicked only to read its name, and
//! clicked again while it is still the target, says hello each time, which is
//! the reference's behaviour and is why the trigger is
//! [`crate::interface::target::UnitClicked`] rather than the selection:
//! the selection does not move on the second click and the greeting has to.
//! A window opening on somebody nothing clicked (a scripted opener) greets
//! too; [`crate::interface::gossip::NpcUnit`] points at whoever the character
//! is talking to across the six conversation windows and the trade, and its
//! edge off a unit is the farewell. A trade partner is a player, has no
//! display row here, and says nothing — which is the reference's behaviour
//! too. `pissed` and `ack` are the two the wire never asks for: they are the
//! *emote* answers, and are owed with the emote packet's sound column.

use bevy::prelude::*;

use super::mixer::{Place, Voices};
use crate::interface::target::UnitClicked;
use crate::interface::gossip::NpcUnit;
use crate::world::session::{EntityIndex, WorldEntity};

pub struct NpcSoundPlugin;

impl Plugin for NpcSoundPlugin {
    fn build(&self, app: &mut App) {
        // After the token has been pointed this frame — see
        // `interface::gossip::point_the_token`, which runs inside `GameSet` —
        // and after the two click systems, which are in the same set.
        app.add_systems(Update, greetings.after(crate::interface::GameSet));
    }
}

/// The two columns, resolved for one display.
fn columns(
    game: &crate::assets::GameAssets,
    display_id: u32,
) -> Option<(Option<u32>, Option<u32>)> {
    let tables = game.display_tables().ok()?;
    let row = tables.npc_sound_id(display_id)?;
    let bank = game.sounds();
    let sounds = bank.npc_sounds(row)?;
    let nonzero = |id: u32| (id != 0).then_some(id);
    Some((nonzero(sounds[0]), nonzero(sounds[1])))
}

/// Play `hello` on every click that lands on somebody, and `goodbye` when a
/// window closes on them, at where they stand, since a farewell is heard
/// walking away.
///
/// **A click greets, every time.** The reference greets on the click whether
/// or not a conversation follows and whether or not the unit was already the
/// target, so this reads the click's own edge rather than the selection's.
/// A window opening on somebody the last click did not land on — a scripted
/// `GossipHello`, a probe — greets as well, once; the right click that opens
/// a window on a real session has already greeted by the time the token
/// moves, a round trip later, and `last_clicked` is what keeps the two from
/// stacking.
#[allow(clippy::too_many_arguments)]
fn greetings(
    npc: Res<NpcUnit>,
    mut clicked: MessageReader<UnitClicked>,
    mut last_token: Local<Option<(u64, Vec3, Option<u32>)>>,
    mut last_clicked: Local<Option<u64>>,
    index: Res<EntityIndex>,
    units: Query<(&WorldEntity, &GlobalTransform)>,
    game: Res<crate::assets::GameAssets>,
    mut voices: Voices,
) {
    let resolve = |guid: u64| -> Option<(Vec3, Option<u32>, Option<u32>)> {
        let (unit, transform) = index.0.get(&guid).and_then(|entity| units.get(*entity).ok())?;
        let (hello, goodbye) = unit
            .display_id
            .and_then(|display| columns(&game, display))
            .unwrap_or((None, None));
        Some((transform.translation(), hello, goodbye))
    };
    let greet = |guid: u64, voices: &mut Voices| {
        if let Some((at, Some(hello), _)) = resolve(guid) {
            let bank = game.sounds();
            voices.play(&bank, hello, Place::At(at));
        }
    };

    // The click's edge: each one, even on the unit already selected.
    for click in clicked.read() {
        *last_clicked = Some(click.guid);
        greet(click.guid, &mut voices);
    }

    // The token's edge: a window opened, or closed.
    let tok = npc.0;
    if last_token.map(|(guid, _, _)| guid) != tok {
        if let Some((_, at, Some(goodbye))) = last_token.take() {
            let bank = game.sounds();
            voices.play(&bank, goodbye, Place::At(at));
        }
        match tok {
            Some(guid) => {
                let (at, _, goodbye) = resolve(guid).unwrap_or((Vec3::ZERO, None, None));
                *last_token = Some((guid, at, goodbye));
                // Opened on somebody no click reached: greet once. Opened by
                // the right click that just greeted: not again.
                if *last_clicked != Some(guid) {
                    *last_clicked = Some(guid);
                    greet(guid, &mut voices);
                }
            }
            None => *last_token = None,
        }
    }
}
