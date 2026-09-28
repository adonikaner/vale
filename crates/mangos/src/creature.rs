//! `creature_template` and `creature`: what an NPC is, and where one stands.
//!
//! ## The two tables
//!
//! A `creature_template` row describes a kind of creature: its name, its level
//! band, its faction, the display ids it may wear, what it sells and what it
//! drops. Changing one changes every Stormwind guard in the world.
//!
//! A `creature` row is one spawn: a template id, a map, a position, a facing,
//! and how it respawns. Changing one moves one guard. The reference install has
//! 66,108 of them, over 36 maps.
//!
//! They are edited as two subjects rather than as two tabs of one, because the
//! scope of an edit differs between them, and a person has to know which scope
//! they are editing before they type.
//!
//! ## The patch column is part of the key
//!
//! `creature_template`'s primary key is `(entry, patch)`, and
//! `ObjectMgr::LoadCreatureTemplates` (`ObjectMgr.cpp:1191`) takes, for each
//! entry, the row with the highest `patch` at or below the server's configured
//! `WowPatch`:
//!
//! ```sql
//! WHERE `patch` = (SELECT max(`patch`) FROM `creature_template` t2
//!                  WHERE t1.`entry` = t2.`entry` && `patch` <= %u)
//! ```
//!
//! `WowPatch = 10` on the reference install, and 10 of the 10,828 entries'
//! patches are spread from 0 to 10. So an `UPDATE … WHERE entry = 3296` with no
//! patch would change every content-patch version of that creature, including
//! the ones the running server is not using. Such a change applies and reloads
//! without error and changes nothing the server uses.
//!
//! This crate therefore edits the row the server would load, named by both its
//! columns, and [`winning_template_query`] is how that row is found. The key is
//! an [`crate::row::Key`] of two columns rather than one id.
//!
//! ## An edit is an `UPDATE` in place, and the undo is the reversal
//!
//! [`crate::spell`] layers a new row at `build = 5875` over the shipped ones,
//! because `spell_template`'s key is `(entry, build)` and 5875 is above every
//! build vmangos ships. There is no equivalent here: `patch` tops out at the
//! server's own `WowPatch`, so there is no value to write at that is both above
//! the shipped rows and still loaded. A new row would be invisible or would
//! collide.
//!
//! So an edit changes the row that is there, and reversibility comes from the
//! undo file rather than from the layer: the columns about to change are read
//! out of the database and the statement that puts them back is written before
//! anything runs. See `crate::row::undo`, and the editor's `server::rows`, which
//! is what writes it.
//!
//! ## A spawn can be created and removed; a template can be created and not removed
//!
//! A `creature` row is created by this crate and removed by it. The guid comes
//! from [`RESERVED_GUID_BASE`], which is clear of anything vmangos' own updates
//! will reach, and a removal takes the five tables keyed by that guid with it.
//! See [`DEPENDENTS`], which is `Creature::DeleteFromDB`'s own list.
//!
//! A `creature_template` row is created by this crate and not removed. A new
//! template starts from [`new_template`], whose values are the table's own
//! defaults except where `ObjectMgr::CheckCreatureTemplate` corrects or refuses
//! them, and its entry comes from [`RESERVED_ENTRY_BASE`]. A removal is not
//! offered: `.reload creature_template` adds and overwrites and never drops an
//! entry it has already read, so a removal cannot be made live at all.
//! [`can_live`] is the one place that asymmetry is written down.
//!
//! ## Neither a created nor a removed spawn reaches a running server
//!
//! `.reload creature` calls `LoadCreatures(true)`, which adds a row it has not
//! seen to its grid (`ObjectMgr.cpp:2473`) and never erases one: the data map
//! keeps a removed spawn and the grid keeps the creature standing in it. A row
//! it does add only becomes a creature when that grid is next loaded from
//! nothing, and a grid a player is standing on is not unloaded while they stand
//! there.
//!
//! So a spawn created or removed here reaches the world the way an edited
//! template does: the statements are applied, and the server is restarted. The
//! editor's panel says so rather than offering a reload that would report a
//! change the game does not show.

use crate::row::{Assignment, Key, Life};

/// What a creature is.
pub const TEMPLATE: &str = "creature_template";

/// One spawn of a creature: where it stands.
pub const SPAWN: &str = "creature";

/// Every table this module writes, for a caller that has to resolve a name read
/// out of a file back to one of these constants.
///
/// A table name in a project's edits file is text somebody may have typed. This
/// is what says whether it is a table this crate knows how to write, and a name
/// that is not on the list is reported rather than passed into a statement.
pub const TABLES: [&str; 2] = [TEMPLATE, SPAWN];

/// The static name for a table read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// The column vocabulary, re-exported from [`crate::schema`].
///
/// `item_template` uses every one of these types unchanged, so they live in
/// [`crate::schema`]. They are re-exported rather than renamed:
/// `creature::Kind::Text` is what every caller here says, and a column of
/// `creature_template` is still described in this file.
pub use crate::schema::{
    mask_words, value_word, Bit, Column, Group, Kind, Row, RowValue, Value, NO_YES,
};

/// `UNIT_NPC_FLAGS`, from `UnitDefines.h:656`. What the client draws over a
/// head and what a right-click offers.
pub const NPC_FLAGS: [Bit; 15] = [
    Bit { bit: 0x00000001, name: "GOSSIP", about: "has something to say" },
    Bit { bit: 0x00000002, name: "QUESTGIVER", about: "gives or takes quests" },
    Bit { bit: 0x00000004, name: "VENDOR", about: "sells from npc_vendor" },
    Bit { bit: 0x00000008, name: "FLIGHTMASTER", about: "taxi node" },
    Bit { bit: 0x00000010, name: "TRAINER", about: "teaches from npc_trainer" },
    Bit { bit: 0x00000020, name: "SPIRITHEALER", about: "resurrects for a penalty" },
    Bit { bit: 0x00000040, name: "SPIRITGUIDE", about: "battleground spirit guide" },
    Bit { bit: 0x00000080, name: "INNKEEPER", about: "sets the hearthstone" },
    Bit { bit: 0x00000100, name: "BANKER", about: "opens the bank" },
    Bit { bit: 0x00000200, name: "PETITIONER", about: "guild charters" },
    Bit { bit: 0x00000400, name: "TABARDDESIGNER", about: "guild tabards" },
    Bit { bit: 0x00000800, name: "BATTLEMASTER", about: "queues for a battleground" },
    Bit { bit: 0x00001000, name: "AUCTIONEER", about: "opens the auction house" },
    Bit { bit: 0x00002000, name: "STABLEMASTER", about: "stables a hunter's pet" },
    Bit { bit: 0x00004000, name: "REPAIR", about: "repairs equipment" },
];

/// `CreatureType`, from `CreatureDefines.h:41`. The same values as
/// `CreatureType.dbc`.
pub const CREATURE_TYPES: [Value; 11] = [
    Value { value: 1, name: "Beast" },
    Value { value: 2, name: "Dragonkin" },
    Value { value: 3, name: "Demon" },
    Value { value: 4, name: "Elemental" },
    Value { value: 5, name: "Giant" },
    Value { value: 6, name: "Undead" },
    Value { value: 7, name: "Humanoid" },
    Value { value: 8, name: "Critter" },
    Value { value: 9, name: "Mechanical" },
    Value { value: 10, name: "Not specified" },
    Value { value: 11, name: "Totem" },
];

/// `CreatureEliteType`, from `CreatureDefines.h:88`.
pub const RANKS: [Value; 5] = [
    Value { value: 0, name: "Normal" },
    Value { value: 1, name: "Elite" },
    Value { value: 2, name: "Rare elite" },
    Value { value: 3, name: "World boss" },
    Value { value: 4, name: "Rare" },
];

/// `unit_class`, which is the four values a creature's power type and combat
/// rating table are chosen by (`ObjectMgr.cpp:1467`, and `CreatureClassLevelStats`).
pub const UNIT_CLASSES: [Value; 4] = [
    Value { value: 1, name: "Warrior" },
    Value { value: 2, name: "Paladin" },
    Value { value: 4, name: "Rogue" },
    Value { value: 8, name: "Mage" },
];

/// `MovementGeneratorType`, from `MotionMaster.h:38`. The three a spawn or a
/// template may declare; the rest are chosen by the server at run time.
pub const MOVEMENT_TYPES: [Value; 3] = [
    Value { value: 0, name: "Idle" },
    Value { value: 1, name: "Random" },
    Value { value: 2, name: "Waypoint" },
];

/// `InhabitTypeValues`, from `CreatureDefines.h:491`. A mask, not a choice:
/// 3 is ground and water, 7 is anywhere.
pub const INHABIT_TYPES: [Bit; 3] = [
    Bit { bit: 1, name: "GROUND", about: "walks" },
    Bit { bit: 2, name: "WATER", about: "swims" },
    Bit { bit: 4, name: "AIR", about: "flies" },
];

