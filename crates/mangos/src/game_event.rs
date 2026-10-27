//! The game event tables: what the world holds while a holiday, a fair or a
//! scheduled event is on.
//!
//! ## Five tables
//!
//! ```text
//! game_event                one event: when it first starts, how often it
//!                           comes back and for how long, its description, and
//!                           whether a script starts it instead of the clock
//! game_event_creature       a creature spawn that exists only while an event
//!                           is on, or, under the negated event, only while it
//!                           is off
//! game_event_gameobject     the same for a game object spawn
//! game_event_creature_data  what a creature spawn changes into while an event
//!                           is on: another entry, display or equipment set,
//!                           and a spell cast at each end of the event
//! game_event_quest          a quest offered only while an event is on
//! ```
//!
//! On the reference database there are 94 events, 3,064 creature members,
//! 12,137 game object members, 576 creature changes and 69 quests.
//! `game_event_mail` is empty there and is not edited here.
//!
//! ## What the server reads
//!
//! `GameEventMgr::LoadFromDB` (`GameEventMgr.cpp:165`) runs at startup only.
//! Event 0 is reserved: a `game_event` row at entry 0 is skipped, and so is
//! a member, change or quest naming event 0. An event with `length` 0 is
//! skipped. `occurence` and `length` are minutes. A `patch_min` above
//! `patch_max`, or a `patch_max` above 10, is reset to 0 and 10; an event
//! whose range leaves the server's `WowPatch` out is disabled. A member or a
//! change naming an event `game_event` does not hold, or a spawn that does
//! not exist, is skipped. A change row is read at the highest `patch` at or
//! below `WowPatch`, per spawn and event (line 444). A quest row is read when
//! its `patch_min` is at or below `WowPatch` (line 534).
//!
//! ## A row is a keyed row of the project's store
//!
//! On `crate::addon`'s terms. An event is keyed by `entry`; a spawn member by
//! `(guid, event)`, which is the whole row, so a member has no column to
//! edit and changing its event or its sign is a removal and a creation; a
//! change by `(guid, event, patch)`; a quest by `(quest, event)`.

use crate::row::{Assignment, Key, Table};

pub use crate::schema::{Column, Group, Kind, Row, RowValue, Value};

pub const EVENT: &str = "game_event";
pub const CREATURE: &str = "game_event_creature";
pub const GAMEOBJECT: &str = "game_event_gameobject";
pub const CREATURE_DATA: &str = "game_event_creature_data";
pub const QUEST: &str = "game_event_quest";

/// The five tables, in the order a plan writes them: an event before the
/// rows that name it.
pub const TABLES: &[Table] = &[EVENT_TABLE, CREATURE_TABLE, GAMEOBJECT_TABLE, CREATURE_DATA_TABLE, QUEST_TABLE];

pub const EVENT_TABLE: Table = Table::new(EVENT, &EVENT_COLUMNS);
pub const CREATURE_TABLE: Table = Table::new(CREATURE, &CREATURE_COLUMNS);
pub const GAMEOBJECT_TABLE: Table = Table::new(GAMEOBJECT, &GAMEOBJECT_COLUMNS);
pub const CREATURE_DATA_TABLE: Table = Table::new(CREATURE_DATA, &CREATURE_DATA_COLUMNS);
pub const QUEST_TABLE: Table = Table::new(QUEST, &QUEST_COLUMNS);

/// A yes-or-no column: `hardcoded` and `disabled`.
pub const NO_YES: [Value; 2] = [Value { value: 0, name: "No" }, Value { value: 1, name: "Yes" }];

/// The highest `patch_max` the loader accepts.
pub const MAX_PATCH: u32 = 10;

/// A year in minutes: an event that comes back every year.
pub const YEAR: u32 = 525_600;

/// What `start_time` and `end_time` hold for "no date": MySQL's zero
/// timestamp, which the shipped hard-coded events carry.
pub const NO_DATE: &str = "0000-00-00 00:00:00";

