//! The nine `*_loot_template` tables: what a creature drops, what a chest
//! holds, what an item turns into, and the shared sets any of them may name.
//!
//! ## One schema, nine tables
//!
//! `LootStore::LoadLootTable` (`LootMgr.cpp:93`) reads every one of them with
//! the same `SELECT`, so there is one column list here and nine names. What
//! differs is what an `entry` is keyed to — see [`Table`] and
//! [`Table::keyed_by`]:
//!
//! ```text
//! creature_loot_template       creature_template.loot_id            what it drops
//! pickpocketing_loot_template  creature_template.pickpocket_loot_id what is stolen from it
//! skinning_loot_template       creature_template.skinning_loot_id   what it is skinned for
//! gameobject_loot_template     gameobject_template's lootId data column: a chest,
//!                              a vein, a herb, a fishing pool
//! item_loot_template           item_template.entry, for an item with the
//!                              LOOTABLE flag — a clam, a sack, a lockbox
//! disenchant_loot_template     item_template.disenchant_id
//! fishing_loot_template        AreaTable.dbc's zone id
//! mail_loot_template           MailTemplate.dbc's id
//! reference_loot_template      a negative mincountOrRef in any of the other eight
//! ```
//!
//! ## A row is five columns
//!
//! [`key`] is `(entry, item, groupid, patch_min, patch_max)`, which is
//! `creature_loot_template`'s own primary key on the reference install and a
//! superset of the others' — `(entry, item)` on five of them, `(entry, item,
//! patch_min, patch_max)` on three. Naming all five identifies exactly one row
//! on every table, which `(entry, item)` does not: `creature_loot_template`
//! holds 5,131 `(entry, item)` pairs at more than one patch band, and 27 of the
//! pairs the server loads at patch 10 are the same item in two groups.
//!
//! **So a group is not edited; a row is moved between groups by removing it
//! and creating it again.** That is one gesture on the editor's side and two
//! claims in the store, and it is what changing a group is to the server too:
//! `LootTemplate::AddEntry` files a row under its group once, at load.
//!
//! ## Three columns whose sign is part of the value
//!
//! ```text
//! ChanceOrQuestChance  > 0 the drop chance in percent; < 0 the same chance for
//!                      a quest drop, offered only to a player on a quest that
//!                      asks for the item. 0 is allowed in a group, where the
//!                      group's members share the remainder equally
//! mincountOrRef        > 0 the least of the item that drops; < 0 the negative
//!                      of a reference_loot_template entry, and `item` is then a
//!                      label the server never reads. 0 is refused
//! groupid              0 rolls on its own; 1..127 one of the group drops, one
//!                      row of it per loot. Stored in seven bits
//! ```
//!
//! `LootStoreItem`'s constructor (`LootMgr.h:119`) is where the first two are
//! split, and [`Entry::check`] is `LootStoreItem::IsValid` (`LootMgr.cpp:279`)
//! as this crate reads it, so a row the server would skip at load is said to
//! be one before it is written.
//!
//! ## Live on a reload, removals included
//!
//! `LoadLootTable` begins with `Clear()`, so `.reload <table>` re-reads the
//! whole table and a row that has gone from it is gone from the server. Each
//! table has its own reload command under its own name (`Chat.cpp:820` and
//! after). What a reload does not reach is loot already rolled: a corpse
//! standing in the world keeps the list it was given when it died.
//!
//! The loader also drops any row whose `item` is in `forbidden_items` for the
//! server's patch, and any row outside `patch_min..=patch_max`. A row made here
//! is written `0..10`, every patch, which is the DDL's default.

use crate::row::{Assignment, Key, Life};

pub use crate::schema::{value_word, Column, Group, Kind, Row, RowValue};

pub const CREATURE: &str = "creature_loot_template";
pub const PICKPOCKETING: &str = "pickpocketing_loot_template";
pub const SKINNING: &str = "skinning_loot_template";
pub const GAMEOBJECT: &str = "gameobject_loot_template";
pub const ITEM: &str = "item_loot_template";
pub const DISENCHANT: &str = "disenchant_loot_template";
pub const FISHING: &str = "fishing_loot_template";
pub const MAIL: &str = "mail_loot_template";
pub const REFERENCE: &str = "reference_loot_template";

