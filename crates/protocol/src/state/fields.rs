//! Update-field indices for WoW 1.12.1 (build 5875).
//!
//! Values follow vmangos' `src/game/Objects/UpdateFields_1_12_1.h`.
//!
//! Indices are **per object type and overlap** — index 6 is
//! `item::OWNER` on an item and `unit::CHARM` on a unit. Always select the
//! module matching the object's `ObjectType` before looking an index up;
//! reading a unit field off an item yields silent nonsense, not an error.
//!
//! Multi-word fields (GUIDs are size 2, and some arrays are much larger) occupy
//! consecutive indices starting at the constant.

/// `object` update fields.
pub mod object {
    /// size 2, GUID
    pub const GUID: u16 = 0;
    /// size 1, INT
    pub const TYPE: u16 = 2;
    /// size 1, INT
    pub const ENTRY: u16 = 3;
    /// size 1, FLOAT
    pub const SCALE_X: u16 = 4;
    /// size 1, INT
    pub const PADDING: u16 = 5;

    /// Name of a field index within this object type, for logging.
    pub fn name_of(index: u16) -> Option<&'static str> {
        Some(match index {
            0 => "GUID",
            2 => "TYPE",
            3 => "ENTRY",
            4 => "SCALE_X",
            5 => "PADDING",
            _ => return None,
        })
    }
}

/// `item` update fields.
pub mod item {
    /// size 2, GUID
    pub const OWNER: u16 = 6;
    /// size 2, GUID
    pub const CONTAINED: u16 = 8;
    /// size 2, GUID
    pub const CREATOR: u16 = 10;
    /// size 2, GUID
    pub const GIFTCREATOR: u16 = 12;
    /// size 1, INT
    pub const STACK_COUNT: u16 = 14;
    /// size 1, INT
    pub const DURATION: u16 = 15;
    /// size 5, INT
    pub const SPELL_CHARGES: u16 = 16;
    /// size 1, INT
    pub const FLAGS: u16 = 21;
    /// size 21, INT
    pub const ENCHANTMENT: u16 = 22;
    /// size 1, INT
    pub const PROPERTY_SEED: u16 = 43;
    /// size 1, INT
    pub const RANDOM_PROPERTIES_ID: u16 = 44;
    /// size 1, INT
    pub const ITEM_TEXT_ID: u16 = 45;
    /// size 1, INT
    pub const DURABILITY: u16 = 46;
    /// size 1, INT
    pub const MAXDURABILITY: u16 = 47;

    /// Name of a field index within this object type, for logging.
    pub fn name_of(index: u16) -> Option<&'static str> {
        Some(match index {
            6 => "OWNER",
            8 => "CONTAINED",
            10 => "CREATOR",
            12 => "GIFTCREATOR",
            14 => "STACK_COUNT",
            15 => "DURATION",
            16 => "SPELL_CHARGES",
            21 => "FLAGS",
            22 => "ENCHANTMENT",
            43 => "PROPERTY_SEED",
            44 => "RANDOM_PROPERTIES_ID",
            45 => "ITEM_TEXT_ID",
            46 => "DURABILITY",
            47 => "MAXDURABILITY",
            _ => return None,
        })
    }
}

/// `container` update fields.
pub mod container {
    /// size 1, INT
    pub const NUM_SLOTS: u16 = 48;
    /// size 1, BYTES
    pub const ALIGN_PAD: u16 = 49;
    /// size 72, GUID
    pub const SLOT_1: u16 = 50;

    /// Name of a field index within this object type, for logging.
    pub fn name_of(index: u16) -> Option<&'static str> {
        Some(match index {
            48 => "NUM_SLOTS",
            49 => "ALIGN_PAD",
            50 => "SLOT_1",
            _ => return None,
        })
    }
}

