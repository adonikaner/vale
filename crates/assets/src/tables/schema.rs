//! The column layout of a DBC table: each column's name, its type, and the
//! table a numeric column refers to.
//!
//! A DBC file states four counts and then bytes. Nothing in the file says that
//! `Spell.dbc`'s field 18 is an index into `SpellCastTimes.dbc` or that field
//! 120 is a string, so each reader in this crate pins its own columns by
//! measurement. [`super::spellbook::spell_fields`] holds sixty of them, each
//! with its measurement written beside it.
//!
//! This module holds that layout as data, for callers that need the whole
//! table rather than the few columns they read: a form that draws all 173
//! fields of `Spell.dbc` needs a type for every field, and a field with no
//! known type is likely to be edited into an invalid value.
//!
//! ## Why the layout is in this crate
//!
//! Which column of `Spell.dbc` holds the cast time is a rule about what a file
//! means, and this crate owns those rules. A crate that writes the bytes or
//! draws a form calls this module rather than carrying a second reading of the
//! layout. The dressing rules are kept out of the renderer for the same reason.
//!
//! ## Field indices and field names come from different sources
//!
//! The field indices are this crate's own measurements. A test checks
//! [`SPELL`] against `spell_fields`, so the two cannot drift. The field names
//! follow vmangos' `SpellEntry.h`, which is the only written source for what
//! these columns are called.
//!
//! vmangos' field numbering is not 1.12's. Its comments number
//! `EffectBasePoints` 76-78 and then `EffectBonusCoefficient` 79-81, a column
//! 1.12 does not have, so every number it gives from 79 on is three too high:
//! it puts the name at 120, and field 120-3=117 in the file is the icon. Take
//! names from vmangos and indices from the measurements.
//!
//! ## Enum names come from the server because the client ships none
//!
//! 1.12.1's `patch.MPQ` deletes `SpellEffectNames.dbc` and
//! `SpellAuraNames.dbc`, so the patched client reads neither.
//! [`super::spellnames`] takes its names from the server's enums for that
//! reason.

use super::spellbits as bits;
use super::spellnames::{SPELL_AURAS, SPELL_EFFECTS};

/// What a field holds, and what a panel may do with it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// Field 0: the row's own id. Editable only by whatever mints new rows.
    Id,
    /// A plain count, a duration in milliseconds, a percentage.
    Int,
    /// An integer whose sign is meaningful. Most of the effect columns are
    /// this kind: `EffectBasePoints` is often negative.
    Signed,
    /// The four bytes are an `f32`.
    Float,
    /// A bit mask, with the names of its bits.
    ///
    /// Drawn as hex, because every mask in this table is read bit by bit and a
    /// decimal rendering of 0x00010040 does not show which bits are set. Where
    /// the names are known it is also drawn as a list of checkboxes, one per
    /// bit. Each entry is `(mask, name, note)`; [`super::spellbits`] is where
    /// the entries come from and says why the server is the authority for them.
    ///
    /// An empty slice means no source names the bits of this mask. That is
    /// different from a mask with no bits: the field is still a mask and is
    /// still drawn as hex, with no list under it. Names are not invented here,
    /// because an invented name would be an unchecked claim about the game.
    Flags(&'static [(u32, &'static str, &'static str)]),
    /// A column whose only values are 0 and 1, and which gates the columns
    /// beside it. A column is given this kind only after its values are
    /// measured over the whole population: a flag with a third value is an
    /// `Int` that has been misread.
    Bool,
    /// One of a named set. The list is not required to be complete: a value it
    /// does not name is shown as the number, which is what the game does.
    Enum(&'static [(u32, &'static str)]),
    /// A row id in another table, named without its `.dbc`. A table browser
    /// follows this column on a click and builds reverse lookups from it.
    Reference(&'static str),
    /// An item's entry. 1.12 ships no item table: an item is a row the server
    /// answers a query with, so nothing in the archives resolves this number.
    /// A form with a world database names it from `item_template`; one
    /// without draws the number.
    Item,
    /// A colour packed one byte per channel, red first: `0x00RRGGBB`.
    ///
    /// Separate from [`Flags`](Self::Flags) because it is drawn as a swatch
    /// and three channel numbers, and because the packing order is a
    /// measurement rather than a convention: see [`super::light`], where
    /// reading it in the other order turns map 0's noon sun pale blue.
    ///
    /// The top byte is preserved, not masked away. It is zero in 17,533 of the
    /// 17,535 colours `LightIntBand` uses; the other two are in one
    /// hand-authored row carrying `0xffefe4ff`, and a writer that cleared the
    /// byte would change that row when nobody edited it.
    Colour,
    /// A byte offset into this table's own string block: the enUS column.
    Text,
    /// One of the seven locale columns after the enUS column. 1.12 ships them
    /// empty in an enUS install, but they are real columns and must not be
    /// written over.
    Locale(&'static str),
    /// The word after a locale run: one bit per locale, set when that locale's
    /// string is present.
    LocaleFlags,
    /// A column the client does not read. It is still named, because a field
    /// with no name is easy to edit by mistake.
    Unused,
}

/// One column.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    /// Its index, which is also its position in [`Schema::columns`].
    pub field: usize,
    pub name: &'static str,
    pub kind: Kind,
    /// What is known about the column and how it was established, shown as a
    /// tooltip.
    ///
    /// Empty for a column whose name says everything. Otherwise it carries the
    /// measurement and states whether the claim is a measurement or an
    /// interpretation. A column pinned by what it
    /// resolves to over the whole population is a stronger claim than one
    /// named after its neighbours, and a form shows the difference to the
    /// person about to type a number into the field.
    pub about: &'static str,
}

/// A run of columns a form draws together.
///
/// A list of fields rather than a range, because the three effect slots are
/// interleaved: slot 0 is fields 61, 64, 67 and so on, so no range describes
/// one slot. See [`SPELL_SECTIONS`].
#[derive(Debug, Clone, Copy)]
pub struct Section {
    pub name: &'static str,
    pub fields: &'static [usize],
}

/// A table's whole layout.
#[derive(Debug, Clone, Copy)]
pub struct Schema {
    /// The table's name, without `.dbc`.
    pub table: &'static str,
    /// Every column, indexed by field.
    pub columns: &'static [Column],
    /// How a form groups the columns.
    pub sections: &'static [Section],
}

impl Schema {
    pub fn column(&self, field: usize) -> Option<&'static Column> {
        self.columns.get(field)
    }

    /// The column a row is named by: the first text column, which is the
    /// name in the table's own locale.
    pub fn name_field(&self) -> Option<usize> {
        self.columns
            .iter()
            .find(|column| column.kind == Kind::Text)
            .map(|column| column.field)
    }

    /// Every field that points at `table`, for a reverse lookup.
    pub fn references_to(&self, table: &str) -> Vec<usize> {
        self.columns
            .iter()
            .filter(|column| matches!(column.kind, Kind::Reference(at) if at == table))
            .map(|column| column.field)
            .collect()
    }
}

/// The names of a mask's set bits, in the order the list declares them.
///
/// This is the one-line reading of a [`Kind::Flags`] column, for a form with
/// room for a line of text but not for a dialog, and for `vale dbc`, which
/// prints a row and has no dialog.
///
/// A set bit the list does not name is not in the result; [`unnamed_bits`]
/// returns those. The two results together make up the whole value. They are
/// kept separate because unnamed bits are a gap in the names and named bits
/// are not.
pub fn named_bits(value: u32, bits: &[(u32, &'static str, &'static str)]) -> Vec<&'static str> {
    bits.iter()
        .filter(|(mask, _, _)| value & mask != 0)
        .map(|(_, name, _)| *name)
        .collect()
}

/// The set bits of a mask that the list does not name, as a mask.
///
/// Zero when the list accounts for the whole value. Non-zero means either a bit
/// no source has named yet or a bit somebody typed by hand. Both are shown
/// rather than discarded.
pub fn unnamed_bits(value: u32, bits: &[(u32, &'static str, &'static str)]) -> u32 {
    value & !bits.iter().fold(0u32, |all, (mask, _, _)| all | mask)
}

/// The schema for a table, or `None` for a table this module does not describe.
pub fn for_table(name: &str) -> Option<&'static Schema> {
    ALL.iter()
        .find(|schema| schema.table.eq_ignore_ascii_case(name))
        .copied()
}

/// The column a row of `table` is named by: its schema's first text column.
/// `None` for a table with no schema and for a schema with no text column.
pub fn name_field(table: &str) -> Option<usize> {
    for_table(table)?.name_field()
}

/// The tables a table's rows are read with, beyond the table itself: the
/// tables its reference columns lead to and the tables its rows are labelled
/// from.
///
/// There are three chains (the spell chain, the light chain and the flight
/// path tables) and the tables a skill table or an item set is labelled
/// from. A spell can be read from `Spell.dbc` alone, but its visual cannot,
/// and neither can a followed reference or a row labelled from the rows it
/// references. Together these tables are smaller than `Spell.dbc`'s string
/// block.
pub fn chain_for(table: &str) -> &'static [&'static str] {
    match table {
        "Spell" => &[
            "SpellVisual",
            "SpellVisualKit",
            "SpellVisualEffectName",
            "AnimationData",
            "SpellIcon",
            "SpellCastTimes",
            "SpellDuration",
            "SpellRadius",
            "SpellRange",
            "SoundEntries",
            "SpellEffectCameraShakes",
            // The skill tables. With `SkillLineAbility` read, a spell's
            // reverse references list the skill lines it is in.
            "SkillLine",
            "SkillLineAbility",
            "SkillRaceClassInfo",
            "SkillLineCategory",
        ],
        "Light" => &[
            "LightParams",
            "LightIntBand",
            "LightFloatBand",
            "LightSkybox",
            // `Map` is not part of the light chain but is read with it, so a
            // light's label can show `Azeroth` instead of `map 0`. `Light`
            // rows have no names, so the map is most of what the label shows.
            "Map",
        ],
        // The paths and their points, which a node lists, and `Map` for the
        // same reason the light chain reads it.
        "TaxiNodes" => &["TaxiPath", "TaxiPathNode", "Map"],
        // A set's bonuses are spells and its requirement is a skill line.
        "ItemSet" => &["Spell", "SkillLine"],
        // The creature display chain: what a display's model and extra
        // columns name, the items an extra row dresses the creature in, and
        // the two sound tables with the sounds they name, so a sound column
        // reads as the sound's name.
        "CreatureDisplayInfo"
        | "CreatureDisplayInfoExtra"
        | "CreatureModelData"
        | "CreatureSoundData"
        | "NPCSounds" => &[
            "CreatureDisplayInfo",
            "CreatureDisplayInfoExtra",
            "CreatureModelData",
            "ItemDisplayInfo",
            "CreatureSoundData",
            "NPCSounds",
            "SoundEntries",
        ],
        // A game object's ten sounds.
        "GameObjectDisplayInfo" => &["SoundEntries"],
        // Every table with a column of `SoundEntries` ids, so the reverse
        // references of a `SoundEntries` row are complete: `CreatureSoundData`
        // and `NPCSounds` with the displays and models that refer to them,
        // `ZoneMusic`, `ZoneIntroMusicTable` and `SoundAmbience` with the
        // areas that refer to them, `GameObjectDisplayInfo` and
        // `SpellVisualKit`.
        "SoundEntries" => &[
            "CreatureSoundData",
            "NPCSounds",
            "ZoneMusic",
            "ZoneIntroMusicTable",
            "SoundAmbience",
            "AreaTable",
            "CreatureDisplayInfo",
            "CreatureModelData",
            "GameObjectDisplayInfo",
            "SpellVisualKit",
            "ItemGroupSounds",
        ],
        // The visual an item's glow is, and its pick-up and put-down sounds
        // with the `SoundEntries` rows they name, so the sound column reads
        // as the sound's name.
        "ItemDisplayInfo" => &["SpellVisual", "ItemGroupSounds", "SoundEntries"],
        // The skill tables, read without the spell chain: each is labelled
        // from the others.
        "SkillLine" => &["SkillLineCategory", "SpellIcon"],
        "SkillLineAbility" | "SkillRaceClassInfo" => &["SkillLine", "Spell"],
        // `Map`, which an area's label names, and the five tables its sound
        // and liquid columns refer to, so each reference is drawn as a name.
        //
        // `SoundEntries` as well, which the music, intro and ambience rows
        // name, so a sound reference reads as the sound's name.
        // `SoundEntries`, which a `ZoneMusic` row's two sound columns refer
        // to, and `AreaTable`, whose rows refer to the `ZoneMusic` row.
        "ZoneMusic" => &["SoundEntries", "AreaTable"],
        // The same two tables, for the same reason.
        "ZoneIntroMusicTable" | "SoundAmbience" => &["SoundEntries", "AreaTable"],
        // An item's sounds, and the item displays that refer to them.
        "ItemGroupSounds" => &["SoundEntries", "ItemDisplayInfo"],
        "AreaTable" => &[
            "Map",
            "ZoneMusic",
            "ZoneIntroMusicTable",
            "SoundAmbience",
            "SoundProviderPreferences",
            "LiquidType",
            "SoundEntries",
        ],
        _ => &[],
    }
}

/// What a blank row of a table holds beyond zeros: `(field, value)` pairs.
///
/// A kit's model columns and its four procedural slots are `-1` for none in
/// the shipped rows (a reverse reference treats 0 and -1 alike), and a blank kit
/// with 0 in `Animation` would resolve to Stand; see the schema's note on that
/// column. In every other table, zero means none.
pub fn blank_defaults(table: &str) -> &'static [(usize, u32)] {
    const KIT: &[(usize, u32)] = &[
        (super::spell::fields::ANIMATION, u32::MAX),
        (3, u32::MAX),
        (4, u32::MAX),
        (5, u32::MAX),
        (6, u32::MAX),
        (7, u32::MAX),
        (8, u32::MAX),
        (9, u32::MAX),
        (10, u32::MAX),
        (12, u32::MAX),
        (15, u32::MAX),
        (16, u32::MAX),
        (17, u32::MAX),
        (18, u32::MAX),
    ];
    match table {
        "SpellVisualKit" => KIT,
        _ => &[],
    }
}

/// Every table this crate can describe.
///
/// The list has three groups.
///
/// The spell group is the `Spell` row and the chain under it, which a spell's
/// visual is built from: a spell names a `SpellVisual`, that names five
/// `SpellVisualKit`s, and each kit names models in `SpellVisualEffectName`, an
/// animation in `AnimationData`, a sound and a camera shake. Every link is a
/// reference column here, so the browser follows it and the storyboard reads
/// it.
///
/// The light group: a `Light` is a sphere on a map naming five `LightParams`
/// rows, each of which owns 18 `LightIntBand` rows and 6 `LightFloatBand` rows.
/// The link from params to bands is the only join in the spell and light
/// groups that is not a reference column. A band is found by arithmetic on
/// its row position, so a browser cannot follow it;
/// [`super::light::int_band_row`] computes it.
///
/// The flight path group: `TaxiNodes`, `TaxiPath` and `TaxiPathNode`.
/// `TaxiPath.From` and `TaxiPath.To` refer to `TaxiNodes`, and
/// `TaxiPathNode.Path` refers to `TaxiPath`.
///
/// The skill group: `SkillLine` is a skill, `SkillLineAbility` says which
/// spells belong to it and for whom, and `SkillRaceClassInfo` says which races
/// and classes have it. Both of the last two refer to `SkillLine`, and
/// `SkillLineAbility` refers to `Spell` twice.
///
/// `ItemSet` stands alone: its items are [`Kind::Item`], its bonuses refer to
/// `Spell` and its requirement to `SkillLine`.
///
/// `AreaTable` stands alone too: an area refers to its `Map`, to the zone
/// that encloses it in the same table, and to the sound and music tables.
/// Three of those follow it: `ZoneMusic`, `ZoneIntroMusicTable` and
/// `SoundAmbience`, each of which refers to `SoundEntries`.
///
/// The place group: `Map`, the maps themselves; `AreaTrigger`, the volumes a
/// character reports standing in; and `WorldSafeLocs`, the places a spirit
/// appears. The last two refer to `Map`, and `Map` refers to `AreaTable` and
/// `LoadingScreens`.
///
/// The display group: what a creature, a game object and an item look like.
/// `CreatureDisplayInfo` refers to `CreatureModelData` and
/// `CreatureDisplayInfoExtra`, which refers to `ItemDisplayInfo` ten times.
/// A creature's sounds are `CreatureSoundData`, which both the display and its
/// model refer to, and `NPCSounds`, which the display refers to; both refer
/// to `SoundEntries`.
pub const ALL: &[&Schema] = &[
    &SPELL,
    &SPELL_VISUAL,
    &SPELL_VISUAL_KIT,
    &SPELL_VISUAL_EFFECT_NAME,
    &ANIMATION_DATA,
    &SPELL_ICON,
    &SPELL_CAST_TIMES,
    &SPELL_DURATION,
    &SPELL_RADIUS,
    &SPELL_CHAIN_EFFECTS,
    &SPELL_EFFECT_CAMERA_SHAKES,
    &SOUND_ENTRIES,
    &SPELL_RANGE,
    &SPELL_DISPEL_TYPE,
    &SPELL_MECHANIC,
    &SPELL_FOCUS_OBJECT,
    &SPELL_CATEGORY,
    &LIGHT,
    &LIGHT_PARAMS,
    &LIGHT_INT_BAND,
    &LIGHT_FLOAT_BAND,
    &LIGHT_SKYBOX,
    &TAXI_NODES,
    &TAXI_PATH,
    &TAXI_PATH_NODE,
    &SKILL_LINE,
    &SKILL_LINE_ABILITY,
    &SKILL_RACE_CLASS_INFO,
    &ITEM_SET,
    &AREA_TABLE,
    &ZONE_MUSIC,
    &ZONE_INTRO_MUSIC,
    &SOUND_AMBIENCE,
    &ITEM_GROUP_SOUNDS,
    &MAP,
    &AREA_TRIGGER,
    &WORLD_SAFE_LOCS,
    &CREATURE_DISPLAY_INFO,
    &CREATURE_DISPLAY_INFO_EXTRA,
    &CREATURE_MODEL_DATA,
    &CREATURE_SOUND_DATA,
    &NPC_SOUNDS,
    &GAME_OBJECT_DISPLAY_INFO,
    &ITEM_DISPLAY_INFO,
];

/// Every file in 1.12.1's `DBFilesClient\`, by bare name: 158 of them, four
/// of which `patch.MPQ` deletes, so the patched client reads no table from
/// them (`CharacterCreateCameras`, `SoundCharacterMacroLines`,
/// `SpellAuraNames`, `SpellEffectNames`). See `archive::DELETE_MARKER`.
///
/// The archives list their files in lower case, and a table is opened, saved
/// and looked up by its name, so one spelling has to be the name. This is the
/// spelling the readers in this crate and vmangos' `DBCStores.cpp` use.
/// [`table_name`] turns any spelling into it.
pub const TABLE_NAMES: [&str; 158] = [
    "AnimationData", "AreaPOI", "AreaTable", "AreaTrigger", "AttackAnimKits",
    "AttackAnimTypes", "AuctionHouse", "BankBagSlotPrices", "CameraShakes",
    "Cfg_Categories", "Cfg_Configs", "CharacterCreateCameras",
    "CharacterFacialHairStyles", "CharBaseInfo", "CharHairGeosets",
    "CharHairTextures", "CharSections", "CharStartOutfit", "CharVariations",
    "ChatChannels", "ChatProfanity", "ChrClasses", "ChrRaces", "CinematicCamera",
    "CinematicSequences", "CreatureDisplayInfo", "CreatureDisplayInfoExtra",
    "CreatureFamily", "CreatureModelData", "CreatureSoundData",
    "CreatureSpellData", "CreatureType", "DeathThudLookups", "DurabilityCosts",
    "DurabilityQuality", "Emotes", "EmotesText", "EmotesTextData",
    "EmotesTextSound", "EnvironmentalDamage", "Exhaustion", "Faction",
    "FactionGroup", "FactionTemplate", "FootprintTextures",
    "FootstepTerrainLookup", "GameObjectArtKit", "GameObjectDisplayInfo",
    "GameTips", "GMSurveyCurrentSurvey", "GMSurveyQuestions", "GMSurveySurveys",
    "GMTicketCategory", "GroundEffectDoodad", "GroundEffectTexture",
    "HelmetGeosetVisData", "ItemBagFamily", "ItemClass", "ItemDisplayInfo",
    "ItemGroupSounds", "ItemPetFood", "ItemRandomProperties", "ItemSet",
    "ItemSubClass", "ItemSubClassMask", "ItemVisualEffects", "ItemVisuals",
    "Languages", "LanguageWords", "LFGDungeons", "Light", "LightFloatBand",
    "LightIntBand", "LightParams", "LightSkybox", "LiquidType", "LoadingScreens",
    "LoadingScreenTaxiSplines", "Lock", "LockType", "MailTemplate", "Map",
    "Material", "NameGen", "NamesProfanity", "NamesReserved", "NPCSounds",
    "Package", "PageTextMaterial", "PaperDollItemFrame", "PetLoyalty",
    "PetPersonality", "QuestInfo", "QuestSort", "Resistances", "ServerMessages",
    "SheatheSoundLookups", "SkillCostsData", "SkillLine", "SkillLineAbility",
    "SkillLineCategory", "SkillRaceClassInfo", "SkillTiers", "SoundAmbience",
    "SoundCharacterMacroLines", "SoundEntries", "SoundProviderPreferences",
    "SoundSamplePreferences", "SoundWaterType", "SpamMessages", "Spell",
    "SpellAuraNames", "SpellCastTimes", "SpellCategory", "SpellChainEffects",
    "SpellDispelType", "SpellDuration", "SpellEffectCameraShakes",
    "SpellEffectNames", "SpellFocusObject", "SpellIcon", "SpellItemEnchantment",
    "SpellMechanic", "SpellRadius", "SpellRange", "SpellShapeshiftForm",
    "SpellVisual", "SpellVisualEffectName", "SpellVisualKit",
    "SpellVisualPrecastTransitions", "StableSlotPrices", "Startup_Strings",
    "Stationery", "StringLookups", "Talent", "TalentTab", "TaxiNodes", "TaxiPath",
    "TaxiPathNode", "TerrainType", "TerrainTypeSounds", "TransportAnimation",
    "UISoundLookups", "UnitBlood", "UnitBloodLevels", "VideoHardware",
    "VocalUISounds", "WeaponImpactSounds", "WeaponSwingSounds2", "WMOAreaTable",
    "WorldMapArea", "WorldMapContinent", "WorldMapOverlay", "WorldSafeLocs",
    "WorldStateUI", "WowError_Strings", "ZoneIntroMusicTable", "ZoneMusic",
];

