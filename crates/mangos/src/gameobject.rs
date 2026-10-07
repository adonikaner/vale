//! `gameobject_template` and `gameobject`: what a game object is, and where one
//! stands.
//!
//! ## The two tables, on [`crate::creature`]'s pattern
//!
//! A `gameobject_template` row is what an object is: its type, its model,
//! its name, and the 24 `data` columns whose meaning its type decides. Changing
//! one changes every Copper Vein in the world. 9,016 rows over 8,993 entries on
//! the reference install.
//!
//! A `gameobject` row is one spawn: a template entry, a map, a position, a
//! facing and a rotation, and how it respawns. 56,547 of them over 34 maps.
//!
//! A creature has, and a game object does not: a waypoint path, an addon
//! row, equipment, an npc flag. The next two sections state what a game object
//! has and a creature does not.
//!
//! ## The 24 data columns are named by the type
//!
//! `data0` is a lock on a door, a chest and a goober, the number of seats on a
//! chair, a spell focus id on a forge, and a page of text on a book.
//! `GameObjectInfo` (`GameObjectDefines.h:211`) is a C union of one struct per
//! type, and [`data_fields`] is that union as data: for a type, what each of
//! its `data` columns is called, what kind of value it holds, and what it does.
//! A column the type's struct does not reach has no name and is drawn as
//! `dataN`.
//!
//! A gathering node is a chest. A Copper Vein is type 3 with `data0`, the
//! lock, naming `Lock.dbc` row 38, which is keyed on Mining at skill 0 (a Tin
//! Vein's row 39 asks for 65), and `data1` naming its
//! `gameobject_loot_template` entry. A herb is the same with Herbalism.
//! No column of the row states that it is a mining node; the lock does, and
//! `Lock.dbc` is the client's file, so the editor resolves it and this crate
//! names the column. [`CHEST_LOCK`] and [`CHEST_LOOT`] are the two indices.
//!
//! ## A spawn has a facing and a rotation, and they have to agree
//!
//! `orientation` is the facing in radians and `rotation0..3` is a quaternion.
//! `GameObject::UpdateRotationFields` (`GameObject.cpp:2054`) sends the four
//! floats as they are unless `rotation2` and `rotation3` are both zero, in which
//! case it derives them from the facing: `sin(o / 2)` and `cos(o / 2)`. 56,037
//! of the reference install's 56,547 rows carry a rotation of their own, so an
//! edit to `orientation` alone leaves the quaternion the client is sent
//! pointing the old way.
//!
//! [`rotation_for`] is the pair `.gobject turn` writes, and an edit to a spawn's
//! facing writes it beside the facing; see [`facing`]. A row whose `rotation0`
//! or `rotation1` is not zero is tilted off the vertical (1,410 rows); its
//! quaternion is not a function of the facing and is left alone.
//!
//! ## The patch column is part of the template's key
//!
//! The key is `(entry, patch)`, loaded as the highest patch at or below the
//! server's `WowPatch` (`ObjectMgr.cpp:7852`). This is [`crate::creature`]'s
//! arrangement, and for the same reason an edit names both columns. 1,083 of
//! the 9,016 rows are at a patch above 0.
//!
//! ## A spawn can be created and removed; a template can be created and not removed
//!
//! The rules are [`crate::creature`]'s. A removal takes the two tables
//! `GameObject::DeleteFromDB` takes (`GameObject.cpp:1037`); see
//! [`DEPENDENTS`]. `LoadGameObjectInfo` overwrites an entry it has read and
//! never drops one, so a removed template cannot be made live. A new template
//! starts from [`new_template`], a generic object with no model, and its entry
//! comes from [`RESERVED_ENTRY_BASE`].
//!
//! ## An edit to either table does not reach a running server
//!
//! `.reload gameobject` calls `LoadGameobjects(true)`, which adds a row it has
//! not seen to its grid (`ObjectMgr.cpp:2646`) and never erases one. `.reload
//! gameobject_template` rewrites the `GameObjectInfo` in place, but an object
//! already in the world took its display id, faction, flags and size from the
//! template when it was created (`GameObject::Create`). An applied row is
//! therefore live after a restart, as a creature's is.

use crate::row::{Assignment, Key, Life};

pub use crate::schema::{mask_words, value_word, Bit, Column, Group, Kind, Row, RowValue, Value};

/// What a game object is.
pub const TEMPLATE: &str = "gameobject_template";

/// Where one stands.
pub const SPAWN: &str = "gameobject";

/// Every table this module writes, for a caller that has to resolve a name read
/// out of a file back to one of these constants.
pub const TABLES: [&str; 2] = [TEMPLATE, SPAWN];

/// The static name for a table read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// `GameobjectTypes`, from `GameObjectDefines.h:25`. The values 1.12.1 has:
/// the header compiles 24 to 30 in by client build and 5875 is past all of
/// them.
pub const TYPES: [Value; 31] = [
    Value { value: 0, name: "Door" },
    Value { value: 1, name: "Button" },
    Value { value: 2, name: "Quest giver" },
    Value { value: 3, name: "Chest" },
    Value { value: 4, name: "Binder" },
    Value { value: 5, name: "Generic" },
    Value { value: 6, name: "Trap" },
    Value { value: 7, name: "Chair" },
    Value { value: 8, name: "Spell focus" },
    Value { value: 9, name: "Text" },
    Value { value: 10, name: "Goober" },
    Value { value: 11, name: "Transport" },
    Value { value: 12, name: "Area damage" },
    Value { value: 13, name: "Camera" },
    Value { value: 14, name: "Map object" },
    Value { value: 15, name: "Map object transport" },
    Value { value: 16, name: "Duel arbiter" },
    Value { value: 17, name: "Fishing node" },
    Value { value: 18, name: "Summoning ritual" },
    Value { value: 19, name: "Mailbox" },
    Value { value: 20, name: "Auction house" },
    Value { value: 21, name: "Guard post" },
    Value { value: 22, name: "Spell caster" },
    Value { value: 23, name: "Meeting stone" },
    Value { value: 24, name: "Flag stand" },
    Value { value: 25, name: "Fishing hole" },
    Value { value: 26, name: "Flag drop" },
    Value { value: 27, name: "Mini game" },
    Value { value: 28, name: "Lottery kiosk" },
    Value { value: 29, name: "Capture point" },
    Value { value: 30, name: "Aura generator" },
];

/// `GAMEOBJECT_TYPE_CHEST`, which is also what a mining vein and a herb are.
pub const TYPE_CHEST: u32 = 3;

/// A chest's `data0`: its `Lock.dbc` row, which is what makes one a gathering
/// node.
pub const CHEST_LOCK: usize = 0;

/// A chest's `data1`: the `gameobject_loot_template` entry it is looted from.
pub const CHEST_LOOT: usize = 1;

/// `GameObjectFlags`, from `GameObjectDefines.h:70`.
pub const FLAGS: [Bit; 7] = [
    Bit { bit: 0x01, name: "IN_USE", about: "disables interaction while animated" },
    Bit { bit: 0x02, name: "LOCKED", about: "needs a key, a spell or an event; the tooltip says Locked" },
    Bit { bit: 0x04, name: "INTERACT_COND", about: "cannot be interacted with until a condition is met" },
    Bit { bit: 0x08, name: "TRANSPORT", about: "carries units standing on it, such as an elevator or a boat" },
    Bit { bit: 0x10, name: "NO_INTERACT", about: "players cannot interact with it" },
    Bit { bit: 0x20, name: "NODESPAWN", about: "never despawns; a door only changes state" },
    Bit { bit: 0x40, name: "TRIGGERED", about: "summoned, or set off by a spell or an event" },
];

/// `GOState`, from `GameObjectDefines.h:177`: what the client draws it as.
pub const STATES: [Value; 3] = [
    Value { value: 0, name: "Active (a door stands open)" },
    Value { value: 1, name: "Ready (a door stands shut)" },
    Value { value: 2, name: "Active, alternative" },
];

/// `SpawnFlags`, from `ObjectDefines.h:127`: one enumeration for both spawn
/// tables, so [`crate::creature::SPAWN_FLAGS`] is the list.
pub use crate::creature::SPAWN_FLAGS;