/// `CreatureStaticFlags`, from `CreatureDefines.h:96`: the first word.
pub const STATIC_FLAGS1: [Bit; 32] = [
    Bit { bit: 0x00000001, name: "MOUNTABLE", about: "not used by the core" },
    Bit { bit: 0x00000002, name: "NO_XP", about: "killing it gives no experience" },
    Bit { bit: 0x00000004, name: "NO_LOOT", about: "not used by the core" },
    Bit { bit: 0x00000008, name: "UNKILLABLE", about: "invincibility threshold at 1 HP" },
    Bit { bit: 0x00000010, name: "TAMEABLE", about: "a hunter may tame it" },
    Bit { bit: 0x00000020, name: "IMMUNE_TO_PC", about: "UNIT_FLAG_IMMUNE_TO_PLAYER on spawn" },
    Bit { bit: 0x00000040, name: "IMMUNE_TO_NPC", about: "UNIT_FLAG_IMMUNE_TO_NPC on spawn" },
    Bit { bit: 0x00000080, name: "CAN_WIELD_LOOT", about: "rolls loot on spawn and equips weapons" },
    Bit { bit: 0x00000100, name: "SESSILE", about: "cannot move" },
    Bit { bit: 0x00000200, name: "UNINTERACTIBLE", about: "UNIT_FLAG_NOT_SELECTABLE on spawn" },
    Bit { bit: 0x00000400, name: "NO_AUTOMATIC_REGEN", about: "does not regenerate" },
    Bit { bit: 0x00000800, name: "DESPAWN_INSTANTLY", about: "corpse goes at once" },
    Bit { bit: 0x00001000, name: "CORPSE_RAID", about: "no distance check for loot and xp" },
    Bit { bit: 0x00002000, name: "CREATOR_LOOT", about: "looted by whoever created it" },
    Bit { bit: 0x00004000, name: "NO_DEFENSE", about: "defense skill is 0" },
    Bit { bit: 0x00008000, name: "NO_SPELL_DEFENSE", about: "cannot resist spells" },
    Bit { bit: 0x00010000, name: "RAID_BOSS_MOB", about: "not used by the core" },
    Bit { bit: 0x00020000, name: "COMBAT_PING", about: "pings the minimap on entering combat" },
    Bit { bit: 0x00040000, name: "AQUATIC", about: "inhabit type water" },
    Bit { bit: 0x00080000, name: "AMPHIBIOUS", about: "inhabit type ground and water" },
    Bit { bit: 0x00100000, name: "NO_MELEE", about: "does not auto attack" },
    Bit { bit: 0x00200000, name: "VISIBLE_TO_GHOSTS", about: "spirit healers" },
    Bit { bit: 0x00400000, name: "PVP_ENABLING", about: "flags attackers for pvp" },
    Bit { bit: 0x00800000, name: "DO_NOT_PLAY_WOUND_ANIM", about: "no wound animation" },
    Bit { bit: 0x01000000, name: "NO_FACTION_TOOLTIP", about: "not used by the core" },
    Bit { bit: 0x02000000, name: "IGNORE_COMBAT", about: "react state passive" },
    Bit { bit: 0x04000000, name: "ONLY_ATTACK_PVP_ENABLING", about: "no proximity aggro on players not flagged for pvp" },
    Bit { bit: 0x08000000, name: "CALLS_GUARDS", about: "summons a guard when an enemy player comes near or attacks" },
    Bit { bit: 0x10000000, name: "CAN_SWIM", about: "UNIT_FLAG_USE_SWIM_ANIMATION on spawn" },
    Bit { bit: 0x20000000, name: "FLOATING", about: "MOVEFLAG_FIXED_Z on spawn" },
    Bit { bit: 0x40000000, name: "MORE_AUDIBLE", about: "heard from further away" },
    Bit { bit: 0x80000000, name: "LARGE_AOI", about: "seen from 200 yards" },
];

/// `CreatureStaticFlags2`, from `CreatureDefines.h:132`: the second word.
pub const STATIC_FLAGS2: [Bit; 7] = [
    Bit { bit: 0x01, name: "NO_PET_SCALING", about: "not used by the core" },
    Bit { bit: 0x02, name: "FORCE_RAID_COMBAT", about: "puts the whole zone in combat on aggro" },
    Bit { bit: 0x04, name: "LOCK_TAPPERS_TO_RAID_ON_DEATH", about: "killing it binds the players to the raid" },
    Bit { bit: 0x08, name: "NO_HARMFUL_VERTEX_COLORING", about: "not used by the core" },
    Bit { bit: 0x10, name: "NO_CRUSHING_BLOWS", about: "never lands a crushing blow" },
    Bit { bit: 0x20, name: "NO_OWNER_THREAT", about: "does not put its owner in combat" },
    Bit { bit: 0x40, name: "NO_WOUNDED_SLOWDOWN", about: "does not slow down at low health" },
];

/// `CreatureFlagsExtra`, from `CreatureDefines.h:155`: vmangos' own flags.
pub const FLAGS_EXTRA: [Bit; 20] = [
    Bit { bit: 0x00000001, name: "NO_LEASH_EVADE", about: "does not evade when its target runs away" },
    Bit { bit: 0x00000002, name: "NO_AGGRO", about: "defensive: does not attack hostiles that come near" },
    Bit { bit: 0x00000004, name: "NO_PARRY", about: "cannot parry" },
    Bit { bit: 0x00000008, name: "NO_UNREACHABLE_EVADE", about: "does not evade when its target cannot be reached" },
    Bit { bit: 0x00000010, name: "NO_BLOCK", about: "cannot block" },
    Bit { bit: 0x00000020, name: "NO_MOVEMENT_PAUSE", about: "does not stop when a player talks to it" },
    Bit { bit: 0x00000040, name: "ALWAYS_RUN", about: "runs out of combat" },
    Bit { bit: 0x00000080, name: "INVISIBLE", about: "never seen by players; a trigger" },
    Bit { bit: 0x00000100, name: "GIGANTIC_AOI", about: "seen from 400 yards" },
    Bit { bit: 0x00000200, name: "INFINITE_AOI", about: "seen from anywhere on the map" },
    Bit { bit: 0x00000400, name: "GUARD", about: "a guard" },
    Bit { bit: 0x00000800, name: "NO_THREAT_LIST", about: "no threat list; a five-second combat timer, as a player has" },
    Bit { bit: 0x00001000, name: "KEEP_POSITIVE_AURAS_ON_EVADE", about: "keeps its buffs when it resets" },
    Bit { bit: 0x00002000, name: "ALWAYS_CRUSH", about: "every hit that lands is a crushing blow" },
    Bit { bit: 0x00004000, name: "APPEAR_DEAD", about: "UNIT_DYNFLAG_DEAD applied" },
    Bit { bit: 0x00008000, name: "CHASE_GEN_NO_BACKING", about: "does not back away from a target inside its reach" },
    Bit { bit: 0x00010000, name: "NO_ASSIST", about: "does not join when creatures nearby aggro" },
    Bit { bit: 0x00020000, name: "NO_TARGET", about: "passive: acquires no targets" },
    Bit { bit: 0x00040000, name: "ONLY_VISIBLE_TO_FRIENDLY", about: "seen only by friendly units" },
    Bit { bit: 0x00080000, name: "CAN_ASSIST", about: "CREATURE_TYPEFLAGS_CAN_ASSIST from TBC" },
];

/// `CreatureImmunityFlags`, from `CreatureDefines.h:179`.
pub const IMMUNITY_FLAGS: [Bit; 7] = [
    Bit { bit: 0x01, name: "AOE", about: "area spells" },
    Bit { bit: 0x02, name: "TAUNT", about: "taunts" },
    Bit { bit: 0x04, name: "MOD_STAT", about: "stat changes" },
    Bit { bit: 0x08, name: "MOD_CAST_SPEED", about: "casting speed changes" },
    Bit { bit: 0x10, name: "DISEASE", about: "diseases" },
    Bit { bit: 0x20, name: "POISON", about: "poisons" },
    Bit { bit: 0x40, name: "CURSE", about: "curses" },
];