/// Every table this module writes, for a caller that has to resolve a name
/// read out of a file back to one of these constants.
pub const TABLES: [&str; 9] = [
    CREATURE,
    PICKPOCKETING,
    SKINNING,
    GAMEOBJECT,
    ITEM,
    DISENCHANT,
    FISHING,
    MAIL,
    REFERENCE,
];

/// The static name for a table read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// **What one of the nine tables is**: its name, the word a window heads it
/// with, and what an `entry` of it is keyed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Table {
    pub name: &'static str,
    /// The heading: `Drops`, `Pickpocket`, `Skinning`, `Holds`…
    pub word: &'static str,
    /// The column, in the server's own words, whose value names an entry.
    pub keyed_by: &'static str,
}

/// The nine, in [`TABLES`]' order.
pub const ALL: [Table; 9] = [
    Table { name: CREATURE, word: "Creature loot", keyed_by: "creature_template.loot_id" },
    Table { name: PICKPOCKETING, word: "Pickpocketing loot", keyed_by: "creature_template.pickpocket_loot_id" },
    Table { name: SKINNING, word: "Skinning loot", keyed_by: "creature_template.skinning_loot_id" },
    Table { name: GAMEOBJECT, word: "Game object loot", keyed_by: "the lootId data column of gameobject_template" },
    Table { name: ITEM, word: "Item loot", keyed_by: "item_template.entry of an item with the LOOTABLE flag" },
    Table { name: DISENCHANT, word: "Disenchant loot", keyed_by: "item_template.disenchant_id" },
    Table { name: FISHING, word: "Fishing loot", keyed_by: "the AreaTable.dbc zone id" },
    Table { name: MAIL, word: "Mail loot", keyed_by: "the MailTemplate.dbc id" },
    Table { name: REFERENCE, word: "Reference loot", keyed_by: "a negative mincountOrRef in any other loot table" },
];

/// One of the nine, by name.
pub fn table(name: &str) -> Option<Table> {
    ALL.into_iter().find(|table| table.name == name)
}

/// **The columns every loot table has**, in `LoadLootTable`'s `SELECT` order
/// with the two patch columns it filters on after them.
///
/// Five are the key — see the module comment for why `groupid` and the patch
/// band are among them.
pub const COLUMNS: [Column; 9] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the loot set the row belongs to: the creature's loot_id, the chest's lootId, or the item's entry" },
    Column { name: "item", kind: Kind::Key, group: Group::Identity, about: "the item_template entry, or a label when the row is a reference" },
    Column { name: "ChanceOrQuestChance", kind: Kind::Float, group: Group::Loot, about: "drop chance in percent; negative is the same chance for a quest drop, offered only to a player whose quest asks for it; 0 in a group shares the remainder" },
    Column { name: "groupid", kind: Kind::Key, group: Group::Loot, about: "group: 0 rolls on its own; in a group 1..127, one row of the group drops per loot" },
    Column { name: "mincountOrRef", kind: Kind::Signed, group: Group::Loot, about: "the least count that drops; a negative value is a reference_loot_template entry, negated" },
    Column { name: "maxcount", kind: Kind::Unsigned, group: Group::Loot, about: "the most that drops, at least mincountOrRef; up to 255" },
    Column { name: "condition_id", kind: Kind::Unsigned, group: Group::Loot, about: "a row of `conditions` the player must meet, or 0" },
    Column { name: "patch_min", kind: Kind::Key, group: Group::Identity, about: "the first content patch the row is loaded at" },
    Column { name: "patch_max", kind: Kind::Key, group: Group::Identity, about: "the last content patch the row is loaded at" },
];

/// The columns of a table, or an empty slice for a name this module does not
/// know.
pub fn columns_of(table: &str) -> &'static [Column] {
    match table_named(table) {
        Some(_) => &COLUMNS,
        None => &[],
    }
}

/// One column, by name, of any loot table.
pub fn column(table: &str, name: &str) -> Option<&'static Column> {
    columns_of(table).iter().find(|column| column.name == name)
}

