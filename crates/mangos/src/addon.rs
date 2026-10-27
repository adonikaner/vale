//! `creature_addon` and `creature_equip_template`: how one spawn stands and
//! what a creature holds.
//!
//! ## Two tables, two keys
//!
//! ```text
//! creature_addon            one row per spawn guid and content patch: the
//!                           display, mount, equipment set, stand state,
//!                           sheath state, emote and auras that override the
//!                           template's for that one spawn
//! creature_equip_template   a set of weapon rows under one entry: each row
//!                           three item entries and a probability, and a
//!                           spawn rolls one row of its set when it loads
//! ```
//!
//! `creature_template.equipment_id` names a set for every spawn of the
//! template, and `creature_addon.equipment_id` names one for a single spawn.
//! On the reference database 5,139 entries hold 5,182 equipment rows, so
//! almost every set is one row, and 3,416 spawns have an addon row.
//!
//! ## What the server reads
//!
//! `ObjectMgr::LoadCreatureAddons` (`ObjectMgr.cpp:1718`) reads
//! `creature_addon` through `SQLStorage::LoadProgressive`: for each guid, the
//! row with the highest `patch` at or below the server's `WowPatch`. It then
//! clears a display, mount or equipment id the tables do not hold, puts a
//! stand state at or past 10 back to standing, a sheath state past 2 back to
//! melee, and an emote `Emotes.dbc` lacks back to 0, each with a log line.
//! `ObjectMgr::LoadEquipmentTemplates` (`ObjectMgr.cpp:1730`) reads the
//! equipment rows whose `patch_min` to `patch_max` range holds `WowPatch`,
//! groups them by entry, and forces to 0 an item that is not in
//! `item_template` or whose inventory type is not one a hand holds
//! ([`HELD_IN_A_HAND`]). Both run at startup, and neither table has a
//! `.reload`: a change needs a restart.
//!
//! ## The addon's three sentinel columns
//!
//! `Creature::LoadCreatureAddon` (`Creature.cpp:2866`) and
//! `LoadDefaultEquipment` (`Creature.cpp:2062`) read the row's `-1` and `0`
//! differently per column:
//!
//! ```text
//! display_id         0 the template's display; else this display
//! mount_display_id   -1 the template's mount; 0 no mount; else this mount
//! equipment_id       -1 the template's set; 0 no weapons; else this set
//! auras              empty the template's auras; else these spells
//! ```
//!
//! The stand state, the sheath state and the emote have no sentinel: a spawn
//! with an addon row stands, sheathes and emotes as the row says, and one
//! without stands up, holds its melee weapon and plays no emote.
//!
//! ## A row is a keyed row of the project's store
//!
//! On `crate::vendor`'s terms: a created row is a `DELETE` and an `INSERT`
//! naming every column, an edit an `UPDATE`, a removal a `DELETE`. An addon
//! is keyed by `(guid, patch)`, which is the table's primary key. An
//! equipment row is keyed by `(entry, item1, item2, item3)`, also its primary
//! key, so changing an item is a removal and a creation; `probability`,
//! `patch_min` and `patch_max` are its ordinary columns.

use crate::row::{Assignment, Key, Table};

pub use crate::schema::{value_word, Column, Group, Kind, Row, RowValue, Value};

pub const ADDON: &str = "creature_addon";
pub const EQUIPMENT: &str = "creature_equip_template";

/// Both tables, in the order a plan writes them: a set before the addon that
/// names it, since the loader clears an equipment id it does not hold.
pub const TABLES: &[Table] = &[EQUIPMENT_TABLE, ADDON_TABLE];

pub const ADDON_TABLE: Table = Table::new(ADDON, &ADDON_COLUMNS);
pub const EQUIPMENT_TABLE: Table = Table::new(EQUIPMENT, &EQUIPMENT_COLUMNS);

/// `UnitStandStateType`, from `UnitDefines.h:100`. The loader puts a value
/// at or past [`MAX_STAND_STATE`] back to 0.
pub const STAND_STATES: [Value; 10] = [
    Value { value: 0, name: "Standing" },
    Value { value: 1, name: "Sitting" },
    Value { value: 2, name: "Sitting on a chair" },
    Value { value: 3, name: "Sleeping" },
    Value { value: 4, name: "Sitting on a low chair" },
    Value { value: 5, name: "Sitting on a medium chair" },
    Value { value: 6, name: "Sitting on a high chair" },
    Value { value: 7, name: "Dead" },
    Value { value: 8, name: "Kneeling" },
    Value { value: 9, name: "Custom (the model's own: submerged, frozen, hiding, resting)" },
];