/// `game_event`'s columns, in the table's order.
pub const EVENT_COLUMNS: [Column; 11] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the event; 0 is reserved" },
    Column { name: "start_time", kind: Kind::Text, group: Group::Respawn, about: "the first time the event starts, as YYYY-MM-DD HH:MM:SS; it never starts before" },
    Column { name: "end_time", kind: Kind::Text, group: Group::Respawn, about: "the time after which the event never starts, as YYYY-MM-DD HH:MM:SS" },
    Column { name: "occurence", kind: Kind::Unsigned, group: Group::Respawn, about: "minutes from one start to the next; 525600 is a year" },
    Column { name: "length", kind: Kind::Unsigned, group: Group::Respawn, about: "minutes the event lasts once started; 0 makes the server skip the event" },
    Column { name: "holiday", kind: Kind::Unsigned, group: Group::Identity, about: "the client's holiday id, or 0" },
    Column { name: "description", kind: Kind::Text, group: Group::Identity, about: "what the event is, as the server's console names it" },
    Column { name: "hardcoded", kind: Kind::Choice(&NO_YES), group: Group::Advanced, about: "started and stopped by the server's own code rather than by the dates" },
    Column { name: "disabled", kind: Kind::Choice(&NO_YES), group: Group::Advanced, about: "never started" },
    Column { name: "patch_min", kind: Kind::Choice(&crate::creature::PATCHES), group: Group::Advanced, about: "lowest content patch the event is enabled in" },
    Column { name: "patch_max", kind: Kind::Choice(&crate::creature::PATCHES), group: Group::Advanced, about: "highest content patch the event is enabled in" },
];

/// A spawn member's columns: the key and nothing else.
pub const CREATURE_COLUMNS: [Column; 2] = [
    Column { name: "guid", kind: Kind::Key, group: Group::Identity, about: "the creature spawn" },
    Column { name: "event", kind: Kind::Key, group: Group::Identity, about: "the event the spawn exists during, or, negated, the event it is removed during" },
];

pub const GAMEOBJECT_COLUMNS: [Column; 2] = [
    Column { name: "guid", kind: Kind::Key, group: Group::Identity, about: "the game object spawn" },
    Column { name: "event", kind: Kind::Key, group: Group::Identity, about: "the event the spawn exists during, or, negated, the event it is removed during" },
];

/// `game_event_creature_data`'s columns, in the table's order.
pub const CREATURE_DATA_COLUMNS: [Column; 8] = [
    Column { name: "guid", kind: Kind::Key, group: Group::Identity, about: "the creature spawn that changes" },
    Column { name: "patch", kind: Kind::Key, group: Group::Identity, about: "the content patch the row belongs to; the server reads the highest at or below its WowPatch" },
    Column { name: "entry_id", kind: Kind::Ref(crate::creature::TEMPLATE), group: Group::Identity, about: "the creature_template entry the spawn becomes, or 0 to keep its own" },
    Column { name: "display_id", kind: Kind::Ref("CreatureDisplayInfo"), group: Group::Appearance, about: "the display the spawn is drawn as, or 0 to keep its own" },
    Column { name: "equipment_id", kind: Kind::Ref("creature_equip_template"), group: Group::Appearance, about: "the equipment set the spawn holds, or 0 to keep its own" },
    Column { name: "spell_start", kind: Kind::Ref("Spell"), group: Group::Effects, about: "the spell the spawn casts on itself when the event starts, or 0" },
    Column { name: "spell_end", kind: Kind::Ref("Spell"), group: Group::Effects, about: "the spell the spawn casts on itself when the event ends, or 0" },
    Column { name: "event", kind: Kind::Key, group: Group::Identity, about: "the event during which the spawn changes" },
];

/// `game_event_quest`'s columns.
pub const QUEST_COLUMNS: [Column; 3] = [
    Column { name: "quest", kind: Kind::Key, group: Group::Identity, about: "the quest offered only during the event" },
    Column { name: "event", kind: Kind::Key, group: Group::Identity, about: "the event" },
    Column { name: "patch_min", kind: Kind::Choice(&crate::creature::PATCHES), group: Group::Advanced, about: "lowest content patch the row is read in" },
];

