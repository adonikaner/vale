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
//! ## The unique key
//!
//! A unique key over `(type, value1, value2, flags, value3, value4)`: two rows
//! may not test the same thing. [`same_test`] finds the row a new one would
//! repeat, so the editor can offer the existing one instead.
//!
//! ## A condition as a tree
//!
//! [`Node`] is a condition as a form edits it: tests, and groups that hold
//! when all or any of their members do. [`tree`] reads rows into one and
//! [`build`] writes one back as rows, naming every row that already tests a
//! part and creating the rest. [`category`] groups the types for a menu, and
//! [`phrase`] with [`render`] says a test as a sentence.
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
    /// Which players count: 0 any, 1 hostile, 2 friendly.
    Which,
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
    Type { id: 56, name: "Player nearby", about: "A player is within the radius: 0 any, 1 hostile, 2 friendly.", needs: "a unit target", values: [s("which players", Which), s("radius", Number), None, None] },
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

// ---------------------------------------------------------------------------
// Type categories and phrases
// ---------------------------------------------------------------------------

/// What a test is about, for grouping the types in a menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Player,
    Target,
    Source,
    Place,
    Instance,
    World,
    Object,
}

impl Category {
    /// Every category, in menu order.
    pub const ALL: [Category; 7] = [
        Category::Player,
        Category::Target,
        Category::Source,
        Category::Place,
        Category::Instance,
        Category::World,
        Category::Object,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::Player => "The player",
            Category::Target => "The target unit",
            Category::Source => "The source and what is near",
            Category::Place => "Where",
            Category::Instance => "The instance and map events",
            Category::World => "The server and the calendar",
            Category::Object => "A game object target",
        }
    }
}

/// The category of a type, or `None` for AND, OR, NOT and None, which are
/// not tests of their own, and for a type vmangos does not define.
pub fn category(id: i32) -> Option<Category> {
    use Category::*;
    Some(match id {
        2 | 3 | 5..=10 | 14 | 17 | 19 | 22 | 23 | 29 | 30 | 45 | 51 | 59 => Player,
        1 | 15 | 27 | 28 | 39..=43 | 46 => Target,
        13 | 16 | 20 | 21 | 25 | 31 | 32 | 37 | 38 | 44 | 52 | 56..=58 => Source,
        4 | 33 | 54 => Place,
        18 | 34..=36 | 47 | 50 => Instance,
        11 | 12 | 24 | 26 | 53 => World,
        48 | 49 | 55 => Object,
        _ => return None,
    })
}