/// `unit` update fields.
pub mod unit {
    /// size 2, GUID
    pub const CHARM: u16 = 6;
    /// size 2, GUID
    pub const SUMMON: u16 = 8;
    /// size 2, GUID
    pub const CHARMEDBY: u16 = 10;
    /// size 2, GUID
    pub const SUMMONEDBY: u16 = 12;
    /// size 2, GUID
    pub const CREATEDBY: u16 = 14;
    /// size 2, GUID
    pub const TARGET: u16 = 16;
    /// size 2, GUID
    pub const PERSUADED: u16 = 18;
    /// size 2, GUID
    pub const CHANNEL_OBJECT: u16 = 20;
    /// size 1, INT
    pub const HEALTH: u16 = 22;
    /// size 1, INT
    pub const POWER1: u16 = 23;
    /// size 1, INT
    pub const POWER2: u16 = 24;
    /// size 1, INT
    pub const POWER3: u16 = 25;
    /// size 1, INT
    pub const POWER4: u16 = 26;
    /// size 1, INT
    pub const POWER5: u16 = 27;
    /// size 1, INT
    pub const MAXHEALTH: u16 = 28;
    /// size 1, INT
    pub const MAXPOWER1: u16 = 29;
    /// size 1, INT
    pub const MAXPOWER2: u16 = 30;
    /// size 1, INT
    pub const MAXPOWER3: u16 = 31;
    /// size 1, INT
    pub const MAXPOWER4: u16 = 32;
    /// size 1, INT
    pub const MAXPOWER5: u16 = 33;
    /// size 1, INT
    pub const LEVEL: u16 = 34;
    /// size 1, INT
    pub const FACTIONTEMPLATE: u16 = 35;
    /// size 1, BYTES
    pub const BYTES_0: u16 = 36;
    /// size 3, INT
    pub const VIRTUAL_ITEM_SLOT_DISPLAY: u16 = 37;
    /// size 6, BYTES
    pub const VIRTUAL_ITEM_INFO: u16 = 40;
    /// size 1, INT
    pub const FLAGS: u16 = 46;
    /// size 48, INT
    pub const AURA: u16 = 47;
    /// size 6, BYTES
    pub const AURAFLAGS: u16 = 95;
    /// size 12, BYTES
    pub const AURALEVELS: u16 = 101;
    /// size 12, BYTES
    pub const AURAAPPLICATIONS: u16 = 113;
    /// size 1, INT
    pub const AURASTATE: u16 = 125;
    /// size 2, INT
    pub const BASEATTACKTIME: u16 = 126;
    /// size 1, INT
    pub const RANGEDATTACKTIME: u16 = 128;
    /// size 1, FLOAT
    pub const BOUNDINGRADIUS: u16 = 129;
    /// size 1, FLOAT
    pub const COMBATREACH: u16 = 130;
    /// size 1, INT
    pub const DISPLAYID: u16 = 131;
    /// size 1, INT
    pub const NATIVEDISPLAYID: u16 = 132;
    /// size 1, INT
    pub const MOUNTDISPLAYID: u16 = 133;
    /// size 1, FLOAT
    pub const MINDAMAGE: u16 = 134;
    /// size 1, FLOAT
    pub const MAXDAMAGE: u16 = 135;
    /// size 1, FLOAT
    pub const MINOFFHANDDAMAGE: u16 = 136;
    /// size 1, FLOAT
    pub const MAXOFFHANDDAMAGE: u16 = 137;
    /// size 1, BYTES
    pub const BYTES_1: u16 = 138;
    /// size 1, INT
    pub const PETNUMBER: u16 = 139;
    /// size 1, INT
    pub const PET_NAME_TIMESTAMP: u16 = 140;
    /// size 1, INT
    pub const PETEXPERIENCE: u16 = 141;
    /// size 1, INT
    pub const PETNEXTLEVELEXP: u16 = 142;
    /// size 1, INT
    pub const DYNAMIC_FLAGS: u16 = 143;
    /// size 1, INT
    pub const CHANNEL_SPELL: u16 = 144;
    /// size 1, FLOAT
    pub const MOD_CAST_SPEED: u16 = 145;
    /// size 1, INT
    pub const CREATED_BY_SPELL: u16 = 146;
    /// size 1, INT
    pub const NPC_FLAGS: u16 = 147;
    /// size 1, INT
    pub const NPC_EMOTESTATE: u16 = 148;
    /// size 1, TWO_SHORT
    pub const TRAINING_POINTS: u16 = 149;
    /// size 1, INT
    pub const STAT0: u16 = 150;
    /// size 1, INT
    pub const STAT1: u16 = 151;
    /// size 1, INT
    pub const STAT2: u16 = 152;
    /// size 1, INT
    pub const STAT3: u16 = 153;
    /// size 1, INT
    pub const STAT4: u16 = 154;
    /// size 7, INT
    pub const RESISTANCES: u16 = 155;
    /// size 1, INT
    pub const BASE_MANA: u16 = 162;
    /// size 1, INT
    pub const BASE_HEALTH: u16 = 163;
    /// size 1, BYTES
    pub const BYTES_2: u16 = 164;
    /// size 1, INT
    pub const ATTACK_POWER: u16 = 165;
    /// size 1, TWO_SHORT
    pub const ATTACK_POWER_MODS: u16 = 166;
    /// size 1, FLOAT
    pub const ATTACK_POWER_MULTIPLIER: u16 = 167;
    /// size 1, INT
    pub const RANGED_ATTACK_POWER: u16 = 168;
    /// size 1, TWO_SHORT
    pub const RANGED_ATTACK_POWER_MODS: u16 = 169;
    /// size 1, FLOAT
    pub const RANGED_ATTACK_POWER_MULTIPLIER: u16 = 170;
    /// size 1, FLOAT
    pub const MINRANGEDDAMAGE: u16 = 171;
    /// size 1, FLOAT
    pub const MAXRANGEDDAMAGE: u16 = 172;
    /// size 7, INT
    pub const POWER_COST_MODIFIER: u16 = 173;
    /// size 7, FLOAT
    pub const POWER_COST_MULTIPLIER: u16 = 180;
    /// size 1, INT
    pub const PADDING: u16 = 187;

