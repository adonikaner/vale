//! **What the character sheet is made of** — the stat block, in the shapes the
//! game's own C functions answer in.
//!
//! `PaperDollFrame.lua` reads eleven numbers off the player and composes the
//! whole panel from them. Every one of those reads is a registered C function
//! returning between two and seven values, and *which update fields each of
//! them combines* is the part nothing else in this project's authority table
//! states: the server sends the fields and has no opinion about how a client
//! adds them up, and the files say nothing at all.
//!
//! So this module follows the 1.12.1 client, function by function, for these
//! Lua API names: `UnitStat`, `UnitResistance`, `UnitArmor`, `UnitAttackPower`,
//! `UnitRangedAttackPower`, `UnitAttackSpeed`, `UnitDamage`,
//! `UnitRangedDamage`, `UnitDefense`, `UnitAttackBothHands` and
//! `UnitRangedAttack`.
//!
//! ## Measured, and what is inferred
//!
//! Everything above follows the client: which field, which arithmetic, which
//! clamp, in what order the values are returned. Three things here are not,
//! and are marked where they are:
//!
//! * the three **skill**-derived answers ([`UnitStats::defense`],
//!   [`UnitStats::weapon_skill`], [`UnitStats::ranged_skill`]) reach their
//!   numbers through a virtual call on the unit that was not followed, so the
//!   *field layout* under them is vmangos' (`PLAYER_SKILL_INDEX`,
//!   `SKILL_VALUE`, `SKILL_TEMP_BONUS`) and only the **clamp** around them is
//!   the client's;
//! * the weapon a skill is looked up for goes through
//!   `ItemPrototype::GetProficiencySkill`'s table, which is vmangos';
//! * the school [`UnitStats::ranged_damage`] takes its damage-done mods from
//!   is the ranged weapon's own first damage entry in the real client, and
//!   this client has no item damage — so it uses **physical**, which is what
//!   every non-caster ranged weapon in 1.12 is anyway.
//!
//! ## Only the player has any of this
//!
//! Every field here is `UF_FLAG_PRIVATE` or `UF_FLAG_OWNER_ONLY`, so the server
//! sends them for our own character and nobody else's. The client agrees from
//! the other side: the buff halves of [`UnitStats::stat`] and
//! [`UnitStats::resistance`] are guarded by a guid comparison against the local
//! player and answer zero for anyone else. [`UnitStats::read`] therefore
//! answers `None` for a unit with no stat block at all, which is every unit but
//! one.

use crate::state::fields;
use crate::state::objects::{Entity, HeldItem};

/// The five attributes, in the order `SPELL_STAT0_NAME`..`4` names them:
/// Strength, Agility, Stamina, Intellect, Spirit.
pub const NUM_STATS: usize = 5;

/// `UNIT_FIELD_RESISTANCES`' seven schools. **0 is armour** — `RESISTANCE0_NAME`
/// in `GlobalStrings.lua` is the word "Armor", and the client keeps the index in
/// a global of its own rather than writing 0 into `UnitArmor`.
pub const NUM_RESISTANCES: usize = 7;

/// The armour school. See [`NUM_RESISTANCES`].
pub const ARMOR_SCHOOL: usize = 0;

/// `SKILL_DEFENSE` — `SkillLine.dbc` row 95, and the only skill line this
/// module needs by name.
pub const SKILL_DEFENSE: u16 = 95;

/// `SKILL_UNARMED` — what a hand with nothing in it swings with.
pub const SKILL_UNARMED: u16 = 162;

/// `PLAYER_SKILL_INFO_1_1` holds 128 of these triplets.
const NUM_SKILL_SLOTS: u16 = 128;

/// An attack-power triple: the value, and the two halves of its modifier.
///
/// **The mods are two `int16`s inside one field** (`UF_TYPE_TWO_SHORT`), read
/// as two signed halves, and the multiplier scales all three — see [`UnitStats::attack_power`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Power {
    value: i32,
    positive: i16,
    negative: i16,
    multiplier: f32,
}

