//! `conditions`: the server's reusable yes-or-no tests.
//!
//! A condition is one row: a type, four values whose meaning the type decides,
//! and two flags. Other tables name a row by its `condition_entry`: a gossip
//! menu's text and option, a loot row, a vendor item, a quest, a teleport, an
//! area trigger's script, an event of a creature's script. The server asks the
//! condition about a target (usually the player) and a source (usually the
//! creature or object), and acts only when it holds.
//!
//! ```text
//! condition_entry  the key
//! type             what is tested; see [`TYPES`]
//! value1..value4   what it is tested against; [`Type::values`] names each
//! flags            1 reverses the result, 2 swaps target and source
//! ```
//!
//! ## Combining conditions
//!
//! Types -1 (AND) and -2 (OR) name two to four other conditions in their
//! values, and -3 (NOT) names one; the reverse flag does what NOT does and
//! vmangos prefers it. `ConditionEntry::IsValid` (`Conditions.cpp:753`)
//! refuses a combining condition whose children are not lower entries than
//! itself, or do not exist, so a tree is built from its leaves up: a new
//! parent takes an entry above every child. [`Condition::check`] makes the
//! checks the row alone can show, and [`check_tree`] the ones that need the
//! other rows.
//!
//! ## What else the table refuses
//!
//! A unique key over `(type, value1, value2, flags, value3, value4)`: two rows
//! may not test the same thing. [`same_test`] finds the row a new one would
//! repeat, so the editor can offer the existing one instead.
//!
//! Live on `.reload conditions` (`Chat.cpp:818`).

use crate::row::{Assignment, Key};
use crate::schema::{Bit, Column, Group, Kind};

pub const TABLE: &str = "conditions";

/// The one table, as the row subjects list their tables.
pub const TABLES: [&str; 1] = [TABLE];

/// The static name for [`TABLE`] read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// `flags`, `ConditionFlags` in `Conditions.h`.
pub const FLAGS: [Bit; 2] = [
    Bit { bit: 0x1, name: "REVERSE_RESULT", about: "the condition holds when its test fails" },
    Bit { bit: 0x2, name: "SWAP_TARGETS", about: "the test is made with the target and the source swapped" },
];

/// The flag that reverses the result.
pub const REVERSE: u8 = 0x1;

/// `conditions`, in table order.
pub const COLUMNS: [Column; 7] = [
    Column { name: "condition_entry", kind: Kind::Key, group: Group::Identity, about: "the id other tables name it by" },
    Column { name: "type", kind: Kind::Signed, group: Group::Identity, about: "what is tested" },
    Column { name: "value1", kind: Kind::Signed, group: Group::Requirements, about: "the first value; the type says what it is" },
    Column { name: "value2", kind: Kind::Signed, group: Group::Requirements, about: "the second value" },
    Column { name: "value3", kind: Kind::Signed, group: Group::Requirements, about: "the third value" },
    Column { name: "value4", kind: Kind::Signed, group: Group::Requirements, about: "the fourth value" },
    Column { name: "flags", kind: Kind::Flags(&FLAGS), group: Group::Requirements, about: "reverse the result, or swap target and source" },
];

/// Every column of [`TABLE`].
pub fn columns_of(table: &str) -> &'static [Column] {
    match table {
        TABLE => &COLUMNS,
        _ => &[],
    }
}

/// One column, by name.
pub fn column(table: &str, name: &str) -> Option<&'static Column> {
    columns_of(table).iter().find(|column| column.name == name)
}

/// The key of a condition.
pub fn key(entry: u32) -> Key {
    Key::one("condition_entry", u64::from(entry))
}

/// What a value of a condition is, for a form to draw and resolve it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Means {
    Number,
    /// Another condition's entry.
    Condition,
    Spell,
    Item,
    Quest,
    Creature,
    GameObject,
    /// An `AreaTable` row.
    Area,
    /// A `Faction` row.
    Faction,
    /// A `SkillLine` row.
    Skill,
    /// A `Map` row.
    Map,
    /// 0 equal to, 1 equal or higher, 2 equal or lower.
    Compare,
    /// 469 Alliance, 67 Horde.
    Team,
    /// A reputation rank, 0 Hated to 7 Exalted.
    Rank,
    /// 0 any state, 1 incomplete, 2 complete.
    QuestState,
    RaceMask,
    ClassMask,
    /// 0 male, 1 female, 2 none.
    Gender,
    /// 0 or 1.
    Bool,
}