    /// Name of a field index within this object type, for logging.
    pub fn name_of(index: u16) -> Option<&'static str> {
        Some(match index {
            6 => "CHARM",
            8 => "SUMMON",
            10 => "CHARMEDBY",
            12 => "SUMMONEDBY",
            14 => "CREATEDBY",
            16 => "TARGET",
            18 => "PERSUADED",
            20 => "CHANNEL_OBJECT",
            22 => "HEALTH",
            23 => "POWER1",
            24 => "POWER2",
            25 => "POWER3",
            26 => "POWER4",
            27 => "POWER5",
            28 => "MAXHEALTH",
            29 => "MAXPOWER1",
            30 => "MAXPOWER2",
            31 => "MAXPOWER3",
            32 => "MAXPOWER4",
            33 => "MAXPOWER5",
            34 => "LEVEL",
            35 => "FACTIONTEMPLATE",
            36 => "BYTES_0",
            37 => "VIRTUAL_ITEM_SLOT_DISPLAY",
            40 => "VIRTUAL_ITEM_INFO",
            46 => "FLAGS",
            47 => "AURA",
            95 => "AURAFLAGS",
            101 => "AURALEVELS",
            113 => "AURAAPPLICATIONS",
            125 => "AURASTATE",
            126 => "BASEATTACKTIME",
            128 => "RANGEDATTACKTIME",
            129 => "BOUNDINGRADIUS",
            130 => "COMBATREACH",
            131 => "DISPLAYID",
            132 => "NATIVEDISPLAYID",
            133 => "MOUNTDISPLAYID",
            134 => "MINDAMAGE",
            135 => "MAXDAMAGE",
            136 => "MINOFFHANDDAMAGE",
            137 => "MAXOFFHANDDAMAGE",
            138 => "BYTES_1",
            139 => "PETNUMBER",
            140 => "PET_NAME_TIMESTAMP",
            141 => "PETEXPERIENCE",
            142 => "PETNEXTLEVELEXP",
            143 => "DYNAMIC_FLAGS",
            144 => "CHANNEL_SPELL",
            145 => "MOD_CAST_SPEED",
            146 => "CREATED_BY_SPELL",
            147 => "NPC_FLAGS",
            148 => "NPC_EMOTESTATE",
            149 => "TRAINING_POINTS",
            150 => "STAT0",
            151 => "STAT1",
            152 => "STAT2",
            153 => "STAT3",
            154 => "STAT4",
            155 => "RESISTANCES",
            162 => "BASE_MANA",
            163 => "BASE_HEALTH",
            164 => "BYTES_2",
            165 => "ATTACK_POWER",
            166 => "ATTACK_POWER_MODS",
            167 => "ATTACK_POWER_MULTIPLIER",
            168 => "RANGED_ATTACK_POWER",
            169 => "RANGED_ATTACK_POWER_MODS",
            170 => "RANGED_ATTACK_POWER_MULTIPLIER",
            171 => "MINRANGEDDAMAGE",
            172 => "MAXRANGEDDAMAGE",
            173 => "POWER_COST_MODIFIER",
            180 => "POWER_COST_MULTIPLIER",
            187 => "PADDING",
            _ => return None,
        })
    }
}