/// `MAX_UNIT_STAND_STATE`.
pub const MAX_STAND_STATE: u32 = 10;

/// `SheathState`, from `UnitDefines.h:168`. The loader puts a value past
/// [`MAX_SHEATH_STATE`] back to 1.
pub const SHEATH_STATES: [Value; 3] = [
    Value { value: 0, name: "Unarmed: weapons on the back and hip" },
    Value { value: 1, name: "Melee weapon in hand" },
    Value { value: 2, name: "Ranged weapon in hand" },
];

/// `SHEATH_STATE_RANGED`, the highest sheath state.
pub const MAX_SHEATH_STATE: u32 = 2;

/// The inventory types `LoadEquipmentTemplates` keeps in an equipment row;
/// an item of any other type is forced to 0 with a log line. The names are
/// `crate::item::INVENTORY_TYPES`'.
pub const HELD_IN_A_HAND: [u32; 10] = [13, 14, 15, 17, 21, 22, 23, 25, 26, 28];

/// Whether an item of this inventory type may be in an equipment row.
pub fn held_in_a_hand(inventory_type: u32) -> bool {
    HELD_IN_A_HAND.contains(&inventory_type)
}

/// The three item columns of an equipment row, in order.
pub const ITEM_COLUMNS: [&str; 3] = ["item1", "item2", "item3"];

/// The three slots' words: what each item column is held in.
pub const SLOT_WORDS: [&str; 3] = ["main hand", "off hand", "ranged"];

/// The addon's columns: `SQLStorages.cpp`'s `CreatureDataAddonInfofmt`
/// (`iiiibbis`) by name.
pub const ADDON_COLUMNS: [Column; 9] = [
    Column { name: "guid", kind: Kind::Key, group: Group::Identity, about: "the creature spawn the row is about" },
    Column { name: "patch", kind: Kind::Key, group: Group::Identity, about: "the content patch the row belongs to; the server reads the highest at or below its WowPatch" },
    Column { name: "display_id", kind: Kind::Ref("CreatureDisplayInfo"), group: Group::Appearance, about: "the CreatureDisplayInfo row this spawn is drawn as, or 0 for the template's" },
    Column { name: "mount_display_id", kind: Kind::Signed, group: Group::Appearance, about: "the CreatureDisplayInfo row this spawn rides: -1 the template's mount, 0 none" },
    Column { name: "equipment_id", kind: Kind::Signed, group: Group::Appearance, about: "the creature_equip_template set this spawn holds: -1 the template's set, 0 no weapons" },
    Column { name: "stand_state", kind: Kind::Choice(&STAND_STATES), group: Group::Behaviour, about: "how the spawn stands, sits, kneels or lies" },
    Column { name: "sheath_state", kind: Kind::Choice(&SHEATH_STATES), group: Group::Behaviour, about: "which weapon the spawn holds in its hands" },
    Column { name: "emote_state", kind: Kind::Ref("Emotes"), group: Group::Behaviour, about: "the Emotes.dbc emote the spawn keeps playing while it stands, or 0" },
    Column { name: "auras", kind: Kind::RefList("Spell"), group: Group::Behaviour, about: "spell ids of the auras the spawn has from the start, space separated; empty uses the template's" },
];

/// An equipment row's columns, in the table's order.
pub const EQUIPMENT_COLUMNS: [Column; 7] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the set: what creature_template.equipment_id and creature_addon.equipment_id name" },
    Column { name: "probability", kind: Kind::Unsigned, group: Group::Appearance, about: "the row's weight when a spawn rolls one row of its set" },
    Column { name: "item1", kind: Kind::Key, group: Group::Appearance, about: "the item_template entry held in the main hand, or 0" },
    Column { name: "item2", kind: Kind::Key, group: Group::Appearance, about: "the item_template entry held in the off hand, or 0" },
    Column { name: "item3", kind: Kind::Key, group: Group::Appearance, about: "the item_template entry held as the ranged weapon, or 0" },
    Column { name: "patch_min", kind: Kind::Choice(&crate::creature::PATCHES), group: Group::Respawn, about: "lowest content patch the row is read in" },
    Column { name: "patch_max", kind: Kind::Choice(&crate::creature::PATCHES), group: Group::Respawn, about: "highest content patch the row is read in" },
];

/// The key of an addon row.
pub fn addon_key(guid: u64, patch: u32) -> Key {
    Key::two(("guid", guid), ("patch", u64::from(patch)))
}

