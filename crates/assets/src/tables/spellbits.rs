//! What a spell's masks and enumerations are called.
//!
//! Names and values follow vmangos' enums.
//!
//! **1.12 ships no table for any of these.** A spell's `Attributes` is a
//! `u32` in `Spell.dbc` and there is nothing in the archives to resolve it
//! against — so, as with the effect and aura names, the server's enums are
//! the only authority. Each entry is `(mask, name, note)`; the note is an
//! optional description of the bit, empty where there is none.
//!
//! A name for a bit no 1.12 spell sets costs nothing. A bit with no name
//! is a real gap and the editor draws it as its number.

/// `Spell.dbc`'s `Attributes`.
///
/// From vmangos' `Spells/SpellDefines.h`'s `SpellAttributes`.
pub const ATTRIBUTES: [(u32, &str, &str); 32] = [
    (0x00000001, "PROC_FAILURE_BURNS_CHARGE", ""),
    (0x00000002, "USES_RANGED_SLOT", ""),
    (0x00000004, "ON_NEXT_SWING_NO_DAMAGE", ""),
    (0x00000008, "NEED_EXOTIC_AMMO", ""),
    (0x00000010, "IS_ABILITY", ""),
    (0x00000020, "IS_TRADESKILL", ""),
    (0x00000040, "PASSIVE", ""),
    (0x00000080, "DO_NOT_DISPLAY", ""),
    (0x00000100, "DO_NOT_LOG", ""),
    (0x00000200, "HELD_ITEM_ONLY", ""),
    (0x00000400, "ON_NEXT_SWING", ""),
    (0x00000800, "WEARER_CASTS_PROC_TRIGGER", ""),
    (0x00001000, "DAYTIME_ONLY", ""),
    (0x00002000, "NIGHT_ONLY", ""),
    (0x00004000, "ONLY_INDOORS", ""),
    (0x00008000, "ONLY_OUTDOORS", ""),
    (0x00010000, "NOT_SHAPESHIFT", ""),
    (0x00020000, "ONLY_STEALTHED", ""),
    (0x00040000, "DO_NOT_SHEATH", ""),
    (0x00080000, "SCALES_WITH_CREATURE_LEVEL", ""),
    (0x00100000, "CANCELS_AUTO_ATTACK_COMBAT", ""),
    (0x00200000, "NO_ACTIVE_DEFENSE", ""),
    (0x00400000, "TRACK_TARGET_IN_CAST_PLAYER_ONLY", ""),
    (0x00800000, "ALLOW_CAST_WHILE_DEAD", ""),
    (0x01000000, "ALLOW_WHILE_MOUNTED", ""),
    (0x02000000, "COOLDOWN_ON_EVENT", ""),
    (0x04000000, "AURA_IS_DEBUFF", ""),
    (0x08000000, "ALLOW_WHILE_SITTING", ""),
    (0x10000000, "NOT_IN_COMBAT_ONLY_PEACEFUL", ""),
    (0x20000000, "NO_IMMUNITIES", ""),
    (0x40000000, "HEARTBEAT_RESIST", ""),
    (0x80000000, "NO_AURA_CANCEL", ""),
];