/// What a type tests, as a sentence with its values left as placeholders;
/// [`render`] fills it. `None` for AND, OR and NOT, which a form draws as a
/// group, and for a type vmangos does not define.
///
/// The wording follows the value notes in vmangos' `Conditions.h`.
pub fn phrase(id: i32) -> Option<&'static str> {
    Some(match id {
        0 => "always",
        1 => "the target has an aura of {1} (effect index {2})",
        2 => "the player carries {2} of {1}",
        3 => "the player has {1} equipped",
        4 => "the target is in {1}",
        5 => "the player is {2} or better with {1}",
        6 => "the player is {1}",
        7 => "the player's {1} skill is at least {2}",
        8 => "the player has completed {1}",
        9 => "the player has {1} in the quest log[, {2}]",
        10 => "the player has an Argent Dawn commission",
        11 => "saved variable {1} is {3} {2}",
        12 => "game event {1} is running",
        13 => "the source is chasing a victim it cannot reach",
        14 => "the player is a[ {1}][ {2}]",
        15 => "the target's level is {2} {1}",
        16 => "the source is entry {1}[ or {2}][ or {3}][ or {4}]",
        17 => "the player {2?has not|has} learned {1}",
        18 => "the instance script of {1} answers yes to its question {2}",
        19 => "the player can accept {1}",
        20 => "a {3?dead|living} {1} is within {2} yards of the target{4?, not counting the target|}",
        21 => "{1} is within {2} yards of the target",
        22 => "the player has neither taken nor completed {1}",
        23 => "the player has {2} of {1}, bank included",
        24 => "the content patch is {2} {1}",
        25 => "the source and target are alive and within {2} yards (escort flags {1})",
        26 => "holiday {1} is active",
        27 => "the target is {1}",
        28 => "the target is a player{1? or owned by one|}",
        29 => "the player's {1} skill is below {2}",
        30 => "the player is {2} or worse with {1}",
        31 => "update field {1} of the source has flag {2}",
        32 => "the source's last waypoint is {2} {1}",
        33 => "the map is {1}",
        34 => "instance data {1} is {3} {2}",
        35 => "data {2} of map event {1} is {4} {3}",
        36 => "map event {1} is running",
        37 => "the source and the target can see each other",
        38 => "the target is {2} {1} yards from the source",
        39 => "the target is moving",
        40 => "the target has a pet",
        41 => "the target's health is {2} {1}%",
        42 => "the target's mana is {2} {1}%",
        43 => "the target is in combat",
        44 => "the target's reaction to the source is {2} {1}",
        45 => "the player is in a group",
        46 => "the target is alive",
        47 => "every extra target of map event {1} meets condition {2}",
        48 => "the game object is spawned",
        49 => "the game object's loot state is {1}",
        50 => "game object guid {1} exists and meets condition {2}",
        51 => "the player's honor rank is {2} {1}",
        52 => "the source is spawn {1}[ or {2}][ or {3}][ or {4}]",
        53 => "the server's local time is between hour {1} minute {2} and hour {3} minute {4}",
        54 => "the target is within {4} yards of {1}, {2}, {3}",
        55 => "the game object's state is {1}",
        56 => "{1} player is within {2} yards of the target",
        57 => "the source is in a creature group[ led by guid {1}]",
        58 => "the source's creature group is dead",
        59 => "the player has explored {1}",
        _ => return None,
    })
}

/// Fill a [`phrase`]. `{n}` is value n as `word(n - 1)` says it. `{n?a|b}`
/// is `a` when value n is not 0 and `b` when it is. Text between `[` and `]`
/// is left out when a value it names is 0.
pub fn render(phrase: &str, values: [i32; 4], word: impl Fn(usize) -> String) -> String {
    let slot = |text: &str| -> Option<usize> {
        let n = text.chars().next()?.to_digit(10)? as usize;
        (1..=4).contains(&n).then_some(n - 1)
    };
    let mut kept = String::new();
    let mut rest = phrase;
    while let Some(open) = rest.find('[') {
        kept.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find(']').unwrap_or(after.len());
        let inner = &after[..close];
        let empty = inner
            .match_indices('{')
            .any(|(at, _)| slot(&inner[at + 1..]).is_some_and(|n| values[n] == 0));
        if !empty {
            kept.push_str(inner);
        }
        rest = after.get(close + 1..).unwrap_or("");
    }
    kept.push_str(rest);

    let mut out = String::new();
    let mut rest = kept.as_str();
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}').unwrap_or(after.len());
        let inner = &after[..close];
        match (slot(inner), inner.get(1..).and_then(|tail| tail.strip_prefix('?'))) {
            (Some(n), Some(choice)) => {
                let (yes, no) = choice.split_once('|').unwrap_or((choice, ""));
                out.push_str(if values[n] != 0 { yes } else { no });
            }
            (Some(n), None) => out.push_str(&word(n)),
            (None, _) => {
                out.push('{');
                out.push_str(inner);
                out.push('}');
            }
        }
        rest = after.get(close + 1..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// A condition as a tree
// ---------------------------------------------------------------------------

/// A condition as a form edits it: tests, and groups of them that hold when
/// all or any of their members do.
///
/// The table stores the same thing as rows: an AND or an OR names two to four
/// other rows by entry, a NOT names one, and every row may reverse its own
/// result (`ConditionEntry::Meets`, `Conditions.cpp:119`). [`tree`] reads rows
/// into a tree and [`build`] writes a tree back as rows. A NOT becomes the
/// reverse flag of what it names, and an AND inside an AND (or an OR inside an
/// OR) with no flags of its own is merged into its parent, since the result
/// is the same.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// An AND (`any` false) or an OR (`any` true), with its row's flags:
    /// reversed, an AND is "not all of these" and an OR "none of these".
    Group { any: bool, flags: u8, members: Vec<Node> },
    /// One test: a type, its four values and its flags.
    Test { kind: i32, values: [i32; 4], flags: u8 },
    /// An entry no row holds.
    Missing(u32),
}

