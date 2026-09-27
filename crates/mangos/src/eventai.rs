//! `creature_ai_events`: what a creature running `EventAI` reacts to, and
//! which scripts it runs when it does.
//!
//! ## One row is one trigger
//!
//! `CreatureEventAIMgr::LoadCreatureEventAI_Events` (`CreatureEventAIMgr.cpp:44`)
//! reads fifteen columns. `event_type` is one of 37 triggers ([`EVENT_TYPES`],
//! `CreatureEventAI.h:40`), and what `event_param1` to `event_param4` mean
//! depends on it: a timer's four are its initial and repeat ranges, a health
//! threshold's are the band and the repeat, a received emote's first is the
//! emote. [`EventType`] names each parameter in the header's own words.
//! `action1_script` to `action3_script` are `creature_ai_scripts` ids
//! ([`crate::scripts`]); the event runs each in turn, or one of them at
//! random with the flag set.
//!
//! `event_inverse_phase_mask` is the phases the event does not fire in: bit
//! `n` set means phase `n` is excluded. `event_chance` is a percentage.
//! `event_flags` is [`EVENT_FLAGS`]. `condition_id` is a row of `conditions`.
//!
//! ## Which creatures read this table
//!
//! Only a creature whose `creature_template.ai_name` is `EventAI`
//! (`CreatureAISelector.cpp:52`). A row for any other creature is loaded and
//! never fired, and the loader says so at start. 5,167 templates on the
//! reference database use `EventAI`; 3,453 have events.
//!
//! ## Ids
//!
//! `id` is the primary key and is the row's own. The reference database
//! numbers a creature's events `creature_id * 100 + n`, `n` from 1, which is
//! a convention and not a rule; [`next_id`] follows it and falls past 99 to
//! the next free id above.
//!
//! ## Live on a reload
//!
//! `.reload creature_ai_events` clears the event store, re-reads
//! `creature_ai_scripts` and then this table (`ServerCommands.cpp:1529`), so
//! a row created, edited or removed is live for the next creature that
//! spawns or resets. A creature already in the world keeps the events it was
//! given until it respawns.

use crate::row::{Assignment, Key, Life};

pub use crate::schema::{mask_words, value_word, Bit, Column, Group, Kind, Row, RowValue, Value};

pub const TABLE: &str = "creature_ai_events";

/// The static name for the table, for a caller resolving a name read out of
/// a file.
pub fn table_named(name: &str) -> Option<&'static str> {
    (name == TABLE).then_some(TABLE)
}

/// One event parameter: its name for one event type, and what a form draws
/// it as.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub about: &'static str,
}

const fn p(name: &'static str, kind: Kind, about: &'static str) -> Option<Param> {
    Some(Param { name, kind, about })
}

const fn ms(name: &'static str, about: &'static str) -> Option<Param> {
    Some(Param { name, kind: Kind::Millis, about })
}

const fn n(name: &'static str, about: &'static str) -> Option<Param> {
    Some(Param { name, kind: Kind::Unsigned, about })
}

/// The two repeat parameters most events end with.
const REPEAT: [Option<Param>; 2] = [
    ms("repeat_min", "least milliseconds before it can fire again"),
    ms("repeat_max", "most"),
];

const fn repeats(first: Option<Param>, second: Option<Param>) -> [Option<Param>; 4] {
    [first, second, REPEAT[0], REPEAT[1]]
}

/// One of the 37 triggers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventType {
    pub value: u32,
    pub name: &'static str,
    /// What the four parameters mean, or `None` for one not read.
    pub params: [Option<Param>; 4],
    pub about: &'static str,
}

const fn e(value: u32, name: &'static str, params: [Option<Param>; 4], about: &'static str) -> EventType {
    EventType { value, name, params, about }
}

const NONE: [Option<Param>; 4] = [None, None, None, None];