/// One value of a type: what the form calls it and what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub label: &'static str,
    pub means: Means,
}

const fn s(label: &'static str, means: Means) -> Option<Slot> {
    Some(Slot { label, means })
}

/// One condition type, as `Conditions.h` documents it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Type {
    pub id: i32,
    pub name: &'static str,
    /// What it tests, in one sentence.
    pub about: &'static str,
    /// What the server must have to test it: a player target, a creature
    /// source, a map, or nothing.
    pub needs: &'static str,
    pub values: [Option<Slot>; 4],
}

use Means::*;

/// Every type vmangos defines, in id order. `CONDITION_NONE` (0) is listed for
/// completeness; it always holds and is internal.
pub const TYPES: [Type; 63] = [
    Type { id: -3, name: "Not", about: "Holds when the named condition does not. The reverse flag is preferred.", needs: "nothing", values: [s("condition", Condition), None, None, None] },
    Type { id: -2, name: "Or", about: "Holds when at least one of the named conditions does.", needs: "nothing", values: [s("condition", Condition), s("condition", Condition), s("condition (optional)", Condition), s("condition (optional)", Condition)] },
    Type { id: -1, name: "And", about: "Holds only when all of the named conditions do.", needs: "nothing", values: [s("condition", Condition), s("condition", Condition), s("condition (optional)", Condition), s("condition (optional)", Condition)] },
    Type { id: 0, name: "None", about: "Always holds. For the server's internal use.", needs: "nothing", values: [None, None, None, None] },
    Type { id: 1, name: "Aura", about: "The target has an aura from this spell.", needs: "a unit target", values: [s("spell", Spell), s("effect index", Number), None, None] },
    Type { id: 2, name: "Item", about: "The player carries at least this many of the item.", needs: "a player target", values: [s("item", Item), s("count", Number), None, None] },
    Type { id: 3, name: "Item equipped", about: "The player has the item equipped.", needs: "a player target", values: [s("item", Item), None, None, None] },
    Type { id: 4, name: "Area", about: "The target or source is in the zone or area.", needs: "a world object", values: [s("area", Area), None, None, None] },
    Type { id: 5, name: "Reputation at least", about: "The player's reputation with the faction is at least this rank.", needs: "a player target", values: [s("faction", Faction), s("minimum rank", Rank), None, None] },
    Type { id: 6, name: "Team", about: "The player is on this side.", needs: "a player target", values: [s("team", Team), None, None, None] },
    Type { id: 7, name: "Skill", about: "The player's skill is at least this value.", needs: "a player target", values: [s("skill", Skill), s("value", Number), None, None] },
    Type { id: 8, name: "Quest rewarded", about: "The player has completed the quest.", needs: "a player target", values: [s("quest", Quest), None, None, None] },
    Type { id: 9, name: "Quest taken", about: "The player has the quest in the quest log.", needs: "a player target", values: [s("quest", Quest), s("state", QuestState), None, None] },
    Type { id: 10, name: "Argent Dawn commission", about: "The player has an Argent Dawn commission aura.", needs: "a player target", values: [None, None, None, None] },
    Type { id: 11, name: "Saved variable", about: "A global saved variable compares to the value.", needs: "nothing", values: [s("index", Number), s("value", Number), s("compare", Compare), None] },
    Type { id: 12, name: "Game event active", about: "The game event is running.", needs: "nothing", values: [s("event", Number), None, None, None] },
    Type { id: 13, name: "Cannot reach victim", about: "The source is chasing a victim it cannot find a path to.", needs: "a unit source", values: [None, None, None, None] },
    Type { id: 14, name: "Race and class", about: "The player's race and class are in the masks.", needs: "a player target", values: [s("races", RaceMask), s("classes", ClassMask), None, None] },
    Type { id: 15, name: "Level", about: "The target's level compares to the value.", needs: "a unit target", values: [s("level", Number), s("compare", Compare), None, None] },
    Type { id: 16, name: "Source entry", about: "The source is one of these creature or object entries.", needs: "a world object source", values: [s("entry", Number), s("entry (optional)", Number), s("entry (optional)", Number), s("entry (optional)", Number)] },
    Type { id: 17, name: "Spell known", about: "The player has learned the spell, or with 1, has not.", needs: "a player target", values: [s("spell", Spell), s("hasn't it", Bool), None, None] },
    Type { id: 18, name: "Instance script", about: "The instance script answers yes to one of its own questions.", needs: "a map", values: [s("map", Map), s("instance condition", Number), None, None] },
    Type { id: 19, name: "Quest available", about: "The player can accept the quest.", needs: "a player target", values: [s("quest", Quest), None, None, None] },
    Type { id: 20, name: "Creature nearby", about: "A creature with this entry is within the radius.", needs: "a world object target", values: [s("creature", Creature), s("radius", Number), s("dead", Bool), s("not self", Bool)] },
    Type { id: 21, name: "Object nearby", about: "A game object with this entry is within the radius.", needs: "a world object target", values: [s("game object", GameObject), s("radius", Number), None, None] },
    Type { id: 22, name: "Quest none", about: "The player has neither taken nor completed the quest.", needs: "a player target", values: [s("quest", Quest), None, None, None] },
    Type { id: 23, name: "Item with bank", about: "The player has at least this many of the item, bank included.", needs: "a player target", values: [s("item", Item), s("count", Number), None, None] },
    Type { id: 24, name: "Content patch", about: "The server's content patch compares to the value.", needs: "nothing", values: [s("patch (0-10)", Number), s("compare", Compare), None, None] },
    Type { id: 25, name: "Escort", about: "The source and target are alive and within the distance. Used for escorts.", needs: "nothing", values: [s("flags (1 source dead, 2 target dead)", Number), s("distance", Number), None, None] },
    Type { id: 26, name: "Holiday active", about: "The holiday is active. Game event active is preferred.", needs: "nothing", values: [s("holiday", Number), None, None, None] },
    Type { id: 27, name: "Gender", about: "The target is this gender.", needs: "a world object target", values: [s("gender", Gender), None, None, None] },
    Type { id: 28, name: "Is a player", about: "The target is a player, or with 1 also something a player owns.", needs: "a world object target", values: [s("player owned too", Bool), None, None, None] },
    Type { id: 29, name: "Skill below", about: "The player knows the skill below this value; with 1, does not know it.", needs: "a player target", values: [s("skill", Skill), s("value", Number), None, None] },
    Type { id: 30, name: "Reputation at most", about: "The player's reputation with the faction is at most this rank.", needs: "a player target", values: [s("faction", Faction), s("maximum rank", Rank), None, None] },
    Type { id: 31, name: "Has flag", about: "An update field of the source has the flag set.", needs: "a world object source", values: [s("field", Number), s("flag", Number), None, None] },
    Type { id: 32, name: "Last waypoint", about: "The source creature's last waypoint compares to the value.", needs: "a creature source", values: [s("waypoint", Number), s("compare", Compare), None, None] },
    Type { id: 33, name: "Map", about: "The current map is this one.", needs: "a map", values: [s("map", Map), None, None, None] },
    Type { id: 34, name: "Instance data", about: "The instance script's data at the index compares to the value.", needs: "a map", values: [s("index", Number), s("value", Number), s("compare", Compare), None] },
    Type { id: 35, name: "Map event data", about: "A scripted map event's data compares to the value.", needs: "a map", values: [s("event", Number), s("index", Number), s("value", Number), s("compare", Compare)] },
    Type { id: 36, name: "Map event active", about: "The scripted map event is running.", needs: "a map", values: [s("event", Number), None, None, None] },
    Type { id: 37, name: "Line of sight", about: "The source and target can see each other.", needs: "a source and a target", values: [None, None, None, None] },
    Type { id: 38, name: "Distance to target", about: "The distance between source and target compares to the value.", needs: "a source and a target", values: [s("distance", Number), s("compare", Compare), None, None] },
    Type { id: 39, name: "Is moving", about: "The target is moving.", needs: "a world object target", values: [None, None, None, None] },
    Type { id: 40, name: "Has pet", about: "The target has a pet.", needs: "a unit target", values: [None, None, None, None] },
    Type { id: 41, name: "Health percent", about: "The target's health percent compares to the value.", needs: "a unit target", values: [s("percent", Number), s("compare", Compare), None, None] },
    Type { id: 42, name: "Mana percent", about: "The target's mana percent compares to the value.", needs: "a unit target", values: [s("percent", Number), s("compare", Compare), None, None] },
    Type { id: 43, name: "In combat", about: "The target is in combat.", needs: "a unit target", values: [None, None, None, None] },
    Type { id: 44, name: "Reaction", about: "The target's reaction to the source compares to the rank.", needs: "a source and a target", values: [s("reaction", Rank), s("compare", Compare), None, None] },
    Type { id: 45, name: "In group", about: "The player is in a group.", needs: "a player target", values: [None, None, None, None] },
    Type { id: 46, name: "Alive", about: "The target is alive.", needs: "a unit target", values: [None, None, None, None] },
    Type { id: 47, name: "Map event targets", about: "Every extra target of the scripted map event meets the condition.", needs: "a map", values: [s("event", Number), s("condition", Condition), None, None] },
    Type { id: 48, name: "Object spawned", about: "The game object target is spawned.", needs: "a game object target", values: [None, None, None, None] },
    Type { id: 49, name: "Object loot state", about: "The game object target is in this loot state.", needs: "a game object target", values: [s("loot state", Number), None, None, None] },
    Type { id: 50, name: "Object meets condition", about: "The game object with this guid exists and meets the condition.", needs: "a map", values: [s("guid", Number), s("condition", Condition), None, None] },
    Type { id: 51, name: "Honor rank", about: "The player's honor rank compares to the value.", needs: "a player target", values: [s("rank", Number), s("compare", Compare), None, None] },
    Type { id: 52, name: "Spawn guid", about: "The source is one of these spawns, by database guid.", needs: "a world object source", values: [s("guid", Number), s("guid (optional)", Number), s("guid (optional)", Number), s("guid (optional)", Number)] },
    Type { id: 53, name: "Local time", about: "The server's local time is in the range.", needs: "nothing", values: [s("start hour", Number), s("start minute", Number), s("end hour", Number), s("end minute", Number)] },
    Type { id: 54, name: "Distance to position", about: "The target is within the distance of the point.", needs: "a world object target", values: [s("x", Number), s("y", Number), s("z", Number), s("distance", Number)] },
    Type { id: 55, name: "Object state", about: "The game object target is in this state.", needs: "a game object target", values: [s("state", Number), None, None, None] },
    Type { id: 56, name: "Player nearby", about: "A player is within the radius: 0 any, 1 hostile, 2 friendly.", needs: "a unit target", values: [s("which", Number), s("radius", Number), None, None] },
    Type { id: 57, name: "Group member", about: "The source creature is in a creature group, led by this guid when given.", needs: "a creature source", values: [s("leader guid (optional)", Number), None, None, None] },
    Type { id: 58, name: "Group dead", about: "The source creature's group is dead.", needs: "a creature source", values: [None, None, None, None] },
    Type { id: 59, name: "Area explored", about: "The player has explored the area.", needs: "a player target", values: [s("area", Area), None, None, None] },
];

