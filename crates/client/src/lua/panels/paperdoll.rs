//! **The C functions `PaperDollFrame.lua` calls** — the character sheet's
//! numbers.
//!
//! The same split the spellbook set: the panel is the game's own 860 lines of
//! Lua, and what the client owes it is a set of reads and a set of events.
//! Nothing here draws a character sheet; it answers questions about one.
//!
//! ```text
//! UnitStat(unit, i)          stat, effectiveStat, posBuff, negBuff
//! UnitResistance(unit, i)    base, resistance, positive, negative
//! UnitArmor(unit)            base, effectiveArmor, armor, posBuff, negBuff
//! UnitAttackPower(unit)      base, posBuff, negBuff
//! UnitRangedAttackPower(u)   …the same three, one array along
//! UnitAttackSpeed(unit)      speed, offhandSpeed — the second nil, not zero
//! UnitDamage(unit)           min, max, minOff, maxOff, +bonus, -bonus, percent
//! UnitRangedDamage(unit)     speed, min, max, +bonus, -bonus, percent
//! UnitDefense(unit)          base, modifier
//! UnitAttackBothHands(unit)  mainBase, mainMod, offBase, offMod
//! UnitRangedAttack(unit)     base, modifier
//! HasWandEquipped()          …which blanks the ranged block rather than zeroing it
//! UnitRace / UnitClass       (localised, fileName) — the level line's two names
//! ```
//!
//! **Which update fields each of those combines is
//! [`vale_protocol::play::stats`]'**, function by function; this file is the registration and the argument handling only. That is the
//! same division `lua::spellbook` has against `assets::book`, and for the same
//! reason: the rule can then be unit-tested with no interpreter and no window.
//!
//! ## An absent unit answers zeroes, and a bad argument does not raise
//!
//! The client distinguishes the two. `UnitStat("party1", 1)` on a token naming
//! nobody pushes four zeroes; `UnitStat(nil)`
//! and `UnitStat("player", 9)` call `luaL_error` with
//! `Usage: UnitStat("unit", statIndex)` and `Invalid stat index in UnitStat`.
//!
//! **This client keeps the first and declines the second**, which is the
//! convention [`super::super::api`] states for the whole read surface: an argument
//! error here would take out the `OnShow` of whatever panel made it, where the
//! absent value degrades to a sheet of zeroes. The usage strings are not
//! reproduced here.

use super::super::api::{one_or_nil, Answers};

