//! What a trainer's list may name: which spell teaches a spell, which spells
//! are talent ranks, and the skill lines a list row may require.
//!
//! A trainer's list names teaching spells. A teaching spell is a row of
//! `Spell.dbc` whose first effect is `SPELL_EFFECT_LEARN_SPELL` and whose
//! `EffectTriggerSpell[0]` is the spell it teaches; casting it teaches that
//! spell. A list that names the taught spell instead teaches nothing, so a
//! row is resolved to its teaching spell before it is written.
//!
//! The rules read their tables through [`Rows`], so they answer the same for
//! a shipped table and an edited one.

use crate::tables::dbc::Rows;
use crate::tables::spellbook::spell_fields;
use crate::tables::talent::Talents;
use crate::tables::trainer::EFFECT_LEARN_SPELL;
use std::collections::HashSet;

/// What `Spell.dbc` says about one spell, for a trainer's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpellFacts {
    /// `Effect[0]`.
    pub effect: u32,
    /// `EffectTriggerSpell[0]`: the spell a teaching spell teaches.
    pub teaches: u32,
    /// `spellLevel`.
    pub level: u32,
}

impl SpellFacts {
    /// Whether the spell is a teaching spell: its first effect learns a spell.
    pub fn is_teaching(&self) -> bool {
        self.effect == EFFECT_LEARN_SPELL
    }
}

/// What `Spell.dbc` says about `spell`, or `None` for a spell the table does
/// not hold.
pub fn spell_facts(spells: &impl Rows, spell: u32) -> Option<SpellFacts> {
    let record = spells.row_of(spell)?;
    Some(SpellFacts {
        effect: spells.u32_at(record, spell_fields::EFFECT).unwrap_or(0),
        teaches: spells.u32_at(record, spell_fields::EFFECT_TRIGGER_SPELL).unwrap_or(0),
        level: spells.u32_at(record, spell_fields::SPELL_LEVEL).unwrap_or(0),
    })
}

/// `CastingTimeIndex` 1, `SpellCastTimes.dbc`'s row for an instant cast.
const INSTANT: u32 = 1;

/// The teaching spell for `spell`: a row of `Spell.dbc` whose first effect is
/// `LEARN_SPELL` and whose `EffectTriggerSpell[0]` is `spell`, which is the
/// search the server makes when it loads trainers, for its log line.
///
/// 522 spells have more than one teaching spell. Fireball rank 2 (143) has
/// two: 483, which has a cast time and is the one a book casts, and 1173,
/// which is instant and is the one both mage trainer lists name. On the
/// reference database 1,821 of the 1,826 teaching spells that trainer lists
/// name are instant, and 468 of the 522 have exactly one instant teacher, so
/// an instant teacher is preferred, then the first in the file.
pub fn teaching_spell(spells: &impl Rows, spell: u32) -> Option<u32> {
    let teachers: Vec<(u32, bool)> = (0..spells.record_count())
        .filter_map(|record| {
            let teaches = spells.u32_at(record, spell_fields::EFFECT_TRIGGER_SPELL)? == spell;
            let learns = spells.u32_at(record, spell_fields::EFFECT)? == EFFECT_LEARN_SPELL;
            let instant = spells.u32_at(record, spell_fields::CASTING_TIME_INDEX) == Some(INSTANT);
            (teaches && learns).then(|| Some((spells.u32_at(record, 0)?, instant))).flatten()
        })
        .collect();
    teachers
        .iter()
        .find(|(_, instant)| *instant)
        .or(teachers.first())
        .map(|(id, _)| *id)
}

/// The spell a trainer row should name for a spell a person chose: the spell
/// itself when it teaches, otherwise the spell that teaches it, when there is
/// one. Returns the spell and, when it was replaced, the spell chosen.
pub fn resolve_teaching(spells: &impl Rows, chosen: u32) -> (u32, Option<u32>) {
    match spell_facts(spells, chosen) {
        Some(facts) if !facts.is_teaching() => match teaching_spell(spells, chosen) {
            Some(teacher) => (teacher, Some(chosen)),
            None => (chosen, None),
        },
        _ => (chosen, None),
    }
}

/// Every spell id that is a rank of a talent. A trainer does not teach these.
pub fn talent_spells(talents: &Talents) -> HashSet<u32> {
    talents
        .talents()
        .iter()
        .flat_map(|row| row.ranks)
        .filter(|spell| *spell != 0)
        .collect()
}

/// One skill line, as a list of skill lines shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedLine {
    pub id: u32,
    pub name: String,
}

impl NamedLine {
    /// Whether the line answers a filter: part of its name, in any case, or
    /// its id. An empty filter answers every line.
    pub fn matches(&self, filter: &str) -> bool {
        let filter = filter.trim();
        filter.is_empty()
            || self.id.to_string() == filter
            || self.name.to_ascii_lowercase().contains(&filter.to_ascii_lowercase())
    }
}

/// The skill lines of one `SkillLineCategory.dbc` row, under its name.
#[derive(Debug, Clone, PartialEq)]
pub struct LineGroup {
    pub category: u32,
    pub name: String,
    pub lines: Vec<NamedLine>,
}

/// `SkillLine.dbc`'s category and enUS name, and `SkillLineCategory.dbc`'s
/// enUS name and sort order: the fields [`crate::tables::skills`] reads.
const LINE_CATEGORY: usize = 1;
const LINE_NAME: usize = 3;
const CATEGORY_NAME: usize = 1;
const CATEGORY_SORT: usize = 10;

/// `SkillLineCategory.dbc` rows 11, Professions, and 9, Secondary Skills:
/// the lines a trainer row's required skill names. They are listed first.
pub(crate) const TRAINED_CATEGORIES: [u32; 2] = [11, 9];

