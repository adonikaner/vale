//! The eleven `*_scripts` tables: one schema under eleven names, and what a
//! script is to the server.
//!
//! ## One schema, eleven tables
//!
//! `ScriptMgr::LoadScripts` (`ScriptMgr.cpp:86`) reads every script table with
//! one `SELECT` of the same 20 columns, `ORDER BY id, delay, priority`, and
//! `CreatureEventAIMgr` reads `creature_ai_scripts` the same way. What differs
//! is what an `id` is named by:
//!
//! ```text
//! creature_ai_scripts         creature_ai_events.action1_script..3
//! creature_movement_scripts   creature_movement(_template).script_id
//! creature_spells_scripts     creature_spells.scriptId_1..8
//! gossip_scripts              gossip_menu.script_id, gossip_menu_option.action_script_id
//! generic_scripts             SCRIPT_COMMAND_START_SCRIPT and its relatives
//! event_scripts               a spell effect's or a game object's event id
//! quest_start_scripts         quest_template.StartScript
//! quest_end_scripts           quest_template.CompleteScript
//! spell_scripts               a spell id
//! gameobject_scripts          a gameobject spawn's guid
//! areatrigger_scripts         an AreaTrigger.dbc id
//! ```
//!
//! ## A script is written whole
//!
//! None of the tables has a primary key. A row is identified by nothing but
//! its content, and four of its columns are floats, so no `WHERE` clause this
//! crate could write is safe to address one row with. A script is therefore
//! the unit of edit, as a waypoint path is (see [`crate::path`]): every row
//! under an `id` is deleted and the rows the project holds are inserted, in
//! `(delay, priority)` order. The undo is the rows that were there, restored
//! the same way. [`Scripts`] is the store, on [`crate::path::Paths`]' shape.
//!
//! ## What a row is
//!
//! `command` picks one of 94 actions ([`COMMANDS`], `ScriptCommands.h:37`),
//! and the meaning of `datalong` to `datalong4`, `dataint` to `dataint4` and
//! the four coordinates depends on it. [`Command`] names each column the
//! command reads, in the header's own words, so a form draws `spell_id`
//! rather than `datalong`. `target_type` picks who the command acts on
//! ([`TARGETS`], `ScriptCommands.h:1140`), with `target_param1` and
//! `target_param2` narrowing the search for the types that search.
//! `data_flags` is [`DATA_FLAGS`], and `condition_id` a row of `conditions`.
//!
//! `delay` is seconds after the script starts: `Map::ScriptsStart`
//! (`Map.cpp:2541`) adds it to the game clock, which counts seconds.
//! `creature_ai_scripts` is the exception. An event runs every row of its
//! script at once, and the loader logs a row with a delay as unsupported
//! (`ScriptMgr.cpp:1631`); [`waits`] says which tables take one. An event
//! that waits starts a `generic_scripts` script, whose rows do.
//!
//! ## Live on a reload
//!
//! `.reload creature_ai_events` re-reads `creature_ai_scripts` first and says
//! so; `gossip_scripts`, `generic_scripts`, `event_scripts`,
//! `quest_start_scripts` and `creature_spells_scripts` have reloads under
//! their own names; `creature_movement_scripts`, `quest_end_scripts`,
//! `spell_scripts`, `gameobject_scripts` and `areatrigger_scripts` are read at
//! start (`Chat.cpp:804` and after). [`Table::reload`] carries which.

use crate::row::Key;

pub use crate::schema::{mask_words, value_word, Bit, Column, Group, Kind, Row, RowValue, Value};

pub const CREATURE_AI: &str = "creature_ai_scripts";
pub const CREATURE_MOVEMENT: &str = "creature_movement_scripts";
pub const CREATURE_SPELLS: &str = "creature_spells_scripts";
pub const GOSSIP: &str = "gossip_scripts";
pub const GENERIC: &str = "generic_scripts";
pub const EVENT: &str = "event_scripts";
pub const QUEST_START: &str = "quest_start_scripts";
pub const QUEST_END: &str = "quest_end_scripts";
pub const SPELL: &str = "spell_scripts";
pub const GAMEOBJECT: &str = "gameobject_scripts";
pub const AREATRIGGER: &str = "areatrigger_scripts";

/// Every table this module writes.
pub const TABLES: [&str; 11] = [
    CREATURE_AI,
    CREATURE_MOVEMENT,
    CREATURE_SPELLS,
    GOSSIP,
    GENERIC,
    EVENT,
    QUEST_START,
    QUEST_END,
    SPELL,
    GAMEOBJECT,
    AREATRIGGER,
];

/// The static name for a table read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// What one of the eleven tables is: its name, what an `id` of it is named
/// by, and the reload that makes an edit live, if one exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Table {
    pub name: &'static str,
    /// The column, in the server's own words, whose value names a script.
    pub keyed_by: &'static str,
    /// The `.reload` command that re-reads it, or `None` for a table read at
    /// start only.
    pub reload: Option<&'static str>,
}

/// The eleven, in [`TABLES`]' order.
pub const ALL: [Table; 11] = [
    Table { name: CREATURE_AI, keyed_by: "creature_ai_events.action1_script, action2_script, action3_script", reload: Some("creature_ai_events") },
    Table { name: CREATURE_MOVEMENT, keyed_by: "creature_movement.script_id", reload: None },
    Table { name: CREATURE_SPELLS, keyed_by: "creature_spells.scriptId_1 to scriptId_8", reload: Some("creature_spells_scripts") },
    Table { name: GOSSIP, keyed_by: "gossip_menu.script_id, gossip_menu_option.action_script_id", reload: Some("gossip_scripts") },
    Table { name: GENERIC, keyed_by: "a START_SCRIPT command's generic_script_id", reload: Some("generic_scripts") },
    Table { name: EVENT, keyed_by: "a spell effect's or a game object's event id", reload: Some("event_scripts") },
    Table { name: QUEST_START, keyed_by: "quest_template.StartScript", reload: Some("quest_start_scripts") },
    Table { name: QUEST_END, keyed_by: "quest_template.CompleteScript", reload: None },
    Table { name: SPELL, keyed_by: "a spell id", reload: None },
    Table { name: GAMEOBJECT, keyed_by: "a gameobject spawn's guid", reload: None },
    Table { name: AREATRIGGER, keyed_by: "an AreaTrigger.dbc id", reload: None },
];

/// One of the eleven, by name.
pub fn table(name: &str) -> Option<Table> {
    ALL.into_iter().find(|table| table.name == name)
}

/// A yes-or-no column.
pub const NO_YES: [Value; 2] = [Value { value: 0, name: "No" }, Value { value: 1, name: "Yes" }];

/// `ChatType`, from `CreatureDefines.h:471`: what `SCRIPT_COMMAND_TALK`'s
/// `datalong` picks.
pub const CHAT_TYPES: [Value; 8] = [
    Value { value: 0, name: "Say" },
    Value { value: 1, name: "Yell" },
    Value { value: 2, name: "Text emote" },
    Value { value: 3, name: "Boss emote" },
    Value { value: 4, name: "Whisper" },
    Value { value: 5, name: "Boss whisper" },
    Value { value: 6, name: "Zone yell" },
    Value { value: 7, name: "Zone emote" },
];

/// `ReactStates`, from `UnitDefines.h:682`.
pub const REACT_STATES: [Value; 3] = [
    Value { value: 0, name: "Passive" },
    Value { value: 1, name: "Defensive" },
    Value { value: 2, name: "Aggressive" },
];

/// `SheathState`, from `UnitDefines.h:170`.
pub const SHEATH_STATES: [Value; 3] = [
    Value { value: 0, name: "Unarmed" },
    Value { value: 1, name: "Melee" },
    Value { value: 2, name: "Ranged" },
];

/// `eMoveToCoordinateTypes`, from `ScriptCommands.h:361`.
pub const MOVE_TO_COORDINATES: [Value; 4] = [
    Value { value: 0, name: "As given" },
    Value { value: 1, name: "Relative to the target" },
    Value { value: 2, name: "x is the distance from the target" },
    Value { value: 3, name: "Random point within o of the coordinates" },
];

/// `TempSummonType`, from `ObjectDefines.h:61`.
pub const SUMMON_DESPAWN: [Value; 10] = [
    Value { value: 1, name: "After the time out of combat, or when it disappears" },
    Value { value: 2, name: "After the time out of combat, or when it dies" },
    Value { value: 3, name: "After the time" },
    Value { value: 4, name: "After the time once out of combat" },
    Value { value: 5, name: "At death" },
    Value { value: 6, name: "After the time from death" },
    Value { value: 7, name: "When it disappears" },
    Value { value: 8, name: "When unsummoned" },
    Value { value: 9, name: "After the time in or out of combat, or when it disappears" },
    Value { value: 10, name: "After the time in or out of combat, or when it dies" },
];

/// `CastFlags`, from `ScriptCommands.h:388`: `SCRIPT_COMMAND_CAST_SPELL`'s
/// `datalong2`, and `creature_spells.castFlags_n`.
pub const CAST_FLAGS: [Bit; 9] = [
    Bit { bit: 0x001, name: "Interrupt the previous cast", about: "" },
    Bit { bit: 0x002, name: "Triggered", about: "no cast time, no cost, no interruption" },
    Bit { bit: 0x004, name: "Force the cast", about: "bypasses the checks in Creature::TryToCast" },
    Bit { bit: 0x008, name: "Main ranged spell", about: "the creature does not chase its target until the cast fails" },
    Bit { bit: 0x010, name: "Only when the target is unreachable", about: "" },
    Bit { bit: 0x020, name: "Only when the target lacks the aura", about: "" },
    Bit { bit: 0x040, name: "Only in melee range", about: "" },
    Bit { bit: 0x080, name: "Only out of melee range", about: "" },
    Bit { bit: 0x100, name: "Only while the target is casting", about: "" },
];