/// `player` update fields.
pub mod player {
    /// size 2, GUID
    pub const DUEL_ARBITER: u16 = 188;
    /// size 1, INT
    pub const FLAGS: u16 = 190;
    /// size 1, INT
    pub const GUILDID: u16 = 191;
    /// size 1, INT
    pub const GUILDRANK: u16 = 192;
    /// size 1, BYTES
    pub const BYTES: u16 = 193;
    /// size 1, BYTES
    pub const BYTES_2: u16 = 194;
    /// size 1, BYTES
    pub const BYTES_3: u16 = 195;
    /// size 1, INT
    pub const DUEL_TEAM: u16 = 196;
    /// size 1, INT
    pub const GUILD_TIMESTAMP: u16 = 197;
    /// size 1, INT
    pub const QUEST_LOG_1_1: u16 = 198;
    /// size 2, INT
    pub const QUEST_LOG_1_2: u16 = 199;
    /// size 1, INT
    pub const QUEST_LOG_2_1: u16 = 201;
    /// size 2, INT
    pub const QUEST_LOG_2_2: u16 = 202;
    /// size 1, INT
    pub const QUEST_LOG_3_1: u16 = 204;
    /// size 2, INT
    pub const QUEST_LOG_3_2: u16 = 205;
    /// size 1, INT
    pub const QUEST_LOG_4_1: u16 = 207;
    /// size 2, INT
    pub const QUEST_LOG_4_2: u16 = 208;
    /// size 1, INT
    pub const QUEST_LOG_5_1: u16 = 210;
    /// size 2, INT
    pub const QUEST_LOG_5_2: u16 = 211;
    /// size 1, INT
    pub const QUEST_LOG_6_1: u16 = 213;
    /// size 2, INT
    pub const QUEST_LOG_6_2: u16 = 214;
    /// size 1, INT
    pub const QUEST_LOG_7_1: u16 = 216;
    /// size 2, INT
    pub const QUEST_LOG_7_2: u16 = 217;
    /// size 1, INT
    pub const QUEST_LOG_8_1: u16 = 219;
    /// size 2, INT
    pub const QUEST_LOG_8_2: u16 = 220;
    /// size 1, INT
    pub const QUEST_LOG_9_1: u16 = 222;
    /// size 2, INT
    pub const QUEST_LOG_9_2: u16 = 223;
    /// size 1, INT
    pub const QUEST_LOG_10_1: u16 = 225;
    /// size 2, INT
    pub const QUEST_LOG_10_2: u16 = 226;
    /// size 1, INT
    pub const QUEST_LOG_11_1: u16 = 228;
    /// size 2, INT
    pub const QUEST_LOG_11_2: u16 = 229;
    /// size 1, INT
    pub const QUEST_LOG_12_1: u16 = 231;
    /// size 2, INT
    pub const QUEST_LOG_12_2: u16 = 232;
    /// size 1, INT
    pub const QUEST_LOG_13_1: u16 = 234;
    /// size 2, INT
    pub const QUEST_LOG_13_2: u16 = 235;
    /// size 1, INT
    pub const QUEST_LOG_14_1: u16 = 237;
    /// size 2, INT
    pub const QUEST_LOG_14_2: u16 = 238;
    /// size 1, INT
    pub const QUEST_LOG_15_1: u16 = 240;
    /// size 2, INT
    pub const QUEST_LOG_15_2: u16 = 241;
    /// size 1, INT
    pub const QUEST_LOG_16_1: u16 = 243;
    /// size 2, INT
    pub const QUEST_LOG_16_2: u16 = 244;
    /// size 1, INT
    pub const QUEST_LOG_17_1: u16 = 246;
    /// size 2, INT
    pub const QUEST_LOG_17_2: u16 = 247;
    /// size 1, INT
    pub const QUEST_LOG_18_1: u16 = 249;
    /// size 2, INT
    pub const QUEST_LOG_18_2: u16 = 250;
    /// size 1, INT
    pub const QUEST_LOG_19_1: u16 = 252;
    /// size 2, INT
    pub const QUEST_LOG_19_2: u16 = 253;
    /// size 1, INT
    pub const QUEST_LOG_20_1: u16 = 255;
    /// size 2, INT
    pub const QUEST_LOG_20_2: u16 = 256;
    /// size 2, GUID
    pub const VISIBLE_ITEM_1_CREATOR: u16 = 258;
    /// size 8, INT
    pub const VISIBLE_ITEM_1_0: u16 = 260;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_1_PROPERTIES: u16 = 268;
    /// size 1, INT
    pub const VISIBLE_ITEM_1_PAD: u16 = 269;
    /// size 2, GUID
    pub const VISIBLE_ITEM_2_CREATOR: u16 = 270;
    /// size 8, INT
    pub const VISIBLE_ITEM_2_0: u16 = 272;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_2_PROPERTIES: u16 = 280;
    /// size 1, INT
    pub const VISIBLE_ITEM_2_PAD: u16 = 281;
    /// size 2, GUID
    pub const VISIBLE_ITEM_3_CREATOR: u16 = 282;
    /// size 8, INT
    pub const VISIBLE_ITEM_3_0: u16 = 284;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_3_PROPERTIES: u16 = 292;
    /// size 1, INT
    pub const VISIBLE_ITEM_3_PAD: u16 = 293;
    /// size 2, GUID
    pub const VISIBLE_ITEM_4_CREATOR: u16 = 294;
    /// size 8, INT
    pub const VISIBLE_ITEM_4_0: u16 = 296;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_4_PROPERTIES: u16 = 304;
    /// size 1, INT
    pub const VISIBLE_ITEM_4_PAD: u16 = 305;
    /// size 2, GUID
    pub const VISIBLE_ITEM_5_CREATOR: u16 = 306;
    /// size 8, INT
    pub const VISIBLE_ITEM_5_0: u16 = 308;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_5_PROPERTIES: u16 = 316;
    /// size 1, INT
    pub const VISIBLE_ITEM_5_PAD: u16 = 317;
    /// size 2, GUID
    pub const VISIBLE_ITEM_6_CREATOR: u16 = 318;
    /// size 8, INT
    pub const VISIBLE_ITEM_6_0: u16 = 320;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_6_PROPERTIES: u16 = 328;
    /// size 1, INT
    pub const VISIBLE_ITEM_6_PAD: u16 = 329;
    /// size 2, GUID
    pub const VISIBLE_ITEM_7_CREATOR: u16 = 330;
    /// size 8, INT
    pub const VISIBLE_ITEM_7_0: u16 = 332;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_7_PROPERTIES: u16 = 340;
    /// size 1, INT
    pub const VISIBLE_ITEM_7_PAD: u16 = 341;
    /// size 2, GUID
    pub const VISIBLE_ITEM_8_CREATOR: u16 = 342;
    /// size 8, INT
    pub const VISIBLE_ITEM_8_0: u16 = 344;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_8_PROPERTIES: u16 = 352;
    /// size 1, INT
    pub const VISIBLE_ITEM_8_PAD: u16 = 353;
    /// size 2, GUID
    pub const VISIBLE_ITEM_9_CREATOR: u16 = 354;
    /// size 8, INT
    pub const VISIBLE_ITEM_9_0: u16 = 356;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_9_PROPERTIES: u16 = 364;
    /// size 1, INT
    pub const VISIBLE_ITEM_9_PAD: u16 = 365;
    /// size 2, GUID
    pub const VISIBLE_ITEM_10_CREATOR: u16 = 366;
    /// size 8, INT
    pub const VISIBLE_ITEM_10_0: u16 = 368;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_10_PROPERTIES: u16 = 376;
    /// size 1, INT
    pub const VISIBLE_ITEM_10_PAD: u16 = 377;
    /// size 2, GUID
    pub const VISIBLE_ITEM_11_CREATOR: u16 = 378;
    /// size 8, INT
    pub const VISIBLE_ITEM_11_0: u16 = 380;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_11_PROPERTIES: u16 = 388;
    /// size 1, INT
    pub const VISIBLE_ITEM_11_PAD: u16 = 389;
    /// size 2, GUID
    pub const VISIBLE_ITEM_12_CREATOR: u16 = 390;
    /// size 8, INT
    pub const VISIBLE_ITEM_12_0: u16 = 392;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_12_PROPERTIES: u16 = 400;
    /// size 1, INT
    pub const VISIBLE_ITEM_12_PAD: u16 = 401;
    /// size 2, GUID
    pub const VISIBLE_ITEM_13_CREATOR: u16 = 402;
    /// size 8, INT
    pub const VISIBLE_ITEM_13_0: u16 = 404;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_13_PROPERTIES: u16 = 412;
    /// size 1, INT
    pub const VISIBLE_ITEM_13_PAD: u16 = 413;
    /// size 2, GUID
    pub const VISIBLE_ITEM_14_CREATOR: u16 = 414;
    /// size 8, INT
    pub const VISIBLE_ITEM_14_0: u16 = 416;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_14_PROPERTIES: u16 = 424;
    /// size 1, INT
    pub const VISIBLE_ITEM_14_PAD: u16 = 425;
    /// size 2, GUID
    pub const VISIBLE_ITEM_15_CREATOR: u16 = 426;
    /// size 8, INT
    pub const VISIBLE_ITEM_15_0: u16 = 428;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_15_PROPERTIES: u16 = 436;
    /// size 1, INT
    pub const VISIBLE_ITEM_15_PAD: u16 = 437;
    /// size 2, GUID
    pub const VISIBLE_ITEM_16_CREATOR: u16 = 438;
    /// size 8, INT
    pub const VISIBLE_ITEM_16_0: u16 = 440;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_16_PROPERTIES: u16 = 448;
    /// size 1, INT
    pub const VISIBLE_ITEM_16_PAD: u16 = 449;
    /// size 2, GUID
    pub const VISIBLE_ITEM_17_CREATOR: u16 = 450;
    /// size 8, INT
    pub const VISIBLE_ITEM_17_0: u16 = 452;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_17_PROPERTIES: u16 = 460;
    /// size 1, INT
    pub const VISIBLE_ITEM_17_PAD: u16 = 461;
    /// size 2, GUID
    pub const VISIBLE_ITEM_18_CREATOR: u16 = 462;
    /// size 8, INT
    pub const VISIBLE_ITEM_18_0: u16 = 464;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_18_PROPERTIES: u16 = 472;
    /// size 1, INT
    pub const VISIBLE_ITEM_18_PAD: u16 = 473;
    /// size 2, GUID
    pub const VISIBLE_ITEM_19_CREATOR: u16 = 474;
    /// size 8, INT
    pub const VISIBLE_ITEM_19_0: u16 = 476;
    /// size 1, TWO_SHORT
    pub const VISIBLE_ITEM_19_PROPERTIES: u16 = 484;
    /// size 1, INT
    pub const VISIBLE_ITEM_19_PAD: u16 = 485;
    /// size 46, GUID
    pub const INV_SLOT_HEAD: u16 = 486;
    /// size 32, GUID
    pub const PACK_SLOT_1: u16 = 532;
    /// size 48, GUID
    pub const BANK_SLOT_1: u16 = 564;
    /// size 12, GUID
    pub const BANKBAG_SLOT_1: u16 = 612;
    /// size 24, GUID
    pub const VENDORBUYBACK_SLOT_1: u16 = 624;
    /// size 64, GUID
    pub const KEYRING_SLOT_1: u16 = 648;
    /// size 2, GUID
    pub const FARSIGHT: u16 = 712;
    /// size 2, GUID
    pub const COMBO_TARGET: u16 = 714;
    /// size 1, INT
    pub const XP: u16 = 716;
    /// size 1, INT
    pub const NEXT_LEVEL_XP: u16 = 717;
    /// size 384, TWO_SHORT
    pub const SKILL_INFO_1_1: u16 = 718;
    /// size 1, INT
    pub const CHARACTER_POINTS1: u16 = 1102;
    /// size 1, INT
    pub const CHARACTER_POINTS2: u16 = 1103;
    /// size 1, INT
    pub const TRACK_CREATURES: u16 = 1104;
    /// size 1, INT
    pub const TRACK_RESOURCES: u16 = 1105;
    /// size 1, FLOAT
    pub const BLOCK_PERCENTAGE: u16 = 1106;
    /// size 1, FLOAT
    pub const DODGE_PERCENTAGE: u16 = 1107;
    /// size 1, FLOAT
    pub const PARRY_PERCENTAGE: u16 = 1108;
    /// size 1, FLOAT
    pub const CRIT_PERCENTAGE: u16 = 1109;
    /// size 1, FLOAT
    pub const RANGED_CRIT_PERCENTAGE: u16 = 1110;
    /// size 64, BYTES
    pub const EXPLORED_ZONES_1: u16 = 1111;
    /// size 1, INT
    pub const REST_STATE_EXPERIENCE: u16 = 1175;
    /// size 1, INT
    pub const COINAGE: u16 = 1176;
    /// size 1, INT
    pub const POSSTAT0: u16 = 1177;
    /// size 1, INT
    pub const POSSTAT1: u16 = 1178;
    /// size 1, INT
    pub const POSSTAT2: u16 = 1179;
    /// size 1, INT
    pub const POSSTAT3: u16 = 1180;
    /// size 1, INT
    pub const POSSTAT4: u16 = 1181;
    /// size 1, INT
    pub const NEGSTAT0: u16 = 1182;
    /// size 1, INT
    pub const NEGSTAT1: u16 = 1183;
    /// size 1, INT
    pub const NEGSTAT2: u16 = 1184;
    /// size 1, INT
    pub const NEGSTAT3: u16 = 1185;
    /// size 1, INT
    pub const NEGSTAT4: u16 = 1186;
    /// size 7, INT
    pub const RESISTANCEBUFFMODSPOSITIVE: u16 = 1187;
    /// size 7, INT
    pub const RESISTANCEBUFFMODSNEGATIVE: u16 = 1194;
    /// size 7, INT
    pub const MOD_DAMAGE_DONE_POS: u16 = 1201;
    /// size 7, INT
    pub const MOD_DAMAGE_DONE_NEG: u16 = 1208;
    /// size 7, INT
    pub const MOD_DAMAGE_DONE_PCT: u16 = 1215;
    /// size 1, BYTES
    pub const FIELD_BYTES: u16 = 1222;
    /// size 1, INT
    pub const AMMO_ID: u16 = 1223;
    /// size 1, INT
    pub const SELF_RES_SPELL: u16 = 1224;
    /// size 1, INT
    pub const PVP_MEDALS: u16 = 1225;
    /// size 12, INT
    pub const BUYBACK_PRICE_1: u16 = 1226;
    /// size 12, INT
    pub const BUYBACK_TIMESTAMP_1: u16 = 1238;
    /// size 1, TWO_SHORT
    pub const SESSION_KILLS: u16 = 1250;
    /// size 1, TWO_SHORT
    pub const YESTERDAY_KILLS: u16 = 1251;
    /// size 1, TWO_SHORT
    pub const LAST_WEEK_KILLS: u16 = 1252;
    /// size 1, TWO_SHORT
    pub const THIS_WEEK_KILLS: u16 = 1253;
    /// size 1, INT
    pub const THIS_WEEK_CONTRIBUTION: u16 = 1254;
    /// size 1, INT
    pub const LIFETIME_HONORBALE_KILLS: u16 = 1255;
    /// size 1, INT
    pub const LIFETIME_DISHONORBALE_KILLS: u16 = 1256;
    /// size 1, INT
    pub const YESTERDAY_CONTRIBUTION: u16 = 1257;
    /// size 1, INT
    pub const LAST_WEEK_CONTRIBUTION: u16 = 1258;
    /// size 1, INT
    pub const LAST_WEEK_RANK: u16 = 1259;
    /// size 1, BYTES
    pub const BYTES2: u16 = 1260;
    /// size 1, INT
    pub const WATCHED_FACTION_INDEX: u16 = 1261;
    /// size 20, INT
    pub const COMBAT_RATING_1: u16 = 1262;