/// The type with this id, or `None` for one vmangos does not define.
pub fn type_of(id: i32) -> Option<&'static Type> {
    TYPES.iter().find(|kind| kind.id == id)
}

/// The names of `Compare`'s three values.
pub const COMPARES: [&str; 3] = ["equal to", "at least", "at most"];

/// The names of the reputation ranks, 0 to 7.
pub const RANKS: [&str; 8] = ["Hated", "Hostile", "Unfriendly", "Neutral", "Friendly", "Honored", "Revered", "Exalted"];

/// One condition row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Condition {
    pub entry: u32,
    pub kind: i32,
    pub values: [i32; 4],
    pub flags: u8,
}

impl Condition {
    pub fn key(&self) -> Key {
        key(self.entry)
    }

    /// Every column but the key, as a created row names them.
    pub fn assignments(&self) -> Vec<Assignment> {
        let mut out = vec![Assignment { column: "type", value: self.kind.to_string() }];
        for (column, value) in ["value1", "value2", "value3", "value4"].into_iter().zip(self.values) {
            out.push(Assignment { column, value: value.to_string() });
        }
        out.push(Assignment { column: "flags", value: self.flags.to_string() });
        out
    }

    /// A row as the database returned it.
    pub fn from_row(row: &crate::schema::Row) -> Option<Condition> {
        use crate::schema::RowValue;
        let int = |column: &str| row.integer(column);
        Some(Condition {
            entry: int("condition_entry")? as u32,
            kind: int("type").unwrap_or(0) as i32,
            values: [
                int("value1").unwrap_or(0) as i32,
                int("value2").unwrap_or(0) as i32,
                int("value3").unwrap_or(0) as i32,
                int("value4").unwrap_or(0) as i32,
            ],
            flags: int("flags").unwrap_or(0) as u8,
        })
    }