/// The 37, in `CreatureEventAI.h:40`'s order.
pub const EVENT_TYPES: [EventType; 37] = [
    e(0, "Timer in combat", [ms("initial_min", "least milliseconds into combat before the first firing"), ms("initial_max", "most"), REPEAT[0], REPEAT[1]], "fires on a timer while fighting"),
    e(1, "Timer out of combat", [ms("initial_min", "least milliseconds before the first firing"), ms("initial_max", "most"), REPEAT[0], REPEAT[1]], "fires on a timer while not fighting"),
    e(2, "Health", repeats(n("hp_max_percent", "fires when health is at or under this"), n("hp_min_percent", "and over this")), "the creature's own health crosses into a band"),
    e(3, "Mana", repeats(n("mana_max_percent", ""), n("mana_min_percent", "")), "the creature's own mana crosses into a band"),
    e(4, "Aggro", NONE, "combat starts"),
    e(5, "Kill", [REPEAT[0], REPEAT[1], p("player_only", Kind::Choice(&crate::scripts::NO_YES), "only when the victim is a player"), None], "the creature kills something"),
    e(6, "Death", NONE, "the creature dies"),
    e(7, "Evade", NONE, "the creature gives up and returns home"),
    e(8, "Hit by spell", repeats(p("spell_id", Kind::Ref("Spell"), "one spell, or 0 for any"), n("school_mask", "or 0 for any")), "a spell lands on the creature"),
    e(9, "Target in range", repeats(n("min_distance", "yards"), n("max_distance", "yards")), "the victim is within a band of distance"),
    e(10, "Line of sight out of combat", repeats(n("reaction", "0 hostile, 1 friendly"), n("max_range", "yards")), "a unit comes into view while not fighting"),
    e(11, "Spawned", NONE, "the creature spawns or respawns"),
    e(12, "Target health", repeats(n("hp_max_percent", ""), n("hp_min_percent", "")), "the victim's health crosses into a band"),
    e(13, "Target casting", [REPEAT[0], REPEAT[1], None, None], "the victim is casting"),
    e(14, "Friendly hurt", repeats(n("hp_deficit", "health missing"), n("radius", "yards")), "a friend nearby is missing at least this much health"),
    e(15, "Friendly crowd controlled", repeats(n("dispel_type", ""), n("radius", "yards")), "a friend nearby is under a crowd control of this type"),
    e(16, "Friendly missing buff", repeats(p("spell_id", Kind::Ref("Spell"), ""), n("radius", "yards")), "a friend nearby lacks this aura"),
    e(17, "Summoned unit", [p("creature_id", Kind::Ref("creature_template"), "or 0 for any"), REPEAT[0], REPEAT[1], None], "the creature summons a unit"),
    e(18, "Target mana", repeats(n("mana_max_percent", ""), n("mana_min_percent", "")), "the victim's mana crosses into a band"),
    e(19, "Quest accepted", [p("quest_id", Kind::Ref("quest_template"), ""), None, None, None], "a player accepts this quest from the creature"),
    e(20, "Quest completed", [p("quest_id", Kind::Ref("quest_template"), ""), None, None, None], "a player completes this quest at the creature"),
    e(21, "Reached home", NONE, "the creature arrives home after an evade"),
    e(22, "Received emote", [p("emote_id", Kind::Ref("EmotesText"), "the text emote a player made at it"), n("condition", "a condition type, or 0"), n("condition_value1", ""), n("condition_value2", "")], "a player emotes at the creature"),
    e(23, "Aura", repeats(p("spell_id", Kind::Ref("Spell"), ""), n("stacks", "at least this many")), "the creature carries this aura"),
    e(24, "Target aura", repeats(p("spell_id", Kind::Ref("Spell"), ""), n("stacks", "at least this many")), "the victim carries this aura"),
    e(25, "Summoned unit died", [p("creature_id", Kind::Ref("creature_template"), "or 0 for any"), REPEAT[0], REPEAT[1], None], "a unit the creature summoned dies"),
    e(26, "Summoned unit despawned", [p("creature_id", Kind::Ref("creature_template"), "or 0 for any"), REPEAT[0], REPEAT[1], None], "a unit the creature summoned despawns"),
    e(27, "Missing aura", repeats(p("spell_id", Kind::Ref("Spell"), ""), n("stacks", "fewer than this many")), "the creature lacks this aura"),
    e(28, "Target missing aura", repeats(p("spell_id", Kind::Ref("Spell"), ""), n("stacks", "fewer than this many")), "the victim lacks this aura"),
    e(29, "Movement inform", repeats(n("motion_type", "MovementGeneratorType"), n("point_id", "the waypoint or point reached")), "a movement generator reports a point reached"),
    e(30, "Left combat", NONE, "combat ends"),
    e(31, "Script", [n("event_id", "what SCRIPT_COMMAND_SEND_SCRIPT_EVENT sends"), n("data", "and its data, or 0 for any"), None, None], "a script sends the creature an event"),
    e(32, "Group member died", [p("creature_id", Kind::Ref("creature_template"), "or 0 for any"), p("is_leader", Kind::Choice(&crate::scripts::NO_YES), ""), None, None], "a member of the creature's group dies"),
    e(33, "Victim rooted", [REPEAT[0], REPEAT[1], None, None], "the victim is rooted"),
    e(34, "Hit by aura", repeats(n("aura_type", "AuraType"), None), "an aura of this type lands on the creature"),
    e(35, "Stealth alert", [REPEAT[0], REPEAT[1], None, None], "a stealthed player is noticed nearby"),
    e(36, "Spell hit target", repeats(p("spell_id", Kind::Ref("Spell"), "one spell, or 0 for any"), n("school_mask", "or 0 for any")), "a spell the creature cast lands"),
];