/// `data_flags`: the general flags every command reads, `ScriptCommands.h:519`.
pub const DATA_FLAGS: [Bit; 5] = [
    Bit { bit: 0x01, name: "Swap the initial source and target", about: "before the buddy is assigned" },
    Bit { bit: 0x02, name: "Swap the final source and target", about: "after the buddy is assigned" },
    Bit { bit: 0x04, name: "Target self", about: "the final target is replaced with the final source" },
    Bit { bit: 0x08, name: "Abort the script on failure", about: "" },
    Bit { bit: 0x10, name: "Skip when the target is missing", about: "the command is skipped rather than failing" },
];

/// One of the thirty things a command can act on: `ScriptTarget`,
/// `ScriptCommands.h:1140`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub value: u32,
    pub name: &'static str,
    /// What `target_param1` and `target_param2` mean for it, or empty when
    /// it reads neither.
    pub param1: &'static str,
    pub param2: &'static str,
}

/// The thirty, in the enum's order.
pub const TARGETS: [Target; 30] = [
    Target { value: 0, name: "The provided target", param1: "", param2: "" },
    Target { value: 1, name: "Current victim", param1: "", param2: "" },
    Target { value: 2, name: "Second on the threat list", param1: "", param2: "" },
    Target { value: 3, name: "Last on the threat list", param1: "", param2: "" },
    Target { value: 4, name: "Random on the threat list", param1: "", param2: "" },
    Target { value: 5, name: "Random on the threat list, not the top", param1: "", param2: "" },
    Target { value: 6, name: "Nearest hostile on the threat list", param1: "", param2: "" },
    Target { value: 7, name: "Farthest hostile on the threat list", param1: "", param2: "" },
    Target { value: 8, name: "Owner, or self", param1: "", param2: "" },
    Target { value: 9, name: "Owner", param1: "", param2: "" },
    Target { value: 10, name: "Nearest creature with entry", param1: "creature entry", param2: "search radius" },
    Target { value: 11, name: "Creature with guid", param1: "creature guid", param2: "" },
    Target { value: 12, name: "Creature from instance data", param1: "instance data field", param2: "" },
    Target { value: 13, name: "Nearest game object with entry", param1: "game object entry", param2: "search radius" },
    Target { value: 14, name: "Game object with guid", param1: "game object guid", param2: "" },
    Target { value: 15, name: "Game object from instance data", param1: "instance data field", param2: "" },
    Target { value: 16, name: "Random friendly unit", param1: "search radius", param2: "exclude self (1) or not (0)" },
    Target { value: 17, name: "Friendly unit missing the most health", param1: "search radius", param2: "least health missing" },
    Target { value: 18, name: "Friendly unit missing the most health, not the provided target", param1: "search radius", param2: "least health missing" },
    Target { value: 19, name: "Friendly unit without an aura", param1: "search radius", param2: "spell id" },
    Target { value: 20, name: "Friendly unit without an aura, not the provided target", param1: "search radius", param2: "spell id" },
    Target { value: 21, name: "Friendly unit under crowd control", param1: "search radius", param2: "dispel type" },
    Target { value: 22, name: "The map event's source", param1: "event id", param2: "" },
    Target { value: 23, name: "The map event's target", param1: "event id", param2: "" },
    Target { value: 24, name: "The map event's extra target", param1: "event id", param2: "target index" },
    Target { value: 25, name: "Nearest player", param1: "search radius", param2: "" },
    Target { value: 26, name: "Nearest hostile player", param1: "search radius", param2: "" },
    Target { value: 27, name: "Nearest friendly player", param1: "search radius", param2: "" },
    Target { value: 28, name: "Random creature with entry, not self", param1: "creature entry", param2: "search radius" },
    Target { value: 29, name: "Random game object with entry", param1: "game object entry", param2: "search radius" },
];

/// [`TARGETS`] as an enumeration, for a form's menu.
pub const TARGET_VALUES: [Value; 30] = {
    let mut out = [Value { value: 0, name: "" }; 30];
    let mut at = 0;
    while at < 30 {
        out[at] = Value { value: TARGETS[at].value, name: TARGETS[at].name };
        at += 1;
    }
    out
};

/// One target, by value.
pub fn target(value: u32) -> Option<&'static Target> {
    TARGETS.iter().find(|target| target.value == value)
}

/// Whether a table's rows may wait: `delay` is honoured in every script table
/// but `creature_ai_scripts`, whose rows run at once. See the module comment.
pub fn waits(table: &str) -> bool {
    table != CREATURE_AI
}

/// A target as the object of a sentence: `the current victim`, `the provided
/// target`, `target type 40` for one this list does not name.
pub fn target_phrase(value: u32) -> String {
    match target(value) {
        Some(target) => {
            let name = target.name.to_lowercase();
            match name.starts_with("the ") {
                true => name,
                false => format!("the {name}"),
            }
        }
        None => format!("target type {value}"),
    }
}

/// One column a command reads: what it is called in `ScriptCommands.h`, and
/// what a form draws it as.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub about: &'static str,
}

const fn p(name: &'static str, kind: Kind, about: &'static str) -> Option<Param> {
    Some(Param { name, kind, about })
}

const fn n(name: &'static str, about: &'static str) -> Option<Param> {
    Some(Param { name, kind: Kind::Unsigned, about })
}

const fn yes(name: &'static str, about: &'static str) -> Option<Param> {
    Some(Param { name, kind: Kind::Choice(&NO_YES), about })
}

const NONE: [Option<Param>; 4] = [None, None, None, None];

/// One of the 94 commands: what it does, what it acts on, and which of the
/// eight data columns and four coordinates it reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Command {
    pub command: u32,
    pub name: &'static str,
    /// What the command runs on, in the header's words.
    pub source: &'static str,
    /// What `datalong` to `datalong4` mean, or `None` for one not read.
    pub datalong: [Option<Param>; 4],
    /// What `dataint` to `dataint4` mean, or `None` for one not read.
    pub dataint: [Option<Param>; 4],
    /// Whether `x`, `y`, `z` and `o` are read.
    pub coords: bool,
    /// Whether the target matters to it.
    pub targets: bool,
}

const fn c(
    command: u32,
    name: &'static str,
    source: &'static str,
    datalong: [Option<Param>; 4],
    dataint: [Option<Param>; 4],
    coords: bool,
    targets: bool,
) -> Command {
    Command { command, name, source, datalong, dataint, coords, targets }
}