/// The most a group may be numbered: seven bits (`LootMgr.cpp:281`).
pub const MAX_GROUP: u32 = 127;

/// The most `maxcount` may hold: it is read into a `uint8` (`LootMgr.cpp:124`).
pub const MAX_COUNT: u32 = 255;

/// The patch band a row made here is written with: every patch.
pub const EVERY_PATCH: (u32, u32) = (0, 10);

/// **The key of one row**: the five columns that name exactly one row on every
/// loot table — see the module comment.
pub fn key(entry: u32, item: u32, group: u32, patch_min: u32, patch_max: u32) -> Key {
    Key(vec![
        ("entry".to_string(), entry.to_string()),
        ("item".to_string(), item.to_string()),
        ("groupid".to_string(), group.to_string()),
        ("patch_min".to_string(), patch_min.to_string()),
        ("patch_max".to_string(), patch_max.to_string()),
    ])
}

/// **One row of a loot table**, as the server reads it: the two signed columns
/// split into what they mean.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub entry: u32,
    /// The item, or the label a reference row carries.
    pub item: u32,
    /// The chance in percent, never negative — the sign is [`Self::quest`].
    pub chance: f32,
    /// Whether the row is a quest drop.
    pub quest: bool,
    pub group: u32,
    /// `mincountOrRef` as stored: the least count, or a reference negated.
    pub min_or_ref: i32,
    pub max: u32,
    pub condition: u32,
    pub patch_min: u32,
    pub patch_max: u32,
}

impl Entry {
    /// A row as [`new_item`] writes it: one of the item, always, on its own.
    pub fn item(entry: u32, item: u32) -> Entry {
        Entry {
            entry,
            item,
            chance: 100.0,
            quest: false,
            group: 0,
            min_or_ref: 1,
            max: 1,
            condition: 0,
            patch_min: EVERY_PATCH.0,
            patch_max: EVERY_PATCH.1,
        }
    }

    /// …and one that names a reference set, at full chance.
    pub fn reference(entry: u32, reference: u32) -> Entry {
        Entry {
            item: reference,
            min_or_ref: -(reference as i32),
            ..Entry::item(entry, reference)
        }
    }

    /// Its key.
    pub fn key(&self) -> Key {
        key(self.entry, self.item, self.group, self.patch_min, self.patch_max)
    }

    /// The `reference_loot_template` entry this row names, if it is a
    /// reference rather than an item.
    pub fn reference_to(&self) -> Option<u32> {
        match self.min_or_ref < 0 {
            true => Some(self.min_or_ref.unsigned_abs()),
            false => None,
        }
    }

    /// The least that drops, for a row that is an item.
    pub fn min(&self) -> u32 {
        self.min_or_ref.max(0) as u32
    }

    /// `ChanceOrQuestChance` as the column holds it.
    pub fn chance_column(&self) -> f32 {
        match self.quest {
            true => -self.chance,
            false => self.chance,
        }
    }

    /// The count in words: `1`, `2–4`, or for a reference the set it names.
    pub fn count_words(&self) -> String {
        match self.reference_to() {
            Some(reference) => format!("reference {reference}"),
            None if self.min() == self.max => self.min().to_string(),
            None => format!("{}\u{2013}{}", self.min(), self.max),
        }
    }

    /// The chance in words: `100%`, `0.5%`, `quest 100%`, or `group share`
    /// for a grouped row at zero.
    pub fn chance_words(&self) -> String {
        let percent = match self.chance == self.chance.trunc() {
            true => format!("{}%", self.chance as i64),
            false => format!("{}%", self.chance),
        };
        match (self.chance == 0.0 && self.group != 0, self.quest) {
            (true, _) => "group share".to_string(),
            (false, true) => format!("quest {percent}"),
            (false, false) => percent,
        }
    }