/// [`EVENT_TYPES`] as an enumeration, for a form's menu.
pub const EVENT_TYPE_VALUES: [Value; 37] = {
    let mut out = [Value { value: 0, name: "" }; 37];
    let mut at = 0;
    while at < 37 {
        out[at] = Value { value: EVENT_TYPES[at].value, name: EVENT_TYPES[at].name };
        at += 1;
    }
    out
};

/// One event type, by value.
pub fn event_type(value: u32) -> Option<&'static EventType> {
    EVENT_TYPES.iter().find(|kind| kind.value == value)
}

/// `EventFlags`, from `CreatureEventAI.h:83`.
pub const EVENT_FLAGS: [Bit; 5] = [
    Bit { bit: 0x01, name: "Repeatable", about: "fires again after its repeat delay; without this, once per spawn" },
    Bit { bit: 0x02, name: "Random action", about: "runs one of its scripts, chosen at random, rather than all three" },
    Bit { bit: 0x04, name: "Not while casting", about: "does not fire while the creature is casting" },
    Bit { bit: 0x08, name: "Check the result", about: "does not go on cooldown when the scripts fail" },
    Bit { bit: 0x10, name: "Debug builds only", about: "" },
];

/// The phases a mask can name, for `event_inverse_phase_mask`. Bit `n` set
/// means the event does not fire in phase `n`.
pub const PHASES: [Bit; 16] = [
    Bit { bit: 1 << 0, name: "not in phase 0", about: "" },
    Bit { bit: 1 << 1, name: "not in phase 1", about: "" },
    Bit { bit: 1 << 2, name: "not in phase 2", about: "" },
    Bit { bit: 1 << 3, name: "not in phase 3", about: "" },
    Bit { bit: 1 << 4, name: "not in phase 4", about: "" },
    Bit { bit: 1 << 5, name: "not in phase 5", about: "" },
    Bit { bit: 1 << 6, name: "not in phase 6", about: "" },
    Bit { bit: 1 << 7, name: "not in phase 7", about: "" },
    Bit { bit: 1 << 8, name: "not in phase 8", about: "" },
    Bit { bit: 1 << 9, name: "not in phase 9", about: "" },
    Bit { bit: 1 << 10, name: "not in phase 10", about: "" },
    Bit { bit: 1 << 11, name: "not in phase 11", about: "" },
    Bit { bit: 1 << 12, name: "not in phase 12", about: "" },
    Bit { bit: 1 << 13, name: "not in phase 13", about: "" },
    Bit { bit: 1 << 14, name: "not in phase 14", about: "" },
    Bit { bit: 1 << 15, name: "not in phase 15", about: "" },
];

/// The fifteen columns, in the loader's `SELECT` order with `comment` after.
pub const COLUMNS: [Column; 15] = [
    Column { name: "id", kind: Kind::Key, group: Group::Identity, about: "the event's own id" },
    Column { name: "creature_id", kind: Kind::Ref("creature_template"), group: Group::Identity, about: "the creature it belongs to" },
    Column { name: "condition_id", kind: Kind::Unsigned, group: Group::Advanced, about: "a row of conditions the event fires only under, or 0" },
    Column { name: "event_type", kind: Kind::Choice(&EVENT_TYPE_VALUES), group: Group::Behaviour, about: "what fires it" },
    Column { name: "event_inverse_phase_mask", kind: Kind::Flags(&PHASES), group: Group::Behaviour, about: "the phases it does not fire in" },
    Column { name: "event_chance", kind: Kind::Unsigned, group: Group::Behaviour, about: "percent chance to fire when triggered" },
    Column { name: "event_flags", kind: Kind::Flags(&EVENT_FLAGS), group: Group::Behaviour, about: "repeatable, random action, not while casting" },
    Column { name: "event_param1", kind: Kind::Signed, group: Group::Behaviour, about: "named by the event type" },
    Column { name: "event_param2", kind: Kind::Signed, group: Group::Behaviour, about: "named by the event type" },
    Column { name: "event_param3", kind: Kind::Signed, group: Group::Behaviour, about: "named by the event type" },
    Column { name: "event_param4", kind: Kind::Signed, group: Group::Behaviour, about: "named by the event type" },
    Column { name: "action1_script", kind: Kind::Ref(crate::scripts::CREATURE_AI), group: Group::Behaviour, about: "the first script run, or 0" },
    Column { name: "action2_script", kind: Kind::Ref(crate::scripts::CREATURE_AI), group: Group::Behaviour, about: "the second, or 0" },
    Column { name: "action3_script", kind: Kind::Ref(crate::scripts::CREATURE_AI), group: Group::Behaviour, about: "the third, or 0" },
    Column { name: "comment", kind: Kind::Text, group: Group::Advanced, about: "for the person; the server never reads it" },
];

/// One column, by name.
pub fn column(name: &str) -> Option<&'static Column> {
    COLUMNS.iter().find(|column| column.name == name)
}