/// The 94 commands, in `ScriptCommands.h:37`'s order. A command not in the
/// table is drawn by its number with every column offered.
pub const COMMANDS: [Command; 94] = [
    c(0, "Talk", "WorldObject", [p("chat_type", Kind::Choice(&CHAT_TYPES), "how it is said"), None, None, None], [p("broadcast_text", Kind::Ref("broadcast_text"), "what is said"), p("broadcast_text 2", Kind::Ref("broadcast_text"), "a second text, chosen at random"), p("broadcast_text 3", Kind::Ref("broadcast_text"), "a third"), p("broadcast_text 4", Kind::Ref("broadcast_text"), "a fourth")], false, true),
    c(1, "Emote", "Unit", [p("emote", Kind::Ref("Emotes"), "Emotes.dbc id"), p("emote 2", Kind::Ref("Emotes"), "a second, chosen at random"), p("emote 3", Kind::Ref("Emotes"), "a third"), p("emote 4", Kind::Ref("Emotes"), "a fourth")], [yes("is_targeted", "face the target while emoting"), None, None, None], false, true),
    c(2, "Set field", "Object", [n("field_id", "an update field index"), n("value", ""), None, None], NONE, false, false),
    c(3, "Move to", "Creature", [p("coordinates_type", Kind::Choice(&MOVE_TO_COORDINATES), "how x, y, z and o are read"), n("time", "milliseconds to take, or 0 for the creature's speed"), n("movement_options", "MoveOptions"), n("flags", "eMoveToFlags: 1 forced, 2 point movement generator")], [n("path_id", "creature_movement_special path"), None, None, None], true, true),
    c(4, "Modify flags", "Object", [n("field_id", "an update field index"), n("bitmask", ""), n("options", "eModifyFlagsOptions: 0 set, 1 remove, 2 toggle"), None], NONE, false, false),
    c(5, "Interrupt casts", "Unit", [yes("with_delayed", ""), p("spell_id", Kind::Ref("Spell"), "one spell, or 0 for any"), None, None], NONE, false, false),
    c(6, "Teleport to", "Unit", [p("map_id", Kind::Ref("Map"), "read for a player only"), n("teleport_options", "TeleportToOptions"), None, None], NONE, true, false),
    c(7, "Quest explored", "Player", [p("quest_id", Kind::Ref("quest_template"), ""), n("distance", "yards from the target, or 0"), yes("group", "credit the group"), None], NONE, false, true),
    c(8, "Kill credit", "Player", [p("creature_entry", Kind::Ref("creature_template"), ""), yes("group", "0 personal, 1 group"), None, None], NONE, false, true),
    c(9, "Respawn game object", "Map", [n("db_guid", "gameobject spawn guid"), n("despawn_delay", "seconds"), None, None], NONE, false, true),
    c(10, "Summon creature", "WorldObject", [p("creature_entry", Kind::Ref("creature_template"), ""), n("despawn_delay", "milliseconds"), n("unique_limit", "at most this many of it nearby"), n("unique_distance", "yards the limit is counted in")], [n("flags", "eSummonCreatureFlags: 1 run, 2 active, 4 unique, 8 unique temp, 16 null AI"), p("generic_script_id", Kind::Ref(GENERIC), "run on the summon"), p("attack_target", Kind::Choice(&TARGET_VALUES), "who it attacks"), p("despawn_type", Kind::Choice(&SUMMON_DESPAWN), "")], true, true),
    c(11, "Open door", "GameObject", [n("db_guid", "gameobject spawn guid, or 0 for the source"), n("reset_delay", "seconds"), None, None], NONE, false, true),
    c(12, "Close door", "GameObject", [n("db_guid", "gameobject spawn guid, or 0 for the source"), n("reset_delay", "seconds"), None, None], NONE, false, true),
    c(13, "Activate object", "GameObject", NONE, NONE, false, true),
    c(14, "Remove aura", "Unit", [p("spell_id", Kind::Ref("Spell"), ""), None, None, None], NONE, false, false),
    c(15, "Cast spell", "Unit", [p("spell_id", Kind::Ref("Spell"), ""), p("cast_flags", Kind::Flags(&CAST_FLAGS), ""), None, None], NONE, false, true),
    c(16, "Play sound", "WorldObject", [n("sound_id", "SoundEntries.dbc id"), n("flags", "1 only to the target, 2 distance dependent, 4 to the whole zone"), None, None], NONE, false, true),
    c(17, "Create item", "Player", [p("item_id", Kind::Ref("item_template"), ""), n("amount", ""), None, None], NONE, false, true),
    c(18, "Despawn creature", "Creature", [n("despawn_delay", "milliseconds"), n("respawn_delay", "seconds"), None, None], NONE, false, false),
    c(19, "Set equipment", "Creature", [yes("reset_default", "back to the template's equipment"), None, None, None], [p("main hand", Kind::Ref("item_template"), ""), p("off hand", Kind::Ref("item_template"), ""), p("ranged", Kind::Ref("item_template"), ""), None], false, false),
    c(20, "Movement", "Creature", [n("movement_type", "MovementGeneratorType: 0 idle, 1 random, 2 waypoint, 3 confused, 4 chase, 5 home, 6 flight, 7 point, 8 fleeing, 9 distract, 10 assistance, 11 timed flee, 12 follow, 13 effect, 14 patrol, 15 charge, 16 distancing"), n("bool_param", "meaning depends on the type"), n("int_param", "meaning depends on the type"), yes("clear", "clear the movement stack first")], NONE, true, true),
    c(21, "Set active object", "Creature", [yes("active", "updated whether or not a player is near"), None, None, None], NONE, false, false),
    c(22, "Set faction", "Creature", [p("faction_id", Kind::Ref("FactionTemplate"), "0 restores the template's"), n("flags", "TemporaryFactionFlags"), None, None], NONE, false, false),
    c(23, "Morph to entry or model", "Unit", [n("creature_id or display_id", "which one datalong2 says"), yes("is_display_id", ""), None, None], NONE, false, false),
    c(24, "Mount to entry or model", "Creature", [n("creature_id or display_id", "which one datalong2 says"), yes("is_display_id", ""), yes("permanent", ""), None], NONE, false, false),
    c(25, "Set run", "Creature", [yes("run", ""), None, None, None], NONE, false, false),
    c(26, "Attack start", "Creature", NONE, NONE, false, true),
    c(27, "Update entry", "Creature", [p("creature_entry", Kind::Ref("creature_template"), ""), None, None, None], NONE, false, false),
    c(28, "Stand state", "Unit", [n("stand_state", "UnitStandStateType: 0 stand, 1 sit, 2 sit chair, 3 sleep, 4 sit low chair, 5 sit medium chair, 6 sit high chair, 7 dead, 8 kneel"), None, None, None], NONE, false, false),
    c(29, "Modify threat", "Creature", [n("targets", "eModifyThreatTargets, or a ScriptTarget"), None, None, None], NONE, true, true),
    c(30, "Send taxi path", "Player", [p("taxi_path_id", Kind::Ref("TaxiPath"), ""), None, None, None], NONE, false, false),
    c(31, "Terminate script", "Any", [p("creature_entry", Kind::Ref("creature_template"), "0 for no search"), n("search_distance", ""), n("options", "eTerminateScriptOptions: 0 if found, 1 if not found"), None], NONE, false, false),
    c(32, "Terminate on condition", "Any", [n("condition_id", "a row of conditions"), p("failed_quest_id", Kind::Ref("quest_template"), "failed when the script ends here"), n("flags", "1 terminate when the condition is false"), None], NONE, false, false),
    c(33, "Enter evade mode", "Creature", NONE, NONE, false, false),
    c(34, "Set home position", "Creature", [n("options", "eSetHomePositionOptions: 0 the coordinates, 1 the current position, 2 the default position, 3 the target's position"), None, None, None], NONE, true, true),
    c(35, "Turn to", "Unit", [n("options", "eTurnToFacingOptions: 0 the target, 1 the o given"), None, None, None], NONE, true, true),
    c(36, "Meeting stone", "Player", [p("area_id", Kind::Ref("AreaTable"), ""), None, None, None], NONE, false, false),
    c(37, "Set instance data", "Map", [n("field", ""), n("data", ""), n("options", "eSetInstDataOptions: 0 set, 1 increment, 2 decrement"), None], NONE, false, false),
    c(38, "Set instance data 64", "Map", [n("field", ""), n("data", ""), n("options", "eSetInstData64Options: 0 the data, 1 the target's guid"), None], NONE, false, true),
    c(39, "Start script", "Map", [p("generic_script_id", Kind::Ref(GENERIC), ""), p("generic_script_id 2", Kind::Ref(GENERIC), ""), p("generic_script_id 3", Kind::Ref(GENERIC), ""), p("generic_script_id 4", Kind::Ref(GENERIC), "")], [n("chance", "of the first, in percent"), n("chance 2", ""), n("chance 3", ""), n("chance 4", "the four together at most 100")], false, true),
    c(40, "Remove item", "Player", [p("item_id", Kind::Ref("item_template"), ""), n("amount", ""), None, None], NONE, false, true),
    c(41, "Remove object", "GameObject or Creature", NONE, NONE, false, false),
    c(42, "Set melee attack", "Creature", [yes("on", ""), None, None, None], NONE, false, false),
    c(43, "Set combat movement", "Creature", [yes("on", ""), None, None, None], NONE, false, false),
    c(44, "Set phase", "Creature", [n("phase", "0 to 31"), n("options", "eSetPhaseOptions: 0 set, 1 increment, 2 decrement"), None, None], NONE, false, false),
    c(45, "Set random phase", "Creature", [n("phase", ""), n("phase 2", ""), n("phase 3", ""), n("phase 4", "one of the four, at random")], NONE, false, false),
    c(46, "Set phase in range", "Creature", [n("phase_min", ""), n("phase_max", ""), None, None], NONE, false, false),
    c(47, "Flee", "Creature", [yes("seek_assistance", ""), None, None, None], NONE, false, false),
    c(48, "Deal damage", "Unit", [n("damage", ""), yes("is_percent", "of the target's health"), None, None], NONE, false, true),
    c(49, "Zone combat pulse", "Creature", [yes("initial_pulse", ""), None, None, None], NONE, false, false),
    c(50, "Call for help", "Creature", NONE, NONE, true, false),
    c(51, "Set sheath", "Unit", [p("sheath", Kind::Choice(&SHEATH_STATES), ""), None, None, None], NONE, false, false),
    c(52, "Invincibility", "Creature", [n("health", "the health it cannot drop below; 0 turns it off"), yes("is_percent", ""), None, None], NONE, false, false),
    c(53, "Game event", "None", [n("event_id", "game_event entry"), yes("start", "1 start, 0 stop"), yes("overwrite", ""), None], NONE, false, false),
    c(54, "Set server variable", "None", [n("index", ""), n("value", ""), None, None], NONE, false, false),
    c(55, "Creature spells", "Creature", [p("creature_spells id", Kind::Ref("creature_spells"), ""), p("creature_spells id 2", Kind::Ref("creature_spells"), ""), p("creature_spells id 3", Kind::Ref("creature_spells"), ""), p("creature_spells id 4", Kind::Ref("creature_spells"), "")], [n("chance", "of the first, in percent"), n("chance 2", ""), n("chance 3", ""), n("chance 4", "the four together at most 100")], false, false),
    c(56, "Remove guardians", "Unit", [p("creature_id", Kind::Ref("creature_template"), "0 for all"), None, None, None], NONE, false, false),
    c(57, "Add spell cooldown", "Unit", [p("spell_id", Kind::Ref("Spell"), ""), n("cooldown", "seconds"), None, None], NONE, false, false),
    c(58, "Remove spell cooldown", "Unit", [p("spell_id", Kind::Ref("Spell"), "0 for all"), None, None, None], NONE, false, false),
    c(59, "Set react state", "Creature", [p("react_state", Kind::Choice(&REACT_STATES), ""), None, None, None], NONE, false, false),
    c(60, "Start waypoints", "Creature", [n("waypoints_source", "0 creature_movement, 1 creature_movement_template, 2 creature_movement_special"), n("start_point", ""), n("initial_delay", "milliseconds"), yes("repeat", "")], [n("overwrite_guid", "read the path of this spawn instead"), p("overwrite_entry", Kind::Ref("creature_template"), "read the template path of this creature instead"), None, None], false, false),
    c(61, "Start map event", "Map", [n("event_id", ""), n("time_limit", "milliseconds"), None, None], [n("success_condition", ""), p("success_script", Kind::Ref(GENERIC), ""), n("failure_condition", ""), p("failure_script", Kind::Ref(GENERIC), "")], false, false),
    c(62, "End map event", "Map", [n("event_id", ""), yes("success", ""), None, None], NONE, false, false),
    c(63, "Add map event target", "Map", [n("event_id", ""), None, None, None], [n("success_condition", ""), p("success_script", Kind::Ref(GENERIC), ""), n("failure_condition", ""), p("failure_script", Kind::Ref(GENERIC), "")], false, true),
    c(64, "Remove map event target", "Map", [n("event_id", ""), n("condition_id", ""), n("options", "eRemoveMapEventTargetOptions"), None], NONE, false, true),
    c(65, "Set map event data", "Map", [n("event_id", ""), n("index", ""), n("data", ""), n("options", "eSetMapScriptDataOptions: 0 set, 1 increment, 2 decrement")], NONE, false, false),
    c(66, "Send map event", "Map", [n("event_id", ""), n("data", ""), n("options", "eSendMapEventOptions"), None], NONE, false, false),
    c(67, "Set default movement", "Creature", [n("movement_type", "0 idle, 1 random, 2 waypoint"), yes("always_replace", ""), n("param1", "wander distance for random"), None], NONE, false, false),
    c(68, "Start script for all", "WorldObject", [p("generic_script_id", Kind::Ref(GENERIC), ""), n("options", "eStartScriptForAllOptions: 0 game objects, 1 creatures, 2 players, 3 units, 4 world objects"), n("object_entry", "or 0 for any"), n("search_radius", "")], NONE, false, false),
    c(69, "Edit map event", "Map", [n("event_id", ""), None, None, None], [n("success_condition", ""), p("success_script", Kind::Ref(GENERIC), ""), n("failure_condition", ""), p("failure_script", Kind::Ref(GENERIC), "")], false, false),
    c(70, "Fail quest", "Player", [p("quest_id", Kind::Ref("quest_template"), ""), None, None, None], NONE, false, false),
    c(71, "Respawn creature", "Creature", [yes("even_if_alive", ""), None, None, None], NONE, false, false),
    c(72, "Assist unit", "Creature", NONE, NONE, false, true),
    c(73, "Combat stop", "Unit", NONE, NONE, false, false),
    c(74, "Add aura", "Unit", [p("spell_id", Kind::Ref("Spell"), ""), n("flags", ""), None, None], NONE, false, false),
    c(75, "Add threat", "Creature", NONE, NONE, false, true),
    c(76, "Summon object", "WorldObject", [p("gameobject_entry", Kind::Ref("gameobject_template"), ""), n("respawn_time", "seconds"), None, None], NONE, true, false),
    c(77, "Set fly", "Unit", [yes("on", ""), None, None, None], NONE, false, false),
    c(78, "Join creature group", "Creature", [n("flags", "creature_groups OptionFlags"), None, None, None], NONE, true, true),
    c(79, "Leave creature group", "Creature", NONE, NONE, false, false),
    c(80, "Set game object state", "GameObject", [n("state", "GOState: 0 active, 1 ready, 2 active alternative"), None, None, None], NONE, false, false),
    c(81, "Despawn game object", "GameObject", [n("db_guid", "gameobject spawn guid, or 0 for the source"), n("despawn_delay", "seconds"), None, None], NONE, false, true),
    c(82, "Load game object spawn", "Map", [n("db_guid", "gameobject spawn guid"), None, None, None], NONE, false, false),
    c(83, "Quest credit", "Player", NONE, NONE, false, true),
    c(84, "Set gossip menu", "Creature", [p("gossip_menu_id", Kind::Ref("gossip_menu"), ""), None, None, None], NONE, false, false),
    c(85, "Send script event", "Creature", [n("event_id", "EVENT_T_SCRIPT's param1"), n("event_data", "EVENT_T_SCRIPT's param2"), None, None], NONE, false, true),
    c(86, "Set PvP", "Player", [yes("on", ""), None, None, None], NONE, false, false),
    c(87, "Reset door or button", "GameObject", NONE, NONE, false, false),
    c(88, "Set command state", "Creature", [n("command_state", "CommandStates: 0 stay, 1 follow, 2 attack, 3 dismiss"), None, None, None], NONE, false, false),
    c(89, "Play custom animation", "GameObject", [n("anim_id", ""), None, None, None], NONE, false, false),
    c(90, "Start script on group", "Unit", [p("generic_script_id", Kind::Ref(GENERIC), ""), p("generic_script_id 2", Kind::Ref(GENERIC), ""), p("generic_script_id 3", Kind::Ref(GENERIC), ""), p("generic_script_id 4", Kind::Ref(GENERIC), "")], [n("chance", "of the first, in percent"), n("chance 2", ""), n("chance 3", ""), n("chance 4", "the four together at most 100")], false, false),
    c(91, "Load creature spawn", "Map", [n("db_guid", "creature spawn guid"), yes("with_group", ""), None, None], NONE, false, false),
    c(92, "Start script on zone", "Map", [p("generic_script_id", Kind::Ref(GENERIC), ""), p("zone_id", Kind::Ref("AreaTable"), ""), yes("with_pets", ""), None], NONE, false, false),
    c(93, "Follow escort", "Creature", [yes("do_follow", ""), None, None, None], NONE, false, true),
];