impl Node {
    /// An empty "all of these" group, which is what a new condition starts as.
    pub fn empty() -> Node {
        Node::Group { any: false, flags: 0, members: Vec::new() }
    }

    /// The node at `path`, each step an index into a group's members.
    pub fn at(&self, path: &[usize]) -> Option<&Node> {
        match path.split_first() {
            None => Some(self),
            Some((first, rest)) => match self {
                Node::Group { members, .. } => members.get(*first)?.at(rest),
                _ => None,
            },
        }
    }

    /// [`Self::at`], mutable.
    pub fn at_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        match path.split_first() {
            None => Some(self),
            Some((first, rest)) => match self {
                Node::Group { members, .. } => members.get_mut(*first)?.at_mut(rest),
                _ => None,
            },
        }
    }

    /// Remove the node at `path`. The root is not removed.
    pub fn remove(&mut self, path: &[usize]) -> Option<Node> {
        let (last, parent) = path.split_last()?;
        match self.at_mut(parent)? {
            Node::Group { members, .. } if *last < members.len() => Some(members.remove(*last)),
            _ => None,
        }
    }

    /// The flags of the node's own row.
    pub fn flags(&self) -> u8 {
        match self {
            Node::Group { flags, .. } | Node::Test { flags, .. } => *flags,
            Node::Missing(_) => 0,
        }
    }

    /// This node with its result reversed.
    pub fn reversed(self) -> Node {
        match self {
            Node::Group { any, flags, members } => Node::Group { any, flags: flags ^ REVERSE, members },
            Node::Test { kind, values, flags } => Node::Test { kind, values, flags: flags ^ REVERSE },
            missing => Node::Group { any: false, flags: REVERSE, members: vec![missing] },
        }
    }
}

/// How deep [`tree`] follows a chain of combining rows. The table cannot
/// hold a loop, since a child is a lower entry, but the project's rows are
/// not checked until they are written.
const DEEPEST: usize = 32;

/// Read the condition `entry` as a tree, through `lookup`, which answers a
/// row by entry.
pub fn tree(entry: u32, lookup: impl Fn(u32) -> Option<Condition>) -> Node {
    read(entry, &lookup, 0)
}

fn read(entry: u32, lookup: &impl Fn(u32) -> Option<Condition>, depth: usize) -> Node {
    let Some(row) = lookup(entry).filter(|_| depth < DEEPEST) else {
        return Node::Missing(entry);
    };
    match row.kind {
        -1 | -2 => {
            let any = row.kind == -2;
            let mut members = Vec::new();
            for child in row.values.iter().filter(|value| **value > 0) {
                match read(*child as u32, lookup, depth + 1) {
                    Node::Group { any: inner, flags: 0, members: theirs } if inner == any => members.extend(theirs),
                    node => members.push(node),
                }
            }
            Node::Group { any, flags: row.flags, members }
        }
        -3 => {
            let named = read(row.values[0].max(0) as u32, lookup, depth + 1).reversed();
            match row.flags {
                0 => named,
                flags => Node::Group { any: false, flags, members: vec![named] },
            }
        }
        kind => Node::Test { kind, values: row.values, flags: row.flags },
    }
}

/// What [`build`] answers: the entry the tree is, and the rows it had to
/// create, in the order they must be written.
#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    pub root: u32,
    pub created: Vec<Condition>,
}