/// The key of one row: the id.
pub fn key(id: u32) -> Key {
    Key::one("id", u64::from(id))
}

/// One event as the server reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: u32,
    pub creature_id: u32,
    pub condition_id: u32,
    pub event_type: u32,
    pub phase_mask: u32,
    pub chance: u32,
    pub flags: u32,
    pub params: [i32; 4],
    pub scripts: [u32; 3],
    pub comment: String,
}

impl Event {
    /// A new event of a creature: a repeating combat timer at five to ten
    /// seconds, with no script yet.
    pub fn new(id: u32, creature_id: u32) -> Event {
        Event {
            id,
            creature_id,
            condition_id: 0,
            event_type: 0,
            phase_mask: 0,
            chance: 100,
            flags: 1,
            params: [5000, 10000, 5000, 10000],
            scripts: [0; 3],
            comment: String::new(),
        }
    }

    /// A new event of one trigger type, with no script yet. The two timers
    /// start as [`Event::new`]'s five-to-ten-second repeating timer. A health
    /// or mana band starts at 50% and fires once, since a repeating band with
    /// no repeat delay fires on every update while the creature is in it.
    /// Every other type starts with its parameters at 0, which each reads as
    /// "any", and fires once, which is what the reference database's rows for
    /// aggro, death and evade do.
    pub fn of_type(id: u32, creature_id: u32, event_type: u32) -> Event {
        let timer = Event::new(id, creature_id);
        match event_type {
            0 | 1 => Event { event_type, ..timer },
            2 | 3 | 12 | 18 => Event { event_type, params: [50, 0, 0, 0], flags: 0, ..timer },
            _ => Event { event_type, params: [0; 4], flags: 0, ..timer },
        }
    }

    pub fn key(&self) -> Key {
        key(self.id)
    }

    /// One row as the database answered it, or `None` for a row with no id.
    pub fn from_row(row: &Row) -> Option<Event> {
        let count = |name: &str| row.integer(name).unwrap_or(0).max(0) as u32;
        let int = |name: &str| row.integer(name).unwrap_or(0) as i32;
        Some(Event {
            id: row.integer("id")? as u32,
            creature_id: count("creature_id"),
            condition_id: count("condition_id"),
            event_type: count("event_type"),
            phase_mask: count("event_inverse_phase_mask"),
            chance: count("event_chance"),
            flags: count("event_flags"),
            params: [int("event_param1"), int("event_param2"), int("event_param3"), int("event_param4")],
            scripts: [count("action1_script"), count("action2_script"), count("action3_script")],
            comment: row.text("comment").unwrap_or_default().to_string(),
        })
    }