    /// Whether the reverse flag is set.
    pub fn reversed(&self) -> bool {
        self.flags & REVERSE != 0
    }

    /// The conditions this one names in its values: an AND's, an OR's or a
    /// NOT's children, and the condition a map event or game object test
    /// names. Zero values are left out.
    pub fn children(&self) -> Vec<u32> {
        let Some(kind) = type_of(self.kind) else {
            return Vec::new();
        };
        kind.values
            .iter()
            .zip(self.values)
            .filter(|(slot, value)| slot.is_some_and(|slot| slot.means == Means::Condition) && *value > 0)
            .map(|(_, value)| value as u32)
            .collect()
    }

    /// Why the server would skip this row, from the row alone.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Some(kind) = type_of(self.kind) else {
            out.push(format!("type {} is not one vmangos defines", self.kind));
            return out;
        };
        if matches!(self.kind, -3..=-1) {
            let needed = if self.kind == -3 { 1 } else { 2 };
            for (n, value) in self.values.iter().take(needed).enumerate() {
                if *value <= 0 {
                    out.push(format!("{} needs a condition in value{}", kind.name, n + 1));
                }
            }
            for (n, value) in self.values.iter().enumerate() {
                if *value > 0 && *value as u32 >= self.entry && self.entry != 0 {
                    out.push(format!(
                        "value{} names condition {value}, which must be a lower entry than {}",
                        n + 1,
                        self.entry
                    ));
                }
            }
        }
        out
    }
}

