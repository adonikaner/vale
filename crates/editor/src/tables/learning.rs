//! The rows that put a spell in a skill line, that give a skill line to races
//! and classes, and the spell that teaches a spell: found and made from the
//! form of the row they are about.

use crate::session::EditSession;
use bevy::prelude::*;
use super::browser::Browser;
use super::rows::{push_row, rows_naming};

// ---------------------------------------------------------------------------
// Rows of one table edited from another table's form
// ---------------------------------------------------------------------------

/// `SkillLineAbility.dbc`'s fields, as the spell form's Learning section and
/// the skill line form read them. The indices are
/// `schema::SKILL_LINE_ABILITY`'s.
pub mod ability {
    pub const TABLE: &str = "SkillLineAbility";
    pub const SKILL: usize = 1;
    pub const SPELL: usize = 2;
    pub const RACES: usize = 3;
    pub const CLASSES: usize = 4;
    pub const REQ_SKILL_VALUE: usize = 7;
    pub const SUPERSEDED_BY: usize = 8;
    pub const LEARN_ON_GET_SKILL: usize = 9;
    pub const MAX_VALUE: usize = 10;
    pub const MIN_VALUE: usize = 11;
    pub const REQ_TRAIN_POINTS: usize = 14;
}

/// `SkillRaceClassInfo.dbc`'s fields, as the skill line form reads them. The
/// indices are `schema::SKILL_RACE_CLASS_INFO`'s.
pub mod race_class {
    pub const TABLE: &str = "SkillRaceClassInfo";
    pub const SKILL: usize = 1;
    pub const RACES: usize = 2;
    pub const CLASSES: usize = 3;
    pub const FLAGS: usize = 4;
    pub const MIN_LEVEL: usize = 5;
    pub const SKILL_TIER: usize = 6;
    /// Every race: 134 of the 201 shipped rows hold it.
    pub const EVERY_RACE: u32 = 511;
    /// Every class: 60 of the shipped rows hold it.
    pub const EVERY_CLASS: u32 = 1503;
}

/// The `SkillLineAbility` rows that put `spell` in a skill line. Most spells
/// have none: 4,753 of the shipped spells are named by a row.
pub fn abilities_of(browser: &mut Browser, session: &EditSession, spell: u32) -> Vec<usize> {
    rows_naming(browser, session, "Spell", spell, ability::TABLE, ability::SPELL)
}

/// The `SkillLineAbility` rows of one skill line.
pub fn abilities_in(browser: &mut Browser, session: &EditSession, skill: u32) -> Vec<usize> {
    rows_naming(browser, session, "SkillLine", skill, ability::TABLE, ability::SKILL)
}

/// The `SkillRaceClassInfo` rows that give `skill` to races and classes.
pub fn race_class_rows_of(browser: &mut Browser, session: &EditSession, skill: u32) -> Vec<usize> {
    rows_naming(browser, session, "SkillLine", skill, race_class::TABLE, race_class::SKILL)
}

/// `SpellCastTimes.dbc`'s row for an instant cast, which is what both mage
/// trainer lists' teaching spells use; see `crate::subjects::services::teaching_spell`.
pub(super) const INSTANT_CAST: u32 = 1;

/// The teaching spells of `spell`, as the record of each and whether it is
/// instant: the rows of `Spell.dbc` whose first effect is `LEARN_SPELL` and
/// whose `EffectTriggerSpell 1` is `spell`. A trainer's list names one of
/// these and not the spell itself.
///
/// 4,183 of the shipped spells have one and 511 of those have more than one,
/// usually an instant one a trainer's list names beside one with a cast time
/// that a book casts.
pub fn teachers_of(browser: &mut Browser, session: &EditSession, spell: u32) -> Vec<(usize, bool)> {
    use vale_assets::tables::spellbook::spell_fields;
    let rows = rows_naming(
        browser,
        session,
        "Spell",
        spell,
        "Spell",
        spell_fields::EFFECT_TRIGGER_SPELL,
    );
    let Some(spells) = session.table("Spell") else {
        return Vec::new();
    };
    rows.into_iter()
        .filter(|&record| {
            spells.u32_at(record, spell_fields::EFFECT) == Some(vale_mangos::trainer::LEARN_SPELL)
        })
        .map(|record| {
            let instant = spells.u32_at(record, spell_fields::CASTING_TIME_INDEX) == Some(INSTANT_CAST);
            (record, instant)
        })
        .collect()
}