/// …and `AttributesEx`.
///
/// From vmangos' `Spells/SpellDefines.h`'s `SpellAttributesEx`.
pub const ATTRIBUTES_EX: [(u32, &str, &str); 32] = [
    (0x00000001, "DISMISS_PET_FIRST", ""),
    (0x00000002, "USE_ALL_MANA", ""),
    (0x00000004, "IS_CHANNELED", ""),
    (0x00000008, "NO_REDIRECTION", ""),
    (0x00000010, "NO_SKILL_INCREASE", ""),
    (0x00000020, "ALLOW_WHILE_STEALTHED", ""),
    (0x00000040, "IS_SELF_CHANNELED", ""),
    (0x00000080, "NO_REFLECTION", ""),
    (0x00000100, "ONLY_PEACEFUL_TARGETS", ""),
    (0x00000200, "INITIATES_COMBAT", ""),
    (0x00000400, "NO_THREAT", ""),
    (0x00000800, "AURA_UNIQUE", ""),
    (0x00001000, "FAILURE_BREAKS_STEALTH", ""),
    (0x00002000, "TOGGLE_FARSIGHT", ""),
    (0x00004000, "TRACK_TARGET_IN_CHANNEL", ""),
    (0x00008000, "IMMUNITY_PURGES_EFFECT", ""),
    (0x00010000, "IMMUNITY_TO_HOSTILE_AND_FRIENDLY_EFFECTS", ""),
    (0x00020000, "NO_AUTOCAST_AI", ""),
    (0x00040000, "PREVENTS_ANIM", ""),
    (0x00080000, "EXCLUDE_CASTER", ""),
    (0x00100000, "FINISHING_MOVE_DAMAGE", ""),
    (0x00200000, "THREAT_ONLY_ON_MISS", ""),
    (0x00400000, "FINISHING_MOVE_DURATION", ""),
    (0x00800000, "IGNORE_CASTER_AND_TARGET_RESTRICTIONS", ""),
    (0x01000000, "SPECIAL_SKILLUP", ""),
    (0x02000000, "UNK25", ""),
    (0x04000000, "REQUIRE_ALL_TARGETS", ""),
    (0x08000000, "DISCOUNT_POWER_ON_MISS", ""),
    (0x10000000, "NO_AURA_ICON", ""),
    (0x20000000, "NAME_IN_CHANNEL_BAR", ""),
    (0x40000000, "COMBO_ON_BLOCK", ""),
    (0x80000000, "CAST_WHEN_LEARNED", ""),
];

/// …`AttributesEx2`.
///
/// From vmangos' `Spells/SpellDefines.h`'s `SpellAttributesEx2`.
pub const ATTRIBUTES_EX2: [(u32, &str, &str); 32] = [
    (0x00000001, "ALLOW_DEAD_TARGET", ""),
    (0x00000002, "NO_SHAPESHIFT_UI", ""),
    (0x00000004, "IGNORE_LINE_OF_SIGHT", ""),
    (0x00000008, "ALLOW_LOW_LEVEL_BUFF", ""),
    (0x00000010, "USE_SHAPESHIFT_BAR", ""),
    (0x00000020, "AUTO_REPEAT", ""),
    (0x00000040, "CANNOT_CAST_ON_TAPPED", ""),
    (0x00000080, "DO_NOT_REPORT_SPELL_FAILURE", ""),
    (0x00000100, "UNK8", ""),
    (0x00000200, "UNK9", ""),
    (0x00000400, "SPECIAL_TAMING_FLAG", ""),
    (0x00000800, "NO_TARGET_PER_SECOND_COSTS", ""),
    (0x00001000, "CHAIN_FROM_CASTER", ""),
    (0x00002000, "ENCHANT_OWN_ITEM_ONLY", ""),
    (0x00004000, "ALLOW_WHILE_INVISIBLE", ""),
    (0x00008000, "ENABLE_AFTER_PARRY", ""),
    (0x00010000, "NO_ACTIVE_PETS", ""),
    (0x00020000, "DO_NOT_RESET_COMBAT_TIMERS", ""),
    (0x00040000, "REQ_DEAD_PET", ""),
    (0x00080000, "ALLOW_WHILE_NOT_SHAPESHIFTED", ""),
    (0x00100000, "INITIATE_COMBAT_POST_CAST", ""),
    (0x00200000, "FAIL_ON_ALL_TARGETS_IMMUNE", ""),
    (0x00400000, "NO_INITIAL_THREAT", ""),
    (0x00800000, "PROC_COOLDOWN_ON_FAILURE", ""),
    (0x01000000, "ITEM_CAST_WITH_OWNER_SKILL", ""),
    (0x02000000, "DONT_BLOCK_MANA_REGEN", ""),
    (0x04000000, "NO_SCHOOL_IMMUNITIES", ""),
    (0x08000000, "IGNORE_WEAPONSKILL", ""),
    (0x10000000, "NOT_AN_ACTION", ""),
    (0x20000000, "CANT_CRIT", ""),
    (0x40000000, "ACTIVE_THREAT", ""),
    (0x80000000, "RETAIN_ITEM_CAST", ""),
];