/// [`COMMANDS`] as an enumeration, for a form's menu.
pub const COMMAND_VALUES: [Value; 94] = {
    let mut out = [Value { value: 0, name: "" }; 94];
    let mut at = 0;
    while at < 94 {
        out[at] = Value { value: COMMANDS[at].command, name: COMMANDS[at].name };
        at += 1;
    }
    out
};

/// One command, by number.
pub fn command(value: u32) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.command == value)
}

/// The twenty columns `LoadScripts` selects, plus `priority` (in its `ORDER
/// BY`) and `comments` (never read by the server, kept for the person).
pub const COLUMNS: [Column; 22] = [
    Column { name: "id", kind: Kind::Key, group: Group::Identity, about: "which script the row belongs to" },
    Column { name: "delay", kind: Kind::Seconds, group: Group::Identity, about: "seconds after the script starts; 0 in creature_ai_scripts" },
    Column { name: "priority", kind: Kind::Unsigned, group: Group::Identity, about: "order among rows with one delay" },
    Column { name: "command", kind: Kind::Choice(&COMMAND_VALUES), group: Group::Behaviour, about: "what the row does" },
    Column { name: "datalong", kind: Kind::Unsigned, group: Group::Behaviour, about: "named by the command" },
    Column { name: "datalong2", kind: Kind::Unsigned, group: Group::Behaviour, about: "named by the command" },
    Column { name: "datalong3", kind: Kind::Unsigned, group: Group::Behaviour, about: "named by the command" },
    Column { name: "datalong4", kind: Kind::Unsigned, group: Group::Behaviour, about: "named by the command" },
    Column { name: "target_param1", kind: Kind::Unsigned, group: Group::Behaviour, about: "named by the target type" },
    Column { name: "target_param2", kind: Kind::Unsigned, group: Group::Behaviour, about: "named by the target type" },
    Column { name: "target_type", kind: Kind::Choice(&TARGET_VALUES), group: Group::Behaviour, about: "who the command acts on" },
    Column { name: "data_flags", kind: Kind::Flags(&DATA_FLAGS), group: Group::Behaviour, about: "how source and target are swapped, and what a failure does" },
    Column { name: "dataint", kind: Kind::Signed, group: Group::Behaviour, about: "named by the command" },
    Column { name: "dataint2", kind: Kind::Signed, group: Group::Behaviour, about: "named by the command" },
    Column { name: "dataint3", kind: Kind::Signed, group: Group::Behaviour, about: "named by the command" },
    Column { name: "dataint4", kind: Kind::Signed, group: Group::Behaviour, about: "named by the command" },
    Column { name: "x", kind: Kind::Float, group: Group::Place, about: "read by the commands that move or place" },
    Column { name: "y", kind: Kind::Float, group: Group::Place, about: "" },
    Column { name: "z", kind: Kind::Float, group: Group::Place, about: "" },
    Column { name: "o", kind: Kind::Float, group: Group::Place, about: "" },
    Column { name: "condition_id", kind: Kind::Unsigned, group: Group::Advanced, about: "a row of conditions the row is skipped without, or 0" },
    Column { name: "comments", kind: Kind::Text, group: Group::Advanced, about: "for the person; the server never reads it" },
];

/// One row of a script: the columns as the server reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptRow {
    pub delay: u32,
    pub priority: u32,
    pub command: u32,
    pub datalong: [u32; 4],
    pub target_param1: u32,
    pub target_param2: u32,
    pub target_type: u32,
    pub data_flags: u32,
    pub dataint: [i32; 4],
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub o: f32,
    pub condition_id: u32,
    pub comments: String,
}

impl ScriptRow {
    /// A row of one command with nothing else set.
    pub fn new(command: u32) -> ScriptRow {
        ScriptRow {
            delay: 0,
            priority: 0,
            command,
            datalong: [0; 4],
            target_param1: 0,
            target_param2: 0,
            target_type: 0,
            data_flags: 0,
            dataint: [0; 4],
            x: 0.0,
            y: 0.0,
            z: 0.0,
            o: 0.0,
            condition_id: 0,
            comments: String::new(),
        }
    }

    /// One row as the database answered it, or `None` for a row with no
    /// command.
    pub fn from_row(row: &Row) -> Option<ScriptRow> {
        let count = |name: &str| row.integer(name).unwrap_or(0).max(0) as u32;
        let int = |name: &str| row.integer(name).unwrap_or(0) as i32;
        let float = |name: &str| row.number(name).unwrap_or(0.0) as f32;
        Some(ScriptRow {
            delay: count("delay"),
            priority: count("priority"),
            command: row.integer("command")? as u32,
            datalong: [count("datalong"), count("datalong2"), count("datalong3"), count("datalong4")],
            target_param1: count("target_param1"),
            target_param2: count("target_param2"),
            target_type: count("target_type"),
            data_flags: count("data_flags"),
            dataint: [int("dataint"), int("dataint2"), int("dataint3"), int("dataint4")],
            x: float("x"),
            y: float("y"),
            z: float("z"),
            o: float("o"),
            condition_id: count("condition_id"),
            comments: row.text("comments").unwrap_or_default().to_string(),
        })
    }

