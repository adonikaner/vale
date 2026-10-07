//! `npc_trainer` and `npc_trainer_template`: what a creature teaches.
//!
//! ## Two tables, one schema
//!
//! `ObjectMgr::LoadTrainers` (`ObjectMgr.cpp:10600`) reads both with one
//! `SELECT`. What differs is what an `entry` is:
//!
//! ```text
//! npc_trainer           creature_template.entry: the creature's own list
//! npc_trainer_template  creature_template.trainer_id: a list shared by every
//!                       creature that names it
//! ```
//!
//! A creature teaches both lists (`WorldSession::SendTrainerList`). 284
//! creatures on the reference database name a `trainer_id`.
//!
//! ## A row is a teaching spell
//!
//! `spell` is not the ability a player ends up with. It is a spell whose first
//! effect is `SPELL_EFFECT_LEARN_SPELL` (36) and whose `EffectTriggerSpell[0]`
//! is that ability: Fireball rank 2 (143) is taught by spell 1173. The client
//! reads the taught spell off the teaching spell to draw the training window.
//! The loader skips a row whose spell is not a teaching spell, and logs the
//! teaching spell it should have been when there is one.
//!
//! ## A row is keyed by `(entry, spell, build_min, build_max)`
//!
//! The unique key of both tables is `(entry, spell, build_max)`; the loader
//! reads `WHERE 5875 BETWEEN build_min AND build_max`, so one spell may be in a
//! list twice under two build ranges. The key here names `build_min` as well,
//! so it identifies one row whatever the ranges. A row made here is written
//! `0..5875`, the DDL's default and every build.
//!
//! ## What the loader skips
//!
//! `LoadTrainers` skips a row, with one log line each, when:
//!
//! * the spell is not in `spell_template`;
//! * its first effect is not `LEARN_SPELL`;
//! * the spell fails `SpellMgr::IsSpellValid`;
//! * the spell is a talent (`GetTalentSpellCost` is not 0);
//! * for the own list only: the creature has no `creature_template` row, its
//!   `npc_flags` lacks `TRAINER`, or the spell is already in the list its
//!   `trainer_id` names.
//!
//! [`Lesson::check`] makes the checks that need neither the database nor the
//! spell tables; [`spell_faults`] makes the spell checks from facts a caller
//! reads out of `Spell.dbc` and `Talent.dbc`.
//!
//! ## Price, level and skill
//!
//! `spellcost` is copper. `reqlevel` 0 is the teaching spell's `spellLevel`,
//! and the loader logs a `reqlevel` equal to it as redundant. `reqskill` and
//! `reqskillvalue` are a `SkillLine.dbc` id and the rank in it the player
//! needs; a profession's recipes use them.
//!
//! ## Live on a reload
//!
//! `.reload npc_trainer` re-reads `npc_trainer_template` and then
//! `npc_trainer` (`ServerCommands.cpp:1280`), and `LoadTrainers` clears each
//! list first, so a removed row is gone from the server. The client asks for
//! the list each time the window is opened.

use crate::row::{Assignment, Key, Life};

pub use crate::schema::{money_words, Column, Group, Kind, Row, RowValue};

pub const TRAINER: &str = "npc_trainer";
pub const TEMPLATE: &str = "npc_trainer_template";

/// Both tables, in the order the server reloads them.
pub const TABLES: [&str; 2] = [TEMPLATE, TRAINER];

/// The static name for a table read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// What an `entry` of each table is, for a window's heading.
pub fn keyed_by(table: &str) -> &'static str {
    match table {
        TEMPLATE => "creature_template.trainer_id",
        _ => "creature_template.entry",
    }
}

/// The columns of both tables, in `LoadTrainers`' `SELECT` order with the two
/// build columns its `WHERE` filters on after them.
pub const COLUMNS: [Column; 8] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the training list: a creature entry, or a trainer_id in the template table" },
    Column { name: "spell", kind: Kind::Key, group: Group::Identity, about: "the teaching spell: its first effect is LEARN_SPELL, and it teaches its EffectTriggerSpell" },
    Column { name: "spellcost", kind: Kind::Money, group: Group::Services, about: "what training costs, in copper" },
    Column { name: "reqskill", kind: Kind::Ref("SkillLine"), group: Group::Requirements, about: "a SkillLine.dbc id the player must have, or 0" },
    Column { name: "reqskillvalue", kind: Kind::Unsigned, group: Group::Requirements, about: "the rank in reqskill the player needs" },
    Column { name: "reqlevel", kind: Kind::Unsigned, group: Group::Requirements, about: "the level the player needs; 0 is the teaching spell's own spellLevel" },
    Column { name: "build_min", kind: Kind::Key, group: Group::Identity, about: "the first client build the row is loaded at" },
    Column { name: "build_max", kind: Kind::Key, group: Group::Identity, about: "the last client build the row is loaded at" },
];

