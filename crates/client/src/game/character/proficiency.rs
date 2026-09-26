//! **What the character is allowed to hold**, and the one packet that says so.
//!
//! `SMSG_SET_PROFICIENCY` arrives twice on login — once for weapons, once for
//! armour — and again whenever a proficiency is learned or lost. Each is that
//! class's **complete** mask rather than a delta, so this module is a resource
//! that replaces and a handful of readers that ask it one question:
//! [`Proficiencies::allows`].
//!
//! ## Nothing in the archives can answer this
//!
//! Which class may hold which weapon is not in a shipped file. `Item.dbc` is not
//! in the MPQs at all, the skill lines that grant a proficiency are the server's
//! own `playercreateinfo_spell` and trainer tables, and a character's *learned*
//! proficiencies are per character by definition. So a client with no reader for
//! this packet has to draw every item as usable — which is what this one did,
//! and which is a character told they may equip a weapon they cannot and finding
//! out from a refusal a round trip later.
//!
//! ## Who asks
//!
//! * The trade window's two `*ItemInfo` reads, whose fourth answer is the red
//!   square (`lua::panels::trade`).
//! * The cast rule, for `SPELL_FAILED_EQUIPPED_ITEM_CLASS` — a spell that
//!   requires a weapon class the character cannot hold.
//!
//! The bag and paperdoll squares do **not** yet: `GetContainerItemInfo` has no
//! usable slot in 1.12 and the red name on a plate is composed from the
//! requirement lines, which are owed separately.

use vale_protocol::play::skills::Proficiency;
use bevy::prelude::*;

/// **One statement about one item class**, on its way from the socket to
/// [`Proficiencies`].
///
/// A message rather than a direct write because the drain is a hub and every arm
/// there ends in a write rather than a decision — see
/// [`crate::game::incoming`].
#[derive(Message, Debug, Clone, Copy)]
pub struct ProficiencyAnswer(pub Proficiency);

/// **Every class's mask, as this session has been told them.**
///
/// The protocol type does the holding; this is the resource wrapper, on the same
/// terms as [`crate::game::character::reputation::PlayerStanding`]: the rule is
/// in a crate with no Bevy in it and can be unit-tested without one.
#[derive(Resource, Default, Debug, Clone)]
pub struct Proficiencies(pub vale_protocol::play::skills::Proficiencies);

pub struct ProficiencyPlugin;

impl Plugin for ProficiencyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Proficiencies>()
            .add_message::<ProficiencyAnswer>()
            .add_systems(Update, (apply, leave_world));
    }
}

fn apply(mut said: MessageReader<ProficiencyAnswer>, mut held: ResMut<Proficiencies>) {
    for ProficiencyAnswer(one) in said.read() {
        held.0.set(*one);
    }
}

/// **Forgotten on the way out**, because the next character is a different
/// person. Nothing re-sends a proficiency for a class the new character has none
/// of, so a kept mask is a warrior's plate proficiency worn by a mage — see
/// `crate::game::session::logout`, which is where this class of bug was found
/// the first three times.
fn leave_world(
    mut leaving: MessageReader<super::super::events::PlayerLeavingWorld>,
    mut held: ResMut<Proficiencies>,
) {
    if leaving.read().next().is_some() {
        *held = Proficiencies::default();
    }
}