/// Why the server would skip a condition because of the other rows: a child
/// it names that `rows` does not hold. `exists` answers whether an entry is a
/// condition.
pub fn check_tree(condition: &Condition, exists: impl Fn(u32) -> bool) -> Vec<String> {
    condition
        .children()
        .into_iter()
        .filter(|child| !exists(*child))
        .map(|child| format!("condition {child}, which it names, does not exist"))
        .collect()
}

/// The row among `others` that tests exactly what `condition` tests, which the
/// table's unique key would refuse a second of. Not the condition itself.
pub fn same_test<'a>(condition: &Condition, others: impl IntoIterator<Item = &'a Condition>) -> Option<&'a Condition> {
    others.into_iter().find(|other| {
        other.entry != condition.entry
            && other.kind == condition.kind
            && other.values == condition.values
            && other.flags == condition.flags
    })
}

/// Why the server would skip a created row, from its columns alone.
pub fn check_created(table: &str, row: &crate::schema::Row) -> Vec<String> {
    match table {
        TABLE => Condition::from_row(row).map(|condition| condition.check()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Every condition, in entry order. The table is a few thousand rows.
pub fn all_query() -> String {
    format!("SELECT * FROM `{TABLE}` ORDER BY `condition_entry`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_has_a_distinct_id_in_order() {
        let ids: Vec<i32> = TYPES.iter().map(|kind| kind.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids, sorted);
        assert_eq!(ids.first(), Some(&-3));
        assert_eq!(ids.last(), Some(&59));
    }

    /// An AND must name lower entries than itself, which is how vmangos
    /// builds a tree from its leaves.
    #[test]
    fn a_combining_condition_names_lower_entries() {
        let and = Condition { entry: 10, kind: -1, values: [4, 7, 0, 0], flags: 0 };
        assert!(and.check().is_empty());
        assert_eq!(and.children(), vec![4, 7]);
        let upward = Condition { values: [4, 12, 0, 0], ..and.clone() };
        assert_eq!(upward.check().len(), 1);
        let empty = Condition { values: [4, 0, 0, 0], ..and.clone() };
        assert_eq!(empty.check().len(), 1, "an AND needs two");
        let missing = check_tree(&and, |entry| entry == 4);
        assert_eq!(missing, vec!["condition 7, which it names, does not exist".to_string()]);
    }

    #[test]
    fn a_repeated_test_is_found() {
        let one = Condition { entry: 1, kind: 8, values: [100, 0, 0, 0], flags: 0 };
        let two = Condition { entry: 2, ..one.clone() };
        let other = Condition { entry: 3, kind: 8, values: [101, 0, 0, 0], flags: 0 };
        assert_eq!(same_test(&two, [&one, &other]).map(|c| c.entry), Some(1));
        assert_eq!(same_test(&other, [&one, &two]), None);
    }
}