/// One column, by name, of either table.
pub fn column(table: &str, name: &str) -> Option<&'static Column> {
    table_named(table)?;
    COLUMNS.iter().find(|column| column.name == name)
}

/// The build a vmangos for 1.12.1 loads rows at: `SUPPORTED_CLIENT_BUILD`.
pub const BUILD: u32 = crate::datadir::CLIENT_BUILD;

/// The build range a row made here is written with: every build.
pub const EVERY_BUILD: (u32, u32) = (0, BUILD);

/// The largest spell id the `spell` column holds: it is a `smallint
/// unsigned`.
pub const MAX_SPELL: u32 = 65_535;

/// The most `reqlevel` holds: a `tinyint unsigned`.
pub const MAX_LEVEL: u32 = 255;

/// `SPELL_EFFECT_LEARN_SPELL`: the first effect of every teaching spell.
pub const LEARN_SPELL: u32 = 36;

/// The key of one row.
pub fn key(entry: u32, spell: u32, build_min: u32, build_max: u32) -> Key {
    Key(vec![
        ("entry".to_string(), entry.to_string()),
        ("spell".to_string(), spell.to_string()),
        ("build_min".to_string(), build_min.to_string()),
        ("build_max".to_string(), build_max.to_string()),
    ])
}

/// One row of either table, as the server reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Lesson {
    pub entry: u32,
    /// The teaching spell.
    pub spell: u32,
    /// `spellcost`, in copper.
    pub cost: u32,
    pub skill: u32,
    pub skill_value: u32,
    /// `reqlevel` as stored: 0 is the teaching spell's own level.
    pub level: u32,
    pub build_min: u32,
    pub build_max: u32,
}

impl Lesson {
    /// A row as a new one is written: free, at the spell's own level, every
    /// build.
    pub fn new(entry: u32, spell: u32) -> Lesson {
        Lesson {
            entry,
            spell,
            cost: 0,
            skill: 0,
            skill_value: 0,
            level: 0,
            build_min: EVERY_BUILD.0,
            build_max: EVERY_BUILD.1,
        }
    }

    pub fn key(&self) -> Key {
        key(self.entry, self.spell, self.build_min, self.build_max)
    }

    /// Every column that is not the key, as SQL literals, which is what a row
    /// this project creates carries.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "spellcost", value: self.cost.to_string() },
            Assignment { column: "reqskill", value: self.skill.to_string() },
            Assignment { column: "reqskillvalue", value: self.skill_value.to_string() },
            Assignment { column: "reqlevel", value: self.level.to_string() },
        ]
    }

    /// One row as the database answered it, or `None` for a row missing a key
    /// column.
    pub fn from_row(row: &Row) -> Option<Lesson> {
        let number = |column: &str| row.integer(column).unwrap_or(0).max(0) as u32;
        Some(Lesson {
            entry: row.integer("entry")? as u32,
            spell: row.integer("spell")? as u32,
            cost: number("spellcost"),
            skill: number("reqskill"),
            skill_value: number("reqskillvalue"),
            level: number("reqlevel"),
            build_min: row.integer("build_min").map_or(EVERY_BUILD.0, |v| v.max(0) as u32),
            build_max: row.integer("build_max").map_or(EVERY_BUILD.1, |v| v.max(0) as u32),
        })
    }

    /// The level a player needs, as the loader settles it: `reqlevel`, or the
    /// teaching spell's level when that is 0.
    pub fn level_needed(&self, spell_level: u32) -> u32 {
        match self.level {
            0 => spell_level,
            level => level,
        }
    }

    /// Whether this row is loaded by a server for 1.12.1.
    pub fn loaded(&self) -> bool {
        (self.build_min..=self.build_max).contains(&BUILD)
    }

    /// Why the server would skip this row, or refuse to store it, each as a
    /// sentence. The checks that need neither the database nor the spell
    /// tables; see [`spell_faults`] for the others.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.spell > MAX_SPELL {
            out.push(format!("spell {} does not fit the column, which holds up to {MAX_SPELL}", self.spell));
        }
        if self.level > MAX_LEVEL {
            out.push(format!("reqlevel {} is over {MAX_LEVEL}", self.level));
        }
        if !self.loaded() {
            out.push(format!(
                "build range {}..{} excludes {BUILD}, so the server does not load the row",
                self.build_min, self.build_max
            ));
        }
        out
    }
}