/// `Mechanics`, from `SpellDefines.h:659`, as the mask
/// `mechanic_immune_mask` is: mechanic `m` is bit `1 << (m - 1)`
/// (`Creature::IsImmuneToSpell`, `Creature.cpp:2445`).
pub const MECHANIC_MASK: [Bit; 31] = [
    Bit { bit: 1 << 0, name: "CHARM", about: "mechanic 1" },
    Bit { bit: 1 << 1, name: "DISORIENTED", about: "mechanic 2" },
    Bit { bit: 1 << 2, name: "DISARM", about: "mechanic 3" },
    Bit { bit: 1 << 3, name: "DISTRACT", about: "mechanic 4" },
    Bit { bit: 1 << 4, name: "FEAR", about: "mechanic 5" },
    Bit { bit: 1 << 5, name: "FUMBLE", about: "mechanic 6" },
    Bit { bit: 1 << 6, name: "ROOT", about: "mechanic 7" },
    Bit { bit: 1 << 7, name: "PACIFY", about: "mechanic 8; no spell uses it" },
    Bit { bit: 1 << 8, name: "SILENCE", about: "mechanic 9" },
    Bit { bit: 1 << 9, name: "SLEEP", about: "mechanic 10" },
    Bit { bit: 1 << 10, name: "SNARE", about: "mechanic 11" },
    Bit { bit: 1 << 11, name: "STUN", about: "mechanic 12" },
    Bit { bit: 1 << 12, name: "FREEZE", about: "mechanic 13" },
    Bit { bit: 1 << 13, name: "KNOCKOUT", about: "mechanic 14" },
    Bit { bit: 1 << 14, name: "BLEED", about: "mechanic 15" },
    Bit { bit: 1 << 15, name: "BANDAGE", about: "mechanic 16" },
    Bit { bit: 1 << 16, name: "POLYMORPH", about: "mechanic 17" },
    Bit { bit: 1 << 17, name: "BANISH", about: "mechanic 18" },
    Bit { bit: 1 << 18, name: "SHIELD", about: "mechanic 19" },
    Bit { bit: 1 << 19, name: "SHACKLE", about: "mechanic 20" },
    Bit { bit: 1 << 20, name: "MOUNT", about: "mechanic 21" },
    Bit { bit: 1 << 21, name: "PERSUADE", about: "mechanic 22; no spell uses it" },
    Bit { bit: 1 << 22, name: "TURN", about: "mechanic 23" },
    Bit { bit: 1 << 23, name: "HORROR", about: "mechanic 24" },
    Bit { bit: 1 << 24, name: "INVULNERABILITY", about: "mechanic 25" },
    Bit { bit: 1 << 25, name: "INTERRUPT", about: "mechanic 26" },
    Bit { bit: 1 << 26, name: "DAZE", about: "mechanic 27" },
    Bit { bit: 1 << 27, name: "DISCOVERY", about: "mechanic 28" },
    Bit { bit: 1 << 28, name: "IMMUNE_SHIELD", about: "mechanic 29: Divine Shield, Blessing of Protection, Ice Block" },
    Bit { bit: 1 << 29, name: "SAPPED", about: "mechanic 30" },
    Bit { bit: 1 << 30, name: "SLOW_CAST_SPEED", about: "mechanic 31, vmangos' own: Curse of Tongues" },
];

/// `SpellSchools`, from `SpellDefines.h:758`, as the mask
/// `school_immune_mask` is: school `s` is bit `1 << s`
/// (`Creature.cpp:2448`).
pub const SCHOOL_MASK: [Bit; 7] = [
    Bit { bit: 0x01, name: "PHYSICAL", about: "school 0: melee and physical spells" },
    Bit { bit: 0x02, name: "HOLY", about: "school 1" },
    Bit { bit: 0x04, name: "FIRE", about: "school 2" },
    Bit { bit: 0x08, name: "NATURE", about: "school 3" },
    Bit { bit: 0x10, name: "FROST", about: "school 4" },
    Bit { bit: 0x20, name: "SHADOW", about: "school 5" },
    Bit { bit: 0x40, name: "ARCANE", about: "school 6" },
];

/// `TrainerType`, from `CreatureDefines.h:29`.
pub const TRAINER_TYPES: [Value; 4] = [
    Value { value: 0, name: "Class" },
    Value { value: 1, name: "Mounts" },
    Value { value: 2, name: "Tradeskills" },
    Value { value: 3, name: "Pets" },
];

/// `Classes`, from `SharedDefines.h:87`, with 0 for none: what
/// `trainer_class` holds.
pub const TRAINER_CLASSES: [Value; 10] = [
    Value { value: 0, name: "None" },
    Value { value: 1, name: "Warrior" },
    Value { value: 2, name: "Paladin" },
    Value { value: 3, name: "Hunter" },
    Value { value: 4, name: "Rogue" },
    Value { value: 5, name: "Priest" },
    Value { value: 7, name: "Shaman" },
    Value { value: 8, name: "Mage" },
    Value { value: 9, name: "Warlock" },
    Value { value: 11, name: "Druid" },
];

/// `Races`, from `SharedDefines.h:56`, with 0 for any: what `trainer_race`
/// holds. A mount trainer with a race trains only that race
/// (`Creature.cpp:1429`).
pub const TRAINER_RACES: [Value; 9] = [
    Value { value: 0, name: "Any" },
    Value { value: 1, name: "Human" },
    Value { value: 2, name: "Orc" },
    Value { value: 3, name: "Dwarf" },
    Value { value: 4, name: "Night Elf" },
    Value { value: 5, name: "Undead" },
    Value { value: 6, name: "Tauren" },
    Value { value: 7, name: "Gnome" },
    Value { value: 8, name: "Troll" },
];

/// `WowPatch`, from `Progression.h:64`: the content patches a spawn's
/// `patch_min` and `patch_max` count in.
pub const PATCHES: [Value; 11] = [
    Value { value: 0, name: "1.2" },
    Value { value: 1, name: "1.3" },
    Value { value: 2, name: "1.4" },
    Value { value: 3, name: "1.5" },
    Value { value: 4, name: "1.6" },
    Value { value: 5, name: "1.7" },
    Value { value: 6, name: "1.8" },
    Value { value: 7, name: "1.9" },
    Value { value: 8, name: "1.10" },
    Value { value: 9, name: "1.11" },
    Value { value: 10, name: "1.12" },
];

/// `SpawnFlags`, from `ObjectDefines.h:127`. A spawn's own, not the template's.
pub const SPAWN_FLAGS: [Bit; 8] = [
    Bit { bit: 0x01, name: "ACTIVE", about: "kept loaded whether or not a player is near" },
    Bit { bit: 0x02, name: "DISABLED", about: "not spawned at all" },
    Bit { bit: 0x04, name: "RANDOM_RESPAWN_TIME", about: "between the two respawn times" },
    Bit { bit: 0x08, name: "DYNAMIC_RESPAWN_TIME", about: "shortened when the zone is busy" },
    Bit { bit: 0x10, name: "FORCE_DYNAMIC_ELITE", about: "scales with the raid" },
    Bit { bit: 0x20, name: "EVADE_OUT_HOME_AREA", about: "evades when pulled out of its area" },
    Bit { bit: 0x40, name: "NOT_VISIBLE", about: "spawned but not shown" },
    Bit { bit: 0x80, name: "DEAD", about: "spawned dead" },
];

