//! `creature_spells`: the eight spells a creature casts in combat, and when.
//!
//! ## One row is one list
//!
//! `ObjectMgr::LoadCreatureSpells` (`ObjectMgr.cpp:1973`) reads one row per
//! list: an `entry`, a `name` for the person, and eight slots of eleven
//! columns each, `spellId_n` to `scriptId_n`. `creature_template.spell_list_id`
//! names a list; 3,636 templates on the reference database do, and 2,645
//! lists exist. A slot with `spellId_n` 0 is empty.
//!
//! Each slot is a spell, its `probability` in percent, a `castTarget` from
//! [`crate::scripts::TARGETS`] with its two parameters, `castFlags` from
//! [`crate::scripts::CAST_FLAGS`], the delay before the first cast as a range
//! in seconds (`LoadCreatureSpells` multiplies by `IN_MILLISECONDS`,
//! `ObjectMgr.cpp:2042`), the delay between casts as another, and a
//! `creature_spells_scripts` id run when the cast lands.
//!
//! ## Live on a reload
//!
//! `.reload creature_spells` re-reads the table (`ServerCommands.cpp:1049`).
//! A creature already in combat keeps the list it started with.

use crate::row::{Assignment, Key, Life};

pub use crate::schema::{mask_words, value_word, Bit, Column, Group, Kind, Row, RowValue, Value};

pub const TABLE: &str = "creature_spells";

/// The static name for the table, for a caller resolving a name read out of
/// a file.
pub fn table_named(name: &str) -> Option<&'static str> {
    (name == TABLE).then_some(TABLE)
}

/// How many slots a list has.
pub const SLOTS: usize = 8;

/// The eleven columns of one slot, without the `_n` suffix.
pub const SLOT_FIELDS: [&str; 11] = [
    "spellId",
    "probability",
    "castTarget",
    "targetParam1",
    "targetParam2",
    "castFlags",
    "delayInitialMin",
    "delayInitialMax",
    "delayRepeatMin",
    "delayRepeatMax",
    "scriptId",
];

/// What each of a slot's eleven columns is drawn as.
pub const SLOT_KINDS: [Kind; 11] = [
    Kind::Ref("Spell"),
    Kind::Unsigned,
    Kind::Choice(&crate::scripts::TARGET_VALUES),
    Kind::Unsigned,
    Kind::Unsigned,
    Kind::Flags(&crate::scripts::CAST_FLAGS),
    Kind::Seconds,
    Kind::Seconds,
    Kind::Seconds,
    Kind::Seconds,
    Kind::Ref(crate::scripts::CREATURE_SPELLS),
];

/// What each of a slot's eleven columns means.
pub const SLOT_ABOUT: [&str; 11] = [
    "the spell, or 0 for an empty slot",
    "percent chance to cast when its timer is up",
    "the cast target type",
    "its meaning depends on the cast target type",
    "its meaning depends on the cast target type",
    "cast flags: how it is cast",
    "least seconds into combat before the first cast",
    "most seconds into combat before the first cast",
    "least seconds between casts",
    "most seconds between casts",
    "a creature_spells_scripts id run when the cast lands, or 0",
];

macro_rules! slot_columns {
    ($n:literal) => {
        [
            Column { name: concat!("spellId_", $n), kind: SLOT_KINDS[0], group: Group::Combat, about: SLOT_ABOUT[0] },
            Column { name: concat!("probability_", $n), kind: SLOT_KINDS[1], group: Group::Combat, about: SLOT_ABOUT[1] },
            Column { name: concat!("castTarget_", $n), kind: SLOT_KINDS[2], group: Group::Combat, about: SLOT_ABOUT[2] },
            Column { name: concat!("targetParam1_", $n), kind: SLOT_KINDS[3], group: Group::Combat, about: SLOT_ABOUT[3] },
            Column { name: concat!("targetParam2_", $n), kind: SLOT_KINDS[4], group: Group::Combat, about: SLOT_ABOUT[4] },
            Column { name: concat!("castFlags_", $n), kind: SLOT_KINDS[5], group: Group::Combat, about: SLOT_ABOUT[5] },
            Column { name: concat!("delayInitialMin_", $n), kind: SLOT_KINDS[6], group: Group::Combat, about: SLOT_ABOUT[6] },
            Column { name: concat!("delayInitialMax_", $n), kind: SLOT_KINDS[7], group: Group::Combat, about: SLOT_ABOUT[7] },
            Column { name: concat!("delayRepeatMin_", $n), kind: SLOT_KINDS[8], group: Group::Combat, about: SLOT_ABOUT[8] },
            Column { name: concat!("delayRepeatMax_", $n), kind: SLOT_KINDS[9], group: Group::Combat, about: SLOT_ABOUT[9] },
            Column { name: concat!("scriptId_", $n), kind: SLOT_KINDS[10], group: Group::Combat, about: SLOT_ABOUT[10] },
        ]
    };
}