/// No and yes, for the `data` columns that are switches.
pub use crate::schema::NO_YES;

/// `gameobject_template`'s 34 columns, in the order
/// `ObjectMgr::LoadGameObjectTemplates` selects them (`ObjectMgr.cpp:7852`).
///
/// `patch` is the other half of the key and is not in the server's `SELECT`, so
/// it is not in this list either; it arrives through the key, as
/// `creature_template`'s does.
///
/// `data1` and `data6` are the two signed ones in the DDL: a goober's `questId`
/// and a trap's `autoCloseTime` are read as `int32`.
pub const TEMPLATE_COLUMNS: [Column; 35] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the game object id" },
    Column { name: "type", kind: Kind::Choice(&TYPES), group: Group::Identity, about: "the game object type; it decides what the 24 data columns mean" },
    Column { name: "displayId", kind: Kind::Ref("GameObjectDisplayInfo"), group: Group::Appearance, about: "GameObjectDisplayInfo.dbc id of its model" },
    Column { name: "name", kind: Kind::Text, group: Group::Identity, about: "the name shown in the tooltip" },
    Column { name: "icon", kind: Kind::Text, group: Group::Identity, about: "the fifth string of the query response; empty for every object except the two that show PVP" },
    Column { name: "faction", kind: Kind::Ref("FactionTemplate"), group: Group::Identity, about: "FactionTemplate.dbc id; decides who may use it" },
    Column { name: "flags", kind: Kind::Flags(&FLAGS), group: Group::Behaviour, about: "flags: locked, no interaction, never despawns, and others" },
    Column { name: "size", kind: Kind::Float, group: Group::Appearance, about: "size multiplier for the model" },
    Column { name: "data0", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data1", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data2", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data3", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data4", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data5", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data6", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data7", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data8", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data9", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data10", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data11", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data12", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data13", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data14", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data15", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data16", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data17", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data18", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data19", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data20", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data21", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data22", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "data23", kind: Kind::Signed, group: Group::Stats, about: "its meaning depends on the type" },
    Column { name: "mingold", kind: Kind::Money, group: Group::Loot, about: "the least copper a chest holds" },
    Column { name: "maxgold", kind: Kind::Money, group: Group::Loot, about: "the most copper a chest holds" },
    Column { name: "script_name", kind: Kind::Text, group: Group::Advanced, about: "the name of a C++ script the server registers, or empty" },
];

/// How many `data` columns a template has.
pub const DATA_COLUMNS: usize = 24;

/// The name of the `index`th data column.
pub fn data_column(index: usize) -> Option<&'static str> {
    const NAMES: [&str; DATA_COLUMNS] = [
        "data0", "data1", "data2", "data3", "data4", "data5", "data6", "data7", "data8", "data9",
        "data10", "data11", "data12", "data13", "data14", "data15", "data16", "data17", "data18",
        "data19", "data20", "data21", "data22", "data23",
    ];
    NAMES.get(index).copied()
}

/// The index of the data column named `column`, or `None` for a column that is
/// not one.
pub fn data_index(column: &str) -> Option<usize> {
    let index: usize = column.strip_prefix("data")?.parse().ok()?;
    (index < DATA_COLUMNS).then_some(index)
}

/// What one `data` column is for one type: the field of `GameObjectInfo`'s
/// union it lands in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DataField {
    /// vmangos' own field name, which is also what the wiki and every SQL
    /// comment call it.
    pub name: &'static str,
    /// What a form draws it as. The value written is the number either way.
    pub kind: Kind,
    pub about: &'static str,
}

const fn field(name: &'static str, kind: Kind, about: &'static str) -> DataField {
    DataField { name, kind, about }
}

const LOCK: Kind = Kind::Ref("Lock");
const SPELL: Kind = Kind::Ref("Spell");
const QUEST: Kind = Kind::Ref("quest_template");
const TRAP: Kind = Kind::Ref(TEMPLATE);
const PAGE: Kind = Kind::Ref("page_text");
const SWITCH: Kind = Kind::Choice(&NO_YES);
const NUMBER: Kind = Kind::Unsigned;

const LOCK_ABOUT: &str = "Lock.dbc id: what opens it, a key item or a skill at a rank";
const TRAP_ABOUT: &str = "gameobject_template entry of a trap set off when this is used";
const AUTO_CLOSE: &str = "time before it resets, in 65536ths of a second; 0 never resets";
const OPEN_TEXT: &str = "broadcast text shown on the cast bar while opening";
const CLOSE_TEXT: &str = "broadcast text shown on the cast bar while closing";
const LOS_OK: &str = "usable without line of sight";
const LARGE: &str = "seen from further away";
const NO_DAMAGE: &str = "taking damage does not interrupt using it";
const MOUNTED: &str = "usable while mounted";
const SERVER_ONLY: &str = "never sent to a client";
const EVENT: &str = "event_scripts id run when it is used";

const DOOR: [DataField; 6] = [
    field("startOpen", SWITCH, "whether the client reads Active as open or as shut"),
    field("lockId", LOCK, LOCK_ABOUT),
    field("autoCloseTime", NUMBER, AUTO_CLOSE),
    field("noDamageImmune", SWITCH, NO_DAMAGE),
    field("openTextID", NUMBER, OPEN_TEXT),
    field("closeTextID", NUMBER, CLOSE_TEXT),
];

const BUTTON: [DataField; 9] = [
    field("startOpen", SWITCH, "whether the client reads Active as pressed or as released"),
    field("lockId", LOCK, LOCK_ABOUT),
    field("autoCloseTime", NUMBER, AUTO_CLOSE),
    field("linkedTrapId", TRAP, TRAP_ABOUT),
    field("noDamageImmune", SWITCH, NO_DAMAGE),
    field("large", SWITCH, LARGE),
    field("openTextID", NUMBER, OPEN_TEXT),
    field("closeTextID", NUMBER, CLOSE_TEXT),
    field("losOK", SWITCH, LOS_OK),
];

const QUESTGIVER: [DataField; 10] = [
    field("lockId", LOCK, LOCK_ABOUT),
    field("questList", NUMBER, "not read by the server: the quests are the two relation tables"),
    field("pageMaterial", NUMBER, "PageTextMaterial.dbc id: parchment, stone, bronze"),
    field("gossipID", NUMBER, "gossip_menu entry shown when it is used"),
    field("customAnim", NUMBER, "which of the model's custom animations plays on use, 1 to 4"),
    field("noDamageImmune", SWITCH, NO_DAMAGE),
    field("openTextID", NUMBER, OPEN_TEXT),
    field("losOK", SWITCH, LOS_OK),
    field("allowMounted", SWITCH, MOUNTED),
    field("large", SWITCH, LARGE),
];

const CHEST: [DataField; 16] = [
    field("lockId", LOCK, "Lock.dbc id: what opens it. Mining or Herbalism at a rank makes it a gathering node"),
    field("lootId", Kind::Ref("gameobject_loot_template"), "gameobject_loot_template entry of the loot it holds"),
    field("chestRestockTime", Kind::Seconds, "seconds before a partly looted chest refills"),
    field("consumable", SWITCH, "despawns when it has been looted"),
    field("minSuccessOpens", NUMBER, "least times a vein can be mined before it is spent"),
    field("maxSuccessOpens", NUMBER, "most times a vein can be mined before it is spent"),
    field("eventId", NUMBER, "event_scripts id run when it is looted"),
    field("linkedTrapId", TRAP, TRAP_ABOUT),
    field("questId", QUEST, "the quest a player must have for it to be usable"),
    field("level", NUMBER, "least level to open it"),
    field("losOK", SWITCH, LOS_OK),
    field("leaveLoot", SWITCH, "loot left in it stays when the window closes"),
    field("notInCombat", SWITCH, "cannot be opened in combat"),
    field("logLoot", SWITCH, "its loot is written to the server log"),
    field("openTextID", NUMBER, OPEN_TEXT),
    field("groupLootRules", SWITCH, "its items are rolled for by the group"),
];