/// The key of an equipment row.
pub fn equipment_key(entry: u32, items: [u32; 3]) -> Key {
    Key(vec![
        ("entry".to_string(), entry.to_string()),
        ("item1".to_string(), items[0].to_string()),
        ("item2".to_string(), items[1].to_string()),
        ("item3".to_string(), items[2].to_string()),
    ])
}

/// One `creature_addon` row, as the server reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Addon {
    pub guid: u64,
    pub patch: u32,
    pub display_id: u32,
    pub mount_display_id: i32,
    pub equipment_id: i32,
    pub stand_state: u32,
    pub sheath_state: u32,
    pub emote_state: u32,
    /// The `auras` column as text: spell ids separated by spaces, or empty.
    pub auras: String,
}

impl Addon {
    /// A row that changes nothing: the template's display, mount, set and
    /// auras, standing, melee weapon in hand, no emote.
    pub fn new(guid: u64, patch: u32) -> Addon {
        Addon {
            guid,
            patch,
            display_id: 0,
            mount_display_id: -1,
            equipment_id: -1,
            stand_state: 0,
            sheath_state: 1,
            emote_state: 0,
            auras: String::new(),
        }
    }

    pub fn key(&self) -> Key {
        addon_key(self.guid, self.patch)
    }

    /// Every column that is not the key, as SQL literals.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "display_id", value: self.display_id.to_string() },
            Assignment { column: "mount_display_id", value: self.mount_display_id.to_string() },
            Assignment { column: "equipment_id", value: self.equipment_id.to_string() },
            Assignment { column: "stand_state", value: self.stand_state.to_string() },
            Assignment { column: "sheath_state", value: self.sheath_state.to_string() },
            Assignment { column: "emote_state", value: self.emote_state.to_string() },
            Assignment { column: "auras", value: crate::sql::text(&self.auras) },
        ]
    }

    /// One row as the database answered it, or `None` for a row missing its
    /// guid.
    pub fn from_row(row: &Row) -> Option<Addon> {
        let unsigned = |column: &str| row.integer(column).unwrap_or(0).max(0) as u32;
        let signed = |column: &str| row.integer(column).unwrap_or(-1) as i32;
        Some(Addon {
            guid: row.integer("guid")?.max(0) as u64,
            patch: unsigned("patch"),
            display_id: unsigned("display_id"),
            mount_display_id: signed("mount_display_id"),
            equipment_id: signed("equipment_id"),
            stand_state: unsigned("stand_state"),
            sheath_state: unsigned("sheath_state"),
            emote_state: unsigned("emote_state"),
            auras: row.text("auras").unwrap_or("").trim().to_string(),
        })
    }

    /// The aura spells, parsed as the loader parses them.
    pub fn aura_spells(&self) -> Vec<u32> {
        self.auras.split_whitespace().filter_map(|word| word.parse().ok()).collect()
    }

    /// What the loader would put back to a default, each as a sentence, or
    /// nothing. The display, mount, set and emote are checked by the caller,
    /// which has the tables.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.stand_state >= MAX_STAND_STATE {
            out.push(format!(
                "stand_state {} is not one of the {MAX_STAND_STATE} stand states; the server stands the spawn up",
                self.stand_state
            ));
        }
        if self.sheath_state > MAX_SHEATH_STATE {
            out.push(format!(
                "sheath_state {} is not one of the 3 sheath states; the server holds the melee weapon",
                self.sheath_state
            ));
        }
        let bad: Vec<&str> = self
            .auras
            .split_whitespace()
            .filter(|word| word.parse::<u32>().is_err())
            .collect();
        if !bad.is_empty() {
            out.push(format!("auras holds {}, which is not a spell id", bad.join(", ")));
        }
        out
    }
}

/// One `creature_equip_template` row, as the server reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Equipment {
    pub entry: u32,
    pub items: [u32; 3],
    pub probability: u32,
    pub patch_min: u32,
    pub patch_max: u32,
}

impl Equipment {
    /// A row holding `items`, at weight 1, read at every patch up to the
    /// last 1.12 one.
    pub fn new(entry: u32, items: [u32; 3]) -> Equipment {
        Equipment {
            entry,
            items,
            probability: 1,
            patch_min: 0,
            patch_max: crate::conn::DEFAULT_WOW_PATCH,
        }
    }

    pub fn key(&self) -> Key {
        equipment_key(self.entry, self.items)
    }