const SLOT_COLUMNS: [[Column; 11]; SLOTS] = [
    slot_columns!("1"),
    slot_columns!("2"),
    slot_columns!("3"),
    slot_columns!("4"),
    slot_columns!("5"),
    slot_columns!("6"),
    slot_columns!("7"),
    slot_columns!("8"),
];

/// The ninety columns, in the loader's `SELECT` order.
pub const COLUMNS: [Column; 2 + 11 * SLOTS] = {
    let mut out = [Column { name: "", kind: Kind::Unsigned, group: Group::Combat, about: "" }; 2 + 11 * SLOTS];
    out[0] = Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the list id; creature_template.spell_list_id references it" };
    out[1] = Column { name: "name", kind: Kind::Text, group: Group::Identity, about: "a label for editors; not read by the server" };
    let mut slot = 0;
    while slot < SLOTS {
        let mut field = 0;
        while field < 11 {
            out[2 + slot * 11 + field] = SLOT_COLUMNS[slot][field];
            field += 1;
        }
        slot += 1;
    }
    out
};

/// One column, by name.
pub fn column(name: &str) -> Option<&'static Column> {
    COLUMNS.iter().find(|column| column.name == name)
}

/// The columns of one slot, 0-based.
pub fn slot_columns(slot: usize) -> &'static [Column] {
    &SLOT_COLUMNS[slot.min(SLOTS - 1)]
}

/// The key of one row: the entry.
pub fn key(entry: u32) -> Key {
    Key::one("entry", u64::from(entry))
}

/// One slot as the server reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Slot {
    pub spell: u32,
    pub probability: u32,
    pub cast_target: u32,
    pub target_params: [u32; 2],
    pub cast_flags: u32,
    pub initial: (u32, u32),
    pub repeat: (u32, u32),
    pub script: u32,
}

impl Slot {
    /// The slot's eleven values, in [`SLOT_FIELDS`]' order.
    pub fn values(&self) -> [u32; 11] {
        [
            self.spell,
            self.probability,
            self.cast_target,
            self.target_params[0],
            self.target_params[1],
            self.cast_flags,
            self.initial.0,
            self.initial.1,
            self.repeat.0,
            self.repeat.1,
            self.script,
        ]
    }

    /// Whether the slot casts anything.
    pub fn is_empty(&self) -> bool {
        self.spell == 0
    }

    /// The slot as a sentence: `Cast Shoot on the current victim, after 2 s
    /// to 5 s, then every 3 s to 6 s`. The chance is left to the caller,
    /// which draws it apart. `names` resolves the spell.
    pub fn sentence(&self, names: crate::schema::Names<'_>) -> String {
        if self.is_empty() {
            return "Empty".to_string();
        }
        let spell = names("Spell", self.spell).unwrap_or_else(|| format!("spell {}", self.spell));
        let target = format!(" on {}", crate::scripts::target_phrase(self.cast_target));
        let seconds = |value: u32| u64::from(value) * 1000;
        let first = match self.initial {
            (0, 0) => String::new(),
            (least, most) => format!(", after {}", crate::schema::span_range(seconds(least), seconds(most))),
        };
        let every = match self.repeat {
            (0, 0) => String::new(),
            (least, most) => format!(", then every {}", crate::schema::span_range(seconds(least), seconds(most))),
        };
        format!("Cast {spell}{target}{first}{every}")
    }
}

/// One list as the server reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct List {
    pub entry: u32,
    pub name: String,
    pub slots: [Slot; SLOTS],
}

impl List {
    /// A list with nothing in it.
    pub fn new(entry: u32, name: &str) -> List {
        List { entry, name: name.to_string(), slots: [Slot::default(); SLOTS] }
    }

    pub fn key(&self) -> Key {
        key(self.entry)
    }