/// The name a table goes by, whatever case it was asked for in: the entry of
/// [`TABLE_NAMES`] that matches, or the name as given for a table the list
/// does not hold, which a patch archive may add.
pub fn table_name(name: &str) -> &str {
    TABLE_NAMES
        .iter()
        .find(|known| known.eq_ignore_ascii_case(name))
        .copied()
        .unwrap_or(name)
}

const fn c(field: usize, name: &'static str, kind: Kind) -> Column {
    Column {
        field,
        name,
        kind,
        about: "",
    }
}

/// A column with an `about` note.
const fn ca(field: usize, name: &'static str, kind: Kind, about: &'static str) -> Column {
    Column {
        field,
        name,
        kind,
        about,
    }
}

/// `Spell.dbc`'s icon column, for a caller that wants the picture rather than
/// the form: field 117, a `SpellIcon.dbc` id.
///
/// A constant rather than a lookup by column name, because a name is a string
/// that can be mistyped and this field is read on every row of a list. It is
/// the same number as `spellbook::spell_fields::ICON_ID`, which the schema's
/// own test asserts.
pub const SPELL_ICON_FIELD: usize = 117;

/// `Spell.dbc`: 22,360 rows of 173 fields.
pub const SPELL: Schema = Schema {
    table: "Spell",
    columns: &SPELL_COLUMNS,
    sections: SPELL_SECTIONS,
};

/// The eight locale columns every string in this table has.
///
/// 1.12 ships the seven after enUS empty in an enUS install, but they are still
/// columns. A writer that puts a new name in field 120 and leaves 121..127
/// alone is correct; one that writes across them destroys a localised
/// install's text.
const LOCALES: [&str; 8] = [
    "enUS", "koKR", "frFR", "deDE", "enCN", "enTW", "esES", "esMX",
];

const SPELL_COLUMNS: [Column; 173] = [
    c(0, "Id", Kind::Id),
    c(1, "School", Kind::Enum(SCHOOLS)),
    c(2, "Category", Kind::Reference("SpellCategory")),
    c(3, "CastUI", Kind::Int),
    c(4, "Dispel", Kind::Reference("SpellDispelType")),
    c(5, "Mechanic", Kind::Reference("SpellMechanic")),
    c(6, "Attributes", Kind::Flags(&bits::ATTRIBUTES)),
    c(7, "AttributesEx", Kind::Flags(&bits::ATTRIBUTES_EX)),
    c(8, "AttributesEx2", Kind::Flags(&bits::ATTRIBUTES_EX2)),
    c(9, "AttributesEx3", Kind::Flags(&bits::ATTRIBUTES_EX3)),
    c(10, "AttributesEx4", Kind::Flags(&bits::ATTRIBUTES_EX4)),
    c(11, "Stances", Kind::Flags(&bits::SHAPESHIFT)),
    c(12, "StancesNot", Kind::Flags(&bits::SHAPESHIFT)),
    c(13, "Targets", Kind::Flags(&bits::TARGET_FLAGS)),
    c(14, "TargetCreatureType", Kind::Flags(&bits::CREATURE_TYPES)),
    c(15, "RequiresSpellFocus", Kind::Reference("SpellFocusObject")),
    c(16, "CasterAuraState", Kind::Enum(&bits::AURA_STATES)),
    c(17, "TargetAuraState", Kind::Enum(&bits::AURA_STATES)),
    c(18, "CastingTimeIndex", Kind::Reference("SpellCastTimes")),
    c(19, "RecoveryTime", Kind::Int),
    c(20, "CategoryRecoveryTime", Kind::Int),
    c(21, "InterruptFlags", Kind::Flags(&bits::INTERRUPT_FLAGS)),
    c(22, "AuraInterruptFlags", Kind::Flags(&bits::AURA_INTERRUPT_FLAGS)),
    c(23, "ChannelInterruptFlags", Kind::Flags(&[])),
    c(24, "ProcFlags", Kind::Flags(&bits::PROC_FLAGS)),
    c(25, "ProcChance", Kind::Int),
    c(26, "ProcCharges", Kind::Int),
    c(27, "MaxLevel", Kind::Int),
    c(28, "BaseLevel", Kind::Int),
    c(29, "SpellLevel", Kind::Int),
    c(30, "DurationIndex", Kind::Reference("SpellDuration")),
    c(31, "PowerType", Kind::Enum(POWERS)),
    c(32, "ManaCost", Kind::Int),
    c(33, "ManaCostPerLevel", Kind::Int),
    c(34, "ManaPerSecond", Kind::Int),
    c(35, "ManaPerSecondPerLevel", Kind::Int),
    c(36, "RangeIndex", Kind::Reference("SpellRange")),
    c(37, "Speed", Kind::Float),
    c(38, "ModalNextSpell", Kind::Reference("Spell")),
    c(39, "StackAmount", Kind::Int),
    c(40, "Totem 1", Kind::Int),
    c(41, "Totem 2", Kind::Int),
    c(42, "Reagent 1", Kind::Signed),
    c(43, "Reagent 2", Kind::Signed),
    c(44, "Reagent 3", Kind::Signed),
    c(45, "Reagent 4", Kind::Signed),
    c(46, "Reagent 5", Kind::Signed),
    c(47, "Reagent 6", Kind::Signed),
    c(48, "Reagent 7", Kind::Signed),
    c(49, "Reagent 8", Kind::Signed),
    c(50, "ReagentCount 1", Kind::Int),
    c(51, "ReagentCount 2", Kind::Int),
    c(52, "ReagentCount 3", Kind::Int),
    c(53, "ReagentCount 4", Kind::Int),
    c(54, "ReagentCount 5", Kind::Int),
    c(55, "ReagentCount 6", Kind::Int),
    c(56, "ReagentCount 7", Kind::Int),
    c(57, "ReagentCount 8", Kind::Int),
    c(58, "EquippedItemClass", Kind::Enum(&bits::ITEM_CLASSES)),
    c(59, "EquippedItemSubClassMask", Kind::Flags(&[])),
    c(60, "EquippedItemInventoryTypeMask", Kind::Flags(&[])),
    c(61, "Effect 1", Kind::Enum(SPELL_EFFECTS_TABLE)),
    c(62, "Effect 2", Kind::Enum(SPELL_EFFECTS_TABLE)),
    c(63, "Effect 3", Kind::Enum(SPELL_EFFECTS_TABLE)),
    c(64, "EffectDieSides 1", Kind::Signed),
    c(65, "EffectDieSides 2", Kind::Signed),
    c(66, "EffectDieSides 3", Kind::Signed),
    c(67, "EffectBaseDice 1", Kind::Int),
    c(68, "EffectBaseDice 2", Kind::Int),
    c(69, "EffectBaseDice 3", Kind::Int),
    c(70, "EffectDicePerLevel 1", Kind::Float),
    c(71, "EffectDicePerLevel 2", Kind::Float),
    c(72, "EffectDicePerLevel 3", Kind::Float),
    c(73, "EffectRealPointsPerLevel 1", Kind::Float),
    c(74, "EffectRealPointsPerLevel 2", Kind::Float),
    c(75, "EffectRealPointsPerLevel 3", Kind::Float),
    c(76, "EffectBasePoints 1", Kind::Signed),
    c(77, "EffectBasePoints 2", Kind::Signed),
    c(78, "EffectBasePoints 3", Kind::Signed),
    c(79, "EffectMechanic 1", Kind::Reference("SpellMechanic")),
    c(80, "EffectMechanic 2", Kind::Reference("SpellMechanic")),
    c(81, "EffectMechanic 3", Kind::Reference("SpellMechanic")),
    c(82, "EffectImplicitTargetA 1", Kind::Enum(&bits::IMPLICIT_TARGETS)),
    c(83, "EffectImplicitTargetA 2", Kind::Enum(&bits::IMPLICIT_TARGETS)),
    c(84, "EffectImplicitTargetA 3", Kind::Enum(&bits::IMPLICIT_TARGETS)),
    c(85, "EffectImplicitTargetB 1", Kind::Enum(&bits::IMPLICIT_TARGETS)),
    c(86, "EffectImplicitTargetB 2", Kind::Enum(&bits::IMPLICIT_TARGETS)),
    c(87, "EffectImplicitTargetB 3", Kind::Enum(&bits::IMPLICIT_TARGETS)),
    c(88, "EffectRadiusIndex 1", Kind::Reference("SpellRadius")),
    c(89, "EffectRadiusIndex 2", Kind::Reference("SpellRadius")),
    c(90, "EffectRadiusIndex 3", Kind::Reference("SpellRadius")),
    c(91, "EffectApplyAuraName 1", Kind::Enum(SPELL_AURAS_TABLE)),
    c(92, "EffectApplyAuraName 2", Kind::Enum(SPELL_AURAS_TABLE)),
    c(93, "EffectApplyAuraName 3", Kind::Enum(SPELL_AURAS_TABLE)),
    c(94, "EffectAmplitude 1", Kind::Int),
    c(95, "EffectAmplitude 2", Kind::Int),
    c(96, "EffectAmplitude 3", Kind::Int),
    c(97, "EffectMultipleValue 1", Kind::Float),
    c(98, "EffectMultipleValue 2", Kind::Float),
    c(99, "EffectMultipleValue 3", Kind::Float),
    c(100, "EffectChainTarget 1", Kind::Int),
    c(101, "EffectChainTarget 2", Kind::Int),
    c(102, "EffectChainTarget 3", Kind::Int),
    c(103, "EffectItemType 1", Kind::Int),
    c(104, "EffectItemType 2", Kind::Int),
    c(105, "EffectItemType 3", Kind::Int),
    c(106, "EffectMiscValue 1", Kind::Signed),
    c(107, "EffectMiscValue 2", Kind::Signed),
    c(108, "EffectMiscValue 3", Kind::Signed),
    c(109, "EffectTriggerSpell 1", Kind::Reference("Spell")),
    c(110, "EffectTriggerSpell 2", Kind::Reference("Spell")),
    c(111, "EffectTriggerSpell 3", Kind::Reference("Spell")),
    c(112, "EffectPointsPerComboPoint 1", Kind::Float),
    c(113, "EffectPointsPerComboPoint 2", Kind::Float),
    c(114, "EffectPointsPerComboPoint 3", Kind::Float),
    c(115, "SpellVisual", Kind::Reference("SpellVisual")),
    c(116, "SpellVisual2", Kind::Reference("SpellVisual")),
    c(117, "SpellIconID", Kind::Reference("SpellIcon")),
    c(118, "ActiveIconID", Kind::Reference("SpellIcon")),
    c(119, "SpellPriority", Kind::Int),
    c(120, "Name", Kind::Text),
    c(121, "Name koKR", Kind::Locale(LOCALES[1])),
    c(122, "Name frFR", Kind::Locale(LOCALES[2])),
    c(123, "Name deDE", Kind::Locale(LOCALES[3])),
    c(124, "Name enCN", Kind::Locale(LOCALES[4])),
    c(125, "Name enTW", Kind::Locale(LOCALES[5])),
    c(126, "Name esES", Kind::Locale(LOCALES[6])),
    c(127, "Name esMX", Kind::Locale(LOCALES[7])),
    c(128, "NameFlags", Kind::LocaleFlags),
    c(129, "Rank", Kind::Text),
    c(130, "Rank koKR", Kind::Locale(LOCALES[1])),
    c(131, "Rank frFR", Kind::Locale(LOCALES[2])),
    c(132, "Rank deDE", Kind::Locale(LOCALES[3])),
    c(133, "Rank enCN", Kind::Locale(LOCALES[4])),
    c(134, "Rank enTW", Kind::Locale(LOCALES[5])),
    c(135, "Rank esES", Kind::Locale(LOCALES[6])),
    c(136, "Rank esMX", Kind::Locale(LOCALES[7])),
    c(137, "RankFlags", Kind::LocaleFlags),
    c(138, "Description", Kind::Text),
    c(139, "Description koKR", Kind::Locale(LOCALES[1])),
    c(140, "Description frFR", Kind::Locale(LOCALES[2])),
    c(141, "Description deDE", Kind::Locale(LOCALES[3])),
    c(142, "Description enCN", Kind::Locale(LOCALES[4])),
    c(143, "Description enTW", Kind::Locale(LOCALES[5])),
    c(144, "Description esES", Kind::Locale(LOCALES[6])),
    c(145, "Description esMX", Kind::Locale(LOCALES[7])),
    c(146, "DescriptionFlags", Kind::LocaleFlags),
    c(147, "AuraDescription", Kind::Text),
    c(148, "AuraDescription koKR", Kind::Locale(LOCALES[1])),
    c(149, "AuraDescription frFR", Kind::Locale(LOCALES[2])),
    c(150, "AuraDescription deDE", Kind::Locale(LOCALES[3])),
    c(151, "AuraDescription enCN", Kind::Locale(LOCALES[4])),
    c(152, "AuraDescription enTW", Kind::Locale(LOCALES[5])),
    c(153, "AuraDescription esES", Kind::Locale(LOCALES[6])),
    c(154, "AuraDescription esMX", Kind::Locale(LOCALES[7])),
    c(155, "AuraDescriptionFlags", Kind::LocaleFlags),
    c(156, "ManaCostPercentage", Kind::Int),
    c(157, "StartRecoveryCategory", Kind::Int),
    c(158, "StartRecoveryTime", Kind::Int),
    c(159, "MaxTargetLevel", Kind::Int),
    c(160, "SpellFamilyName", Kind::Enum(&bits::SPELL_FAMILIES)),
    c(161, "SpellFamilyFlags low", Kind::Flags(&[])),
    c(162, "SpellFamilyFlags high", Kind::Flags(&[])),
    c(163, "MaxAffectedTargets", Kind::Int),
    c(164, "DmgClass", Kind::Enum(DAMAGE_CLASSES)),
    c(165, "PreventionType", Kind::Enum(PREVENTION)),
    c(166, "StanceBarOrder", Kind::Signed),
    c(167, "DmgMultiplier 1", Kind::Float),
    c(168, "DmgMultiplier 2", Kind::Float),
    c(169, "DmgMultiplier 3", Kind::Float),
    c(170, "MinFactionId", Kind::Reference("Faction")),
    c(171, "MinReputation", Kind::Int),
    c(172, "RequiredAuraVision", Kind::Int),
];

/// The fields of one effect slot. Each of the three slots has the same
/// eighteen fields, interleaved with the other two slots at a stride of three.
const fn effect_slot(slot: usize) -> [usize; 18] {
    [
        61 + slot,
        64 + slot,
        67 + slot,
        70 + slot,
        73 + slot,
        76 + slot,
        79 + slot,
        82 + slot,
        85 + slot,
        88 + slot,
        91 + slot,
        94 + slot,
        97 + slot,
        100 + slot,
        103 + slot,
        106 + slot,
        109 + slot,
        112 + slot,
    ]
}

const EFFECT_1: [usize; 18] = effect_slot(0);
const EFFECT_2: [usize; 18] = effect_slot(1);
const EFFECT_3: [usize; 18] = effect_slot(2);

const SPELL_SECTIONS: &[Section] = &[
    Section {
        name: "Identity",
        fields: &[0, 120, 129, 117, 118, 119, 2, 3],
    },
    Section {
        name: "Text",
        fields: &[
            138, 147, 121, 122, 123, 124, 125, 126, 127, 128, 130, 131, 132, 133, 134, 135, 136,
            137, 139, 140, 141, 142, 143, 144, 145, 146, 148, 149, 150, 151, 152, 153, 154, 155,
        ],
    },
    Section {
        name: "Casting",
        fields: &[18, 36, 19, 20, 157, 158, 30, 39, 37, 38, 21, 22, 23],
    },
    Section {
        name: "Cost",
        fields: &[31, 32, 33, 34, 35, 156, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 40, 41],
    },
    Section {
        name: "Targeting",
        fields: &[13, 14, 15, 16, 17, 159, 163, 11, 12, 58, 59, 60],
    },
    Section {
        name: "Levels and proc",
        fields: &[27, 28, 29, 24, 25, 26],
    },
    Section {
        name: "Attributes",
        fields: &[6, 7, 8, 9, 10, 1, 4, 5, 164, 165, 166, 160, 161, 162, 170, 171, 172],
    },
    Section {
        name: "Effect 1",
        fields: &EFFECT_1,
    },
    Section {
        name: "Effect 2",
        fields: &EFFECT_2,
    },
    Section {
        name: "Effect 3",
        fields: &EFFECT_3,
    },
    Section {
        name: "Visual",
        fields: &[115, 116, 167, 168, 169],
    },
];


/// `SpellVisual.dbc`: 2,167 rows of 16 fields. One row is the whole visual of
/// a spell, and the row the storyboard shows.
///
/// A row names five kits, a missile and an area; a 1.12 spell's appearance is
/// made of nothing else. Every index is taken from `tables::spell::fields`,
/// and the test below asserts it.
pub const SPELL_VISUAL: Schema = Schema {
    table: "SpellVisual",
    columns: &SPELL_VISUAL_COLUMNS,
    sections: &[
        Section {
            name: "Kits",
            fields: &[0, 1, 2, 3, 4, 5],
        },
        Section {
            name: "Missile",
            fields: &[6, 7, 8, 9, 10],
        },
        Section {
            name: "Area",
            fields: &[11, 12, 13, 14, 15],
        },
    ],
};

const KIT: Kind = Kind::Reference("SpellVisualKit");
const MODEL: Kind = Kind::Reference("SpellVisualEffectName");

const SPELL_VISUAL_COLUMNS: [Column; 16] = [
    c(0, "Id", Kind::Id),
    ca(1, "PrecastKit", KIT, "Played while the cast bar fills."),
    ca(2, "CastKit", KIT, "Played on the frame the cast completes."),
    ca(
        3,
        "ImpactKit",
        KIT,
        "Played on the target, not the caster, when the spell lands. Set on \
         8,751 rows. No server message marks the impact, so the client times \
         it from the missile's arrival.",
    ),
    ca(
        4,
        "StateKit",
        KIT,
        "Played for as long as the aura is on the unit. The other four kits \
         play at one point of a cast; this one lasts as long as the aura, so \
         UNIT_FIELD_AURA drives it rather than a cast.",
    ),
    ca(5, "ChannelKit", KIT, "Held for the duration of a channel."),
    ca(
        6,
        "HasMissile",
        Kind::Bool,
        "Set on 228 rows, always to 1. The four columns after it are read only \
         when it is set.",
    ),
    ca(7, "MissileModel", MODEL, "The model that flies from the caster to the target."),
    ca(
        8,
        "MissilePathType",
        Kind::Int,
        "Set on 17 rows, to 1 or 2: a straight line and an arc.",
    ),
    ca(
        9,
        "MissileDestination",
        Kind::Enum(ATTACHMENTS),
        "The attachment point on the target that the missile aims at. Set on \
         1,460 rows, with four distinct values. The names are the attachment \
         ids `world::m2::attach` measured on the character models.",
    ),
    ca(
        10,
        "MissileSound",
        Kind::Reference("SoundEntries"),
        "Measured: 140 rows, every value a SoundEntries id.",
    ),
    ca(
        11,
        "HasAreaEffect",
        Kind::Bool,
        "The two columns after it are read only when it is set, and the \
         client draws no DynamicObject visual unless it is set. Set on 217 \
         rows.",
    ),
    ca(
        12,
        "AreaModel",
        MODEL,
        "The model a persistent area effect is drawn as, such as Flamestrike's \
         fire or Consecration. All 217 set values are SpellVisualEffectName ids.",
    ),
    ca(
        13,
        "AreaKit",
        KIT,
        "The kit played with AreaModel. The falling-impact procedural, such \
         as Blizzard's shards, is set in this kit.",
    ),
    ca(
        14,
        "AreaSound",
        Kind::Reference("SoundEntries"),
        "Set on ten rows; all five distinct values are SoundEntries ids. The \
         role is inferred from the column's position and is unconfirmed, \
         because this client does not read the column.",
    ),
    ca(
        15,
        "Unknown 15",
        Kind::Int,
        "Set on 56 rows, to 1 or 2. Meaning unknown; this client does not read \
         it.",
    ),
];