    /// Every editable column as a SQL literal, which is what a row this
    /// project creates carries.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "creature_id", value: self.creature_id.to_string() },
            Assignment { column: "condition_id", value: self.condition_id.to_string() },
            Assignment { column: "event_type", value: self.event_type.to_string() },
            Assignment { column: "event_inverse_phase_mask", value: self.phase_mask.to_string() },
            Assignment { column: "event_chance", value: self.chance.to_string() },
            Assignment { column: "event_flags", value: self.flags.to_string() },
            Assignment { column: "event_param1", value: self.params[0].to_string() },
            Assignment { column: "event_param2", value: self.params[1].to_string() },
            Assignment { column: "event_param3", value: self.params[2].to_string() },
            Assignment { column: "event_param4", value: self.params[3].to_string() },
            Assignment { column: "action1_script", value: self.scripts[0].to_string() },
            Assignment { column: "action2_script", value: self.scripts[1].to_string() },
            Assignment { column: "action3_script", value: self.scripts[2].to_string() },
            Assignment { column: "comment", value: crate::sql::text(&self.comment) },
        ]
    }

    /// One column's value as a form shows it.
    pub fn get(&self, column: &str) -> String {
        match column {
            "id" => self.id.to_string(),
            "creature_id" => self.creature_id.to_string(),
            "condition_id" => self.condition_id.to_string(),
            "event_type" => self.event_type.to_string(),
            "event_inverse_phase_mask" => self.phase_mask.to_string(),
            "event_chance" => self.chance.to_string(),
            "event_flags" => self.flags.to_string(),
            "event_param1" => self.params[0].to_string(),
            "event_param2" => self.params[1].to_string(),
            "event_param3" => self.params[2].to_string(),
            "event_param4" => self.params[3].to_string(),
            "action1_script" => self.scripts[0].to_string(),
            "action2_script" => self.scripts[1].to_string(),
            "action3_script" => self.scripts[2].to_string(),
            "comment" => self.comment.clone(),
            _ => String::new(),
        }
    }

    /// Set one column from a form's text, or a SQL literal for `comment`.
    /// `false` for a column the row does not have or a value that does not
    /// read.
    pub fn set(&mut self, column: &str, value: &str) -> bool {
        let value = value.trim();
        let count = || value.parse::<i64>().ok().map(|n| n.max(0) as u32);
        let int = || value.parse::<i64>().ok().map(|n| n as i32);
        match column {
            "creature_id" => self.creature_id = match count() { Some(n) => n, None => return false },
            "condition_id" => self.condition_id = match count() { Some(n) => n, None => return false },
            "event_type" => self.event_type = match count() { Some(n) => n, None => return false },
            "event_inverse_phase_mask" => self.phase_mask = match count() { Some(n) => n, None => return false },
            "event_chance" => self.chance = match count() { Some(n) => n, None => return false },
            "event_flags" => self.flags = match count() { Some(n) => n, None => return false },
            "event_param1" => self.params[0] = match int() { Some(n) => n, None => return false },
            "event_param2" => self.params[1] = match int() { Some(n) => n, None => return false },
            "event_param3" => self.params[2] = match int() { Some(n) => n, None => return false },
            "event_param4" => self.params[3] = match int() { Some(n) => n, None => return false },
            "action1_script" => self.scripts[0] = match count() { Some(n) => n, None => return false },
            "action2_script" => self.scripts[1] = match count() { Some(n) => n, None => return false },
            "action3_script" => self.scripts[2] = match count() { Some(n) => n, None => return false },
            "comment" => self.comment = unquote(value),
            _ => return false,
        }
        true
    }

    /// The trigger in one line, for a list: the type's name and the
    /// parameters it reads.
    pub fn summary(&self) -> String {
        let Some(kind) = event_type(self.event_type) else {
            return format!("event type {}", self.event_type);
        };
        let mut parts: Vec<String> = Vec::new();
        for (at, param) in kind.params.iter().enumerate() {
            let Some(param) = param else { continue };
            let value = self.params[at];
            let shown = match param.kind {
                Kind::Millis => crate::schema::millis_words(i64::from(value)),
                Kind::Choice(values) => value_word(values, value.max(0) as u32),
                _ => value.to_string(),
            };
            parts.push(format!("{} {shown}", param.name));
        }
        match parts.is_empty() {
            true => kind.name.to_string(),
            false => format!("{}: {}", kind.name, parts.join(", ")),
        }
    }

    /// The trigger as a sentence: `In combat, after 5 s to 10 s, then every
    /// 5 s to 10 s`, `Health at or below 15%`, `Hit by Frostbolt`. `names`
    /// resolves the spells, creatures, quests and emotes a parameter names.
    /// A type with no sentence of its own falls back to [`Event::summary`].
    pub fn sentence(&self, names: crate::schema::Names<'_>) -> String {
        let p = self.params;
        let millis = |value: i32| value.max(0) as u64;
        let range = |least: i32, most: i32| crate::schema::span_range(millis(least), millis(most));
        let repeats = self.flags & 0x01 != 0;
        let every = |least: i32, most: i32| match repeats && (least > 0 || most > 0) {
            true => format!(", then every {}", range(least, most)),
            false => String::new(),
        };
        let named = |table: &str, id: i32, any: &str| match id {
            id if id <= 0 => any.to_string(),
            id => names(table, id as u32).unwrap_or_else(|| format!("{} {id}", crate::schema::ref_word(table))),
        };
        let band = |what: &str, most: i32, least: i32| match least <= 0 {
            true => format!("{what} at or below {most}%"),
            false => format!("{what} between {least}% and {most}%"),
        };
        let stacks = |count: i32, word: &str| match count > 1 {
            true => format!(" ({word} {count} stacks)"),
            false => String::new(),
        };
        match self.event_type {
            0 => format!("In combat, after {}{}", range(p[0], p[1]), every(p[2], p[3])),
            1 => format!("Out of combat, after {}{}", range(p[0], p[1]), every(p[2], p[3])),
            2 => format!("{}{}", band("Health", p[0], p[1]), every(p[2], p[3])),
            3 => format!("{}{}", band("Mana", p[0], p[1]), every(p[2], p[3])),
            4 => "Combat starts".to_string(),
            5 => format!(
                "Kills {}{}",
                match p[2] {
                    0 => "anything",
                    _ => "a player",
                },
                every(p[0], p[1])
            ),
            6 => "Dies".to_string(),
            7 => "Evades".to_string(),
            8 => format!("Hit by {}{}", named("Spell", p[0], "any spell"), every(p[2], p[3])),
            9 => format!("Victim {} to {} yards away{}", p[0], p[1], every(p[2], p[3])),
            10 => format!(
                "A {} unit comes within {} yards out of combat{}",
                match p[0] {
                    0 => "hostile",
                    _ => "friendly",
                },
                p[1],
                every(p[2], p[3])
            ),
            11 => "Spawns".to_string(),
            12 => format!("{}{}", band("Victim's health", p[0], p[1]), every(p[2], p[3])),
            13 => format!("Victim is casting{}", every(p[0], p[1])),
            14 => format!("A friend within {} yards is missing {} health{}", p[1], p[0], every(p[2], p[3])),
            15 => format!("A friend within {} yards is crowd controlled{}", p[1], every(p[2], p[3])),
            16 => format!("A friend within {} yards lacks {}{}", p[1], named("Spell", p[0], "an aura"), every(p[2], p[3])),
            17 => format!("Summons {}{}", named("creature_template", p[0], "a unit"), every(p[1], p[2])),
            18 => format!("{}{}", band("Victim's mana", p[0], p[1]), every(p[2], p[3])),
            19 => format!("A player accepts {}", named("quest_template", p[0], "a quest")),
            20 => format!("A player completes {}", named("quest_template", p[0], "a quest")),
            21 => "Reaches home after evading".to_string(),
            22 => format!("A player emotes {} at it", named("EmotesText", p[0], "anything")),
            23 => format!("Has {}{}{}", named("Spell", p[0], "an aura"), stacks(p[1], "at least"), every(p[2], p[3])),
            24 => format!("Victim has {}{}{}", named("Spell", p[0], "an aura"), stacks(p[1], "at least"), every(p[2], p[3])),
            25 => format!("{} it summoned dies{}", named("creature_template", p[0], "A unit"), every(p[1], p[2])),
            26 => format!("{} it summoned despawns{}", named("creature_template", p[0], "A unit"), every(p[1], p[2])),
            27 => format!("Lacks {}{}{}", named("Spell", p[0], "an aura"), stacks(p[1], "fewer than"), every(p[2], p[3])),
            28 => format!("Victim lacks {}{}{}", named("Spell", p[0], "an aura"), stacks(p[1], "fewer than"), every(p[2], p[3])),
            29 => format!("Reaches point {} (movement type {})", p[1], p[0]),
            30 => "Leaves combat".to_string(),
            31 => format!("A script sends event {}", p[0]),
            32 => format!("{} in its group dies", named("creature_template", p[0], "A member")),
            33 => format!("Victim is rooted{}", every(p[0], p[1])),
            34 => format!("An aura of type {} lands on it{}", p[0], every(p[2], p[3])),
            35 => format!("Notices a stealthed player{}", every(p[0], p[1])),
            36 => format!("Its {} lands{}", named("Spell", p[0], "spell"), every(p[2], p[3])),
            _ => self.summary(),
        }
    }

    /// The conditions on the trigger that the sentence leaves out, each as a
    /// short phrase: the chance when it is under 100, `once` when the event
    /// does not repeat, and the other flags, phases and condition in words.
    pub fn terms(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.chance != 100 {
            out.push(format!("{}%", self.chance));
        }
        if self.flags & 0x01 == 0 {
            out.push("once".to_string());
        }
        if self.flags & 0x04 != 0 {
            out.push("not while casting".to_string());
        }
        if self.flags & 0x08 != 0 {
            out.push("checks the result".to_string());
        }
        if self.phase_mask != 0 {
            let phases: Vec<String> = (0..32)
                .filter(|bit| self.phase_mask & (1 << bit) != 0)
                .map(|bit: u32| bit.to_string())
                .collect();
            out.push(format!("not in phase {}", phases.join(", ")));
        }
        if self.condition_id != 0 {
            out.push(format!("condition {}", self.condition_id));
        }
        out
    }

    /// Whether the event runs one of its scripts at random rather than each in
    /// turn.
    pub fn random_action(&self) -> bool {
        self.flags & 0x02 != 0
    }

    /// The same trigger for another creature under another id, with the same
    /// scripts. The comment's lead, the creature name the reference database
    /// writes before ` - `, becomes `label`.
    pub fn copied_for(&self, id: u32, creature_id: u32, label: &str) -> Event {
        Event {
            id,
            creature_id,
            comment: renamed(&self.comment, label),
            ..self.clone()
        }
    }
}