/// Write a tree as rows. A row that already tests exactly what a node needs
/// (among `existing` and the rows created so far) is named rather than made
/// again, which the table's unique key requires anyway. A new row takes
/// `next` and up, and members are made before their group, so every group
/// names lower entries than itself, which the server requires.
///
/// A group of more than four members becomes nested groups of the same type
/// (AND or OR), four to a row. A group of one member is that member, reversed
/// when the group is. An empty group and a missing entry are refused.
pub fn build(node: &Node, existing: &[Condition], next: u32) -> Result<Built, String> {
    let mut building = Building { existing, created: Vec::new(), next };
    let root = building.node(node)?;
    Ok(Built { root, created: building.created })
}

struct Building<'a> {
    existing: &'a [Condition],
    created: Vec<Condition>,
    next: u32,
}

impl Building<'_> {
    fn node(&mut self, node: &Node) -> Result<u32, String> {
        match node {
            Node::Test { kind, values, flags } => Ok(self.row(*kind, *values, *flags)),
            Node::Missing(entry) => Err(format!("condition {entry} does not exist; remove it or replace it")),
            Node::Group { members, .. } if members.is_empty() => {
                Err("a group has no tests; add one to it or remove it".to_string())
            }
            Node::Group { flags, members, .. } if members.len() == 1 => match *flags {
                0 => self.node(&members[0]),
                REVERSE => self.node(&members[0].clone().reversed()),
                flags => {
                    let named = self.node(&members[0])?;
                    Ok(self.row(-3, [named as i32, 0, 0, 0], flags ^ REVERSE))
                }
            },
            Node::Group { any, flags, members } => {
                let kind = if *any { -2 } else { -1 };
                let mut entries = Vec::new();
                for member in members {
                    entries.push(self.node(member)?);
                }
                while entries.len() > 4 {
                    let four: Vec<u32> = entries.drain(..4).collect();
                    let joined = self.row(kind, values_of(&four), 0);
                    entries.insert(0, joined);
                }
                Ok(self.row(kind, values_of(&entries), *flags))
            }
        }
    }

    fn row(&mut self, kind: i32, values: [i32; 4], flags: u8) -> u32 {
        let wanted = Condition { entry: 0, kind, values, flags };
        if let Some(same) = same_test(&wanted, self.existing.iter().chain(self.created.iter())) {
            return same.entry;
        }
        let entry = self.next;
        self.next += 1;
        self.created.push(Condition { entry, ..wanted });
        entry
    }
}