/// `SpellVisualKit.dbc`: 1,778 rows of 35 fields. One row is one phase of a
/// spell's visual.
///
/// A row holds an animation, up to eight models each hanging from a named
/// attachment, a sound, a camera shake, and four procedural slots with four
/// parameters each.
pub(crate) const SPELL_VISUAL_KIT: Schema = Schema {
    table: "SpellVisualKit",
    columns: &SPELL_VISUAL_KIT_COLUMNS,
    sections: &[
        Section {
            name: "Animation, sound and camera shake",
            fields: &[0, 1, 2, 13, 14],
        },
        Section {
            name: "Models",
            fields: &[3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        },
        Section {
            name: "Procedurals",
            fields: &[15, 16, 17, 18],
        },
        Section {
            name: "Parameters",
            fields: &[
                19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34,
            ],
        },
    ],
};

/// The values the four procedural slots take, each as the 1.12.1 client acts
/// on it; see `tables::spell`.
const CHAR_PROCS: &[(u32, &str)] = &[
    (0, "Chain effect"),
    (1, "Model colour"),
    (9, "Falling impact (Blizzard)"),
    (12, "Chain effect (same as 0)"),
    (13, "Model glow"),
];

const SPELL_VISUAL_KIT_COLUMNS: [Column; 35] = [
    c(0, "Id", Kind::Id),
    ca(
        1,
        "StartAnim",
        Kind::Int,
        "Set on 975 rows, with four distinct values (1, 2, 3, 5). Later builds \
         name it startAnimID; the name is not confirmed by measurement. This \
         client does not read it.",
    ),
    ca(
        2,
        "Animation",
        Kind::Reference("AnimationData"),
        "The animation the caster plays. -1 is none. 0 is Stand, which is also \
         treated as none: playing Stand would interrupt the caster to stand \
         still.",
    ),
    ca(3, "HeadEffect", MODEL, "Drawn at the head attachment."),
    ca(4, "ChestEffect", MODEL, "Drawn at the chest attachment."),
    ca(5, "BaseEffect", MODEL, "Drawn at the unit's feet."),
    ca(6, "LeftHandEffect", MODEL, "Drawn at the left spell-hand attachment."),
    ca(7, "RightHandEffect", MODEL, "Drawn at the right spell-hand attachment."),
    ca(
        8,
        "BreathEffect",
        MODEL,
        "Set on 68 rows, each a SpellVisualEffectName id. Its attachment is \
         unknown, and this client does not draw it.",
    ),
    ca(
        9,
        "LeftWeaponEffect",
        MODEL,
        "Set on 17 rows; 16 are SpellVisualEffectName ids. Not drawn by this client.",
    ),
    ca(
        10,
        "RightWeaponEffect",
        MODEL,
        "Set on two rows. Not drawn by this client.",
    ),
    ca(11, "Unused 11", Kind::Unused, "Zero on all 1,778 rows."),
    ca(
        12,
        "GroundEffect",
        MODEL,
        "A model drawn at the base attachment. Set on 37 kits, all \
         SpellVisualEffectName ids: EntanglingRoots_State, Frost_Nova_state, \
         Net_State, Web_State, ThunderClap_Cast_Base and others. 29 of the 37 \
         set no other model column, so this is their only model.",
    ),
    ca(
        13,
        "Sound",
        Kind::Reference("SoundEntries"),
        "Set on 1,123 rows. 27 of them name an id that SoundEntries does not \
         have. The shipped file holds these broken references; they are not a \
         reading error.",
    ),
    ca(
        14,
        "CameraShake",
        Kind::Reference("SpellEffectCameraShakes"),
        "Identified from its values: set on 58 rows, with seven distinct values, \
         each an id in the nine-row SpellEffectCameraShakes table. The ids \
         include 26 and 66, so the match is not chance.",
    ),
    ca(
        15,
        "CharProc 1",
        Kind::Enum(CHAR_PROCS),
        "A procedural effect slot. Each slot takes its four parameters from the \
         CharParam columns with the same number.",
    ),
    c(16, "CharProc 2", Kind::Enum(CHAR_PROCS)),
    c(17, "CharProc 3", Kind::Enum(CHAR_PROCS)),
    c(18, "CharProc 4", Kind::Enum(CHAR_PROCS)),
    ca(
        19,
        "CharParamZero 1",
        Kind::Float,
        "For the falling-impact procedural: an index, stored as a float, into a \
         list of seven models built into the client and stored in no file.",
    ),
    c(20, "CharParamZero 2", Kind::Float),
    c(21, "CharParamZero 3", Kind::Float),
    c(22, "CharParamZero 4", Kind::Float),
    ca(
        23,
        "CharParamOne 1",
        Kind::Float,
        "For the falling-impact procedural: impacts per second. Blizzard's kit \
         holds 5.",
    ),
    c(24, "CharParamOne 2", Kind::Float),
    c(25, "CharParamOne 3", Kind::Float),
    c(26, "CharParamOne 4", Kind::Float),
    c(27, "CharParamTwo 1", Kind::Float),
    c(28, "CharParamTwo 2", Kind::Float),
    c(29, "CharParamTwo 3", Kind::Float),
    c(30, "CharParamTwo 4", Kind::Float),
    c(31, "CharParamThree 1", Kind::Float),
    c(32, "CharParamThree 2", Kind::Float),
    c(33, "CharParamThree 3", Kind::Float),
    c(34, "CharParamThree 4", Kind::Float),
];

/// `SpellVisualEffectName.dbc`: 782 rows of 5 fields. Each row is one effect
/// model.
pub(crate) const SPELL_VISUAL_EFFECT_NAME: Schema = Schema {
    table: "SpellVisualEffectName",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(
            1,
            "Name",
            Kind::Text,
            "The row's name. The client finds the few visuals that no \
             SpellVisualKit references by this name.",
        ),
        ca(
            2,
            "Model",
            Kind::Text,
            "An .mdx path under Spells\\ or Particles\\. The model holds only \
             particle emitters. The archives hold the .m2 of the same name.",
        ),
        ca(3, "Unused 3", Kind::Unused, "Zero on every row."),
        ca(
            4,
            "Scale",
            Kind::Float,
            "The effect model's scale. It applies to this model only and is not \
             the caster's scale.",
        ),
    ],
    sections: &[Section {
        name: "Effect",
        fields: &[0, 1, 2, 4, 3],
    }],
};

/// `AnimationData.dbc`: what a pose is called.
pub(crate) const ANIMATION_DATA: Schema = Schema {
    table: "AnimationData",
    columns: &[
        c(0, "Id", Kind::Id),
        c(1, "Name", Kind::Text),
        c(2, "WeaponFlags", Kind::Flags(&[])),
        c(3, "BodyFlags", Kind::Flags(&[])),
        c(4, "Flags", Kind::Flags(&[])),
        ca(
            5,
            "Fallback",
            Kind::Reference("AnimationData"),
            "The animation played instead when a model does not have this one.",
        ),
        c(6, "BehaviourId", Kind::Int),
    ],
    sections: &[Section {
        name: "Animation",
        fields: &[0, 1, 5, 2, 3, 4, 6],
    }],
};

/// `SpellIcon.dbc`: an id and a texture path.
pub(crate) const SPELL_ICON: Schema = Schema {
    table: "SpellIcon",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(
            1,
            "Texture",
            Kind::Text,
            "An Interface\\Icons\\ path with no extension: the client appends \
             .blp.",
        ),
    ],
    sections: &[Section {
        name: "Icon",
        fields: &[0, 1],
    }],
};

/// `SpellCastTimes.dbc`: the length of a cast, in milliseconds.
pub(crate) const SPELL_CAST_TIMES: Schema = Schema {
    table: "SpellCastTimes",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(1, "Base", Kind::Signed, "Milliseconds. Row 16, which Fireball uses, is 1500."),
        c(2, "PerLevel", Kind::Signed),
        c(3, "Minimum", Kind::Signed),
    ],
    sections: &[Section {
        name: "Cast time",
        fields: &[0, 1, 2, 3],
    }],
};

/// `SpellDuration.dbc`: how long an aura lasts, in milliseconds.
pub(crate) const SPELL_DURATION: Schema = Schema {
    table: "SpellDuration",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(1, "Duration", Kind::Signed, "Milliseconds. -1 is infinite."),
        c(2, "PerLevel", Kind::Signed),
        c(3, "Maximum", Kind::Signed),
    ],
    sections: &[Section {
        name: "Duration",
        fields: &[0, 1, 2, 3],
    }],
};

/// `SpellRadius.dbc`: how wide an effect reaches, in yards.
pub(crate) const SPELL_RADIUS: Schema = Schema {
    table: "SpellRadius",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(1, "Radius", Kind::Float, "Yards."),
        c(2, "PerLevel", Kind::Float),
        c(3, "Maximum", Kind::Float),
    ],
    sections: &[Section {
        name: "Radius",
        fields: &[0, 1, 2, 3],
    }],
};

/// `SpellChainEffects.dbc`: the bolt strung between two units.
///
/// Every column is read in order by the client; see `tables::spell`, where the
/// same seven are parsed.
pub(crate) const SPELL_CHAIN_EFFECTS: Schema = Schema {
    table: "SpellChainEffects",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(
            1,
            "AverageSegmentLength",
            Kind::Float,
            "The length of one bolt segment, in yards.",
        ),
        c(2, "Width", Kind::Float),
        ca(
            3,
            "NoiseScale",
            Kind::Float,
            "How far each segment deviates from the straight line.",
        ),
        c(4, "TexCoordScale", Kind::Float),
        c(5, "SegmentDuration", Kind::Int),
        c(6, "SegmentDelay", Kind::Int),
        ca(7, "Texture", Kind::Text, "The bolt's texture."),
    ],
    sections: &[Section {
        name: "Chain",
        fields: &[0, 7, 1, 2, 3, 4, 5, 6],
    }],
};

/// `SpellEffectCameraShakes.dbc`: nine rows. A `SpellVisualKit` row names one
/// of them.
pub(crate) const SPELL_EFFECT_CAMERA_SHAKES: Schema = Schema {
    table: "SpellEffectCameraShakes",
    columns: &[
        c(0, "Id", Kind::Id),
        c(1, "CameraShake 1", Kind::Int),
        c(2, "CameraShake 2", Kind::Int),
        c(3, "CameraShake 3", Kind::Int),
    ],
    sections: &[Section {
        name: "Shake",
        fields: &[0, 1, 2, 3],
    }],
};

/// `SoundEntries.dbc`: 4,623 rows of 29 fields. A sound id is a row of this
/// table.
///
/// The indices are those of [`super::sound::fields::entry`], which the sound
/// bank reads through; a test asserts each one by name. A row is up to ten
/// files under one directory, one picked by weight; see
/// [`super::sound::SoundEntry`].
pub(crate) const SOUND_ENTRIES: Schema = Schema {
    table: "SoundEntries",
    columns: &SOUND_ENTRIES_COLUMNS,
    sections: &[
        Section {
            name: "Sound",
            fields: &[0, 2, 1, 23, 24, 25, 26, 27, 28],
        },
        Section {
            name: "Files",
            fields: &[3, 13, 4, 14, 5, 15, 6, 16, 7, 17, 8, 18, 9, 19, 10, 20, 11, 21, 12, 22],
        },
    ],
};

/// `SoundEntries`' `SoundType` values. Each name describes the shipped rows of
/// that value, from the folder most of them are in and from their names: 1 is
/// 302 rows in `Sound\Spells`, and 28 is all 110 rows in `Sound\Music`. What a
/// value changes in the client is not established, so a name says what the
/// rows hold, not what the client does with them. The list holds every value
/// a shipped row has; a value not in it is drawn as its number.
pub(crate) const SOUND_TYPES: [(u32, &str); 26] = [
    (0, "None"),
    (1, "Spells"),
    (2, "Interface"),
    (3, "Character on terrain"),
    (4, "Weapon impacts"),
    (6, "Weapon swings"),
    (9, "Item handling"),
    (10, "Creature and character combat"),
    (12, "Character errors"),
    (14, "Doodad loops"),
    (16, "Death thuds"),
    (17, "NPC greetings"),
    (18, "Tests"),
    (19, "Armour foley"),
    (20, "Footstep splashes"),
    (21, "Character splashes"),
    (22, "Water"),
    (23, "Gathering"),
    (25, "Objects"),
    (26, "Spell fizzles"),
    (27, "Creature loops"),
    (28, "Music"),
    (29, "Emote voices"),
    (30, "Fly-by music"),
    (31, "Cinematic voices"),
    (50, "Ambience"),
];

const SOUND_ENTRIES_COLUMNS: [Column; 29] = [
    c(0, "Id", Kind::Id),
    ca(
        1,
        "SoundType",
        Kind::Enum(&SOUND_TYPES),
        "The sound's type. Each value is named after the folder and the names of its \
         shipped rows; what a value changes in the client is not established. `vale sound` \
         counts sounds by it.",
    ),
    c(2, "Name", Kind::Text),
    c(3, "File 1", Kind::Text),
    c(4, "File 2", Kind::Text),
    c(5, "File 3", Kind::Text),
    c(6, "File 4", Kind::Text),
    c(7, "File 5", Kind::Text),
    c(8, "File 6", Kind::Text),
    c(9, "File 7", Kind::Text),
    c(10, "File 8", Kind::Text),
    c(11, "File 9", Kind::Text),
    c(12, "File 10", Kind::Text),
    ca(
        13,
        "Weight 1",
        Kind::Int,
        "The relative chance that the file with the same number is picked.",
    ),
    c(14, "Weight 2", Kind::Int),
    c(15, "Weight 3", Kind::Int),
    c(16, "Weight 4", Kind::Int),
    c(17, "Weight 5", Kind::Int),
    c(18, "Weight 6", Kind::Int),
    c(19, "Weight 7", Kind::Int),
    c(20, "Weight 8", Kind::Int),
    c(21, "Weight 9", Kind::Int),
    c(22, "Weight 10", Kind::Int),
    ca(
        23,
        "Directory",
        Kind::Text,
        "The folder that holds every file above. Three rows end it with a \
         path separator, which the reader removes.",
    ),
    c(24, "Volume", Kind::Float),
    c(25, "Flags", Kind::Flags(&[])),
    ca(26, "MinDistance", Kind::Float, "Within this distance the sound plays at full volume."),
    ca(
        27,
        "DistanceCutoff",
        Kind::Float,
        "Beyond this range the sound is not started at all.",
    ),
    ca(
        28,
        "EAXDef",
        Kind::Int,
        "Name taken from WDBX. Not read by this client.",
    ),
];

/// `SpellRange.dbc`: 28 rows of 22 fields. Field 2 decides whether a spell has
/// a target at all: zero is a self-cast.
pub const SPELL_RANGE: Schema = Schema {
    table: "SpellRange",
    columns: &SPELL_RANGE_COLUMNS,
    sections: &[Section {
        name: "Range",
        fields: &[
            0, 4, 13, 1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 14, 15, 16, 17, 18, 19, 20, 21,
        ],
    }],
};

const SPELL_RANGE_COLUMNS: [Column; 22] = [
    c(0, "Id", Kind::Id),
    ca(1, "RangeMin", Kind::Float, "Yards."),
    ca(
        2,
        "RangeMax",
        Kind::Float,
        "Yards. Row 1, Combat Range, holds 5.0, which identifies the column. \
         Zero is a spell cast on its own caster.",
    ),
    c(3, "Flags", Kind::Flags(&[])),
    c(4, "DisplayName", Kind::Text),
    c(5, "DisplayName koKR", Kind::Locale("koKR")),
    c(6, "DisplayName frFR", Kind::Locale("frFR")),
    c(7, "DisplayName deDE", Kind::Locale("deDE")),
    c(8, "DisplayName enCN", Kind::Locale("enCN")),
    c(9, "DisplayName enTW", Kind::Locale("enTW")),
    c(10, "DisplayName esES", Kind::Locale("esES")),
    c(11, "DisplayName esMX", Kind::Locale("esMX")),
    c(12, "DisplayNameFlags", Kind::LocaleFlags),
    c(13, "ShortName", Kind::Text),
    c(14, "ShortName koKR", Kind::Locale("koKR")),
    c(15, "ShortName frFR", Kind::Locale("frFR")),
    c(16, "ShortName deDE", Kind::Locale("deDE")),
    c(17, "ShortName enCN", Kind::Locale("enCN")),
    c(18, "ShortName enTW", Kind::Locale("enTW")),
    c(19, "ShortName esES", Kind::Locale("esES")),
    c(20, "ShortName esMX", Kind::Locale("esMX")),
    c(21, "ShortNameFlags", Kind::LocaleFlags),
];

/// `SpellDispelType.dbc`: 11 rows of 12 fields: Magic, Curse, Disease, Poison
/// and the rest.
pub(crate) const SPELL_DISPEL_TYPE: Schema = Schema {
    table: "SpellDispelType",
    columns: &SPELL_DISPEL_TYPE_COLUMNS,
    sections: &[Section {
        name: "Dispel type",
        fields: &[0, 1, 10, 11, 2, 3, 4, 5, 6, 7, 8, 9],
    }],
};

const SPELL_DISPEL_TYPE_COLUMNS: [Column; 12] = [
    c(0, "Id", Kind::Id),
    c(1, "Name", Kind::Text),
    c(2, "Name koKR", Kind::Locale("koKR")),
    c(3, "Name frFR", Kind::Locale("frFR")),
    c(4, "Name deDE", Kind::Locale("deDE")),
    c(5, "Name enCN", Kind::Locale("enCN")),
    c(6, "Name enTW", Kind::Locale("enTW")),
    c(7, "Name esES", Kind::Locale("esES")),
    c(8, "Name esMX", Kind::Locale("esMX")),
    c(9, "NameFlags", Kind::LocaleFlags),
    ca(
        10,
        "Unknown 10",
        Kind::Int,
        "WDBX names this Mask. Not measured.",
    ),
    ca(
        11,
        "Unknown 11",
        Kind::Int,
        "WDBX names this ImmunityPossible. Not measured.",
    ),
];

/// `SpellMechanic.dbc`: 27 rows of 10 fields: an id and a localised word
/// (disoriented, disarmed, rooted).
pub(crate) const SPELL_MECHANIC: Schema = Schema {
    table: "SpellMechanic",
    columns: &SPELL_MECHANIC_COLUMNS,
    sections: &[Section {
        name: "Mechanic",
        fields: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    }],
};

const SPELL_MECHANIC_COLUMNS: [Column; 10] = [
    c(0, "Id", Kind::Id),
    c(1, "Name", Kind::Text),
    c(2, "Name koKR", Kind::Locale("koKR")),
    c(3, "Name frFR", Kind::Locale("frFR")),
    c(4, "Name deDE", Kind::Locale("deDE")),
    c(5, "Name enCN", Kind::Locale("enCN")),
    c(6, "Name enTW", Kind::Locale("enTW")),
    c(7, "Name esES", Kind::Locale("esES")),
    c(8, "Name esMX", Kind::Locale("esMX")),
    c(9, "NameFlags", Kind::LocaleFlags),
];

/// `SpellFocusObject.dbc`: 138 rows of 10 fields (Anvil, Loom, Forge), in the
/// same shape as [`SPELL_MECHANIC`].
pub(crate) const SPELL_FOCUS_OBJECT: Schema = Schema {
    table: "SpellFocusObject",
    columns: &SPELL_FOCUS_OBJECT_COLUMNS,
    sections: &[Section {
        name: "Focus object",
        fields: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    }],
};

const SPELL_FOCUS_OBJECT_COLUMNS: [Column; 10] = [
    c(0, "Id", Kind::Id),
    c(1, "Name", Kind::Text),
    c(2, "Name koKR", Kind::Locale("koKR")),
    c(3, "Name frFR", Kind::Locale("frFR")),
    c(4, "Name deDE", Kind::Locale("deDE")),
    c(5, "Name enCN", Kind::Locale("enCN")),
    c(6, "Name enTW", Kind::Locale("enTW")),
    c(7, "Name esES", Kind::Locale("esES")),
    c(8, "Name esMX", Kind::Locale("esMX")),
    c(9, "NameFlags", Kind::LocaleFlags),
];

/// `SpellCategory.dbc`: 166 rows of 2 fields: an id and a flags word. A
/// category has no name; it is the shared cooldown group a spell is in.
pub(crate) const SPELL_CATEGORY: Schema = Schema {
    table: "SpellCategory",
    columns: &[c(0, "Id", Kind::Id), c(1, "Flags", Kind::Flags(&[]))],
    sections: &[Section {
        name: "Category",
        fields: &[0, 1],
    }],
};

/// The attachment points a `SpellVisual` column may name, by the ids
/// `world::m2::attach` measured. The list is not complete; a value it does not
/// name is shown as its number.
const ATTACHMENTS: &[(u32, &str)] = &[
    (crate::world::m2::attach::SADDLE, "Saddle / shield"),
    (crate::world::m2::attach::HAND_RIGHT, "Right hand"),
    (crate::world::m2::attach::HAND_LEFT, "Left hand"),
    (crate::world::m2::attach::SHOULDER_RIGHT, "Right shoulder"),
    (crate::world::m2::attach::SHOULDER_LEFT, "Left shoulder"),
    (crate::world::m2::attach::HELM, "Helm"),
    (crate::world::m2::attach::BASE, "Base (feet)"),
    (crate::world::m2::attach::HEAD, "Head"),
    (crate::world::m2::attach::SPELL_HAND_LEFT, "Spell left hand"),
    (crate::world::m2::attach::SPELL_HAND_RIGHT, "Spell right hand"),
    (crate::world::m2::attach::CHEST, "Chest"),
];

const SCHOOLS: &[(u32, &str)] = &[
    (0, "Physical"),
    (1, "Holy"),
    (2, "Fire"),
    (3, "Nature"),
    (4, "Frost"),
    (5, "Shadow"),
    (6, "Arcane"),
];

const POWERS: &[(u32, &str)] = &[
    (0, "Mana"),
    (1, "Rage"),
    (2, "Focus"),
    (3, "Energy"),
    (4, "Happiness"),
];

const DAMAGE_CLASSES: &[(u32, &str)] = &[
    (0, "None"),
    (1, "Magic"),
    (2, "Melee"),
    (3, "Ranged"),
];

const PREVENTION: &[(u32, &str)] = &[
    (0, "None"),
    (1, "Silence"),
    (2, "Pacify"),
];

const SPELL_EFFECTS_TABLE: &[(u32, &str)] = &SPELL_EFFECTS;
const SPELL_AURAS_TABLE: &[(u32, &str)] = &SPELL_AURAS;

// ---------------------------------------------------------------------------
// The light chain
// ---------------------------------------------------------------------------