/// The key of an event.
pub fn event_key(entry: u32) -> Key {
    Key::one("entry", u64::from(entry))
}

/// The key of a spawn member: `event` is negative for a spawn removed
/// during the event.
pub fn member_key(guid: u64, event: i32) -> Key {
    Key(vec![("guid".to_string(), guid.to_string()), ("event".to_string(), event.to_string())])
}

/// The key of a creature change.
pub fn change_key(guid: u64, event: u32, patch: u32) -> Key {
    Key(vec![
        ("guid".to_string(), guid.to_string()),
        ("event".to_string(), event.to_string()),
        ("patch".to_string(), patch.to_string()),
    ])
}

/// The key of an event quest.
pub fn quest_key(quest: u32, event: u32) -> Key {
    Key::two(("quest", u64::from(quest)), ("event", u64::from(event)))
}

/// A key column read as a signed number.
pub fn key_number(key: &Key, column: &str) -> Option<i64> {
    key.0.iter().find(|(name, _)| name == column)?.1.parse().ok()
}

/// One `game_event` row.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub entry: u32,
    pub start_time: String,
    pub end_time: String,
    pub occurence: u32,
    pub length: u32,
    pub holiday: u32,
    pub description: String,
    pub hardcoded: u32,
    pub disabled: u32,
    pub patch_min: u32,
    pub patch_max: u32,
}

impl Event {
    /// An event that a script starts and stops: no dates, a day long, once a
    /// year, enabled at every patch. A person sets the dates to schedule it.
    pub fn new(entry: u32, description: &str) -> Event {
        Event {
            entry,
            start_time: NO_DATE.to_string(),
            end_time: NO_DATE.to_string(),
            occurence: YEAR,
            length: 1440,
            holiday: 0,
            description: description.to_string(),
            hardcoded: 1,
            disabled: 0,
            patch_min: 0,
            patch_max: MAX_PATCH,
        }
    }

    pub fn key(&self) -> Key {
        event_key(self.entry)
    }

    /// Every column that is not the key, as SQL literals.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "start_time", value: crate::sql::text(&self.start_time) },
            Assignment { column: "end_time", value: crate::sql::text(&self.end_time) },
            Assignment { column: "occurence", value: self.occurence.to_string() },
            Assignment { column: "length", value: self.length.to_string() },
            Assignment { column: "holiday", value: self.holiday.to_string() },
            Assignment { column: "description", value: crate::sql::text(&self.description) },
            Assignment { column: "hardcoded", value: self.hardcoded.to_string() },
            Assignment { column: "disabled", value: self.disabled.to_string() },
            Assignment { column: "patch_min", value: self.patch_min.to_string() },
            Assignment { column: "patch_max", value: self.patch_max.to_string() },
        ]
    }

    pub fn from_row(row: &Row) -> Option<Event> {
        let unsigned = |column: &str| row.integer(column).unwrap_or(0).max(0) as u32;
        Some(Event {
            entry: row.integer("entry")?.max(0) as u32,
            start_time: row.text("start_time").unwrap_or(NO_DATE).to_string(),
            end_time: row.text("end_time").unwrap_or(NO_DATE).to_string(),
            occurence: unsigned("occurence"),
            length: unsigned("length"),
            holiday: unsigned("holiday"),
            description: row.text("description").unwrap_or("").to_string(),
            hardcoded: unsigned("hardcoded"),
            disabled: unsigned("disabled"),
            patch_min: unsigned("patch_min"),
            patch_max: unsigned("patch_max"),
        })
    }

    /// Whether the server enables the event at `wow_patch`.
    pub fn enabled_at(&self, wow_patch: u32) -> bool {
        self.disabled == 0 && (self.patch_min..=self.patch_max).contains(&wow_patch)
    }

    /// What the server would do with the row that the row alone shows.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.entry == 0 {
            out.push("entry 0 is reserved; the server skips the event".to_string());
        }
        if self.length == 0 {
            out.push("length is 0; the server skips the event".to_string());
        }
        if self.patch_min > self.patch_max || self.patch_max > MAX_PATCH {
            out.push(format!(
                "patch_min {} and patch_max {} are not a range of patches; the server resets them to 0 and 10",
                self.patch_min, self.patch_max
            ));
        }
        for (column, value) in [("start_time", &self.start_time), ("end_time", &self.end_time)] {
            if !is_timestamp(value) {
                out.push(format!("{column} '{value}' is not YYYY-MM-DD HH:MM:SS"));
            }
        }
        out
    }
}

