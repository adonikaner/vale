//! **The character sheet's news feed** — the nine `UNIT_*` stat events, on
//! exactly [`super::vitals`]' terms: written on a change and never per frame.
//!
//! `PaperDollFrame_OnEvent` re-reads a different set of functions per name and
//! *nothing* polls, so a panel that is open while a buff lands, a level is
//! gained or a weapon is swapped shows the numbers it was opened with until it
//! is closed and reopened. The panel's own `OnShow` covers the opening; this
//! covers the rest.
//!
//! ## What is compared is what a re-read would answer
//!
//! Every field of [`Reading`] is one of [`vale_protocol::play::stats::UnitStats`]'
//! own public answers rather than a raw update field — the same argument
//! `vitals` makes. A diff on the fields would fire `UNIT_STATS` for a change
//! the panel cannot see (a buff field moving under a clamp), and — worse — a
//! diff on a *subset* of them would miss one it can.
//!
//! ## `player` and `pet`
//!
//! Unlike the vitals, there is nothing to watch on a target: every field the
//! stat block is made of is `PRIVATE`/`OWNER_ONLY` and the server sends nobody
//! else's. The one other unit whose block does arrive is our own pet —
//! `OWNER_ONLY` means the owner gets it — and the one panel that reads another
//! unit's stats is `PetPaperDollFrame`, whose `OnEvent` re-reads on the same
//! nine names at `arg1 == "pet"`. So the watch is those two tokens and no
//! more; every other token reads `None` on its first line and costs nothing.

use bevy::prelude::*;

use super::super::api::{UnitId, Units};
use super::super::events::{StatGroup, UnitStatsChanged};
use vale_protocol::play::stats::{Damage, RangedDamage, NUM_RESISTANCES, NUM_STATS};

/// One reading of everything the sheet draws, in the shapes the interface asks
/// for it in.
#[derive(Debug, Clone, PartialEq)]
struct Reading {
    stats: [(i32, i32, i32, i32); NUM_STATS],
    resistances: [(i32, i32, i32, i32); NUM_RESISTANCES],
    damage: Damage,
    ranged_damage: RangedDamage,
    attack_power: (i32, i32, i32),
    ranged_attack_power: (i32, i32, i32),
    attack_speed: (f32, Option<f32>),
    /// Both weapon skills, the ranged one and defense — the four answers that
    /// come out of the skill block rather than out of a field of their own.
    skills: [(i32, i32); 4],
}

impl Reading {
    fn read(units: &Units, id: UnitId) -> Option<Self> {
        let stats = units.stats(id)?;
        let mut out = Reading {
            stats: [(0, 0, 0, 0); NUM_STATS],
            resistances: [(0, 0, 0, 0); NUM_RESISTANCES],
            damage: stats.damage(),
            ranged_damage: stats.ranged_damage(),
            attack_power: stats.attack_power(),
            ranged_attack_power: stats.ranged_attack_power(),
            attack_speed: stats.attack_speed(),
            skills: [
                stats.weapon_skill(0),
                stats.weapon_skill(1),
                stats.ranged_skill(),
                stats.defense(),
            ],
        };
        for i in 0..NUM_STATS {
            // One-based, the way the panel's own loop passes it.
            out.stats[i] = stats.stat(i + 1).unwrap_or_default();
        }
        for i in 0..NUM_RESISTANCES {
            out.resistances[i] = stats.resistance(i).unwrap_or_default();
        }
        Some(out)
    }
}

pub struct StatsPlugin;

impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, announce.in_set(super::super::GameSet));
    }
}

/// One write per changed group, once per change.
///
/// The first reading announces **nothing**, which is the opposite of
/// [`super::vitals`]' rule and is deliberate: a unit frame that has never heard
/// an event sits at its loaded state and needs telling, where the character
/// sheet fills itself from `PaperDollFrame_OnShow` and is hidden until then. An
/// event at login would be read by a panel that is not visible and dropped on
/// its handler's second line.
fn announce(
    units: Units,
    mut last: Local<[Option<Reading>; 2]>,
    mut changed: MessageWriter<UnitStatsChanged>,
) {
    for (slot, id) in [UnitId::Player, UnitId::Pet].into_iter().enumerate() {
        let now = Reading::read(&units, id);
        let previous = std::mem::replace(&mut last[slot], now.clone());
        let (Some(now), Some(previous)) = (now, previous) else {
            continue;
        };

        let mut raise = |what: StatGroup| {
            changed.write(UnitStatsChanged { unit: id, what });
        };
        if now.stats != previous.stats {
            raise(StatGroup::Attributes);
        }
        // **Armour rides on this one**, which is the panel's own wiring:
        // `UNIT_RESISTANCES` runs `PaperDollFrame_SetResistances` *and*
        // `PaperDollFrame_SetArmor`, because armour is resistance school 0.
        if now.resistances != previous.resistances {
            raise(StatGroup::Resistances);
        }
        // The three damage-done mods are half of what `UnitDamage` answers, and
        // the panel gives them their own name — so a change in them is both this
        // and `UNIT_DAMAGE` in the real client's ordering. Sent as the mods'
        // event, which re-runs all four setters anyway.
        if (now.damage.bonus_positive, now.damage.bonus_negative, now.damage.percent)
            != (previous.damage.bonus_positive, previous.damage.bonus_negative, previous.damage.percent)
        {
            raise(StatGroup::DamageDoneMods);
        } else if now.damage != previous.damage {
            raise(StatGroup::Damage);
        }
        if now.ranged_damage != previous.ranged_damage {
            raise(StatGroup::RangedDamage);
        }
        if now.attack_speed != previous.attack_speed {
            raise(StatGroup::AttackSpeed);
        }
        if now.attack_power != previous.attack_power {
            raise(StatGroup::AttackPower);
        }
        if now.ranged_attack_power != previous.ranged_attack_power {
            raise(StatGroup::RangedAttackPower);
        }
        if now.skills != previous.skills {
            raise(StatGroup::WeaponSkill);
        }
    }
}