/// …`AttributesEx3`.
///
/// From vmangos' `Spells/SpellDefines.h`'s `SpellAttributesEx3`.
pub const ATTRIBUTES_EX3: [(u32, &str, &str); 32] = [
    (0x00000001, "PVP_ENABLING", ""),
    (0x00000002, "NO_PROC_EQUIP_REQUIREMENT", ""),
    (0x00000004, "NO_CASTING_BAR_TEXT", ""),
    (0x00000008, "COMPLETELY_BLOCKED", ""),
    (0x00000010, "NO_RES_TIMER", ""),
    (0x00000020, "NO_DURABILITY_LOSS", ""),
    (0x00000040, "NO_AVOIDANCE", ""),
    (0x00000080, "DOT_STACKING_RULE", ""),
    (0x00000100, "ONLY_ON_PLAYER", ""),
    (0x00000200, "NOT_A_PROC", ""),
    (0x00000400, "REQUIRES_MAIN_HAND_WEAPON", ""),
    (0x00000800, "ONLY_BATTLEGROUNDS", ""),
    (0x00001000, "ONLY_ON_GHOSTS", ""),
    (0x00002000, "HIDE_CHANNEL_BAR", ""),
    (0x00004000, "HIDE_IN_RAID_FILTER", ""),
    (0x00008000, "NORMAL_RANGED_ATTACK", ""),
    (0x00010000, "SUPPRESS_CASTER_PROCS", ""),
    (0x00020000, "SUPPRESS_TARGET_PROCS", ""),
    (0x00040000, "ALWAYS_HIT", ""),
    (0x00080000, "INSTANT_TARGET_PROCS", ""),
    (0x00100000, "ALLOW_AURA_WHILE_DEAD", ""),
    (0x00200000, "ONLY_PROC_OUTDOORS", ""),
    (0x00400000, "CASTING_CANCELS_AUTOREPEAT", ""),
    (0x00800000, "NO_DAMAGE_HISTORY", ""),
    (0x01000000, "REQUIRES_OFFHAND_WEAPON", ""),
    (0x02000000, "TREAT_AS_PERIODIC", ""),
    (0x04000000, "CAN_PROC_FROM_PROCS", ""),
    (0x08000000, "ONLY_PROC_ON_CASTER", ""),
    (0x10000000, "IGNORE_CASTER_AND_TARGET_RESTRICTIONS", ""),
    (0x20000000, "IGNORE_CASTER_MODIFIERS", ""),
    (0x40000000, "DO_NOT_DISPLAY_RANGE", ""),
    (0x80000000, "NOT_ON_AOE_IMMUNE", ""),
];

/// …and `AttributesEx4`, the last one 1.12 has.
///
/// From vmangos' `Spells/SpellDefines.h`'s `SpellAttributesEx4`.
pub const ATTRIBUTES_EX4: [(u32, &str, &str); 10] = [
    (0x00000001, "IGNORE_RESISTANCES", ""),
    (0x00000002, "CLASS_TRIGGER_ONLY_ON_TARGET", ""),
    (0x00000004, "AURA_EXPIRES_OFFLINE", ""),
    (0x00000008, "NO_HELPFUL_THREAT", ""),
    (0x00000010, "NO_HARMFUL_THREAT", ""),
    (0x00000020, "ALLOW_CLIENT_TARGETING", ""),
    (0x00000040, "CANNOT_BE_STOLEN", ""),
    (0x00000080, "CAN_CAST_WHILE_CASTING", ""),
    (0x00000100, "IGNORE_DAMAGE_TAKEN_MODIFIERS", ""),
    (0x00000200, "COMBAT_FEEDBACK_WHEN_USABLE", ""),
];

/// `Targets`: what a cast may be aimed at, and what the cast packet carries.
///
/// From vmangos' `Database/DBCEnums.h`'s `SpellCastTargetFlags`.
pub const TARGET_FLAGS: [(u32, &str, &str); 17] = [
    (0x00000001, "UNUSED1", ""),
    (0x00000002, "UNIT", ""),
    (0x00000004, "UNUSED2", ""),
    (0x00000008, "UNUSED3", ""),
    (0x00000010, "ITEM", ""),
    (0x00000020, "SOURCE_LOCATION", ""),
    (0x00000040, "DEST_LOCATION", ""),
    (0x00000080, "OBJECT_UNK", ""),
    (0x00000100, "UNIT_UNK", ""),
    (0x00000200, "PVP_CORPSE", ""),
    (0x00000400, "UNIT_CORPSE", ""),
    (0x00000800, "OBJECT", ""),
    (0x00001000, "TRADE_ITEM", ""),
    (0x00002000, "STRING", ""),
    (0x00004000, "UNK1", ""),
    (0x00008000, "CORPSE", ""),
    (0x00010000, "UNK2", ""),
];