/// Whether a value has a timestamp's shape, `YYYY-MM-DD HH:MM:SS`.
pub fn is_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 19
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            4 | 7 => *byte == b'-',
            10 => *byte == b' ',
            13 | 16 => *byte == b':',
            _ => byte.is_ascii_digit(),
        })
}

/// A duration in minutes, in the largest whole units: "30 days", "2 h 30 min".
pub fn minutes_words(minutes: u32) -> String {
    let (days, rest) = (minutes / 1440, minutes % 1440);
    let (hours, rest) = (rest / 60, rest % 60);
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days} d"));
    }
    if hours > 0 {
        parts.push(format!("{hours} h"));
    }
    if rest > 0 || parts.is_empty() {
        parts.push(format!("{rest} min"));
    }
    parts.join(" ")
}

/// One `game_event_creature_data` row.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub guid: u64,
    pub event: u32,
    pub patch: u32,
    pub entry_id: u32,
    pub display_id: u32,
    pub equipment_id: u32,
    pub spell_start: u32,
    pub spell_end: u32,
}

impl Change {
    /// A change that changes nothing yet, at patch 0.
    pub fn new(guid: u64, event: u32) -> Change {
        Change { guid, event, patch: 0, entry_id: 0, display_id: 0, equipment_id: 0, spell_start: 0, spell_end: 0 }
    }

    pub fn key(&self) -> Key {
        change_key(self.guid, self.event, self.patch)
    }

    /// Every column that is not the key, as SQL literals.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "entry_id", value: self.entry_id.to_string() },
            Assignment { column: "display_id", value: self.display_id.to_string() },
            Assignment { column: "equipment_id", value: self.equipment_id.to_string() },
            Assignment { column: "spell_start", value: self.spell_start.to_string() },
            Assignment { column: "spell_end", value: self.spell_end.to_string() },
        ]
    }

    pub fn from_row(row: &Row) -> Option<Change> {
        let unsigned = |column: &str| row.integer(column).unwrap_or(0).max(0) as u32;
        Some(Change {
            guid: row.integer("guid")?.max(0) as u64,
            event: unsigned("event"),
            patch: unsigned("patch"),
            entry_id: unsigned("entry_id"),
            display_id: unsigned("display_id"),
            equipment_id: unsigned("equipment_id"),
            spell_start: unsigned("spell_start"),
            spell_end: unsigned("spell_end"),
        })
    }
}

/// One `game_event_creature` or `game_event_gameobject` row: a spawn, and
/// the event it exists during, negated for the event it is removed during.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Member {
    pub guid: u64,
    pub event: i32,
}

impl Member {
    pub fn key(&self) -> Key {
        member_key(self.guid, self.event)
    }

    /// None: the key is the whole row.
    pub fn assignments(&self) -> Vec<Assignment> {
        Vec::new()
    }

    pub fn from_row(row: &Row) -> Option<Member> {
        Some(Member { guid: row.integer("guid")?.max(0) as u64, event: row.integer("event")? as i32 })
    }

    /// The event, whichever way the spawn answers it.
    pub fn entry(&self) -> u32 {
        self.event.unsigned_abs()
    }

    /// Whether the spawn is removed during the event rather than added.
    pub fn removed(&self) -> bool {
        self.event < 0
    }
}

/// One `game_event_quest` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventQuest {
    pub quest: u32,
    pub event: u32,
    pub patch_min: u32,
}

impl EventQuest {
    pub fn key(&self) -> Key {
        quest_key(self.quest, self.event)
    }

    pub fn assignments(&self) -> Vec<Assignment> {
        vec![Assignment { column: "patch_min", value: self.patch_min.to_string() }]
    }