impl Power {
    fn read(field: &impl Fn(u16) -> Option<u32>, value: u16) -> Self {
        let mods = field(value + 1).unwrap_or(0);
        Self {
            value: field(value).unwrap_or(0) as i32,
            positive: mods as u16 as i16,
            negative: (mods >> 16) as u16 as i16,
            multiplier: field(value + 2).map_or(0.0, f32::from_bits),
        }
    }

    /// `(base, posBuff, negBuff)`, each scaled by `1 + multiplier` and
    /// converted to an integer — the client does the same on all three.
    fn triple(&self) -> (i32, i32, i32) {
        let scale = 1.0 + self.multiplier;
        let scaled = |n: i32| (scale * n as f32).round() as i32;
        (
            scaled(self.value),
            scaled(i32::from(self.positive)),
            scaled(i32::from(self.negative)),
        )
    }
}

/// One skill line's value and the bonus on top of it.
///
/// `PLAYER_SKILL_INFO_1_1 + slot*3` is `(id | step<<16)`, `+1` is
/// `(value | max<<16)` and `+2` is `(temporary | permanent<<16)` as two signed
/// shorts. The **modifier is the sum of the two bonuses**, which is what the
/// panel prints in green beside the base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Skill {
    value: i32,
    modifier: i32,
}

impl Skill {
    /// The skill's `(base, modifier)` with the client's own clamp: a modifier
    /// that would take the total below zero is replaced by `-base`, so the two
    /// always sum to something the panel can print. `UnitDefense` and
    /// `UnitAttackBothHands` apply the same clamp.
    fn pair(&self) -> (i32, i32) {
        if self.value + self.modifier < 0 {
            (self.value, -self.value)
        } else {
            (self.value, self.modifier)
        }
    }
}

/// `UnitDamage`'s seven answers, named the way `PaperDollFrame_SetDamage`
/// unpacks them.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Damage {
    pub min: f32,
    pub max: f32,
    pub min_offhand: f32,
    pub max_offhand: f32,
    /// `PLAYER_FIELD_MOD_DAMAGE_DONE_POS` for the school, as an integer.
    pub bonus_positive: i32,
    pub bonus_negative: i32,
    /// `…_PCT`, and **1.0 rather than 0.0 when it is absent** — the panel
    /// divides by it on its second line.
    pub percent: f32,
}

/// `UnitRangedDamage`'s six, in its own order — the speed comes *first* here
/// and last in [`Damage`], which is the game's own inconsistency and not one
/// introduced by this shape.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RangedDamage {
    pub speed: f32,
    pub min: f32,
    pub max: f32,
    pub bonus_positive: i32,
    pub bonus_negative: i32,
    pub percent: f32,
}

/// Everything the paper doll asks about one unit, decoded once.
///
/// A snapshot rather than a view: it is read where the world lock is already
/// held and answered from wherever the interface asks. ~320 bytes, `Copy`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitStats {
    stats: [i32; NUM_STATS],
    stat_positive: [i32; NUM_STATS],
    stat_negative: [i32; NUM_STATS],
    resistances: [i32; NUM_RESISTANCES],
    resist_positive: [i32; NUM_RESISTANCES],
    resist_negative: [i32; NUM_RESISTANCES],
    melee: Power,
    ranged: Power,
    /// `UNIT_FIELD_BASEATTACKTIME` and the off hand's, in milliseconds.
    attack_time_ms: [u32; 2],
    ranged_attack_time_ms: u32,
    damage: [f32; 4],
    ranged_damage: [f32; 2],
    mod_damage_positive: [i32; NUM_RESISTANCES],
    mod_damage_negative: [i32; NUM_RESISTANCES],
    mod_damage_percent: [f32; NUM_RESISTANCES],
    defense: Skill,
    /// Main hand, off hand — the skill of whatever is in it, or Unarmed.
    weapon: [Skill; 2],
    ranged_skill: Skill,
    /// Whether there is a second weapon to answer an off-hand speed for. See
    /// [`UnitStats::attack_speed`], where the absence is `nil` and not zero.
    dual_wielding: bool,
    /// Whether the ranged slot holds a wand — `HasWandEquipped`, which is a
    /// question about the same three items and belongs beside them.
    wand: bool,
}