/// `InterruptFlags`: what stops the cast.
///
/// From vmangos' `Spells/SpellDefines.h`'s `SpellInterruptFlags`.
pub const INTERRUPT_FLAGS: [(u32, &str, &str); 5] = [
    (0x00000001, "MOVEMENT", ""),
    (0x00000002, "DAMAGE_PUSHBACK", ""),
    (0x00000004, "STUN", ""),
    (0x00000008, "COMBAT", ""),
    (0x00000010, "DAMAGE_CANCELS", ""),
];

/// `AuraInterruptFlags`: what takes the aura off.
///
/// From vmangos' `Spells/SpellDefines.h`'s `SpellAuraInterruptFlags`.
pub const AURA_INTERRUPT_FLAGS: [(u32, &str, &str); 23] = [
    (0x00000001, "HOSTILE_ACTION_RECEIVED_CANCELS", ""),
    (0x00000002, "DAMAGE_CANCELS", ""),
    (0x00000004, "ACTION_CANCELS", ""),
    (0x00000008, "MOVING_CANCELS", ""),
    (0x00000010, "TURNING_CANCELS", ""),
    (0x00000020, "ANIM_CANCELS", ""),
    (0x00000040, "DISMOUNT_CANCELS", ""),
    (0x00000080, "UNDER_WATER_CANCELS", ""),
    (0x00000100, "ABOVE_WATER_CANCELS", ""),
    (0x00000200, "SHEATHING_CANCELS", ""),
    (0x00000400, "INTERACTING_CANCELS", ""),
    (0x00000800, "LOOTING_CANCELS", ""),
    (0x00001000, "ATTACKING_CANCELS", ""),
    (0x00002000, "ITEM_USE_CANCELS", ""),
    (0x00004000, "DAMAGE_CHANNEL_DURATION", ""),
    (0x00008000, "SHAPESHIFTING_CANCELS", ""),
    (0x00010000, "ACTION_CANCELS_LATE", ""),
    (0x00020000, "MOUNT_CANCELS", ""),
    (0x00040000, "STANDING_CANCELS", ""),
    (0x00080000, "LEAVE_WORLD_CANCELS", ""),
    (0x00100000, "STEALTH_INVIS_CANCELS", ""),
    (0x00200000, "INVULNERABILITY_BUFF_CANCELS", ""),
    (0x00400000, "ENTER_WORLD_CANCELS", ""),
];

/// `ProcFlags`: what makes it fire.
///
/// From vmangos' `Spells/SpellDefines.h`'s `ProcFlags`.
pub const PROC_FLAGS: [(u32, &str, &str); 24] = [
    (0x00000001, "HEARTBEAT", ""),
    (0x00000002, "KILL", ""),
    (0x00000004, "DEAL_MELEE_SWING", ""),
    (0x00000008, "TAKE_MELEE_SWING", ""),
    (0x00000010, "DEAL_MELEE_ABILITY", ""),
    (0x00000020, "TAKE_MELEE_ABILITY", ""),
    (0x00000040, "DEAL_RANGED_ATTACK", ""),
    (0x00000080, "TAKE_RANGED_ATTACK", ""),
    (0x00000100, "DEAL_RANGED_ABILITY", ""),
    (0x00000200, "TAKE_RANGED_ABILITY", ""),
    (0x00000400, "DEAL_HELPFUL_ABILITY", ""),
    (0x00000800, "TAKE_HELPFUL_ABILITY", ""),
    (0x00001000, "DEAL_HARMFUL_ABILITY", ""),
    (0x00002000, "TAKE_HARMFUL_ABILITY", ""),
    (0x00004000, "DEAL_HELPFUL_SPELL", ""),
    (0x00008000, "TAKE_HELPFUL_SPELL", ""),
    (0x00010000, "DEAL_HARMFUL_SPELL", ""),
    (0x00020000, "TAKE_HARMFUL_SPELL", ""),
    (0x00040000, "DEAL_HARMFUL_PERIODIC", ""),
    (0x00080000, "TAKE_HARMFUL_PERIODIC", ""),
    (0x00100000, "TAKEN_ANY_DAMAGE", ""),
    (0x00200000, "ON_TRAP_ACTIVATION", ""),
    (0x00400000, "MAIN_HAND_WEAPON_SWING", ""),
    (0x00800000, "OFF_HAND_WEAPON_SWING", ""),
];