    /// **Every column as a SQL literal**, which is what a row this project
    /// creates carries — see [`crate::row::Life::Insert`].
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "ChanceOrQuestChance", value: crate::sql::float(self.chance_column()) },
            Assignment { column: "mincountOrRef", value: self.min_or_ref.to_string() },
            Assignment { column: "maxcount", value: self.max.to_string() },
            Assignment { column: "condition_id", value: self.condition.to_string() },
        ]
    }

    /// One row as the database answered it, or `None` for a row missing a key
    /// column.
    pub fn from_row(row: &Row) -> Option<Entry> {
        let chance = row.number("ChanceOrQuestChance").unwrap_or(100.0) as f32;
        Some(Entry {
            entry: row.integer("entry")? as u32,
            item: row.integer("item")? as u32,
            chance: chance.abs(),
            quest: chance < 0.0,
            group: row.integer("groupid").unwrap_or(0) as u32,
            min_or_ref: row.integer("mincountOrRef").unwrap_or(1) as i32,
            max: row.integer("maxcount").unwrap_or(1) as u32,
            condition: row.integer("condition_id").unwrap_or(0) as u32,
            patch_min: row.integer("patch_min").unwrap_or(EVERY_PATCH.0 as i64) as u32,
            patch_max: row.integer("patch_max").unwrap_or(EVERY_PATCH.1 as i64) as u32,
        })
    }

    /// **Why the server would skip this row at load**, each as a sentence, or
    /// nothing. `LootStoreItem::IsValid` (`LootMgr.cpp:279`) and the two checks
    /// before it in `LoadLootTable`, in their order. Whether `item` is in
    /// `item_template` is the one check that needs the database and is not
    /// made here.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.max > MAX_COUNT {
            out.push(format!("maxcount {} is over {MAX_COUNT}", self.max));
        }
        if self.group > MAX_GROUP {
            out.push(format!("group {} is over {MAX_GROUP}", self.group));
        }
        if self.min_or_ref == 0 {
            out.push("mincountOrRef is 0: neither a count nor a reference".to_string());
        }
        match self.reference_to() {
            None if self.min_or_ref > 0 => {
                if self.chance == 0.0 && self.group == 0 {
                    out.push("chance 0 is allowed only in a group".to_string());
                }
                if self.chance != 0.0 && self.chance < 0.000_001 {
                    out.push(format!("chance {} is too low to roll", self.chance));
                }
                if self.max < self.min() {
                    out.push(format!("maxcount {} is under mincountOrRef {}", self.max, self.min()));
                }
            }
            Some(_) => {
                if self.condition != 0 {
                    out.push("a reference may not carry a condition".to_string());
                }
                if self.quest {
                    out.push("quest chance on a reference is read as an ordinary chance".to_string());
                } else if self.chance == 0.0 {
                    out.push("a reference with chance 0 never rolls".to_string());
                }
            }
            None => {}
        }
        out
    }
}

/// **Every column of a new row naming an item**: one of it, always, on its own.
pub fn new_item(entry: u32, item: u32) -> Vec<Assignment> {
    Entry::item(entry, item).assignments()
}

/// …and of one naming a reference set.
pub fn new_reference(entry: u32, reference: u32) -> Vec<Assignment> {
    Entry::reference(entry, reference).assignments()
}

/// **The statements a save emits for one row this project claims**: an
/// `UPDATE` for a row it edits, a `DELETE`/`INSERT` pair for one it creates —
/// so applying twice means the same as applying once — and a `DELETE` for one
/// it removes.
pub fn statements(table: &str, key: &Key, life: Life, changes: &[Assignment]) -> Vec<String> {
    match life {
        Life::Update => crate::row::update(table, key, changes).into_iter().collect(),
        Life::Insert => match crate::row::insert(table, key, changes) {
            Some(statement) => vec![crate::row::delete(table, key), statement],
            None => Vec::new(),
        },
        Life::Delete => vec![crate::row::delete(table, key)],
    }
}

// ---------------------------------------------------------------------------
// The queries
// ---------------------------------------------------------------------------

/// **Every row of one loot set the server would load**, which is what a window
/// reads: a few rows for most sets, a few hundred for a raid boss.
pub fn rows_query(table: &str, entry: u32, wow_patch: u32) -> String {
    format!(
        "SELECT * FROM {} WHERE `entry` = {entry} AND {wow_patch} BETWEEN `patch_min` AND `patch_max` \
         ORDER BY `groupid`, `item`",
        crate::sql::name(table)
    )
}