/// `Light.dbc`: 374 rows of 12 fields. A row is a sphere on a map and the five
/// `LightParams` rows it uses under the five conditions.
///
/// A row with `FalloffEnd` of zero is the map's default light, and its
/// position is not read. Every other row applies within its sphere, blended by
/// [`super::light::PositionalLight::weight`]. These indices are the ones in
/// [`super::light`]'s `light_field` module.
///
/// The three coordinates are not world coordinates. They are in the internal
/// representation every `MDDF` and `MODF` placement uses (x westward and z
/// northward from the corner of the 64x64 grid, y the height), scaled by 36
/// like every other distance in these tables. 1,180,800 is therefore the far
/// corner, not an overflowed value. `vale light <Map>` prints the world
/// position of each row.
pub(crate) const LIGHT: Schema = Schema {
    table: "Light",
    columns: &[
        c(0, "Id", Kind::Id),
        c(1, "Map", Kind::Reference("Map")),
        ca(
            2,
            "InternalX",
            Kind::Float,
            "Westward from the map's corner, in 1/36 of a yard. Yields the \
             world's y.",
        ),
        ca(
            3,
            "InternalY",
            Kind::Float,
            "The height, in 1/36 of a yard. Unlike InternalX and InternalZ it \
             is not measured from the map's corner.",
        ),
        ca(
            4,
            "InternalZ",
            Kind::Float,
            "Northward from the map's corner, in 1/36 of a yard. Yields the \
             world's x.",
        ),
        ca(
            5,
            "FalloffStart",
            Kind::Float,
            "1/36 of a yard. Inside this radius the row applies at full \
             strength.",
        ),
        ca(
            6,
            "FalloffEnd",
            Kind::Float,
            "1/36 of a yard, falling linearly from FalloffStart to nothing \
             here. Zero marks the map's default light, which applies \
             everywhere and whose position is not read.",
        ),
        ca(
            7,
            "ParamsClear",
            Kind::Reference("LightParams"),
            "Clear weather, above the water. The default set.",
        ),
        ca(
            8,
            "ParamsClearUnderwater",
            Kind::Reference("LightParams"),
            "The same light seen from under the water. A row of 0 falls back \
             to ParamsClear.",
        ),
        ca(
            9,
            "ParamsStorm",
            Kind::Reference("LightParams"),
            "The set the sky blends toward when SMSG_WEATHER starts weather. A \
             row of 0 is a zone whose sky does not change when it rains.",
        ),
        ca(
            10,
            "ParamsStormUnderwater",
            Kind::Reference("LightParams"),
            "Storm weather, seen from under the water.",
        ),
        ca(
            11,
            "ParamsDeath",
            Kind::Reference("LightParams"),
            "Used while the character is a ghost, which this client does not \
             support. Every light uses one of four rows: 2, 3, 4 or 5.",
        ),
    ],
    sections: &[
        Section {
            name: "Light",
            fields: &[0, 1],
        },
        Section {
            name: "Sphere",
            fields: &[2, 3, 4, 5, 6],
        },
        Section {
            name: "Conditions",
            fields: &[7, 8, 9, 10, 11],
        },
    ],
};

/// `LightParams.dbc`: 426 rows of 9 fields. A row is one set of lighting: the
/// sky flags, the glow and the four water opacities.
///
/// The colours are not in this table. A params row owns 18 `LightIntBand`
/// rows and 6 `LightFloatBand` rows, found by arithmetic on the row position
/// rather than by a reference column; see [`super::light::int_band_row`].
///
/// Of its nine columns this client reads only the id and the four water
/// alphas. `LightTables::parse` takes fields 5..8 and no others, so an edit to
/// `HighlightSky`, `Skybox`, `CloudType` or `Glow` changes nothing on screen.
/// That is a property of this renderer, not of the file, and each of those
/// columns' notes states it.
pub(crate) const LIGHT_PARAMS: Schema = Schema {
    table: "LightParams",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(1, "HighlightSky", Kind::Bool, "0 or 1 over all 426 rows."),
        ca(
            2,
            "Skybox",
            Kind::Reference("LightSkybox"),
            "Five rows set a skybox, and all five name row 3, DeathClouds.mdx. \
             Four of them are the rows Light's ParamsDeath column uses; no \
             Light row uses the fifth. The other 421 rows hold 0.",
        ),
        ca(
            3,
            "CloudType",
            Kind::Unused,
            "Zero on all 426 rows. The client draws no cloud sheet — see the \
             note on bands 10..12 in `tables::light`.",
        ),
        ca(4, "Glow", Kind::Float, "0.0..1.0 over 13 distinct values."),
        ca(
            5,
            "WaterShallowAlpha",
            Kind::Float,
            "How opaque a river is near the camera. 0.0..1.0. One of the four \
             columns of this table the renderer reads; it affects water only.",
        ),
        c(6, "WaterDeepAlpha", Kind::Float),
        c(7, "OceanShallowAlpha", Kind::Float),
        c(8, "OceanDeepAlpha", Kind::Float),
    ],
    sections: &[
        Section {
            name: "Sky",
            fields: &[0, 1, 2, 3, 4],
        },
        Section {
            name: "Water opacity",
            fields: &[5, 6, 7, 8],
        },
    ],
};

/// `LightIntBand.dbc`: 7,668 rows of 34 fields. A row is one colour over the
/// day, as up to sixteen keys.
///
/// `EntryCount` says how many of the sixteen slots are live. The rest hold
/// whatever was in memory when the file was written, which is `0xCCCCCCCC` in
/// 101,116 of the unused time slots. A writer must leave that tail unchanged,
/// or it rewrites rows nobody edited.
///
/// The times are half-minutes past midnight, 0..2,879. All 17,535 live entries
/// are inside that range, which pins fields 2..17 as the times rather than the
/// values. The times ascend within a row in 7,641 of 7,668 rows; the 27 that do
/// not are rows whose last live time is 0, so a reader must not assume ascent.
///
/// Which band a row holds is given by its position, not by a column: band `b`
/// of params `p` is row `(p - 1) * 18 + b + 1`. See
/// [`super::light::int_band_row`] and [`super::light::INT_BAND_NAMES`].
pub(crate) const LIGHT_INT_BAND: Schema = Schema {
    table: "LightIntBand",
    columns: &LIGHT_INT_BAND_COLUMNS,
    sections: &[
        Section {
            name: "Band",
            fields: &[0, 1],
        },
        Section {
            name: "Keys",
            fields: &BAND_KEY_FIELDS,
        },
    ],
};

/// The thirty-two key columns, each time next to the value it keys.
///
/// A form drawn in field order would put sixteen times above sixteen values,
/// and the reader could not see which time pairs with which value.
const BAND_KEY_FIELDS: [usize; 32] = [
    2, 18, 3, 19, 4, 20, 5, 21, 6, 22, 7, 23, 8, 24, 9, 25, 10, 26, 11, 27, 12, 28, 13, 29, 14, 30,
    15, 31, 16, 32, 17, 33,
];

const LIGHT_INT_BAND_COLUMNS: [Column; 34] = [
    c(0, "Id", Kind::Id),
    ca(
        1,
        "EntryCount",
        Kind::Int,
        "How many of the sixteen keys are live. 0..11 over the shipped rows.",
    ),
    ca(
        2,
        "Time 1",
        Kind::Int,
        "Half-minutes past midnight, 0..2879. 1440 is noon.",
    ),
    c(3, "Time 2", Kind::Int),
    c(4, "Time 3", Kind::Int),
    c(5, "Time 4", Kind::Int),
    c(6, "Time 5", Kind::Int),
    c(7, "Time 6", Kind::Int),
    c(8, "Time 7", Kind::Int),
    c(9, "Time 8", Kind::Int),
    c(10, "Time 9", Kind::Int),
    c(11, "Time 10", Kind::Int),
    c(12, "Time 11", Kind::Int),
    c(13, "Time 12", Kind::Int),
    c(14, "Time 13", Kind::Int),
    c(15, "Time 14", Kind::Int),
    c(16, "Time 15", Kind::Int),
    c(17, "Time 16", Kind::Int),
    ca(
        18,
        "Colour 1",
        Kind::Colour,
        "Packed as 0x00RRGGBB, red first. Measured: read in the reverse order, \
         map 0's noon sun is pale blue.",
    ),
    c(19, "Colour 2", Kind::Colour),
    c(20, "Colour 3", Kind::Colour),
    c(21, "Colour 4", Kind::Colour),
    c(22, "Colour 5", Kind::Colour),
    c(23, "Colour 6", Kind::Colour),
    c(24, "Colour 7", Kind::Colour),
    c(25, "Colour 8", Kind::Colour),
    c(26, "Colour 9", Kind::Colour),
    c(27, "Colour 10", Kind::Colour),
    c(28, "Colour 11", Kind::Colour),
    c(29, "Colour 12", Kind::Colour),
    c(30, "Colour 13", Kind::Colour),
    c(31, "Colour 14", Kind::Colour),
    c(32, "Colour 15", Kind::Colour),
    c(33, "Colour 16", Kind::Colour),
];

/// `LightFloatBand.dbc`: 2,556 rows of 34 fields, in the same shape as
/// [`LIGHT_INT_BAND`] with an `f32` in place of the colour.
///
/// There are six rows per `LightParams` rather than eighteen. That count is
/// checked against the record counts, not assumed, because reading one light's
/// fog as another's produces a plausible wrong result rather than a failure.
/// Only bands 0 and 1 are read: the fog's end, and where the fog starts as a
/// fraction of that. Bands 2..5 hold the same three constants across all
/// nineteen default lights.
///
/// The distances are in 1/36 of a yard, like `Light`'s own coordinates: map 0's
/// `fogEnd` of 18,000 is 500 yards.
pub(crate) const LIGHT_FLOAT_BAND: Schema = Schema {
    table: "LightFloatBand",
    columns: &LIGHT_FLOAT_BAND_COLUMNS,
    sections: &[
        Section {
            name: "Band",
            fields: &[0, 1],
        },
        Section {
            name: "Keys",
            fields: &BAND_KEY_FIELDS,
        },
    ],
};

const LIGHT_FLOAT_BAND_COLUMNS: [Column; 34] = [
    c(0, "Id", Kind::Id),
    ca(
        1,
        "EntryCount",
        Kind::Int,
        "How many of the sixteen keys are live. 0..10 over the shipped rows.",
    ),
    ca(
        2,
        "Time 1",
        Kind::Int,
        "Half-minutes past midnight, 0..2879. 1440 is noon.",
    ),
    c(3, "Time 2", Kind::Int),
    c(4, "Time 3", Kind::Int),
    c(5, "Time 4", Kind::Int),
    c(6, "Time 5", Kind::Int),
    c(7, "Time 6", Kind::Int),
    c(8, "Time 7", Kind::Int),
    c(9, "Time 8", Kind::Int),
    c(10, "Time 9", Kind::Int),
    c(11, "Time 10", Kind::Int),
    c(12, "Time 11", Kind::Int),
    c(13, "Time 12", Kind::Int),
    c(14, "Time 13", Kind::Int),
    c(15, "Time 14", Kind::Int),
    c(16, "Time 15", Kind::Int),
    c(17, "Time 16", Kind::Int),
    ca(
        18,
        "Value 1",
        Kind::Float,
        "Band 0 is the fog's end and band 1 is where it starts as a fraction of \
         that. In 1/36 of a yard, so 18,000 is 500 yards.",
    ),
    c(19, "Value 2", Kind::Float),
    c(20, "Value 3", Kind::Float),
    c(21, "Value 4", Kind::Float),
    c(22, "Value 5", Kind::Float),
    c(23, "Value 6", Kind::Float),
    c(24, "Value 7", Kind::Float),
    c(25, "Value 8", Kind::Float),
    c(26, "Value 9", Kind::Float),
    c(27, "Value 10", Kind::Float),
    c(28, "Value 11", Kind::Float),
    c(29, "Value 12", Kind::Float),
    c(30, "Value 13", Kind::Float),
    c(31, "Value 14", Kind::Float),
    c(32, "Value 15", Kind::Float),
    c(33, "Value 16", Kind::Float),
];

/// `LightSkybox.dbc`: 6 rows of 2 fields: an id and a model path.
///
/// `LightParams` names only one of the six; see that schema's `Skybox` column.
pub(crate) const LIGHT_SKYBOX: Schema = Schema {
    table: "LightSkybox",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(
            1,
            "Model",
            Kind::Text,
            "An .mdx under Environments\\Stars\\. All six shipped rows name a \
             file that is in the archives.",
        ),
    ],
    sections: &[Section {
        name: "Skybox",
        fields: &[0, 1],
    }],
};

/// `TaxiNodes.dbc`: 85 rows of 16 fields. A row is a flight master's node:
/// where it stands, what it is called and which mounts fly from it. The
/// indices are those of [`super::taxi::node_fields`].
///
/// vmangos does not read this file. It reads the same rows from its own
/// `taxi_nodes` table, so an edit here reaches the server as a row of that
/// table and not as a copied file.
pub(crate) const TAXI_NODES: Schema = Schema {
    table: "TaxiNodes",
    columns: &[
        c(0, "Id", Kind::Id),
        c(1, "Map", Kind::Reference("Map")),
        ca(2, "X", Kind::Float, "North, in yards, as .gps prints it."),
        ca(3, "Y", Kind::Float, "West, in yards."),
        ca(4, "Z", Kind::Float, "Up, in yards."),
        c(5, "Name", Kind::Text),
        c(6, "Name koKR", Kind::Locale(LOCALES[1])),
        c(7, "Name frFR", Kind::Locale(LOCALES[2])),
        c(8, "Name deDE", Kind::Locale(LOCALES[3])),
        c(9, "Name enCN", Kind::Locale(LOCALES[4])),
        c(10, "Name enTW", Kind::Locale(LOCALES[5])),
        c(11, "Name esES", Kind::Locale(LOCALES[6])),
        c(12, "Name esMX", Kind::Locale(LOCALES[7])),
        c(13, "NameFlags", Kind::LocaleFlags),
        ca(
            14,
            "MountCreatureId 1",
            Kind::Int,
            "A creature_template entry. vmangos reads this column as the Horde \
             mount, and a node with 0 here is not offered to the Horde. The \
             client decides the side by the value: 2224 and 3574 are Horde, \
             541 and 3837 Alliance.",
        ),
        ca(
            15,
            "MountCreatureId 2",
            Kind::Int,
            "A creature_template entry. vmangos reads this column as the \
             Alliance mount, and a node with 0 here is not offered to the \
             Alliance.",
        ),
    ],
    sections: &[
        Section {
            name: "Node",
            fields: &[0, 1, 2, 3, 4, 14, 15],
        },
        Section {
            name: "Name",
            fields: &[5, 6, 7, 8, 9, 10, 11, 12, 13],
        },
    ],
};

/// `TaxiPath.dbc`: 287 rows of 4 fields. A row is one flight between two
/// nodes, in one direction. The indices are those of
/// [`super::taxi::path_fields`].
pub(crate) const TAXI_PATH: Schema = Schema {
    table: "TaxiPath",
    columns: &[
        c(0, "Id", Kind::Id),
        c(1, "From", Kind::Reference("TaxiNodes")),
        c(2, "To", Kind::Reference("TaxiNodes")),
        ca(
            3,
            "Cost",
            Kind::Int,
            "Copper, before the reputation discount the server applies.",
        ),
    ],
    sections: &[Section {
        name: "Path",
        fields: &[0, 1, 2, 3],
    }],
};

/// `TaxiPathNode.dbc`: 9,582 rows of 9 fields. A row is one point of one path.
/// The indices are those of [`super::taxi::path_node_fields`].
pub(crate) const TAXI_PATH_NODE: Schema = Schema {
    table: "TaxiPathNode",
    columns: &[
        c(0, "Id", Kind::Id),
        c(1, "Path", Kind::Reference("TaxiPath")),
        ca(
            2,
            "NodeIndex",
            Kind::Int,
            "The point's place along its path, from 0. vmangos sizes the path \
             to the largest index plus one, so the indices of one path must run \
             0, 1, 2 with no gap.",
        ),
        c(3, "Map", Kind::Reference("Map")),
        ca(4, "X", Kind::Float, "North, in yards."),
        ca(5, "Y", Kind::Float, "West, in yards."),
        ca(6, "Z", Kind::Float, "Up, in yards."),
        ca(
            7,
            "ActionFlag",
            Kind::Int,
            "1 is a teleport to the next point. 2 is a stop, which the boats \
             and zeppelins use; a flight uses 0.",
        ),
        ca(
            8,
            "Delay",
            Kind::Int,
            "Seconds waited at a stop. Read only when ActionFlag is 2.",
        ),
    ],
    sections: &[Section {
        name: "Point",
        fields: &[0, 1, 2, 3, 4, 5, 6, 7, 8],
    }],
};

// ---------------------------------------------------------------------------
// The skill tables and the item sets
// ---------------------------------------------------------------------------

/// The races a mask names: bit `id - 1` for each `ChrRaces` id. The names
/// follow vmangos' `SharedDefines.h` (`Races`).
const RACE_BITS: &[(u32, &str, &str)] = &[
    (0x001, "Human", ""),
    (0x002, "Orc", ""),
    (0x004, "Dwarf", ""),
    (0x008, "Night Elf", ""),
    (0x010, "Undead", ""),
    (0x020, "Tauren", ""),
    (0x040, "Gnome", ""),
    (0x080, "Troll", ""),
    (0x100, "Goblin", "Race 9. Not a playable race."),
];

/// The classes a mask names: bit `id - 1` for each `ChrClasses` id. Ids 6 and
/// 10 are no class in 1.12, so bits 0x020 and 0x200 have no name. The names
/// follow vmangos' `SharedDefines.h` (`Classes`).
const CLASS_BITS: &[(u32, &str, &str)] = &[
    (0x001, "Warrior", ""),
    (0x002, "Paladin", ""),
    (0x004, "Hunter", ""),
    (0x008, "Rogue", ""),
    (0x010, "Priest", ""),
    (0x040, "Shaman", ""),
    (0x080, "Mage", ""),
    (0x100, "Warlock", ""),
    (0x400, "Druid", ""),
];

/// `SkillLine.dbc`: 123 rows of 22 fields. A row is one skill: a class
/// specialisation, a weapon skill, a profession, a language. The indices are
/// those [`super::skills`] reads, and the names follow vmangos'
/// `SkillLineEntry`.
///
/// vmangos reads this file from `DataDir`, so an edit reaches the server as
/// the copied file.
pub(crate) const SKILL_LINE: Schema = Schema {
    table: "SkillLine",
    columns: &SKILL_LINE_COLUMNS,
    sections: &[
        Section {
            name: "Skill line",
            fields: &[0, 1, 2, 21],
        },
        Section {
            name: "Name",
            fields: &[3, 4, 5, 6, 7, 8, 9, 10, 11],
        },
        Section {
            name: "Description",
            fields: &[12, 13, 14, 15, 16, 17, 18, 19, 20],
        },
    ],
};

const SKILL_LINE_COLUMNS: [Column; 22] = [
    c(0, "Id", Kind::Id),
    ca(
        1,
        "Category",
        Kind::Reference("SkillLineCategory"),
        "The heading the skills panel lists the line under: 6 weapon skills, 7 \
         class skills, 8 armour proficiencies, 9 secondary skills, 10 \
         languages, 11 professions, 12 not displayed. One shipped row holds -1.",
    ),
    ca(
        2,
        "SkillCosts",
        Kind::Reference("SkillCostsData"),
        "Zero on all 123 shipped rows.",
    ),
    c(3, "Name", Kind::Text),
    c(4, "Name koKR", Kind::Locale(LOCALES[1])),
    c(5, "Name frFR", Kind::Locale(LOCALES[2])),
    c(6, "Name deDE", Kind::Locale(LOCALES[3])),
    c(7, "Name enCN", Kind::Locale(LOCALES[4])),
    c(8, "Name enTW", Kind::Locale(LOCALES[5])),
    c(9, "Name esES", Kind::Locale(LOCALES[6])),
    c(10, "Name esMX", Kind::Locale(LOCALES[7])),
    c(11, "NameFlags", Kind::LocaleFlags),
    ca(
        12,
        "Description",
        Kind::Text,
        "The line's tooltip on the skills panel. 34 of the 123 rows have one.",
    ),
    c(13, "Description koKR", Kind::Locale(LOCALES[1])),
    c(14, "Description frFR", Kind::Locale(LOCALES[2])),
    c(15, "Description deDE", Kind::Locale(LOCALES[3])),
    c(16, "Description enCN", Kind::Locale(LOCALES[4])),
    c(17, "Description enTW", Kind::Locale(LOCALES[5])),
    c(18, "Description esES", Kind::Locale(LOCALES[6])),
    c(19, "Description esMX", Kind::Locale(LOCALES[7])),
    c(20, "DescriptionFlags", Kind::LocaleFlags),
    ca(
        21,
        "SpellIcon",
        Kind::Reference("SpellIcon"),
        "The picture on the line's spellbook tab.",
    ),
];

/// `SkillLineAbility.dbc`: 5,072 rows of 15 fields. A row puts one spell in
/// one skill line for the races and classes its masks name. The indices are
/// those [`super::skills`] and [`super::tradeskill`] read, and the names
/// follow vmangos' `SkillLineAbilityEntry`.
///
/// The spellbook files a spell under the skill line of the row that matches
/// the character, and a class trainer's service is dropped by the client
/// when its spell has no matching row, so a new class spell needs a row here
/// to be trainable.
///
/// vmangos does not read this file. It reads the same rows from its
/// `skill_line_ability` table, so an edit here reaches the server as a row
/// of that table and not as a copied file.
pub const SKILL_LINE_ABILITY: Schema = Schema {
    table: "SkillLineAbility",
    columns: &[
        c(0, "Id", Kind::Id),
        c(1, "Skill", Kind::Reference("SkillLine")),
        c(2, "Spell", Kind::Reference("Spell")),
        ca(
            3,
            "RaceMask",
            Kind::Flags(RACE_BITS),
            "The races the row is for. Zero is every race, on 4,955 of the \
             5,072 rows.",
        ),
        ca(
            4,
            "ClassMask",
            Kind::Flags(CLASS_BITS),
            "The classes the row is for. Zero is every class, on 3,131 rows.",
        ),
        ca(
            5,
            "ExcludeRace",
            Kind::Flags(RACE_BITS),
            "Zero on every shipped row, and vmangos has no column for it.",
        ),
        ca(
            6,
            "ExcludeClass",
            Kind::Flags(CLASS_BITS),
            "Zero on every shipped row, and vmangos has no column for it.",
        ),
        ca(
            7,
            "ReqSkillValue",
            Kind::Int,
            "The skill value a trade skill recipe needs. 1 on 5,026 rows.",
        ),
        ca(
            8,
            "SupersededBySpell",
            Kind::Reference("Spell"),
            "The next rank: the spell that replaces this one when it is learned.",
        ),
        ca(
            9,
            "LearnOnGetSkill",
            Kind::Enum(&[
                (0, "Not given with the skill"),
                (1, "Given with a profession skill"),
                (2, "Given with a race or class skill"),
            ]),
            "Whether learning the skill line teaches the spell. The values are \
             vmangos' AbilytyLearnType. 1 on 50 rows and 2 on 295.",
        ),
        ca(
            10,
            "MaxValue",
            Kind::Int,
            "The skill value at which a recipe turns grey and gives no skill.",
        ),
        ca(
            11,
            "MinValue",
            Kind::Int,
            "The skill value at which a recipe turns yellow. It is green from \
             halfway between this and MaxValue.",
        ),
        ca(12, "Unused 12", Kind::Unused, "Zero on every shipped row."),
        ca(13, "Unused 13", Kind::Unused, "Zero on every shipped row."),
        ca(
            14,
            "ReqTrainPoints",
            Kind::Int,
            "The training points a pet ability costs. Non-zero on 280 rows.",
        ),
    ],
    sections: &[
        Section {
            name: "Ability",
            fields: &[0, 1, 2, 8, 9],
        },
        Section {
            name: "Races and classes",
            fields: &[3, 4, 5, 6],
        },
        Section {
            name: "Skill values",
            fields: &[7, 11, 10, 14],
        },
        Section {
            name: "Unused",
            fields: &[12, 13],
        },
    ],
};