/// A comment with its lead replaced: the reference database starts a comment
/// with the creature's name and ` - `, so `Huklah - Flee at 15% HP` copied to
/// a Stormwind City Guard reads `Stormwind City Guard - Flee at 15% HP`. A
/// comment without that lead is returned unchanged.
pub fn renamed(comment: &str, label: &str) -> String {
    match comment.split_once(" - ") {
        Some((_, rest)) if !label.is_empty() => format!("{label} - {rest}"),
        _ => comment.to_string(),
    }
}

/// The inside of a quoted SQL literal, or the text itself when it is not one.
/// The store holds a text column as the literal `sql::text` wrote.
fn unquote(literal: &str) -> String {
    let Some(inner) = literal.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')) else {
        return literal.to_string();
    };
    inner.replace("\\'", "'").replace("\\\\", "\\")
}

/// The next free id for a creature's event, on the reference database's own
/// convention: `creature_id * 100 + n`, the first `n` from 1 to 99 not in
/// `taken`; past 99, the first free id above `creature_id * 100`.
pub fn next_id(creature_id: u32, taken: &[u32]) -> u32 {
    let base = creature_id.saturating_mul(100);
    let mut n = 1;
    loop {
        let id = base.saturating_add(n);
        if !taken.contains(&id) {
            return id;
        }
        n += 1;
    }
}