/// **The reads this module registers**, for the count that measures the gap —
/// the same list [`super::super::api::READS`] is, checked the same way.
pub const READS: [&str; 14] = [
    "HasWandEquipped",
    "UnitArmor",
    "UnitAttackBothHands",
    "UnitAttackPower",
    "UnitAttackSpeed",
    "UnitClass",
    "UnitDamage",
    "UnitDefense",
    "UnitRace",
    "UnitRangedAttack",
    "UnitRangedAttackPower",
    "UnitRangedDamage",
    "UnitResistance",
    "UnitStat",
];

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    // A unit token and nothing else, which is nine of the fourteen. The body
    // gets the stat block or `None`; **every one of them has an answer for
    // `None`**, and it is the shape the client pushes for a unit that is not
    // there rather than a `nil` the panel would then do arithmetic on.
    macro_rules! stats {
        ($name:expr, |$stats:ident| $body:expr, $absent:expr) => {{
            let f = scope.create_function(move |_, token: Option<String>| {
                let token = token.unwrap_or_default();
                Ok(match answers.unit_stats(&token) {
                    Some($stats) => $body,
                    None => $absent,
                })
            })?;
            globals.set($name, f)?;
        }};
    }

    // --- the five attributes and the seven resistances ---

    globals.set(
        "UnitStat",
        scope.create_function(move |_, (token, index): (Option<String>, Option<usize>)| {
            let stats = answers.unit_stats(&token.unwrap_or_default());
            // **One-based**, as `for i=1, NUM_STATS` passes it.
            Ok(stats
                .and_then(|s| s.stat(index.unwrap_or(0)))
                .unwrap_or((0, 0, 0, 0)))
        })?,
    )?;
    globals.set(
        "UnitResistance",
        scope.create_function(move |_, (token, school): (Option<String>, Option<usize>)| {
            let stats = answers.unit_stats(&token.unwrap_or_default());
            // **Zero-based**, and the panel passes `frame:GetID()` — which for
            // `MagicResFrame1` is 6, arcane. The five frames carry 6, 2, 3, 4,
            // 5 and holy has no frame at all.
            Ok(stats
                .and_then(|s| s.resistance(school.unwrap_or(usize::MAX)))
                .unwrap_or((0, 0, 0, 0)))
        })?,
    )?;
    stats!("UnitArmor", |s| s.armor(), (0, 0, 0, 0, 0));

    // --- attack power, and its ranged twin ---

    stats!("UnitAttackPower", |s| s.attack_power(), (0, 0, 0));
    stats!("UnitRangedAttackPower", |s| s.ranged_attack_power(), (0, 0, 0));

    // --- what the weapons do ---

    stats!(
        "UnitAttackSpeed",
        |s| {
            let (speed, offhand) = s.attack_speed();
            (speed, offhand)
        },
        (0.0, None)
    );
    stats!(
        "UnitDamage",
        |s| {
            let d = s.damage();
            (
                d.min,
                d.max,
                d.min_offhand,
                d.max_offhand,
                d.bonus_positive,
                d.bonus_negative,
                d.percent,
            )
        },
        // **The seventh is 1.0 and not 0.0** even here, because
        // `PaperDollFrame_SetDamage` divides by it on its second line — the
        // client pushes the same literal down its own absent branch.
        (0.0, 0.0, 0.0, 0.0, 0, 0, 1.0)
    );
    stats!(
        "UnitRangedDamage",
        |s| {
            let d = s.ranged_damage();
            (d.speed, d.min, d.max, d.bonus_positive, d.bonus_negative, d.percent)
        },
        (0.0, 0.0, 0.0, 0, 0, 1.0)
    );

    // --- the three that come out of the skill block ---

    stats!("UnitDefense", |s| s.defense(), (0, 0));
    stats!(
        "UnitAttackBothHands",
        // **Four values, of which the panel unpacks two** — there is a
        // `FIXME: The offhand stats aren't displayed yet` where the other two
        // would go, and an addon can still read them.
        |s| {
            let (main_base, main_mod) = s.weapon_skill(0);
            let (off_base, off_mod) = s.weapon_skill(1);
            (main_base, main_mod, off_base, off_mod)
        },
        (0, 0, 0, 0)
    );
    stats!("UnitRangedAttack", |s| s.ranged_skill(), (0, 0));
    stats!("HasWandEquipped", |s| one_or_nil(s.has_wand()), mlua::Value::Nil);

    // --- who the character is ---

    // **Two nils rather than one**, on the spellbook's own precedent: the pair
    // is unpacked positionally (`local race, fileName = UnitRace("player")`)
    // and a single nil would leave the second variable holding whatever the
    // previous call left in that stack slot.
    macro_rules! two_names {
        ($name:expr, $answer:ident) => {{
            let f = scope.create_function(move |_, token: Option<String>| {
                Ok(match answers.$answer(&token.unwrap_or_default()) {
                    Some((localised, file)) => (Some(localised), Some(file)),
                    None => (None, None),
                })
            })?;
            globals.set($name, f)?;
        }};
    }
    two_names!("UnitRace", unit_race);
    two_names!("UnitClass", unit_class);
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::lua::api::tests::{eval, Stub};

    /// The whole sheet's arithmetic in one line each, against a stub block —
    /// what matters here is the **arity and the order**, since every one of
    /// these is unpacked positionally by the panel.
    #[test]
    fn every_read_answers_in_the_games_own_order() {
        let world = Stub::default().stats();
        assert_eq!(
            eval(&world, "local a,b,c,d = UnitStat('player', 1); return a..','..b..','..c..','..d"),
            r#"String("60,60,10,-4")"#
        );
        assert_eq!(
            eval(
                &world,
                "local a,b,c,d = UnitResistance('player', 2); return a..','..b..','..c..','..d"
            ),
            r#"String("40,50,10,0")"#
        );
        assert_eq!(
            eval(
                &world,
                "local a,b,c,d,e = UnitArmor('player'); return a..','..b..','..c..','..d..','..e"
            ),
            r#"String("1000,1000,1000,0,0")"#
        );
        assert_eq!(
            eval(&world, "local a,b,c = UnitAttackPower('player'); return a..','..b..','..c"),
            r#"String("200,0,0")"#
        );
        // Defense is skill line 95 — 295 with a +5 permanent bonus — and the
        // "Melee Attack" line is the *sword* line, which is a different row of
        // the same block at a different number.
        assert_eq!(
            eval(&world, "local a,b = UnitDefense('player'); return a..','..b"),
            r#"String("295,5")"#
        );
        assert_eq!(
            eval(&world, "local a,b = UnitAttackBothHands('player'); return a..','..b"),
            r#"String("300,0")"#
        );
    }

    /// **A unit with no stat block answers zeroes rather than nothing**, which
    /// is the client's own absent branch — and is what keeps a panel opened at
    /// a character screen, or on a target, from failing on its first line.
    #[test]
    fn a_unit_without_a_block_answers_the_absent_shape() {
        let world = Stub::default();
        assert_eq!(
            eval(&world, "local a,b,c,d = UnitStat('target', 1); return a..','..b..','..c..','..d"),
            r#"String("0,0,0,0")"#
        );
        assert_eq!(
            eval(&world, "return tostring(UnitRace('target'))"),
            r#"String("nil")"#
        );
        // …and the damage multiplier is still 1, because the panel divides.
        assert_eq!(
            eval(&world, "local _,_,_,_,_,_,p = UnitDamage('nobody'); return tostring(p)"),
            r#"String("1")"#
        );
    }

    /// **An off-hand speed is `nil` and not `0`** without a second weapon:
    /// `if ( offhandSpeed )` is what decides whether the damage tooltip grows a
    /// second block.
    #[test]
    fn the_offhand_speed_is_nil_without_a_second_weapon() {
        let world = Stub::default().stats();
        assert_eq!(
            eval(
                &world,
                "local s,o = UnitAttackSpeed('player'); return string.format('%.2f,%s', s, tostring(o))"
            ),
            r#"String("2.90,nil")"#
        );
    }

    /// **The stat index is one-based and the resistance index is zero-based**,
    /// which is the one asymmetry in this file — both are the panel's own, and
    /// getting either wrong draws real numbers in the wrong rows.
    #[test]
    fn the_two_indices_count_from_different_places() {
        let world = Stub::default().stats();
        // Stat 1 is Strength; there is no stat 0 and no stat 6.
        assert_eq!(eval(&world, "return (UnitStat('player', 0))"), "Integer(0)");
        assert_eq!(eval(&world, "return (UnitStat('player', 6))"), "Integer(0)");
        // Resistance 0 is armour and 6 is arcane — `MagicResFrame1`'s own id.
        assert_eq!(eval(&world, "return (select(2, UnitResistance('player', 0)))"), "Integer(1000)");
        assert_eq!(eval(&world, "return (select(2, UnitResistance('player', 7)))"), "Integer(0)");
    }

    /// `UnitRace`'s second answer is the un-spaced name — `DressUpFrame.lua`
    /// compares against `"Gnome"` and `"GNOME"` on one line, and against
    /// `"NightElf"` nowhere, which is what makes the spelling worth pinning.
    #[test]
    fn the_race_and_class_answer_a_name_and_a_file_name() {
        let world = Stub::default().stats();
        assert_eq!(
            eval(&world, "local a,b = UnitRace('player'); return a..'|'..b"),
            r#"String("Night Elf|NightElf")"#
        );
        assert_eq!(
            eval(&world, "local a,b = UnitClass('player'); return a..'|'..b"),
            r#"String("Warrior|Warrior")"#
        );
    }
}
