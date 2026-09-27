//! **What a talent does to a spell's numbers** — the two
//! `SMSG_SET_*_SPELL_MODIFIER` packets, and the four values this client prints
//! that they change.
//!
//! A modifier is `(bit, operation, value)`: which bit of a spell's
//! `SpellFamilyFlags` it is about, which number it moves, and by how much. The
//! server sends **one packet per bit**, carrying that bit's running total for
//! that operation — see [`vale_protocol::play::spells::SpellModifier`], whose
//! own note is that these are totals rather than deltas and that a reader which
//! accumulates doubles every talent on every login.
//!
//! ## What this client can honestly do with them
//!
//! Four of the thirty-odd operations move a number this client computes for
//! itself and shows to the player:
//!
//! ```text
//! CASTING_TIME  the wind-up: the cast bar's length, and "x sec cast"
//! COST          the mana, rage or energy the tooltip prints and check_cast tests
//! COOLDOWN      the recovery the button's sweep is drawn from
//! RANGE         the yards the bar's red border and the out-of-range refusal use
//! ```
//!
//! The rest are the server's arithmetic — damage, threat, crit chance, proc
//! chance, jump targets — and there is nothing on this side to apply them to. A
//! packet for one of those is stored and never read, which is the right shape:
//! the day a consumer exists, the number is already there.
//!
//! ## Both kinds, in the reference's order
//!
//! Flat is added and percent is a percentage, and the reference applies **flat
//! first, then percent** — `value = (base + flat) * (100 + pct) / 100`. The
//! order matters for anything carrying both, and both are signed: every
//! cast-time and cost reduction in the game is a negative percent.
//!
//! ## Which spells a bit is about, and the one thing not answered here
//!
//! A modifier applies to a spell when the spell's `SpellFamilyName` matches the
//! talent's and its `SpellFamilyFlags` has the bit. **This client reads neither
//! column**, so [`SpellMods::of`] takes the bit mask from the caller and the one
//! caller that exists passes the spell's own — see
//! [`crate::interface::spellbook`]. Until the two columns are read a talented
//! cast prints its untalented number, which is exactly what it did before this
//! module and is the honest failure rather than a wrong one.

use vale_protocol::play::spells::{spell_mod_op, SpellModifier};
use bevy::prelude::*;

/// One modifier on its way from the socket to [`SpellMods`].
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellModAnswer(pub SpellModifier);

/// **Every modifier this session has been told about**, by operation and bit.
///
/// A flat list rather than a `[[i32; 64]; 32]` pair: a character has a handful
/// of talents and the array would be 16 KB of zeros walked on every tooltip.
#[derive(Resource, Default, Debug, Clone)]
pub struct SpellMods {
    held: Vec<SpellModifier>,
}

impl SpellMods {
    /// Take one packet, **replacing** the total it supersedes.
    pub fn set(&mut self, said: SpellModifier) {
        let same = |held: &&mut SpellModifier| {
            (held.effect_bit, held.op, held.percent) == (said.effect_bit, said.op, said.percent)
        };
        match self.held.iter_mut().find(same) {
            Some(held) => *held = said,
            None => self.held.push(said),
        }
    }

    /// The flat and percent totals for one operation over one set of bits.
    ///
    /// `bits` is the spell's `SpellFamilyFlags`; every modifier whose bit is in
    /// it contributes. Returns `(flat, percent)`, both zero when nothing
    /// applies — which is the answer for every spell of a character with no
    /// talents, and the reason [`Self::apply`] is cheap enough for a tooltip.
    pub fn of(&self, op: u8, bits: u64) -> (i32, i32) {
        let mut flat = 0;
        let mut percent = 0;
        for held in &self.held {
            if held.op != op || bits & (1u64 << held.effect_bit) == 0 {
                continue;
            }
            if held.percent {
                percent += held.value;
            } else {
                flat += held.value;
            }
        }
        (flat, percent)
    }

    /// One value through both, in the reference's order: flat, then percent.
    ///
    /// Clamped at zero rather than allowed to go negative — a cast time or a
    /// cost below zero is not a thing the rest of this client can carry.
    pub fn apply(&self, op: u8, bits: u64, base: i32) -> i32 {
        let (flat, percent) = self.of(op, bits);
        let moved = i64::from(base.saturating_add(flat)) * i64::from(100 + percent) / 100;
        moved.clamp(0, i64::from(i32::MAX)) as i32
    }