    /// Every column but `id`, as `(name, literal)` in [`COLUMNS`]' order.
    pub fn values(&self) -> Vec<(&'static str, String)> {
        vec![
            ("delay", self.delay.to_string()),
            ("priority", self.priority.to_string()),
            ("command", self.command.to_string()),
            ("datalong", self.datalong[0].to_string()),
            ("datalong2", self.datalong[1].to_string()),
            ("datalong3", self.datalong[2].to_string()),
            ("datalong4", self.datalong[3].to_string()),
            ("target_param1", self.target_param1.to_string()),
            ("target_param2", self.target_param2.to_string()),
            ("target_type", self.target_type.to_string()),
            ("data_flags", self.data_flags.to_string()),
            ("dataint", self.dataint[0].to_string()),
            ("dataint2", self.dataint[1].to_string()),
            ("dataint3", self.dataint[2].to_string()),
            ("dataint4", self.dataint[3].to_string()),
            ("x", crate::sql::float(self.x)),
            ("y", crate::sql::float(self.y)),
            ("z", crate::sql::float(self.z)),
            ("o", crate::sql::float(self.o)),
            ("condition_id", self.condition_id.to_string()),
            ("comments", crate::sql::text(&self.comments)),
        ]
    }

    /// One column's value as the form shows it: a number, or the comment.
    pub fn get(&self, column: &str) -> String {
        match column {
            "delay" => self.delay.to_string(),
            "priority" => self.priority.to_string(),
            "command" => self.command.to_string(),
            "datalong" => self.datalong[0].to_string(),
            "datalong2" => self.datalong[1].to_string(),
            "datalong3" => self.datalong[2].to_string(),
            "datalong4" => self.datalong[3].to_string(),
            "target_param1" => self.target_param1.to_string(),
            "target_param2" => self.target_param2.to_string(),
            "target_type" => self.target_type.to_string(),
            "data_flags" => self.data_flags.to_string(),
            "dataint" => self.dataint[0].to_string(),
            "dataint2" => self.dataint[1].to_string(),
            "dataint3" => self.dataint[2].to_string(),
            "dataint4" => self.dataint[3].to_string(),
            "x" => crate::sql::float(self.x),
            "y" => crate::sql::float(self.y),
            "z" => crate::sql::float(self.z),
            "o" => crate::sql::float(self.o),
            "condition_id" => self.condition_id.to_string(),
            "comments" => self.comments.clone(),
            _ => String::new(),
        }
    }

    /// Set one column from the text a form typed. `false` for a column this
    /// row does not have or a value that does not read.
    pub fn set(&mut self, column: &str, value: &str) -> bool {
        let value = value.trim();
        let count = || value.parse::<i64>().ok().map(|n| n.max(0) as u32);
        let int = || value.parse::<i64>().ok().map(|n| n as i32);
        let float = || value.parse::<f32>().ok();
        match column {
            "delay" => self.delay = match count() { Some(n) => n, None => return false },
            "priority" => self.priority = match count() { Some(n) => n, None => return false },
            "command" => self.command = match count() { Some(n) => n, None => return false },
            "datalong" => self.datalong[0] = match count() { Some(n) => n, None => return false },
            "datalong2" => self.datalong[1] = match count() { Some(n) => n, None => return false },
            "datalong3" => self.datalong[2] = match count() { Some(n) => n, None => return false },
            "datalong4" => self.datalong[3] = match count() { Some(n) => n, None => return false },
            "target_param1" => self.target_param1 = match count() { Some(n) => n, None => return false },
            "target_param2" => self.target_param2 = match count() { Some(n) => n, None => return false },
            "target_type" => self.target_type = match count() { Some(n) => n, None => return false },
            "data_flags" => self.data_flags = match count() { Some(n) => n, None => return false },
            "dataint" => self.dataint[0] = match int() { Some(n) => n, None => return false },
            "dataint2" => self.dataint[1] = match int() { Some(n) => n, None => return false },
            "dataint3" => self.dataint[2] = match int() { Some(n) => n, None => return false },
            "dataint4" => self.dataint[3] = match int() { Some(n) => n, None => return false },
            "x" => self.x = match float() { Some(n) => n, None => return false },
            "y" => self.y = match float() { Some(n) => n, None => return false },
            "z" => self.z = match float() { Some(n) => n, None => return false },
            "o" => self.o = match float() { Some(n) => n, None => return false },
            "condition_id" => self.condition_id = match count() { Some(n) => n, None => return false },
            "comments" => self.comments = value.to_string(),
            _ => return false,
        }
        true
    }

    /// The row in one line, for a list: the command's name and the columns
    /// it reads.
    pub fn summary(&self) -> String {
        let Some(command) = command(self.command) else {
            return format!("command {}", self.command);
        };
        let mut parts: Vec<String> = Vec::new();
        for (at, param) in command.datalong.iter().enumerate() {
            if let Some(param) = param {
                let value = self.datalong[at];
                if value != 0 || at == 0 {
                    parts.push(format!("{} {}", param.name, named(param.kind, value)));
                }
            }
        }
        for (at, param) in command.dataint.iter().enumerate() {
            if let Some(param) = param {
                let value = self.dataint[at];
                if value != 0 || at == 0 {
                    let shown = match value < 0 {
                        true => value.to_string(),
                        false => named(param.kind, value as u32),
                    };
                    parts.push(format!("{} {shown}", param.name));
                }
            }
        }
        if command.coords && (self.x != 0.0 || self.y != 0.0 || self.z != 0.0) {
            parts.push(format!(
                "at {} {} {}",
                crate::sql::float(self.x),
                crate::sql::float(self.y),
                crate::sql::float(self.z)
            ));
        }
        match parts.is_empty() {
            true => command.name.to_string(),
            false => format!("{}: {}", command.name, parts.join(", ")),
        }
    }

    /// The row as a sentence: `Say: "Stand fast!"`, `Cast Shoot on the
    /// current victim`, `Set phase to 2`. `names` resolves the spells,
    /// creatures, emotes and texts a column names; `broadcast_text` is the
    /// table a Talk row's text is looked up under. A command with no sentence
    /// of its own lists the columns it reads, named, as [`ScriptRow::summary`]
    /// does.
    pub fn sentence(&self, names: crate::schema::Names<'_>) -> String {
        let d = self.datalong;
        let named = |table: &str, id: u32| match id {
            0 => "nothing".to_string(),
            id => names(table, id).unwrap_or_else(|| format!("{} {id}", crate::schema::ref_word(table))),
        };
        let on_off = |value: u32| match value {
            0 => "off",
            _ => "on",
        };
        let body = match self.command {
            0 => {
                let how = value_word(&CHAT_TYPES, d[0]);
                let texts: Vec<u32> = self.dataint.iter().filter(|id| **id > 0).map(|id| *id as u32).collect();
                let quoted = |id: u32| match names("broadcast_text", id) {
                    Some(text) => format!("\u{201c}{text}\u{201d}"),
                    None => format!("text {id}"),
                };
                match texts.as_slice() {
                    [] => format!("{how}: no text"),
                    [one] => format!("{how}: {}", quoted(*one)),
                    [first, ..] => format!("{how} one of {} texts: {}, \u{2026}", texts.len(), quoted(*first)),
                }
            }
            1 => format!("Emote {}", named("Emotes", d[0])),
            3 => format!(
                "Move to {}, {}, {}",
                crate::sql::float(self.x),
                crate::sql::float(self.y),
                crate::sql::float(self.z)
            ),
            10 => match d[1] {
                0 => format!("Summon {}", named("creature_template", d[0])),
                despawn => format!(
                    "Summon {} for {}",
                    named("creature_template", d[0]),
                    crate::schema::span_words(u64::from(despawn))
                ),
            },
            14 => format!("Remove {}", named("Spell", d[0])),
            15 => match d[1] {
                0 => format!("Cast {}", named("Spell", d[0])),
                flags => format!("Cast {} ({})", named("Spell", d[0]), mask_words(&CAST_FLAGS, flags).to_lowercase()),
            },
            18 => match d[0] {
                0 => "Despawn".to_string(),
                delay => format!("Despawn after {}", crate::schema::span_words(u64::from(delay))),
            },
            20 => format!(
                "Movement: {}",
                MOVEMENT_TYPES.get(d[0] as usize).copied().unwrap_or("unknown type")
            ),
            22 => match d[0] {
                0 => "Restore the template's faction".to_string(),
                faction => format!("Set faction to {}", named("FactionTemplate", faction)),
            },
            25 => match d[0] {
                0 => "Walk".to_string(),
                _ => "Run".to_string(),
            },
            26 => "Attack".to_string(),
            33 => "Evade".to_string(),
            39 => {
                let ids: Vec<String> = d.iter().filter(|id| **id != 0).map(|id| id.to_string()).collect();
                match ids.len() {
                    0 => "Start no generic script".to_string(),
                    1 => format!("Start generic script {}", ids[0]),
                    _ => format!("Start one of generic scripts {}", ids.join(", ")),
                }
            }
            42 => format!("Melee attack {}", on_off(d[0])),
            43 => format!("Combat movement {}", on_off(d[0])),
            44 => match d[1] {
                1 => format!("Raise the phase by {}", d[0]),
                2 => format!("Lower the phase by {}", d[0]),
                _ => format!("Set phase to {}", d[0]),
            },
            45 => {
                let phases: Vec<String> = d.iter().map(|phase| phase.to_string()).collect();
                format!("Set phase to one of {}", phases.join(", "))
            }
            46 => format!("Set phase to one of {} to {}", d[0], d[1]),
            47 => match d[0] {
                0 => "Flee".to_string(),
                _ => "Flee for help".to_string(),
            },
            48 => format!(
                "Deal {}{} damage",
                d[0],
                match d[1] {
                    0 => "",
                    _ => "%",
                }
            ),
            50 => "Call for help".to_string(),
            52 => match (d[0], d[1]) {
                (0, _) => "Invincibility off".to_string(),
                (health, 0) => format!("Cannot drop below {health} health"),
                (health, _) => format!("Cannot drop below {health}% health"),
            },
            71 => "Respawn".to_string(),
            73 => "Stop combat".to_string(),
            74 => format!("Add {}", named("Spell", d[0])),
            _ => self.described(names),
        };
        match self.target_words(names) {
            Some(target) => format!("{body} on {target}"),
            None => body,
        }
    }