    /// Every column that is not the key, as SQL literals.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "probability", value: self.probability.to_string() },
            Assignment { column: "patch_min", value: self.patch_min.to_string() },
            Assignment { column: "patch_max", value: self.patch_max.to_string() },
        ]
    }

    /// One row as the database answered it, or `None` for a row missing a
    /// key column.
    pub fn from_row(row: &Row) -> Option<Equipment> {
        let unsigned = |column: &str| row.integer(column).unwrap_or(0).max(0) as u32;
        Some(Equipment {
            entry: row.integer("entry")? as u32,
            items: [
                row.integer("item1")? as u32,
                row.integer("item2")? as u32,
                row.integer("item3")? as u32,
            ],
            probability: unsigned("probability"),
            patch_min: unsigned("patch_min"),
            patch_max: unsigned("patch_max"),
        })
    }

    /// Whether the server reads the row at `wow_patch`.
    pub fn read_at(&self, wow_patch: u32) -> bool {
        (self.patch_min..=self.patch_max).contains(&wow_patch)
    }

    /// Why the server would skip or empty the row, from the row alone. The
    /// items' inventory types are checked by the caller, which has the item
    /// rows.
    ///
    /// A row of three empty slots is not a fault: the loader keeps it, and a
    /// spawn that rolls it holds nothing, so a set can arm a creature only
    /// some of the time.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.patch_min > self.patch_max {
            out.push(format!(
                "patch_min {} is above patch_max {}, so no patch reads the row",
                self.patch_min, self.patch_max
            ));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// The queries
// ---------------------------------------------------------------------------

/// The addon row the server would load for one spawn: the highest patch at
/// or below `wow_patch`, which is what `LoadProgressive` keeps.
pub fn addon_query(guid: u64, wow_patch: u32) -> String {
    format!(
        "SELECT * FROM {} WHERE `guid` = {guid} AND `patch` <= {wow_patch} ORDER BY `patch` DESC LIMIT 1",
        crate::sql::name(ADDON)
    )
}

/// Every row of one equipment set, at every patch, in the table's key order.
/// The rows outside the server's patch are read too, so a window can say
/// why a row is not held.
pub fn equipment_query(entry: u32) -> String {
    format!(
        "SELECT * FROM {} WHERE `entry` = {entry} ORDER BY `patch_min`, `item1`, `item2`, `item3`",
        crate::sql::name(EQUIPMENT)
    )
}

/// Every row of several sets, at every patch, in the table's key order.
/// `None` for an empty list rather than a statement with an empty `IN ()`,
/// which MySQL refuses.
pub fn sets_query(entries: &[u32]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let list: Vec<String> = entries.iter().map(u32::to_string).collect();
    Some(format!(
        "SELECT * FROM {} WHERE `entry` IN ({}) ORDER BY `entry`, `patch_min`, `item1`, `item2`, `item3`",
        crate::sql::name(EQUIPMENT),
        list.join(", ")
    ))
}

/// The creatures whose template names a set, one line per entry, and how
/// many spawns name it in their addon row, as a second query.
pub fn users_query(entry: u32) -> String {
    format!(
        "SELECT `entry`, MAX(`name`) AS `name` FROM `creature_template` \
         WHERE `equipment_id` = {entry} GROUP BY `entry` ORDER BY `entry`"
    )
}

/// How many addon rows name a set.
pub fn addon_users_query(entry: u32) -> String {
    format!("SELECT COUNT(*) AS `n` FROM {} WHERE `equipment_id` = {entry}", crate::sql::name(ADDON))
}