/// Every named skill line, grouped by category: the two trained categories
/// first, then the others in the category table's own sort order, and by name
/// inside a group. A category the table does not name is headed by its id.
pub fn skill_groups<L: Rows, C: Rows>(lines: &L, categories: Option<&C>) -> Vec<LineGroup> {
    let mut groups: Vec<LineGroup> = Vec::new();
    for record in 0..lines.record_count() {
        let (Some(id), Some(name)) = (lines.u32_at(record, 0), lines.string_at(record, LINE_NAME)) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let category = lines.u32_at(record, LINE_CATEGORY).unwrap_or(0);
        let line = NamedLine { id, name };
        match groups.iter_mut().find(|group| group.category == category) {
            Some(group) => group.lines.push(line),
            None => groups.push(LineGroup {
                category,
                name: String::new(),
                lines: vec![line],
            }),
        }
    }
    let category_row = |category: u32| categories.and_then(|table| Some((table, table.row_of(category)?)));
    for group in &mut groups {
        group.name = category_row(group.category)
            .and_then(|(table, record)| table.string_at(record, CATEGORY_NAME))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("category {}", group.category));
        group.lines.sort_by(|a, b| a.name.cmp(&b.name));
    }
    let sort = |category: u32| {
        let trained = TRAINED_CATEGORIES.iter().position(|c| *c == category);
        let order = category_row(category).and_then(|(table, record)| table.u32_at(record, CATEGORY_SORT));
        (trained.is_none(), trained, order.unwrap_or(u32::MAX), category)
    };
    groups.sort_by_key(|group| sort(group.category));
    groups
}

/// What a skill line id is called, out of the groups.
pub fn skill_name(groups: &[LineGroup], id: u32) -> Option<&str> {
    groups
        .iter()
        .flat_map(|group| &group.lines)
        .find(|line| line.id == id)
        .map(|line| line.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;
    use crate::tables::dbc::Dbc;

    /// A `Spell.dbc` with one row per `(id, effect, teaches, cast time)`.
    fn spells(rows: &[(u32, u32, u32, u32)]) -> Dbc {
        let width = spell_fields::EFFECT_TRIGGER_SPELL + 3;
        let rows: Vec<Vec<u32>> = rows
            .iter()
            .map(|&(id, effect, teaches, cast)| {
                let mut row = vec![0; width];
                row[0] = id;
                row[spell_fields::EFFECT] = effect;
                row[spell_fields::EFFECT_TRIGGER_SPELL] = teaches;
                row[spell_fields::CASTING_TIME_INDEX] = cast;
                row[spell_fields::SPELL_LEVEL] = 4;
                row
            })
            .collect();
        Dbc::parse(&dbc(&rows, width, b"\0")).unwrap()
    }

    /// Fireball rank 2 has a cast-time teacher and an instant one; the
    /// instant one is chosen, and a spell that already teaches is kept.
    #[test]
    fn a_taught_spell_resolves_to_its_instant_teacher() {
        let table = spells(&[
            (143, 2, 0, 4),
            (483, EFFECT_LEARN_SPELL, 143, 4),
            (1173, EFFECT_LEARN_SPELL, 143, INSTANT),
            (5000, 2, 0, 1),
        ]);
        let facts = spell_facts(&table, 143).unwrap();
        assert!(!facts.is_teaching());
        assert_eq!(facts.level, 4);
        assert!(spell_facts(&table, 1173).unwrap().is_teaching());
        assert_eq!(spell_facts(&table, 9), None);
        assert_eq!(teaching_spell(&table, 143), Some(1173));
        assert_eq!(resolve_teaching(&table, 143), (1173, Some(143)));
        assert_eq!(resolve_teaching(&table, 1173), (1173, None), "already a teacher");
        assert_eq!(resolve_teaching(&table, 5000), (5000, None), "nothing teaches it");
        // With no instant teacher, the first in the file.
        let slow = spells(&[(143, 2, 0, 4), (483, EFFECT_LEARN_SPELL, 143, 4), (484, EFFECT_LEARN_SPELL, 143, 5)]);
        assert_eq!(teaching_spell(&slow, 143), Some(483));
    }

    /// The trained categories come first, the rest in the category table's
    /// sort order, and a line with no name is left out.
    #[test]
    fn skill_lines_group_by_category_with_the_trained_ones_first() {
        // SkillLine: id, category, _, name.
        let strings = b"\0Mining\0Swords\0Herbalism\0Fishing\0";
        let at = |text: &str| {
            let block = std::str::from_utf8(strings).unwrap();
            block.find(&format!("\0{text}\0")).unwrap() as u32 + 1
        };
        let lines = Dbc::parse(&dbc(
            &[
                vec![186, 11, 0, at("Mining")],
                vec![43, 6, 0, at("Swords")],
                vec![182, 11, 0, at("Herbalism")],
                vec![356, 9, 0, at("Fishing")],
                vec![999, 6, 0, 0],
            ],
            4,
            strings,
        ))
        .unwrap();
        let groups = skill_groups::<Dbc, Dbc>(&lines, None);
        let order: Vec<u32> = groups.iter().map(|group| group.category).collect();
        assert_eq!(order, vec![11, 9, 6]);
        let names: Vec<&str> = groups[0].lines.iter().map(|line| line.name.as_str()).collect();
        assert_eq!(names, vec!["Herbalism", "Mining"]);
        assert_eq!(groups[2].name, "category 6");
        assert_eq!(groups[2].lines.len(), 1, "the unnamed line is left out");
        assert_eq!(skill_name(&groups, 356), Some("Fishing"));
        assert!(groups[0].lines[0].matches("herb") && groups[0].lines[0].matches("182"));
        assert!(!groups[0].lines[0].matches("min"));
    }
}