    /// Who the row acts on, when the command reads a target and the target is
    /// not the one the script was started with: `the current victim`,
    /// `the nearest Defias Thug within 30 yards`.
    fn target_words(&self, names: crate::schema::Names<'_>) -> Option<String> {
        let command = command(self.command)?;
        if !command.targets || self.target_type == 0 {
            return None;
        }
        let target = target(self.target_type)?;
        let (a, b) = (self.target_param1, self.target_param2);
        let entry = |table: &str| names(table, a).unwrap_or_else(|| format!("{} {a}", crate::schema::ref_word(table)));
        let within = match b {
            0 => String::new(),
            yards => format!(" within {yards} yards"),
        };
        Some(match self.target_type {
            10 => format!("the nearest {}{within}", entry("creature_template")),
            13 => format!("the nearest {}{within}", entry("gameobject_template")),
            28 => format!("a random {}{within}", entry("creature_template")),
            29 => format!("a random {}{within}", entry("gameobject_template")),
            _ => {
                let phrase = target_phrase(self.target_type);
                match (target.param1.is_empty(), target.param2.is_empty()) {
                    (true, _) => phrase,
                    (false, true) => format!("{phrase} ({} {a})", target.param1),
                    (false, false) => format!("{phrase} ({} {a}, {} {b})", target.param1, target.param2),
                }
            }
        })
    }

    /// [`ScriptRow::summary`] with the references named.
    fn described(&self, names: crate::schema::Names<'_>) -> String {
        let Some(command) = command(self.command) else {
            return format!("command {}", self.command);
        };
        let word = |kind: Kind, value: u32| match kind {
            Kind::Ref(table) if value != 0 => names(table, value).unwrap_or_else(|| value.to_string()),
            kind => named(kind, value),
        };
        let mut parts: Vec<String> = Vec::new();
        for (at, param) in command.datalong.iter().enumerate() {
            if let Some(param) = param {
                let value = self.datalong[at];
                if value != 0 || at == 0 {
                    parts.push(format!("{} {}", param.name, word(param.kind, value)));
                }
            }
        }
        for (at, param) in command.dataint.iter().enumerate() {
            if let Some(param) = param {
                let value = self.dataint[at];
                if value != 0 {
                    let shown = match value < 0 {
                        true => value.to_string(),
                        false => word(param.kind, value as u32),
                    };
                    parts.push(format!("{} {shown}", param.name));
                }
            }
        }
        match parts.is_empty() {
            true => command.name.to_string(),
            false => format!("{}: {}", command.name, parts.join(", ")),
        }
    }

    /// The row as one line of text, for the undo stack: the 21 values
    /// separated by tabs, the comment last so that it may hold anything but
    /// a tab or a line break.
    pub fn to_line(&self) -> String {
        let mut out: Vec<String> = self
            .values()
            .into_iter()
            .map(|(_, value)| value)
            .take(20)
            .collect();
        out.push(self.comments.replace(['\t', '\n', '\r'], " "));
        out.join("\t")
    }

    /// A row read from a line [`ScriptRow::to_line`] wrote. `None` for a line
    /// that does not read.
    pub fn from_line(line: &str) -> Option<ScriptRow> {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 20 {
            return None;
        }
        let mut row = ScriptRow::new(0);
        let names: Vec<&'static str> = COLUMNS.iter().skip(1).map(|column| column.name).collect();
        for (at, name) in names.iter().enumerate() {
            let value = match *name {
                "comments" => fields.get(at).copied().unwrap_or(""),
                _ => fields.get(at).copied()?,
            };
            if !row.set(name, value) {
                return None;
            }
        }
        Some(row)
    }
}

/// What a parameter's value is called, for a summary: the enumeration's word
/// where the kind has one, the number otherwise.
fn named(kind: Kind, value: u32) -> String {
    match kind {
        Kind::Choice(values) => value_word(values, value),
        _ => value.to_string(),
    }
}

/// `MovementGeneratorType` in words, by value: what the Movement command's
/// `movement_type` picks.
const MOVEMENT_TYPES: [&str; 17] = [
    "idle",
    "random",
    "waypoint",
    "confused",
    "chase",
    "home",
    "flight",
    "point",
    "fleeing",
    "distract",
    "assistance",
    "timed flee",
    "follow",
    "effect",
    "patrol",
    "charge",
    "distancing",
];

/// Every row under one id of one table, in the order the server runs them.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub table: &'static str,
    pub id: u32,
    pub rows: Vec<ScriptRow>,
}

impl Script {
    /// A script with no rows, which written is a script removed.
    pub fn empty(table: &'static str, id: u32) -> Script {
        Script { table, id, rows: Vec::new() }
    }

    /// The key an edit is recorded under: the id alone.
    pub fn key(&self) -> Key {
        Key::one("id", u64::from(self.id))
    }

    /// The rows in `(delay, priority)` order, which is the server's.
    pub fn sorted(&self) -> Vec<ScriptRow> {
        let mut rows = self.rows.clone();
        rows.sort_by_key(|row| (row.delay, row.priority));
        rows
    }

    /// The rows as one line of text, for the undo stack: the rows'
    /// [`ScriptRow::to_line`]s separated by U+001E.
    pub fn to_line(&self) -> String {
        self.rows.iter().map(ScriptRow::to_line).collect::<Vec<_>>().join("\u{1e}")
    }

    /// A script read from a line [`Script::to_line`] wrote. A row that does
    /// not read is dropped rather than failing the script.
    pub fn from_line(table: &'static str, id: u32, line: &str) -> Script {
        let rows = line
            .split('\u{1e}')
            .filter(|part| !part.is_empty())
            .filter_map(ScriptRow::from_line)
            .collect();
        Script { table, id, rows }
    }

    // --- steps ---------------------------------------------------------
    //
    // A step is one row of [`Script::sorted`], and `at` is its index there.
    // The server orders rows by `(delay, priority)` and nothing else, so an
    // order is changed by changing those two columns. Each operation below
    // keeps the delays non-decreasing in step order and then settles the
    // priorities (see `settle`), so the server's order is the order shown.

    /// How long the server waits before step `at`, after the step before it
    /// or after the start for the first: its delay less the previous delay.
    pub fn wait_before(&self, at: usize) -> u32 {
        let rows = self.sorted();
        let Some(row) = rows.get(at) else { return 0 };
        match at {
            0 => row.delay,
            _ => row.delay.saturating_sub(rows[at - 1].delay),
        }
    }

    /// The script with the wait before step `at` set to `wait` seconds.
    /// Step `at` and every step after it move by the same amount, so the
    /// waits between the later steps are unchanged.
    pub fn with_wait(&self, at: usize, wait: u32) -> Script {
        let mut rows = self.sorted();
        if at >= rows.len() {
            return self.clone();
        }
        let shift = i64::from(wait) - i64::from(self.wait_before(at));
        for row in rows.iter_mut().skip(at) {
            row.delay = (i64::from(row.delay) + shift).max(0) as u32;
        }
        self.with_rows(rows)
    }

    /// The script with step `at` swapped with the step before it (`up`) or
    /// after it. The two steps exchange everything but their delays, so the
    /// timeline keeps its waits and the two actions trade places on it.
    pub fn with_step_moved(&self, at: usize, up: bool) -> Script {
        let mut rows = self.sorted();
        let other = match up {
            true if at > 0 => at - 1,
            false if at + 1 < rows.len() => at + 1,
            _ => return self.clone(),
        };
        if at >= rows.len() {
            return self.clone();
        }
        let (delay_at, delay_other) = (rows[at].delay, rows[other].delay);
        rows.swap(at, other);
        rows[at].delay = delay_at;
        rows[other].delay = delay_other;
        self.with_rows(rows)
    }

    /// The script with `row` inserted as step `at`, run straight after the
    /// step before it: its delay is that step's, or 0 for the first step.
    /// `at` past the end appends.
    pub fn with_step_inserted(&self, at: usize, mut row: ScriptRow) -> Script {
        let mut rows = self.sorted();
        let at = at.min(rows.len());
        row.delay = match at {
            0 => 0,
            _ => rows[at - 1].delay,
        };
        rows.insert(at, row);
        self.with_rows(rows)
    }

    /// The script with a copy of step `at` straight after it.
    pub fn with_step_copied(&self, at: usize) -> Script {
        let mut rows = self.sorted();
        let Some(row) = rows.get(at).cloned() else {
            return self.clone();
        };
        rows.insert(at + 1, row);
        self.with_rows(rows)
    }

    /// The script without step `at`.
    pub fn with_step_removed(&self, at: usize) -> Script {
        let mut rows = self.sorted();
        if at < rows.len() {
            rows.remove(at);
        }
        self.with_rows(rows)
    }