/// What a caller knows about a teaching spell, read out of `Spell.dbc` and
/// `Talent.dbc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Taught {
    /// `Effect[0]`.
    pub effect: u32,
    /// `EffectTriggerSpell[0]`: the spell a teaching spell teaches.
    pub teaches: u32,
    /// `spellLevel`.
    pub level: u32,
    /// Whether the spell id is a rank of a talent.
    pub talent: bool,
}

impl Taught {
    pub fn is_teaching(&self) -> bool {
        self.effect == LEARN_SPELL
    }
}

/// Why `LoadTrainers` would skip a row for its spell, in its order. `None` is
/// a spell the tables do not hold. `teacher` is the teaching spell for the
/// row's spell, when the row names the taught spell and one exists.
pub fn spell_faults(taught: Option<Taught>, teacher: Option<u32>) -> Vec<String> {
    let Some(taught) = taught else {
        return vec!["the spell is not in Spell.dbc".to_string()];
    };
    let mut out = Vec::new();
    if !taught.is_teaching() {
        out.push(match teacher {
            Some(teacher) => format!(
                "not a teaching spell: its first effect is {}, not LEARN_SPELL; spell {teacher} teaches it",
                taught.effect
            ),
            None => format!("not a teaching spell: its first effect is {}, not LEARN_SPELL", taught.effect),
        });
    }
    if taught.talent {
        out.push("the spell is a talent rank, and the server does not train talents".to_string());
    }
    out
}

/// What `creature_template.trainer_type` makes of a player, as a sentence.
/// The rules are `Creature::IsTrainerOf` (`Creature.cpp:1359`).
pub fn who_words(trainer_type: u32, class: u32, race: u32, spell: &str) -> String {
    match trainer_type {
        0 => match class_word(class) {
            Some(class) => format!("A class trainer: it trains {class}s only."),
            None => format!("A class trainer for class {class}, which is not a 1.12 class, so it trains nobody."),
        },
        1 => match race_word(race) {
            Some(race) => format!("A mount trainer: it trains {race}s, and anyone exalted with its faction."),
            None => "A mount trainer: it trains every race.".to_string(),
        },
        2 => match spell.is_empty() {
            true => "A tradeskill trainer: it trains anyone.".to_string(),
            false => format!("A tradeskill trainer: it trains a player who knows {spell}."),
        },
        3 => "A pet trainer: it trains hunters only.".to_string(),
        other => format!("trainer_type {other} is not a type the server has, so it trains nobody."),
    }
}

/// `Classes`, from `SharedDefines.h`.
pub fn class_word(class: u32) -> Option<&'static str> {
    Some(match class {
        1 => "Warrior",
        2 => "Paladin",
        3 => "Hunter",
        4 => "Rogue",
        5 => "Priest",
        7 => "Shaman",
        8 => "Mage",
        9 => "Warlock",
        11 => "Druid",
        _ => return None,
    })
}

/// `Races`, from `SharedDefines.h`.
pub fn race_word(race: u32) -> Option<&'static str> {
    Some(match race {
        1 => "Human",
        2 => "Orc",
        3 => "Dwarf",
        4 => "Night Elf",
        5 => "Undead",
        6 => "Tauren",
        7 => "Gnome",
        8 => "Troll",
        _ => return None,
    })
}

/// The statements a save emits for one row this project claims, on
/// [`crate::vendor::statements`]' terms.
pub fn statements(table: &str, key: &Key, life: Life, changes: &[Assignment]) -> Vec<String> {
    crate::vendor::statements(table, key, life, changes)
}

// ---------------------------------------------------------------------------
// The queries
// ---------------------------------------------------------------------------

/// Every row of one list the server loads, lowest level first.
pub fn rows_query(table: &str, entry: u32) -> String {
    format!(
        "SELECT * FROM {} WHERE `entry` = {entry} AND {BUILD} BETWEEN `build_min` AND `build_max` \
         ORDER BY `reqlevel`, `spell`",
        crate::sql::name(table)
    )
}

/// The row a key names — what an undo is taken from.
pub fn row_query(table: &str, key: &Key) -> String {
    crate::vendor::row_query(table, key)
}

/// Whether a row is already there, which an apply asks before it creates one.
pub fn exists_query(table: &str, key: &Key) -> String {
    crate::vendor::exists_query(table, key)
}