const GENERIC: [DataField; 6] = [
    field("floatingTooltip", SWITCH, "the name is drawn over it rather than on the pointer"),
    field("highlight", SWITCH, "glows under the pointer"),
    field("serverOnly", SWITCH, SERVER_ONLY),
    field("large", SWITCH, LARGE),
    field("floatOnWater", SWITCH, "stands on the water's surface"),
    field("questID", QUEST, "the quest a player must have for it to sparkle"),
];

const TRAP_FIELDS: [DataField; 14] = [
    field("lockId", LOCK, LOCK_ABOUT),
    field("level", NUMBER, "the level its spell is cast at"),
    field("radius", NUMBER, "yards within which a unit sets it off; 0 fires only when another object links to it"),
    field("spellId", SPELL, "the spell it casts"),
    field("charges", NUMBER, "0 is never spent, 1 despawns after firing once"),
    field("cooldown", Kind::Seconds, "seconds between firings"),
    field("autoCloseTime", Kind::Signed, AUTO_CLOSE),
    field("startDelay", Kind::Seconds, "seconds after spawning before it is armed"),
    field("serverOnly", SWITCH, SERVER_ONLY),
    field("stealthed", SWITCH, "invisible until detected"),
    field("large", SWITCH, LARGE),
    field("stealthAffected", SWITCH, "a stealthed unit does not set it off"),
    field("openTextID", NUMBER, OPEN_TEXT),
    field("closeTextID", NUMBER, CLOSE_TEXT),
];

const CHAIR: [DataField; 3] = [
    field("slots", NUMBER, "how many players can sit on it"),
    field("height", NUMBER, "which sitting pose: 0 low, 1 medium, 2 high"),
    field("onlyCreatorUse", SWITCH, "only whoever summoned it may sit"),
];

const SPELL_FOCUS: [DataField; 6] = [
    field("focusId", Kind::Ref("SpellFocusObject"), "SpellFocusObject.dbc id: anvil, forge, cooking fire, moonwell"),
    field("dist", NUMBER, "yards within which a spell that needs the focus can be cast"),
    field("linkedTrapId", TRAP, TRAP_ABOUT),
    field("serverOnly", SWITCH, SERVER_ONLY),
    field("questID", QUEST, "the quest a player must have for it to count"),
    field("large", SWITCH, LARGE),
];

const TEXT: [DataField; 4] = [
    field("pageID", PAGE, "page_text entry of the first page"),
    field("language", Kind::Ref("Languages"), "Languages.dbc id it is written in"),
    field("pageMaterial", NUMBER, "PageTextMaterial.dbc id: parchment, stone, bronze"),
    field("allowMounted", SWITCH, MOUNTED),
];

const GOOBER: [DataField; 20] = [
    field("lockId", LOCK, LOCK_ABOUT),
    field("questId", Kind::Signed, "the quest a player must have for it to be usable; using it counts towards that quest"),
    field("eventId", NUMBER, EVENT),
    field("autoCloseTime", NUMBER, AUTO_CLOSE),
    field("customAnim", NUMBER, "which of the model's custom animations plays on use, 1 to 4"),
    field("consumable", SWITCH, "despawns when it has been used"),
    field("cooldown", Kind::Signed, "seconds before it can be used again"),
    field("pageId", PAGE, "page_text entry shown when it is used"),
    field("language", Kind::Ref("Languages"), "Languages.dbc id the page is written in"),
    field("pageMaterial", NUMBER, "PageTextMaterial.dbc id: parchment, stone, bronze"),
    field("spellId", SPELL, "the spell cast on whoever uses it"),
    field("noDamageImmune", SWITCH, NO_DAMAGE),
    field("linkedTrapId", TRAP, TRAP_ABOUT),
    field("large", SWITCH, LARGE),
    field("openTextID", NUMBER, OPEN_TEXT),
    field("closeTextID", NUMBER, CLOSE_TEXT),
    field("losOK", SWITCH, LOS_OK),
    field("allowMounted", SWITCH, MOUNTED),
    field("floatingTooltip", SWITCH, "the name is drawn over it rather than on the pointer"),
    field("gossipID", NUMBER, "gossip_menu entry shown when it is used"),
];

const TRANSPORT: [DataField; 3] = [
    field("pause", NUMBER, "milliseconds it waits at each end"),
    field("startOpen", SWITCH, "whether it starts at the far end of its travel"),
    field("autoCloseTime", NUMBER, AUTO_CLOSE),
];

const AREA_DAMAGE: [DataField; 8] = [
    field("lockId", LOCK, LOCK_ABOUT),
    field("radius", NUMBER, "yards it reaches"),
    field("damageMin", NUMBER, "least damage a tick"),
    field("damageMax", NUMBER, "most damage a tick"),
    field("damageSchool", NUMBER, "damage school: 0 physical, 1 holy, 2 fire, and so on"),
    field("autoCloseTime", NUMBER, AUTO_CLOSE),
    field("openTextID", NUMBER, OPEN_TEXT),
    field("closeTextID", NUMBER, CLOSE_TEXT),
];

const CAMERA: [DataField; 4] = [
    field("lockId", LOCK, LOCK_ABOUT),
    field("cinematicId", Kind::Ref("CinematicSequences"), "CinematicSequences.dbc id played on use"),
    field("eventID", NUMBER, EVENT),
    field("openTextID", NUMBER, OPEN_TEXT),
];

const MO_TRANSPORT: [DataField; 7] = [
    field("taxiPathId", Kind::Ref("TaxiPath"), "TaxiPath.dbc id: the route a boat or zeppelin sails"),
    field("moveSpeed", NUMBER, "movement speed, in yards a second"),
    field("accelRate", NUMBER, "acceleration, in yards a second per second"),
    field("startEventID", NUMBER, "event_scripts id run when it leaves a dock"),
    field("stopEventID", NUMBER, "event_scripts id run when it arrives"),
    field("transportPhysics", NUMBER, "TransportPhysics.dbc id of its rocking motion"),
    field("mapID", Kind::Ref("Map"), "Map.dbc id of the transport's own map"),
];

const FISHING_NODE: [DataField; 2] = [
    field("data0", NUMBER, "not read by the server"),
    field("lootId", Kind::Ref("gameobject_loot_template"), "not read: a bobber's catch is fishing_loot_template by zone"),
];

const RITUAL: [DataField; 8] = [
    field("reqParticipants", NUMBER, "how many players must click it"),
    field("spellId", SPELL, "the spell cast when reqParticipants players have clicked it"),
    field("animSpell", SPELL, "the spell each participant channels"),
    field("ritualPersistent", SWITCH, "stays after the ritual completes"),
    field("casterTargetSpell", SPELL, "a spell cast on the summoner"),
    field("casterTargetSpellTargets", NUMBER, "how many participants that spell reaches"),
    field("castersGrouped", SWITCH, "participants must be in the summoner's group"),
    field("ritualNoTargetCheck", SWITCH, "the summoner needs no target"),
];

const AUCTION_HOUSE: [DataField; 1] =
    [field("auctionHouseID", Kind::Ref("AuctionHouse"), "AuctionHouse.dbc id")];

const GUARD_POST: [DataField; 2] = [
    field("creatureID", Kind::Ref(crate::creature::TEMPLATE), "creature_template entry of the guard it summons"),
    field("charges", NUMBER, "how many guards it can summon"),
];

const SPELL_CASTER: [DataField; 6] = [
    field("spellId", SPELL, "the spell cast on whoever uses it: a portal, a soulwell"),
    field("charges", NUMBER, "uses before it despawns; 0 is unlimited"),
    field("partyOnly", SWITCH, "only the summoner's group may use it"),
    field("allowMounted", SWITCH, MOUNTED),
    field("large", SWITCH, LARGE),
    field("conditionID1", NUMBER, "conditions id a player must meet"),
];

const MEETING_STONE: [DataField; 3] = [
    field("minLevel", NUMBER, "lowest level it will queue"),
    field("maxLevel", NUMBER, "highest level it will queue"),
    field("areaID", Kind::Ref("AreaTable"), "AreaTable.dbc id of the dungeon it is for"),
];