/// Up to four entries as a row's values, 0 for the rest.
fn values_of(entries: &[u32]) -> [i32; 4] {
    let mut values = [0; 4];
    for (value, entry) in values.iter_mut().zip(entries) {
        *value = *entry as i32;
    }
    values
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

    /// Every type that is a test of its own has a category and a phrase.
    #[test]
    fn every_test_has_a_category_and_a_phrase() {
        for kind in TYPES.iter().filter(|kind| kind.id > 0) {
            assert!(category(kind.id).is_some(), "type {} has no category", kind.id);
            assert!(phrase(kind.id).is_some(), "type {} has no phrase", kind.id);
        }
        assert_eq!(category(-1), None);
    }

    /// A phrase drops an optional part whose value is 0, and chooses words
    /// by whether a value is 0.
    #[test]
    fn a_phrase_is_filled_from_the_values() {
        let word = |n: usize| format!("<{}>", n + 1);
        assert_eq!(render(phrase(9).unwrap(), [783, 0, 0, 0], word), "the player has <1> in the quest log");
        assert_eq!(render(phrase(9).unwrap(), [783, 2, 0, 0], word), "the player has <1> in the quest log, <2>");
        assert_eq!(render(phrase(17).unwrap(), [133, 1, 0, 0], word), "the player has not learned <1>");
        assert_eq!(render(phrase(16).unwrap(), [5, 6, 0, 0], word), "the source is entry <1> or <2>");
        assert_eq!(render("{x} and {9}", [0; 4], word), "{x} and {9}");
    }

    fn rows(list: &[Condition]) -> impl Fn(u32) -> Option<Condition> + '_ {
        move |entry| list.iter().find(|row| row.entry == entry).cloned()
    }

    fn test(kind: i32, value: i32) -> Node {
        Node::Test { kind, values: [value, 0, 0, 0], flags: 0 }
    }

    /// Rows read as a tree: a NOT is a reversed member, and an AND inside an
    /// AND is merged.
    #[test]
    fn rows_read_as_a_tree() {
        let table = [
            Condition { entry: 1, kind: 8, values: [100, 0, 0, 0], flags: 0 },
            Condition { entry: 2, kind: 6, values: [469, 0, 0, 0], flags: 0 },
            Condition { entry: 3, kind: -3, values: [2, 0, 0, 0], flags: 0 },
            Condition { entry: 4, kind: -1, values: [1, 3, 0, 0], flags: 0 },
            Condition { entry: 5, kind: 2, values: [2589, 1, 0, 0], flags: 0 },
            Condition { entry: 6, kind: -1, values: [4, 5, 99, 0], flags: 0 },
        ];
        let read = tree(6, rows(&table));
        assert_eq!(
            read,
            Node::Group {
                any: false,
                flags: 0,
                members: vec![
                    test(8, 100),
                    Node::Test { kind: 6, values: [469, 0, 0, 0], flags: REVERSE },
                    Node::Test { kind: 2, values: [2589, 1, 0, 0], flags: 0 },
                    Node::Missing(99),
                ],
            }
        );
    }

    /// Writing an unchanged tree names the rows it was read from and makes
    /// none; a changed member makes that test and a new group over it.
    #[test]
    fn a_tree_reuses_the_rows_that_already_test_it() {
        let table = [
            Condition { entry: 1, kind: 8, values: [100, 0, 0, 0], flags: 0 },
            Condition { entry: 2, kind: 6, values: [469, 0, 0, 0], flags: 0 },
            Condition { entry: 3, kind: -2, values: [1, 2, 0, 0], flags: 0 },
        ];
        let read = tree(3, rows(&table));
        assert_eq!(build(&read, &table, 10), Ok(Built { root: 3, created: vec![] }));

        let mut changed = read.clone();
        *changed.at_mut(&[1]).unwrap() = test(6, 67);
        let built = build(&changed, &table, 10).unwrap();
        assert_eq!(built.root, 11);
        assert_eq!(
            built.created,
            vec![
                Condition { entry: 10, kind: 6, values: [67, 0, 0, 0], flags: 0 },
                Condition { entry: 11, kind: -2, values: [1, 10, 0, 0], flags: 0 },
            ]
        );
    }

    /// Six members become an inner group of four and an outer group of three,
    /// and every group names lower entries than itself.
    #[test]
    fn a_long_group_is_nested_four_to_a_row() {
        let group = Node::Group { any: true, flags: REVERSE, members: (1..=6).map(|quest| test(8, quest)).collect() };
        let built = build(&group, &[], 100).unwrap();
        let groups: Vec<&Condition> = built.created.iter().filter(|row| row.kind == -2).collect();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].values, [100, 101, 102, 103]);
        assert_eq!(groups[0].flags, 0, "only the outer row is reversed");
        assert_eq!(groups[1].values, [groups[0].entry as i32, 104, 105, 0]);
        assert_eq!((groups[1].entry, groups[1].flags), (built.root, REVERSE));
        for row in &built.created {
            assert!(row.check().is_empty(), "{row:?}: {:?}", row.check());
        }
        // Read back, the nesting merges and the tree is the same.
        assert_eq!(tree(built.root, rows(&built.created)), group);
    }

    /// A group of one is its member, reversed with the group; an empty group
    /// and a missing entry are refused.
    #[test]
    fn a_group_of_one_is_its_member() {
        let one = Node::Group { any: false, flags: REVERSE, members: vec![test(8, 100)] };
        let built = build(&one, &[], 5).unwrap();
        assert_eq!(built.created, vec![Condition { entry: 5, kind: 8, values: [100, 0, 0, 0], flags: REVERSE }]);
        assert!(build(&Node::empty(), &[], 5).is_err());
        let missing = Node::Group { any: false, flags: 0, members: vec![test(8, 1), Node::Missing(7)] };
        assert!(build(&missing, &[], 5).is_err());
    }
}