    pub fn from_row(row: &Row) -> Option<EventQuest> {
        Some(EventQuest {
            quest: row.integer("quest")?.max(0) as u32,
            event: row.integer("event")?.max(0) as u32,
            patch_min: row.integer("patch_min").unwrap_or(0).max(0) as u32,
        })
    }
}

/// The fault every member, change and quest row shares: event 0 is
/// reserved and skipped.
pub fn event_fault(event: i64) -> Option<String> {
    (event == 0).then(|| "event 0 is reserved; the server skips the row".to_string())
}

// ---------------------------------------------------------------------------
// The queries
// ---------------------------------------------------------------------------

/// Every event. 94 rows on the reference database. The two times are read
/// as text in the table's own format.
pub fn events_query() -> String {
    format!(
        "SELECT `entry`, DATE_FORMAT(`start_time`, '%Y-%m-%d %H:%i:%s') AS `start_time`, \
         DATE_FORMAT(`end_time`, '%Y-%m-%d %H:%i:%s') AS `end_time`, `occurence`, `length`, \
         `holiday`, `description`, `hardcoded`, `disabled`, `patch_min`, `patch_max` \
         FROM {} ORDER BY `entry`",
        crate::sql::name(EVENT)
    )
}

/// Every row of one of the four member tables, read whole: the largest,
/// `game_event_gameobject`, is 12,137 rows on the reference database.
pub fn rows_query(table: &str) -> String {
    format!("SELECT * FROM {}", crate::sql::name(table))
}

/// The largest event entry, which a new event is numbered above.
pub fn max_entry_query() -> String {
    format!("SELECT MAX(`entry`) AS `n` FROM {}", crate::sql::name(EVENT))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(values: &[(&str, &str)]) -> Row {
        values
            .iter()
            .map(|(column, value)| (column.to_string(), Some(value.to_string())))
            .collect()
    }

    /// An event reads back from its columns, and the loader's checks say
    /// what it would skip or reset.
    #[test]
    fn an_event_the_server_skips_or_resets_says_why() {
        let read = Event::from_row(&row(&[
            ("entry", "1"),
            ("start_time", "2020-06-21 22:00:00"),
            ("end_time", "2038-01-01 00:59:59"),
            ("occurence", "525600"),
            ("length", "20160"),
            ("holiday", "341"),
            ("description", "Midsummer Fire Festival"),
            ("hardcoded", "0"),
            ("disabled", "0"),
            ("patch_min", "9"),
            ("patch_max", "10"),
        ]))
        .unwrap();
        assert!(read.check().is_empty(), "{:?}", read.check());
        assert!(read.enabled_at(10) && !read.enabled_at(8));
        assert!(Event { length: 0, ..read.clone() }.check()[0].contains("length is 0"));
        assert!(Event { patch_max: 11, ..read.clone() }.check()[0].contains("resets"));
        assert!(Event { start_time: "June".into(), ..read.clone() }.check()[0].contains("start_time"));
        assert!(Event::new(0, "x").check()[0].contains("reserved"));
        assert!(is_timestamp(NO_DATE));
        assert_eq!(minutes_words(20160), "14 d");
        assert_eq!(minutes_words(150), "2 h 30 min");
        assert_eq!(minutes_words(0), "0 min");
    }

    /// A spawn member's key carries its sign, and a change and a quest their
    /// three and two columns.
    #[test]
    fn the_keys_carry_the_event_and_its_sign() {
        let removed = member_key(79819, -5);
        assert_eq!(removed.text(), "guid=79819;event=-5");
        assert_eq!(key_number(&removed, "event"), Some(-5));
        assert_eq!(change_key(1, 2, 3).text(), "guid=1;event=2;patch=3");
        assert_eq!(quest_key(783, 1).text(), "quest=783;event=1");
        assert!(event_fault(0).is_some());
        assert_eq!(event_fault(-3), None);
        assert!(events_query().contains("DATE_FORMAT(`start_time`"));
        assert_eq!(rows_query(CREATURE), "SELECT * FROM `game_event_creature`");
        assert!(Change::new(5, 1).assignments().iter().all(|a| a.value == "0"));
    }
}