    /// The script with step `at` replaced by `row`, keeping the step's place.
    pub fn with_step(&self, at: usize, row: ScriptRow) -> Script {
        let mut rows = self.sorted();
        if let Some(target) = rows.get_mut(at) {
            *target = row;
        }
        Script { table: self.table, id: self.id, rows }
    }

    /// The same table and id over `rows`, which are in the order meant, with
    /// the priorities settled.
    fn with_rows(&self, rows: Vec<ScriptRow>) -> Script {
        Script { table: self.table, id: self.id, rows: settle(rows) }
    }
}

/// Make the server's `(delay, priority)` order the order `rows` are in.
///
/// `rows` are in the order meant, with delays that never decrease. Rows that
/// share a delay are ordered by priority, so a run of them whose priorities do
/// not already rise in the order meant is renumbered 0, 1, 2 and on. A run
/// that already rises keeps its numbers, so an operation leaves the rows it
/// does not reorder as the database has them.
fn settle(mut rows: Vec<ScriptRow>) -> Vec<ScriptRow> {
    let mut start = 0;
    while start < rows.len() {
        let delay = rows[start].delay;
        let end = rows[start..]
            .iter()
            .position(|row| row.delay != delay)
            .map_or(rows.len(), |len| start + len);
        let rises = rows[start..end].windows(2).all(|pair| pair[0].priority < pair[1].priority);
        if !rises {
            for (n, row) in rows[start..end].iter_mut().enumerate() {
                row.priority = n as u32;
            }
        }
        start = end;
    }
    rows
}

/// The statements a script comes to: every row under the id deleted, then
/// one `INSERT` per row in the server's order. A script with no rows is the
/// `DELETE` alone.
pub fn statements(script: &Script) -> Vec<String> {
    let mut out = vec![format!(
        "DELETE FROM {} WHERE `id` = {};",
        crate::sql::name(script.table),
        script.id
    )];
    for row in script.sorted() {
        let values = row.values();
        let names: Vec<String> = std::iter::once("id".to_string())
            .chain(values.iter().map(|(name, _)| name.to_string()))
            .map(|name| crate::sql::name(&name))
            .collect();
        let literals: Vec<String> = std::iter::once(script.id.to_string())
            .chain(values.into_iter().map(|(_, value)| value))
            .collect();
        out.push(format!(
            "INSERT INTO {} ({}) VALUES ({});",
            crate::sql::name(script.table),
            names.join(", "),
            literals.join(", ")
        ));
    }
    out
}

/// Every row of one script, in the order the server runs them.
pub fn rows_query(table: &str, id: u32) -> String {
    format!(
        "SELECT * FROM {} WHERE `id` = {id} ORDER BY `delay`, `priority`",
        crate::sql::name(table)
    )
}

/// What puts a script back as it stood: the same `DELETE`, then the rows read.
pub fn undo_from_rows(table: &str, id: u32, rows: &[Row]) -> Vec<String> {
    let mut out = vec![format!(
        "DELETE FROM {} WHERE `id` = {id};",
        crate::sql::name(table)
    )];
    for row in rows {
        out.extend(crate::row::insert_from_row(table, row));
    }
    out
}

/// Every row of several scripts of one table, id by id in the server's order,
/// or `None` for no ids. A script with no rows reads back as no rows.
pub fn rows_of_query(table: &str, ids: &[u32]) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let list: Vec<String> = ids.iter().map(u32::to_string).collect();
    Some(format!(
        "SELECT * FROM {} WHERE `id` IN ({}) ORDER BY `id`, `delay`, `priority`",
        crate::sql::name(table),
        list.join(", ")
    ))
}

/// Every row of the scripts of one table whose comments hold `term`, or
/// whose id is `term`, at most `limit` scripts: the search behind choosing
/// an existing script.
pub fn search_query(table: &str, term: &str, limit: usize) -> String {
    let like = crate::sql::text(&format!("%{}%", term.trim()));
    let by_id = match term.trim().parse::<u32>() {
        Ok(id) => format!(" OR `id` = {id}"),
        Err(_) => String::new(),
    };
    let table = crate::sql::name(table);
    format!(
        "SELECT s.* FROM {table} s JOIN (SELECT DISTINCT `id` FROM {table} \
         WHERE `comments` LIKE {like}{by_id} ORDER BY `id` LIMIT {limit}) m ON m.`id` = s.`id` \
         ORDER BY s.`id`, s.`delay`, s.`priority`"
    )
}

/// The `broadcast_text` entries a row says, when it is a Talk row. See
/// [`crate::broadcast`], which reads them.
pub fn texts_of(row: &ScriptRow) -> Vec<u32> {
    match row.command {
        0 => row.dataint.iter().filter(|id| **id > 0).map(|id| *id as u32).collect(),
        _ => Vec::new(),
    }
}

/// The highest id a table holds, for numbering a new script.
pub fn max_id_query(table: &str) -> String {
    format!("SELECT MAX(`id`) AS `id` FROM {}", crate::sql::name(table))
}

/// Every script a project has edited.
///
/// [`crate::path::Paths`] for scripts: keyed by table and id, so a script
/// edited twice is one entry and the last reading wins. A script that is
/// present with no rows is the edit that removes it; a script not present is
/// one the project says nothing about.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Scripts {
    scripts: std::collections::BTreeMap<(&'static str, u32), Vec<ScriptRow>>,
}

impl Scripts {
    /// Set what this project says a script is, in the server's order. No
    /// rows is a script removed.
    pub fn set(&mut self, script: &Script) {
        self.scripts.insert((script.table, script.id), script.sorted());
    }

    /// Stop claiming anything about a script.
    pub fn forget(&mut self, table: &'static str, id: u32) {
        self.scripts.remove(&(table, id));
    }

    /// What this project says a script is, if it says anything.
    pub fn get(&self, table: &'static str, id: u32) -> Option<Script> {
        let rows = self.scripts.get(&(table, id))?;
        Some(Script { table, id, rows: rows.clone() })
    }

    /// Whether the project claims the script at all.
    pub fn touches(&self, table: &'static str, id: u32) -> bool {
        self.scripts.contains_key(&(table, id))
    }

    pub fn len(&self) -> usize {
        self.scripts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }

    /// Every edited script, table by table and id by id.
    pub fn iter(&self) -> impl Iterator<Item = Script> + '_ {
        self.scripts
            .iter()
            .map(|((table, id), rows)| Script { table, id: *id, rows: rows.clone() })
    }

    /// The project's reading of a script laid over the database's.
    pub fn over(&self, from_database: &Script) -> Script {
        self.get(from_database.table, from_database.id)
            .unwrap_or_else(|| from_database.clone())
    }

    /// The file a project keeps this in: a `script` line per script, then a
    /// `row` line per row, tab separated, the comment last.
    pub fn to_text(&self, project: &str) -> String {
        let mut out = format!(
            "# {project} — the scripts this project changes.\n\
             # `script` declares that this project owns every row under that id;\n\
             # a `script` with no `row` lines after it is a script removed. A `row`\n\
             # line carries the 21 columns after `id`, in the table's order, the\n\
             # comment last. See crates/mangos/src/scripts.rs.\n\
             #\n\
             # script\ttable\tid\n\
             # row\ttable\tid\t{}\n",
            COLUMNS
                .iter()
                .skip(1)
                .map(|column| column.name)
                .collect::<Vec<_>>()
                .join("\t")
        );
        for script in self.iter() {
            out.push_str(&format!("script\t{}\t{}\n", script.table, script.id));
            for row in script.sorted() {
                out.push_str(&format!("row\t{}\t{}\t{}\n", script.table, script.id, row.to_line()));
            }
        }
        out
    }

    /// The store read from a file [`Scripts::to_text`] wrote. A damaged line
    /// is skipped and the rest are read, on [`crate::row::Edits::from_text`]'s
    /// rule. A `row` line for a script with no `script` line declares the
    /// script as well.
    pub fn from_text(text: &str) -> Scripts {
        let mut out = Scripts::default();
        for line in text.lines() {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let mut fields = line.splitn(4, '\t');
            let kind = fields.next().unwrap_or("").trim();
            let Some(table) = fields.next().and_then(|name| table_named(name.trim())) else {
                continue;
            };
            let Some(id) = fields.next().and_then(|id| id.trim().parse::<u32>().ok()) else {
                continue;
            };
            match kind {
                "script" => {
                    out.scripts.entry((table, id)).or_default();
                }
                "row" => {
                    let Some(row) = fields.next().and_then(ScriptRow::from_line) else {
                        continue;
                    };
                    out.scripts.entry((table, id)).or_default().push(row);
                }
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 94 commands are numbered 0 to 93 in order, and the enumeration
    /// built from them names each once.
    #[test]
    fn the_commands_are_numbered_in_order() {
        for (at, command) in COMMANDS.iter().enumerate() {
            assert_eq!(command.command as usize, at, "{}", command.name);
            assert_eq!(COMMAND_VALUES[at].name, command.name);
        }
        assert_eq!(command(15).map(|c| c.name), Some("Cast spell"));
        assert_eq!(command(15).and_then(|c| c.datalong[0]).map(|p| p.name), Some("spell_id"));
        assert!(command(94).is_none());
        for (at, target) in TARGETS.iter().enumerate() {
            assert_eq!(target.value as usize, at);
        }
    }

    /// The schema is `LoadScripts`' `SELECT`, column for column, with
    /// `priority` and `comments` where the loader's `ORDER BY` and the table
    /// put them.
    #[test]
    fn every_column_is_the_servers_own() {
        const SELECTED: &str = "id, delay, command, datalong, datalong2, datalong3, datalong4, \
                                target_param1, target_param2, target_type, data_flags, dataint, \
                                dataint2, dataint3, dataint4, x, y, z, o, condition_id";
        let ours: Vec<&str> = COLUMNS
            .iter()
            .map(|column| column.name)
            .filter(|name| !matches!(*name, "priority" | "comments"))
            .collect();
        let theirs: Vec<&str> = SELECTED.split(", ").map(str::trim).collect();
        assert_eq!(ours, theirs);
        assert_eq!(COLUMNS.len(), 22);
    }

    /// A script is one `DELETE` under its id and one `INSERT` per row, in
    /// `(delay, priority)` order; a script with no rows is the `DELETE` alone.
    #[test]
    fn a_script_is_written_whole() {
        let mut later = ScriptRow::new(1);
        later.delay = 2000;
        later.datalong[0] = 66;
        let mut first = ScriptRow::new(0);
        first.dataint[0] = 1234;
        first.comments = "Guard - Talk".to_string();
        let script = Script { table: CREATURE_AI, id: 6801, rows: vec![later, first] };
        let sql = statements(&script);
        assert_eq!(sql.len(), 3);
        assert_eq!(sql[0], "DELETE FROM `creature_ai_scripts` WHERE `id` = 6801;");
        assert!(sql[1].contains("`command`") && sql[1].contains("VALUES (6801, 0, 0, 0,"), "{}", sql[1]);
        assert!(sql[1].contains("'Guard - Talk'"), "{}", sql[1]);
        assert!(sql[2].contains("VALUES (6801, 2000, 0, 1, 66,"), "{}", sql[2]);
        assert_eq!(statements(&Script::empty(GOSSIP, 5)), vec!["DELETE FROM `gossip_scripts` WHERE `id` = 5;"]);
    }

    /// A row round-trips through the undo line and the store's file, comment
    /// included; a row with no command does not read.
    #[test]
    fn a_row_round_trips_through_text() {
        let mut row = ScriptRow::new(3);
        row.delay = 500;
        row.x = -8817.3;
        row.dataint[1] = -4;
        row.comments = "Move to the gate".to_string();
        assert_eq!(ScriptRow::from_line(&row.to_line()), Some(row.clone()));
        let script = Script { table: GENERIC, id: 9, rows: vec![row.clone(), ScriptRow::new(0)] };
        assert_eq!(Script::from_line(GENERIC, 9, &script.to_line()), script);
        let mut store = Scripts::default();
        store.set(&script);
        store.set(&Script::empty(CREATURE_AI, 11));
        let read = Scripts::from_text(&store.to_text("test"));
        assert_eq!(read, store);
        assert_eq!(read.get(CREATURE_AI, 11).map(|script| script.rows.len()), Some(0));
        assert!(!read.touches(CREATURE_AI, 12));
        assert!(ScriptRow::from_line("not a row").is_none());
    }

    /// A row reads back out of the database's columns, and its summary names
    /// the command's parameters in the header's words.
    #[test]
    fn a_row_is_read_as_the_server_reads_it() {
        let mut db = Row::new();
        for (column, value) in [("id", "6801"), ("delay", "0"), ("command", "1"), ("datalong", "2"), ("dataint", "1"), ("x", "0"), ("comments", "Bow")] {
            db.insert(column.to_string(), Some(value.to_string()));
        }
        let row = ScriptRow::from_row(&db).expect("a row");
        assert_eq!(row.command, 1);
        assert_eq!(row.datalong[0], 2);
        assert_eq!(row.dataint[0], 1);
        assert_eq!(row.summary(), "Emote: emote 2, is_targeted Yes");
        let mut talk = ScriptRow::new(0);
        talk.dataint[0] = 1234;
        assert_eq!(talk.summary(), "Talk: chat_type Say, broadcast_text 1234");
        assert_eq!(ScriptRow::new(33).summary(), "Enter evade mode");
        assert_eq!(ScriptRow::new(200).summary(), "command 200");
    }

    /// A row reads as a sentence, its references named where the lookup can
    /// and numbered where it cannot, with the target after it.
    #[test]
    fn a_row_reads_as_a_sentence() {
        let names = |table: &str, id: u32| match (table, id) {
            ("Spell", 6660) => Some("Shoot".to_string()),
            ("broadcast_text", 1234) => Some("Stand fast!".to_string()),
            ("creature_template", 68) => Some("Stormwind City Guard".to_string()),
            _ => None,
        };
        let mut cast = ScriptRow::new(15);
        cast.datalong[0] = 6660;
        cast.target_type = 1;
        assert_eq!(cast.sentence(&names), "Cast Shoot on the current victim");
        cast.datalong[1] = 0x002;
        assert_eq!(cast.sentence(&crate::schema::no_names), "Cast spell 6660 (triggered) on the current victim");
        let mut talk = ScriptRow::new(0);
        talk.dataint[0] = 1234;
        assert_eq!(talk.sentence(&names), "Say: \u{201c}Stand fast!\u{201d}");
        talk.datalong[0] = 1;
        talk.dataint[1] = 99;
        assert_eq!(talk.sentence(&names), "Yell one of 2 texts: \u{201c}Stand fast!\u{201d}, \u{2026}");
        assert_eq!(texts_of(&talk), vec![1234, 99]);
        let mut summon = ScriptRow::new(10);
        summon.datalong = [68, 30_000, 0, 0];
        assert_eq!(summon.sentence(&names), "Summon Stormwind City Guard for 30 s");
        let mut phase = ScriptRow::new(44);
        phase.datalong[0] = 2;
        assert_eq!(phase.sentence(&names), "Set phase to 2");
        let mut near = ScriptRow::new(15);
        near.datalong[0] = 6660;
        near.target_type = 10;
        near.target_param1 = 68;
        near.target_param2 = 30;
        assert_eq!(near.sentence(&names), "Cast Shoot on the nearest Stormwind City Guard within 30 yards");
        assert_eq!(ScriptRow::new(47).sentence(&names), "Flee");
        assert_eq!(ScriptRow::new(200).sentence(&names), "command 200");
    }

    /// The steps of a script: the wait before each, and the five operations,
    /// each of which leaves the server's `(delay, priority)` order the order
    /// meant.
    #[test]
    fn steps_are_moved_and_timed_in_the_servers_order() {
        let step = |command: u32, delay: u32| ScriptRow { delay, ..ScriptRow::new(command) };
        let script = Script { table: CREATURE_AI, id: 1, rows: vec![step(15, 0), step(0, 2000), step(1, 5000)] };
        let commands = |script: &Script| script.sorted().iter().map(|row| (row.command, row.delay)).collect::<Vec<_>>();
        assert_eq!((script.wait_before(0), script.wait_before(1), script.wait_before(2)), (0, 2000, 3000));

        // A longer wait before the second step moves it and the third.
        let waited = script.with_wait(1, 2500);
        assert_eq!(commands(&waited), vec![(15, 0), (0, 2500), (1, 5500)]);
        assert_eq!(waited.wait_before(2), 3000);

        // Moving the third step up swaps the actions and keeps the times.
        let moved = script.with_step_moved(2, true);
        assert_eq!(commands(&moved), vec![(15, 0), (1, 2000), (0, 5000)]);
        assert_eq!(script.with_step_moved(0, true), script);

        // An inserted step runs straight after the one before it, and the
        // two that share a delay are ordered by priority.
        let inserted = script.with_step_inserted(1, ScriptRow::new(44));
        assert_eq!(commands(&inserted), vec![(15, 0), (44, 0), (0, 2000), (1, 5000)]);
        assert_eq!(inserted.sorted()[1].priority, 1);
        let first = script.with_step_inserted(0, ScriptRow::new(44));
        assert_eq!(commands(&first), vec![(44, 0), (15, 0), (0, 2000), (1, 5000)]);

        // A copy follows its original; a removal drops the step.
        let copied = script.with_step_copied(1);
        assert_eq!(commands(&copied), vec![(15, 0), (0, 2000), (0, 2000), (1, 5000)]);
        assert_eq!(commands(&script.with_step_removed(1)), vec![(15, 0), (1, 5000)]);

        // Two steps at one delay swap by priority.
        let swapped = inserted.with_step_moved(1, true);
        assert_eq!(commands(&swapped), vec![(44, 0), (15, 0), (0, 2000), (1, 5000)]);

        // A run whose priorities already rise keeps them.
        let kept = settle(vec![ScriptRow { priority: 3, ..step(1, 0) }, ScriptRow { priority: 7, ..step(2, 0) }]);
        assert_eq!((kept[0].priority, kept[1].priority), (3, 7));
    }

    /// The batch and search queries name what they read.
    #[test]
    fn the_reads_are_one_query_each() {
        assert_eq!(rows_of_query(CREATURE_AI, &[]), None);
        let rows = rows_of_query(CREATURE_AI, &[6801, 6802]).unwrap_or_default();
        assert!(rows.contains("`id` IN (6801, 6802)") && rows.ends_with("ORDER BY `id`, `delay`, `priority`"), "{rows}");
        let search = search_query(CREATURE_AI, "Shadow Bolt", 40);
        assert!(search.contains("`comments` LIKE '%Shadow Bolt%'") && search.contains("LIMIT 40"), "{search}");
        assert!(search_query(CREATURE_AI, "6801", 40).contains("OR `id` = 6801"));
    }

    /// Every table is named once, and the reload each has is the server's.
    #[test]
    fn the_eleven_tables_are_named_once() {
        assert_eq!(ALL.len(), TABLES.len());
        for name in TABLES {
            assert_eq!(table_named(name), Some(name));
            assert!(name.ends_with("_scripts"));
        }
        assert_eq!(table(CREATURE_AI).and_then(|t| t.reload), Some("creature_ai_events"));
        assert_eq!(table(QUEST_END).and_then(|t| t.reload), None);
        assert!(table_named("creature_template").is_none());
        assert!(!waits(CREATURE_AI) && waits(GENERIC) && waits(QUEST_START));
    }
}