    /// One row as the database answered it, or `None` for a row with no entry.
    pub fn from_row(row: &Row) -> Option<List> {
        let count = |name: &str| row.integer(name).unwrap_or(0).max(0) as u32;
        let mut list = List::new(row.integer("entry")? as u32, row.text("name").unwrap_or_default());
        for (at, slot) in list.slots.iter_mut().enumerate() {
            let n = at + 1;
            let field = |name: &str| count(&format!("{name}_{n}"));
            *slot = Slot {
                spell: field("spellId"),
                probability: field("probability"),
                cast_target: field("castTarget"),
                target_params: [field("targetParam1"), field("targetParam2")],
                cast_flags: field("castFlags"),
                initial: (field("delayInitialMin"), field("delayInitialMax")),
                repeat: (field("delayRepeatMin"), field("delayRepeatMax")),
                script: field("scriptId"),
            };
        }
        Some(list)
    }

    /// Every editable column as a SQL literal, which is what a row this
    /// project creates carries.
    pub fn assignments(&self) -> Vec<Assignment> {
        let mut out = vec![Assignment { column: "name", value: crate::sql::text(&self.name) }];
        for (at, slot) in self.slots.iter().enumerate() {
            for (field, value) in slot_columns(at).iter().zip(slot.values()) {
                out.push(Assignment { column: field.name, value: value.to_string() });
            }
        }
        out
    }

    /// One column's value as a form shows it: `name`, or a slot's field by
    /// its full column name.
    pub fn get(&self, column: &str) -> String {
        if column == "entry" {
            return self.entry.to_string();
        }
        if column == "name" {
            return self.name.clone();
        }
        let Some((field, slot)) = split(column) else {
            return String::new();
        };
        self.slots[slot].values()[field].to_string()
    }

    /// Set one column from a form's text. `false` for a column the row does
    /// not have or a value that does not read.
    pub fn set(&mut self, column: &str, value: &str) -> bool {
        let value = value.trim();
        if column == "name" {
            self.name = match value.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')) {
                Some(inner) => inner.replace("\\'", "'").replace("\\\\", "\\"),
                None => value.to_string(),
            };
            return true;
        }
        let Some((field, slot)) = split(column) else {
            return false;
        };
        let Some(number) = value.parse::<i64>().ok().map(|n| n.max(0) as u32) else {
            return false;
        };
        let target = &mut self.slots[slot];
        match field {
            0 => target.spell = number,
            1 => target.probability = number,
            2 => target.cast_target = number,
            3 => target.target_params[0] = number,
            4 => target.target_params[1] = number,
            5 => target.cast_flags = number,
            6 => target.initial.0 = number,
            7 => target.initial.1 = number,
            8 => target.repeat.0 = number,
            9 => target.repeat.1 = number,
            _ => target.script = number,
        }
        true
    }

    /// How many slots cast something.
    pub fn filled(&self) -> usize {
        self.slots.iter().filter(|slot| !slot.is_empty()).count()
    }

    /// The same slots under another entry and name, for a list copied so it
    /// can be edited without changing the creatures that share the original.
    pub fn copied_as(&self, entry: u32, name: &str) -> List {
        List { entry, name: name.to_string(), slots: self.slots }
    }
}

/// A slot column's field index and 0-based slot, or `None` for any other
/// name: `castFlags_3` is `(5, 2)`.
pub fn split(column: &str) -> Option<(usize, usize)> {
    let (field, slot) = column.rsplit_once('_')?;
    let field = SLOT_FIELDS.iter().position(|name| *name == field)?;
    let slot: usize = slot.parse().ok()?;
    (1..=SLOTS).contains(&slot).then_some((field, slot - 1))
}

/// The statements a save emits for one row, on [`crate::eventai::statements`]'
/// terms.
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

/// The one row a list is.
pub fn row_query(entry: u32) -> String {
    format!("SELECT * FROM {} WHERE `entry` = {entry} LIMIT 1", crate::sql::name(TABLE))
}

/// Whether a row is already there, which an apply asks before it creates one.
pub fn exists_query(key: &Key) -> String {
    format!("SELECT 1 FROM {} WHERE {} LIMIT 1", crate::sql::name(TABLE), key.where_clause())
}