/// `SkillRaceClassInfo.dbc`: 201 rows of 8 fields. A row gives one skill line
/// to the races and classes its masks name, with the flags that decide how
/// the client shows it. The indices are those [`super::skills`] reads, and the
/// names follow vmangos' `SkillRaceClassInfoEntry`.
///
/// vmangos reads this file from `DataDir`.
pub const SKILL_RACE_CLASS_INFO: Schema = Schema {
    table: "SkillRaceClassInfo",
    columns: &[
        c(0, "Id", Kind::Id),
        c(1, "Skill", Kind::Reference("SkillLine")),
        ca(
            2,
            "RaceMask",
            Kind::Flags(RACE_BITS),
            "511, all nine races, on 134 of the 201 rows.",
        ),
        ca(
            3,
            "ClassMask",
            Kind::Flags(CLASS_BITS),
            "1503, all nine classes, on 60 rows.",
        ),
        ca(
            4,
            "Flags",
            Kind::Flags(&[
                (
                    super::skills::panel_flags::ALWAYS,
                    "Always listed",
                    "The skills panel lists the line whether or not the character \
                     has a value in it. No shipped row sets it.",
                ),
                (
                    super::skills::panel_flags::NEVER,
                    "Never listed",
                    "The skills panel leaves the line out. vmangos names the bit \
                     SKILL_FLAG_NO_SKILLUP_MESSAGE.",
                ),
                (
                    super::skills::panel_flags::AT_LEVEL,
                    "Listed at level",
                    "The skills panel lists the line once the character reaches \
                     MinLevel.",
                ),
                (0x010, "Always max value", "vmangos' SKILL_FLAG_ALWAYS_MAX_VALUE."),
                (
                    0x020,
                    "Can be unlearned",
                    "vmangos' SKILL_FLAG_UNLEARNABLE: the skill can be unlearned.",
                ),
                (
                    super::skills::NO_SPELLBOOK_TAB,
                    "No spellbook tab",
                    "The line's spells go on the General tab and not on a tab of \
                     their own. vmangos names the bit SKILL_FLAG_INCLUDE_IN_SORT.",
                ),
                (0x100, "Not trainable", "vmangos' SKILL_FLAG_NOT_TRAINABLE."),
                (
                    super::skills::panel_flags::PROFICIENCY,
                    "Proficiency",
                    "Rank and maximum are shown as 1, a full bar with no number. \
                     vmangos' SKILL_FLAG_MONO_VALUE.",
                ),
            ]),
            "Thirteen values over the 201 rows; 0x80 on 77 of them.",
        ),
        ca(
            5,
            "MinLevel",
            Kind::Int,
            "The character level at which a line the character does not have \
             yet is listed. Zero on 186 rows.",
        ),
        ca(
            6,
            "SkillTier",
            Kind::Reference("SkillTiers"),
            "The row of maximum values the line's ranks step through. Zero on \
             160 rows.",
        ),
        ca(7, "SkillCostIndex", Kind::Int, "0, 1 or 2."),
    ],
    sections: &[
        Section {
            name: "Skill line",
            fields: &[0, 1, 4, 5, 6, 7],
        },
        Section {
            name: "Races and classes",
            fields: &[2, 3],
        },
    ],
};

/// `ItemSet.dbc`: 172 rows of 45 fields. A row is one set: its name, up to
/// seventeen items and up to eight bonus spells, each with the number of
/// pieces that grants it. The indices are those [`super::itemset`] reads, and
/// the names follow vmangos' `ItemSetEntry`.
///
/// An item names its set in `item_template.set_id`; the item columns here are
/// what the client lists under the set's name on a tooltip. No shipped set
/// uses more than nine items or six bonuses.
///
/// vmangos reads this file from `DataDir`.
pub(crate) const ITEM_SET: Schema = Schema {
    table: "ItemSet",
    columns: &ITEM_SET_COLUMNS,
    sections: &[
        Section {
            name: "Set",
            fields: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        },
        Section {
            name: "Items",
            fields: &[10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26],
        },
        // A bonus is a spell and the pieces that grant it, eight columns
        // apart in the file and drawn together.
        Section {
            name: "Bonuses",
            fields: &[27, 35, 28, 36, 29, 37, 30, 38, 31, 39, 32, 40, 33, 41, 34, 42],
        },
        Section {
            name: "Requirement",
            fields: &[43, 44],
        },
    ],
};

/// What a bonus's piece count means, for each of the eight columns' notes.
const SET_PIECES: &str = "How many pieces of the set must be worn for the bonus \
     beside it. The client lists the bonuses in this order, whatever order the \
     row stores them in.";

const ITEM_SET_COLUMNS: [Column; 45] = [
    c(0, "Id", Kind::Id),
    c(1, "Name", Kind::Text),
    c(2, "Name koKR", Kind::Locale(LOCALES[1])),
    c(3, "Name frFR", Kind::Locale(LOCALES[2])),
    c(4, "Name deDE", Kind::Locale(LOCALES[3])),
    c(5, "Name enCN", Kind::Locale(LOCALES[4])),
    c(6, "Name enTW", Kind::Locale(LOCALES[5])),
    c(7, "Name esES", Kind::Locale(LOCALES[6])),
    c(8, "Name esMX", Kind::Locale(LOCALES[7])),
    c(9, "NameFlags", Kind::LocaleFlags),
    c(10, "Item 1", Kind::Item),
    c(11, "Item 2", Kind::Item),
    c(12, "Item 3", Kind::Item),
    c(13, "Item 4", Kind::Item),
    c(14, "Item 5", Kind::Item),
    c(15, "Item 6", Kind::Item),
    c(16, "Item 7", Kind::Item),
    c(17, "Item 8", Kind::Item),
    c(18, "Item 9", Kind::Item),
    c(19, "Item 10", Kind::Item),
    c(20, "Item 11", Kind::Item),
    c(21, "Item 12", Kind::Item),
    c(22, "Item 13", Kind::Item),
    c(23, "Item 14", Kind::Item),
    c(24, "Item 15", Kind::Item),
    c(25, "Item 16", Kind::Item),
    c(26, "Item 17", Kind::Item),
    c(27, "Bonus spell 1", Kind::Reference("Spell")),
    c(28, "Bonus spell 2", Kind::Reference("Spell")),
    c(29, "Bonus spell 3", Kind::Reference("Spell")),
    c(30, "Bonus spell 4", Kind::Reference("Spell")),
    c(31, "Bonus spell 5", Kind::Reference("Spell")),
    c(32, "Bonus spell 6", Kind::Reference("Spell")),
    c(33, "Bonus spell 7", Kind::Reference("Spell")),
    c(34, "Bonus spell 8", Kind::Reference("Spell")),
    ca(35, "Bonus pieces 1", Kind::Int, SET_PIECES),
    ca(36, "Bonus pieces 2", Kind::Int, SET_PIECES),
    ca(37, "Bonus pieces 3", Kind::Int, SET_PIECES),
    ca(38, "Bonus pieces 4", Kind::Int, SET_PIECES),
    ca(39, "Bonus pieces 5", Kind::Int, SET_PIECES),
    ca(40, "Bonus pieces 6", Kind::Int, SET_PIECES),
    ca(41, "Bonus pieces 7", Kind::Int, SET_PIECES),
    ca(42, "Bonus pieces 8", Kind::Int, SET_PIECES),
    ca(
        43,
        "RequiredSkill",
        Kind::Reference("SkillLine"),
        "The skill the wearer needs for any bonus to be active. Four shipped \
         sets have one, each at rank 300.",
    ),
    c(44, "RequiredSkillRank", Kind::Int),
];

/// `AreaTable.dbc`: 1,081 rows of 25 fields. A row is one named place: a zone,
/// or a sub-area of a zone. Each terrain chunk carries one row's id, and that
/// is the only thing that says where a character is standing. The indices are
/// those [`super::area`] reads, each measured over the shipped file; the flag
/// names follow vmangos' `AreaFlags` in `DBCEnums.h`.
///
/// The table is two levels deep: 122 rows have no parent and are zones, and
/// no shipped row's parent has a parent of its own.
///
/// vmangos does not read this file. It reads nine of the columns from its
/// `area_template` table, so an edit here reaches the server as a row of that
/// table. Fields 5 to 9 and the seven other locales are the client's alone.
pub(crate) const AREA_TABLE: Schema = Schema {
    table: "AreaTable",
    columns: &AREA_TABLE_COLUMNS,
    sections: &[
        // The seven other locale names fold away under the name, as a form
        // draws every locale run.
        Section {
            name: "Area",
            fields: &[0, 11, 12, 13, 14, 15, 16, 17, 18, 19, 1, 2, 20, 4],
        },
        Section {
            name: "Exploration",
            fields: &[3, 10],
        },
        Section {
            name: "Sound",
            fields: &[8, 9, 7, 5, 6],
        },
        Section {
            name: "Other",
            fields: &[24, 21, 22, 23],
        },
    ],
};

/// `ZoneMusic.dbc`: 99 rows of 8 fields. A row is a zone's music: a
/// `SoundEntries` row played by day and one by night, and the silence after
/// each track, a random length between a shortest and a longest, by day and by
/// night. `AreaTable`'s `ZoneMusic` column refers to a row: 447 areas refer to
/// one of 41 rows, and no area refers to the other 58. The indices are
/// [`super::sound::fields::zone_music`]'s, and each column's note gives its
/// measured counts.
pub(crate) const ZONE_MUSIC: Schema = Schema {
    table: "ZoneMusic",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(
            1,
            "SetName",
            Kind::Text,
            "The set's name, such as Zone-Forest. Shipped row 251 has none, and row 36's is \
             GhostMusic (DONOTRENAME).",
        ),
        ca(
            2,
            "SilenceIntervalMinDay",
            Kind::Int,
            "The shortest silence after a track by day, in milliseconds. 95 of the 99 shipped \
             rows wait 180,000 to 300,000 (3 to 5 minutes).",
        ),
        ca(
            3,
            "SilenceIntervalMinNight",
            Kind::Int,
            "The shortest silence after a track by night, in milliseconds. Equal to the day's in \
             98 shipped rows. Row 225's is 1,800,000, more than its SilenceIntervalMaxNight of \
             300,000.",
        ),
        ca(
            4,
            "SilenceIntervalMaxDay",
            Kind::Int,
            "The longest silence after a track by day, in milliseconds.",
        ),
        ca(
            5,
            "SilenceIntervalMaxNight",
            Kind::Int,
            "The longest silence after a track by night, in milliseconds.",
        ),
        ca(
            6,
            "SoundDay",
            Kind::Reference("SoundEntries"),
            "The music by day: a SoundEntries row whose files are the tracks; each play picks \
             one file by weight (Zone-Mountain Day). Set on 98 rows, 97 of which resolve. Every \
             row it resolves to is SoundType 28 (Music).",
        ),
        ca(
            7,
            "SoundNight",
            Kind::Reference("SoundEntries"),
            "The music by night (Zone-Mountain Night). Set on 97 rows, 96 of which resolve. The \
             same SoundEntries row as SoundDay in 89 of the 99 shipped rows.",
        ),
    ],
    sections: &[
        Section { name: "Music", fields: &[0, 1, 6, 7] },
        Section { name: "Silence between tracks", fields: &[2, 4, 3, 5] },
    ],
};

/// `ZoneIntroMusicTable.dbc`: 43 rows of 5 fields. A row is a piece played
/// once on entering an area: a name, a `SoundEntries` row, a priority and the
/// minimum time before it plays again. `AreaTable`'s `IntroMusic` column
/// refers to a row: 62 areas hold one of 25 values, 4 of those areas hold 65,
/// 67, 68 or 71, which are not rows of this table, and no area refers to 22
/// of its rows. The indices are
/// [`super::sound::fields::intro`]'s.
pub(crate) const ZONE_INTRO_MUSIC: Schema = Schema {
    table: "ZoneIntroMusicTable",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(1, "Name", Kind::Text, "The piece's name, such as Orgrimmar Intro or Spooky01."),
        ca(
            2,
            "SoundId",
            Kind::Reference("SoundEntries"),
            "The music: a SoundEntries row (Moment-Orgrimmar Intro). Set on all 43 shipped rows; \
             every value resolves, and every row it resolves to is SoundType 28 (Music).",
        ),
        ca(3, "Priority", Kind::Int, "1 in every shipped row."),
        ca(
            4,
            "MinDelayMinutes",
            Kind::Int,
            "The minimum time before the piece plays again, in minutes. 27 shipped rows wait \
             10, 8 wait 3, 7 wait 60 and one waits 120.",
        ),
    ],
    sections: &[Section { name: "Intro", fields: &[0, 1, 2, 3, 4] }],
};

/// `SoundAmbience.dbc`: 68 rows of 3 fields. A row is an area's looping
/// background sound, played under its music: one `SoundEntries` row by day and
/// one by night. `AreaTable`'s `Ambience` column refers to a row: 179 areas
/// refer to one of 27 rows, and no area refers to the other 41. The indices are
/// [`super::sound::fields::ambience`]'s.
pub(crate) const SOUND_AMBIENCE: Schema = Schema {
    table: "SoundAmbience",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(
            1,
            "AmbienceDay",
            Kind::Reference("SoundEntries"),
            "The ambience by day (BeachDay). Set on all 68 shipped rows; every value resolves, \
             and every row it resolves to is SoundType 50 (Ambience).",
        ),
        ca(
            2,
            "AmbienceNight",
            Kind::Reference("SoundEntries"),
            "The ambience by night (BeachNight). The same SoundEntries row as AmbienceDay in 45 \
             of the 68 shipped rows.",
        ),
    ],
    sections: &[Section { name: "Ambience", fields: &[0, 1, 2] }],
};

/// `ItemGroupSounds.dbc`: 24 rows of 5 fields. The sounds an item makes when
/// it is picked up and put down. `ItemDisplayInfo`'s `GroupSoundIndex` refers
/// to a row; 20,513 of the 29,604 display rows hold one of 22 rows, and no
/// display row holds row 1 or 2. The indices are
/// [`super::itemsound::fields`]'s.
pub(crate) const ITEM_GROUP_SOUNDS: Schema = Schema {
    table: "ItemGroupSounds",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(
            1,
            "SoundPickUp",
            Kind::Reference("SoundEntries"),
            "Played when the item is picked up onto the cursor or looted (PickUpCloth_Leather). \
             Rows 7 to 24 resolve, each to SoundType 9; rows 1 and 3 hold 273, which is not a \
             SoundEntries row. Row 13 holds PutDownWoodSmall.",
        ),
        ca(
            2,
            "SoundPutDown",
            Kind::Reference("SoundEntries"),
            "Played when the item is put down off the cursor (PutDownClothLeather). Rows 7 to 24 \
             resolve, each to SoundType 9; row 1 holds 274, which is not a SoundEntries row.",
        ),
        ca(
            3,
            "Sound3",
            Kind::Reference("SoundEntries"),
            "Set on rows 1, 2, 4, 5 and 6, to 273, 274 or 275, none of which is a SoundEntries \
             row. The 1.12.1 client does not read it.",
        ),
        ca(4, "Unused", Kind::Unused, "0 in every shipped row."),
    ],
    sections: &[Section { name: "Sounds", fields: &[0, 1, 2, 3, 4] }],
};

/// vmangos' `AreaFlags`, with what each bit does as its comments state it.
pub(crate) const AREA_FLAGS: [(u32, &str, &str); 10] = [
    (0x001, "Snow", "Breath is drawn. Dun Morogh, Winterspring, Naxxramas and Razorfen Downs."),
    (0x002, "Override parent snow", "Two shipped rows: in Naxxramas and Razorfen Downs."),
    (0x004, "Development", "Used on the development map only."),
    (0x008, "Trade channel", "The trade channel exists here: a capital's sub-areas."),
    (0x010, "Enemies flagged", "An enemy player here is flagged for PvP."),
    (0x020, "Resting", "A character here is resting, as in an inn."),
    (0x040, "Duels", "A duel may be started here. 969 of the 1,081 shipped rows."),
    (0x080, "Arena", "A free-for-all arena."),
    (0x100, "Capital", "A capital city's own row."),
    (0x200, "City channels", "The highest area id with this bit names the city chat channels."),
];

/// The three values the team column holds over the shipped rows.
pub(crate) const AREA_TEAMS: [(u32, &str); 3] = [(0, "Neither"), (2, "Alliance"), (4, "Horde")];

const AREA_UNUSED: &str = "Zero on all 1,081 shipped rows. No reader of this column is known.";

const AREA_TABLE_COLUMNS: [Column; 25] = [
    c(0, "Id", Kind::Id),
    ca(
        1,
        "Map",
        Kind::Reference("Map"),
        "The map the area is on. Its chunks can only be painted on that map.",
    ),
    ca(
        2,
        "Parent",
        Kind::Reference("AreaTable"),
        "The zone this area is inside, or none for a zone. The interface shows \
         the zone's name as the zone and this row's as the sub-zone. No shipped \
         row's parent has a parent.",
    ),
    ca(
        3,
        "ExploreBit",
        Kind::Int,
        "Which bit of the character's explored-zones mask says this area has \
         been seen. The shipped rows use 0 to 1076, and four values are held \
         by two rows each. The mask is 64 words, so a bit must be under 2,048.",
    ),
    ca(4, "Flags", Kind::Flags(&AREA_FLAGS), "vmangos' AreaFlags."),
    ca(
        5,
        "SoundProviderPref",
        Kind::Reference("SoundProviderPreferences"),
        "The reverb preset heard in the area. Eight shipped rows have one.",
    ),
    ca(
        6,
        "SoundProviderPrefUnderwater",
        Kind::Reference("SoundProviderPreferences"),
        "The reverb preset heard under water. 568 shipped rows hold 11 and the \
         rest none.",
    ),
    ca(
        7,
        "Ambience",
        Kind::Reference("SoundAmbience"),
        "The ambient sound, by day and by night. 179 shipped rows have one.",
    ),
    ca(
        8,
        "ZoneMusic",
        Kind::Reference("ZoneMusic"),
        "The music played here. 447 shipped rows have one; an area with none \
         keeps its zone's.",
    ),
    ca(
        9,
        "IntroMusic",
        Kind::Reference("ZoneIntroMusicTable"),
        "The piece played once on entering. 62 shipped rows have one, and four \
         of those name a row the table does not hold.",
    ),
    ca(
        10,
        "ExploreLevel",
        Kind::Signed,
        "The level of the area, which scales the experience given for \
         discovering it; 0 or less gives none. A negative value also shows \
         the area's map landmarks before it is explored. 512 shipped rows \
         hold 0.",
    ),
    ca(11, "Name", Kind::Text, "What the minimap and the zone text call the place."),
    c(12, "Name koKR", Kind::Locale(LOCALES[1])),
    c(13, "Name frFR", Kind::Locale(LOCALES[2])),
    c(14, "Name deDE", Kind::Locale(LOCALES[3])),
    c(15, "Name enCN", Kind::Locale(LOCALES[4])),
    c(16, "Name enTW", Kind::Locale(LOCALES[5])),
    c(17, "Name esES", Kind::Locale(LOCALES[6])),
    c(18, "Name esMX", Kind::Locale(LOCALES[7])),
    c(19, "NameFlags", Kind::LocaleFlags),
    ca(
        20,
        "Team",
        Kind::Enum(&AREA_TEAMS),
        "Which side the area belongs to. 25 shipped rows are the Alliance's \
         and 37 the Horde's; the rest are neither.",
    ),
    ca(21, "Unused 21", Kind::Unused, AREA_UNUSED),
    ca(22, "Unused 22", Kind::Unused, AREA_UNUSED),
    ca(23, "Unused 23", Kind::Unused, AREA_UNUSED),
    ca(
        24,
        "LiquidType",
        Kind::Reference("LiquidType"),
        "A liquid that replaces the one drawn in the area. One shipped row has \
         one: Naxxramas, 21.",
    ),
];

/// `Map.dbc`: 44 rows of 42 fields. The indices are those of
/// [`super::map::fields`]. The counts in each column's note are measured over
/// the shipped file.
pub const MAP: Schema = Schema {
    table: "Map",
    columns: &MAP_COLUMNS,
    sections: &[
        Section {
            name: "Map",
            fields: &[0, 1, 4, 5, 6, 7, 8, 9, 10, 11, 12, 2, 3, 19, 38],
        },
        Section {
            name: "Players",
            fields: &[13, 14, 15],
        },
        Section {
            name: "Descriptions",
            fields: &[
                20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37,
            ],
        },
        Section {
            name: "Unidentified",
            fields: &[16, 17, 18, 39, 40, 41],
        },
    ],
};

/// What field 2 of `Map.dbc` holds; see [`super::map::INSTANCE_TYPES`].
pub(crate) const MAP_INSTANCE_TYPES: [(u32, &str); 4] = super::map::INSTANCE_TYPES;