/// The statements a save emits for one row: an `UPDATE` for a row it edits,
/// a `DELETE`/`INSERT` pair for one it creates, and a `DELETE` for one it
/// removes.
pub fn statements(key: &Key, life: Life, changes: &[Assignment]) -> Vec<String> {
    match life {
        Life::Update => crate::row::update(TABLE, key, changes).into_iter().collect(),
        Life::Insert => match crate::row::insert(TABLE, key, changes) {
            Some(statement) => vec![crate::row::delete(TABLE, key), statement],
            None => Vec::new(),
        },
        Life::Delete => vec![crate::row::delete(TABLE, key)],
    }
}

/// Every event of one creature, in id order.
pub fn events_query(creature_id: u32) -> String {
    format!(
        "SELECT * FROM {} WHERE `creature_id` = {creature_id} ORDER BY `id`",
        crate::sql::name(TABLE)
    )
}

/// Every creature's events whose comment holds `term`, or whose id or
/// creature id is `term`: the search behind adding an existing event.
pub fn search_query(term: &str, limit: usize) -> String {
    let like = crate::sql::text(&format!("%{}%", term.trim()));
    let by_id = match term.trim().parse::<u32>() {
        Ok(id) => format!(" OR `id` = {id} OR `creature_id` = {id}"),
        Err(_) => String::new(),
    };
    format!(
        "SELECT * FROM {} WHERE `comment` LIKE {like}{by_id} ORDER BY `id` LIMIT {limit}",
        crate::sql::name(TABLE)
    )
}

/// The row a key names, which an undo is taken from.
pub fn row_query(key: &Key) -> String {
    format!("SELECT * FROM {} WHERE {} LIMIT 1", crate::sql::name(TABLE), key.where_clause())
}