    /// [`Self::apply`] over a `u32`, which is what the three time-and-cost
    /// values on a `SpellInfo` are.
    pub fn apply_u32(&self, op: u8, bits: u64, base: u32) -> u32 {
        self.apply(op, bits, base.min(i32::MAX as u32) as i32).max(0) as u32
    }

    /// …and over the one that is yards.
    pub fn apply_yards(&self, bits: u64, base: f32) -> f32 {
        let (flat, percent) = self.of(spell_mod_op::RANGE, bits);
        ((base + flat as f32) * (100.0 + percent as f32) / 100.0).max(0.0)
    }

    /// **[`Self::apply_u32`], or `None` when nothing applies** — the shape a
    /// `CastConditions` override wants.
    ///
    /// The `None` is not a convenience: it is what says "use the row's own", so
    /// a character with no talents leaves every check exactly as it was rather
    /// than routing every number through an identity.
    pub fn moved(&self, op: u8, bits: u64, base: u32) -> Option<u32> {
        (self.of(op, bits) != (0, 0)).then(|| self.apply_u32(op, bits, base))
    }

    /// …and the same for the one that is yards.
    pub fn moved_yards(&self, bits: u64, base: f32) -> Option<f32> {
        (self.of(spell_mod_op::RANGE, bits) != (0, 0)).then(|| self.apply_yards(bits, base))
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

pub struct SpellModPlugin;

impl Plugin for SpellModPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpellMods>()
            .add_message::<SpellModAnswer>()
            .add_systems(Update, (apply, leave_world));
    }
}

fn apply(mut said: MessageReader<SpellModAnswer>, mut held: ResMut<SpellMods>) {
    for SpellModAnswer(one) in said.read() {
        held.set(*one);
    }
}

/// **Forgotten on the way out.** The next character has different talents and
/// nothing re-sends a zero for a modifier they do not have.
fn leave_world(
    mut leaving: MessageReader<crate::interface::events::PlayerLeavingWorld>,
    mut held: ResMut<SpellMods>,
) {
    if leaving.read().next().is_some() {
        *held = SpellMods::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modifier(bit: u8, op: u8, value: i32, percent: bool) -> SpellModifier {
        SpellModifier { effect_bit: bit, op, value, percent }
    }

    /// **A re-sent total replaces, it does not accumulate** — see the module
    /// note, which is why this is the first test in the file.
    #[test]
    fn a_second_packet_for_one_bit_replaces_the_first() {
        let mut mods = SpellMods::default();
        mods.set(modifier(3, spell_mod_op::CASTING_TIME, -20, true));
        mods.set(modifier(3, spell_mod_op::CASTING_TIME, -40, true));
        assert_eq!(mods.of(spell_mod_op::CASTING_TIME, 1 << 3), (0, -40));
        // …and flat and percent for the same bit are two separate totals.
        mods.set(modifier(3, spell_mod_op::CASTING_TIME, -500, false));
        assert_eq!(mods.of(spell_mod_op::CASTING_TIME, 1 << 3), (-500, -40));
    }

    /// Flat first, then percent, and the clamp at zero.
    #[test]
    fn a_value_goes_through_flat_then_percent() {
        let mut mods = SpellMods::default();
        mods.set(modifier(0, spell_mod_op::CASTING_TIME, -500, false));
        mods.set(modifier(0, spell_mod_op::CASTING_TIME, -10, true));
        // (3000 - 500) * 0.9
        assert_eq!(mods.apply_u32(spell_mod_op::CASTING_TIME, 1, 3000), 2250);
        // A reduction bigger than the value floors rather than wrapping.
        mods.set(modifier(0, spell_mod_op::CASTING_TIME, -9000, false));
        assert_eq!(mods.apply_u32(spell_mod_op::CASTING_TIME, 1, 3000), 0);
    }

    /// **A bit the spell does not carry contributes nothing**, which is what
    /// keeps a talent for one spell off every other one.
    #[test]
    fn only_the_bits_the_spell_carries_apply() {
        let mut mods = SpellMods::default();
        mods.set(modifier(5, spell_mod_op::COST, -50, true));
        assert_eq!(mods.apply_u32(spell_mod_op::COST, 1 << 5, 100), 50);
        assert_eq!(mods.apply_u32(spell_mod_op::COST, 1 << 6, 100), 100);
        // …and so does a different operation.
        assert_eq!(mods.apply_u32(spell_mod_op::COOLDOWN, 1 << 5, 100), 100);
        // Nothing at all is the identity, which is every spell of every
        // character with no talents.
        assert!(SpellMods::default().is_empty());
        assert_eq!(SpellMods::default().apply_u32(spell_mod_op::COST, u64::MAX, 100), 100);
    }
}
