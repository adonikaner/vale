//! What to play when a model has no sequence for the animation asked for.
//!
//! [`fallbacks`] maps an animation id to the ids to try in order. It needs no
//! renderer and no unit state, only the id, so it is a rule of this crate. The
//! client's animation choice in `world/entities/pose.rs` asks for an id and
//! plays the first entry of its chain that the model carries.

use crate::world::m2::anim;

/// The fallback chain for an animation id: what to play when the model has no
/// sequence for what was asked.
///
/// A model without the requested sequence plays the first sequence in the
/// chain that it has. This applies often: 42 of the game's 405 animated models
/// have no Run, and a critter has no attack at all. The last entry of every
/// chain is Stand, which every animated model has.
pub fn fallbacks(wanted: u16) -> &'static [u16] {
    match wanted {
        anim::RUN => &[anim::RUN, anim::WALK, anim::STAND],
        anim::WALK => &[anim::WALK, anim::RUN, anim::STAND],
        // Moving backward. The chain comes from `AnimationData.dbc`'s
        // fallback column, which gives `Walkbackwards -> Walk -> Stand`. The
        // `Run` in the middle is added by this client, for the same reason as
        // in the two chains above: 42 of the game's 405 animated models have
        // no Walk, and one of those moving backward should move rather than
        // stand.
        anim::WALK_BACKWARDS => &[
            anim::WALK_BACKWARDS,
            anim::WALK,
            anim::RUN,
            anim::STAND,
        ],
        // The two stealth clips. Their chains come from `AnimationData.dbc`:
        // row 119's fallback column reads 4 (`Walk`) and row 120's reads 0
        // (`Stand`), so a creature with no crouch that the server has hidden
        // walks and stands rather than freezing. The `Run` after `Walk` is
        // added by this client, for the same reason as in the gait chains
        // above: 42 of the 405 animated models have no `Walk`, and one of
        // those creeping should still move.
        //
        // These chains make the creep rule safe to apply to every unit with
        // the creep bit, not only to characters. All sixteen character models
        // carry both clips and 0 of the 411 creature models carry either
        // (`vale anim`, whose population line prints all three numbers). So a
        // prowling cat, the most common creeping creature, takes the fallback
        // and keeps walking, which is what 5875 does.
        anim::STEALTH_WALK => &[
            anim::STEALTH_WALK,
            anim::WALK,
            anim::RUN,
            anim::STAND,
        ],
        anim::STEALTH_STAND => &[anim::STEALTH_STAND, anim::STAND],
        // The full-speed run. Its fallback column reads 5 (`Run`), so a model
        // with no `Sprint` runs. That is every one of the 411 creature models,
        // and so every mount. `Walk` and `Stand` after it are added by this
        // client, for the same reason the `Run` chain has them.
        anim::SPRINT => &[anim::SPRINT, anim::RUN, anim::WALK, anim::STAND],
        // The turn-in-place foot shuffle, carried by 104 of the 411 models:
        // the humanoids, which are also the only units a player watches turn
        // on the spot. It falls back to Stand only. The character is not
        // travelling, so a gait would make it run on the spot, which looks
        // worse than not shuffling.
        anim::SHUFFLE_LEFT => &[anim::SHUFFLE_LEFT, anim::STAND],
        anim::SHUFFLE_RIGHT => &[anim::SHUFFLE_RIGHT, anim::STAND],
        // In the air. `Jump` is the mid-flight loop and `Fall` is the loop for
        // an arc that did not start with a jump. Each falls back to the other,
        // because a model with only one of them looks better in the wrong
        // airborne pose than standing upright over a cliff.
        //
        // The ends of the arc (`JumpStart`, `JumpEnd` and `JumpLandRun`) are
        // one-shots played by the client's jump playback and deliberately have
        // no chain, which is what the `_ => &[]` arm at the bottom means: they
        // are played if the model has them and dropped otherwise. That is the
        // only safe form. A one-shot is held for the length of the sequence it
        // resolved to, so substituting Stand for a missing `JumpEnd` would
        // freeze a landing wolf upright for the 2.6 seconds of its idle loop
        // while it ran away.
        anim::JUMP => &[anim::JUMP, anim::JUMP_START, anim::FALL, anim::STAND],
        anim::FALL => &[anim::FALL, anim::JUMP, anim::STAND],
        // The rider's pose. The chain comes from `AnimationData.dbc`: row 91's
        // fallback column reads 0, `Stand`, and no pose fits between the two.
        // A model with no `Mount` is not built to ride, and every seated pose
        // in the table is made for a chair rather than a saddle. All eighteen
        // character models carry it, so the fallback is for a creature the
        // server has mounted, such as a kodo rider's kodo, and standing upright
        // on the saddle is the least wrong result.
        anim::MOUNT => &[anim::MOUNT, anim::STAND],
        // The loot crouch, which falls back to standing only. All sixteen
        // character models carry it and only a player loots, so this chain
        // covers a case that does not occur. Every seated pose in the table is
        // made for a chair rather than for crouching over a body, so
        // substituting one would look worse than not crouching.
        anim::LOOT => &[anim::LOOT, anim::STAND],
        // Death has no useful substitute: a creature that dies into its idle
        // loop looks as if it did not die. So a model without it holds Stand,
        // as it did before this table existed.
        anim::DEATH => &[anim::DEATH, anim::STAND],

        // The swings, one chain per weapon family. Each falls back through the
        // families a model is most likely to have instead, and every chain ends
        // at the unarmed swing before Stand: a critter has no attack at all, and
        // a wolf has only its bite, which is `ATTACK_UNARMED`.
        anim::ATTACK_UNARMED => &[
            anim::ATTACK_UNARMED,
            anim::ATTACK_1H,
            anim::ATTACK_2H,
            anim::STAND,
        ],
        anim::ATTACK_1H => &[anim::ATTACK_1H, anim::ATTACK_UNARMED, anim::STAND],
        // The stab, which `AnimationData.dbc` falls back to the swing (row
        // 85 -> 17). Every character model carries it and no creature does,
        // so a creature the server says stabbed plays whatever attack it has.
        anim::ATTACK_1H_PIERCE => &[
            anim::ATTACK_1H_PIERCE,
            anim::ATTACK_1H,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        anim::ATTACK_2H => &[
            anim::ATTACK_2H,
            anim::ATTACK_2HL,
            anim::ATTACK_1H,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        anim::ATTACK_2HL => &[
            anim::ATTACK_2HL,
            anim::ATTACK_2H,
            anim::ATTACK_1H,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        // A ranged attack falls back to the melee ones rather than to Stand:
        // a creature with a bow model and no bow animation must still visibly
        // act when it shoots.
        anim::ATTACK_BOW => &[anim::ATTACK_BOW, anim::ATTACK_RIFLE, anim::ATTACK_UNARMED, anim::STAND],
        anim::ATTACK_RIFLE => &[anim::ATTACK_RIFLE, anim::ATTACK_BOW, anim::ATTACK_UNARMED, anim::STAND],
        anim::ATTACK_THROWN => &[anim::ATTACK_THROWN, anim::ATTACK_UNARMED, anim::STAND],
        // The three off-hand swings and the critical: the animations the
        // packet and the off-hand item choose, rather than the main hand. All
        // of them fall back through the one-handed swing to the unarmed one.
        // Every character model has 87 and 88 and no creature model has any,
        // so these chains cover a creature the server says swung with its left
        // hand, which should play the ordinary attack it has. None falls back
        // through the two-handed swings, because a model with a 2H attack and
        // no `AttackOff` is a creature with no off hand.
        anim::ATTACK_OFF => &[
            anim::ATTACK_OFF,
            anim::ATTACK_1H,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        anim::ATTACK_OFF_PIERCE => &[
            anim::ATTACK_OFF_PIERCE,
            anim::ATTACK_OFF,
            anim::ATTACK_1H,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        // The empty left hand, whose first fallback comes from
        // `AnimationData.dbc` (row 117 -> 87). This chain is used more than
        // the others because `HumanMale.m2` lacks the clip: a sword-and-board
        // fighter's off-hand blow plays `AttackOff` for every human.
        anim::ATTACK_UNARMED_OFF => &[
            anim::ATTACK_UNARMED_OFF,
            anim::ATTACK_OFF,
            anim::ATTACK_UNARMED,
            anim::STAND,
        ],
        // The critical flinch falls back through the ordinary one, not through
        // the swings: 10, 9 and 8 are the three results of one wound reaction,
        // and a model that lacks the large flinch still has the small one.
        // This chain once ended at `ATTACK_1H`, part of the same error as
        // treating 10 as the attacker's blow; see the client's choice of swing.
        anim::COMBAT_CRITICAL => &[
            anim::COMBAT_CRITICAL,
            anim::COMBAT_WOUND,
            anim::STAND_WOUND,
            anim::STAND,
        ],

        // The flinch: 9 is the version for a unit already in a combat stance and
        // 8 the standing one; models carry one, the other, or neither.
        anim::COMBAT_WOUND => &[anim::COMBAT_WOUND, anim::STAND_WOUND, anim::STAND],
        // The three defensive reactions. Each falls back to the flinch rather
        // than to Stand: a creature with no dodge that dodges should still
        // visibly react, rather than stand motionless while the combat log
        // fills with misses.
        anim::DODGE => &[anim::DODGE, anim::COMBAT_WOUND, anim::STAND_WOUND, anim::STAND],
        anim::SHIELD_BLOCK => &[anim::SHIELD_BLOCK, anim::PARRY_1H, anim::COMBAT_WOUND, anim::STAND],
        anim::PARRY_UNARMED => &[anim::PARRY_UNARMED, anim::PARRY_1H, anim::COMBAT_WOUND, anim::STAND],
        anim::PARRY_1H => &[anim::PARRY_1H, anim::PARRY_UNARMED, anim::COMBAT_WOUND, anim::STAND],
        anim::PARRY_2H => &[anim::PARRY_2H, anim::PARRY_2HL, anim::PARRY_1H, anim::COMBAT_WOUND, anim::STAND],
        anim::PARRY_2HL => &[anim::PARRY_2HL, anim::PARRY_2H, anim::PARRY_1H, anim::COMBAT_WOUND, anim::STAND],

        // The ready stances, one per family, each falling back to the unarmed
        // guard every humanoid model carries.
        anim::READY_UNARMED => &[
            anim::READY_UNARMED,
            anim::READY_1H,
            anim::READY_2H,
            anim::STAND,
        ],
        anim::READY_1H => &[anim::READY_1H, anim::READY_UNARMED, anim::STAND],
        anim::READY_2H => &[anim::READY_2H, anim::READY_2HL, anim::READY_1H, anim::READY_UNARMED, anim::STAND],
        anim::READY_2HL => &[anim::READY_2HL, anim::READY_2H, anim::READY_1H, anim::READY_UNARMED, anim::STAND],
        anim::READY_BOW => &[anim::READY_BOW, anim::READY_RIFLE, anim::READY_UNARMED, anim::STAND],
        anim::READY_RIFLE => &[anim::READY_RIFLE, anim::READY_BOW, anim::READY_UNARMED, anim::STAND],
        anim::READY_THROWN => &[anim::READY_THROWN, anim::READY_UNARMED, anim::STAND],

        // The cast. These chains are required: `SpellCast` (32) and
        // `SpellPrecast` (31) are the `AnimationData` names, and no character
        // model carries either. `HumanMale.m2` has 51..54, the directed and
        // omni pairs, and nothing at 31 or 32. Without the chain, a request
        // for 32 would animate no player's cast at all.
        anim::SPELL_CAST => &[
            anim::SPELL_CAST,
            anim::SPELL_CAST_OMNI,
            anim::SPELL_CAST_DIRECTED,
            anim::SPELL,
            anim::STAND,
        ],
        anim::SPELL_PRECAST => &[
            anim::SPELL_PRECAST,
            anim::READY_SPELL_OMNI,
            anim::READY_SPELL_DIRECTED,
            anim::CHANNEL_CAST_OMNI,
            anim::CHANNEL_CAST_DIRECTED,
            anim::SPELL,
            anim::STAND,
        ],
        // The four ids the spell tables name, which a cast resolves to
        // whenever the tables give one: `SpellVisual` gives a fireball the
        // directed pair and a heal the omni pair. Requesting the generic id
        // would play the same pose for both.
        //
        // They have their own chains because creatures often cast players'
        // spells (a Defias mage casts the same fireball), and a creature model
        // carries at most `Spell` (2) and often nothing. Each therefore tries
        // its own pair, then the generic id, then Stand.
        anim::READY_SPELL_DIRECTED => &[
            anim::READY_SPELL_DIRECTED,
            anim::READY_SPELL_OMNI,
            anim::SPELL_PRECAST,
            anim::SPELL,
            anim::STAND,
        ],
        anim::READY_SPELL_OMNI => &[
            anim::READY_SPELL_OMNI,
            anim::READY_SPELL_DIRECTED,
            anim::SPELL_PRECAST,
            anim::SPELL,
            anim::STAND,
        ],
        anim::SPELL_CAST_DIRECTED => &[
            anim::SPELL_CAST_DIRECTED,
            anim::SPELL_CAST_OMNI,
            anim::SPELL_CAST,
            anim::SPELL,
            anim::STAND,
        ],
        anim::SPELL_CAST_OMNI => &[
            anim::SPELL_CAST_OMNI,
            anim::SPELL_CAST_DIRECTED,
            anim::SPELL_CAST,
            anim::SPELL,
            anim::STAND,
        ],
        // A channel is held like a wind-up, so it falls back to the wind-up
        // rather than the release: a caster whose model has no channel
        // animation should stand ready, not throw the spell repeatedly.
        anim::CHANNEL_CAST_DIRECTED => &[
            anim::CHANNEL_CAST_DIRECTED,
            anim::CHANNEL_CAST_OMNI,
            anim::READY_SPELL_DIRECTED,
            anim::SPELL,
            anim::STAND,
        ],
        anim::CHANNEL_CAST_OMNI => &[
            anim::CHANNEL_CAST_OMNI,
            anim::CHANNEL_CAST_DIRECTED,
            anim::READY_SPELL_OMNI,
            anim::SPELL,
            anim::STAND,
        ],

        anim::SWIM => &[anim::SWIM, anim::SWIM_IDLE, anim::RUN, anim::STAND],
        // A swimmer at rest. Falls back to Swim rather than to Stand, because a
        // creature swimming in place looks better than one standing upright in
        // the water.
        anim::SWIM_IDLE => &[anim::SWIM_IDLE, anim::SWIM, anim::STAND],
        // The three other strokes, each falling back to Swim.
        // `AnimationData.dbc` gives that chain for `SwimBackwards` and sends
        // the two side strokes straight to Stand. Going through Swim for those
        // is this client's choice, for the same reason as the gait chains: a
        // creature with one stroke that swims sideways should swim.
        anim::SWIM_BACKWARDS => &[
            anim::SWIM_BACKWARDS,
            anim::SWIM,
            anim::SWIM_IDLE,
            anim::STAND,
        ],
        anim::SWIM_LEFT => &[anim::SWIM_LEFT, anim::SWIM, anim::SWIM_IDLE, anim::STAND],
        anim::SWIM_RIGHT => &[anim::SWIM_RIGHT, anim::SWIM, anim::SWIM_IDLE, anim::STAND],
        // The seated poses. A model with no sitting animation stands, which is
        // wrong but visibly so.
        anim::SIT_CHAIR_MED => &[
            anim::SIT_CHAIR_MED,
            anim::SIT_CHAIR_LOW,
            anim::SIT_GROUND,
            anim::STAND,
        ],
        anim::SIT_CHAIR_LOW => &[anim::SIT_CHAIR_LOW, anim::SIT_GROUND, anim::STAND],
        anim::SIT_CHAIR_HIGH => &[
            anim::SIT_CHAIR_HIGH,
            anim::SIT_CHAIR_MED,
            anim::SIT_GROUND,
            anim::STAND,
        ],
        anim::SIT_GROUND => &[anim::SIT_GROUND, anim::STAND],
        // The two held states of a game object. Each ends at Stand, so a game
        // object whose model is a plain prop (a waving banner, a brazier)
        // plays its own idle. `AnimationData.dbc`'s chain runs the other way
        // (`Closed -> Close -> Open`, and `Open -> Close`, a cycle). That
        // works when followed one step at a time, but not as a list to search:
        // an open chest with no `Opened` would play the closing animation and
        // hold its last frame, which shows a shut chest.
        anim::CLOSED => &[anim::CLOSED, anim::STAND],
        anim::OPENED => &[anim::OPENED, anim::CLOSED, anim::STAND],
        anim::SLEEP => &[anim::SLEEP, anim::SIT_GROUND, anim::STAND],
        anim::KNEEL => &[anim::KNEEL, anim::SIT_GROUND, anim::STAND],
        // No chain. An id not listed here is played if the model has it and
        // dropped if not. This covers the 78
        // emotes, which are scattered across `AnimationData` ids, and for
        // which a substitution would show a character standing still.
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fallback for row 91 in `AnimationData.dbc` is `Stand`, with nothing in
    /// between. See [`fallbacks`].
    #[test]
    fn a_model_with_no_mount_pose_stands_up() {
        assert_eq!(fallbacks(anim::MOUNT), &[anim::MOUNT, anim::STAND]);
    }
}