/// The highest set entry the table holds, for a new set's number.
pub fn max_entry_query() -> String {
    format!("SELECT MAX(`entry`) AS `n` FROM {}", crate::sql::name(EQUIPMENT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::row::Life;

    /// The columns are the storage format's and the table's own, and the keys
    /// are each table's primary key.
    #[test]
    fn every_column_is_the_servers_own_in_the_tables_order() {
        let addon: Vec<&str> = ADDON_COLUMNS.iter().map(|column| column.name).collect();
        assert_eq!(
            addon,
            ["guid", "patch", "display_id", "mount_display_id", "equipment_id", "stand_state", "sheath_state", "emote_state", "auras"]
        );
        let keyed: Vec<&str> = ADDON_COLUMNS.iter().filter(|c| !c.editable()).map(|c| c.name).collect();
        assert_eq!(keyed, ["guid", "patch"]);
        let keyed: Vec<&str> = EQUIPMENT_COLUMNS.iter().filter(|c| !c.editable()).map(|c| c.name).collect();
        assert_eq!(keyed, ["entry", "item1", "item2", "item3"]);
        assert_eq!(STAND_STATES.len() as u32, MAX_STAND_STATE);
        assert_eq!(SHEATH_STATES.len() as u32, MAX_SHEATH_STATE + 1);
        assert!(held_in_a_hand(21) && held_in_a_hand(14) && !held_in_a_hand(1) && !held_in_a_hand(0));
    }

    /// A new addon changes nothing about the spawn, and the three sentinels
    /// read back as the server reads them.
    #[test]
    fn a_new_addon_is_the_templates_own_values() {
        let addon = Addon::new(4805, 0);
        assert_eq!((addon.mount_display_id, addon.equipment_id, addon.sheath_state), (-1, -1, 1));
        assert!(addon.check().is_empty());
        let mut row = Row::new();
        for (column, value) in [
            ("guid", "4805"),
            ("patch", "4"),
            ("display_id", "0"),
            ("mount_display_id", "-1"),
            ("equipment_id", "0"),
            ("stand_state", "1"),
            ("sheath_state", "0"),
            ("emote_state", "234"),
            ("auras", "18950 12544"),
        ] {
            row.insert(column.to_string(), Some(value.to_string()));
        }
        let read = Addon::from_row(&row).expect("a whole row");
        assert_eq!((read.patch, read.equipment_id, read.emote_state), (4, 0, 234));
        assert_eq!(read.aura_spells(), [18950, 12544]);
        let faulty = Addon { stand_state: 10, sheath_state: 3, auras: "18950 x".into(), ..Addon::new(1, 0) };
        let faults = faulty.check();
        assert_eq!(faults.len(), 3, "{faults:?}");
    }

    /// A created addon is a `DELETE` and an `INSERT` naming every column, and
    /// an equipment row is keyed by its entry and its three items.
    #[test]
    fn a_row_becomes_the_statements_it_means() {
        let addon = Addon { mount_display_id: 2402, ..Addon::new(4805, 0) };
        let created = ADDON_TABLE.statements(&addon.key(), Life::Insert, &addon.assignments());
        assert_eq!(
            created,
            vec![
                "DELETE FROM `creature_addon` WHERE `guid` = 4805 AND `patch` = 0;".to_string(),
                "INSERT INTO `creature_addon` (`guid`, `patch`, `display_id`, `mount_display_id`, \
                 `equipment_id`, `stand_state`, `sheath_state`, `emote_state`, `auras`) VALUES \
                 (4805, 0, 0, 2402, -1, 0, 1, 0, '');"
                    .to_string(),
            ]
        );
        let set = Equipment::new(12799, [10612, 0, 0]);
        let created = EQUIPMENT_TABLE.statements(&set.key(), Life::Insert, &set.assignments());
        assert_eq!(
            created[1],
            "INSERT INTO `creature_equip_template` (`entry`, `item1`, `item2`, `item3`, `probability`, \
             `patch_min`, `patch_max`) VALUES (12799, 10612, 0, 0, 1, 0, 10);"
        );
        let edited = EQUIPMENT_TABLE.statements(
            &equipment_key(12799, [10612, 0, 0]),
            Life::Update,
            &[Assignment { column: "probability", value: "3".into() }],
        );
        assert_eq!(
            edited,
            vec![
                "UPDATE `creature_equip_template` SET `probability` = 3 WHERE `entry` = 12799 AND `item1` = 10612 \
                 AND `item2` = 0 AND `item3` = 0;"
            ]
        );
    }

    /// An equipment row says which patches read it, and what the loader
    /// would do with it.
    #[test]
    fn an_equipment_row_knows_its_patches_and_its_faults() {
        let set = Equipment::new(62, [1904, 0, 0]);
        assert!(set.read_at(0) && set.read_at(10) && !set.read_at(11));
        assert!(set.check().is_empty());
        let empty = Equipment::new(62, [0, 0, 0]);
        assert!(empty.check().is_empty());
        let backwards = Equipment { patch_min: 5, patch_max: 2, ..set.clone() };
        assert!(backwards.check()[0].contains("above"));
        assert!(!backwards.read_at(3));
        assert!(addon_query(4805, 10).contains("`patch` <= 10"));
        assert!(equipment_query(62).ends_with("ORDER BY `patch_min`, `item1`, `item2`, `item3`"));
        assert_eq!(sets_query(&[]), None);
        assert!(sets_query(&[62, 68]).unwrap().contains("WHERE `entry` IN (62, 68)"));
    }
}