const FLAG_STAND: [DataField; 8] = [
    field("lockId", LOCK, LOCK_ABOUT),
    field("pickupSpell", SPELL, "the spell cast on whoever takes the flag"),
    field("radius", NUMBER, "yards within which it can be taken"),
    field("returnAura", SPELL, "the aura that marks the flag as away"),
    field("returnSpell", SPELL, "the spell that returns it"),
    field("noDamageImmune", SWITCH, NO_DAMAGE),
    field("openTextID", NUMBER, OPEN_TEXT),
    field("losOK", SWITCH, LOS_OK),
];

const FISHING_HOLE: [DataField; 5] = [
    field("radius", NUMBER, "yards within which a bobber counts as in the pool"),
    field("lootId", Kind::Ref("gameobject_loot_template"), "gameobject_loot_template entry: what the pool holds"),
    field("minSuccessOpens", NUMBER, "least catches before the pool is spent"),
    field("maxSuccessOpens", NUMBER, "most catches before the pool is spent"),
    field("lockId", LOCK, "Lock.dbc id, keyed on Fishing"),
];

const FLAG_DROP: [DataField; 5] = [
    field("lockId", LOCK, LOCK_ABOUT),
    field("eventID", NUMBER, EVENT),
    field("pickupSpell", SPELL, "the spell cast on whoever picks the flag up"),
    field("noDamageImmune", SWITCH, NO_DAMAGE),
    field("openTextID", NUMBER, OPEN_TEXT),
];

const MINI_GAME: [DataField; 1] = [field("gameType", NUMBER, "the mini game type")];

const CAPTURE_POINT: [DataField; 20] = [
    field("radius", NUMBER, "yards within which a player counts"),
    field("spell", SPELL, "not read by the server"),
    field("worldState1", NUMBER, "world state that shows the slider"),
    field("worldstate2", NUMBER, "world state holding the slider's position"),
    field("winEventID1", NUMBER, "event run when the Alliance wins it"),
    field("winEventID2", NUMBER, "event run when the Horde wins it"),
    field("contestedEventID1", NUMBER, "event run when the Alliance contests it"),
    field("contestedEventID2", NUMBER, "event run when the Horde contests it"),
    field("progressEventID1", NUMBER, "event run when the Alliance takes the lead"),
    field("progressEventID2", NUMBER, "event run when the Horde takes the lead"),
    field("neutralEventID1", NUMBER, "event run when it goes neutral from the Alliance"),
    field("neutralEventID2", NUMBER, "event run when it goes neutral from the Horde"),
    field("neutralPercent", NUMBER, "width of the neutral band, in percent"),
    field("worldstate3", NUMBER, "world state holding the neutral band's width"),
    field("minSuperiority", NUMBER, "the smallest player advantage that moves the slider"),
    field("maxSuperiority", NUMBER, "the player advantage at which the slider moves fastest"),
    field("minTime", Kind::Seconds, "seconds to capture at the greatest superiority"),
    field("maxTime", Kind::Seconds, "seconds to capture at the least superiority"),
    field("large", SWITCH, LARGE),
    field("highlight", SWITCH, "glows under the pointer"),
];

const AURA_GENERATOR: [DataField; 6] = [
    field("startOpen", SWITCH, "whether it starts switched on"),
    field("radius", NUMBER, "yards it reaches"),
    field("auraID1", SPELL, "the first aura it gives"),
    field("conditionID1", NUMBER, "conditions id a player must meet for the first aura"),
    field("auraID2", SPELL, "the second aura it gives"),
    field("conditionID2", NUMBER, "conditions id a player must meet for the second aura"),
];

/// What a type's `data` columns are, in column order: `data_fields(3)[0]`
/// is a chest's `data0`.
///
/// `GameObjectInfo`'s union (`GameObjectDefines.h:220`), one struct per type.
/// A type with no struct (a binder, a map object, a duel arbiter, a mailbox, a
/// lottery kiosk) reads none of its data columns and answers an empty slice.
pub fn data_fields(kind: u32) -> &'static [DataField] {
    match kind {
        0 => &DOOR,
        1 => &BUTTON,
        2 => &QUESTGIVER,
        3 => &CHEST,
        5 => &GENERIC,
        6 => &TRAP_FIELDS,
        7 => &CHAIR,
        8 => &SPELL_FOCUS,
        9 => &TEXT,
        10 => &GOOBER,
        11 => &TRANSPORT,
        12 => &AREA_DAMAGE,
        13 => &CAMERA,
        15 => &MO_TRANSPORT,
        17 => &FISHING_NODE,
        18 => &RITUAL,
        20 => &AUCTION_HOUSE,
        21 => &GUARD_POST,
        22 => &SPELL_CASTER,
        23 => &MEETING_STONE,
        24 => &FLAG_STAND,
        25 => &FISHING_HOLE,
        26 => &FLAG_DROP,
        27 => &MINI_GAME,
        29 => &CAPTURE_POINT,
        30 => &AURA_GENERATOR,
        _ => &[],
    }
}

/// One `data` column of one type, or `None` for a column the type does not
/// read.
pub fn data_field(kind: u32, index: usize) -> Option<&'static DataField> {
    data_fields(kind).get(index)
}

/// Which `data` column of a type holds its lock, or `None` for a type that
/// has none.
///
/// A lock is `data0` on nine types, `data1` on a door and a button, and `data4`
/// on a fishing hole, so the index is looked up rather than assumed.
pub fn lock_column(kind: u32) -> Option<usize> {
    data_fields(kind).iter().position(|field| field.kind == LOCK)
}

/// `GAMEOBJECT_TYPE_FISHINGHOLE`: a pool, whose catch is a loot set of its own.
pub const TYPE_FISHINGHOLE: u32 = 25;

/// Which `data` column names a type's `gameobject_loot_template` entry,
/// or `None` for a type the server takes no loot from.
///
/// `GameObjectInfo::GetLootId` (`GameObjectDefines.h:670`) answers for two
/// types only, a chest's `lootId` and a fishing hole's, and both are
/// `data1`. A fishing node (type 17, the bobber) also carries a `lootId`
/// column and the server never reads it: a bobber's catch is
/// `fishing_loot_template` by zone.
pub fn loot_column(kind: u32) -> Option<usize> {
    match kind {
        TYPE_CHEST | TYPE_FISHINGHOLE => Some(CHEST_LOOT),
        _ => None,
    }
}

/// `gameobject`'s 19 columns, in the order the table declares them, which
/// is also the order `GameObject::SaveToDB` writes them in
/// (`GameObject.cpp:926`).
pub const SPAWN_COLUMNS: [Column; 19] = [
    Column { name: "guid", kind: Kind::Key, group: Group::Identity, about: "the spawn's guid" },
    Column { name: "id", kind: Kind::Ref(TEMPLATE), group: Group::Identity, about: "the gameobject_template entry spawned here" },
    Column { name: "map", kind: Kind::Ref("Map"), group: Group::Place, about: "Map.dbc id of the map it spawns on" },
    Column { name: "position_x", kind: Kind::Float, group: Group::Place, about: "x position: north, in world coordinates" },
    Column { name: "position_y", kind: Kind::Float, group: Group::Place, about: "y position: west, in world coordinates" },
    Column { name: "position_z", kind: Kind::Float, group: Group::Place, about: "z position: height, in world coordinates" },
    Column { name: "orientation", kind: Kind::Float, group: Group::Place, about: "radians it faces, anticlockwise from north; editing it also writes rotation2 and rotation3" },
    Column { name: "rotation0", kind: Kind::Float, group: Group::Place, about: "the rotation quaternion's x: a tilt, 0 for an upright object" },
    Column { name: "rotation1", kind: Kind::Float, group: Group::Place, about: "the rotation quaternion's y: a tilt, 0 for an upright object" },
    Column { name: "rotation2", kind: Kind::Float, group: Group::Place, about: "the rotation quaternion's z: sin(orientation / 2) for an upright object" },
    Column { name: "rotation3", kind: Kind::Float, group: Group::Place, about: "the rotation quaternion's w: cos(orientation / 2) for an upright object" },
    Column { name: "spawntimesecsmin", kind: Kind::Signed, group: Group::Respawn, about: "the least seconds before it respawns; negative: not spawned until a script or event spawns it" },
    Column { name: "spawntimesecsmax", kind: Kind::Signed, group: Group::Respawn, about: "the most seconds before it respawns" },
    Column { name: "animprogress", kind: Kind::Unsigned, group: Group::Respawn, about: "animation progress, 0 to 255; .gobject add writes 100" },
    Column { name: "state", kind: Kind::Choice(&STATES), group: Group::Respawn, about: "the state the client draws, such as a door open or closed" },
    Column { name: "spawn_flags", kind: Kind::Flags(&SPAWN_FLAGS), group: Group::Respawn, about: "flags for this spawn only" },
    Column { name: "visibility_mod", kind: Kind::Float, group: Group::Respawn, about: "yards added to the distance it is seen from" },
    Column { name: "patch_min", kind: Kind::Unsigned, group: Group::Respawn, about: "lowest content patch this spawn exists in" },
    Column { name: "patch_max", kind: Kind::Unsigned, group: Group::Respawn, about: "highest content patch this spawn exists in" },
];