    /// Name of a field index within this object type, for logging.
    pub fn name_of(index: u16) -> Option<&'static str> {
        Some(match index {
            188 => "DUEL_ARBITER",
            190 => "FLAGS",
            191 => "GUILDID",
            192 => "GUILDRANK",
            193 => "BYTES",
            194 => "BYTES_2",
            195 => "BYTES_3",
            196 => "DUEL_TEAM",
            197 => "GUILD_TIMESTAMP",
            198 => "QUEST_LOG_1_1",
            199 => "QUEST_LOG_1_2",
            201 => "QUEST_LOG_2_1",
            202 => "QUEST_LOG_2_2",
            204 => "QUEST_LOG_3_1",
            205 => "QUEST_LOG_3_2",
            207 => "QUEST_LOG_4_1",
            208 => "QUEST_LOG_4_2",
            210 => "QUEST_LOG_5_1",
            211 => "QUEST_LOG_5_2",
            213 => "QUEST_LOG_6_1",
            214 => "QUEST_LOG_6_2",
            216 => "QUEST_LOG_7_1",
            217 => "QUEST_LOG_7_2",
            219 => "QUEST_LOG_8_1",
            220 => "QUEST_LOG_8_2",
            222 => "QUEST_LOG_9_1",
            223 => "QUEST_LOG_9_2",
            225 => "QUEST_LOG_10_1",
            226 => "QUEST_LOG_10_2",
            228 => "QUEST_LOG_11_1",
            229 => "QUEST_LOG_11_2",
            231 => "QUEST_LOG_12_1",
            232 => "QUEST_LOG_12_2",
            234 => "QUEST_LOG_13_1",
            235 => "QUEST_LOG_13_2",
            237 => "QUEST_LOG_14_1",
            238 => "QUEST_LOG_14_2",
            240 => "QUEST_LOG_15_1",
            241 => "QUEST_LOG_15_2",
            243 => "QUEST_LOG_16_1",
            244 => "QUEST_LOG_16_2",
            246 => "QUEST_LOG_17_1",
            247 => "QUEST_LOG_17_2",
            249 => "QUEST_LOG_18_1",
            250 => "QUEST_LOG_18_2",
            252 => "QUEST_LOG_19_1",
            253 => "QUEST_LOG_19_2",
            255 => "QUEST_LOG_20_1",
            256 => "QUEST_LOG_20_2",
            258 => "VISIBLE_ITEM_1_CREATOR",
            260 => "VISIBLE_ITEM_1_0",
            268 => "VISIBLE_ITEM_1_PROPERTIES",
            269 => "VISIBLE_ITEM_1_PAD",
            270 => "VISIBLE_ITEM_2_CREATOR",
            272 => "VISIBLE_ITEM_2_0",
            280 => "VISIBLE_ITEM_2_PROPERTIES",
            281 => "VISIBLE_ITEM_2_PAD",
            282 => "VISIBLE_ITEM_3_CREATOR",
            284 => "VISIBLE_ITEM_3_0",
            292 => "VISIBLE_ITEM_3_PROPERTIES",
            293 => "VISIBLE_ITEM_3_PAD",
            294 => "VISIBLE_ITEM_4_CREATOR",
            296 => "VISIBLE_ITEM_4_0",
            304 => "VISIBLE_ITEM_4_PROPERTIES",
            305 => "VISIBLE_ITEM_4_PAD",
            306 => "VISIBLE_ITEM_5_CREATOR",
            308 => "VISIBLE_ITEM_5_0",
            316 => "VISIBLE_ITEM_5_PROPERTIES",
            317 => "VISIBLE_ITEM_5_PAD",
            318 => "VISIBLE_ITEM_6_CREATOR",
            320 => "VISIBLE_ITEM_6_0",
            328 => "VISIBLE_ITEM_6_PROPERTIES",
            329 => "VISIBLE_ITEM_6_PAD",
            330 => "VISIBLE_ITEM_7_CREATOR",
            332 => "VISIBLE_ITEM_7_0",
            340 => "VISIBLE_ITEM_7_PROPERTIES",
            341 => "VISIBLE_ITEM_7_PAD",
            342 => "VISIBLE_ITEM_8_CREATOR",
            344 => "VISIBLE_ITEM_8_0",
            352 => "VISIBLE_ITEM_8_PROPERTIES",
            353 => "VISIBLE_ITEM_8_PAD",
            354 => "VISIBLE_ITEM_9_CREATOR",
            356 => "VISIBLE_ITEM_9_0",
            364 => "VISIBLE_ITEM_9_PROPERTIES",
            365 => "VISIBLE_ITEM_9_PAD",
            366 => "VISIBLE_ITEM_10_CREATOR",
            368 => "VISIBLE_ITEM_10_0",
            376 => "VISIBLE_ITEM_10_PROPERTIES",
            377 => "VISIBLE_ITEM_10_PAD",
            378 => "VISIBLE_ITEM_11_CREATOR",
            380 => "VISIBLE_ITEM_11_0",
            388 => "VISIBLE_ITEM_11_PROPERTIES",
            389 => "VISIBLE_ITEM_11_PAD",
            390 => "VISIBLE_ITEM_12_CREATOR",
            392 => "VISIBLE_ITEM_12_0",
            400 => "VISIBLE_ITEM_12_PROPERTIES",
            401 => "VISIBLE_ITEM_12_PAD",
            402 => "VISIBLE_ITEM_13_CREATOR",
            404 => "VISIBLE_ITEM_13_0",
            412 => "VISIBLE_ITEM_13_PROPERTIES",
            413 => "VISIBLE_ITEM_13_PAD",
            414 => "VISIBLE_ITEM_14_CREATOR",
            416 => "VISIBLE_ITEM_14_0",
            424 => "VISIBLE_ITEM_14_PROPERTIES",
            425 => "VISIBLE_ITEM_14_PAD",
            426 => "VISIBLE_ITEM_15_CREATOR",
            428 => "VISIBLE_ITEM_15_0",
            436 => "VISIBLE_ITEM_15_PROPERTIES",
            437 => "VISIBLE_ITEM_15_PAD",
            438 => "VISIBLE_ITEM_16_CREATOR",
            440 => "VISIBLE_ITEM_16_0",
            448 => "VISIBLE_ITEM_16_PROPERTIES",
            449 => "VISIBLE_ITEM_16_PAD",
            450 => "VISIBLE_ITEM_17_CREATOR",
            452 => "VISIBLE_ITEM_17_0",
            460 => "VISIBLE_ITEM_17_PROPERTIES",
            461 => "VISIBLE_ITEM_17_PAD",
            462 => "VISIBLE_ITEM_18_CREATOR",
            464 => "VISIBLE_ITEM_18_0",
            472 => "VISIBLE_ITEM_18_PROPERTIES",
            473 => "VISIBLE_ITEM_18_PAD",
            474 => "VISIBLE_ITEM_19_CREATOR",
            476 => "VISIBLE_ITEM_19_0",
            484 => "VISIBLE_ITEM_19_PROPERTIES",
            485 => "VISIBLE_ITEM_19_PAD",
            486 => "INV_SLOT_HEAD",
            532 => "PACK_SLOT_1",
            564 => "BANK_SLOT_1",
            612 => "BANKBAG_SLOT_1",
            624 => "VENDORBUYBACK_SLOT_1",
            648 => "KEYRING_SLOT_1",
            712 => "FARSIGHT",
            714 => "COMBO_TARGET",
            716 => "XP",
            717 => "NEXT_LEVEL_XP",
            718 => "SKILL_INFO_1_1",
            1102 => "CHARACTER_POINTS1",
            1103 => "CHARACTER_POINTS2",
            1104 => "TRACK_CREATURES",
            1105 => "TRACK_RESOURCES",
            1106 => "BLOCK_PERCENTAGE",
            1107 => "DODGE_PERCENTAGE",
            1108 => "PARRY_PERCENTAGE",
            1109 => "CRIT_PERCENTAGE",
            1110 => "RANGED_CRIT_PERCENTAGE",
            1111 => "EXPLORED_ZONES_1",
            1175 => "REST_STATE_EXPERIENCE",
            1176 => "COINAGE",
            1177 => "POSSTAT0",
            1178 => "POSSTAT1",
            1179 => "POSSTAT2",
            1180 => "POSSTAT3",
            1181 => "POSSTAT4",
            1182 => "NEGSTAT0",
            1183 => "NEGSTAT1",
            1184 => "NEGSTAT2",
            1185 => "NEGSTAT3",
            1186 => "NEGSTAT4",
            1187 => "RESISTANCEBUFFMODSPOSITIVE",
            1194 => "RESISTANCEBUFFMODSNEGATIVE",
            1201 => "MOD_DAMAGE_DONE_POS",
            1208 => "MOD_DAMAGE_DONE_NEG",
            1215 => "MOD_DAMAGE_DONE_PCT",
            1222 => "FIELD_BYTES",
            1223 => "AMMO_ID",
            1224 => "SELF_RES_SPELL",
            1225 => "PVP_MEDALS",
            1226 => "BUYBACK_PRICE_1",
            1238 => "BUYBACK_TIMESTAMP_1",
            1250 => "SESSION_KILLS",
            1251 => "YESTERDAY_KILLS",
            1252 => "LAST_WEEK_KILLS",
            1253 => "THIS_WEEK_KILLS",
            1254 => "THIS_WEEK_CONTRIBUTION",
            1255 => "LIFETIME_HONORBALE_KILLS",
            1256 => "LIFETIME_DISHONORBALE_KILLS",
            1257 => "YESTERDAY_CONTRIBUTION",
            1258 => "LAST_WEEK_CONTRIBUTION",
            1259 => "LAST_WEEK_RANK",
            1260 => "BYTES2",
            1261 => "WATCHED_FACTION_INDEX",
            1262 => "COMBAT_RATING_1",
            _ => return None,
        })
    }
}