/// `creature_template`'s 78 columns, in the order
/// `ObjectMgr::LoadCreatureTemplates` selects them (`ObjectMgr.cpp:1191`),
/// which is also the order of the table.
///
/// The order is kept because that is the list to compare against the source
/// when vmangos adds a column: a schema that has drifted shows up as a name in
/// the wrong place rather than as a missing one.
pub const TEMPLATE_COLUMNS: [Column; 75] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the creature id" },
    Column { name: "name", kind: Kind::Text, group: Group::Identity, about: "what the nameplate says" },
    Column { name: "subname", kind: Kind::Text, group: Group::Identity, about: "the <title> under the name" },
    Column { name: "level_min", kind: Kind::Unsigned, group: Group::Identity, about: "lowest level it spawns at" },
    Column { name: "level_max", kind: Kind::Unsigned, group: Group::Identity, about: "highest level it spawns at" },
    Column { name: "faction", kind: Kind::Ref("FactionTemplate"), group: Group::Identity, about: "FactionTemplate.dbc id: who it fights" },
    Column { name: "npc_flags", kind: Kind::Flags(&NPC_FLAGS), group: Group::Services, about: "what a right-click offers" },
    Column { name: "gossip_menu_id", kind: Kind::Ref("gossip_menu"), group: Group::Services, about: "gossip_menu id: what it says" },
    Column { name: "display_id1", kind: Kind::Ref("CreatureDisplayInfo"), group: Group::Appearance, about: "the model it is drawn as" },
    Column { name: "display_id2", kind: Kind::Ref("CreatureDisplayInfo"), group: Group::Appearance, about: "a second model it may pick" },
    Column { name: "display_id3", kind: Kind::Ref("CreatureDisplayInfo"), group: Group::Appearance, about: "a third" },
    Column { name: "display_id4", kind: Kind::Ref("CreatureDisplayInfo"), group: Group::Appearance, about: "a fourth" },
    Column { name: "display_scale1", kind: Kind::Float, group: Group::Appearance, about: "size multiplier for model 1" },
    Column { name: "display_scale2", kind: Kind::Float, group: Group::Appearance, about: "…for model 2" },
    Column { name: "display_scale3", kind: Kind::Float, group: Group::Appearance, about: "…for model 3" },
    Column { name: "display_scale4", kind: Kind::Float, group: Group::Appearance, about: "…for model 4" },
    Column { name: "display_probability1", kind: Kind::Unsigned, group: Group::Appearance, about: "weight for model 1" },
    Column { name: "display_probability2", kind: Kind::Unsigned, group: Group::Appearance, about: "weight for model 2" },
    Column { name: "display_probability3", kind: Kind::Unsigned, group: Group::Appearance, about: "weight for model 3" },
    Column { name: "display_probability4", kind: Kind::Unsigned, group: Group::Appearance, about: "weight for model 4" },
    Column { name: "display_total_probability", kind: Kind::Unsigned, group: Group::Appearance, about: "the four weights added up" },
    Column { name: "mount_display_id", kind: Kind::Ref("CreatureDisplayInfo"), group: Group::Appearance, about: "what it rides" },
    Column { name: "speed_walk", kind: Kind::Float, group: Group::Behaviour, about: "walk speed, 1.0 = 2.5 yards a second" },
    Column { name: "speed_run", kind: Kind::Float, group: Group::Behaviour, about: "run speed, 1.0 = 7.0 yards a second" },
    Column { name: "detection_range", kind: Kind::Float, group: Group::Behaviour, about: "yards it notices a player at" },
    Column { name: "call_for_help_range", kind: Kind::Float, group: Group::Behaviour, about: "yards its shout reaches" },
    Column { name: "leash_range", kind: Kind::Float, group: Group::Behaviour, about: "yards from home before it evades" },
    Column { name: "type", kind: Kind::Choice(&CREATURE_TYPES), group: Group::Stats, about: "beast, humanoid, undead…" },
    Column { name: "pet_family", kind: Kind::Ref("CreatureFamily"), group: Group::Stats, about: "CreatureFamily.dbc id, for a tameable beast" },
    Column { name: "rank", kind: Kind::Choice(&RANKS), group: Group::Stats, about: "normal, elite, rare, world boss" },
    Column { name: "unit_class", kind: Kind::Choice(&UNIT_CLASSES), group: Group::Stats, about: "which stat table its level is read from" },
    Column { name: "xp_multiplier", kind: Kind::Float, group: Group::Stats, about: "experience for killing it" },
    Column { name: "health_multiplier", kind: Kind::Float, group: Group::Stats, about: "health against the class and level table" },
    Column { name: "mana_multiplier", kind: Kind::Float, group: Group::Stats, about: "mana against the same table" },
    Column { name: "armor_multiplier", kind: Kind::Float, group: Group::Stats, about: "armour against the same table" },
    Column { name: "damage_multiplier", kind: Kind::Float, group: Group::Combat, about: "melee damage against the same table" },
    Column { name: "damage_variance", kind: Kind::Float, group: Group::Combat, about: "spread between the low and high hit" },
    Column { name: "damage_school", kind: Kind::Choice(&crate::item::DAMAGE_SCHOOLS), group: Group::Combat, about: "the school its melee damage is" },
    Column { name: "base_attack_time", kind: Kind::Millis, group: Group::Combat, about: "milliseconds between melee swings" },
    Column { name: "ranged_attack_time", kind: Kind::Millis, group: Group::Combat, about: "milliseconds between ranged shots" },
    Column { name: "holy_res", kind: Kind::Signed, group: Group::Combat, about: "holy resistance" },
    Column { name: "fire_res", kind: Kind::Signed, group: Group::Combat, about: "fire resistance" },
    Column { name: "nature_res", kind: Kind::Signed, group: Group::Combat, about: "nature resistance" },
    Column { name: "frost_res", kind: Kind::Signed, group: Group::Combat, about: "frost resistance" },
    Column { name: "shadow_res", kind: Kind::Signed, group: Group::Combat, about: "shadow resistance" },
    Column { name: "arcane_res", kind: Kind::Signed, group: Group::Combat, about: "arcane resistance" },
    Column { name: "trainer_type", kind: Kind::Choice(&TRAINER_TYPES), group: Group::Services, about: "what kind of trainer it is" },
    Column { name: "trainer_spell", kind: Kind::Ref("Spell"), group: Group::Services, about: "the spell a pet or mount trainer requires" },
    Column { name: "trainer_class", kind: Kind::Choice(&TRAINER_CLASSES), group: Group::Services, about: "the class it will train, or none" },
    Column { name: "trainer_race", kind: Kind::Choice(&TRAINER_RACES), group: Group::Services, about: "the race it will train, or any" },
    Column { name: "loot_id", kind: Kind::Ref(crate::loot::CREATURE), group: Group::Loot, about: "creature_loot_template entry" },
    Column { name: "pickpocket_loot_id", kind: Kind::Ref(crate::loot::PICKPOCKETING), group: Group::Loot, about: "pickpocketing_loot_template entry" },
    Column { name: "skinning_loot_id", kind: Kind::Ref(crate::loot::SKINNING), group: Group::Loot, about: "skinning_loot_template entry" },
    Column { name: "gold_min", kind: Kind::Money, group: Group::Loot, about: "least copper on the corpse" },
    Column { name: "gold_max", kind: Kind::Money, group: Group::Loot, about: "most copper on the corpse" },
    Column { name: "spell_list_id", kind: Kind::Ref(crate::creaturespells::TABLE), group: Group::Combat, about: "creature_spells entry: what it casts, and when" },
    Column { name: "pet_spell_list_id", kind: Kind::Ref(crate::creaturespells::TABLE), group: Group::Combat, about: "what it knows when tamed" },
    Column { name: "spawn_spell_id", kind: Kind::Ref("Spell"), group: Group::Behaviour, about: "cast on spawn; unattackable until it finishes" },
    Column { name: "totem_spell_id", kind: Kind::Ref("Spell"), group: Group::Combat, about: "what a totem casts, which is the whole of what a totem does" },
    Column { name: "auras", kind: Kind::Text, group: Group::Behaviour, about: "spell ids it spawns with, space separated" },
    Column { name: "ai_name", kind: Kind::Text, group: Group::Behaviour, about: "which AI runs it, or empty" },
    Column { name: "movement_type", kind: Kind::Choice(&MOVEMENT_TYPES), group: Group::Behaviour, about: "the default a spawn inherits" },
    Column { name: "inhabit_type", kind: Kind::Flags(&INHABIT_TYPES), group: Group::Behaviour, about: "ground, water, air" },
    Column { name: "civilian", kind: Kind::Choice(&NO_YES), group: Group::Behaviour, about: "killing it costs honour" },
    Column { name: "racial_leader", kind: Kind::Choice(&NO_YES), group: Group::Behaviour, about: "a city leader, worth a bounty" },
    Column { name: "equipment_id", kind: Kind::Ref("creature_equip_template"), group: Group::Appearance, about: "creature_equip_template entry: what it holds" },
    Column { name: "trainer_id", kind: Kind::Ref(crate::trainer::TEMPLATE), group: Group::Services, about: "npc_trainer_template entry" },
    Column { name: "vendor_id", kind: Kind::Ref(crate::vendor::TEMPLATE), group: Group::Services, about: "npc_vendor_template entry" },
    Column { name: "mechanic_immune_mask", kind: Kind::Flags(&MECHANIC_MASK), group: Group::Advanced, about: "spell mechanics it ignores" },
    Column { name: "school_immune_mask", kind: Kind::Flags(&SCHOOL_MASK), group: Group::Advanced, about: "spell schools it ignores" },
    Column { name: "immunity_flags", kind: Kind::Flags(&IMMUNITY_FLAGS), group: Group::Advanced, about: "further immunities" },
    Column { name: "static_flags1", kind: Kind::Flags(&STATIC_FLAGS1), group: Group::Advanced, about: "what it is, as bits" },
    Column { name: "static_flags2", kind: Kind::Flags(&STATIC_FLAGS2), group: Group::Advanced, about: "the second word of the same" },
    Column { name: "flags_extra", kind: Kind::Flags(&FLAGS_EXTRA), group: Group::Advanced, about: "vmangos' own extra flags" },
    Column { name: "script_name", kind: Kind::Text, group: Group::Advanced, about: "a compiled script, or empty" },
];