/// The `rotation2` and `rotation3` of an upright object facing that way:
/// `sin(o / 2)` and `cos(o / 2)`, which is what
/// `GameObject::UpdateRotationFields` derives and `.gobject turn` saves.
pub fn rotation_for(orientation: f32) -> (f32, f32) {
    ((orientation / 2.0).sin(), (orientation / 2.0).cos())
}

/// A spawn's facing, as the three columns that state it.
///
/// `orientation` with the quaternion's `z` and `w` beside it, so the two the
/// client is sent agree. For an upright object only; see the module comment
/// and [`is_tilted`], which a caller asks first.
pub fn facing(orientation: f32) -> [Assignment; 3] {
    let (z, w) = rotation_for(orientation);
    [
        Assignment { column: "orientation", value: crate::sql::float(orientation) },
        Assignment { column: "rotation2", value: crate::sql::float(z) },
        Assignment { column: "rotation3", value: crate::sql::float(w) },
    ]
}

/// Whether a rotation leans off the vertical, so that its quaternion is not
/// a function of the facing.
pub fn is_tilted(rotation0: f32, rotation1: f32) -> bool {
    rotation0.abs() > 1e-4 || rotation1.abs() > 1e-4
}

/// Every game-object spawn on one map, with the columns a marker, a model
/// and a heading need from whichever template row the server would load.
///
/// The winning patch per entry is a derived table, for
/// [`crate::creature::spawns_on_map_query`]'s measured reason.
pub fn spawns_on_map_query(map: u32, wow_patch: u32) -> String {
    format!(
        "SELECT g.`guid`, g.`id`, g.`map`, g.`position_x`, g.`position_y`, g.`position_z`, \
         g.`orientation`, g.`rotation0`, g.`rotation1`, g.`state`, g.`spawn_flags`, \
         t.`name`, t.`type`, t.`displayId`, t.`size`, t.`faction`, t.`flags`, t.`data0`, \
         t.`data1`, t.`data4`, t.`patch` \
         FROM `gameobject` g \
         LEFT JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM `gameobject_template` \
                    WHERE `patch` <= {wow_patch} GROUP BY `entry`) w ON w.`entry` = g.`id` \
         LEFT JOIN `gameobject_template` t ON t.`entry` = g.`id` AND t.`patch` = w.`patch` \
         WHERE g.`map` = {map} ORDER BY g.`guid`"
    )
}

/// The template row the server would load for one entry, with its `patch`.
pub fn winning_template_query(entry: u32, wow_patch: u32) -> String {
    format!(
        "SELECT * FROM `gameobject_template` t1 WHERE `entry` = {entry} AND `patch` = \
         (SELECT MAX(t2.`patch`) FROM `gameobject_template` t2 \
          WHERE t2.`entry` = t1.`entry` AND t2.`patch` <= {wow_patch})"
    )
}

/// One template row, by both halves of its key.
pub fn template_query(entry: u32, patch: u32) -> String {
    format!("SELECT * FROM `gameobject_template` WHERE `entry` = {entry} AND `patch` = {patch}")
}

/// One spawn, by guid.
pub fn spawn_query(guid: u64) -> String {
    format!("SELECT * FROM `gameobject` WHERE `guid` = {guid}")
}

/// The row a key names, and whether one is there: the two questions an apply
/// asks, which are the same for every keyed table.
pub use crate::creature::{exists_query, row_query};

/// The key an edit to a template row is written under.
pub fn template_key(entry: u32, patch: u32) -> Key {
    Key::two(("entry", entry as u64), ("patch", patch as u64))
}

/// The key an edit to a spawn is written under.
pub fn spawn_key(guid: u64) -> Key {
    Key::one("guid", guid)
}

/// Where a spawn this project creates gets its guid from.
///
/// `gameobject.guid` is the low 24 bits of the object's `ObjectGuid`, as a
/// creature's is: `HIGHGUID_GAMEOBJECT` carries an entry (`ObjectGuid.h:228`),
/// so the counter is `0x00FFFFFF` wide (`ObjectGuid.h:167`) and
/// [`MAX_GUID`] is the ceiling.
///
/// The reference install's highest is 3,998,644, thirteen times the creature
/// table's, so ten million clears it by six million where the creature base
/// clears its own by 9.7. The cost is the same as the creature base's:
/// `ObjectMgr::LoadGuids` reads `MAX(guid) FROM gameobject`
/// (`ObjectMgr.cpp:7674`) and every map's temporary game objects (traps,
/// bobbers, summoned chests) are numbered from there, so one spawn at ten
/// million leaves each map 6.7 million a run.
pub const RESERVED_GUID_BASE: u64 = 10_000_000;

/// The highest guid the server can hold at all.
pub const MAX_GUID: u64 = 0x00FF_FFFF;

/// Where a template this project creates gets its entry from.
///
/// The reference install's highest `gameobject_template.entry` is 182,121.
/// Two million is [`crate::item::RESERVED_ENTRY_BASE`]'s and
/// [`crate::creature::RESERVED_ENTRY_BASE`]'s number, for the same reason:
/// clear of anything upstream will reach and inside what the column holds.
pub const RESERVED_ENTRY_BASE: u32 = 2_000_000;

/// The highest entry the column can hold: `mediumint unsigned`.
pub const MAX_ENTRY: u32 = 0x00FF_FFFF;

/// The highest entry the whole table holds, at any patch. A new template
/// numbered from the rows the server loads alone would collide with a row that
/// exists at a patch the server is not loading.
pub const MAX_ENTRY_QUERY: &str = "SELECT MAX(`entry`) AS `entry` FROM `gameobject_template`";

/// Every column of a new template, as the values a row created here starts
/// from.
///
/// The `entry` and the `patch` are not among them: they are the key, and
/// [`crate::row::insert`] writes a key's own columns first. Every other column
/// is named, because a column an `INSERT` leaves out takes the table's default
/// rather than a value somebody chose.
///
/// The values are the table's own defaults, except for the type.
/// `ObjectMgr::CheckGameObjectTemplate` (`ObjectMgr.cpp:8208`) refuses a
/// `size` of 0 and a type past the enumeration, and checks nothing else that
/// a zero would fail:
///
/// * `type` 5, `GAMEOBJECT_TYPE_GENERIC`: a model standing in the world
///   with no use, which is the type the server itself forces an invalid type
///   to. The default of 0 is a door, whose `data` columns name a lock and an
///   open time.
/// * `displayId` 0. The client draws a mark and no model until one is
///   chosen, as it does for a trap; a model nobody chose would render as a
///   plausible object rather than as a row that still needs one.
/// * `size` 1, `faction` 0 (no faction, which 8,150 of 9,530 rows
///   have), `flags` 0, the 24 `data` columns 0, no gold, and the empty
///   string for `icon` and `script_name`.
pub fn new_template(name: &str) -> Vec<Assignment> {
    let mut out: Vec<(&'static str, String)> = vec![
        ("type", TYPE_GENERIC.to_string()),
        ("displayId", "0".to_string()),
        ("name", crate::sql::text(name)),
        ("size", "1".to_string()),
    ];
    let already: Vec<&str> = out.iter().map(|(column, _)| *column).collect();
    for column in TEMPLATE_COLUMNS.iter().filter(|column| column.editable()) {
        if already.contains(&column.name) {
            continue;
        }
        out.push((
            column.name,
            match column.kind {
                Kind::Text => crate::sql::text(""),
                _ => "0".to_string(),
            },
        ));
    }
    out.into_iter()
        .map(|(column, value)| Assignment { column, value })
        .collect()
}