/// The row a key names — what an undo is taken from.
pub fn row_query(table: &str, key: &Key) -> String {
    format!(
        "SELECT * FROM {} WHERE {} LIMIT 1",
        crate::sql::name(table),
        key.where_clause()
    )
}

/// Whether a row is already there, which an apply asks before it creates one.
pub fn exists_query(table: &str, key: &Key) -> String {
    format!(
        "SELECT 1 FROM {} WHERE {} LIMIT 1",
        crate::sql::name(table),
        key.where_clause()
    )
}

/// **How many rows each entry of a table holds**, for a picker over the sets
/// that exist — 644 references, 4,074 creature sets.
pub fn entries_query(table: &str, wow_patch: u32) -> String {
    format!(
        "SELECT `entry`, COUNT(*) AS `n` FROM {} \
         WHERE {wow_patch} BETWEEN `patch_min` AND `patch_max` GROUP BY `entry` ORDER BY `entry`",
        crate::sql::name(table)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The schema is `LoadLootTable`'s `SELECT`, column for column**, with
    /// the two patch columns its `WHERE` filters on after them.
    #[test]
    fn every_column_is_the_servers_own_in_the_servers_own_order() {
        const SELECTED: &str = "entry, item, ChanceOrQuestChance, groupid, mincountOrRef, \
                                maxcount, condition_id, patch_min, patch_max";
        let ours: Vec<&str> = COLUMNS.iter().map(|column| column.name).collect();
        let theirs: Vec<&str> = SELECTED.split(", ").map(str::trim).collect();
        assert_eq!(ours, theirs);
    }

    /// The key names all five columns, so a statement meets one row on every
    /// table — see the module comment.
    #[test]
    fn the_key_names_five_columns() {
        let key = key(1502, 2770, 0, 0, 10);
        assert_eq!(
            key.where_clause(),
            "`entry` = 1502 AND `item` = 2770 AND `groupid` = 0 AND `patch_min` = 0 AND `patch_max` = 10"
        );
        assert_eq!(Key::parse(&key.text()), Some(key));
        for column in COLUMNS.iter().filter(|column| column.kind == Kind::Key) {
            assert!(
                Entry::item(1, 2).key().0.iter().any(|(name, _)| name == column.name),
                "{} is a key column and not in the key",
                column.name
            );
        }
    }

    /// **The two signed columns read as what they mean.**
    #[test]
    fn a_row_is_read_as_the_server_reads_it() {
        let mut row = Row::new();
        for (column, value) in [
            ("entry", "1502"),
            ("item", "30001"),
            ("ChanceOrQuestChance", "-45.5"),
            ("groupid", "1"),
            ("mincountOrRef", "-30001"),
            ("maxcount", "1"),
            ("condition_id", "0"),
            ("patch_min", "0"),
            ("patch_max", "10"),
        ] {
            row.insert(column.to_string(), Some(value.to_string()));
        }
        let entry = Entry::from_row(&row).expect("a whole row");
        assert!(entry.quest);
        assert_eq!(entry.chance, 45.5);
        assert_eq!(entry.chance_column(), -45.5);
        assert_eq!(entry.reference_to(), Some(30001));
        assert_eq!(entry.count_words(), "reference 30001");
        assert_eq!(entry.chance_words(), "quest 45.5%");

        let plain = Entry { min_or_ref: 2, max: 4, quest: false, chance: 100.0, ..entry.clone() };
        assert_eq!(plain.count_words(), "2\u{2013}4");
        assert_eq!(plain.chance_words(), "100%");
        let shared = Entry { chance: 0.0, group: 1, ..plain.clone() };
        assert_eq!(shared.chance_words(), "group share");
    }

    /// **`IsValid`, as this crate reads it**: each refusal by its own cause,
    /// and a row the server loads without a word answers nothing.
    #[test]
    fn the_servers_own_checks_are_made_before_a_row_is_written() {
        assert!(Entry::item(1, 2).check().is_empty());
        assert!(Entry::reference(1, 30001).check().is_empty());
        let zero = Entry { min_or_ref: 0, ..Entry::item(1, 2) };
        assert!(zero.check()[0].contains("mincountOrRef is 0"), "{:?}", zero.check());
        let backwards = Entry { min_or_ref: 3, max: 2, ..Entry::item(1, 2) };
        assert!(backwards.check()[0].contains("under"), "{:?}", backwards.check());
        let ungrouped = Entry { chance: 0.0, ..Entry::item(1, 2) };
        assert!(ungrouped.check()[0].contains("only in a group"));
        assert!(Entry { chance: 0.0, group: 1, ..Entry::item(1, 2) }.check().is_empty());
        let big = Entry { group: 128, ..Entry::item(1, 2) };
        assert!(big.check()[0].contains("over 127"));
        let conditioned = Entry { condition: 5, ..Entry::reference(1, 30001) };
        assert!(conditioned.check()[0].contains("condition"));
        let never = Entry { chance: 0.0, ..Entry::reference(1, 30001) };
        assert!(never.check()[0].contains("never rolls"));
    }

    /// A created row is a `DELETE` and an `INSERT` naming every column, an
    /// edit is one `UPDATE`, and a removal is one `DELETE` — each on the whole
    /// key.
    #[test]
    fn a_row_becomes_the_statements_it_means() {
        let key = Entry::item(1502, 2770).key();
        let created = statements(GAMEOBJECT, &key, Life::Insert, &new_item(1502, 2770));
        assert_eq!(created.len(), 2);
        assert_eq!(
            created[1],
            "INSERT INTO `gameobject_loot_template` (`entry`, `item`, `groupid`, `patch_min`, \
             `patch_max`, `ChanceOrQuestChance`, `mincountOrRef`, `maxcount`, `condition_id`) \
             VALUES (1502, 2770, 0, 0, 10, 100, 1, 1, 0);"
        );
        let edited = statements(
            CREATURE,
            &key,
            Life::Update,
            &[Assignment { column: "ChanceOrQuestChance", value: "-25".into() }],
        );
        assert_eq!(
            edited,
            vec![
                "UPDATE `creature_loot_template` SET `ChanceOrQuestChance` = -25 WHERE `entry` = 1502 \
                 AND `item` = 2770 AND `groupid` = 0 AND `patch_min` = 0 AND `patch_max` = 10;"
            ]
        );
        assert_eq!(
            statements(CREATURE, &key, Life::Delete, &[]),
            vec![
                "DELETE FROM `creature_loot_template` WHERE `entry` = 1502 AND `item` = 2770 AND \
                 `groupid` = 0 AND `patch_min` = 0 AND `patch_max` = 10;"
            ]
        );
        // A reference row carries the entry negated, and the float is written
        // as the table prints it.
        let reference = new_reference(1502, 30001);
        assert!(reference.iter().any(|a| a.column == "mincountOrRef" && a.value == "-30001"));
        assert!(reference.iter().any(|a| a.column == "ChanceOrQuestChance" && a.value == "100"));
        let half = Entry { chance: 0.5, ..Entry::item(1, 2) }.assignments();
        assert!(half.iter().any(|a| a.column == "ChanceOrQuestChance" && a.value == "0.5"));
    }

    /// Every table is named once, resolves to itself, and has a heading.
    #[test]
    fn the_nine_tables_are_named_once() {
        assert_eq!(ALL.len(), TABLES.len());
        for name in TABLES {
            assert_eq!(table_named(name), Some(name));
            assert_eq!(table(name).map(|t| t.name), Some(name));
            assert!(name.ends_with("_loot_template"));
        }
        assert!(table_named("creature_template").is_none());
        assert!(columns_of("npc_vendor").is_empty());
    }

    /// The read filters on the patch band and orders by group, so a window
    /// draws a group's rows together.
    #[test]
    fn the_read_is_one_set_at_the_servers_patch() {
        let sql = rows_query(CREATURE, 68, 10);
        assert!(sql.contains("`entry` = 68"), "{sql}");
        assert!(sql.contains("10 BETWEEN `patch_min` AND `patch_max`"), "{sql}");
        assert!(sql.ends_with("ORDER BY `groupid`, `item`"), "{sql}");
    }
}