/// Put `spell` in a skill line: a new `SkillLineAbility` row naming it, as
/// one undo entry. The skill line is not chosen here; the form opens the
/// picker on the new row. `ReqSkillValue` starts at 1, which 5,026 of the
/// 5,072 shipped rows hold.
pub fn add_ability(session: &mut EditSession, spell: u32) -> Option<usize> {
    session.history.begin("Add to a skill line");
    let at = push_row(
        session,
        ability::TABLE,
        &[(ability::SPELL, spell), (ability::REQ_SKILL_VALUE, 1)],
        &[],
    );
    session.history.end();
    at
}

/// Give `skill` to races and classes: a new `SkillRaceClassInfo` row naming
/// it, for every race and every class, as one undo entry. The flags start at
/// zero, which lists the line normally and gives it a spellbook tab.
pub fn add_race_class_row(session: &mut EditSession, skill: u32) -> Option<usize> {
    session.history.begin("Add race/class row");
    let at = push_row(
        session,
        race_class::TABLE,
        &[
            (race_class::SKILL, skill),
            (race_class::RACES, race_class::EVERY_RACE),
            (race_class::CLASSES, race_class::EVERY_CLASS),
        ],
        &[],
    );
    session.history.end();
    at
}

/// What a new teaching spell holds besides its name, rank, icon and the spell
/// it teaches, as `(field, value)`. Each value is the one most of the 3,408
/// shipped instant teaching spells hold, with the count beside it; a field
/// not listed is zero on most of them.
const TEACHING_SPELL: [(usize, u32); 12] = [
    // Attributes: 3,265.
    (6, 0x0004_0100),
    // Targets: 3,194.
    (13, 0x100),
    // CastingTimeIndex, instant: all of them, by the population's definition.
    (18, INSTANT_CAST),
    // ProcChance: 3,381.
    (25, 101),
    // RangeIndex: 3,350.
    (36, 6),
    // EquippedItemClass, none: all 3,408.
    (58, u32::MAX),
    // EquippedItemSubClassMask: 2,959.
    (59, u32::MAX),
    // Effect 1.
    (61, vale_mangos::trainer::LEARN_SPELL),
    // SpellVisual, the learning visual: 3,390.
    (115, 107),
    // StanceBarOrder, none: 3,397.
    (166, u32::MAX),
    // DmgMultiplier 1 and 2, 1.0: 3,408 and 3,397.
    (167, 0x3F80_0000),
    (168, 0x3F80_0000),
];

/// `DmgMultiplier 3`, which is 1.0 on 3,397 of the shipped instant teaching
/// spells, as the other two are.
const TEACHING_SPELL_MULTIPLIER_3: (usize, u32) = (169, 0x3F80_0000);

/// Make the spell that teaches `spell`: a new `Spell.dbc` row whose first
/// effect is `LEARN_SPELL` and whose `EffectTriggerSpell 1` is `spell`, as one
/// undo entry. Returns the new spell's id, or `None` when `spell` is not a row
/// of the open table.
///
/// The row takes the taught spell's name, rank and icon, which is what the
/// shipped ones do (2,958, 3,126 and 2,748 of the 3,392 whose taught spell
/// exists), and [`TEACHING_SPELL`] for the rest.
pub fn add_teaching_spell(session: &mut EditSession, spell: u32) -> Option<u32> {
    use vale_assets::tables::spellbook::spell_fields;
    let spells = session.table("Spell")?;
    let taught = spells.row_of(spell)?;
    let name = spells.string_at(taught, spell_fields::NAME).unwrap_or_default();
    let rank = spells.string_at(taught, spell_fields::RANK).unwrap_or_default();
    let icon = spells.u32_at(taught, spell_fields::ICON_ID).unwrap_or(0);
    // The locale flag words after the name and the rank, copied so the new
    // row marks the same locales as present.
    let name_flags = spells.u32_at(taught, spell_fields::NAME + 8).unwrap_or(0);
    let rank_flags = spells.u32_at(taught, spell_fields::RANK + 8).unwrap_or(0);

    let mut numbers: Vec<(usize, u32)> = TEACHING_SPELL.to_vec();
    numbers.push(TEACHING_SPELL_MULTIPLIER_3);
    numbers.push((spell_fields::EFFECT_TRIGGER_SPELL, spell));
    numbers.push((spell_fields::ICON_ID, icon));
    numbers.push((spell_fields::NAME + 8, name_flags));
    numbers.push((spell_fields::RANK + 8, rank_flags));
    let mut texts = vec![(spell_fields::NAME, name)];
    if !rank.is_empty() {
        texts.push((spell_fields::RANK, rank));
    }

    session.history.begin("Create teaching spell");
    let at = push_row(session, "Spell", &numbers, &texts);
    session.history.end();
    session.table("Spell")?.u32_at(at?, 0)
}