/// `GAMEOBJECT_TYPE_GENERIC`: a model with no use, and the type a new template
/// starts as. See [`new_template`].
pub const TYPE_GENERIC: u32 = 5;

/// The tables a removed spawn takes with it, and the column each keys it
/// by: `GameObject::DeleteFromDB`'s own list (`GameObject.cpp:1037`).
pub const DEPENDENTS: [(&str, &str); 2] = [
    ("game_event_gameobject", "guid"),
    ("gameobject_battleground", "guid"),
];

/// Every column of the world database that names a game object by its
/// template entry, read off the reference install's `information_schema`.
///
/// `quest_template.ReqCreatureOrGOId` names a game object by the negative of
/// its entry. `spell_script_target.targetEntry` is a game object for type 0 and
/// a creature for the other two.
///
/// Not reached: a template's own `linkedTrapId`, which is a `data` column
/// whose index depends on the type; `gameobject_loot_template.entry`, which is
/// named by `data1` and is not the object's entry; a script's `datalong` where
/// the command takes an object entry; and the C++ scripts.
pub const TEMPLATE_REFERENCES: [crate::row::Reference; 10] = {
    use crate::row::Reference as R;
    [
        R::new(SPAWN, "id"),
        R::new("gameobject_questrelation", "id"),
        R::new("gameobject_involvedrelation", "id"),
        R::new("pool_gameobject_template", "id"),
        R::new("locales_gameobject", "entry"),
        R::only("spell_script_target", "targetEntry", "`type` = 0"),
        R::negated("quest_template", "ReqCreatureOrGOId1"),
        R::negated("quest_template", "ReqCreatureOrGOId2"),
        R::negated("quest_template", "ReqCreatureOrGOId3"),
        R::negated("quest_template", "ReqCreatureOrGOId4"),
    ]
};

/// Every column that names a spawn by its guid: [`DEPENDENTS`], and
/// the four that mention a spawn from a row about something else.
///
/// `gameobject_scripts.id` is a spawn's guid (`ScriptMgr.cpp:1446`).
/// `gameobject_requirement.reqGuid` is a game-object guid while `reqType` is 1,
/// `GOBJ_REQUIRE_ACTIVE_OBJECT`, and a creature's while it is 0.
pub const SPAWN_REFERENCES: [crate::row::Reference; 6] = {
    use crate::row::Reference as R;
    [
        R::new("game_event_gameobject", "guid"),
        R::new("gameobject_battleground", "guid"),
        R::new("pool_gameobject", "guid"),
        R::new("gameobject_scripts", "id"),
        R::new("gameobject_requirement", "guid"),
        R::only("gameobject_requirement", "reqGuid", "`reqType` = 1"),
    ]
};

/// The references a move of a row of `table` takes along.
pub fn references(table: &str) -> &'static [crate::row::Reference] {
    match table {
        TEMPLATE => &TEMPLATE_REFERENCES,
        SPAWN => &SPAWN_REFERENCES,
        _ => &[],
    }
}

/// How long a new spawn takes to come back, in seconds.
///
/// 300, which is the commonest value on the reference install (12,766 of
/// 56,547 rows). The DDL's default is 0, and a chest or a goober with 0 is one
/// `LoadGameobjects` logs at every start: it despawns when it is used and never
/// returns. vmangos' own `.gobject add` writes 25, which is `GameObject`'s
/// in-memory default.
pub const NEW_SPAWN_SECONDS: i32 = 300;

/// Every column of a new spawn, as the values a row created here starts
/// from. The `guid` is the key and is not among them.
///
/// Every other column is named, on [`crate::creature::new_spawn`]'s argument: a
/// column an `INSERT` leaves out takes the table's default, and this table's
/// default `animprogress` of 0 and respawn time of 0 are both wrong for an
/// object placed by hand.
///
/// * `rotation0` and `rotation1` 0, `rotation2` and `rotation3` from the
///   facing: an upright object, as `.gobject add` makes.
/// * `animprogress` 100, `GO_ANIMPROGRESS_DEFAULT`, and `state` 1,
///   `GO_STATE_READY`: the pair `.gobject add` passes to `Create`.
/// * `patch_min` 0 and `patch_max` 10, the DDL's own.
pub fn new_spawn(entry: u32, map: u32, at: (f32, f32, f32), orientation: f32) -> Vec<Assignment> {
    let float = |value: f32| crate::sql::float(value);
    let (z, w) = rotation_for(orientation);
    let columns: Vec<(&'static str, String)> = vec![
        ("id", entry.to_string()),
        ("map", map.to_string()),
        ("position_x", float(at.0)),
        ("position_y", float(at.1)),
        ("position_z", float(at.2)),
        ("orientation", float(orientation)),
        ("rotation0", "0".to_string()),
        ("rotation1", "0".to_string()),
        ("rotation2", float(z)),
        ("rotation3", float(w)),
        ("spawntimesecsmin", NEW_SPAWN_SECONDS.to_string()),
        ("spawntimesecsmax", NEW_SPAWN_SECONDS.to_string()),
        ("animprogress", "100".to_string()),
        ("state", "1".to_string()),
        ("spawn_flags", "0".to_string()),
        ("visibility_mod", "0".to_string()),
        ("patch_min", "0".to_string()),
        ("patch_max", "10".to_string()),
    ];
    columns
        .into_iter()
        .map(|(column, value)| Assignment { column, value })
        .collect()
}

/// The highest guid the `gameobject` table holds.
pub const MAX_SPAWN_GUID_QUERY: &str = "SELECT MAX(`guid`) AS `guid` FROM `gameobject`";

/// The template columns a marker, a model and a heading need, as the `SELECT`
/// two queries share. They are the ones [`spawns_on_map_query`] carries for
/// every spawn.
const BRIEF: &str = "t.`entry`, t.`patch`, t.`name`, t.`type`, t.`displayId`, t.`size`, \
     t.`faction`, t.`flags`, t.`data0`, t.`data1`, t.`data4`";

/// The join that picks the row the server would load.
fn winning_join(wow_patch: u32) -> String {
    format!(
        "FROM `gameobject_template` t \
         JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM `gameobject_template` \
               WHERE `patch` <= {wow_patch} GROUP BY `entry`) w \
           ON w.`entry` = t.`entry` AND w.`patch` = t.`patch`"
    )
}

/// Named templates, by entry. `None` for an empty list, since MySQL refuses
/// an empty `IN ()`.
pub fn templates_query(entries: &[u32], wow_patch: u32) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let list: Vec<String> = entries.iter().map(u32::to_string).collect();
    Some(format!(
        "SELECT {BRIEF} {} WHERE t.`entry` IN ({})",
        winning_join(wow_patch),
        list.join(", ")
    ))
}

/// Game objects whose name or entry matches what was typed, optionally of
/// one type.
///
/// The type filter makes the picker usable for this table: 2,332 of
/// the templates are spell focuses and 2,297 are chairs, and somebody placing
/// an ore vein wants the 671 chests.
pub fn search_query(term: &str, kind: Option<u32>, wow_patch: u32, limit: usize) -> String {
    let like = crate::sql::text(&format!("%{term}%"));
    // 0 is a real game-object entry in no install, so a term that is not a
    // number matches nothing by entry and the `LIKE` is the whole search.
    let exact = term.trim().parse::<u32>().unwrap_or(0);
    let of_type = match kind {
        Some(kind) => format!(" AND t.`type` = {kind}"),
        None => String::new(),
    };
    format!(
        "SELECT {BRIEF} {} WHERE (t.`name` LIKE {like} OR t.`entry` = {exact}){of_type} \
         ORDER BY t.`entry` = {exact} DESC, LENGTH(t.`name`), t.`entry` LIMIT {limit}",
        winning_join(wow_patch)
    )
}

/// The columns of whichever of the two tables, or an empty slice.
pub fn columns_of(table: &str) -> &'static [Column] {
    match table {
        TEMPLATE => &TEMPLATE_COLUMNS,
        SPAWN => &SPAWN_COLUMNS,
        _ => &[],
    }
}