/// Whether a row is already there, which an apply asks before it creates one.
pub fn exists_query(key: &Key) -> String {
    format!("SELECT 1 FROM {} WHERE {} LIMIT 1", crate::sql::name(TABLE), key.where_clause())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The schema is the loader's `SELECT`, column for column, with `comment`
    /// after it.
    #[test]
    fn every_column_is_the_servers_own_in_the_servers_own_order() {
        const SELECTED: &str = "id, creature_id, condition_id, event_type, event_inverse_phase_mask, \
                                event_chance, event_flags, event_param1, event_param2, event_param3, \
                                event_param4, action1_script, action2_script, action3_script";
        let ours: Vec<&str> = COLUMNS.iter().map(|column| column.name).filter(|name| *name != "comment").collect();
        let theirs: Vec<&str> = SELECTED.split(", ").map(str::trim).collect();
        assert_eq!(ours, theirs);
        for (at, kind) in EVENT_TYPES.iter().enumerate() {
            assert_eq!(kind.value as usize, at, "{}", kind.name);
            assert_eq!(EVENT_TYPE_VALUES[at].name, kind.name);
        }
    }

    /// A row reads back as the server reads it, and its summary names the
    /// parameters in the header's words.
    #[test]
    fn an_event_is_read_and_summarised() {
        let mut row = Row::new();
        for (column, value) in [
            ("id", "6801"), ("creature_id", "68"), ("condition_id", "3"), ("event_type", "22"),
            ("event_inverse_phase_mask", "0"), ("event_chance", "100"), ("event_flags", "1"),
            ("event_param1", "58"), ("event_param2", "0"), ("event_param3", "0"), ("event_param4", "0"),
            ("action1_script", "6801"), ("action2_script", "0"), ("action3_script", "0"),
            ("comment", "Bow on Kiss"),
        ] {
            row.insert(column.to_string(), Some(value.to_string()));
        }
        let event = Event::from_row(&row).expect("an event");
        assert_eq!(event.scripts, [6801, 0, 0]);
        assert_eq!(event.summary(), "Received emote: emote_id 58, condition 0, condition_value1 0, condition_value2 0");
        let timer = Event::new(6802, 68);
        assert_eq!(timer.summary(), "Timer in combat: initial_min 5s, initial_max 10s, repeat_min 5s, repeat_max 10s");
        let health = Event { event_type: 2, params: [30, 0, 5000, 5000], ..timer.clone() };
        assert!(health.summary().starts_with("Health: hp_max_percent 30, hp_min_percent 0"), "{}", health.summary());
        assert_eq!(Event { event_type: 4, ..timer }.summary(), "Aggro");
    }

    /// A created row is a `DELETE` and an `INSERT` naming every column; an
    /// edit is one `UPDATE`; a removal one `DELETE`.
    #[test]
    fn a_row_becomes_the_statements_it_means() {
        let event = Event::new(6803, 68);
        let created = statements(&event.key(), Life::Insert, &event.assignments());
        assert_eq!(created.len(), 2);
        assert_eq!(created[0], "DELETE FROM `creature_ai_events` WHERE `id` = 6803;");
        assert!(created[1].starts_with("INSERT INTO `creature_ai_events` (`id`, `creature_id`, `condition_id`, `event_type`"), "{}", created[1]);
        assert!(created[1].contains("VALUES (6803, 68, 0, 0, 0, 100, 1, 5000, 10000, 5000, 10000, 0, 0, 0, '');"), "{}", created[1]);
        let edited = statements(&key(6801), Life::Update, &[Assignment { column: "event_chance", value: "50".into() }]);
        assert_eq!(edited, vec!["UPDATE `creature_ai_events` SET `event_chance` = 50 WHERE `id` = 6801;"]);
        assert_eq!(statements(&key(6801), Life::Delete, &[]), vec!["DELETE FROM `creature_ai_events` WHERE `id` = 6801;"]);
    }

    /// A trigger reads as a sentence with its references named, and the
    /// conditions it leaves out are listed apart.
    #[test]
    fn a_trigger_reads_as_a_sentence() {
        let names = |table: &str, id: u32| match (table, id) {
            ("Spell", 116) => Some("Frostbolt".to_string()),
            _ => None,
        };
        let timer = Event::new(6801, 68);
        assert_eq!(timer.sentence(&names), "In combat, after 5 s to 10 s, then every 5 s to 10 s");
        let once = Event { flags: 0, ..timer.clone() };
        assert_eq!(once.sentence(&names), "In combat, after 5 s to 10 s");
        assert_eq!(once.terms(), vec!["once".to_string()]);
        let flee = Event { event_type: 2, params: [15, 0, 0, 0], flags: 0x04, chance: 50, ..timer.clone() };
        assert_eq!(flee.sentence(&names), "Health at or below 15%");
        assert_eq!(flee.terms(), vec!["50%", "once", "not while casting"]);
        let hit = Event { event_type: 8, params: [116, 0, 0, 0], ..timer.clone() };
        assert_eq!(hit.sentence(&names), "Hit by Frostbolt");
        let any = Event { event_type: 8, params: [0, 0, 0, 0], ..timer.clone() };
        assert_eq!(any.sentence(&crate::schema::no_names), "Hit by any spell");
        let unnamed = Event { event_type: 23, params: [999, 3, 0, 0], ..timer.clone() };
        assert_eq!(unnamed.sentence(&crate::schema::no_names), "Has spell 999 (at least 3 stacks)");
        let phased = Event { phase_mask: 0b110, ..timer };
        assert_eq!(phased.terms(), vec!["not in phase 1, 2"]);
        assert_eq!(Event::of_type(6801, 68, 2).sentence(&names), "Health at or below 50%");
        assert_eq!(Event::of_type(6801, 68, 4).sentence(&names), "Combat starts");
        assert_eq!(Event::of_type(6801, 68, 4).terms(), vec!["once"]);
        assert_eq!(Event::of_type(6801, 68, 0), Event::new(6801, 68));
    }

    /// A copied event keeps its trigger and scripts, takes the new id and
    /// creature, and its comment's lead names the new creature.
    #[test]
    fn a_copied_event_belongs_to_the_new_creature() {
        let mut flee = Event::new(316001, 3160);
        flee.scripts = [316001, 0, 0];
        flee.comment = "Huklah - Flee at 15% HP".to_string();
        let copy = flee.copied_for(6805, 68, "Stormwind City Guard");
        assert_eq!((copy.id, copy.creature_id, copy.scripts), (6805, 68, [316001, 0, 0]));
        assert_eq!(copy.comment, "Stormwind City Guard - Flee at 15% HP");
        assert_eq!(renamed("no lead here", "Guard"), "no lead here");
        let sql = search_query("Flee", 40);
        assert!(sql.contains("`comment` LIKE '%Flee%'") && sql.ends_with("LIMIT 40"), "{sql}");
        assert!(search_query("3160", 40).contains("`creature_id` = 3160"));
    }

    /// Ids follow the reference database's convention and skip the taken.
    #[test]
    fn the_next_id_follows_the_convention() {
        assert_eq!(next_id(68, &[]), 6801);
        assert_eq!(next_id(68, &[6801, 6802, 6803, 6804, 6805]), 6806);
        assert_eq!(next_id(68, &[6801, 6803]), 6802);
        assert!(events_query(68).contains("`creature_id` = 68"));
    }
}