/// `Stances` and `StancesNot`, which are a mask of forms.
///
/// From vmangos' `SharedDefines.h`'s `ShapeshiftForm`.
pub const SHAPESHIFT: [(u32, &str, &str); 6] = [
    (0x00000001, "CAT", ""),
    (0x00000002, "TREE", ""),
    (0x00000004, "AQUA", ""),
    (0x00000008, "DIREBEAR", ""),
    (0x00000010, "GHOSTWOLF", ""),
    (0x00000020, "SPIRITOFREDEMPTION", ""),
];

/// `TargetCreatureType`: which kinds of creature the spell may be aimed at.
///
/// From vmangos' `Objects/CreatureDefines.h`'s `CreatureType`.
pub const CREATURE_TYPES: [(u32, &str, &str); 11] = [
    (0x00000001, "BEAST", ""),
    (0x00000002, "DRAGONKIN", ""),
    (0x00000004, "DEMON", ""),
    (0x00000008, "ELEMENTAL", ""),
    (0x00000010, "GIANT", ""),
    (0x00000020, "UNDEAD", ""),
    (0x00000040, "HUMANOID", ""),
    (0x00000080, "CRITTER", ""),
    (0x00000100, "MECHANICAL", ""),
    (0x00000200, "NOT_SPECIFIED", ""),
    (0x00000400, "TOTEM", ""),
];

// --- and the columns whose value is one of a set, not a mask ---

/// `EffectImplicitTargetA` and `B`: who or what the effect lands on.
///
/// From vmangos' `Spells/SpellDefines.h`'s `SpellTarget`.
pub const IMPLICIT_TARGETS: [(u32, &str); 64] = [
    (0, "NONE"),
    (1, "UNIT_CASTER"),
    (2, "UNIT_ENEMY_NEAR_CASTER"),
    (3, "UNIT_FRIEND_NEAR_CASTER"),
    (4, "UNIT_NEAR_CASTER"),
    (5, "UNIT_CASTER_PET"),
    (6, "UNIT_ENEMY"),
    (7, "ENUM_UNITS_SCRIPT_AOE_AT_SRC_LOC"),
    (8, "ENUM_UNITS_SCRIPT_AOE_AT_DEST_LOC"),
    (9, "LOCATION_CASTER_HOME_BIND"),
    (10, "LOCATION_CASTER_DIVINE_BIND_NYI"),
    (11, "PLAYER_NYI"),
    (12, "PLAYER_NEAR_CASTER_NYI"),
    (13, "PLAYER_ENEMY_NYI"),
    (14, "PLAYER_FRIEND_NYI"),
    (15, "ENUM_UNITS_ENEMY_AOE_AT_SRC_LOC"),
    (16, "ENUM_UNITS_ENEMY_AOE_AT_DEST_LOC"),
    (17, "LOCATION_DATABASE"),
    (18, "LOCATION_CASTER_DEST"),
    (19, "UNK_19"),
    (20, "ENUM_UNITS_PARTY_WITHIN_CASTER_RANGE"),
    (21, "UNIT_FRIEND"),
    (22, "LOCATION_CASTER_SRC"),
    (23, "GAMEOBJECT"),
    (24, "ENUM_UNITS_ENEMY_IN_CONE_24"),
    (25, "UNIT"),
    (26, "LOCKED"),
    (27, "UNIT_CASTER_MASTER"),
    (28, "ENUM_UNITS_ENEMY_AOE_AT_DYNOBJ_LOC"),
    (29, "ENUM_UNITS_FRIEND_AOE_AT_DYNOBJ_LOC"),
    (30, "ENUM_UNITS_FRIEND_AOE_AT_SRC_LOC"),
    (31, "ENUM_UNITS_FRIEND_AOE_AT_DEST_LOC"),
    (32, "LOCATION_UNIT_MINION_POSITION"),
    (33, "ENUM_UNITS_PARTY_AOE_AT_SRC_LOC"),
    (34, "ENUM_UNITS_PARTY_AOE_AT_DEST_LOC"),
    (35, "UNIT_PARTY"),
    (36, "ENUM_UNITS_ENEMY_WITHIN_CASTER_RANGE"),
    (37, "UNIT_FRIEND_AND_PARTY"),
    (38, "UNIT_SCRIPT_NEAR_CASTER"),
    (39, "LOCATION_CASTER_FISHING_SPOT"),
    (40, "GAMEOBJECT_SCRIPT_NEAR_CASTER"),
    (41, "LOCATION_CASTER_FRONT_RIGHT"),
    (42, "LOCATION_CASTER_BACK_RIGHT"),
    (43, "LOCATION_CASTER_BACK_LEFT"),
    (44, "LOCATION_CASTER_FRONT_LEFT"),
    (45, "UNIT_FRIEND_CHAIN_HEAL"),
    (46, "LOCATION_SCRIPT_NEAR_CASTER"),
    (47, "LOCATION_CASTER_FRONT"),
    (48, "LOCATION_CASTER_BACK"),
    (49, "LOCATION_CASTER_LEFT"),
    (50, "LOCATION_CASTER_RIGHT"),
    (51, "ENUM_GAMEOBJECTS_SCRIPT_AOE_AT_SRC_LOC"),
    (52, "ENUM_GAMEOBJECTS_SCRIPT_AOE_AT_DEST_LOC"),
    (53, "LOCATION_CASTER_TARGET_POSITION"),
    (54, "ENUM_UNITS_ENEMY_IN_CONE_54"),
    (55, "LOCATION_CASTER_FRONT_LEAP"),
    (56, "ENUM_UNITS_RAID_WITHIN_CASTER_RANGE"),
    (57, "UNIT_RAID"),
    (58, "UNIT_RAID_NEAR_CASTER"),
    (59, "ENUM_UNITS_FRIEND_IN_CONE"),
    (60, "ENUM_UNITS_SCRIPT_IN_CONE_60"),
    (61, "UNIT_RAID_AND_CLASS"),
    (62, "PLAYER_RAID_NYI"),
    (63, "LOCATION_UNIT_POSITION"),
];