/// The creatures that name a template list, one line per entry.
pub fn users_query(trainer_id: u32) -> String {
    format!(
        "SELECT `entry`, MAX(`name`) AS `name` FROM `creature_template` \
         WHERE `trainer_id` = {trainer_id} GROUP BY `entry` ORDER BY `entry`"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The columns are `LoadTrainers`' `SELECT`, with the two build columns
    /// its `WHERE` reads after them, and the key names four of them.
    #[test]
    fn every_column_is_the_servers_own_in_the_servers_own_order() {
        let ours: Vec<&str> = COLUMNS.iter().map(|column| column.name).collect();
        assert_eq!(
            ours,
            ["entry", "spell", "spellcost", "reqskill", "reqskillvalue", "reqlevel", "build_min", "build_max"]
        );
        let lesson = Lesson::new(328, 1173);
        for column in COLUMNS.iter().filter(|column| !column.editable()) {
            assert!(lesson.key().0.iter().any(|(name, _)| name == column.name), "{}", column.name);
        }
        assert_eq!(
            lesson.key().where_clause(),
            "`entry` = 328 AND `spell` = 1173 AND `build_min` = 0 AND `build_max` = 5875"
        );
    }

    /// A new row is loaded at 5875, and a row outside the build is said to be
    /// one the server does not load.
    #[test]
    fn a_row_outside_the_build_is_not_loaded() {
        assert!(Lesson::new(328, 1173).check().is_empty());
        let old = Lesson { build_max: 4878, ..Lesson::new(328, 1173) };
        assert!(old.check()[0].contains("excludes 5875"), "{:?}", old.check());
        let wide = Lesson { spell: 70_000, ..Lesson::new(328, 1173) };
        assert!(wide.check()[0].contains("does not fit"));
    }

    /// `LoadTrainers`' spell checks, in its order.
    #[test]
    fn the_spell_checks_are_the_loaders() {
        let teaching = Taught { effect: LEARN_SPELL, teaches: 143, level: 6, talent: false };
        assert!(spell_faults(Some(teaching), None).is_empty());
        assert!(spell_faults(None, None)[0].contains("not in Spell.dbc"));
        let fireball = Taught { effect: 2, teaches: 0, level: 6, talent: false };
        let faults = spell_faults(Some(fireball), Some(1173));
        assert!(faults[0].contains("spell 1173 teaches it"), "{faults:?}");
        let talent = Taught { talent: true, ..teaching };
        assert!(spell_faults(Some(talent), None)[0].contains("talent"));
    }

    /// `reqlevel` 0 is the spell's own level.
    #[test]
    fn a_level_of_zero_is_the_spells_own() {
        assert_eq!(Lesson::new(328, 1173).level_needed(6), 6);
        assert_eq!(Lesson { level: 10, ..Lesson::new(328, 1173) }.level_needed(6), 10);
    }

    /// A created row is a `DELETE` and an `INSERT` on the whole key.
    #[test]
    fn a_row_becomes_the_statements_it_means() {
        let lesson = Lesson { cost: 100, ..Lesson::new(328, 1173) };
        let created = statements(TRAINER, &lesson.key(), Life::Insert, &lesson.assignments());
        assert_eq!(
            created[1],
            "INSERT INTO `npc_trainer` (`entry`, `spell`, `build_min`, `build_max`, `spellcost`, \
             `reqskill`, `reqskillvalue`, `reqlevel`) VALUES (328, 1173, 0, 5875, 100, 0, 0, 0);"
        );
        let mut row = Row::new();
        for (column, value) in [
            ("entry", "1"),
            ("spell", "1173"),
            ("spellcost", "100"),
            ("reqskill", "0"),
            ("reqskillvalue", "0"),
            ("reqlevel", "0"),
            ("build_min", "0"),
            ("build_max", "5875"),
        ] {
            row.insert(column.to_string(), Some(value.to_string()));
        }
        assert_eq!(Lesson::from_row(&row), Some(Lesson { entry: 1, cost: 100, ..Lesson::new(1, 1173) }));
        assert!(rows_query(TEMPLATE, 1).contains("5875 BETWEEN `build_min` AND `build_max`"));
    }

    #[test]
    fn a_trainer_type_reads_as_who_it_trains() {
        assert_eq!(who_words(0, 8, 0, ""), "A class trainer: it trains Mages only.");
        assert!(who_words(1, 0, 0, "").contains("every race"));
        assert!(who_words(2, 0, 0, "Apprentice Tailor").contains("knows Apprentice Tailor"));
        assert!(who_words(7, 0, 0, "").contains("trains nobody"));
    }
}