impl UnitStats {
    /// Decode, or `None` for a unit whose stat block the server never sent —
    /// which is every unit but the local player. See the module comment.
    ///
    /// `weapons` is [`crate::state::objects::ObjectManager::weapons_of`]'s answer,
    /// which is where the three skill lines and the off-hand test come from:
    /// the *fields* say what a skill is worth and the *item* says which skill
    /// to look up.
    pub fn read(entity: &Entity, weapons: &[HeldItem; 3]) -> Option<UnitStats> {
        Self::decode(|index| entity.field(index), weapons)
    }

    /// The same, from bare `(index, value)` pairs.
    ///
    /// For **tests on the far side of the crate boundary**: an [`Entity`] is
    /// the object manager's to make, and `lua::paperdoll`'s checks want a real
    /// block rather than a hand-written double of one. Same decode, so a test
    /// written against this cannot pass while the client's read fails.
    pub fn from_fields(fields: &[(u16, u32)], weapons: &[HeldItem; 3]) -> Option<UnitStats> {
        Self::decode(
            |index| {
                fields
                    .iter()
                    .find(|(at, _)| *at == index)
                    .map(|(_, value)| *value)
            },
            weapons,
        )
    }

    /// The decode itself, over anything that can answer a field.
    fn decode(
        field: impl Fn(u16) -> Option<u32>,
        weapons: &[HeldItem; 3],
    ) -> Option<UnitStats> {
        // The presence test is the first stat: the whole block arrives in one
        // update and a unit with none of it is not one this can answer about.
        field(fields::unit::STAT0)?;

        let int = |index: u16| field(index).unwrap_or(0) as i32;
        let float = |index: u16| f32::from_bits(field(index).unwrap_or(0));
        let mut out = UnitStats {
            stats: [0; NUM_STATS],
            stat_positive: [0; NUM_STATS],
            stat_negative: [0; NUM_STATS],
            resistances: [0; NUM_RESISTANCES],
            resist_positive: [0; NUM_RESISTANCES],
            resist_negative: [0; NUM_RESISTANCES],
            melee: Power::read(&field, fields::unit::ATTACK_POWER),
            ranged: Power::read(&field, fields::unit::RANGED_ATTACK_POWER),
            attack_time_ms: [
                field(fields::unit::BASEATTACKTIME).unwrap_or(0),
                field(fields::unit::BASEATTACKTIME + 1).unwrap_or(0),
            ],
            ranged_attack_time_ms: field(fields::unit::RANGEDATTACKTIME).unwrap_or(0),
            damage: [
                float(fields::unit::MINDAMAGE),
                float(fields::unit::MAXDAMAGE),
                float(fields::unit::MINOFFHANDDAMAGE),
                float(fields::unit::MAXOFFHANDDAMAGE),
            ],
            ranged_damage: [
                float(fields::unit::MINRANGEDDAMAGE),
                float(fields::unit::MAXRANGEDDAMAGE),
            ],
            mod_damage_positive: [0; NUM_RESISTANCES],
            mod_damage_negative: [0; NUM_RESISTANCES],
            // **1.0, not 0.0.** `PaperDollFrame_SetDamage` divides by it, and
            // the client answers a literal 1.0 where the field is not there.
            mod_damage_percent: [1.0; NUM_RESISTANCES],
            defense: Skill::default(),
            weapon: [Skill::default(); 2],
            ranged_skill: Skill::default(),
            dual_wielding: weapons[1].class == ITEM_CLASS_WEAPON,
            wand: weapons[2].class == ITEM_CLASS_WEAPON
                && weapons[2].subclass == ITEM_SUBCLASS_WAND,
        };

        for i in 0..NUM_STATS {
            let i16 = i as u16;
            out.stats[i] = int(fields::unit::STAT0 + i16);
            out.stat_positive[i] = int(fields::player::POSSTAT0 + i16);
            out.stat_negative[i] = int(fields::player::NEGSTAT0 + i16);
        }
        for i in 0..NUM_RESISTANCES {
            let i16 = i as u16;
            out.resistances[i] = int(fields::unit::RESISTANCES + i16);
            out.resist_positive[i] = int(fields::player::RESISTANCEBUFFMODSPOSITIVE + i16);
            out.resist_negative[i] = int(fields::player::RESISTANCEBUFFMODSNEGATIVE + i16);
            out.mod_damage_positive[i] = int(fields::player::MOD_DAMAGE_DONE_POS + i16);
            out.mod_damage_negative[i] = int(fields::player::MOD_DAMAGE_DONE_NEG + i16);
            out.mod_damage_percent[i] = field(fields::player::MOD_DAMAGE_DONE_PCT + i16)
                .map_or(1.0, f32::from_bits);
        }

        out.defense = read_skill(&field, SKILL_DEFENSE);
        for (skill, held) in out.weapon.iter_mut().zip(weapons) {
            *skill = read_skill(&field, proficiency_skill(held));
        }
        out.ranged_skill = read_skill(&field, proficiency_skill(&weapons[2]));
        Some(out)
    }