/// `game_object` update fields.
pub mod game_object {
    /// size 2, GUID
    pub const OBJECT_FIELD_CREATED_BY: u16 = 6;
    /// size 1, INT
    pub const DISPLAYID: u16 = 8;
    /// size 1, INT
    pub const FLAGS: u16 = 9;
    /// size 4, FLOAT
    pub const ROTATION: u16 = 10;
    /// size 1, INT
    pub const STATE: u16 = 14;
    /// size 1, FLOAT
    pub const POS_X: u16 = 15;
    /// size 1, FLOAT
    pub const POS_Y: u16 = 16;
    /// size 1, FLOAT
    pub const POS_Z: u16 = 17;
    /// size 1, FLOAT
    pub const FACING: u16 = 18;
    /// size 1, INT
    pub const DYN_FLAGS: u16 = 19;
    /// size 1, INT
    pub const FACTION: u16 = 20;
    /// size 1, INT
    pub const TYPE_ID: u16 = 21;
    /// size 1, INT
    pub const LEVEL: u16 = 22;
    /// size 1, INT
    pub const ARTKIT: u16 = 23;
    /// size 1, INT
    pub const ANIMPROGRESS: u16 = 24;
    /// size 1, INT
    pub const PADDING: u16 = 25;

    /// Name of a field index within this object type, for logging.
    pub fn name_of(index: u16) -> Option<&'static str> {
        Some(match index {
            6 => "OBJECT_FIELD_CREATED_BY",
            8 => "DISPLAYID",
            9 => "FLAGS",
            10 => "ROTATION",
            14 => "STATE",
            15 => "POS_X",
            16 => "POS_Y",
            17 => "POS_Z",
            18 => "FACING",
            19 => "DYN_FLAGS",
            20 => "FACTION",
            21 => "TYPE_ID",
            22 => "LEVEL",
            23 => "ARTKIT",
            24 => "ANIMPROGRESS",
            25 => "PADDING",
            _ => return None,
        })
    }
}