/// Every list whose name holds `term`, or whose entry is `term`: the search
/// behind choosing an existing list.
pub fn search_query(term: &str, limit: usize) -> String {
    let like = crate::sql::text(&format!("%{}%", term.trim()));
    let by_entry = match term.trim().parse::<u32>() {
        Ok(entry) => format!(" OR `entry` = {entry}"),
        Err(_) => String::new(),
    };
    format!(
        "SELECT * FROM {} WHERE `name` LIKE {like}{by_entry} ORDER BY `name` LIMIT {limit}",
        crate::sql::name(TABLE)
    )
}

/// The creature templates that name a list, one `entry` each. A template has
/// a row per patch, so the entries are listed rather than the rows; the
/// editor folds its project's own `spell_list_id` edits over them.
pub fn users_query(entry: u32) -> String {
    format!(
        "SELECT DISTINCT `entry` FROM {} WHERE `spell_list_id` = {entry}",
        crate::sql::name(crate::creature::TEMPLATE)
    )
}

/// The highest entry the table holds, for numbering a new list.
pub fn max_entry_query() -> String {
    format!("SELECT MAX(`entry`) AS `entry` FROM {}", crate::sql::name(TABLE))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ninety columns are the loader's, slot by slot, and each is named
    /// once.
    #[test]
    fn every_column_is_the_servers_own() {
        assert_eq!(COLUMNS.len(), 90);
        assert_eq!(COLUMNS[0].name, "entry");
        assert_eq!(COLUMNS[1].name, "name");
        assert_eq!(COLUMNS[2].name, "spellId_1");
        assert_eq!(COLUMNS[12].name, "scriptId_1");
        assert_eq!(COLUMNS[13].name, "spellId_2");
        assert_eq!(COLUMNS[89].name, "scriptId_8");
        let mut names: Vec<&str> = COLUMNS.iter().map(|column| column.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 90);
        assert_eq!(column("castFlags_3").map(|c| c.kind), Some(Kind::Flags(&crate::scripts::CAST_FLAGS)));
    }

    /// A row reads back slot by slot, and a created list names every column.
    #[test]
    fn a_list_is_read_and_written_whole() {
        let mut row = Row::new();
        row.insert("entry".into(), Some("680".into()));
        row.insert("name".into(), Some("Stormwind City Guard".into()));
        for (column, value) in [("spellId_1", "6660"), ("probability_1", "100"), ("castTarget_1", "1"), ("castFlags_1", "16"), ("delayRepeatMin_1", "2"), ("delayRepeatMax_1", "4"), ("spellId_2", "12169")] {
            row.insert(column.into(), Some(value.into()));
        }
        let list = List::from_row(&row).expect("a list");
        assert_eq!(list.slots[0].spell, 6660);
        assert_eq!(list.slots[0].cast_flags, 16);
        assert_eq!(list.slots[0].repeat, (2, 4));
        assert_eq!(list.slots[1].spell, 12169);
        assert_eq!(list.filled(), 2);
        let changes = list.assignments();
        assert_eq!(changes.len(), 89);
        let created = statements(&list.key(), Life::Insert, &changes);
        assert_eq!(created[0], "DELETE FROM `creature_spells` WHERE `entry` = 680;");
        assert!(created[1].contains("`spellId_1`") && created[1].contains("6660"), "{}", created[1]);
        assert!(created[1].contains("'Stormwind City Guard'"), "{}", created[1]);
    }

    /// A slot reads as a sentence with its delays in seconds; a copy keeps
    /// the slots under its own entry.
    #[test]
    fn a_slot_reads_as_a_sentence() {
        let names = |table: &str, id: u32| (table == "Spell" && id == 6660).then(|| "Shoot".to_string());
        let slot = Slot { spell: 6660, probability: 100, cast_target: 1, initial: (2, 5), repeat: (3, 6), ..Slot::default() };
        assert_eq!(slot.sentence(&names), "Cast Shoot on the current victim, after 2 s to 5 s, then every 3 s to 6 s");
        let bare = Slot { spell: 99, cast_target: 0, ..Slot::default() };
        assert_eq!(bare.sentence(&names), "Cast spell 99 on the provided target");
        assert_eq!(Slot::default().sentence(&names), "Empty");
        let mut list = List::new(680, "Guard");
        list.slots[0] = slot;
        let copy = list.copied_as(9000, "Guard copy");
        assert_eq!((copy.entry, copy.name.as_str(), copy.slots[0]), (9000, "Guard copy", slot));
        assert!(search_query("Guard", 40).contains("`name` LIKE '%Guard%'"));
        assert!(users_query(680).contains("`spell_list_id` = 680"));
    }
}