const MAP_COLUMNS: [Column; 42] = [
    c(0, "Id", Kind::Id),
    ca(
        1,
        "Directory",
        Kind::Text,
        "The folder under World\\Maps\\ that holds the map's WDT, and the prefix of \
         every tile's file name. Changing it on a shipped map points the map at \
         files that do not exist.",
    ),
    ca(
        2,
        "InstanceType",
        Kind::Enum(&MAP_INSTANCE_TYPES),
        "11 shipped rows are world maps, 22 dungeons, 8 raids and 3 battlegrounds.",
    ),
    ca(3, "PvP", Kind::Bool, "1 on 4 of the 44 shipped rows."),
    c(4, "Name", Kind::Text),
    c(5, "Name koKR", Kind::Locale(LOCALES[1])),
    c(6, "Name frFR", Kind::Locale(LOCALES[2])),
    c(7, "Name deDE", Kind::Locale(LOCALES[3])),
    c(8, "Name enCN", Kind::Locale(LOCALES[4])),
    c(9, "Name enTW", Kind::Locale(LOCALES[5])),
    c(10, "Name esES", Kind::Locale(LOCALES[6])),
    c(11, "Name esMX", Kind::Locale(LOCALES[7])),
    c(12, "NameFlags", Kind::LocaleFlags),
    ca(13, "MinLevel", Kind::Int, "Set on 4 shipped rows, 0 on the rest."),
    ca(14, "MaxLevel", Kind::Int, "Set on 4 shipped rows, 0 on the rest."),
    ca(15, "MaxPlayers", Kind::Int, "Set on 4 shipped rows, 0 on the rest."),
    ca(
        16,
        "Field16",
        Kind::Signed,
        "-1 on 41 of the 44 shipped rows and 0 on the other three. No reader is known.",
    ),
    ca(
        17,
        "Field17",
        Kind::Float,
        "0 on 42 shipped rows; 0.74 on Alterac Valley and -5409 on one other. No reader is known.",
    ),
    ca(
        18,
        "Field18",
        Kind::Float,
        "0 on 42 shipped rows; 0.34 on Alterac Valley and -2884 on one other. No reader is known.",
    ),
    ca(
        19,
        "Area",
        Kind::Reference("AreaTable"),
        "The area the map belongs to. 0 on 20 shipped rows. vmangos keeps its own \
         copy as map_template.linked_zone.",
    ),
    c(20, "Description 0", Kind::Text),
    c(21, "Description 0 koKR", Kind::Locale(LOCALES[1])),
    c(22, "Description 0 frFR", Kind::Locale(LOCALES[2])),
    c(23, "Description 0 deDE", Kind::Locale(LOCALES[3])),
    c(24, "Description 0 enCN", Kind::Locale(LOCALES[4])),
    c(25, "Description 0 enTW", Kind::Locale(LOCALES[5])),
    c(26, "Description 0 esES", Kind::Locale(LOCALES[6])),
    c(27, "Description 0 esMX", Kind::Locale(LOCALES[7])),
    c(28, "Description 0 Flags", Kind::LocaleFlags),
    c(29, "Description 1", Kind::Text),
    c(30, "Description 1 koKR", Kind::Locale(LOCALES[1])),
    c(31, "Description 1 frFR", Kind::Locale(LOCALES[2])),
    c(32, "Description 1 deDE", Kind::Locale(LOCALES[3])),
    c(33, "Description 1 enCN", Kind::Locale(LOCALES[4])),
    c(34, "Description 1 enTW", Kind::Locale(LOCALES[5])),
    c(35, "Description 1 esES", Kind::Locale(LOCALES[6])),
    c(36, "Description 1 esMX", Kind::Locale(LOCALES[7])),
    c(37, "Description 1 Flags", Kind::LocaleFlags),
    ca(
        38,
        "LoadingScreen",
        Kind::Reference("LoadingScreens"),
        "The picture shown while the map loads; vale loading traces the join.",
    ),
    ca(39, "Field39", Kind::Int, "0 on 42 shipped rows and 10 on two. No reader is known."),
    ca(40, "Field40", Kind::Int, "1 on 43 shipped rows and 0 on Alterac Valley. No reader is known."),
    ca(41, "Field41", Kind::Float, "1.0 on 43 shipped rows and 1.25 on one. No reader is known."),
];

/// `AreaTrigger.dbc`: 432 rows of 10 fields. The indices are those of
/// [`super::areatrigger::fields`].
pub(crate) const AREA_TRIGGER: Schema = Schema {
    table: "AreaTrigger",
    columns: &[
        c(0, "Id", Kind::Id),
        c(1, "Map", Kind::Reference("Map")),
        ca(2, "X", Kind::Float, "North, in yards: the volume's centre."),
        ca(3, "Y", Kind::Float, "West, in yards."),
        ca(4, "Z", Kind::Float, "Up, in yards."),
        ca(
            5,
            "Radius",
            Kind::Float,
            "Above 0, the volume is a sphere of this radius and the box fields are 0. \
             352 of the 432 shipped rows are spheres.",
        ),
        ca(6, "BoxLength", Kind::Float, "The box's whole length along its own x, not half."),
        ca(7, "BoxWidth", Kind::Float, "The box's whole width along its own y."),
        ca(8, "BoxHeight", Kind::Float, "The box's whole height."),
        ca(9, "BoxYaw", Kind::Float, "Radians about up, turning the box's own x from north."),
    ],
    sections: &[Section {
        name: "Volume",
        fields: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    }],
};

/// `WorldSafeLocs.dbc`: 122 rows of 14 fields. The indices are those of
/// [`super::safeloc::fields`].
pub(crate) const WORLD_SAFE_LOCS: Schema = Schema {
    table: "WorldSafeLocs",
    columns: &[
        c(0, "Id", Kind::Id),
        c(1, "Map", Kind::Reference("Map")),
        ca(2, "X", Kind::Float, "North, in yards: where the spirit appears."),
        ca(3, "Y", Kind::Float, "West, in yards."),
        ca(4, "Z", Kind::Float, "Up, in yards."),
        c(5, "Name", Kind::Text),
        c(6, "Name koKR", Kind::Locale(LOCALES[1])),
        c(7, "Name frFR", Kind::Locale(LOCALES[2])),
        c(8, "Name deDE", Kind::Locale(LOCALES[3])),
        c(9, "Name enCN", Kind::Locale(LOCALES[4])),
        c(10, "Name enTW", Kind::Locale(LOCALES[5])),
        c(11, "Name esES", Kind::Locale(LOCALES[6])),
        c(12, "Name esMX", Kind::Locale(LOCALES[7])),
        c(13, "NameFlags", Kind::LocaleFlags),
    ],
    sections: &[
        Section {
            name: "Location",
            fields: &[0, 1, 2, 3, 4],
        },
        Section {
            name: "Name",
            fields: &[5, 6, 7, 8, 9, 10, 11, 12, 13],
        },
    ],
};

/// `CreatureDisplayInfo.dbc`: 10,534 rows of 12 fields. What a creature looks
/// like: a model, a scale, and up to three skins.
///
/// The model, extra-info and skin indices are [`super::dbc`]'s
/// `display_fields`, the sound column its `npc_sound_id`. The names follow
/// wowdev.wiki's layout for 1.12, and every column's values agree with it over
/// the whole table: field 9 holds only -1 and 0 to 4, field 10 only 0 to 3.
pub(crate) const CREATURE_DISPLAY_INFO: Schema = Schema {
    table: "CreatureDisplayInfo",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(1, "ModelId", Kind::Reference("CreatureModelData"), "The model the skins are painted on."),
        ca(2, "SoundId", Kind::Reference("CreatureSoundData"), "273 of the 10,534 shipped rows set it."),
        ca(
            3,
            "ExtendedDisplayInfoId",
            Kind::Reference("CreatureDisplayInfoExtra"),
            "A character-model creature's race, gender, face, hair, worn items and baked \
             skin. Read in place of TextureVariation1 when that is empty.",
        ),
        ca(4, "Scale", Kind::Float, "Multiplied by the model's own ModelScale."),
        ca(5, "Alpha", Kind::Int, "Opacity, 0 to 255."),
        ca(
            6,
            "TextureVariation1",
            Kind::Text,
            "A skin: a file name without its folder or .blp, in the folder of the model's \
             path. The model's main skin.",
        ),
        ca(7, "TextureVariation2", Kind::Text, "A second skin, in the same folder."),
        ca(8, "TextureVariation3", Kind::Text, "A third skin, in the same folder."),
        ca(9, "SizeClass", Kind::Signed, "-1, or 0 to 4, in the shipped rows. Not read by this project."),
        ca(10, "BloodId", Kind::Int, "0 to 3 in the shipped rows; 36 rows set it. Not read by this project."),
        ca(11, "NpcSoundId", Kind::Reference("NPCSounds"), "The greeting and farewell sounds."),
    ],
    sections: &[
        Section { name: "Model", fields: &[0, 1, 4, 5, 9] },
        Section { name: "Skins", fields: &[6, 7, 8, 3] },
        Section { name: "Sound and blood", fields: &[2, 11, 10] },
    ],
};

/// `CreatureDisplayInfoExtra.dbc`: 6,939 rows of 19 fields. The appearance
/// of a creature drawn on a character model, and the skin the game baked from
/// it. The indices are [`super::dbc`]'s `display_fields`; the ten items are in
/// [`super::dbc::DisplayTables::EQUIPPED_SLOTS`]' order.
pub(crate) const CREATURE_DISPLAY_INFO_EXTRA: Schema = Schema {
    table: "CreatureDisplayInfoExtra",
    columns: &[
        c(0, "Id", Kind::Id),
        c(1, "Race", Kind::Reference("ChrRaces")),
        ca(2, "Gender", Kind::Enum(&[(0, "Male"), (1, "Female")]), "0 or 1 in every shipped row."),
        c(3, "Skin", Kind::Int),
        c(4, "Face", Kind::Int),
        c(5, "HairStyle", Kind::Int),
        c(6, "HairColour", Kind::Int),
        c(7, "FacialHair", Kind::Int),
        c(8, "ItemHead", Kind::Reference("ItemDisplayInfo")),
        c(9, "ItemShoulders", Kind::Reference("ItemDisplayInfo")),
        c(10, "ItemShirt", Kind::Reference("ItemDisplayInfo")),
        c(11, "ItemChest", Kind::Reference("ItemDisplayInfo")),
        c(12, "ItemWaist", Kind::Reference("ItemDisplayInfo")),
        c(13, "ItemLegs", Kind::Reference("ItemDisplayInfo")),
        c(14, "ItemFeet", Kind::Reference("ItemDisplayInfo")),
        c(15, "ItemWrists", Kind::Reference("ItemDisplayInfo")),
        c(16, "ItemHands", Kind::Reference("ItemDisplayInfo")),
        c(17, "ItemTabard", Kind::Reference("ItemDisplayInfo")),
        ca(
            18,
            "BakeName",
            Kind::Text,
            "The baked skin, a file in Textures\\BakedNpcTextures\\. The game ships the \
             bake; a changed face, hair or item is not seen until the skin is baked again.",
        ),
    ],
    sections: &[
        Section { name: "Body", fields: &[0, 1, 2, 3, 4, 5, 6, 7, 18] },
        Section { name: "Worn items", fields: &[8, 9, 10, 11, 12, 13, 14, 15, 16, 17] },
    ],
};

/// `CreatureModelData.dbc`: 430 rows of 16 fields. A creature model and how
/// the game treats it. The path and scale indices are [`super::dbc`]'s
/// `display_fields`. The other names follow wowdev.wiki's layout for 1.12,
/// and the values agree with it: fields 7 to 9 and 14 to 15 are floats, field
/// 10 is zero in every row.
pub(crate) const CREATURE_MODEL_DATA: Schema = Schema {
    table: "CreatureModelData",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(1, "Flags", Kind::Flags(&[]), "Nine values, up to 11, in the shipped rows. No source names the bits."),
        ca(2, "ModelName", Kind::Text, "The model's path. A .mdx name is read as the .m2 beside it."),
        ca(3, "SizeClass", Kind::Int, "0 to 4 in the shipped rows."),
        ca(4, "ModelScale", Kind::Float, "Multiplied by each display's Scale."),
        ca(5, "BloodId", Kind::Signed, "-1, 1, 2 or 3 in the shipped rows."),
        ca(6, "FootprintTextureId", Kind::Signed, "-1, or 0 to 7, in the shipped rows."),
        c(7, "FootprintLength", Kind::Float),
        c(8, "FootprintWidth", Kind::Float),
        c(9, "FootprintParticleScale", Kind::Float),
        ca(10, "FoleyMaterialId", Kind::Int, "Zero in every shipped row."),
        c(11, "FootstepShakeSize", Kind::Int),
        c(12, "DeathThudShakeSize", Kind::Int),
        c(13, "SoundId", Kind::Reference("CreatureSoundData")),
        ca(14, "CollisionWidth", Kind::Float, "Yards."),
        ca(15, "CollisionHeight", Kind::Float, "Yards."),
    ],
    sections: &[
        Section { name: "Model", fields: &[0, 2, 4, 3, 1, 14, 15] },
        Section { name: "Footprints and sound", fields: &[13, 5, 6, 7, 8, 9, 10, 11, 12] },
    ],
};

/// `CreatureSoundData.dbc`: 406 rows of 30 fields. A row is a creature's
/// combat, movement and pet sounds. `CreatureDisplayInfo.SoundId` refers to a
/// row; when it is 0, `CreatureModelData.SoundId` does. The indices
/// [`super::sound`] reads are its `fields::creature`, each pinned there.
///
/// The other columns are named after what their values resolve to over the
/// whole table, with the counts in each column's note: a column of
/// `SoundEntries` ids is named after the sounds it holds (`WingFlapSmall`,
/// `TrexStomp`, `WaterElementalLoop`). Some sound columns hold ids that
/// `SoundEntries` does not have; each column's note gives how many of its
/// values resolve.
pub(crate) const CREATURE_SOUND_DATA: Schema = Schema {
    table: "CreatureSoundData",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(1, "SoundExertion", Kind::Reference("SoundEntries"), "The attack grunt. 263 rows; every value resolves (BasiliskAttack)."),
        ca(2, "SoundExertionCritical", Kind::Reference("SoundEntries"), "27 rows; 18 resolve (HumanMaleCombatExertionCritical)."),
        ca(3, "SoundInjury", Kind::Reference("SoundEntries"), "The wound. 275 rows; 273 resolve (BasiliskWound)."),
        ca(4, "SoundInjuryCritical", Kind::Reference("SoundEntries"), "259 rows; 257 resolve (BasiliskWoundCritical)."),
        ca(
            5,
            "SoundInjuryCrushingBlow",
            Kind::Reference("SoundEntries"),
            "The wound of a crushing blow. Zero in every shipped row.",
        ),
        ca(6, "SoundDeath", Kind::Reference("SoundEntries"), "278 rows; every value resolves (BasiliskDeath)."),
        ca(7, "SoundStun", Kind::Reference("SoundEntries"), "4 rows; 1 resolves (OrcFemaleCombatStun). Not read by this project."),
        ca(
            8,
            "SoundStand",
            Kind::Reference("SoundEntries"),
            "19 rows; 10 resolve (OrcFemaleStand, GnollStand). Not read by this project.",
        ),
        ca(
            9,
            "SoundFootstep",
            Kind::Int,
            "Not a sound: a creature key of FootstepTerrainLookup, which pairs it with the \
             ground under the foot. All 244 nonzero values are keys there.",
        ),
        ca(10, "SoundAggro", Kind::Reference("SoundEntries"), "233 rows; every value resolves (BasiliskAggro)."),
        ca(11, "SoundWingFlap", Kind::Reference("SoundEntries"), "26 rows; every value resolves (WingFlapSmall). Not read by this project."),
        ca(12, "SoundWingGlide", Kind::Reference("SoundEntries"), "1 row (GenericWingGlide). Not read by this project."),
        ca(13, "SoundAlert", Kind::Reference("SoundEntries"), "The bark before aggro. 150 rows; every value resolves."),
        ca(14, "SoundFidget1", Kind::Reference("SoundEntries"), "39 rows; 23 resolve (GhoulFidget1). Not read by this project."),
        ca(15, "SoundFidget2", Kind::Reference("SoundEntries"), "15 rows; 13 resolve (TarantulaFidget2). Not read by this project."),
        ca(16, "SoundFidget3", Kind::Reference("SoundEntries"), "30 rows; every value resolves (BasiliskStand2). Not read by this project."),
        ca(17, "SoundFidget4", Kind::Reference("SoundEntries"), "8 rows; every value resolves (BasiliskStand3). Not read by this project."),
        ca(
            18,
            "SoundFidget5",
            Kind::Reference("SoundEntries"),
            "3 rows (the wolf's, the horse's, the T-rex's), each the same sound as CustomAttack1 \
             beside it. Not read by this project.",
        ),
        ca(
            19,
            "CustomAttack1",
            Kind::Reference("SoundEntries"),
            "A natural weapon's impact, played in place of a weapon's. 12 rows; every value \
             resolves (BiteMedium on the basilisk).",
        ),
        ca(20, "CustomAttack2", Kind::Reference("SoundEntries"), "1 row (WolfCustomAttack0)."),
        ca(21, "CustomAttack3", Kind::Reference("SoundEntries"), "1 row (WolfCustomAttack0)."),
        ca(22, "CustomAttack4", Kind::Reference("SoundEntries"), "Zero in every shipped row."),
        ca(
            23,
            "SoundLoop",
            Kind::Reference("SoundEntries"),
            "34 rows; every value resolves, and every name ends in Loop (WaterElementalLoop, \
             WispLoop). Not read by this project.",
        ),
        ca(
            24,
            "Unknown 24",
            Kind::Int,
            "Two rows set it, both to 1: 24, the Infernal's, and 84, the Stone Golem's. \
             Zero elsewhere.",
        ),
        ca(
            25,
            "CreatureImpactType",
            Kind::Int,
            "The creature's material, which selects a blow's impact sound, as tables::sound \
             reads it: 0 flesh, 1 chain, 2 plate. Zero in every shipped row; field 24 beside it \
             is the one two rows set.",
        ),
        ca(26, "Unknown 26", Kind::Int, "Zero in every shipped row."),
        ca(27, "SoundPetAttack", Kind::Reference("SoundEntries"), "What a pet says when sent to attack. The four warlock demons' rows (A_IMP_KILL)."),
        ca(
            28,
            "SoundPetOrder",
            Kind::Reference("SoundEntries"),
            "What a pet says when ordered to cast a special spell. The same four rows (A_IMP_ORDER).",
        ),
        ca(
            29,
            "SoundPetDismiss",
            Kind::Reference("SoundEntries"),
            "What a pet says when dismissed. The same four rows (A_Imp_Dismiss).",
        ),
    ],
    sections: &[
        Section { name: "Combat", fields: &[0, 1, 2, 3, 4, 5, 6, 7, 10, 13] },
        Section { name: "Natural weapon", fields: &[19, 20, 21, 22, 25, 24, 26] },
        Section { name: "Movement and idle", fields: &[9, 11, 12, 23, 8, 14, 15, 16, 17, 18] },
        Section { name: "Pet", fields: &[27, 28, 29] },
    ],
};

/// `NPCSounds.dbc`: 156 rows of 5 fields. A row is what a creature says when
/// it is spoken to. `CreatureDisplayInfo.NpcSoundId` refers to a row: 4,509 of
/// the 10,534 shipped displays hold one, and every value resolves.
pub(crate) const NPC_SOUNDS: Schema = Schema {
    table: "NPCSounds",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(
            1,
            "Greeting",
            Kind::Reference("SoundEntries"),
            "The greeting, such as DwarfMaleStandardNPCGreetings. Set on all 156 rows; every \
             value resolves.",
        ),
        ca(
            2,
            "Farewell",
            Kind::Reference("SoundEntries"),
            "The farewell, such as DwarfMaleStandardNPCFarewells. 118 rows; 116 resolve.",
        ),
        ca(
            3,
            "Pissed",
            Kind::Reference("SoundEntries"),
            "The annoyed line, such as DwarfMaleStandardNPCPissed. 120 rows; every value \
             resolves.",
        ),
        ca(4, "Unknown 4", Kind::Int, "Zero in every shipped row. tables::sound keeps it as a fourth sound."),
    ],
    sections: &[Section { name: "Sounds", fields: &[0, 1, 2, 3, 4] }],
};

/// `GameObjectDisplayInfo.dbc`: 1,638 rows of 12 fields. A game object's
/// model and its ten sounds. The path index is [`super::dbc`]'s
/// `display_fields`; the ten sounds follow wowdev.wiki's layout for 1.12.
pub(crate) const GAME_OBJECT_DISPLAY_INFO: Schema = Schema {
    table: "GameObjectDisplayInfo",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(
            1,
            "ModelName",
            Kind::Text,
            "The model's path. A .mdx name is read as the .m2 beside it; a few rows name a \
             .wmo.",
        ),
        c(2, "Sound1", Kind::Reference("SoundEntries")),
        c(3, "Sound2", Kind::Reference("SoundEntries")),
        c(4, "Sound3", Kind::Reference("SoundEntries")),
        c(5, "Sound4", Kind::Reference("SoundEntries")),
        c(6, "Sound5", Kind::Reference("SoundEntries")),
        c(7, "Sound6", Kind::Reference("SoundEntries")),
        c(8, "Sound7", Kind::Reference("SoundEntries")),
        c(9, "Sound8", Kind::Reference("SoundEntries")),
        c(10, "Sound9", Kind::Reference("SoundEntries")),
        c(11, "Sound10", Kind::Reference("SoundEntries")),
    ],
    sections: &[
        Section { name: "Model", fields: &[0, 1] },
        Section { name: "Sounds", fields: &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11] },
    ],
};