/// One column of one of the two tables, by name.
pub fn column(table: &str, name: &str) -> Option<&'static Column> {
    columns_of(table).iter().find(|column| column.name == name)
}

/// The statements a save emits for one row this project claims: one `UPDATE`
/// for a row it edits, a `DELETE` and an `INSERT` for a row it creates, and
/// three `DELETE`s for a spawn it removes.
///
/// The creation is the pair `GameObject::SaveToDB` writes (`GameObject.cpp:948`)
/// so that applying twice means the same as applying once; it is safe only
/// because the caller has established that the key is this project's. A life
/// the table cannot carry produces no statement; see [`can_live`].
pub fn statements(table: &str, key: &Key, life: Life, changes: &[Assignment]) -> Vec<String> {
    match life {
        Life::Update => crate::row::update(table, key, changes).into_iter().collect(),
        Life::Insert if can_live(table, life) => vec![
            crate::row::delete(table, key),
            match crate::row::insert(table, key, changes) {
                Some(statement) => statement,
                None => return Vec::new(),
            },
        ],
        Life::Delete if can_live(table, life) => delete_statements(key),
        _ => Vec::new(),
    }
}

/// Whether this table can hold a row that is created or removed. Both tables
/// can hold a created row; only the spawn table can hold a removed one. See
/// the module comment.
pub fn can_live(table: &str, life: Life) -> bool {
    match life {
        Life::Update => true,
        Life::Insert => table == SPAWN || table == TEMPLATE,
        Life::Delete => table == SPAWN,
    }
}

/// The statements that take back a row this project created, once its
/// `INSERT` has run: a spawn with the two tables keyed by its guid, a template
/// on its own. A table that cannot hold a created row answers nothing. See
/// [`crate::creature::insert_undo_statements`].
pub fn insert_undo_statements(table: &str, key: &Key) -> Vec<String> {
    match table {
        SPAWN => delete_statements(key),
        TEMPLATE => vec![crate::row::delete(TEMPLATE, key)],
        _ => Vec::new(),
    }
}