/// `dynamic_object` update fields.
pub mod dynamic_object {
    /// size 2, GUID
    pub const CASTER: u16 = 6;
    /// size 1, BYTES
    pub const BYTES: u16 = 8;
    /// size 1, INT
    pub const SPELLID: u16 = 9;
    /// size 1, FLOAT
    pub const RADIUS: u16 = 10;
    /// size 1, FLOAT
    pub const POS_X: u16 = 11;
    /// size 1, FLOAT
    pub const POS_Y: u16 = 12;
    /// size 1, FLOAT
    pub const POS_Z: u16 = 13;
    /// size 1, FLOAT
    pub const FACING: u16 = 14;
    /// size 1, BYTES
    pub const PAD: u16 = 15;

    /// Name of a field index within this object type, for logging.
    pub fn name_of(index: u16) -> Option<&'static str> {
        Some(match index {
            6 => "CASTER",
            8 => "BYTES",
            9 => "SPELLID",
            10 => "RADIUS",
            11 => "POS_X",
            12 => "POS_Y",
            13 => "POS_Z",
            14 => "FACING",
            15 => "PAD",
            _ => return None,
        })
    }
}

/// `corpse` update fields.
pub mod corpse {
    /// size 2, GUID
    pub const OWNER: u16 = 6;
    /// size 1, FLOAT
    pub const FACING: u16 = 8;
    /// size 1, FLOAT
    pub const POS_X: u16 = 9;
    /// size 1, FLOAT
    pub const POS_Y: u16 = 10;
    /// size 1, FLOAT
    pub const POS_Z: u16 = 11;
    /// size 1, INT
    pub const DISPLAY_ID: u16 = 12;
    /// size 19, INT
    pub const ITEM: u16 = 13;
    /// size 1, BYTES
    pub const BYTES_1: u16 = 32;
    /// size 1, BYTES
    pub const BYTES_2: u16 = 33;
    /// size 1, INT
    pub const GUILD: u16 = 34;
    /// size 1, INT
    pub const FLAGS: u16 = 35;
    /// size 1, INT
    pub const DYNAMIC_FLAGS: u16 = 36;
    /// size 1, INT
    pub const PAD: u16 = 37;

    /// Name of a field index within this object type, for logging.
    pub fn name_of(index: u16) -> Option<&'static str> {
        Some(match index {
            6 => "OWNER",
            8 => "FACING",
            9 => "POS_X",
            10 => "POS_Y",
            11 => "POS_Z",
            12 => "DISPLAY_ID",
            13 => "ITEM",
            32 => "BYTES_1",
            33 => "BYTES_2",
            34 => "GUILD",
            35 => "FLAGS",
            36 => "DYNAMIC_FLAGS",
            37 => "PAD",
            _ => return None,
        })
    }
}