    /// `UnitStat(unit, i)` — `(stat, effectiveStat, posBuff, negBuff)`, with
    /// `i` **one-based** as the panel's `for i=1, NUM_STATS` passes it.
    ///
    /// `stat` is the field verbatim and `effectiveStat` is the same number
    /// clamped at zero; the panel prints the
    /// second and reconstructs the base as `stat - posBuff - negBuff`.
    pub fn stat(&self, index: usize) -> Option<(i32, i32, i32, i32)> {
        let i = index.checked_sub(1).filter(|i| *i < NUM_STATS)?;
        Some((
            self.stats[i],
            self.stats[i].max(0),
            self.stat_positive[i],
            self.stat_negative[i],
        ))
    }

    /// `UnitResistance(unit, school)` — `(base, resistance, positive,
    /// negative)`, `school` **zero-based** and 0..=6 (`MagicResFrame1` carries
    /// `id="6"`, arcane).
    ///
    /// The base is *derived*: `total - positive - negative`, as the client
    /// computes it. `negative` is a
    /// negative number, which is why the panel adds rather than subtracts it.
    pub fn resistance(&self, school: usize) -> Option<(i32, i32, i32, i32)> {
        if school >= NUM_RESISTANCES {
            return None;
        }
        let (positive, negative) = (self.resist_positive[school], self.resist_negative[school]);
        let total = self.resistances[school];
        Some((total - positive - negative, total.max(0), positive, negative))
    }

    /// `UnitArmor` — `(base, effectiveArmor, armor, posBuff, negBuff)`, which
    /// is [`UnitStats::resistance`] of school 0 with the total **returned
    /// twice**: the helper writes it into two out-parameters and 1.12 has
    /// nothing that makes them differ.
    pub fn armor(&self) -> (i32, i32, i32, i32, i32) {
        let (base, total, positive, negative) = self
            .resistance(ARMOR_SCHOOL)
            .expect("school 0 is in range");
        (base, total, total, positive, negative)
    }

    /// `UnitAttackPower` — `(base, posBuff, negBuff)`, all three scaled by
    /// `1 + UNIT_FIELD_ATTACK_POWER_MULTIPLIER`.
    pub fn attack_power(&self) -> (i32, i32, i32) {
        self.melee.triple()
    }

    /// `UnitRangedAttackPower`, on exactly the same three fields one array
    /// along.
    pub fn ranged_attack_power(&self) -> (i32, i32, i32) {
        self.ranged.triple()
    }

    /// `UnitAttackSpeed` — `(speed, offhandSpeed)` in **seconds**, the field's
    /// milliseconds times `0.001`, as the client does it.
    ///
    /// The second is `None` — a Lua `nil`, not a zero — for a unit that is not
    /// dual-wielding, because `PaperDollFrame_SetDamage` tests it with
    /// `if ( offhandSpeed )` and prints a whole second block if it is there.
    pub fn attack_speed(&self) -> (f32, Option<f32>) {
        let seconds = |ms: u32| ms as f32 * 0.001;
        (
            seconds(self.attack_time_ms[0]),
            self.dual_wielding.then(|| seconds(self.attack_time_ms[1])),
        )
    }