/// `ItemDisplayInfo.dbc`: 29,604 rows of 23 fields. What an item looks like
/// in the bag, in the hand and on the body. The indices are
/// [`super::item`]'s `fields`, each with its measurement there.
pub(crate) const ITEM_DISPLAY_INFO: Schema = Schema {
    table: "ItemDisplayInfo",
    columns: &[
        c(0, "Id", Kind::Id),
        ca(1, "ModelName1", Kind::Text, "A model the item hangs on the wearer: a weapon, a shoulder, a helm."),
        ca(2, "ModelName2", Kind::Text, "The second model: the other shoulder."),
        ca(3, "ModelTexture1", Kind::Text, "ModelName1's skin, without its folder or .blp."),
        ca(4, "ModelTexture2", Kind::Text, "ModelName2's skin."),
        ca(5, "InventoryIcon", Kind::Text, "The bag icon, a file in Interface\\Icons\\ without .blp."),
        ca(6, "GeosetGroup1", Kind::Int, "A variant of the wearer's own geometry; the slot decides which."),
        c(7, "GeosetGroup2", Kind::Int),
        c(8, "GeosetGroup3", Kind::Int),
        ca(9, "Flags", Kind::Flags(&[]), "No source names the bits."),
        ca(10, "SpellVisualId", Kind::Reference("SpellVisual"), "The glow a weapon carries. Not read by this project."),
        ca(11, "GroupSoundIndex", Kind::Reference("ItemGroupSounds"), "The sounds of picking it up and putting it down."),
        ca(12, "HelmetGeosetVis1", Kind::Reference("HelmetGeosetVisData"), "What a helm hides on a male wearer."),
        ca(13, "HelmetGeosetVis2", Kind::Reference("HelmetGeosetVisData"), "What a helm hides on a female wearer."),
        ca(14, "TextureArmUpper", Kind::Text, "A body texture, a file in Item\\TextureComponents\\ArmUpperTexture\\."),
        c(15, "TextureArmLower", Kind::Text),
        c(16, "TextureHand", Kind::Text),
        c(17, "TextureTorsoUpper", Kind::Text),
        c(18, "TextureTorsoLower", Kind::Text),
        c(19, "TextureLegUpper", Kind::Text),
        c(20, "TextureLegLower", Kind::Text),
        c(21, "TextureFoot", Kind::Text),
        ca(22, "ItemVisual", Kind::Reference("ItemVisuals"), "The effects an enchanted weapon shows."),
    ],
    sections: &[
        Section { name: "Icon and models", fields: &[0, 5, 1, 3, 2, 4] },
        Section { name: "On the body", fields: &[14, 15, 16, 17, 18, 19, 20, 21, 6, 7, 8, 12, 13] },
        Section { name: "Effects and sound", fields: &[22, 10, 11, 9] },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The five display schemas are the widths their files are (`vale dbc`
    /// over the 1.12.1 archives), number their columns in order, and put each
    /// field in exactly one section.
    #[test]
    fn the_display_schemas_are_their_files_widths() {
        for (schema, width) in [
            (&CREATURE_DISPLAY_INFO, 12),
            (&CREATURE_DISPLAY_INFO_EXTRA, 19),
            (&CREATURE_MODEL_DATA, 16),
            (&CREATURE_SOUND_DATA, 30),
            (&NPC_SOUNDS, 5),
            (&GAME_OBJECT_DISPLAY_INFO, 12),
            (&ITEM_DISPLAY_INFO, 23),
        ] {
            assert_eq!(schema.columns.len(), width, "{}", schema.table);
            for (n, column) in schema.columns.iter().enumerate() {
                assert_eq!(column.field, n, "{}.{}", schema.table, column.name);
            }
            let mut drawn: Vec<usize> = schema.sections.iter().flat_map(|s| s.fields.iter().copied()).collect();
            drawn.sort_unstable();
            assert_eq!(drawn, (0..width).collect::<Vec<_>>(), "{}", schema.table);
            assert!(for_table(schema.table).is_some(), "{} is in ALL", schema.table);
        }
    }

    /// The `ZoneMusic` schema names the columns `tables::sound` reads, at the
    /// indices it reads them, and `AreaTable` refers to it from the column the
    /// sound bank reads.
    #[test]
    fn the_zone_music_schema_agrees_with_the_module_that_reads_it() {
        use super::super::sound::fields::zone_music as f;
        let column = |field: usize| ZONE_MUSIC.columns[field];
        assert_eq!(ZONE_MUSIC.columns.len(), 8, "ZoneMusic.dbc is 8 fields");
        assert_eq!(column(f::SET_NAME).name, "SetName");
        assert_eq!(column(f::SILENCE_MIN).name, "SilenceIntervalMinDay");
        assert_eq!(column(f::SILENCE_MIN + 1).name, "SilenceIntervalMinNight");
        assert_eq!(column(f::SILENCE_MAX).name, "SilenceIntervalMaxDay");
        assert_eq!(column(f::SILENCE_MAX + 1).name, "SilenceIntervalMaxNight");
        assert_eq!(ZONE_MUSIC.references_to("SoundEntries"), [f::SOUND, f::SOUND + 1]);
        assert_eq!(column(f::SOUND).name, "SoundDay");
        assert_eq!(AREA_TABLE.references_to("ZoneMusic"), [super::super::sound::fields::area::ZONE_MUSIC]);
    }

    /// The intro and ambience schemas name the columns `tables::sound` reads,
    /// and `AreaTable` names a row of each.
    #[test]
    fn the_intro_and_ambience_schemas_agree_with_the_module_that_reads_them() {
        use super::super::sound::fields::{ambience, area, intro};
        assert_eq!(ZONE_INTRO_MUSIC.columns.len(), 5, "ZoneIntroMusicTable.dbc is 5 fields");
        assert_eq!(ZONE_INTRO_MUSIC.references_to("SoundEntries"), [intro::SOUND]);
        assert_eq!(ZONE_INTRO_MUSIC.columns[intro::MIN_DELAY_MINUTES].name, "MinDelayMinutes");
        assert_eq!(SOUND_AMBIENCE.columns.len(), 3, "SoundAmbience.dbc is 3 fields");
        assert_eq!(SOUND_AMBIENCE.references_to("SoundEntries"), [ambience::SOUND, ambience::SOUND + 1]);
        assert_eq!(AREA_TABLE.references_to("SoundAmbience"), [area::AMBIENCE]);
        assert_eq!(AREA_TABLE.references_to("ZoneIntroMusicTable"), [area::INTRO_MUSIC]);
    }

    /// The item sound schema names the columns `tables::itemsound` reads, and
    /// `ItemDisplayInfo` refers to a row of it through field 11.
    #[test]
    fn the_item_sound_schema_agrees_with_the_module_that_reads_it() {
        use super::super::itemsound::fields as f;
        assert_eq!(ITEM_GROUP_SOUNDS.columns.len(), f::COUNT);
        assert_eq!(ITEM_GROUP_SOUNDS.columns[f::PICK_UP].name, "SoundPickUp");
        assert_eq!(ITEM_GROUP_SOUNDS.columns[f::PUT_DOWN].name, "SoundPutDown");
        assert_eq!(ITEM_GROUP_SOUNDS.columns[f::THIRD].name, "Sound3");
        assert_eq!(ITEM_GROUP_SOUNDS.columns[f::UNUSED].kind, Kind::Unused);
        assert_eq!(ITEM_DISPLAY_INFO.references_to("ItemGroupSounds"), [11]);
    }

    /// Every `SoundType` is named once, in order.
    #[test]
    fn every_sound_type_is_named_once() {
        assert!(SOUND_TYPES.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(SOUND_TYPES.iter().find(|(value, _)| *value == 28).map(|(_, name)| *name), Some("Music"));
    }

    /// The creature sound schema names the columns `tables::sound` reads, as
    /// what that module reads them as: sound ids where it reads a sound, and
    /// the footstep key and the impact type as numbers.
    #[test]
    fn the_creature_sound_schema_agrees_with_the_module_that_reads_it() {
        use super::super::sound::fields::creature as f;
        let column = |field: usize| CREATURE_SOUND_DATA.columns[field];
        for (field, name) in [
            (f::EXERTION, "SoundExertion"),
            (f::WOUND, "SoundInjury"),
            (f::WOUND_CRITICAL, "SoundInjuryCritical"),
            (f::WOUND_CRUSHING, "SoundInjuryCrushingBlow"),
            (f::DEATH, "SoundDeath"),
            (f::AGGRO, "SoundAggro"),
            (f::ALERT, "SoundAlert"),
            (f::CUSTOM_ATTACK, "CustomAttack1"),
            (f::PET_ATTACK, "SoundPetAttack"),
            (f::PET_ORDER, "SoundPetOrder"),
            (f::PET_DISMISS, "SoundPetDismiss"),
        ] {
            assert_eq!(column(field).name, name);
            assert_eq!(column(field).kind, Kind::Reference("SoundEntries"), "{name}");
        }
        assert_eq!(column(f::CUSTOM_ATTACK + f::CUSTOM_ATTACK_COUNT - 1).name, "CustomAttack4");
        assert_eq!(column(f::FOOTSTEP).kind, Kind::Int, "a footstep key is not a sound");
        assert_eq!(column(f::IMPACT_TYPE).name, "CreatureImpactType");
        assert_eq!(CREATURE_DISPLAY_INFO.references_to("CreatureSoundData"), [2]);
        assert_eq!(CREATURE_MODEL_DATA.references_to("CreatureSoundData"), [13]);
        assert_eq!(CREATURE_DISPLAY_INFO.references_to("NPCSounds"), [11]);
        assert_eq!(NPC_SOUNDS.references_to("SoundEntries"), [1, 2, 3]);
        assert!(chain_for("CreatureDisplayInfo").contains(&"SoundEntries"));
        assert_eq!(chain_for("NPCSounds"), chain_for("CreatureSoundData"));
    }

    /// The three place schemas are the width their files are, put each field in
    /// exactly one section, and name the fields their modules read.
    #[test]
    fn the_place_schemas_agree_with_the_modules_that_read_them() {
        use super::super::{areatrigger::fields as at, map::fields as mf, safeloc::fields as sf};
        for (schema, count) in [(&MAP, mf::COUNT), (&AREA_TRIGGER, at::COUNT), (&WORLD_SAFE_LOCS, sf::COUNT)] {
            assert_eq!(schema.columns.len(), count, "{}", schema.table);
            for (n, column) in schema.columns.iter().enumerate() {
                assert_eq!(column.field, n, "{}.{}", schema.table, column.name);
            }
            let mut drawn: Vec<usize> = schema
                .sections
                .iter()
                .flat_map(|section| section.fields.iter().copied())
                .collect();
            drawn.sort_unstable();
            assert_eq!(drawn, (0..count).collect::<Vec<_>>(), "{}", schema.table);
        }
        assert_eq!(MAP.columns[mf::DIRECTORY].name, "Directory");
        assert_eq!(MAP.columns[mf::AREA].kind, Kind::Reference("AreaTable"));
        assert_eq!(MAP.columns[mf::LOADING_SCREEN].kind, Kind::Reference("LoadingScreens"));
        assert_eq!(AREA_TRIGGER.columns[at::RADIUS].name, "Radius");
        assert_eq!(AREA_TRIGGER.columns[at::BOX_YAW].name, "BoxYaw");
        assert_eq!(WORLD_SAFE_LOCS.columns[sf::NAME].kind, Kind::Text);
        assert!(for_table("worldsafelocs").is_some());
    }

    /// The area schema is the width the file is and names the fields
    /// `tables::area` reads, so a form and the reader cannot drift apart.
    #[test]
    fn the_area_schema_agrees_with_the_module_that_reads_it() {
        use super::super::area::fields as af;
        assert_eq!(AREA_TABLE.columns.len(), af::COUNT);
        let named = |field: usize| AREA_TABLE.columns[field].name;
        assert_eq!(named(af::MAP_ID), "Map");
        assert_eq!(named(af::PARENT), "Parent");
        assert_eq!(named(af::AREA_BIT), "ExploreBit");
        assert_eq!(named(af::FLAGS), "Flags");
        assert_eq!(named(af::ZONE_MUSIC), "ZoneMusic");
        assert_eq!(named(af::EXPLORE_LEVEL), "ExploreLevel");
        assert_eq!(named(af::NAME), "Name");
        assert_eq!(named(af::NAME_FLAGS), "NameFlags");
        assert_eq!(named(af::TEAM), "Team");
        assert_eq!(named(af::LIQUID_TYPE), "LiquidType");
        assert_eq!(AREA_TABLE.columns[af::PARENT].kind, Kind::Reference("AreaTable"));
        // The two flags `tables::area` reads are in the list under their bits.
        let bit = |name: &str| AREA_FLAGS.iter().find(|flag| flag.1 == name).map(|flag| flag.0);
        assert_eq!(bit("Trade channel"), Some(super::super::area::AREA_FLAG_SLAVE_CAPITAL));
        assert_eq!(bit("City channels"), Some(super::super::area::AREA_FLAG_CITY));
        // Every field is in exactly one section.
        let mut drawn: Vec<usize> = AREA_TABLE
            .sections
            .iter()
            .flat_map(|section| section.fields.iter().copied())
            .collect();
        drawn.sort_unstable();
        assert_eq!(drawn, (0..af::COUNT).collect::<Vec<_>>());
    }

    /// Every named mask is a distinct single bit.
    ///
    /// A `Kind::Flags` list must have this shape for one checkbox per entry to
    /// represent the value exactly. A duplicate mask gives two checkboxes that
    /// control the same bit, and an entry with two bits gives a checkbox that
    /// cannot represent one bit set without the other. A table taken from the
    /// server's enums can make both mistakes: vmangos declares composite values
    /// in some of these enums (`CREATURE_TYPEMASK_HUMANOID_OR_UNDEAD` is one),
    /// and they must not be picked up.
    #[test]
    fn a_named_mask_is_one_bit_per_row_and_no_repeats() {
        for schema in ALL {
            for column in schema.columns {
                let Kind::Flags(bits) = column.kind else {
                    continue;
                };
                let mut seen = 0u32;
                for (mask, name, _) in bits {
                    assert_eq!(
                        mask.count_ones(),
                        1,
                        "{}.{} bit {name} is {mask:#010X}, which is not one bit",
                        schema.table,
                        column.name
                    );
                    assert_eq!(
                        seen & mask,
                        0,
                        "{}.{} names {mask:#010X} twice, the second time {name}",
                        schema.table,
                        column.name
                    );
                    seen |= mask;
                }
            }
        }
    }

    /// `named_bits` and `unnamed_bits` together account for the whole value.
    #[test]
    fn the_named_bits_and_the_unnamed_ones_are_the_whole_value() {
        let bits = &super::super::spellbits::ATTRIBUTES;
        // Two named bits and one unnamed bit. All 32 bits of `Attributes` are
        // named, so the unnamed case is built from a shortened list.
        let short: &[(u32, &str, &str)] = &bits[..2];
        let value = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;
        assert_eq!(named_bits(value, short).len(), 2);
        assert_eq!(unnamed_bits(value, short), 0x0000_0004);
        // The full list leaves no unnamed bits.
        assert_eq!(unnamed_bits(value, bits), 0);
        assert_eq!(named_bits(0, bits), Vec::<&str>::new());
    }

    /// The `Spell` mask columns that have bit names, listed by name rather
    /// than counted.
    ///
    /// A column that loses its names, for example after an enum is renamed,
    /// fails this test instead of
    /// silently being drawn as a plain hex field again.
    #[test]
    fn the_spell_masks_that_have_names_have_them() {
        let named: Vec<&str> = SPELL
            .columns
            .iter()
            .filter(|column| matches!(column.kind, Kind::Flags(bits) if !bits.is_empty()))
            .map(|column| column.name)
            .collect();
        assert_eq!(
            named,
            [
                "Attributes",
                "AttributesEx",
                "AttributesEx2",
                "AttributesEx3",
                "AttributesEx4",
                "Stances",
                "StancesNot",
                "Targets",
                "TargetCreatureType",
                "InterruptFlags",
                "AuraInterruptFlags",
                "ProcFlags",
            ]
        );
    }

    /// The `Spell` columns drawn as a drop-down have a non-empty list of
    /// names.
    #[test]
    fn the_spell_columns_that_became_a_choice_are_a_choice() {
        for name in [
            "CasterAuraState",
            "TargetAuraState",
            "EquippedItemClass",
            "SpellFamilyName",
            "EffectImplicitTargetA 1",
            "EffectImplicitTargetB 3",
        ] {
            let column = SPELL
                .columns
                .iter()
                .find(|column| column.name == name)
                .unwrap_or_else(|| panic!("no column {name}"));
            assert!(
                matches!(column.kind, Kind::Enum(names) if !names.is_empty()),
                "{name} is {:?}",
                column.kind
            );
        }
    }

    #[test]
    fn every_column_is_at_its_own_index() {
        for (at, column) in SPELL.columns.iter().enumerate() {
            assert_eq!(column.field, at, "{} is at {at}", column.name);
        }
        assert_eq!(SPELL.columns.len(), 173, "Spell.dbc is 173 fields");
    }

    #[test]
    fn every_field_is_in_exactly_one_section() {
        let mut seen = vec![0usize; SPELL.columns.len()];
        for section in SPELL.sections {
            for &field in section.fields {
                assert!(field < seen.len(), "{}: field {field}", section.name);
                seen[field] += 1;
            }
        }
        let missing: Vec<usize> = (0..seen.len()).filter(|&at| seen[at] == 0).collect();
        let twice: Vec<usize> = (0..seen.len()).filter(|&at| seen[at] > 1).collect();
        assert!(missing.is_empty(), "in no section: {missing:?}");
        assert!(twice.is_empty(), "in two sections: {twice:?}");
    }

    /// The schema's column positions match the measured constants in
    /// `spell_fields`.
    ///
    /// Sixty of those constants were pinned one at a time, each with the spell
    /// it was measured on written beside it. Without this test the schema would
    /// be an unchecked second copy of that layout. Every constant
    /// `spell_fields` publishes is asserted here, by name.
    #[test]
    fn the_measured_constants_and_this_table_agree() {
        use super::super::spellbook::spell_fields as f;
        let named: &[(usize, &str)] = &[
            (f::CATEGORY, "Category"),
            (f::CAST_UI, "CastUI"),
            (f::DISPEL, "Dispel"),
            (f::ATTRIBUTES, "Attributes"),
            (f::ATTRIBUTES_EX, "AttributesEx"),
            (f::STANCES, "Stances"),
            (f::STANCES_NOT, "StancesNot"),
            (f::TARGETS, "Targets"),
            (f::REQUIRES_SPELL_FOCUS, "RequiresSpellFocus"),
            (f::CASTER_AURA_STATE, "CasterAuraState"),
            (f::TARGET_AURA_STATE, "TargetAuraState"),
            (f::CASTING_TIME_INDEX, "CastingTimeIndex"),
            (f::RECOVERY_TIME, "RecoveryTime"),
            (f::CATEGORY_RECOVERY_TIME, "CategoryRecoveryTime"),
            (f::INTERRUPT_FLAGS, "InterruptFlags"),
            (f::AURA_INTERRUPT_FLAGS, "AuraInterruptFlags"),
            (f::CHANNEL_INTERRUPT_FLAGS, "ChannelInterruptFlags"),
            (f::PROC_CHANCE, "ProcChance"),
            (f::MAX_LEVEL, "MaxLevel"),
            (f::BASE_LEVEL, "BaseLevel"),
            (f::SPELL_LEVEL, "SpellLevel"),
            (f::DURATION_INDEX, "DurationIndex"),
            (f::POWER_TYPE, "PowerType"),
            (f::MANA_COST, "ManaCost"),
            (f::RANGE_INDEX, "RangeIndex"),
            (f::TOTEM, "Totem 1"),
            (f::REAGENT, "Reagent 1"),
            (f::REAGENT_COUNT, "ReagentCount 1"),
            (f::EQUIPPED_ITEM_CLASS, "EquippedItemClass"),
            (f::EQUIPPED_ITEM_SUBCLASS_MASK, "EquippedItemSubClassMask"),
            (
                f::EQUIPPED_ITEM_INVENTORY_TYPE_MASK,
                "EquippedItemInventoryTypeMask",
            ),
            (f::EFFECT, "Effect 1"),
            (f::EFFECT_DIE_SIDES, "EffectDieSides 1"),
            (f::EFFECT_BASE_DICE, "EffectBaseDice 1"),
            (f::EFFECT_DICE_PER_LEVEL, "EffectDicePerLevel 1"),
            (
                f::EFFECT_REAL_POINTS_PER_LEVEL,
                "EffectRealPointsPerLevel 1",
            ),
            (f::EFFECT_BASE_POINTS, "EffectBasePoints 1"),
            (f::IMPLICIT_TARGET_A, "EffectImplicitTargetA 1"),
            (f::IMPLICIT_TARGET_B, "EffectImplicitTargetB 1"),
            (f::EFFECT_RADIUS_INDEX, "EffectRadiusIndex 1"),
            (f::EFFECT_APPLY_AURA, "EffectApplyAuraName 1"),
            (f::EFFECT_AMPLITUDE, "EffectAmplitude 1"),
            (f::EFFECT_CHAIN_TARGET, "EffectChainTarget 1"),
            (f::EFFECT_ITEM_TYPE, "EffectItemType 1"),
            (f::EFFECT_MISC_VALUE, "EffectMiscValue 1"),
            (f::EFFECT_TRIGGER_SPELL, "EffectTriggerSpell 1"),
            (f::ICON_ID, "SpellIconID"),
            (f::ACTIVE_ICON_ID, "ActiveIconID"),
            (f::NAME, "Name"),
            (f::RANK, "Rank"),
            (f::DESCRIPTION, "Description"),
            (f::AURA_DESCRIPTION, "AuraDescription"),
            (f::START_RECOVERY_CATEGORY, "StartRecoveryCategory"),
            (f::START_RECOVERY_TIME, "StartRecoveryTime"),
            (f::SPELL_FAMILY_NAME, "SpellFamilyName"),
            (f::SPELL_FAMILY_FLAGS_LOW, "SpellFamilyFlags low"),
            (f::SPELL_FAMILY_FLAGS_HIGH, "SpellFamilyFlags high"),
            (f::MAX_AFFECTED_TARGETS, "MaxAffectedTargets"),
            (f::PREVENTION_TYPE, "PreventionType"),
            (f::SHAPESHIFT_ORDER, "StanceBarOrder"),
            (f::MIN_FACTION_ID, "MinFactionId"),
            (f::MIN_REPUTATION, "MinReputation"),
        ];
        for &(field, name) in named {
            assert_eq!(
                SPELL.columns[field].name, name,
                "field {field} is {} here and {name} in spell_fields",
                SPELL.columns[field].name
            );
        }
    }

    /// `SPELL_ICON_FIELD`, the one field index published as its own constant,
    /// is the measured icon column.
    #[test]
    fn the_icon_field_is_the_one_the_measurements_name() {
        use super::super::spellbook::spell_fields;
        assert_eq!(SPELL_ICON_FIELD, spell_fields::ICON_ID);
        assert_eq!(SPELL.columns[SPELL_ICON_FIELD].name, "SpellIconID");
    }


    /// The visual chain's indices match those in `tables::spell`, which
    /// measured them.
    ///
    /// That module walks Spell -> SpellVisual -> SpellVisualKit on every cast,
    /// and each of its column constants has the spell it was pinned on written
    /// beside it. This schema describes the same three files for a form and a
    /// writer, so without this test there would be two readings of one layout.
    /// A writer that goes through the schema damages the file if it is wrong.
    #[test]
    fn the_visual_chain_agrees_with_the_module_that_walks_it() {
        use super::super::spell::fields as f;
        let named: &[(&Schema, usize, &str)] = &[
            (&SPELL_VISUAL, f::PRECAST_KIT, "PrecastKit"),
            (&SPELL_VISUAL, f::CAST_KIT, "CastKit"),
            (&SPELL_VISUAL, f::IMPACT_KIT, "ImpactKit"),
            (&SPELL_VISUAL, f::STATE_KIT, "StateKit"),
            (&SPELL_VISUAL, f::CHANNEL_KIT, "ChannelKit"),
            (&SPELL_VISUAL, f::HAS_MISSILE, "HasMissile"),
            (&SPELL_VISUAL, f::MISSILE_MODEL, "MissileModel"),
            (&SPELL_VISUAL, f::MISSILE_PATH_TYPE, "MissilePathType"),
            (&SPELL_VISUAL, f::MISSILE_DESTINATION, "MissileDestination"),
            (&SPELL_VISUAL, f::AREA_FLAG, "HasAreaEffect"),
            (&SPELL_VISUAL, f::AREA_MODEL, "AreaModel"),
            (&SPELL_VISUAL, f::AREA_KIT, "AreaKit"),
            (&SPELL_VISUAL_KIT, f::ANIMATION, "Animation"),
            (&SPELL_VISUAL_KIT, f::CHAR_PROC, "CharProc 1"),
            (&SPELL_VISUAL_KIT, f::CHAR_PARAM_ZERO, "CharParamZero 1"),
            (&SPELL_VISUAL_KIT, f::CHAR_PARAM_ONE, "CharParamOne 1"),
            (&SPELL_VISUAL_KIT, f::CHAR_PARAM_TWO, "CharParamTwo 1"),
            (&SPELL_VISUAL_KIT, f::CHAR_PARAM_THREE, "CharParamThree 1"),
            (&SPELL_VISUAL_EFFECT_NAME, f::EFFECT_NAME, "Name"),
            (&SPELL_VISUAL_EFFECT_NAME, f::EFFECT_MODEL, "Model"),
            (&SPELL_VISUAL_EFFECT_NAME, f::EFFECT_SCALE, "Scale"),
            (&SPELL_CHAIN_EFFECTS, f::CHAIN_AVG_SEG_LEN, "AverageSegmentLength"),
            (&SPELL_CHAIN_EFFECTS, f::CHAIN_WIDTH, "Width"),
            (&SPELL_CHAIN_EFFECTS, f::CHAIN_NOISE_SCALE, "NoiseScale"),
            (&SPELL_CHAIN_EFFECTS, f::CHAIN_TEX_COORD_SCALE, "TexCoordScale"),
            (&SPELL_CHAIN_EFFECTS, f::CHAIN_SEG_DURATION, "SegmentDuration"),
            (&SPELL_CHAIN_EFFECTS, f::CHAIN_SEG_DELAY, "SegmentDelay"),
            (&SPELL_CHAIN_EFFECTS, f::CHAIN_TEXTURE, "Texture"),
            (&SPELL, f::SPELL_VISUAL, "SpellVisual"),
        ];
        for &(schema, field, name) in named {
            assert_eq!(
                schema.columns[field].name, name,
                "{} field {field} is {} here",
                schema.table, schema.columns[field].name
            );
        }
        // The model columns, which that module holds as (field, attachment)
        // pairs rather than as constants.
        for (field, _) in vale_assets_effect_points() {
            assert!(
                matches!(
                    SPELL_VISUAL_KIT.columns[field].kind,
                    Kind::Reference("SpellVisualEffectName")
                ),
                "kit field {field} is drawn and is not a model here",
            );
        }
    }

    fn vale_assets_effect_points() -> [(usize, u32); 6] {
        super::super::spell::EFFECT_POINTS
    }

    /// Every schema in `ALL` indexes its columns by field and puts each field
    /// in exactly one section, as the spell schema does.
    #[test]
    fn every_schema_indexes_and_sections_its_own_columns() {
        for schema in ALL {
            for (at, column) in schema.columns.iter().enumerate() {
                assert_eq!(column.field, at, "{}: {}", schema.table, column.name);
            }
            let mut seen = vec![0usize; schema.columns.len()];
            for section in schema.sections {
                for &field in section.fields {
                    assert!(field < seen.len(), "{}: field {field}", schema.table);
                    seen[field] += 1;
                }
            }
            let missing: Vec<usize> = (0..seen.len()).filter(|&at| seen[at] == 0).collect();
            let twice: Vec<usize> = (0..seen.len()).filter(|&at| seen[at] > 1).collect();
            assert!(missing.is_empty(), "{}: in no section {missing:?}", schema.table);
            assert!(twice.is_empty(), "{}: in two sections {twice:?}", schema.table);
        }
    }

    /// Every table a reference column names has a schema here, or is in the
    /// list of tables deliberately left without one.
    ///
    /// A reference to a table with no schema still works: the browser opens it
    /// as numbered fields. The list makes those tables explicit, and a typo in
    /// a table name fails here instead of when somebody follows the reference.
    #[test]
    fn every_reference_target_is_either_described_or_named_here() {
        // These are referenced deliberately without a schema of their own.
        // Each opens as numbered fields, which is better than a reference the
        // browser refuses to follow.
        const UNDESCRIBED: [&str; 10] = [
            "ChrRaces",
            "HelmetGeosetVisData",
            "ItemVisuals",
            "Faction",
            "LiquidType",
            "LoadingScreens",
            "SkillCostsData",
            "SkillLineCategory",
            "SkillTiers",
            "SoundProviderPreferences",
        ];
        for target in UNDESCRIBED {
            assert!(TABLE_NAMES.contains(&target), "{target} is not a table");
        }
        for schema in ALL {
            for column in schema.columns {
                let Kind::Reference(target) = column.kind else {
                    continue;
                };
                assert!(
                    for_table(target).is_some() || UNDESCRIBED.contains(&target),
                    "{}.{} points at {target}, which is neither described nor \
                     listed as deliberately undescribed",
                    schema.table,
                    column.name
                );
            }
        }
    }

    /// The `SoundEntries` indices match the sound bank's, which reads every row
    /// through them. The visual chain is held to the same rule.
    #[test]
    fn the_sound_table_agrees_with_the_bank_that_reads_it() {
        use super::super::sound::fields::entry as f;
        let named: &[(usize, &str)] = &[
            (f::KIND, "SoundType"),
            (f::NAME, "Name"),
            (f::FILE, "File 1"),
            (f::FILE + f::FILE_COUNT - 1, "File 10"),
            (f::FREQ, "Weight 1"),
            (f::DIRECTORY, "Directory"),
            (f::VOLUME, "Volume"),
            (f::FLAGS, "Flags"),
            (f::MIN_DISTANCE, "MinDistance"),
            (f::CUTOFF, "DistanceCutoff"),
        ];
        for &(field, name) in named {
            assert_eq!(SOUND_ENTRIES.columns[field].name, name, "field {field}");
        }
        assert_eq!(SOUND_ENTRIES.columns.len(), 29, "SoundEntries.dbc is 29 fields");
    }

    /// The five small tables the spell row points at, at the widths the
    /// archives state (`vale dbc <Table>`).
    #[test]
    fn the_small_spell_tables_are_the_width_the_files_are() {
        assert_eq!(SPELL_RANGE.columns.len(), 22);
        assert_eq!(SPELL_DISPEL_TYPE.columns.len(), 12);
        assert_eq!(SPELL_MECHANIC.columns.len(), 10);
        assert_eq!(SPELL_FOCUS_OBJECT.columns.len(), 10);
        assert_eq!(SPELL_CATEGORY.columns.len(), 2);
        // The one `SpellRange` column the preview reads is the maximum range.
        assert_eq!(SPELL_RANGE.columns[2].name, "RangeMax");
    }

    /// A `Bool` column is one whose values over the population are only 0 and
    /// 1; the `about` on each states the measurement. There are two, the gates
    /// on the missile and area blocks.
    #[test]
    fn the_two_gates_are_bools() {
        assert_eq!(SPELL_VISUAL.columns[6].kind, Kind::Bool);
        assert_eq!(SPELL_VISUAL.columns[11].kind, Kind::Bool);
    }

    #[test]
    fn a_reference_can_be_asked_for_backwards() {
        let to_spell = SPELL.references_to("Spell");
        assert!(to_spell.contains(&109), "EffectTriggerSpell 1");
        assert!(to_spell.contains(&38), "ModalNextSpell");
        assert!(SPELL.references_to("SpellVisual").contains(&115));
        assert!(SPELL.references_to("NotATable").is_empty());
    }

    #[test]
    fn the_table_is_found_by_name_either_way_around() {
        assert!(for_table("Spell").is_some());
        assert!(for_table("spell").is_some());
        assert!(for_table("Creature").is_none());
    }

    /// Every table has one spelling: the list holds no name twice in any
    /// case, every described table is in it under its own spelling, and a
    /// name asked for in the archives' lower case comes back as that spelling.
    #[test]
    fn a_table_has_one_name_whatever_case_it_is_asked_in() {
        let mut lower: Vec<String> =
            TABLE_NAMES.iter().map(|name| name.to_ascii_lowercase()).collect();
        lower.sort();
        let count = lower.len();
        lower.dedup();
        assert_eq!(lower.len(), count);
        for schema in ALL {
            assert!(TABLE_NAMES.contains(&schema.table), "{}", schema.table);
        }
        assert_eq!(table_name("itemset"), "ItemSet");
        assert_eq!(table_name("SKILLLINEABILITY"), "SkillLineAbility");
        assert_eq!(table_name("wmoareatable"), "WMOAreaTable");
        assert_eq!(table_name("NotATable"), "NotATable");
    }

    /// The skill tables and the item sets are the width the files are, and
    /// their joins are the ones the modules that read them use: an ability
    /// names its skill line in field 1 and its spell in field 2, a set's
    /// seventeen items are fields 10 to 26 and its eight bonus spells 27 to
    /// 34, each with its piece count eight fields on.
    #[test]
    fn the_skill_and_set_tables_are_the_shape_the_files_are() {
        assert_eq!(SKILL_LINE.columns.len(), 22);
        assert_eq!(SKILL_LINE_ABILITY.columns.len(), 15);
        assert_eq!(SKILL_RACE_CLASS_INFO.columns.len(), 8);
        assert_eq!(ITEM_SET.columns.len(), 45);

        assert_eq!(SKILL_LINE.columns[3].kind, Kind::Text);
        assert_eq!(SKILL_LINE.columns[21].kind, Kind::Reference("SpellIcon"));
        assert_eq!(SKILL_LINE_ABILITY.references_to("SkillLine"), vec![1]);
        assert_eq!(SKILL_LINE_ABILITY.references_to("Spell"), vec![2, 8]);
        assert_eq!(SKILL_RACE_CLASS_INFO.references_to("SkillLine"), vec![1]);

        let items: Vec<usize> = ITEM_SET
            .columns
            .iter()
            .filter(|column| column.kind == Kind::Item)
            .map(|column| column.field)
            .collect();
        assert_eq!(items, (10..27).collect::<Vec<usize>>());
        assert_eq!(ITEM_SET.references_to("Spell"), (27..35).collect::<Vec<usize>>());
        assert_eq!(ITEM_SET.references_to("SkillLine"), vec![43]);
        let bonuses = ITEM_SET.sections.iter().find(|section| section.name == "Bonuses").unwrap();
        for pair in bonuses.fields.chunks(2) {
            assert_eq!(pair[1], pair[0] + 8, "a spell and its piece count");
        }
        // The masks name the nine races and the nine classes.
        let Kind::Flags(classes) = SKILL_LINE_ABILITY.columns[4].kind else {
            panic!("ClassMask is a mask");
        };
        assert_eq!(classes.iter().fold(0, |all, (bit, _, _)| all | bit), 1503);
        let Kind::Flags(races) = SKILL_LINE_ABILITY.columns[3].kind else {
            panic!("RaceMask is a mask");
        };
        assert_eq!(races.iter().fold(0, |all, (bit, _, _)| all | bit), 511);
    }

    /// The five light tables, at the widths the archives state (`vale dbc
    /// <Table>`).
    ///
    /// The band tables' width of 34 matters most: it is `2 + 16 + 16`, and a
    /// schema with 33 or 35 columns would pair every time with the wrong
    /// value.
    #[test]
    fn the_light_tables_are_the_width_the_files_are() {
        assert_eq!(LIGHT.columns.len(), 12);
        assert_eq!(LIGHT_PARAMS.columns.len(), 9);
        assert_eq!(LIGHT_INT_BAND.columns.len(), 34);
        assert_eq!(LIGHT_FLOAT_BAND.columns.len(), 34);
        assert_eq!(LIGHT_SKYBOX.columns.len(), 2);
    }

    /// The taxi schemas have the width of the files, and name the fields the
    /// reader in `tables::taxi` reads, so a form and the reader cannot drift
    /// apart.
    #[test]
    fn the_taxi_schemas_agree_with_the_module_that_reads_them() {
        use super::super::taxi::{node_fields as nf, path_fields as pf, path_node_fields as wf};
        assert_eq!(TAXI_NODES.columns.len(), nf::COUNT);
        assert_eq!(TAXI_PATH.columns.len(), pf::COUNT);
        assert_eq!(TAXI_PATH_NODE.columns.len(), wf::COUNT);
        let named = |schema: &Schema, field: usize| schema.columns[field].name;
        assert_eq!(named(&TAXI_NODES, nf::MAP), "Map");
        assert_eq!(named(&TAXI_NODES, nf::X), "X");
        assert_eq!(named(&TAXI_NODES, nf::Z), "Z");
        assert_eq!(named(&TAXI_NODES, nf::NAME), "Name");
        assert_eq!(named(&TAXI_NODES, nf::NAME_FLAGS), "NameFlags");
        assert_eq!(named(&TAXI_NODES, nf::MOUNT), "MountCreatureId 1");
        assert_eq!(named(&TAXI_PATH, pf::FROM), "From");
        assert_eq!(named(&TAXI_PATH, pf::TO), "To");
        assert_eq!(named(&TAXI_PATH, pf::COST), "Cost");
        assert_eq!(named(&TAXI_PATH_NODE, wf::PATH), "Path");
        assert_eq!(named(&TAXI_PATH_NODE, wf::INDEX), "NodeIndex");
        assert_eq!(named(&TAXI_PATH_NODE, wf::MAP), "Map");
        assert_eq!(named(&TAXI_PATH_NODE, wf::X), "X");
        assert_eq!(named(&TAXI_PATH_NODE, wf::ACTION_FLAG), "ActionFlag");
        assert_eq!(named(&TAXI_PATH_NODE, wf::DELAY), "Delay");
    }

    /// The light schemas' indices match the reader's. This is the rule
    /// [`the_visual_chain_agrees_with_the_module_that_walks_it`] applies to
    /// the spell chain: `tables::light` parses these files through its own
    /// constants and a form draws them through these columns, so the two must
    /// not drift apart.
    #[test]
    fn the_light_schema_agrees_with_the_module_that_reads_it() {
        use super::super::light::{light_field as lf, params_field as pf};
        let named: &[(usize, &str)] = &[
            (lf::MAP, "Map"),
            (lf::INTERNAL_X, "InternalX"),
            (lf::INTERNAL_Y, "InternalY"),
            (lf::INTERNAL_Z, "InternalZ"),
            (lf::FALLOFF_START, "FalloffStart"),
            (lf::FALLOFF_END, "FalloffEnd"),
            (lf::PARAMS_CLEAR, "ParamsClear"),
            (lf::PARAMS_CLEAR_UNDERWATER, "ParamsClearUnderwater"),
            (lf::PARAMS_STORM, "ParamsStorm"),
            (lf::PARAMS_STORM_UNDERWATER, "ParamsStormUnderwater"),
        ];
        for &(field, name) in named {
            assert_eq!(
                LIGHT.columns[field].name, name,
                "field {field} is {} here and {name} in light_field",
                LIGHT.columns[field].name
            );
        }
        let params: &[(usize, &str)] = &[
            (pf::WATER_SHALLOW_ALPHA, "WaterShallowAlpha"),
            (pf::WATER_DEEP_ALPHA, "WaterDeepAlpha"),
            (pf::OCEAN_SHALLOW_ALPHA, "OceanShallowAlpha"),
            (pf::OCEAN_DEEP_ALPHA, "OceanDeepAlpha"),
        ];
        for &(field, name) in params {
            assert_eq!(LIGHT_PARAMS.columns[field].name, name, "field {field}");
        }
    }

    /// A band row's times and values are where
    /// [`super::super::light::BAND_TIME_FIELD`] says, and each time is paired
    /// with the value 16 columns along.
    ///
    /// A form draws the section, so this checks that the pairing shown on the
    /// form is the pairing in the file.
    #[test]
    fn a_bands_keys_are_paired_time_then_value() {
        use super::super::light::{BAND_KEYS, BAND_TIME_FIELD, BAND_VALUE_FIELD};
        assert_eq!(BAND_VALUE_FIELD, BAND_TIME_FIELD + BAND_KEYS);
        for schema in [&LIGHT_INT_BAND, &LIGHT_FLOAT_BAND] {
            let keys = schema
                .sections
                .iter()
                .find(|section| section.name == "Keys")
                .expect("a band schema has a Keys section");
            assert_eq!(keys.fields.len(), BAND_KEYS * 2);
            for (n, pair) in keys.fields.chunks(2).enumerate() {
                assert_eq!(pair[0], BAND_TIME_FIELD + n, "{} time {n}", schema.table);
                assert_eq!(pair[1], BAND_VALUE_FIELD + n, "{} value {n}", schema.table);
            }
        }
        // The value columns have different types in the two tables, which is
        // why there are two band tables: the same 34 columns hold a packed
        // colour in one and an `f32` in the other.
        assert_eq!(LIGHT_INT_BAND.columns[BAND_VALUE_FIELD].kind, Kind::Colour);
        assert_eq!(LIGHT_FLOAT_BAND.columns[BAND_VALUE_FIELD].kind, Kind::Float);
        for n in 0..BAND_KEYS {
            assert_eq!(LIGHT_INT_BAND.columns[BAND_TIME_FIELD + n].kind, Kind::Int);
            assert_eq!(
                LIGHT_FLOAT_BAND.columns[BAND_TIME_FIELD + n].kind,
                Kind::Int
            );
        }
    }

    /// A band row is found by arithmetic on the params row, and the
    /// arithmetic inverts.
    ///
    /// This is the one join in the spell and light groups that is not a
    /// reference column, so there is no `Kind::Reference` to check and this
    /// test replaces that check. The row counts are those of the shipped
    /// files: 426 params against 7,668 int bands and 2,556 float bands.
    #[test]
    fn a_band_row_is_found_by_position_and_read_back() {
        use super::super::light::{float_band_of, float_band_row, int_band_of, int_band_row};
        // The first band of the first params row is row 1, not row 0.
        assert_eq!(int_band_row(1, 0), Some(1));
        assert_eq!(int_band_row(1, 17), Some(18));
        assert_eq!(int_band_row(2, 0), Some(19));
        assert_eq!(float_band_row(1, 0), Some(1));
        assert_eq!(float_band_row(2, 0), Some(7));
        // Past the end of either band set, and params row 0, which no light
        // names.
        assert_eq!(int_band_row(1, 18), None);
        assert_eq!(float_band_row(1, 6), None);
        assert_eq!(int_band_row(0, 0), None);
        // The last row of each shipped file is the last band of params 426.
        assert_eq!(int_band_row(426, 17), Some(7668));
        assert_eq!(float_band_row(426, 5), Some(2556));
        for params in 1..=426 {
            for band in 0..18 {
                let row = int_band_row(params, band).expect("inside the table");
                assert_eq!(int_band_of(row), Some((params, band)));
                assert!((1..=7668).contains(&row));
            }
            for band in 0..6 {
                let row = float_band_row(params, band).expect("inside the table");
                assert_eq!(float_band_of(row), Some((params, band)));
                assert!((1..=2556).contains(&row));
            }
        }
    }

    /// Every band has a name, and the two lists are the lengths the row
    /// arithmetic divides by.
    ///
    /// A band with no label is easy to edit by mistake, and a form pages
    /// through the bands by these lengths.
    #[test]
    fn every_band_is_named() {
        use super::super::light::{FLOAT_BAND_NAMES, INT_BAND_NAMES};
        assert_eq!(INT_BAND_NAMES.len(), 18);
        assert_eq!(FLOAT_BAND_NAMES.len(), 6);
        for name in INT_BAND_NAMES.iter().chain(FLOAT_BAND_NAMES.iter()) {
            assert!(!name.is_empty());
        }
        // The two float bands the renderer reads, checked by name rather than
        // by number; see `float_band` in `tables::light`.
        assert_eq!(FLOAT_BAND_NAMES[0], "Fog end");
        assert_eq!(FLOAT_BAND_NAMES[1], "Fog start scaler");
    }
}