/// `CasterAuraState` and `TargetAuraState`: the state required for a cast.
///
/// From vmangos' `Spells/SpellDefines.h`'s `AuraState`.
pub const AURA_STATES: [(u32, &str); 10] = [
    (1, "DEFENSE"),
    (2, "HEALTHLESS_20_PERCENT"),
    (3, "BERSERKING"),
    (4, "FROZEN"),
    (5, "JUDGEMENT"),
    (7, "HUNTER_PARRY"),
    (8, "ROGUE_ATTACK_FROM_STEALTH"),
    (9, "HEALTHLESS_15_PERCENT"),
    (10, "HEALTHLESS_10_PERCENT"),
    (11, "HEALTHLESS_5_PERCENT"),
];

/// `EquippedItemClass`, which is -1 for a spell with no item requirement.
///
/// From vmangos' `Objects/ItemPrototype.h`'s `ItemClass`.
pub const ITEM_CLASSES: [(u32, &str); 16] = [
    (0, "CONSUMABLE"),
    (1, "CONTAINER"),
    (2, "WEAPON"),
    (3, "GEM"),
    (4, "ARMOR"),
    (5, "REAGENT"),
    (6, "PROJECTILE"),
    (7, "TRADE_GOODS"),
    (8, "GENERIC"),
    (9, "RECIPE"),
    (10, "MONEY"),
    (11, "QUIVER"),
    (12, "QUEST"),
    (13, "KEY"),
    (14, "PERMANENT"),
    (15, "JUNK"),
];

/// `SpellFamilyName`, which is what a talent's mask is read against.
///
/// From vmangos' `Database/DBCEnums.h`'s `SpellFamily`.
pub const SPELL_FAMILIES: [(u32, &str); 15] = [
    (0, "GENERIC"),
    (1, "UNK1"),
    (3, "MAGE"),
    (4, "WARRIOR"),
    (5, "WARLOCK"),
    (6, "PRIEST"),
    (7, "DRUID"),
    (8, "ROGUE"),
    (9, "HUNTER"),
    (10, "PALADIN"),
    (11, "SHAMAN"),
    (12, "UNK2"),
    (13, "POTION"),
    (15, "DEATHKNIGHT"),
    (17, "UNK3"),
];