    /// `UnitDamage` — the four weapon damages off the unit's own fields, plus
    /// the three **physical** damage-done mods off the player's.
    pub fn damage(&self) -> Damage {
        Damage {
            min: self.damage[0],
            max: self.damage[1],
            min_offhand: self.damage[2],
            max_offhand: self.damage[3],
            bonus_positive: self.mod_damage_positive[PHYSICAL_SCHOOL],
            bonus_negative: self.mod_damage_negative[PHYSICAL_SCHOOL],
            percent: self.mod_damage_percent[PHYSICAL_SCHOOL],
        }
    }

    /// `UnitRangedDamage` — the ranged speed, its two damages, and the mods for
    /// **the school the ranged weapon deals**, which this client takes as
    /// physical. See the module comment on what that costs.
    pub fn ranged_damage(&self) -> RangedDamage {
        RangedDamage {
            speed: self.ranged_attack_time_ms as f32 * 0.001,
            min: self.ranged_damage[0],
            max: self.ranged_damage[1],
            bonus_positive: self.mod_damage_positive[PHYSICAL_SCHOOL],
            bonus_negative: self.mod_damage_negative[PHYSICAL_SCHOOL],
            percent: self.mod_damage_percent[PHYSICAL_SCHOOL],
        }
    }

    /// `UnitDefense` — `(base, modifier)` off skill line 95.
    pub fn defense(&self) -> (i32, i32) {
        self.defense.pair()
    }

    /// `UnitAttackBothHands` — one hand's `(base, modifier)`; the game returns
    /// both hands' four values and `PaperDollFrame_SetAttackBothHands` unpacks
    /// two, with a `FIXME` beside it.
    pub fn weapon_skill(&self, hand: usize) -> (i32, i32) {
        self.weapon.get(hand).copied().unwrap_or_default().pair()
    }

    /// `UnitRangedAttack` — the same for the ranged slot. **No clamp**: the
    /// client's ranged path hands the pair straight back where the
    /// other two correct it, and this keeps the difference.
    pub fn ranged_skill(&self) -> (i32, i32) {
        (self.ranged_skill.value, self.ranged_skill.modifier)
    }

    /// `HasWandEquipped` — the ranged slot holds an `ITEM_SUBCLASS_WEAPON_WAND`,
    /// which is what blanks the ranged attack power rather than showing it.
    pub fn has_wand(&self) -> bool {
        self.wand
    }
}

/// `SPELL_SCHOOL_NORMAL` — physical, and index 0 of every one of the three
/// damage-done arrays as well as of the resistances.
const PHYSICAL_SCHOOL: usize = 0;

/// `ITEM_CLASS_WEAPON`, and the wand's subclass within it.
const ITEM_CLASS_WEAPON: u8 = 2;
const ITEM_SUBCLASS_WAND: u8 = 19;

/// Which skill line swings an item — `ItemPrototype::GetProficiencySkill`'s
/// table, verbatim, with an empty hand answering Unarmed.
///
/// **vmangos', not the client's** (see the module comment): the client reaches
/// the same place through a virtual call that was not followed. The two
/// agree about every weapon a player can hold in 1.12, which is what makes the
/// substitution safe rather than merely convenient — the table is a property of
/// the item data both sides read.
fn proficiency_skill(item: &HeldItem) -> u16 {
    if item.class != ITEM_CLASS_WEAPON {
        // Nothing, or a shield — either way the hand swings unarmed.
        return SKILL_UNARMED;
    }
    // Axes, 2H axes, bows, guns, maces, 2H maces, polearms, swords, 2H swords,
    // (unused), staves, (unused), (unused), fist weapons, (misc), daggers,
    // thrown, spears, crossbows, wands, fishing poles.
    const BY_SUBCLASS: [u16; 21] = [
        44, 172, 45, 46, 54, 160, 229, 43, 55, 0, 136, 0, 0, SKILL_UNARMED, 0, 173, 176, 253, 226,
        228, 356,
    ];
    BY_SUBCLASS
        .get(usize::from(item.subclass))
        .copied()
        .unwrap_or(0)
}