/// Removing a spawn, whole: the two dependent tables and then the row, so a
/// run that stops half way leaves the object rather than the orphans.
pub fn delete_statements(key: &Key) -> Vec<String> {
    let Some(guid) = key.first() else {
        return Vec::new();
    };
    let mut out: Vec<String> = DEPENDENTS
        .iter()
        .map(|(table, column)| crate::row::delete(table, &Key::one(column, guid)))
        .collect();
    out.push(crate::row::delete(SPAWN, key));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The template schema is the server's own `SELECT`, in order
    /// (`ObjectMgr.cpp:7852`). A column in the wrong place is the failure a
    /// hand-written schema has.
    #[test]
    fn the_template_columns_are_the_servers_own_select() {
        const SELECTED: &str = "entry type displayId name icon faction flags size data0 data1 \
            data2 data3 data4 data5 data6 data7 data8 data9 data10 data11 data12 data13 \
            data14 data15 data16 data17 data18 data19 data20 data21 data22 data23 mingold \
            maxgold script_name";
        let want: Vec<&str> = SELECTED.split_whitespace().collect();
        let have: Vec<&str> = TEMPLATE_COLUMNS.iter().map(|column| column.name).collect();
        assert_eq!(have, want);
    }

    /// The spawn schema is the table's own column order, which is the
    /// order `GameObject::SaveToDB` writes an unnamed `INSERT` in.
    #[test]
    fn the_spawn_columns_are_the_tables_own_order() {
        const DECLARED: &str = "guid id map position_x position_y position_z orientation \
            rotation0 rotation1 rotation2 rotation3 spawntimesecsmin spawntimesecsmax \
            animprogress state spawn_flags visibility_mod patch_min patch_max";
        let want: Vec<&str> = DECLARED.split_whitespace().collect();
        let have: Vec<&str> = SPAWN_COLUMNS.iter().map(|column| column.name).collect();
        assert_eq!(have, want);
    }

    #[test]
    fn no_column_is_named_twice() {
        for table in TABLES {
            let mut seen = std::collections::HashSet::new();
            for column in columns_of(table) {
                assert!(seen.insert(column.name), "{table} names {} twice", column.name);
                assert!(!column.about.is_empty(), "{} says nothing", column.name);
            }
        }
    }

    /// The data columns are indexed by their own names, in both directions,
    /// and a column that merely starts with the word is not one.
    #[test]
    fn a_data_column_is_found_by_index_and_by_name() {
        for index in 0..DATA_COLUMNS {
            let name = data_column(index).expect("a name");
            assert_eq!(data_index(name), Some(index));
            assert!(column(TEMPLATE, name).is_some(), "{name} is not a column");
        }
        assert_eq!(data_column(DATA_COLUMNS), None);
        assert_eq!(data_index("data24"), None);
        assert_eq!(data_index("database"), None);
        assert_eq!(data_index("name"), None);
    }

    /// No type names more data columns than the table has, and none names a
    /// field twice. The tables are transcribed from a C union of 26 structs,
    /// where a transposed field is not caught by reading.
    #[test]
    fn a_types_fields_fit_the_table_and_do_not_repeat() {
        for named in TYPES {
            let fields = data_fields(named.value);
            assert!(fields.len() <= DATA_COLUMNS, "{} has {} fields", named.name, fields.len());
            let mut seen = std::collections::HashSet::new();
            for field in fields {
                assert!(seen.insert(field.name), "{} names {} twice", named.name, field.name);
                assert!(!field.about.is_empty(), "{}.{} says nothing", named.name, field.name);
            }
        }
        // The structs' own sizes, from `GameObjectDefines.h`.
        assert_eq!(data_fields(0).len(), 6);
        assert_eq!(data_fields(3).len(), 16);
        assert_eq!(data_fields(10).len(), 20);
        assert_eq!(data_fields(29).len(), 20);
        // A type with no struct reads none of them.
        assert!(data_fields(19).is_empty(), "a mailbox");
        assert!(data_fields(99).is_empty());
    }

    /// A chest's lock is `data0` and its loot is `data1`, which is what a
    /// gathering node is read from; a door's lock is `data1` and a fishing
    /// hole's is `data4`.
    #[test]
    fn the_lock_is_where_the_type_puts_it() {
        assert_eq!(lock_column(TYPE_CHEST), Some(CHEST_LOCK));
        assert_eq!(data_fields(TYPE_CHEST)[CHEST_LOOT].name, "lootId");
        assert_eq!(lock_column(0), Some(1), "a door");
        assert_eq!(lock_column(1), Some(1), "a button");
        assert_eq!(lock_column(10), Some(0), "a goober");
        assert_eq!(lock_column(25), Some(4), "a fishing hole");
        assert_eq!(lock_column(7), None, "a chair");
        // The loot is `data1` on the two types `GetLootId` answers for, and
        // is `lootId` by name on both.
        for kind in [TYPE_CHEST, TYPE_FISHINGHOLE] {
            let column = loot_column(kind).expect("a looted type");
            assert_eq!(data_fields(kind)[column].name, "lootId");
        }
        assert_eq!(loot_column(17), None, "a bobber's lootId is not read");
        assert_eq!(loot_column(0), None, "a door");
    }

    /// A new spawn names every column the table has but its key.
    #[test]
    fn a_new_spawn_names_every_column_but_the_key() {
        let row = new_spawn(1731, 0, (-9400.0, -100.0, 60.0), 1.5);
        let named: Vec<&str> = row.iter().map(|change| change.column).collect();
        for column in SPAWN_COLUMNS.iter().filter(|column| column.editable()) {
            assert!(named.contains(&column.name), "{} is not written", column.name);
        }
        assert!(!named.contains(&"guid"));
        assert_eq!(named.len(), SPAWN_COLUMNS.len() - 1);
    }

    /// A new spawn is upright, ready, and comes back. The table's own
    /// defaults are an `animprogress` of 0 and a respawn time of 0, and a chest
    /// with the second is one the server logs at every start.
    #[test]
    fn a_new_spawn_is_upright_ready_and_respawns() {
        let row = new_spawn(1731, 0, (0.0, 0.0, 0.0), std::f32::consts::PI);
        let value = |name: &str| {
            row.iter()
                .find(|change| change.column == name)
                .map(|change| change.value.clone())
                .expect("the column")
        };
        assert_eq!(value("rotation0"), "0");
        assert_eq!(value("rotation1"), "0");
        // Facing pi: sin(pi / 2) is 1 and cos(pi / 2) is 0.
        assert!((value("rotation2").parse::<f32>().expect("a float") - 1.0).abs() < 1e-5);
        assert!(value("rotation3").parse::<f32>().expect("a float").abs() < 1e-5);
        assert_eq!(value("animprogress"), "100");
        assert_eq!(value("state"), "1");
        assert_eq!(value("spawntimesecsmin"), "300");
        assert_eq!(value("patch_max"), "10");
    }

    /// A facing is three columns, and the quaternion's two are a unit pair:
    /// `LoadGameobjects` skips a row whose rotation leaves [-1, 1].
    #[test]
    fn a_facing_writes_the_quaternion_beside_it() {
        for o in [0.0_f32, 0.7, 1.5708, 3.1416, 4.7, 6.2] {
            let (z, w) = rotation_for(o);
            assert!((z * z + w * w - 1.0).abs() < 1e-5, "{o}");
            let written = facing(o);
            assert_eq!(written[0].column, "orientation");
            assert_eq!(written[1].column, "rotation2");
            assert_eq!(written[2].column, "rotation3");
        }
        assert!(!is_tilted(0.0, 0.0));
        assert!(is_tilted(0.0, 0.0436));
    }

    /// Removing a spawn removes the two tables keyed by its guid too, and
    /// the row itself goes last.
    #[test]
    fn removing_a_spawn_takes_its_two_dependents_with_it() {
        let sql = delete_statements(&spawn_key(42));
        assert_eq!(sql.len(), DEPENDENTS.len() + 1);
        assert_eq!(sql[0], "DELETE FROM `game_event_gameobject` WHERE `guid` = 42;");
        assert_eq!(
            sql.last().map(String::as_str),
            Some("DELETE FROM `gameobject` WHERE `guid` = 42;")
        );
    }

    /// A template can be created and not removed; see [`can_live`]. A removal
    /// statement for one would be a change that cannot be made live.
    #[test]
    fn a_template_can_be_created_and_not_removed() {
        let key = template_key(1731, 0);
        assert!(can_live(TEMPLATE, Life::Insert));
        assert!(!can_live(TEMPLATE, Life::Delete));
        assert!(statements(TEMPLATE, &key, Life::Delete, &[]).is_empty());
        for life in [Life::Insert, Life::Delete] {
            assert!(can_live(SPAWN, life));
        }
        let change = [Assignment { column: "size", value: "2".into() }];
        assert_eq!(statements(TEMPLATE, &key, Life::Update, &change).len(), 1);
        let made = template_key(RESERVED_ENTRY_BASE, 10);
        let sql = statements(TEMPLATE, &made, Life::Insert, &new_template("Test Object"));
        assert_eq!(sql.len(), 2);
        assert_eq!(
            sql[0],
            "DELETE FROM `gameobject_template` WHERE `entry` = 2000000 AND `patch` = 10;"
        );
        assert!(sql[1].starts_with("INSERT INTO `gameobject_template` (`entry`, `patch`, "));
        assert!(sql[1].contains("'Test Object'"));
    }

    /// A new template names every column the table has but its key, and is a
    /// generic object of size 1 with no model.
    #[test]
    fn a_new_template_names_every_column_but_the_key() {
        let row = new_template("Test Object");
        let named: Vec<&str> = row.iter().map(|change| change.column).collect();
        for column in TEMPLATE_COLUMNS.iter().filter(|column| column.editable()) {
            assert!(named.contains(&column.name), "{} is not written", column.name);
        }
        assert!(!named.contains(&"entry"));
        assert_eq!(named.len(), TEMPLATE_COLUMNS.len() - 1);
        let value = |name: &str| {
            row.iter()
                .find(|change| change.column == name)
                .map(|change| change.value.as_str())
        };
        assert_eq!(value("type"), Some("5"));
        assert_eq!(value("size"), Some("1"));
        assert_eq!(value("displayId"), Some("0"));
        assert_eq!(value("name"), Some("'Test Object'"));
        assert_eq!(value("icon"), Some("''"));
        assert_eq!(value("data23"), Some("0"));
        assert_eq!(value("script_name"), Some("''"));
    }

    /// A created template is undone by one `DELETE` of its key, a created
    /// spawn by the three statements a removal comes to.
    #[test]
    fn a_created_rows_undo_depends_on_its_table() {
        let template = insert_undo_statements(TEMPLATE, &template_key(2_000_000, 10));
        assert_eq!(
            template,
            vec!["DELETE FROM `gameobject_template` WHERE `entry` = 2000000 AND `patch` = 10;"]
        );
        assert_eq!(insert_undo_statements(SPAWN, &spawn_key(42)).len(), DEPENDENTS.len() + 1);
        assert!(insert_undo_statements("npc_vendor", &Key::one("entry", 1)).is_empty());
    }

    /// The reserved entry base is inside the column and above the reference
    /// install's highest entry, 182,121.
    #[test]
    fn a_reserved_entry_is_one_the_column_can_hold() {
        assert!(RESERVED_ENTRY_BASE < MAX_ENTRY);
        assert!(RESERVED_ENTRY_BASE > 182_121);
        assert_eq!(RESERVED_ENTRY_BASE, crate::creature::RESERVED_ENTRY_BASE);
    }

    /// The reserved base clears the table's own highest, 3,998,644 on the
    /// reference install and thirteen times the creature table's, and is inside
    /// the 24 bits a game-object guid has.
    #[test]
    fn a_reserved_guid_is_one_the_server_can_hold() {
        assert!(RESERVED_GUID_BASE < MAX_GUID);
        assert!(RESERVED_GUID_BASE > 3_998_644 + 1_000_000);
    }

    /// The search takes a name or an entry, narrows to a type when asked, and
    /// escapes the name once.
    #[test]
    fn the_search_matches_a_name_or_an_entry_within_a_type() {
        let sql = search_query("vein", Some(TYPE_CHEST), 10, 50);
        assert!(sql.contains("LIKE '%vein%'"), "{sql}");
        assert!(sql.contains("t.`type` = 3"), "{sql}");
        assert!(sql.contains("`patch` <= 10"), "{sql}");
        let sql = search_query("1731", None, 10, 50);
        assert!(sql.contains("t.`entry` = 1731"), "{sql}");
        assert!(!sql.contains("t.`type` ="), "{sql}");
        assert!(search_query("x'; DROP TABLE `gameobject`; --", None, 10, 50)
            .contains("'%x\\'; DROP TABLE `gameobject`; --%'"));
    }

    /// A LEFT join: a spawn whose template row is missing is a spawn worth
    /// seeing, and an inner join would drop it.
    #[test]
    fn the_spawn_query_joins_the_template_the_server_would_load() {
        let sql = spawns_on_map_query(1, 10);
        assert!(sql.contains("MAX(`patch`)"), "{sql}");
        assert!(sql.contains("g.`map` = 1"), "{sql}");
        assert!(sql.contains("LEFT JOIN"), "{sql}");
    }

    /// A quest names a game object by the negative of its entry, so a moved
    /// template's reference writes the negative too.
    #[test]
    fn a_quest_objective_follows_a_moved_template_negated() {
        let reference = TEMPLATE_REFERENCES
            .iter()
            .find(|reference| reference.column == "ReqCreatureOrGOId1")
            .expect("the reference");
        assert_eq!(
            reference.follow(1731, 2_000_000),
            "UPDATE `quest_template` SET `ReqCreatureOrGOId1` = -2000000 \
             WHERE `ReqCreatureOrGOId1` = -1731;"
        );
    }

    #[test]
    fn a_table_name_from_a_file_resolves_to_a_constant_or_to_nothing() {
        assert_eq!(table_named("gameobject_template"), Some(TEMPLATE));
        assert_eq!(table_named("gameobject"), Some(SPAWN));
        assert_eq!(table_named("creature"), None);
        assert_eq!(table_named(""), None);
    }
}