/// `creature`'s columns, in the order the table declares them.
///
/// `id2`..`id5` are the other templates this spawn may pick from, which vmangos
/// chooses between at spawn time. They are kept in the list because a spawn
/// that uses them may be drawn with a model other than `id`'s, and a form that
/// did not show them would misstate what is standing there.
pub const SPAWN_COLUMNS: [Column; 21] = [
    Column { name: "guid", kind: Kind::Key, group: Group::Identity, about: "this spawn's own id" },
    Column { name: "id", kind: Kind::Ref(TEMPLATE), group: Group::Identity, about: "which creature stands here" },
    Column { name: "id2", kind: Kind::Ref(TEMPLATE), group: Group::Identity, about: "an alternative it may spawn as" },
    Column { name: "id3", kind: Kind::Ref(TEMPLATE), group: Group::Identity, about: "a third" },
    Column { name: "id4", kind: Kind::Ref(TEMPLATE), group: Group::Identity, about: "a fourth" },
    Column { name: "id5", kind: Kind::Ref(TEMPLATE), group: Group::Identity, about: "a fifth" },
    Column { name: "map", kind: Kind::Ref("Map"), group: Group::Place, about: "Map.dbc id" },
    Column { name: "position_x", kind: Kind::Float, group: Group::Place, about: "north, in the world's own axes" },
    Column { name: "position_y", kind: Kind::Float, group: Group::Place, about: "west" },
    Column { name: "position_z", kind: Kind::Float, group: Group::Place, about: "up" },
    Column { name: "orientation", kind: Kind::Float, group: Group::Place, about: "radians it faces, anticlockwise from north" },
    Column { name: "spawntimesecsmin", kind: Kind::Seconds, group: Group::Respawn, about: "least seconds before it comes back" },
    Column { name: "spawntimesecsmax", kind: Kind::Seconds, group: Group::Respawn, about: "most seconds before it comes back" },
    Column { name: "wander_distance", kind: Kind::Float, group: Group::Respawn, about: "yards it strays from here" },
    Column { name: "health_percent", kind: Kind::Float, group: Group::Respawn, about: "health it spawns with" },
    Column { name: "mana_percent", kind: Kind::Float, group: Group::Respawn, about: "mana it spawns with" },
    Column { name: "movement_type", kind: Kind::Choice(&MOVEMENT_TYPES), group: Group::Respawn, about: "idle, random, waypoint" },
    Column { name: "spawn_flags", kind: Kind::Flags(&SPAWN_FLAGS), group: Group::Respawn, about: "how this one spawn behaves" },
    Column { name: "visibility_mod", kind: Kind::Float, group: Group::Respawn, about: "yards added to how far it is seen" },
    Column { name: "patch_min", kind: Kind::Choice(&PATCHES), group: Group::Respawn, about: "lowest content patch this spawn exists in" },
    Column { name: "patch_max", kind: Kind::Choice(&PATCHES), group: Group::Respawn, about: "highest content patch this spawn exists in" },
];

/// Every creature spawn on one map, with the name and the first display id of
/// whichever template row the server would load.
///
/// One statement rather than a spawn query plus a template query per row: a map
/// has up to twenty-nine thousand spawns, and the join makes the read one round
/// trip.
///
/// The winning patch per entry is a derived table rather than a correlated
/// sub-select. `LoadCreatureTemplates` writes it as a sub-select per row
/// (`ObjectMgr.cpp:1191`), which the server pays once at startup and which
/// measured at over two minutes for map 0 here; grouping the whole of
/// `creature_template` by entry once and joining to that runs the same map in
/// 0.34 s. The rows it picks are the same rows.
pub fn spawns_on_map_query(map: u32, wow_patch: u32) -> String {
    format!(
        "SELECT c.`guid`, c.`id`, c.`map`, c.`position_x`, c.`position_y`, c.`position_z`,          c.`orientation`, c.`spawn_flags`, c.`wander_distance`, c.`movement_type`,          t.`name`, t.`subname`, t.`display_id1`, t.`display_scale1`, t.`level_min`,          t.`level_max`, t.`faction`, t.`npc_flags`, t.`rank`, t.`patch`          FROM `creature` c          LEFT JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM `creature_template`                     WHERE `patch` <= {wow_patch} GROUP BY `entry`) w ON w.`entry` = c.`id`          LEFT JOIN `creature_template` t ON t.`entry` = c.`id` AND t.`patch` = w.`patch`          WHERE c.`map` = {map} ORDER BY c.`guid`"
    )
}

/// The template row the server would load for one entry, with its `patch`.
///
/// The `patch` is selected rather than assumed, because it is half the key an
/// edit is written under. See the module comment.
pub fn winning_template_query(entry: u32, wow_patch: u32) -> String {
    format!(
        "SELECT * FROM `creature_template` t1 WHERE `entry` = {entry} AND `patch` = \
         (SELECT MAX(t2.`patch`) FROM `creature_template` t2 \
          WHERE t2.`entry` = t1.`entry` AND t2.`patch` <= {wow_patch})"
    )
}

/// One spawn, by guid.
pub fn spawn_query(guid: u64) -> String {
    format!("SELECT * FROM `creature` WHERE `guid` = {guid}")
}

/// The row a key names, whichever table it is in. An undo is taken from it.
pub fn row_query(table: &str, key: &Key) -> String {
    format!(
        "SELECT * FROM {} WHERE {} LIMIT 1",
        crate::sql::name(table),
        key.where_clause()
    )
}

/// The key an edit to a template row is written under.
pub fn template_key(entry: u32, patch: u32) -> Key {
    Key::two(("entry", entry as u64), ("patch", patch as u64))
}

/// The key an edit to a spawn row is written under.
pub fn spawn_key(guid: u64) -> Key {
    Key::one("guid", guid)
}

/// The first guid a spawn this project creates is numbered from.
///
/// `creature.guid` is the low 24 bits of a unit's `ObjectGuid`: vmangos packs
/// `counter | entry << 24 | HIGHGUID_UNIT << 48` (`ObjectGuid.h:129`), so the
/// whole space is [`MAX_GUID`] and not a `u32`. A guid past it shuts the server
/// down at the next spawn: `ObjectGuidGenerator::Generate` calls
/// `World::StopNow(ERROR_EXIT_CODE)`.
///
/// The reference install's highest is 303,733, and vmangos' own world database
/// updates add spawns above that, so a new spawn numbered from the top of the
/// existing rows would collide with the next update. Ten million is clear of
/// anything upstream is going to reach and inside the space.
///
/// The cost of the base: `ObjectMgr::LoadGuids` reads `MAX(guid) FROM
/// creature`, and every map's temporary creature guids (summons, pets, totems)
/// are generated from there plus the configured reserve (`ObjectMgr.cpp:7670`,
/// `Map.cpp:136`). So one spawn at ten million leaves each map 6.7 million
/// temporary guids per server run instead of 16.4 million.
pub const RESERVED_GUID_BASE: u64 = 10_000_000;

/// The highest guid the server can hold. See [`RESERVED_GUID_BASE`].
pub const MAX_GUID: u64 = 0x00FF_FFFF;

/// The first entry a template this project creates is numbered from.
///
/// The reference install's highest `creature_template.entry` is 18,199, and
/// vmangos' own world updates add entries above that. Two million is
/// [`crate::item::RESERVED_ENTRY_BASE`]'s number, chosen for the same reason:
/// clear of anything upstream will reach and inside what the column holds.
pub const RESERVED_ENTRY_BASE: u32 = 2_000_000;

/// The highest entry the column can hold: `mediumint unsigned`.
pub const MAX_ENTRY: u32 = 0x00FF_FFFF;

/// The highest entry the whole table holds, at any patch. A new template
/// numbered from the rows the server loads alone would collide with a row that
/// exists at a patch the server is not loading.
pub const MAX_ENTRY_QUERY: &str = "SELECT MAX(`entry`) AS `entry` FROM `creature_template`";