/// One skill line's slot out of `PLAYER_SKILL_INFO_1_1`, or a zeroed [`Skill`]
/// for a line the character does not have.
///
/// A linear walk of 128 slots rather than an index: the slots are **not**
/// ordered and a skill's position is wherever the server first wrote it
/// (`Player::SetSkill` takes the first empty one), so there is nothing to index
/// by. It runs once per read of the whole block, not once per question.
fn read_skill(field: &impl Fn(u16) -> Option<u32>, line: u16) -> Skill {
    if line == 0 {
        return Skill::default();
    }
    for slot in 0..NUM_SKILL_SLOTS {
        let base = fields::player::SKILL_INFO_1_1 + slot * 3;
        let Some(id) = field(base) else {
            continue;
        };
        if id as u16 != line {
            continue;
        }
        let value = field(base + 1).unwrap_or(0);
        let bonus = field(base + 2).unwrap_or(0);
        return Skill {
            value: i32::from(value as u16),
            modifier: i32::from(bonus as u16 as i16) + i32::from((bonus >> 16) as u16 as i16),
        };
    }
    Skill::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weapon(subclass: u8) -> HeldItem {
        HeldItem { display_id: 1, class: ITEM_CLASS_WEAPON, subclass, ..HeldItem::default() }
    }

    const NONE: [HeldItem; 3] = [HeldItem {
        display_id: 0,
        class: 0,
        subclass: 0,
        material: 0,
        inventory_type: 0,
        sheath: 0,
    }; 3];

    /// **A unit with no stat block answers nothing**, which is every unit but
    /// the local player — the fields are private and the server never sends
    /// them. Answering zeroes would draw a character sheet full of them.
    #[test]
    fn a_unit_without_the_block_has_no_stats() {
        assert!(UnitStats::from_fields(&[], &NONE).is_none());
        assert!(UnitStats::from_fields(&[(fields::unit::HEALTH, 100)], &NONE).is_none());
        assert!(UnitStats::from_fields(&[(fields::unit::STAT0, 20)], &NONE).is_some());
    }

    /// `UnitStat`'s four, and the panel's own reconstruction of the base from
    /// them: `stat - posBuff - negBuff` has to come back to the unbuffed
    /// number, which is the arithmetic every stat tooltip prints.
    #[test]
    fn a_stat_carries_its_two_buff_halves() {
        let stats = UnitStats::from_fields(
            &[
                (fields::unit::STAT0, 60),
                (fields::unit::STAT0 + 1, 40),
                (fields::player::POSSTAT0, 15),
                (fields::player::NEGSTAT0, (-5i32) as u32),
            ],
            &NONE,
        )
        .expect("has a block");

        assert_eq!(stats.stat(1), Some((60, 60, 15, -5)));
        let (stat, effective, positive, negative) = stats.stat(1).unwrap();
        assert_eq!(stat - positive - negative, 50, "the panel's own base");
        assert_eq!(effective, 60);
        // A stat with no buffs on it, and one past the end.
        assert_eq!(stats.stat(2), Some((40, 40, 0, 0)));
        assert_eq!(stats.stat(6), None);
        assert_eq!(stats.stat(0), None, "the panel counts from 1");
    }

    /// **`effectiveStat` is clamped at zero and `stat` is not**, as in the
    /// client.
    #[test]
    fn a_negative_stat_is_clamped_only_in_the_second_answer() {
        let stats = UnitStats::from_fields(
            &[(fields::unit::STAT0, (-3i32) as u32)],
            &NONE,
        )
        .unwrap();
        assert_eq!(stats.stat(1), Some((-3, 0, 0, 0)));
    }

    /// `UnitResistance`'s base is **derived**, not stored: the client
    /// subtracts both buff halves off the total.
    #[test]
    fn a_resistance_derives_its_base_from_the_total() {
        let stats = UnitStats::from_fields(
            &[
                (fields::unit::STAT0, 1),
                (fields::unit::RESISTANCES + 2, 90),
                (fields::player::RESISTANCEBUFFMODSPOSITIVE + 2, 30),
                (fields::player::RESISTANCEBUFFMODSNEGATIVE + 2, (-10i32) as u32),
            ],
            &NONE,
        )
        .unwrap();
        assert_eq!(stats.resistance(2), Some((70, 90, 30, -10)));
        assert_eq!(stats.resistance(7), None, "seven schools, 0..=6");
    }

    /// **Armour is resistance school 0**, and `UnitArmor` returns the total
    /// twice — `GlobalStrings.lua`'s `RESISTANCE0_NAME` is the word "Armor",
    /// which is the other half of that identification.
    #[test]
    fn armour_is_the_first_resistance_returned_twice() {
        let stats = UnitStats::from_fields(
            &[
                (fields::unit::STAT0, 1),
                (fields::unit::RESISTANCES, 1200),
                (fields::player::RESISTANCEBUFFMODSPOSITIVE, 200),
            ],
            &NONE,
        )
        .unwrap();
        assert_eq!(stats.armor(), (1000, 1200, 1200, 200, 0));
    }

    /// **The attack-power mods are two shorts in one field**, and the
    /// multiplier scales all three answers.
    #[test]
    fn attack_power_unpacks_two_shorts_and_scales_by_the_multiplier() {
        let mods = (100u32 & 0xFFFF) | (((-40i32) as u32 & 0xFFFF) << 16);
        let stats = UnitStats::from_fields(
            &[
                (fields::unit::STAT0, 1),
                (fields::unit::ATTACK_POWER, 500),
                (fields::unit::ATTACK_POWER_MODS, mods),
                (fields::unit::ATTACK_POWER_MULTIPLIER, 0.25f32.to_bits()),
            ],
            &NONE,
        )
        .unwrap();
        assert_eq!(stats.attack_power(), (625, 125, -50));
        // No multiplier field at all is a scale of 1, not of 0.
        let plain = UnitStats::from_fields(
            &[(fields::unit::STAT0, 1), (fields::unit::ATTACK_POWER, 500)],
            &NONE,
        )
        .unwrap();
        assert_eq!(plain.attack_power(), (500, 0, 0));
    }

    /// **An off-hand speed is absent rather than zero** when there is no second
    /// weapon: `PaperDollFrame_SetDamage` tests `if ( offhandSpeed )` and draws
    /// a whole extra tooltip block when it is there.
    #[test]
    fn the_offhand_speed_is_absent_without_a_second_weapon() {
        let fields = [
            (fields::unit::STAT0, 1),
            (fields::unit::BASEATTACKTIME, 2900),
            (fields::unit::BASEATTACKTIME + 1, 1800),
        ];
        let alone = UnitStats::from_fields(&fields, &NONE).unwrap();
        assert_eq!(alone.attack_speed(), (2.9, None));

        let mut dual = NONE;
        dual[1] = weapon(15);
        let both = UnitStats::from_fields(&fields, &dual).unwrap();
        // …to within the `ms * 0.001f` the client itself does: 1800 of them
        // come back as 1.8000001, and the panel prints `%.2f`.
        let (_, offhand) = both.attack_speed();
        assert!(offhand.is_some_and(|s| (s - 1.8).abs() < 1e-5), "{offhand:?}");
    }

    /// **The damage percent defaults to 1.0**, because the panel divides by it
    /// — a zero there is every damage number on the sheet reading `inf`.
    #[test]
    fn the_damage_multiplier_is_one_when_the_field_is_absent() {
        let stats = UnitStats::from_fields(
            &[
                (fields::unit::STAT0, 1),
                (fields::unit::MINDAMAGE, 40.5f32.to_bits()),
                (fields::unit::MAXDAMAGE, 60.5f32.to_bits()),
            ],
            &NONE,
        )
        .unwrap();
        let damage = stats.damage();
        assert_eq!((damage.min, damage.max), (40.5, 60.5));
        assert_eq!(damage.percent, 1.0);
        assert_eq!(stats.ranged_damage().percent, 1.0);
    }

    /// A skill is found by **walking** the block, because the slots are in the
    /// order the server first wrote them and not in id order.
    #[test]
    fn a_skill_is_found_wherever_the_server_put_it() {
        let slot = |n: u16| fields::player::SKILL_INFO_1_1 + n * 3;
        let stats = UnitStats::from_fields(
            &[
                (fields::unit::STAT0, 1),
                // Slot 0 is something else entirely.
                (slot(0), 6),
                (slot(0) + 1, 300 | (300 << 16)),
                // Defense in slot 7, at 295 with a +10 permanent bonus.
                (slot(7), u32::from(SKILL_DEFENSE)),
                (slot(7) + 1, 295 | (300 << 16)),
                (slot(7) + 2, 10 << 16),
            ],
            &NONE,
        )
        .unwrap();
        assert_eq!(stats.defense(), (295, 10));
    }

    /// **The two bonus halves add**, and either may be negative.
    #[test]
    fn a_skill_bonus_is_the_temporary_plus_the_permanent() {
        let slot = fields::player::SKILL_INFO_1_1;
        let bonus = ((-5i32) as u32 & 0xFFFF) | (12u32 << 16);
        let stats = UnitStats::from_fields(
            &[
                (fields::unit::STAT0, 1),
                (slot, u32::from(SKILL_DEFENSE)),
                (slot + 1, 100),
                (slot + 2, bonus),
            ],
            &NONE,
        )
        .unwrap();
        assert_eq!(stats.defense(), (100, 7));
    }

    /// **A modifier that would take a skill below zero is replaced by
    /// `-base`** — the client's own clamp, so that the panel's
    /// `base + modifier` never prints a negative skill.
    #[test]
    fn a_skill_modifier_cannot_take_the_total_below_zero() {
        let slot = fields::player::SKILL_INFO_1_1;
        let stats = UnitStats::from_fields(
            &[
                (fields::unit::STAT0, 1),
                (slot, u32::from(SKILL_DEFENSE)),
                (slot + 1, 20),
                (slot + 2, (-50i32) as u32 & 0xFFFF),
            ],
            &NONE,
        )
        .unwrap();
        let (base, modifier) = stats.defense();
        assert_eq!((base, modifier), (20, -20));
        assert_eq!(base + modifier, 0);
    }

    /// **Which skill a hand swings with is the item's**, and an empty hand — or
    /// a shield — is Unarmed. This is the join the character sheet's "Melee
    /// Attack" line stands on.
    #[test]
    fn a_hand_swings_with_its_own_weapons_skill() {
        assert_eq!(proficiency_skill(&weapon(0)), 44, "axes");
        assert_eq!(proficiency_skill(&weapon(7)), 43, "swords");
        assert_eq!(proficiency_skill(&weapon(15)), 173, "daggers");
        assert_eq!(proficiency_skill(&weapon(13)), SKILL_UNARMED, "fist weapons");
        assert_eq!(proficiency_skill(&weapon(19)), 228, "wands");
        assert_eq!(proficiency_skill(&HeldItem::default()), SKILL_UNARMED);
        // A shield is `ITEM_CLASS_ARMOR`, and the hand holding it is empty as
        // far as a swing is concerned.
        let shield = HeldItem { display_id: 5, class: 4, subclass: 6, ..HeldItem::default() };
        assert_eq!(proficiency_skill(&shield), SKILL_UNARMED);
        // A subclass past the table is no skill rather than a panic.
        assert_eq!(proficiency_skill(&weapon(200)), 0);
    }

    /// The whole join: a sword in the main hand reads the *sword* line out of
    /// the block, and a dagger would read a different one.
    #[test]
    fn the_weapon_skill_read_follows_the_weapon() {
        let slot = |n: u16| fields::player::SKILL_INFO_1_1 + n * 3;
        let fields = [
            (fields::unit::STAT0, 1),
            (slot(0), 43),
            (slot(0) + 1, 300 | (300 << 16)),
            (slot(1), 173),
            (slot(1) + 1, 137 | (300 << 16)),
        ];
        let mut held = NONE;
        held[0] = weapon(7);
        assert_eq!(UnitStats::from_fields(&fields, &held).unwrap().weapon_skill(0), (300, 0));
        held[0] = weapon(15);
        assert_eq!(UnitStats::from_fields(&fields, &held).unwrap().weapon_skill(0), (137, 0));
    }

    /// `HasWandEquipped` is a question about the ranged slot's subclass.
    #[test]
    fn a_wand_is_recognised_in_the_ranged_slot() {
        let fields = [(fields::unit::STAT0, 1)];
        let mut held = NONE;
        held[2] = weapon(19);
        assert!(UnitStats::from_fields(&fields, &held).unwrap().has_wand());
        held[2] = weapon(2);
        assert!(!UnitStats::from_fields(&fields, &held).unwrap().has_wand(), "a bow is not a wand");
    }
}