/// Every column of a new template, as the values a row created here starts
/// from.
///
/// The `entry` and the `patch` are not among them: they are the key, and
/// [`crate::row::insert`] writes a key's own columns first. Every other column
/// is named, because a column an `INSERT` leaves out takes the table's default
/// rather than a value somebody chose.
///
/// The values are the table's own defaults, except where
/// `ObjectMgr::CheckCreatureTemplate` (`ObjectMgr.cpp:1454`) corrects or
/// refuses the default:
///
/// * `faction` 35, which `FactionTemplate.dbc` resolves to faction 31,
///   Friendly, and which is the most common faction in the table (1,608 rows).
///   The default of 0 names no faction template and is logged at every start.
/// * `unit_class` 1, warrior, which 13,047 of 15,217 rows use. The check
///   refuses 0.
/// * `type` 7, humanoid, which 10,447 rows use. 0 passes the check and names
///   nothing.
/// * `display_id1` 0 and every probability 0. Until a model is chosen the
///   server logs the row at load and draws `UNIT_DISPLAY_ID_BOX`, display 4,
///   in its place (`Creature::ChooseDisplayId`, `Creature.cpp:780`). The zero
///   is kept because a model nobody chose would render as a plausible creature
///   rather than as a row that still needs one.
/// * `inhabit_type` 3, ground and water, the default; the check refuses 0.
/// * `base_attack_time` and `ranged_attack_time` 2000, the default; the check
///   corrects 0.
/// * `speed_walk` 1 and `speed_run` 1.14286, `detection_range` 18,
///   `call_for_help_range` 5, `damage_variance` 0.14, and the five multipliers
///   1, all the table's own defaults.
///
/// Everything else is zero or the empty string, which for each remaining column
/// means "none": no gossip, no loot, no spells, no trainer, no vendor, no
/// script.
pub fn new_template(name: &str) -> Vec<Assignment> {
    let mut out: Vec<(&'static str, String)> = vec![
        ("name", crate::sql::text(name)),
        ("subname", crate::sql::text("")),
        ("level_min", "1".to_string()),
        ("level_max", "1".to_string()),
        ("faction", "35".to_string()),
        ("speed_walk", "1".to_string()),
        ("speed_run", "1.14286".to_string()),
        ("detection_range", "18".to_string()),
        ("call_for_help_range", "5".to_string()),
        ("type", "7".to_string()),
        ("unit_class", "1".to_string()),
        ("xp_multiplier", "1".to_string()),
        ("health_multiplier", "1".to_string()),
        ("mana_multiplier", "1".to_string()),
        ("armor_multiplier", "1".to_string()),
        ("damage_multiplier", "1".to_string()),
        ("damage_variance", "0.14".to_string()),
        ("base_attack_time", "2000".to_string()),
        ("ranged_attack_time", "2000".to_string()),
        ("inhabit_type", "3".to_string()),
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

/// The tables a removed spawn takes with it, and the column each keys it by.
///
/// `Creature::DeleteFromDB` (`Creature.cpp:2104`) is the list, in its own
/// order. Leaving one behind is a row addressed to a guid that is not there (a
/// path with no creature, an addon with nothing to add to), and the server
/// reports each of them at its next start.
pub const DEPENDENTS: [(&str, &str); 5] = [
    ("creature_addon", "guid"),
    ("creature_movement", "id"),
    ("game_event_creature", "guid"),
    ("game_event_creature_data", "guid"),
    ("creature_battleground", "guid"),
];

/// Every column of the world database that names a creature by its template
/// entry. These follow a `creature_template` whose entry changes; see
/// [`crate::row::move_statements`]. Read off the reference install's
/// `information_schema`.
///
/// `creature.id` is every spawn of it, and `id2`..`id5` the alternatives a spawn
/// may be instead. `quest_template.ReqCreatureOrGOId` names a creature when it
/// is positive and a game object when it is not, so the positive form is the
/// only one listed. `spell_script_target.targetEntry` is a creature for types 1
/// and 2 and a game object for type 0.
///
/// Not reached: a script's `datalong` where the command happens to take a
/// creature entry, `conditions`, and the C++ scripts, which name entries as
/// constants.
pub const TEMPLATE_REFERENCES: [crate::row::Reference; 26] = {
    use crate::row::Reference as R;
    [
        R::new(SPAWN, "id"),
        R::new(SPAWN, "id2"),
        R::new(SPAWN, "id3"),
        R::new(SPAWN, "id4"),
        R::new(SPAWN, "id5"),
        R::new("creature_questrelation", "id"),
        R::new("creature_involvedrelation", "id"),
        R::new("npc_vendor", "entry"),
        R::new("npc_trainer", "entry"),
        R::new("creature_ai_events", "creature_id"),
        R::new("creature_onkill_reputation", "creature_id"),
        R::new("creature_movement_template", "entry"),
        R::new("creature_linking_template", "entry"),
        R::new("creature_linking_template", "master_entry"),
        R::new("pool_creature_template", "id"),
        R::new("locales_creature", "entry"),
        R::new("game_event_creature_data", "entry_id"),
        R::new("petcreateinfo_spell", "entry"),
        R::new("pet_levelstats", "entry"),
        R::new("script_escort_data", "creature_id"),
        R::new("script_waypoint", "entry"),
        R::only("spell_script_target", "targetEntry", "`type` IN (1, 2)"),
        R::new("quest_template", "ReqCreatureOrGOId1"),
        R::new("quest_template", "ReqCreatureOrGOId2"),
        R::new("quest_template", "ReqCreatureOrGOId3"),
        R::new("quest_template", "ReqCreatureOrGOId4"),
    ]
};

/// Every column that names a spawn by its guid: [`DEPENDENTS`], which a
/// removal takes, and the six that mention a spawn from a row about something
/// else.
pub const SPAWN_REFERENCES: [crate::row::Reference; 11] = {
    use crate::row::Reference as R;
    [
        R::new("creature_addon", "guid"),
        R::new("creature_movement", "id"),
        R::new("game_event_creature", "guid"),
        R::new("game_event_creature_data", "guid"),
        R::new("creature_battleground", "guid"),
        R::new("pool_creature", "guid"),
        R::new("creature_linking", "guid"),
        R::new("creature_linking", "master_guid"),
        R::new("creature_groups", "leader_guid"),
        R::new("creature_groups", "member_guid"),
        R::new("npc_gossip", "npc_guid"),
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

/// Every column of a new spawn, as the values a row created here starts from.
///
/// The `guid` is not among them: it is the key, and [`crate::row::insert`]
/// writes a key's own columns first. Everything else the table has is named,
/// because a column an `INSERT` leaves out takes the column's default rather
/// than a value somebody chose, and three of those defaults are wrong for a
/// spawn placed by hand.
///
/// The values are vmangos' own, from the `creature` DDL and from what
/// `ObjectMgr::LoadCreatures` refuses:
///
/// * `movement_type` 0 and `wander_distance` 0 together. The DDL's wander
///   default is 5, and idle movement with a wander distance is one of the four
///   rows `LoadCreatures` logs and corrects at every start.
/// * `health_percent` and `mana_percent` 100. Anything less is logged unless
///   the template carries `NO_AUTOMATIC_REGEN`.
/// * `patch_min` 0 and `patch_max` 10, the DDL's own, which is every content
///   patch a 1.12 server runs at. A spawn outside the server's `WowPatch` band
///   loads with `SPAWN_FLAG_DISABLED` and stands nowhere.
/// * `spawntimesecsmin` and `spawntimesecsmax` 120, the DDL's default.
///   vmangos' own `.npc add` writes 25, which is `Creature`'s in-memory
///   default rather than the table's.
pub fn new_spawn(entry: u32, map: u32, at: (f32, f32, f32), orientation: f32) -> Vec<Assignment> {
    let float = |value: f32| crate::sql::float(value);
    let columns: Vec<(&'static str, String)> = vec![
        ("id", entry.to_string()),
        ("id2", "0".to_string()),
        ("id3", "0".to_string()),
        ("id4", "0".to_string()),
        ("id5", "0".to_string()),
        ("map", map.to_string()),
        ("position_x", float(at.0)),
        ("position_y", float(at.1)),
        ("position_z", float(at.2)),
        ("orientation", float(orientation)),
        ("spawntimesecsmin", "120".to_string()),
        ("spawntimesecsmax", "120".to_string()),
        ("wander_distance", "0".to_string()),
        ("health_percent", "100".to_string()),
        ("mana_percent", "100".to_string()),
        ("movement_type", "0".to_string()),
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

/// Whether a row is already there. An apply asks this before it creates one.
///
/// A guid this project allocated can still be taken, by another project, by a
/// GM's `.npc add`, or by an upstream update applied since the spawn was
/// placed, and an `INSERT` onto an occupied key fails the whole apply. Asking
/// first lets the apply report the occupied guid instead.
pub fn exists_query(table: &str, key: &Key) -> String {
    format!(
        "SELECT 1 FROM {} WHERE {} LIMIT 1",
        crate::sql::name(table),
        key.where_clause()
    )
}

/// The highest guid the `creature` table holds. A new guid is numbered from it
/// when the reserved base is already in use.
pub const MAX_SPAWN_GUID_QUERY: &str = "SELECT MAX(`guid`) AS `guid` FROM `creature`";

/// The template columns a marker, a model and a heading need, as the `SELECT`
/// two queries share.
///
/// These are the same columns [`spawns_on_map_query`] carries for every spawn
/// on the map, so a creature read this way and one read from the map carry the
/// same fields.
const BRIEF: &str = "t.`entry`, t.`patch`, t.`name`, t.`subname`, t.`display_id1`, \
     t.`display_scale1`, t.`level_min`, t.`level_max`, t.`faction`, t.`npc_flags`, t.`rank`";

/// The join that picks the row the server would load, shared by both queries.
fn winning_join(wow_patch: u32) -> String {
    format!(
        "FROM `creature_template` t \
         JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM `creature_template` \
               WHERE `patch` <= {wow_patch} GROUP BY `entry`) w \
           ON w.`entry` = t.`entry` AND w.`patch` = t.`patch`"
    )
}

/// Named templates, by entry. A caller reads this when it holds a spawn of a
/// creature nothing else on the map is a spawn of.
///
/// `None` for an empty list rather than a statement with an empty `IN ()`,
/// which MySQL refuses.
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

/// Creatures whose name or entry matches what was typed, with the template
/// row the server would load for each.
///
/// This is the picker behind New spawn: a creature is placed by entry, and a
/// person usually knows the name and not the entry. `term` is matched against
/// the name with `LIKE` and against the entry exactly, so either works in one
/// box.
///
/// The winning patch is a derived table for [`spawns_on_map_query`]'s reason.
pub fn search_query(term: &str, wow_patch: u32, limit: usize) -> String {
    let like = crate::sql::text(&format!("%{term}%"));
    // 0 is no creature, so a term that is not a number matches nothing by
    // entry and the `LIKE` is the whole of the search.
    let exact = term.trim().parse::<u32>().unwrap_or(0);
    format!(
        "SELECT {BRIEF} {} WHERE t.`name` LIKE {like} OR t.`entry` = {exact} \
         ORDER BY t.`entry` = {exact} DESC, LENGTH(t.`name`), t.`entry` LIMIT {limit}",
        winning_join(wow_patch)
    )
}

/// The columns of whichever of the two tables, or an empty slice for a name
/// this module does not know.
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

/// The statements a save emits for one row this project claims.
///
/// One `UPDATE` for a row it edits, a `DELETE` and an `INSERT` for a row it
/// creates, and six `DELETE`s for a spawn it removes: the row and the five
/// tables keyed by its guid. See [`Life`] and [`DEPENDENTS`].
///
/// Only a spawn can be removed. `creature_template` cannot:
/// `.reload creature_template` adds and overwrites and never drops an entry it
/// has already read, so a removal cannot be made live at all. A life this
/// table cannot carry produces no statement, and [`can_live`] is what a caller
/// asks before it offers one.
pub fn statements(table: &str, key: &Key, life: Life, changes: &[Assignment]) -> Vec<String> {
    match life {
        Life::Update => crate::row::update(table, key, changes).into_iter().collect(),
        // A `DELETE` of the key, then the `INSERT`. `Creature::SaveToDB` writes
        // the same pair (`Creature.cpp:1646`), for the same reason: applying
        // twice then means the same as applying once. A bare `INSERT` on the
        // second apply fails the whole transaction with a duplicate-key error.
        //
        // The pair is safe only because the caller has already established
        // that the row is this project's; see the editor's `apply`, which
        // refuses a guid that is in the database and not in the project's own
        // revert file. Without that check this pair would delete somebody
        // else's row.
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

/// Whether this table can hold a row that is created or removed at all.
///
/// Both tables can hold a created row. Only the spawn table can hold a removed
/// one; see [`statements`] for the reason.
pub fn can_live(table: &str, life: Life) -> bool {
    match life {
        Life::Update => true,
        Life::Insert => table == SPAWN || table == TEMPLATE,
        Life::Delete => table == SPAWN,
    }
}

/// The statements that take back a row this project created, once its
/// `INSERT` has run.
///
/// A spawn goes with the five tables keyed by its guid, which is what its
/// `INSERT` and its later edits could have added. A template is one row: no
/// table is keyed by a template's entry the way [`DEPENDENTS`] are keyed by a
/// guid, and a row that names the entry, a spawn or a vendor line, is the
/// project's own claim with its own undo. A table that cannot hold a created
/// row answers nothing.
pub fn insert_undo_statements(table: &str, key: &Key) -> Vec<String> {
    match table {
        SPAWN => delete_statements(key),
        TEMPLATE => vec![crate::row::delete(TEMPLATE, key)],
        _ => Vec::new(),
    }
}

/// The statements that remove a spawn whole: the row and the five tables keyed
/// by its guid.
///
/// `Creature::DeleteFromDB` (`Creature.cpp:2104`) in its own order, with the
/// spawn last, so that a run that stops half way leaves the orphans rather than
/// the creature. A creature with no addon stands; an addon with no creature is
/// a row the server reports at every start and nothing points at.
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

    /// A new spawn names every column the table has but its key.
    ///
    /// A column an `INSERT` leaves out takes the column's default, and three of
    /// this table's defaults are wrong for a spawn placed by hand, so a column
    /// added to the schema and not to [`new_spawn`] would take a value nobody
    /// chose, with no error.
    #[test]
    fn a_new_spawn_names_every_column_but_the_key() {
        let row = new_spawn(68, 0, (-8817.5, 809.0, 98.7), 1.5);
        let named: Vec<&str> = row.iter().map(|change| change.column).collect();
        for column in SPAWN_COLUMNS.iter().filter(|column| column.editable()) {
            assert!(named.contains(&column.name), "{} is not written", column.name);
        }
        assert!(!named.contains(&"guid"), "the key is the key, not a column");
        assert_eq!(named.len(), SPAWN_COLUMNS.len() - 1);
    }

    /// Idle movement and a wander distance together is one of the four rows
    /// `LoadCreatures` logs and corrects at every start, and the table's own
    /// default for `wander_distance` is 5. So the default cannot be taken.
    #[test]
    fn a_new_spawn_does_not_wander_while_it_stands_still() {
        let row = new_spawn(68, 0, (0.0, 0.0, 0.0), 0.0);
        let value = |name: &str| {
            row.iter()
                .find(|change| change.column == name)
                .map(|change| change.value.as_str())
        };
        assert_eq!(value("movement_type"), Some("0"));
        assert_eq!(value("wander_distance"), Some("0"));
        assert_eq!(value("health_percent"), Some("100"));
        assert_eq!(value("mana_percent"), Some("100"));
        // Outside the server's patch band a spawn loads disabled and stands
        // nowhere, which is a spawn that applied cleanly and did nothing.
        assert_eq!(value("patch_min"), Some("0"));
        assert_eq!(value("patch_max"), Some("10"));
    }

    /// Removing a spawn removes the five tables keyed by its guid too, and the
    /// row itself goes last: a creature with no addon stands, and an addon with
    /// no creature is a row the server reports at every start.
    #[test]
    fn removing_a_spawn_takes_its_five_dependents_with_it() {
        let sql = delete_statements(&spawn_key(19272));
        assert_eq!(sql.len(), DEPENDENTS.len() + 1);
        assert_eq!(
            sql[1],
            "DELETE FROM `creature_movement` WHERE `id` = 19272;",
            "the path is keyed by `id` and not by `guid`"
        );
        assert_eq!(
            sql.last().map(String::as_str),
            Some("DELETE FROM `creature` WHERE `guid` = 19272;")
        );
    }

    /// A template can be created and not removed, and the asymmetry is written
    /// down once, in [`can_live`]. A removal statement for one would be a
    /// change that cannot be made live on the server at all.
    #[test]
    fn a_template_can_be_created_and_not_removed() {
        let key = template_key(68, 0);
        assert!(can_live(TEMPLATE, Life::Insert));
        assert!(!can_live(TEMPLATE, Life::Delete));
        assert!(statements(TEMPLATE, &key, Life::Delete, &[]).is_empty());
        for life in [Life::Insert, Life::Delete] {
            assert!(can_live(SPAWN, life));
        }
        // An ordinary edit is the same on both.
        let change = [Assignment { column: "level_min", value: "57".into() }];
        assert_eq!(statements(TEMPLATE, &key, Life::Update, &change).len(), 1);
        // A creation is the `DELETE` and `INSERT` pair, keyed by both columns.
        let made = template_key(RESERVED_ENTRY_BASE, 10);
        let sql = statements(TEMPLATE, &made, Life::Insert, &new_template("Test Subject"));
        assert_eq!(sql.len(), 2);
        assert_eq!(
            sql[0],
            "DELETE FROM `creature_template` WHERE `entry` = 2000000 AND `patch` = 10;"
        );
        assert!(sql[1].starts_with("INSERT INTO `creature_template` (`entry`, `patch`, "));
        assert!(sql[1].contains("'Test Subject'"));
    }

    /// A new template names every column the table has but its key. A column
    /// left out of the `INSERT` takes the table's default, which for
    /// `unit_class` and `faction` is a value `CheckCreatureTemplate` logs at
    /// every start.
    #[test]
    fn a_new_template_names_every_column_but_the_key() {
        let row = new_template("Test Subject");
        let named: Vec<&str> = row.iter().map(|change| change.column).collect();
        for column in TEMPLATE_COLUMNS.iter().filter(|column| column.editable()) {
            assert!(named.contains(&column.name), "{} is not written", column.name);
        }
        assert!(!named.contains(&"entry"), "the key is the key, not a column");
        assert_eq!(named.len(), TEMPLATE_COLUMNS.len() - 1);
        let value = |name: &str| {
            row.iter()
                .find(|change| change.column == name)
                .map(|change| change.value.as_str())
        };
        assert_eq!(value("name"), Some("'Test Subject'"));
        assert_eq!(value("faction"), Some("35"));
        assert_eq!(value("unit_class"), Some("1"));
        assert_eq!(value("type"), Some("7"));
        assert_eq!(value("inhabit_type"), Some("3"));
        assert_eq!(value("base_attack_time"), Some("2000"));
        assert_eq!(value("display_id1"), Some("0"));
        assert_eq!(value("display_total_probability"), Some("0"));
        assert_eq!(value("script_name"), Some("''"));
    }

    /// A created template is undone by one `DELETE` of its key, and a created
    /// spawn by the six statements a removal comes to.
    #[test]
    fn a_created_rows_undo_depends_on_its_table() {
        let template = insert_undo_statements(TEMPLATE, &template_key(2_000_000, 10));
        assert_eq!(
            template,
            vec!["DELETE FROM `creature_template` WHERE `entry` = 2000000 AND `patch` = 10;"]
        );
        let spawn = insert_undo_statements(SPAWN, &spawn_key(10_000_000));
        assert_eq!(spawn.len(), DEPENDENTS.len() + 1);
        assert!(insert_undo_statements("npc_vendor", &Key::one("entry", 1)).is_empty());
    }

    /// The reserved entry base is inside the column and above the reference
    /// install's highest entry, 18,199.
    #[test]
    fn a_reserved_entry_is_one_the_column_can_hold() {
        assert!(RESERVED_ENTRY_BASE < MAX_ENTRY);
        assert!(RESERVED_ENTRY_BASE > 18_199);
        assert_eq!(MAX_ENTRY, 16_777_215);
        assert_eq!(RESERVED_ENTRY_BASE, crate::item::RESERVED_ENTRY_BASE);
    }

    /// The reserved guid base is inside what a unit guid can hold.
    ///
    /// `creature.guid` is the low 24 bits of an `ObjectGuid`, so a guid past
    /// `MAX_GUID` makes `ObjectGuidGenerator::Generate` shut the server down.
    /// The column is declared `int(10)`, but the space is 24 bits and not a
    /// `u32`.
    #[test]
    fn a_reserved_guid_is_one_the_server_can_hold() {
        assert!(RESERVED_GUID_BASE < MAX_GUID);
        assert_eq!(MAX_GUID, 16_777_215);
        // It is also clear of the reference install's own highest, 303,733, by
        // enough that an upstream update cannot reach it.
        assert!(RESERVED_GUID_BASE > 1_000_000);
    }

    /// The search takes a name and an entry in the same box, because both are
    /// ways somebody knows a creature. The name is escaped once, by the rule
    /// every other value here is escaped by.
    #[test]
    fn the_search_matches_a_name_or_an_entry() {
        let sql = search_query("guard", 10, 50);
        assert!(sql.contains("LIKE '%guard%'"), "{sql}");
        assert!(sql.contains("t.`entry` = 0"), "{sql}");
        assert!(search_query("68", 10, 50).contains("t.`entry` = 68"));
        // A name that tries to close the literal and add a statement cannot.
        assert!(search_query("x'; DROP TABLE `creature`; --", 10, 50)
            .contains("'%x\\'; DROP TABLE `creature`; --%'"));
    }

    /// The template schema is the server's own `SELECT`, in order.
    ///
    /// Transcribed from `ObjectMgr.cpp:1191`. A column in the wrong place is
    /// the failure this schema can have: the values would go into the wrong
    /// columns on a save. Keeping the list in the source's order makes the two
    /// comparable by eye when vmangos adds one.
    #[test]
    fn the_template_columns_are_the_servers_own_select() {
        const SELECTED: &str = "entry name subname level_min level_max faction npc_flags \
            gossip_menu_id display_id1 display_id2 display_id3 display_id4 display_scale1 \
            display_scale2 display_scale3 display_scale4 display_probability1 \
            display_probability2 display_probability3 display_probability4 \
            display_total_probability mount_display_id speed_walk speed_run detection_range \
            call_for_help_range leash_range type pet_family rank unit_class xp_multiplier \
            health_multiplier mana_multiplier armor_multiplier damage_multiplier \
            damage_variance damage_school base_attack_time ranged_attack_time holy_res \
            fire_res nature_res frost_res shadow_res arcane_res trainer_type trainer_spell \
            trainer_class trainer_race loot_id pickpocket_loot_id skinning_loot_id gold_min \
            gold_max spell_list_id pet_spell_list_id spawn_spell_id totem_spell_id \
            auras ai_name movement_type inhabit_type civilian racial_leader \
            equipment_id trainer_id vendor_id mechanic_immune_mask school_immune_mask \
            immunity_flags static_flags1 static_flags2 flags_extra script_name";
        let want: Vec<&str> = SELECTED.split_whitespace().collect();
        let have: Vec<&str> = TEMPLATE_COLUMNS.iter().map(|column| column.name).collect();
        assert_eq!(have, want);
    }

    /// No column is named twice in either table. A hand-written schema of 78
    /// entries can repeat a name without any other check noticing.
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

    /// The key columns are the table's own primary key, which is what an
    /// `UPDATE` has to name to reach one row.
    #[test]
    fn the_key_columns_are_the_primary_key() {
        let template: Vec<&str> = TEMPLATE_COLUMNS
            .iter()
            .filter(|column| column.kind == Kind::Key)
            .map(|column| column.name)
            .collect();
        // `patch` is the other half of it and is not in the server's SELECT, so
        // it is not in the column list either; it arrives through the key.
        assert_eq!(template, ["entry"]);
        assert_eq!(template_key(3296, 0).where_clause(), "`entry` = 3296 AND `patch` = 0");

        let spawn: Vec<&str> = SPAWN_COLUMNS
            .iter()
            .filter(|column| column.kind == Kind::Key)
            .map(|column| column.name)
            .collect();
        assert_eq!(spawn, ["guid"]);
    }

    /// A key column is never written, which is what stops an edit from moving
    /// the row it is about.
    #[test]
    fn a_key_column_is_not_editable() {
        assert!(!column(TEMPLATE, "entry").expect("entry").editable());
        assert!(column(TEMPLATE, "name").expect("name").editable());
        assert!(!column(SPAWN, "guid").expect("guid").editable());
        assert!(column(SPAWN, "position_x").expect("position_x").editable());
    }

    /// A named mask is one bit per row and no repeats. A hand transcription can
    /// put two names on one bit, which would draw two checkboxes that set the
    /// same thing.
    #[test]
    fn a_named_mask_is_one_bit_per_row_and_no_repeats() {
        for (what, bits) in [
            ("npc_flags", &NPC_FLAGS[..]),
            ("inhabit_type", &INHABIT_TYPES[..]),
            ("static_flags1", &STATIC_FLAGS1[..]),
            ("spawn_flags", &SPAWN_FLAGS[..]),
        ] {
            let mut seen = 0u32;
            for bit in bits {
                assert_eq!(bit.bit.count_ones(), 1, "{what}: {} is not one bit", bit.name);
                assert_eq!(seen & bit.bit, 0, "{what}: {} repeats a bit", bit.name);
                seen |= bit.bit;
            }
        }
    }

    /// A named enumeration has no value twice.
    #[test]
    fn a_named_enumeration_has_no_value_twice() {
        for values in [&CREATURE_TYPES[..], &RANKS[..], &UNIT_CLASSES[..], &MOVEMENT_TYPES[..]] {
            let mut seen = std::collections::HashSet::new();
            for named in values {
                assert!(seen.insert(named.value), "{} repeats a value", named.name);
            }
        }
    }

    #[test]
    fn a_mask_says_what_is_set_and_what_is_not_named() {
        assert_eq!(mask_words(&NPC_FLAGS, 0), "none");
        assert_eq!(mask_words(&NPC_FLAGS, 3), "GOSSIP, QUESTGIVER");
        // A bit past the fifteen the list has.
        assert_eq!(mask_words(&NPC_FLAGS, 0x10000), "+0x00010000 unnamed");
        assert_eq!(value_word(&RANKS, 3), "World boss");
        assert_eq!(value_word(&RANKS, 9), "9");
    }

    /// The spawn query names the patch sub-select, which is what makes the
    /// name and the model the ones the server is using rather than whichever
    /// patch row the join happened to reach first.
    #[test]
    fn the_spawn_query_joins_the_template_the_server_would_load() {
        let sql = spawns_on_map_query(0, 10);
        assert!(sql.contains("MAX(`patch`)"), "{sql}");
        assert!(sql.contains("`patch` <= 10"), "{sql}");
        assert!(sql.contains("c.`map` = 0"), "{sql}");
        // A LEFT join: a spawn whose template row is missing is a spawn worth
        // seeing, and an inner join would silently drop it.
        assert!(sql.contains("LEFT JOIN"), "{sql}");
    }

    #[test]
    fn a_row_is_read_by_column_name() {
        let mut row = Row::new();
        row.insert("map".into(), Some("0".into()));
        row.insert("position_z".into(), Some("41.5".into()));
        row.insert("subname".into(), None);
        assert_eq!(row.integer("map"), Some(0));
        assert_eq!(row.number("position_z"), Some(41.5));
        // A float column asked for as an integer goes through the float.
        assert_eq!(row.integer("position_z"), Some(41));
        assert_eq!(row.text("subname"), None);
        assert_eq!(row.text("nothing"), None);
    }

    /// A text column's value is quoted on the way into a statement and a
    /// numeric one is not.
    #[test]
    fn a_columns_literal_is_quoted_only_when_it_is_text() {
        let mut row = Row::new();
        row.insert("name".into(), Some("Kobold Vermin".into()));
        row.insert("level_min".into(), Some("1".into()));
        assert_eq!(
            column(TEMPLATE, "name").expect("name").literal(&row),
            Some("'Kobold Vermin'".to_string())
        );
        assert_eq!(
            column(TEMPLATE, "level_min").expect("level_min").literal(&row),
            Some("1".to_string())
        );
    }

    #[test]
    fn a_table_name_from_a_file_resolves_to_a_constant_or_to_nothing() {
        assert_eq!(table_named("creature_template"), Some(TEMPLATE));
        assert_eq!(table_named("creature"), Some(SPAWN));
        assert_eq!(table_named("creature_template; DROP TABLE x"), None);
        assert_eq!(table_named(""), None);
    }
}
