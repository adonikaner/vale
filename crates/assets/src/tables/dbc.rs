//! DBC: the client's static database tables (`DBFilesClient\*.dbc`).
//!
//! File layout:
//! ```text
//! 'WDBC'  magic
//! u32 recordCount, u32 fieldCount, u32 recordSize, u32 stringBlockSize
//! recordCount x recordSize bytes of fixed-width records (all fields u32-sized)
//! stringBlockSize bytes of \0-separated strings
//! ```
//! A "string" field holds a byte offset into that trailing block.
//!
//! Field meanings are not in the file; each table has its own convention. This
//! module gives typed access by field index and leaves naming to the caller, so
//! one reader serves every table.

use crate::look::character::{Appearance, CharGeosets};
use crate::AssetError;
use std::collections::HashMap;

const DBC_MAGIC: &[u8; 4] = b"WDBC";
const HEADER_SIZE: usize = 20;
/// Every DBC field occupies four bytes, whatever its type.
const FIELD_SIZE: usize = 4;

pub struct Dbc {
    pub record_count: usize,
    pub field_count: usize,
    record_size: usize,
    records: Vec<u8>,
    strings: Vec<u8>,
}

impl Dbc {
    pub fn parse(buf: &[u8]) -> Result<Dbc, AssetError> {
        if buf.len() < HEADER_SIZE || &buf[0..4] != DBC_MAGIC {
            return Err(AssetError::malformed("DBC", "missing WDBC magic"));
        }
        let u32_at = |o: usize| {
            u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]) as usize
        };
        let record_count = u32_at(4);
        let field_count = u32_at(8);
        let record_size = u32_at(12);
        let string_size = u32_at(16);

        let records_end = HEADER_SIZE + record_count * record_size;
        if records_end > buf.len() {
            return Err(AssetError::malformed(
                "DBC",
                format!("record block runs past EOF ({records_end} > {})", buf.len()),
            ));
        }
        let strings_end = (records_end + string_size).min(buf.len());

        Ok(Dbc {
            record_count,
            field_count,
            record_size,
            records: buf[HEADER_SIZE..records_end].to_vec(),
            strings: buf[records_end..strings_end].to_vec(),
        })
    }

    /// Raw `u32` at `(record, field)`, or `None` if either is out of range.
    pub fn u32_at(&self, record: usize, field: usize) -> Option<u32> {
        if record >= self.record_count || field >= self.field_count {
            return None;
        }
        let offset = record * self.record_size + field * FIELD_SIZE;
        self.records
            .get(offset..offset + FIELD_SIZE)
            .and_then(|s| s.try_into().ok())
            .map(u32::from_le_bytes)
    }

    pub fn f32_at(&self, record: usize, field: usize) -> Option<f32> {
        self.u32_at(record, field).map(f32::from_bits)
    }

    /// One record's raw bytes, for the one table in the game whose record is
    /// narrower than a field.
    ///
    /// `CharBaseInfo.dbc` is 41 records of two bytes each, a race and a class.
    /// [`Self::u32_at`] cannot read it: the last record starts two bytes before
    /// the end of the block, so the four-byte read runs past it and returns
    /// `None`. Reading it that way dropped `(Troll, Mage)`, the 41st row, and
    /// the character-create screen lost that class button for that race.
    pub fn record_bytes(&self, record: usize) -> Option<&[u8]> {
        if record >= self.record_count {
            return None;
        }
        let offset = record * self.record_size;
        self.records.get(offset..offset + self.record_size)
    }

    /// String at `(record, field)`, resolved through the string block.
    pub fn string_at(&self, record: usize, field: usize) -> Option<String> {
        let offset = self.u32_at(record, field)? as usize;
        let rest = self.strings.get(offset..)?;
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        Some(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
}

/// `Map.dbc`: map id -> the directory name used under `World\Maps\`.
///
/// Field 0 is the id and field 1 the directory string; the localised display
/// name sits further along and is not needed to locate terrain. Reading the
/// table instead of hardcoding "0 = Azeroth" lets custom maps resolve without
/// code changes.
pub fn map_directories(map_dbc: &[u8]) -> Result<HashMap<u32, String>, AssetError> {
    let dbc = Dbc::parse(map_dbc)?;
    let mut out = HashMap::new();
    for record in 0..dbc.record_count {
        if let (Some(id), Some(dir)) = (dbc.u32_at(record, 0), dbc.string_at(record, 1)) {
            if !dir.is_empty() {
                out.insert(id, dir);
            }
        }
    }
    Ok(out)
}

/// `Map.dbc`: map id -> the localised display name, such as "Eastern Kingdoms"
/// rather than "Azeroth".
///
/// This reads a different column from [`map_directories`]: field 1 is the
/// folder under `World\Maps\`, and field 4 is the first of eight locale columns
/// holding the player-facing name. The continent drop-down on the world map
/// shows the locale column for the client's locale, field `4 + locale`.
pub fn map_display_names(map_dbc: &[u8]) -> HashMap<u32, String> {
    const DISPLAY_NAME: usize = 4;
    let Ok(dbc) = Dbc::parse(map_dbc) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for record in 0..dbc.record_count {
        if let (Some(id), Some(name)) = (dbc.u32_at(record, 0), dbc.string_at(record, DISPLAY_NAME))
        {
            if !name.is_empty() {
                out.insert(id, name);
            }
        }
    }
    out
}

/// What kind of place a map is, out of `Map.dbc` field 2.
///
/// Measured against maps whose kind is known, not inferred from a name:
/// Azeroth and Kalimdor read 0, Deadmines 1, Ahn'Qiraj 2, Alterac Valley 3.
///
/// It is read because [`crate::tables::light`]'s checks apply only to maps
/// with a sky. The shapes the light bands hold outdoors (a warm sun over a cool
/// fill, a zenith darker than the sky beneath it) do not hold in interiors, by
/// design: Scarlet Monastery's "sun" is torchlight and its "zenith" is black,
/// because it is a corridor. Grouping by this field separates those interior
/// results from the outdoor checks, so three counts that would otherwise read
/// as failures are reported as expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapKind {
    /// A continent: the only kind with weather and a horizon.
    World,
    Dungeon,
    Raid,
    Battleground,
}

impl MapKind {
    /// Whether a map is drawn under a sky. Battlegrounds are outdoors and are
    /// counted with the world. The light data confirms this: Alterac Valley's
    /// light is a snowstorm and shares nothing with a dungeon's.
    pub fn has_sky(self) -> bool {
        matches!(self, MapKind::World | MapKind::Battleground)
    }
}

/// `Map.dbc`: map id -> what kind of place it is. See [`MapKind`].
pub fn map_kinds(map_dbc: &[u8]) -> Result<HashMap<u32, MapKind>, AssetError> {
    let dbc = Dbc::parse(map_dbc)?;
    let mut out = HashMap::new();
    for record in 0..dbc.record_count {
        let (Some(id), Some(kind)) = (dbc.u32_at(record, 0), dbc.u32_at(record, 2)) else {
            continue;
        };
        out.insert(
            id,
            match kind {
                1 => MapKind::Dungeon,
                2 => MapKind::Raid,
                3 => MapKind::Battleground,
                _ => MapKind::World,
            },
        );
    }
    Ok(out)
}

/// Virtual path of a DBC table.
pub fn dbc_path(table: &str) -> String {
    format!("DBFilesClient\\{table}.dbc")
}

/// Directory the baked NPC skins live in. `CreatureDisplayInfoExtra` names them
/// bare, as a 32-hex-digit `.blp`.
const BAKED_NPC_TEXTURES: &str = "Textures\\BakedNpcTextures\\";

/// A model an entity should be drawn with, resolved from its display id.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayModel {
    /// Archive path, extension already fixed to `.m2`.
    pub path: String,
    /// Skin textures the model does not carry itself, indexed by the slot the
    /// renderer maps each client-supplied texture type onto.
    ///
    /// Slot 0 is the body: a creature M2 declares it as texture type 11 and a
    /// character M2 as type 1. No model uses both, so the two share the slot.
    /// Slots 1 and 2 are creature texture variations 2 and 3 (types 12 and 13).
    /// The filename in slot 0 depends on the kind of model: a creature's comes
    /// from `CreatureDisplayInfo`'s texture variations, and a character-model
    /// NPC's comes from the baked skin in `CreatureDisplayInfoExtra`.
    pub skins: Vec<String>,
    /// `modelScale * displayScale`, the fallback for an entity whose
    /// `OBJECT_FIELD_SCALE_X` never arrived. The field already holds this
    /// product (`Unit::GetScaleForDisplayId`), so the two must not be
    /// multiplied together.
    pub scale: f32,
    /// Set when this display id is an NPC wearing a character model. It holds
    /// the geosets that select which hair and beard to draw.
    ///
    /// It comes from the seven appearance ids in the same
    /// `CreatureDisplayInfoExtra` row as the baked skin. The bake covers the
    /// texture part of that appearance; it does not cover the geometry, and
    /// this field is the geometry. `None` for an ordinary creature, which is
    /// drawn [`Dress::Creature`].
    ///
    /// [`Dress::Creature`]: crate::world::m2::Dress::Creature
    pub character: Option<crate::world::m2::CharacterGeosets>,
    /// The appearance behind [`DisplayModel::character`]. It is kept because
    /// helms are modelled per race: `Helm_Plate_D_04.mdx` in the row means one
    /// of sixteen files in the archive. Attaching an NPC's gear therefore needs
    /// the race and gender, not only the geosets.
    pub appearance: Option<Appearance>,
    /// The items this NPC wears, as `(ItemDisplayInfo id, Slot)`.
    ///
    /// Empty for an ordinary creature and for a player, whose gear comes off the
    /// wire instead. See [`DisplayTables::equipped_items`] for why the bake does
    /// not cover this.
    pub equipment: Vec<crate::tables::item::Equipped>,
}

impl DisplayModel {
    /// Whether a player wearing this display id needs a composed body texture
    /// instead of one read from the archive.
    ///
    /// Display ids 49..57 are the bare race models, and the game ships no body
    /// texture for them: a player's skin is built at runtime from `CharSections`
    /// layers. Every other character-model display id is an NPC whose skin was
    /// baked offline and shipped, and slot 0 names it.
    ///
    /// The test is "does this display id supply a skin", not "is this a
    /// player". A shapeshifted or disguised player wears an NPC's display id
    /// and must be drawn with that NPC's bake, not with their own composed face
    /// on a bear model.
    pub fn composes_its_skin(&self) -> bool {
        self.skins.first().is_none_or(|skin| skin.is_empty())
    }
}

/// The display tables, parsed once and indexed by id.
///
/// The model chain uses four tables. A unit's `DISPLAYID` names a
/// `CreatureDisplayInfo` row, which names a `CreatureModelData` row, which
/// holds a path. Game objects take one step: `GameObjectDisplayInfo` holds the
/// path directly. The fourth, `CreatureDisplayInfoExtra`, has rows only for
/// the NPCs that wear a character model; see [`DisplayTables::creature`].
///
/// Field indices below are measured, not taken from a wiki. `vale dbc <Table>`
/// prints each column with its candidate types, and a string field is easy to
/// identify: `CreatureModelData` field 2 on record 0 resolves to
/// `Creature\Basilisk\Basilisk.mdx`, and on record 1 to `BogBeast`.
pub struct DisplayTables {
    creature_display: Indexed,
    creature_model: Indexed,
    object_display: Indexed,
    /// Optional: a chain missing this table loses baked NPC skins and nothing
    /// else.
    creature_extra: Option<Indexed>,
    /// The two tables that say which hair and facial-hair geometry an
    /// appearance selects. They are held here, not beside `CharSections`,
    /// because both character-model NPCs and players resolve them, and the NPCs
    /// reach them through this table.
    char_geosets: CharGeosets,
    /// `ItemDisplayInfo.dbc`. Optional like the bakes: without it every player
    /// is drawn without equipment.
    items: Option<crate::tables::item::ItemDisplays>,
    /// `HelmetGeosetVisData.dbc`, which says what a helmet hides. Optional:
    /// without it a helm is drawn over the wearer's hair.
    helmet_vis: Option<crate::tables::item::HelmetVisibility>,
    /// `Emotes.dbc`, which turns the id in `SMSG_EMOTE` into an animation.
    /// Optional: without it an emote such as `/dance` plays no animation and
    /// the character stands still.
    emotes: Option<Emotes>,
    /// `EmotesText.dbc` and its two companion tables: what a text emote says
    /// and which voice line it plays. Optional: without them a `/dance` plays
    /// the animation only.
    emote_texts: Option<crate::tables::emotetext::EmoteTexts>,
    /// `AnimationData.dbc`: each animation's name, and what it implies about
    /// the weapons. Optional: without it the sheath reconcile has no policy to
    /// read, so the weapons follow only the combat draw and the server's sheath
    /// byte, and a character swims or gossips with the sword still in hand.
    animations: Option<AnimationData>,
    /// The light chain, which gives the sun, sky, fog and water colours; see
    /// [`crate::tables::light`]. Optional: without it every liquid draws white
    /// and the world uses [`crate::tables::light::Atmosphere::PLACEHOLDER`].
    light: Option<crate::tables::light::LightTables>,
    /// `Spell` + `SpellVisual` + `SpellVisualKit`, which together give a
    /// caster's animations; see [`crate::tables::spell`]. Optional: without
    /// them every spell is cast with the same two generic poses.
    spells: Option<crate::tables::spell::SpellVisuals>,
    /// `Spell` read a second time, for what the action bar and spellbook need;
    /// see [`crate::tables::spellbook`]. Optional: without it the action bar has
    /// ids but no names, costs or cast bars, and the aiming rule has nothing to
    /// read, so every cast is sent at the raw selection.
    spellbook: Option<crate::tables::spellbook::Spells>,
    /// `SpellShapeshiftForm.dbc`: which of the ten action bars is on the
    /// screen; see [`crate::tables::spellbook::ShapeshiftForms`]. An absent
    /// table answers 0 (the ordinary page) for every form, so a class that uses
    /// stances loses its stance bars entirely rather than getting a partial
    /// result.
    shapeshift: crate::tables::spellbook::ShapeshiftForms,
    /// `FactionTemplate.dbc`, which defines whether units are friendly; see
    /// [`crate::tables::faction`]. Optional: without it every unit reads
    /// neutral, which errs towards attackable, so Tab-targeting selects
    /// innkeepers and the server refuses the attack.
    factions: Option<crate::tables::faction::Factions>,
    /// `Faction.dbc`; see [`crate::tables::reputation`]. The field is named for
    /// the reputation panel, not for the file, because [`Self::factions`]
    /// already holds `FactionTemplate.dbc` and the two names are easy to
    /// confuse.
    reputation: Option<crate::tables::reputation::Factions>,
    /// `TaxiNodes` + `TaxiPath` + `TaxiPathNode`, joined against
    /// `WorldMapContinent`'s taxi box. These hold the whole flight map; none of
    /// it is sent by the server. See [`crate::tables::taxi`]. Optional as a set:
    /// without them `NumTaxiNodes()` is 0 and a flight master's window opens
    /// empty.
    taxi: Option<crate::tables::taxi::TaxiTables>,
    /// `SkillLineAbility` + `SkillLine` + `SkillRaceClassInfo`, which together
    /// say which spellbook page a spell goes on and which pages exist; see
    /// [`crate::tables::skills`]. Optional: without the first two every spell
    /// falls to the General tab, so the book is one list in name order instead
    /// of the game's four pages. Without the third, every weapon skill and
    /// profession gets its own page.
    skills: Option<crate::tables::skills::Skills>,
    /// A second reading of `SkillLineAbility.dbc`: the recipe skill thresholds
    /// the two profession windows use for colour. See
    /// [`crate::tables::tradeskill`].
    tradeskills: Option<crate::tables::tradeskill::TradeSkills>,
    /// `SpellFocusObject.dbc`: id to name ("Anvil", "Forge"), for the Requires
    /// line under a recipe. Field 0 is the id and field 1 the enUS name,
    /// measured: row 2 reads "Anvil", row 3 "Forge".
    spell_focus: std::collections::HashMap<u32, String>,
    /// `Talent.dbc` and `TalentTab.dbc`: the three trees a class may spend
    /// points in. See [`crate::tables::talent`]. Both tables or neither, a rule
    /// that module owns: a tab list with no talents draws parchment and no
    /// buttons. When absent, the talent panel answers
    /// `GetNumTalentTabs() == 0` and the talent UI's Lua takes its "classes
    /// without talents" branch.
    talents: Option<crate::tables::talent::Talents>,
    /// `AreaTable.dbc`: the game's names for where the character is. See
    /// [`crate::tables::area`]. Optional: without it every zone name in the
    /// interface is empty, including the minimap's title bar.
    areas: Option<crate::tables::area::Areas>,
    /// The six chat channels and which of them apply in a given area; see
    /// [`crate::tables::channels`].
    chat_channels: Option<crate::tables::channels::ChatChannels>,
    /// `QuestSort.dbc`: the second table a quest log heading can come from,
    /// chosen by the sign of `ZoneOrSort`. See [`crate::tables::questsort`].
    /// Not wrapped in `Option`: an absent file is an empty map, which leaves
    /// the few profession and "Epic" headings blank and every zone heading
    /// unchanged.
    quest_sorts: crate::tables::questsort::QuestSorts,
    /// `Stationery` + `Package` + `MailTemplate`: the paper a letter is written
    /// on, and which of the five the send panel offers. See
    /// [`crate::tables::stationery`], which owns the rule that reduces five rows
    /// to the one choice a new character has. Not wrapped in `Option`: absent
    /// files are empty tables. The cost is a send panel whose Send button never
    /// enables, because the client refuses to build a letter with no
    /// stationery.
    mail: crate::tables::stationery::MailTables,
    /// `Resistances.dbc`: the name of each damage school, which is the last
    /// word of every combat log line that names one. See
    /// [`crate::tables::resistances`]. Not wrapped in `Option`: an absent file
    /// is an empty table, and the `…SCHOOL…` combat log strings fall back to
    /// the short key instead of printing a sentence with a gap in it.
    resistances: crate::tables::resistances::Resistances,
    /// The five pet tables: `PetPersonality`, `CreatureFamily`, `ItemPetFood`,
    /// `PetLoyalty` and `StableSlotPrices`. See [`crate::tables::pet`]. Not
    /// wrapped in `Option`: each of the five tolerates absence on its own, and
    /// the degradations are listed there: a happiness icon that never shows, a
    /// pet with no family name, an empty diet line, a blank loyalty label, and
    /// a stable slot priced at zero.
    pet: crate::tables::pet::PetTables,
    /// `BankBagSlotPrices.dbc`: the price the bank window's purchase button
    /// shows. See [`crate::tables::bank`]. Tolerates absence like the pet
    /// tables: a slot priced at zero.
    bank: crate::tables::bank::BankPrices,
    /// `WMOAreaTable.dbc`: the area name when the character is inside a WMO
    /// whose name overrides the terrain's. See [`crate::tables::wmoarea`].
    /// Optional: without it a city takes the zone of the terrain under it and
    /// no building has its own name, so Ironforge reads "Dun Morogh".
    wmo_areas: Option<crate::tables::wmoarea::WmoAreas>,
    /// `WorldMapArea` + `WorldMapContinent`; see [`crate::tables::worldmap`].
    /// Optional: without them the world map opens on the cosmic parchment, with
    /// no zone under the pointer and no player arrow.
    world_map: Option<crate::tables::worldmap::WorldMap>,
    /// `AreaPOI.dbc`: the landmark icons drawn on the world map. Optional:
    /// without it `GetNumMapLandmarks` answers 0 and the map shows no landmarks
    /// except those a guard's directions add. See [`crate::tables::areapoi`].
    area_pois: Option<crate::tables::areapoi::AreaPois>,
    /// `Map.dbc`'s localised names, which the continent drop-down shows. This
    /// is a different column from the directory names [`map_directories`]
    /// reads.
    map_names: HashMap<u32, String>,
    /// The kind of each map: `Map.dbc` field 2, see [`MapKind`].
    ///
    /// Loaded beside the names because it comes from the same table in the same
    /// read. Its one consumer so far is [`Self::map_kind`].
    map_kinds: HashMap<u32, MapKind>,
    /// `ChrRaces.dbc`: the display id for each race and gender, used before
    /// the player has entered the world.
    ///
    /// Everywhere else a unit's model arrives as `UNIT_FIELD_DISPLAYID` from
    /// the server, so this table is not needed. Character select is the one
    /// screen with no display id from the server, because no character has
    /// been logged in yet. See [`DisplayTables::race_display`]. Optional:
    /// without it the character-select plinth is empty.
    races: Option<Indexed>,
    /// `PaperDollItemFrame` + `StringLookups` + `ItemClass` + `ItemSubClass`:
    /// the item data that comes from files instead of from the server. See
    /// [`crate::tables::inventory`]. Never `None` as a whole: each of the four
    /// degrades on its own, and the icon directory has a stated fallback.
    item_tables: crate::tables::inventory::ItemTables,
    /// `DurabilityCosts` + `DurabilityQuality`: repair costs, the one price in
    /// the game the client computes itself. See [`crate::tables::repair`].
    repair: crate::tables::repair::RepairCosts,
    /// `TransportAnimation`: the position of an elevator, lift or the tram in
    /// its cycle. Optional: without it every platform stands at its spawn
    /// point. See [`crate::tables::transport`].
    transports: crate::tables::transport::Transports,
    /// `ItemGroupSounds`: the sounds an item makes when picked up or put down.
    /// Optional like every table other than the three required ones: without
    /// it items are silent. See [`crate::tables::itemsound`].
    item_sounds: crate::tables::itemsound::ItemGroupSounds,
    /// `PageTextMaterial`: the material a sign or book page is drawn on, given
    /// as a name, not a colour. See [`crate::tables::pagetext`].
    page_materials: crate::tables::pagetext::PageMaterials,
    /// `Lock` + `LockType`: the requirements to open a door, a chest or an ore
    /// vein, and therefore which cursor shows over one. See
    /// [`crate::tables::lock`] and [`crate::look::object`].
    locks: crate::tables::lock::Locks,
    /// `SpellItemEnchantment.dbc`: what an enchantment on an item is called.
    /// See [`crate::tables::enchant`]. An absent file is an empty table, and
    /// an item plate then draws no enchantment lines.
    enchantments: crate::tables::enchant::Enchantments,
    /// `ItemRandomProperties.dbc`: a random suffix's name and enchantments.
    /// An absent file is an empty table, and an item keeps its plain name.
    random_properties: crate::tables::enchant::RandomProperties,
    /// `ItemSet.dbc`: the pieces and bonuses of a set. See
    /// [`crate::tables::itemset`]. An absent file is an empty table, and an
    /// item plate then draws no set block.
    item_sets: crate::tables::itemset::ItemSets,
    /// `ItemVisuals.dbc` and `ItemVisualEffects.dbc`: the models a held item's
    /// visual hangs on it. See [`crate::tables::itemvisual`]. An absent file
    /// is an empty table, and no held item carries a glow.
    item_visuals: crate::tables::itemvisual::ItemVisuals,
}

mod display_fields {
    /// `CreatureDisplayInfo`: id, modelId, soundId, extendedDisplayInfoId,
    /// scale, alpha, textureVariation[3], portrait, blood, npcSound.
    pub const MODEL_ID: usize = 1;
    pub const EXTENDED_DISPLAY_INFO_ID: usize = 3;
    pub const SCALE: usize = 4;
    pub const TEXTURE_VARIATION: usize = 6;
    pub const TEXTURE_VARIATION_COUNT: usize = 3;

    /// `CreatureModelData`: id, flags, modelName, sizeClass, modelScale, ...
    pub const MODEL_NAME: usize = 2;
    pub const MODEL_SCALE: usize = 4;

    /// `GameObjectDisplayInfo`: id, modelName, then sounds and a bounding box.
    pub const OBJECT_MODEL_NAME: usize = 1;

    /// `CreatureDisplayInfoExtra`: id, race, gender, skin, face, hairStyle,
    /// hairColour, facialHair, then ten equipped-item display ids, then the
    /// baked texture name. That is 19 fields, and the last is the only string.
    ///
    /// The seven appearance ids and the ten item ids are the inputs the bake
    /// was computed from. The game ships the baked texture, so the texture
    /// needs only the name. Race in field 1 and gender in field 2 cross-check
    /// the layout: record 0 reads race 3 gender 0 (dwarf male), and its model
    /// is `Character\Dwarf\Male\DwarfMale.mdx`.
    pub const BAKE_NAME: usize = 18;
    /// The seven appearance ids, fields 1..7. The hairstyle and beard are
    /// geometry and cannot be baked into a texture, so these ids are the only
    /// record of which head geosets the NPC uses.
    pub const APPEARANCE: usize = 1;
    pub const APPEARANCE_COUNT: usize = 7;

    /// The ten `ItemDisplayInfo` ids, at 8..17.
    ///
    /// These have the same split as the appearance ids. The bake covers the
    /// texture part of each garment but not the geometry: a pauldron is a
    /// separate model and a bootleg is a geoset, and neither can be painted
    /// into a 256x256 body atlas. A client that reads only the bake draws an
    /// NPC with its garment textures and none of its garment geometry; a
    /// Gadgetzan Bruiser then has a correctly coloured leather vest and
    /// trousers and no shoulders.
    pub const EQUIPPED_ITEM: usize = 8;
    pub const EQUIPPED_ITEM_COUNT: usize = 10;
}

impl DisplayTables {
    /// Reads every table this needs from `read`, which takes a bare table name
    /// (`"CreatureDisplayInfo"`, not a path) and returns `None` for a table the
    /// chain does not have.
    ///
    /// It takes a reader instead of a list of byte slices because the list of
    /// tables keeps growing. With positional arguments, each new table changed
    /// the signature and every caller, and the arguments were values of one
    /// type in a fixed order, which is the pattern that caused the
    /// `CharSections` and `geosetGroup` field-index bugs one level down. Adding
    /// a table is now one line here, and no caller can transpose two tables.
    ///
    /// Three tables are required, because without them no entity resolves to a
    /// model. Every other table is optional, and its absence is a documented
    /// degradation, not an error: without `CreatureDisplayInfoExtra`
    /// character-model NPCs are grey, without `CharHairGeosets` they are bald,
    /// without `ItemDisplayInfo` players have no equipment, and without
    /// `HelmetGeosetVisData` hair is drawn through a helm.
    pub fn load(
        mut read: impl FnMut(&str) -> Option<Vec<u8>>,
    ) -> Result<DisplayTables, AssetError> {
        let creature_display = required(&mut read, "CreatureDisplayInfo")?;
        let creature_model = required(&mut read, "CreatureModelData")?;
        let object_display = required(&mut read, "GameObjectDisplayInfo")?;
        // Read before the struct is built, because the world map needs it: the
        // zone list on a continent is sorted by the `AreaTable` name, so
        // `WorldMap::parse` takes the areas instead of looking them up later.
        let areas = crate::tables::area::Areas::parse(&read("AreaTable").unwrap_or_default());
        let world_map = crate::tables::worldmap::WorldMap::parse(
            &read("WorldMapArea").unwrap_or_default(),
            &read("WorldMapContinent").unwrap_or_default(),
            &read("WorldMapOverlay").unwrap_or_default(),
            areas.as_ref(),
        );
        let map_raw = read("Map").unwrap_or_default();
        let map_names = map_display_names(&map_raw);
        let map_kinds = map_kinds(&map_raw).unwrap_or_default();
        let chat_channels = read("ChatChannels")
            .and_then(|raw| crate::tables::channels::ChatChannels::parse(&raw).ok());
        Ok(DisplayTables {
            areas,
            chat_channels,
            quest_sorts: crate::tables::questsort::QuestSorts::parse(
                &read("QuestSort").unwrap_or_default(),
            ),
            mail: crate::tables::stationery::MailTables::parse(
                &read("Stationery").unwrap_or_default(),
                &read("Package").unwrap_or_default(),
                &read("MailTemplate").unwrap_or_default(),
            ),
            resistances: crate::tables::resistances::Resistances::parse(
                &read("Resistances").unwrap_or_default(),
            ),
            pet: crate::tables::pet::PetTables::parse(
                &read("PetPersonality").unwrap_or_default(),
                &read("CreatureFamily").unwrap_or_default(),
                &read("ItemPetFood").unwrap_or_default(),
                &read("PetLoyalty").unwrap_or_default(),
                &read("StableSlotPrices").unwrap_or_default(),
            ),
            bank: crate::tables::bank::BankPrices::parse(
                &read("BankBagSlotPrices").unwrap_or_default(),
            ),
            wmo_areas: crate::tables::wmoarea::WmoAreas::parse(
                &read("WMOAreaTable").unwrap_or_default(),
            ),
            page_materials: crate::tables::pagetext::PageMaterials::parse(
                &read("PageTextMaterial").unwrap_or_default(),
            ),
            world_map,
            area_pois: crate::tables::areapoi::AreaPois::parse(
                &read("AreaPOI").unwrap_or_default(),
            )
            .ok(),
            map_names,
            map_kinds,
            creature_display,
            creature_model,
            object_display,
            creature_extra: read("CreatureDisplayInfoExtra")
                .and_then(|raw| Indexed::parse(&raw).ok()),
            char_geosets: CharGeosets::parse(
                &read("CharHairGeosets").unwrap_or_default(),
                &read("CharacterFacialHairStyles").unwrap_or_default(),
            ),
            items: read("ItemDisplayInfo")
                .and_then(|raw| crate::tables::item::ItemDisplays::parse(&raw).ok()),
            helmet_vis: read("HelmetGeosetVisData")
                .and_then(|raw| crate::tables::item::HelmetVisibility::parse(&raw).ok()),
            emotes: read("Emotes").and_then(|raw| Emotes::parse(&raw).ok()),
            emote_texts: read("EmotesText").zip(read("EmotesTextData")).and_then(|(text, data)| {
                crate::tables::emotetext::EmoteTexts::parse(&text, &data, read("EmotesTextSound").as_deref()).ok()
            }),
            animations: read("AnimationData").and_then(|raw| AnimationData::parse(&raw).ok()),
            // The first three tables or none: the row arithmetic that turns a
            // `LightParams` id into a band row can only be checked with both
            // tables present. The fourth degrades on its own; see
            // `LightTables::parse`.
            light: crate::tables::light::LightTables::parse(
                &read("Light").unwrap_or_default(),
                &read("LightParams").unwrap_or_default(),
                &read("LightIntBand").unwrap_or_default(),
                &read("LightFloatBand").unwrap_or_default(),
            ),
            // The first three tables or none: a chain with `SpellVisual` but no
            // kits resolves every spell to nothing, and that result cannot be
            // told apart from correctly read data with no spell animations.
            spells: crate::tables::spell::SpellVisuals::parse(
                &read("Spell").unwrap_or_default(),
                &read("SpellVisual").unwrap_or_default(),
                &read("SpellVisualKit").unwrap_or_default(),
                // The fourth is optional: without it a cast plays its pose and
                // shows no models, which is a degradation, not a wrong answer.
                &read("SpellVisualEffectName").unwrap_or_default(),
                // The fifth is also optional. It is the only table in the chain
                // that describes a shape instead of naming a model: without it
                // the 192 spells that draw a bolt between units draw nothing.
                &read("SpellChainEffects").unwrap_or_default(),
            ),
            // `Spell.dbc` is required by this table and the three it points
            // into are not; see `Spells::parse` for what each absence costs.
            spellbook: crate::tables::spellbook::Spells::parse(
                &read("Spell").unwrap_or_default(),
                &read("SpellCastTimes").unwrap_or_default(),
                &read("SpellRange").unwrap_or_default(),
                &read("SpellIcon").unwrap_or_default(),
                // The two tables the spell description points into. Each
                // degrades on its own: without them `$d` reads as no duration
                // and `$a` as zero yards, so the description loses a number and
                // the spell stays in the panel.
                &read("SpellDuration").unwrap_or_default(),
                &read("SpellRadius").unwrap_or_default(),
                // The table the buff bar points into. Without it every debuff
                // border draws the "none" red, which the interface already uses
                // for an undispellable debuff, so the absence loses the four
                // coloured borders and nothing else.
                &read("SpellDispelType").unwrap_or_default(),
            )
            .ok(),
            // The table that says which action bar page the buttons are on.
            // For a warrior or a druid, it decides between a full action bar
            // and an empty one. See `ShapeshiftForms`.
            shapeshift: crate::tables::spellbook::ShapeshiftForms::parse(
                &read("SpellShapeshiftForm").unwrap_or_default(),
            ),
            // `FactionTemplate.dbc` with `FactionGroup.dbc` attached, which
            // turns the group mask into the name `UnitFactionGroup` returns.
            // `FactionGroup.dbc` is optional on its own; see
            // `Factions::with_groups`.
            factions: read("FactionTemplate")
                .and_then(|raw| crate::tables::faction::Factions::parse(&raw).ok())
                .map(|factions| factions.with_groups(&read("FactionGroup").unwrap_or_default())),
            // `Faction.dbc`, the second faction table: a player's own standing,
            // not a creature's reaction. Optional on its own: without it the
            // reputation panel is empty. See `tables::reputation`.
            reputation: read("Faction")
                .and_then(|raw| crate::tables::reputation::Factions::parse(&raw).ok()),
            // All four tables or none; `TaxiTables::parse` gives the reason: a
            // partial chain draws a window that looks correct but has wrong
            // routes in it.
            taxi: crate::tables::taxi::TaxiTables::parse(
                &read("TaxiNodes").unwrap_or_default(),
                &read("TaxiPath").unwrap_or_default(),
                &read("TaxiPathNode").unwrap_or_default(),
                &read("WorldMapContinent").unwrap_or_default(),
            ),
            // The first two tables both or neither; `Skills::parse` gives the
            // reason. `SpellIcon` is shared with the spellbook above and
            // degrades on its own (a tab with no icon). `SkillRaceClassInfo`
            // also degrades on its own: without it the book shows a tab for
            // every weapon skill and profession.
            skills: crate::tables::skills::Skills::parse(
                &read("SkillLineAbility").unwrap_or_default(),
                &read("SkillLine").unwrap_or_default(),
                &read("SpellIcon").unwrap_or_default(),
                &read("SkillRaceClassInfo").unwrap_or_default(),
            )
            // The fifth table, `SkillLineCategory`, is needed only by the skills
            // panel: it gives the eight headings and the column they are
            // ordered by. Optional on its own; see `Skills::with_categories`.
            .map(|skills| skills.with_categories(&read("SkillLineCategory").unwrap_or_default())),
            // `SkillLineAbility.dbc` again, for the columns the spellbook's
            // reading drops: the profession windows' skill thresholds.
            tradeskills: crate::tables::tradeskill::TradeSkills::parse(
                &read("SkillLineAbility").unwrap_or_default(),
            ),
            spell_focus: Dbc::parse(&read("SpellFocusObject").unwrap_or_default())
                .map(|table| {
                    (0..table.record_count)
                        .filter_map(|record| {
                            Some((
                                table.u32_at(record, 0)?,
                                table.string_at(record, 1)?,
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            // The talent panel's two tables. Both or neither; see the field
            // and `Talents::parse`, which gives the reason.
            talents: crate::tables::talent::Talents::parse(
                &read("Talent").unwrap_or_default(),
                &read("TalentTab").unwrap_or_default(),
            ),
            races: read("ChrRaces").and_then(|raw| Indexed::parse(&raw).ok()),
            // Four tables, each degrading independently; see
            // `ItemTables::parse`. They are read together because all four
            // describe items and no consumer wants only three of them.
            item_tables: crate::tables::inventory::ItemTables::parse(
                &read("PaperDollItemFrame").unwrap_or_default(),
                &read("StringLookups").unwrap_or_default(),
                &read("ItemClass").unwrap_or_default(),
                &read("ItemSubClass").unwrap_or_default(),
            ),
            // `repair` (below): both tables or neither. Without them
            // `GetRepairAllCost` returns nothing and the armourer's tooltip
            // loses its money line. The repair itself still works, because the
            // server computes the cost again from the same two files.
            // `locks`: both tables or neither; without them the cursor over a
            // locked object is wrong, and no panel is affected. See
            // `Locks::parse`. `LockType` carries only the names, so losing it
            // alone removes the tooltip's "Requires Mining" line and keeps the
            // pick cursor.
            locks: crate::tables::lock::Locks::parse(
                &read("Lock").unwrap_or_default(),
                &read("LockType").unwrap_or_default(),
            ),
            item_sounds: crate::tables::itemsound::ItemGroupSounds::parse(
                &read("ItemGroupSounds").unwrap_or_default(),
            ),
            repair: crate::tables::repair::RepairCosts::parse(
                &read("DurabilityCosts").unwrap_or_default(),
                &read("DurabilityQuality").unwrap_or_default(),
            ),
            // The only table in the chain about objects that move. Optional:
            // without it every elevator is drawn and stood on at its spawn
            // position.
            transports: crate::tables::transport::Transports::parse(
                &read("TransportAnimation").unwrap_or_default(),
            ),
            enchantments: crate::tables::enchant::Enchantments::parse(
                &read("SpellItemEnchantment").unwrap_or_default(),
            ),
            random_properties: crate::tables::enchant::RandomProperties::parse(
                &read("ItemRandomProperties").unwrap_or_default(),
            ),
            item_sets: crate::tables::itemset::ItemSets::parse(
                &read("ItemSet").unwrap_or_default(),
            ),
            item_visuals: crate::tables::itemvisual::ItemVisuals::parse(
                &read("ItemVisuals").unwrap_or_default(),
                &read("ItemVisualEffects").unwrap_or_default(),
            ),
        })
    }

    /// `SpellItemEnchantment.dbc`: see [`crate::tables::enchant`].
    pub fn enchantments(&self) -> &crate::tables::enchant::Enchantments {
        &self.enchantments
    }

    /// `ItemRandomProperties.dbc`: see [`crate::tables::enchant`].
    pub fn random_properties(&self) -> &crate::tables::enchant::RandomProperties {
        &self.random_properties
    }

    /// `ItemSet.dbc`: see [`crate::tables::itemset`].
    pub fn item_sets(&self) -> &crate::tables::itemset::ItemSets {
        &self.item_sets
    }

    /// `ItemVisuals.dbc` and `ItemVisualEffects.dbc`: see
    /// [`crate::tables::itemvisual`].
    pub fn item_visuals(&self) -> &crate::tables::itemvisual::ItemVisuals {
        &self.item_visuals
    }

    /// `PageTextMaterial.dbc`; see [`crate::tables::pagetext`]. Empty without
    /// the file, and every page is then drawn on parchment.
    pub fn page_materials(&self) -> &crate::tables::pagetext::PageMaterials {
        &self.page_materials
    }

    /// The requirements to open a lockable object; see
    /// [`crate::tables::lock`].
    ///
    /// Not an `Option`, because an absent file is an empty table with a stated
    /// degradation: every game object reads as unlocked, so an ore vein shows
    /// the plain interact cursor and nothing worse.
    pub fn locks(&self) -> &crate::tables::lock::Locks {
        &self.locks
    }

    /// The four tables an item's appearance in the interface comes from; see
    /// [`crate::tables::inventory`].
    pub fn item_tables(&self) -> &crate::tables::inventory::ItemTables {
        &self.item_tables
    }

    /// The cost of repairing an item; see [`crate::tables::repair`].
    pub fn repair(&self) -> &crate::tables::repair::RepairCosts {
        &self.repair
    }

    /// The position of a moving platform in its cycle; see
    /// [`crate::tables::transport`].
    ///
    /// Not an `Option`, for the same reason as [`Self::locks`]: an absent file
    /// is an empty table, every game object answers "does not animate", and
    /// every elevator stands still. That is a stated degradation.
    pub fn transports(&self) -> &crate::tables::transport::Transports {
        &self.transports
    }

    /// A bare icon name as an archive path, through `StringLookups` row 3; see
    /// [`crate::tables::inventory::InventoryTables::icon_path`]. Public for the
    /// one icon that does not come from `ItemDisplayInfo`:
    /// [`crate::tables::inventory::coin_icon`]'s.
    pub fn icon_path(&self, icon_name: &str) -> Option<String> {
        self.item_tables.icon_path(icon_name)
    }

    /// An item's inventory icon, resolved from a display id to a path.
    ///
    /// This joins the two tables that each hold half of the path:
    /// `ItemDisplayInfo.dbc` gives the bare name and `StringLookups.dbc` row 3
    /// gives the folder. It is a single entry point so that callers do not need
    /// to know the folder is a table lookup. A literal `Interface\Icons\` gives
    /// the same result with the shipped data, but the client reads the folder
    /// from the table.
    pub fn item_icon(&self, display_id: u32) -> Option<String> {
        let name = self.items.as_ref()?.inventory_icon(display_id)?;
        self.item_tables.icon_path(&name)
    }

    pub fn item_sounds(&self) -> &crate::tables::itemsound::ItemGroupSounds {
        &self.item_sounds
    }

    /// The `SoundEntries` id an item plays when it is picked up or put down,
    /// resolved from a display id. This is the same join the client makes. It
    /// is a single entry point for the same reason as [`Self::item_icon`]:
    /// otherwise the caller would need to know that `ItemDisplayInfo` field 11
    /// is a row id in a second table, not a sound.
    ///
    /// `None` for a display id with no row, a row whose group is 0, or a group
    /// whose column is empty. All three are ordinary and mean silence.
    pub fn item_sound(
        &self,
        display_id: u32,
        which: crate::tables::itemsound::ItemSound,
    ) -> Option<u32> {
        let group = self.items.as_ref()?.group_sound(display_id)?;
        self.item_sounds.sound(group, which)
    }

    /// The `CreatureDisplayInfo` id for a race and gender. The character-select
    /// screen needs this lookup; the world never does.
    ///
    /// `gender` is the byte from the server: 0 male, 1 female. Any other value
    /// is read as male, which is what an unset byte means.
    ///
    /// Fields 4 and 5 are measured, and the data confirms them: record 0 is
    /// race 1 and reads 49 and 50, and `vale npc 49` resolves
    /// `Character\Human\Male\HumanMale.m2`; record 3 is race 4 and reads 55 and
    /// 56, the two night-elf models. Only the correct pair of columns matches
    /// both. The neighbouring columns do not hold display ids (field 3 is a
    /// sound id in the four thousands and field 7 is a float), so a wrong index
    /// resolves to no model instead of the wrong one.
    pub fn race_display(&self, race: u8, gender: u8) -> Option<u32> {
        const MALE_DISPLAY_ID: usize = 4;
        const FEMALE_DISPLAY_ID: usize = 5;
        let races = self.races.as_ref()?;
        let row = races.row(u32::from(race))?;
        let field = if gender == 1 {
            FEMALE_DISPLAY_ID
        } else {
            MALE_DISPLAY_ID
        };
        races.dbc.u32_at(row, field).filter(|id| *id != 0)
    }

    /// `CreatureDisplayInfo` field 11, `npcSound`: the `NPCSounds.dbc` row
    /// holding a display's greeting and farewell sounds. 0 for the many
    /// displays with no sounds.
    /// See `crate::tables::sound::SoundBank::npc_sounds`, which resolves the
    /// row into its four columns.
    pub fn npc_sound_id(&self, display_id: u32) -> Option<u32> {
        const NPC_SOUND: usize = 11;
        let display = self.creature_display.row(display_id)?;
        self.creature_display
            .dbc
            .u32_at(display, NPC_SOUND)
            .filter(|id| *id != 0)
    }

    /// `ChrRaces` field 12, `ResSicknessSpellID`: the spell the spirit
    /// healer's warning is worded from. The 1.12.1 client's
    /// `GetResSicknessDuration` takes this field from the player's race row,
    /// then follows that spell's `DurationIndex` into `SpellDuration.dbc`. The
    /// value is 15007 on all nine rows in 5875; it is read, not assumed, for
    /// the same reason as the display ids above. See
    /// [`crate::tables::spellbook::Spells::duration_at`], which completes the
    /// calculation.
    pub fn res_sickness_spell(&self, race: u8) -> Option<u32> {
        const RES_SICKNESS_SPELL_ID: usize = 12;
        let races = self.races.as_ref()?;
        let row = races.row(u32::from(race))?;
        races.dbc.u32_at(row, RES_SICKNESS_SPELL_ID).filter(|id| *id != 0)
    }

    /// The three talent trees, or `None` if either file is missing; see
    /// [`crate::tables::talent`]. `None` gives an empty talent panel and
    /// nothing worse: the talent UI addon handles a class with no talents.
    pub fn talents(&self) -> Option<&crate::tables::talent::Talents> {
        self.talents.as_ref()
    }

    /// The recipe skill thresholds, or `None` without `SkillLineAbility.dbc`.
    /// `None` draws both profession windows empty, the same degradation an
    /// absent `SkillLine.dbc` gives the skills panel.
    pub fn tradeskills(&self) -> Option<&crate::tables::tradeskill::TradeSkills> {
        self.tradeskills.as_ref()
    }

    /// A spell focus object's name, such as "Anvil" or "Forge". `None` omits
    /// it from the Requires line instead of printing an id.
    pub fn spell_focus_name(&self, id: u32) -> Option<String> {
        self.spell_focus.get(&id).cloned()
    }

    /// Which spellbook page a spell goes on; see [`crate::tables::skills`].
    /// `None` puts every spell in General, which changes the layout but gives
    /// no wrong answer.
    pub fn skills(&self) -> Option<&crate::tables::skills::Skills> {
        self.skills.as_ref()
    }

    /// Area data for where the character is standing, by area id; see
    /// [`crate::tables::area`].
    pub fn areas(&self) -> Option<&crate::tables::area::Areas> {
        self.areas.as_ref()
    }

    /// The six chat channels; see [`crate::tables::channels`]. Optional like
    /// every table except the three that resolve a model: without it no zone
    /// channel is joined, and `/join` still works for custom channels.
    pub fn chat_channels(&self) -> Option<&crate::tables::channels::ChatChannels> {
        self.chat_channels.as_ref()
    }

    /// `QuestSort.dbc`, the second source of quest log headings besides
    /// `AreaTable`; see [`crate::tables::questsort`].
    pub fn quest_sorts(&self) -> &crate::tables::questsort::QuestSorts {
        &self.quest_sorts
    }

    /// The three tables the mail window reads; see
    /// [`crate::tables::stationery`].
    pub fn mail(&self) -> &crate::tables::stationery::MailTables {
        &self.mail
    }

    /// The name of each damage school; see [`crate::tables::resistances`].
    pub fn resistances(&self) -> &crate::tables::resistances::Resistances {
        &self.resistances
    }

    /// The pet tables: happiness bands, family and diet; see
    /// [`crate::tables::pet`].
    pub fn pet(&self) -> &crate::tables::pet::PetTables {
        &self.pet
    }

    /// The price of each bank bag slot; see [`crate::tables::bank`].
    pub fn bank(&self) -> &crate::tables::bank::BankPrices {
        &self.bank
    }

    /// Area names for the inside of WMO buildings, which override the zone of
    /// the terrain; see [`crate::tables::wmoarea`].
    pub fn wmo_areas(&self) -> Option<&crate::tables::wmoarea::WmoAreas> {
        self.wmo_areas.as_ref()
    }

    /// Loads the world map's one input that is not a table; see
    /// [`crate::tables::worldmap::WorldMap::load_zone_grids`], which reads
    /// `Interface\WorldMap\<continent>.zmp`.
    ///
    /// This is a separate step, not a second argument to [`Self::load`],
    /// because that reader takes a table name and this is an archive path, and
    /// because `Assets::read` takes `&mut self`, so a caller cannot create two
    /// closures over it. Skipping it is a documented degradation: hover and
    /// click on the world map fall back to the rectangle rule.
    pub fn load_zone_grids(&mut self, read: impl FnMut(&str) -> Option<Vec<u8>>) {
        let DisplayTables {
            world_map, areas, ..
        } = self;
        if let Some(map) = world_map.as_mut() {
            map.load_zone_grids(read, areas.as_ref());
        }
    }

    /// `AreaPOI.dbc`, for the landmark layer; see [`crate::tables::areapoi`].
    pub fn area_pois(&self) -> Option<&crate::tables::areapoi::AreaPois> {
        self.area_pois.as_ref()
    }

    /// The world map's two tables; see [`crate::tables::worldmap`].
    pub fn world_map(&self) -> Option<&crate::tables::worldmap::WorldMap> {
        self.world_map.as_ref()
    }

    /// A continent's name: `Map.dbc`'s localised name, which is what
    /// `GetMapContinents` returns. Empty for a map the table does not carry,
    /// which draws an unnamed drop-down entry instead of dropping the
    /// continent.
    pub fn map_name(&self, map: u32) -> &str {
        self.map_names.get(&map).map_or("", String::as_str)
    }

    /// The kind of a map; see [`MapKind`]. `None` for a map the table does not
    /// carry. A caller should read that as "not an instance", not as
    /// "unknown": the table is the only source, and a map missing from it is
    /// one this client cannot be standing in.
    pub fn map_kind(&self, map: u32) -> Option<MapKind> {
        self.map_kinds.get(&map).copied()
    }

    /// Every map id in `Map.dbc`, for a census that iterates over them.
    ///
    /// One caller, `vale zones`, which crosses them with
    /// [`crate::tables::area::Areas::zone_of_map`]. Unordered; the caller
    /// sorts.
    pub fn map_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.map_names.keys().copied()
    }

    /// Spell data for buttons and the spellbook; see
    /// [`crate::tables::spellbook`]. `None` when the chain has no `Spell.dbc`,
    /// which loses every name, cost and cast time on the bar.
    pub fn spellbook(&self) -> Option<&crate::tables::spellbook::Spells> {
        self.spellbook.as_ref()
    }

    /// Which action bar a form puts on the screen; see
    /// [`crate::tables::spellbook::ShapeshiftForms`]. Never `None`: an absent
    /// table answers "the ordinary page" for every form.
    pub fn shapeshift(&self) -> &crate::tables::spellbook::ShapeshiftForms {
        &self.shapeshift
    }

    /// Faction templates, which decide friend or foe; see
    /// [`crate::tables::faction`]. `None` reads as neutral everywhere, which
    /// errs towards attackable.
    pub fn factions(&self) -> Option<&crate::tables::faction::Factions> {
        self.factions.as_ref()
    }

    /// `Faction.dbc`, for the reputation panel. `None` without the table, which
    /// draws an empty panel instead of a wrong one.
    pub fn reputation(&self) -> Option<&crate::tables::reputation::Factions> {
        self.reputation.as_ref()
    }

    /// The flight map's tables; see [`crate::tables::taxi`]. `None` opens a
    /// flight master's window with no nodes on it.
    pub fn taxi(&self) -> Option<&crate::tables::taxi::TaxiTables> {
        self.taxi.as_ref()
    }

    /// The side a faction template belongs to:
    /// `FactionTemplate.factionGroup`, the mask the taxi map compares against
    /// two fixed values. See [`crate::tables::taxi::Team::of_group`].
    pub fn faction_group(&self, template: u32) -> Option<u32> {
        self.factions.as_ref()?.group(template)
    }

    /// The same side as names: the two values `UnitFactionGroup` returns. See
    /// [`crate::tables::faction::Factions::group_name`] for the empty-name rule.
    pub fn faction_group_name(&self, template: u32) -> Option<(&str, &str)> {
        self.factions.as_ref()?.group_name(template)
    }

    /// How one unit stands towards another, by faction template.
    ///
    /// Provided here as well as on [`crate::tables::faction::Factions`] so that
    /// a caller holding the tables does not have to handle a missing table:
    /// with no table the answer is
    /// [`crate::tables::faction::Reaction::Neutral`], which is also what a
    /// missing template resolves to.
    pub fn reaction(
        &self,
        template: Option<u32>,
        towards: Option<u32>,
    ) -> crate::tables::faction::Reaction {
        self.template_rank(template, towards).into()
    }

    /// The same answer on the client's eight-rank scale, for the two callers
    /// that compare it against a minimum instead of reducing it to three
    /// values. See [`crate::look::cursor::can_interact`], whose test is
    /// `>= Neutral` and therefore refuses Unfriendly; the three-value
    /// [`crate::tables::faction::Reaction`] cannot express that.
    pub fn template_rank(
        &self,
        template: Option<u32>,
        towards: Option<u32>,
    ) -> crate::tables::faction::Rank {
        match &self.factions {
            Some(factions) => factions.template_rank(template, towards),
            None => crate::tables::faction::Rank::Neutral,
        }
    }

    /// The light chain itself, for `vale water`'s band survey.
    pub fn light(&self) -> Option<&crate::tables::light::LightTables> {
        self.light.as_ref()
    }

    /// The colour and opacity of one liquid at position `at` on one map, at
    /// `time` half-minutes past midnight. See [`crate::tables::light`] for why
    /// this is not in the texture.
    ///
    /// `None` means the liquid is drawn from its own texture. That is always
    /// the case for magma and slime, whose flipbooks are the only two that
    /// carry colour. Water and ocean fall back to
    /// [`crate::tables::light::LiquidLight::UNLIT`], a conspicuous white, when
    /// the chain has no light tables at all.
    ///
    /// `at` is the camera position, not the liquid surface's position. A
    /// liquid's two colours are bands of a `LightParams` row, the same row the
    /// fog, sky and sun come from, which the renderer already resolves once a
    /// frame at the character. Resolving per surface also splits unevenly: a
    /// draw is per ADT tile per liquid kind, so a lake crossing a tile boundary
    /// used two centres 533 yards apart, on either side of a falloff sphere,
    /// and drew a straight tint seam down the middle of the water. The cost of
    /// the camera position is that a lake's colour changes as the viewer moves
    /// around it.
    ///
    /// `vale-client` calls this once a frame, from the camera's cell; see
    /// `crate::render::water` there, the only caller. `vale water`
    /// passes a tile centre, because it reports what the water at that tile
    /// looks like.
    pub fn liquid_light(
        &self,
        map: u32,
        at: [f32; 3],
        kind: crate::world::wmo::Liquid,
        time: u32,
    ) -> Option<crate::tables::light::LiquidLight> {
        match &self.light {
            Some(tables) => tables.liquid_at(map, at, kind, time),
            None => match kind {
                crate::world::wmo::Liquid::Water | crate::world::wmo::Liquid::Ocean => {
                    Some(crate::tables::light::LiquidLight::UNLIT)
                }
                _ => None,
            },
        }
    }

    /// The same liquid as a shallow and a deep colour, which is how the shipped
    /// minimaps were rendered; see
    /// [`crate::tables::light::LightTables::liquid_by_depth_at`] and the
    /// measurement on `band::OCEAN_SHALLOW`. `close` is the colour at depth
    /// byte 0 and `far` at 255. The same `None` and the same fallback as
    /// [`Self::liquid_light`].
    pub fn liquid_depth_light(
        &self,
        map: u32,
        at: [f32; 3],
        kind: crate::world::wmo::Liquid,
        time: u32,
    ) -> Option<crate::tables::light::LiquidLight> {
        match &self.light {
            Some(tables) => tables.liquid_by_depth_at(map, at, kind, time),
            None => match kind {
                crate::world::wmo::Liquid::Water | crate::world::wmo::Liquid::Ocean => {
                    Some(crate::tables::light::LiquidLight::UNLIT)
                }
                _ => None,
            },
        }
    }

    /// What a helmet hides on the wearer, from `HelmetGeosetVisData.dbc`.
    ///
    /// `None` when the chain has no such table. The only cost is hair drawn
    /// through a helm.
    pub fn helmet_visibility(&self) -> Option<&crate::tables::item::HelmetVisibility> {
        self.helmet_vis.as_ref()
    }

    /// Which animation an `SMSG_EMOTE` id means, or `None` for an emote with no
    /// animation of its own and for a chain with no `Emotes.dbc`.
    pub fn emote_animation(&self, emote_id: u32) -> Option<u16> {
        self.emotes.as_ref()?.animation(emote_id)
    }

    /// The emote table itself, for the survey that checks it.
    pub fn emotes(&self) -> Option<&Emotes> {
        self.emotes.as_ref()
    }

    /// What a text emote says; see [`crate::tables::emotetext`].
    pub fn emote_texts(&self) -> Option<&crate::tables::emotetext::EmoteTexts> {
        self.emote_texts.as_ref()
    }

    /// The weapon policy of the animation about to be played; see
    /// [`crate::look::sheath::reconcile`], the only caller.
    ///
    /// Zero for an unknown id and for a chain with no table. The two cases give
    /// the same answer by design: zero means "this clip has no sheath policy",
    /// and most of the table's 208 rows hold it. A missing `AnimationData.dbc`
    /// therefore degrades to "no clip forces a change", not to a wrong change.
    pub fn weapon_flags(&self, anim_id: u16) -> u32 {
        self.animations
            .as_ref()
            .map_or(0, |table| table.weapon_flags(anim_id))
    }

    /// The animation table itself: the names, for a check that reports
    /// `Attack1H` instead of `17`.
    pub fn animations(&self) -> Option<&AnimationData> {
        self.animations.as_ref()
    }

    /// The caster's animation for a spell: the wind-up held while the cast bar
    /// runs, and the release at the end.
    ///
    /// `None` for a spell with no visual and for a chain with no spell tables.
    /// In both cases the caller falls back to the generic cast animations.
    pub fn cast_animation(&self, spell_id: u32) -> Option<crate::tables::spell::CastAnimation> {
        self.spells.as_ref()?.cast(spell_id)
    }

    /// The models a spell's kits attach to its caster. They are separate from
    /// the pose because either can exist without the other; see
    /// [`crate::tables::spell`].
    pub fn cast_effects(&self, spell_id: u32) -> Option<&crate::tables::spell::CastEffects> {
        self.spells.as_ref()?.effects(spell_id)
    }

    /// The projectile a spell launches. Unlike the pose and the kit models, it
    /// is not attached to the caster. See [`crate::tables::spell`].
    pub fn cast_missile(&self, spell_id: u32) -> Option<&crate::tables::spell::Missile> {
        self.spells.as_ref()?.missile(spell_id)
    }

    /// The bolt a spell draws between units. It is the only spell visual in
    /// this file that names no model: `SpellChainEffects` gives a texture and
    /// six numbers, and the client builds the geometry. See
    /// [`crate::tables::spell::SpellVisuals::chain`].
    pub fn cast_chain(&self, spell_id: u32) -> Option<&crate::tables::spell::ChainVisual> {
        self.spells.as_ref()?.chain(spell_id)
    }

    /// How a persistent area of a spell is drawn, such as Blizzard,
    /// Flamestrike or Rain of Fire. It is the one appearance in the game that
    /// is not reached through a display id. See
    /// [`crate::tables::spell::SpellVisuals::area`].
    pub fn spell_area(&self, spell_id: u32) -> Option<&crate::tables::spell::AreaEffect> {
        self.spells.as_ref()?.area(spell_id)
    }

    /// The models a `SpellVisualKit` attaches, looked up by kit id.
    ///
    /// This is the only entry into the spell visual chain that does not start
    /// at a spell, and the only one that can handle `SMSG_PLAY_SPELL_VISUAL`,
    /// whose body is a guid and a kit id with no spell id. See
    /// [`crate::tables::spell::SpellVisuals::kit`].
    pub fn kit_effects(&self, kit_id: u32) -> Option<&Vec<crate::tables::spell::KitEffect>> {
        self.spells.as_ref()?.kit(kit_id)
    }

    /// The pose a kit holds. For kits 406 and 438 (food and drink), the pose is
    /// the visible part of the effect.
    pub fn kit_pose(&self, kit_id: u32) -> Option<u16> {
        self.spells.as_ref()?.kit_pose(kit_id)
    }

    /// The models a unit wears while a spell's aura is on it: the `stateKit`.
    /// It is the only answer in this chain that lasts for a duration instead of
    /// a moment. See [`crate::tables::spell::SpellVisuals::state`].
    pub fn aura_effects(&self, spell_id: u32) -> Option<&Vec<crate::tables::spell::KitEffect>> {
        self.spells.as_ref()?.state(spell_id)
    }

    /// The pose the same state kit holds, from another column of the same row;
    /// this is how a stun is shown. See
    /// [`crate::tables::spell::SpellVisuals::aura_pose`].
    pub fn aura_pose(&self, spell_id: u32) -> Option<u16> {
        self.spells.as_ref()?.aura_pose(spell_id)
    }

    /// The colour the aura applies to the bearer's own model, from a third
    /// column of the same row; this is how Stoneform, Ghost and Shadowform are
    /// shown. See [`crate::tables::spell::ModelTint`].
    pub fn aura_tint(&self, spell_id: u32) -> Option<crate::tables::spell::ModelTint> {
        self.spells.as_ref()?.aura_tint(spell_id)
    }

    /// Whether the spell lands on the unit that cast it. See
    /// [`crate::tables::spell::SpellVisuals::is_self_cast`]; it is asked only
    /// for a release whose hit list named no target. `false` with no spell
    /// tables, which is the safer default: an impact drawn on nobody costs
    /// nothing, while one drawn on the wrong unit puts a burst on the caster.
    pub fn is_self_cast(&self, spell_id: u32) -> bool {
        self.spells
            .as_ref()
            .is_some_and(|s| s.is_self_cast(spell_id))
    }

    /// The spell tables themselves, for the survey that checks them.
    pub fn spells(&self) -> Option<&crate::tables::spell::SpellVisuals> {
        self.spells.as_ref()
    }

    /// Which geosets an appearance selects. This is the entry point for a
    /// player, whose appearance comes from the server instead of a DBC.
    pub fn character_geosets(&self, look: &Appearance) -> crate::world::m2::CharacterGeosets {
        self.char_geosets.geosets(look)
    }

    /// `ItemDisplayInfo.dbc`, if the chain has it: the appearance of each piece
    /// of equipment, once the server has said which row an item entry uses.
    pub fn items(&self) -> Option<&crate::tables::item::ItemDisplays> {
        self.items.as_ref()
    }

    /// The geoset tables themselves, for the survey that checks them.
    pub fn char_geosets(&self) -> &CharGeosets {
        &self.char_geosets
    }

    /// The model for a unit's or player's `DISPLAYID`.
    pub fn creature(&self, display_id: u32) -> Option<DisplayModel> {
        let display = self.creature_display.row(display_id)?;
        let model_id = self.creature_display.dbc.u32_at(display, display_fields::MODEL_ID)?;
        let model = self.creature_model.row(model_id)?;
        let name = self
            .creature_model
            .dbc
            .string_at(model, display_fields::MODEL_NAME)
            .filter(|s| !s.is_empty())?;
        let path = crate::world::m2::model_path(&name);

        // Texture variations are bare file names living beside the model, so
        // `Creature\Wolf\Wolf.m2` + `WolfSkinGrey` is
        // `Creature\Wolf\WolfSkinGrey.blp`. Absent variations are empty strings
        // and are kept: a variation's position is its texture type, so
        // dropping one shifts the rest onto the wrong slots.
        let dir = match path.rfind('\\') {
            Some(at) => &path[..=at],
            None => "",
        };
        let mut skins: Vec<String> = (0..display_fields::TEXTURE_VARIATION_COUNT)
            .map(|i| {
                self.creature_display
                    .dbc
                    .string_at(display, display_fields::TEXTURE_VARIATION + i)
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("{dir}{s}.blp"))
                    .unwrap_or_default()
            })
            .collect();

        // An NPC wearing a character model has no texture variations. Its skin
        // was baked offline from a race, a face, a hairstyle and ten equipped
        // items, and the result ships as one texture named only in
        // `CreatureDisplayInfoExtra`. It fills slot 0, because the character
        // M2 requests it as texture type 1 where a creature requests type 11.
        if skins[0].is_empty() {
            if let Some(baked) = self.baked_skin(display) {
                skins[0] = baked;
            }
        }

        let display_scale = self
            .creature_display
            .dbc
            .f32_at(display, display_fields::SCALE)
            .filter(|s| s.is_finite() && *s > 0.0)
            .unwrap_or(1.0);
        let model_scale = self
            .creature_model
            .dbc
            .f32_at(model, display_fields::MODEL_SCALE)
            .filter(|s| s.is_finite() && *s > 0.0)
            .unwrap_or(1.0);

        let appearance = self.extra_appearance(display);
        Some(DisplayModel {
            path,
            skins,
            scale: display_scale * model_scale,
            character: appearance.map(|look| self.char_geosets.geosets(&look)),
            appearance,
            // Only an NPC in a character model has equipment: the columns are
            // in the same `CreatureDisplayInfoExtra` row as the appearance.
            equipment: match appearance {
                Some(_) => self.equipped_items(display_id),
                None => Vec::new(),
            },
        })
    }

    /// The appearance behind a display id, for the survey that checks the
    /// geosets against the models. In the data, only a display id ties a race
    /// and gender to a model path.
    pub fn appearance(&self, display_id: u32) -> Option<Appearance> {
        self.extra_appearance(self.creature_display.row(display_id)?)
    }

    /// The items an NPC in a character model wears, as
    /// `(ItemDisplayInfo id, Slot)` pairs, for drawing the geometry of its
    /// gear.
    ///
    /// The ten columns are in equipment-slot order. The order is measured, not
    /// transcribed from documentation: `vale npc` resolves every column of
    /// every row through `ItemDisplayInfo` and prints which garment each one
    /// names, from the component textures' filenames. Column 2 is a shirt in
    /// every row that fills it, column 6 a boot, column 8 a glove, and so on
    /// through the standard slot list.
    ///
    /// The textures are not returned. They are already in the bake, so
    /// painting them again would duplicate precomputed work, and on a model
    /// whose UVs already address the baked atlas it would change nothing. The
    /// bake lacks only the geometry: the attached models and the geosets.
    pub fn equipped_items(&self, display_id: u32) -> Vec<crate::tables::item::Equipped> {
        Self::EQUIPPED_SLOTS
            .iter()
            .zip(self.equipped_item_columns(display_id))
            .filter(|(_, id)| *id != 0)
            .map(|(slot, display_id)| crate::tables::item::Equipped {
                display_id,
                slot: *slot,
            })
            .collect()
    }

    /// The slot each of the ten columns fills, in order. Public so the survey
    /// can print the mapping it checks instead of restating it.
    pub const EQUIPPED_SLOTS: [crate::tables::item::Slot; display_fields::EQUIPPED_ITEM_COUNT] = {
        use crate::tables::item::Slot;
        [
            Slot::Head,
            Slot::Shoulders,
            Slot::Shirt,
            Slot::Chest,
            Slot::Waist,
            Slot::Legs,
            Slot::Feet,
            Slot::Wrists,
            Slot::Hands,
            Slot::Tabard,
        ]
    };

    /// The ten columns verbatim, zeros included.
    ///
    /// Only the survey needs the zeros: dropping them compacts the list and
    /// loses the column index, which is what the survey measures.
    pub fn equipped_item_columns(&self, display_id: u32) -> [u32; display_fields::EQUIPPED_ITEM_COUNT] {
        let mut out = [0u32; display_fields::EQUIPPED_ITEM_COUNT];
        let Some(display) = self.creature_display.row(display_id) else {
            return out;
        };
        let Some(extra_id) = self
            .creature_display
            .dbc
            .u32_at(display, display_fields::EXTENDED_DISPLAY_INFO_ID)
            .filter(|&id| id != 0)
        else {
            return out;
        };
        let Some(extra) = self.creature_extra.as_ref() else {
            return out;
        };
        let Some(row) = extra.row(extra_id) else {
            return out;
        };
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = extra
                .dbc
                .u32_at(row, display_fields::EQUIPPED_ITEM + i)
                .unwrap_or(0);
        }
        out
    }

    /// The appearance an NPC's `CreatureDisplayInfoExtra` row describes, if it
    /// has one. Only character-model NPCs have one, so the presence of the row
    /// is the test for "is this a character model". It is a better test than
    /// the model path, which would have to be matched by name.
    fn extra_appearance(&self, display: usize) -> Option<Appearance> {
        let extra_id = self
            .creature_display
            .dbc
            .u32_at(display, display_fields::EXTENDED_DISPLAY_INFO_ID)
            .filter(|&id| id != 0)?;
        let extra = self.creature_extra.as_ref()?;
        let row = extra.row(extra_id)?;
        let mut ids = [0u8; display_fields::APPEARANCE_COUNT];
        for (i, id) in ids.iter_mut().enumerate() {
            *id = extra
                .dbc
                .u32_at(row, display_fields::APPEARANCE + i)
                .unwrap_or(0) as u8;
        }
        let [race, gender, skin, face, hair_style, hair_colour, facial_hair] = ids;
        Some(Appearance {
            race,
            gender,
            skin,
            face,
            hair_style,
            hair_colour,
            facial_hair,
        })
    }

    /// The baked skin for one `CreatureDisplayInfo` record, or `None` if that
    /// row has no extended info. `None` is the common case, since every
    /// non-character creature is textured by its own model or its variations.
    fn baked_skin(&self, display: usize) -> Option<String> {
        let extra_id = self
            .creature_display
            .dbc
            .u32_at(display, display_fields::EXTENDED_DISPLAY_INFO_ID)
            .filter(|&id| id != 0)?;
        let extra = self.creature_extra.as_ref()?;
        let row = extra.row(extra_id)?;
        let name = extra
            .dbc
            .string_at(row, display_fields::BAKE_NAME)
            .filter(|s| !s.is_empty())?;
        Some(format!("{BAKED_NPC_TEXTURES}{name}"))
    }

    /// The model for a game object's `DISPLAYID`: chests, mailboxes, doors.
    ///
    /// Some of them are `.wmo` files, not M2s. The transports (zeppelins,
    /// boats, the Deeprun Tram) are WMO buildings, and `GameObjectDisplayInfo`
    /// names them with the `.wmo` extension. `model_path` would append `.m2`
    /// to those and produce `…\transport_zeppelin.wmo.m2`, which is in no
    /// archive. The name is therefore kept unchanged, so a caller can tell the
    /// file type from the extension.
    pub fn game_object(&self, display_id: u32) -> Option<DisplayModel> {
        let row = self.object_display.row(display_id)?;
        let name = self
            .object_display
            .dbc
            .string_at(row, display_fields::OBJECT_MODEL_NAME)
            .filter(|s| !s.is_empty())?;
        let path = if name.to_ascii_lowercase().ends_with(".wmo") {
            name
        } else {
            crate::world::m2::model_path(&name)
        };
        Some(DisplayModel {
            path,
            skins: Vec::new(),
            scale: 1.0,
            // Game objects have no character appearance and no equipment.
            character: None,
            appearance: None,
            equipment: Vec::new(),
        })
    }
}

/// One of [`DisplayTables::load`]'s three mandatory tables.
///
/// A missing table is reported as [`AssetError::NotFound`] with the table's
/// archive path, because that path tells the user which file to look for. The
/// reader itself only returns `None`.
fn required(
    read: &mut impl FnMut(&str) -> Option<Vec<u8>>,
    table: &str,
) -> Result<Indexed, AssetError> {
    let raw = read(table).ok_or_else(|| AssetError::NotFound(dbc_path(table)))?;
    Indexed::parse(&raw)
}

/// A DBC plus an id -> record index map.
///
/// Field 0 is the id in every table, but the records are not guaranteed to be
/// dense or sorted, so the map is built instead of using the id as an offset.
struct Indexed {
    dbc: Dbc,
    by_id: HashMap<u32, usize>,
}

impl Indexed {
    fn parse(buf: &[u8]) -> Result<Indexed, AssetError> {
        let dbc = Dbc::parse(buf)?;
        let mut by_id = HashMap::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            if let Some(id) = dbc.u32_at(record, 0) {
                by_id.insert(id, record);
            }
        }
        Ok(Indexed { dbc, by_id })
    }

    fn row(&self, id: u32) -> Option<usize> {
        self.by_id.get(&id).copied()
    }
}

/// `Emotes.dbc`: the link between an emote id from the server and an
/// animation.
///
/// `SMSG_EMOTE` is the only packet that comes close to naming an animation,
/// and this table completes the mapping: `u32 emoteId` in the packet is an id
/// here, and field 2 is an `AnimationData.dbc` id.
///
/// Field 2 is measured, and the data confirms it. That matters because the
/// surrounding columns are all small integers, and a wrong index would produce
/// a plausible animation instead of a failure. The two tables name their rows
/// independently and the names agree: row 1 is `ONESHOT_TALK(DNR)` and its
/// field 2 is 60, which `AnimationData` calls `EmoteTalk`; row 2 is
/// `ONESHOT_BOW` and its field 2 is 66, `EmoteBow`. Only the correct column
/// matches both.
///
/// The remaining columns are `id`, the name string, then flags, a spec proc,
/// its parameter and a sound id. This client reads none of them.
pub struct Emotes {
    dbc: Dbc,
    /// Emote id -> `AnimationData` id, built once. The table has 78 rows, so
    /// the map exists to state the mapping more than for speed.
    animation: HashMap<u32, u16>,
    /// The rows whose `EmoteType` is non-zero: the held emotes, which the
    /// server never sends as a packet. See [`Emotes::EMOTE_TYPE`].
    states: std::collections::HashSet<u32>,
}

impl Emotes {
    /// Which `AnimationData.dbc` id each emote plays.
    ///
    /// Field 2. Zero is treated as no animation: `ONESHOT_NONE` is row 0 with
    /// animation 0, and animation 0 is `Stand`. An emote that resolved to Stand
    /// would interrupt whatever the character was doing, which is worse than
    /// ignoring it.
    const ANIMATION: usize = 2;

    /// `EmoteType`: 0 is a one-shot and any other value is a state. The
    /// distinction decides which part of the protocol carries the emote.
    ///
    /// vmangos `Unit::HandleEmote` branches on this: a one-shot is sent as
    /// `SMSG_EMOTE`, and a state is written into `UNIT_NPC_EMOTESTATE`, an
    /// update field that persists. A client that reads only the packet
    /// animates `/wave` but not `/dance`, and not an innkeeper held in
    /// `STATE_WORK`, which is the more visible case.
    ///
    /// Measured, and the data is unambiguous: every `ONESHOT_*` row reads 0
    /// here and every `STATE_*` row reads 2.
    const EMOTE_TYPE: usize = 4;

    pub fn parse(buf: &[u8]) -> Result<Emotes, AssetError> {
        let dbc = Dbc::parse(buf)?;
        let mut animation = HashMap::with_capacity(dbc.record_count);
        let mut states = std::collections::HashSet::new();
        for record in 0..dbc.record_count {
            let Some(id) = dbc.u32_at(record, 0) else {
                continue;
            };
            if dbc.u32_at(record, Self::EMOTE_TYPE).unwrap_or(0) != 0 {
                states.insert(id);
            }
            let Some(anim) = dbc.u32_at(record, Self::ANIMATION).filter(|a| *a != 0) else {
                continue;
            };
            animation.insert(id, anim as u16);
        }
        Ok(Emotes {
            dbc,
            animation,
            states,
        })
    }

    /// Whether this emote is a held state instead of a one-shot.
    pub fn is_state(&self, emote_id: u32) -> bool {
        self.states.contains(&emote_id)
    }

    pub fn animation(&self, emote_id: u32) -> Option<u16> {
        self.animation.get(&emote_id).copied()
    }

    /// How many rows the table has, and how many resolve to an animation: the
    /// pair `vale emote` reports.
    pub fn counts(&self) -> (usize, usize) {
        (self.dbc.record_count, self.animation.len())
    }

    /// Every row, as `(emote id, name, animation id)`. The name is field 1 and
    /// lets the animation column be checked by eye: `ONESHOT_BOW` must resolve
    /// to an animation named `EmoteBow`.
    pub fn rows(&self) -> Vec<(u32, String, Option<u16>)> {
        (0..self.dbc.record_count)
            .filter_map(|record| {
                let id = self.dbc.u32_at(record, 0)?;
                let name = self.dbc.string_at(record, 1).unwrap_or_default();
                Some((id, name, self.animation(id)))
            })
            .collect()
    }
}

/// `AnimationData.dbc`: the table that names every animation in this project.
/// One of its columns is a rule, not a label.
///
/// 208 rows of seven columns: `id`, the name, `WeaponFlags`, `BodyFlags`, two
/// unidentified, and a fallback animation. This client reads the first three.
///
/// Column 2 is the sheath policy; see [`crate::look::sheath`], which consumes
/// it and documents the bits. The column matches the 1.12.1 client's sheath
/// behaviour, and the data confirms the identification: the column's entire
/// value set in 5875 is `{0, 4, 16, 20, 32}`, which is the three tested bits
/// and their one combination, a pattern a wrongly indexed column would not
/// produce. The names in the same rows agree: 17 is `Attack1H` and reads 32,
/// 16 is `AttackUnarmed` and reads 16, 42 is `Swim` and reads 4.
///
/// Column 6 is the game's own fallback for a missing clip. It is read because
/// keeping it costs nothing, but nothing uses it yet. `entities::fallbacks` is
/// a hand-written table that agrees with it in most cases. Adopting this
/// column is separate work: the file contains a cycle
/// (`Open -> Close -> Open`), and this client is deliberately more generous in
/// a few places. Until then, checks compare against [`Self::fallback`].
pub struct AnimationData {
    /// Animation id -> `WeaponFlags`. Only the non-zero rows, since zero is
    /// both the default and the most common value.
    weapon_flags: HashMap<u16, u32>,
    /// Animation id -> its `Name` string, for readable checks.
    names: HashMap<u16, String>,
    /// Animation id -> the substitute the file names, for ids that name one.
    fallback: HashMap<u16, u16>,
}

impl AnimationData {
    const NAME: usize = 1;
    const WEAPON_FLAGS: usize = 2;
    const FALLBACK: usize = 6;

    pub fn parse(buf: &[u8]) -> Result<AnimationData, AssetError> {
        let dbc = Dbc::parse(buf)?;
        let mut weapon_flags = HashMap::new();
        let mut names = HashMap::with_capacity(dbc.record_count);
        let mut fallback = HashMap::new();
        for record in 0..dbc.record_count {
            let Some(id) = dbc.u32_at(record, 0) else {
                continue;
            };
            let id = id as u16;
            if let Some(flags) = dbc.u32_at(record, Self::WEAPON_FLAGS).filter(|f| *f != 0) {
                weapon_flags.insert(id, flags);
            }
            if let Some(name) = dbc.string_at(record, Self::NAME).filter(|n| !n.is_empty()) {
                names.insert(id, name);
            }
            // Zero is `Stand`, which the file uses to mean "no substitute
            // stated", not as a substitute. A row that falls back to itself is
            // not a fallback either.
            if let Some(to) = dbc
                .u32_at(record, Self::FALLBACK)
                .map(|f| f as u16)
                .filter(|f| *f != 0 && *f != id)
            {
                fallback.insert(id, to);
            }
        }
        Ok(AnimationData {
            weapon_flags,
            names,
            fallback,
        })
    }

    /// The sheath policy for an animation. Zero for an unknown id, which is the
    /// same as "no policy"; see [`DisplayTables::weapon_flags`].
    pub fn weapon_flags(&self, id: u16) -> u32 {
        self.weapon_flags.get(&id).copied().unwrap_or(0)
    }

    pub fn name(&self, id: u16) -> Option<&str> {
        self.names.get(&id).map(String::as_str)
    }

    /// The substitute the file names for a model that lacks this clip, for a
    /// check that compares it against the compiled fallback chains. Not used
    /// to resolve anything; see the type's comment.
    pub fn fallback(&self, id: u16) -> Option<u16> {
        self.fallback.get(&id).copied()
    }

    /// How many rows carry each `WeaponFlags` value, ascending. The survey
    /// prints this to show the column was read correctly. The set must be
    /// exactly `{4, 16, 20, 32}` (zero is not stored).
    pub fn policy_census(&self) -> Vec<(u32, usize)> {
        let mut counts: HashMap<u32, usize> = HashMap::new();
        for flags in self.weapon_flags.values() {
            *counts.entry(*flags).or_default() += 1;
        }
        let mut out: Vec<_> = counts.into_iter().collect();
        out.sort_unstable();
        out
    }

    /// Rows, and rows carrying a policy.
    pub fn counts(&self) -> (usize, usize) {
        (self.names.len(), self.weapon_flags.len())
    }
}

/// Hand-built DBCs, for tests in other modules that need a [`DisplayTables`]
/// but do not test the tables themselves.
///
/// [`crate::look::dress`] uses it: the dressing rule is tested without an
/// archive, and it takes tables as input. Keeping the builders here instead of
/// copying them there means a change to a field layout needs updating in one
/// place.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A DBC of `fields` u32 columns, with `strings` as the string block.
    ///
    /// Rows shorter than `fields` are zero-padded, so a test states only the
    /// columns it is about.
    pub fn dbc(rows: &[Vec<u32>], fields: usize, strings: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(DBC_MAGIC);
        v.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        v.extend_from_slice(&(fields as u32).to_le_bytes());
        v.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        v.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for r in rows {
            for f in 0..fields {
                v.extend_from_slice(&r.get(f).copied().unwrap_or(0).to_le_bytes());
            }
        }
        v.extend_from_slice(strings);
        v
    }

    /// [`DisplayTables::load`] over a fixed set of tables, by name.
    ///
    /// A table not listed is absent (the reader returns `None`), so a test
    /// lists only the tables it needs, and the rest degrade as
    /// [`DisplayTables::load`] documents.
    pub fn tables(named: &[(&str, Vec<u8>)]) -> Result<DisplayTables, AssetError> {
        DisplayTables::load(|table| {
            named
                .iter()
                .find(|(name, _)| *name == table)
                .map(|(_, raw)| raw.clone())
        })
    }

    /// The three mandatory tables, as empty but valid DBCs, for a test of one
    /// of the optional tables.
    pub fn empty_required() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("CreatureDisplayInfo", dbc(&[vec![0; 12]], 12, b"\0")),
            ("CreatureModelData", dbc(&[vec![0; 16]], 16, b"\0")),
            ("GameObjectDisplayInfo", dbc(&[vec![0; 12]], 12, b"\0")),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{dbc as build_wide, empty_required, tables};
    use super::*;

    fn build(records: &[[u32; 2]], strings: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(DBC_MAGIC);
        v.extend_from_slice(&(records.len() as u32).to_le_bytes());
        v.extend_from_slice(&2u32.to_le_bytes()); // field count
        v.extend_from_slice(&8u32.to_le_bytes()); // record size
        v.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for r in records {
            for f in r {
                v.extend_from_slice(&f.to_le_bytes());
            }
        }
        v.extend_from_slice(strings);
        v
    }

    #[test]
    fn reads_ids_and_strings() {
        // String block: offset 0 is an empty string by convention.
        let strings = b"\0Azeroth\0Kalimdor\0";
        let dbc = build(&[[0, 1], [1, 9]], strings);
        let maps = map_directories(&dbc).unwrap();
        assert_eq!(maps.get(&0).map(String::as_str), Some("Azeroth"));
        assert_eq!(maps.get(&1).map(String::as_str), Some("Kalimdor"));
    }

    #[test]
    fn rejects_non_dbc() {
        assert!(Dbc::parse(b"not a dbc file at all").is_err());
    }

    #[test]
    fn a_display_id_resolves_through_two_tables_to_a_model_and_its_skins() {
        // Strings: 1 = the model path, 31 = a skin name.
        let strings = b"\0Creature\\Wolf\\Wolf.mdx\0WolfSkinGrey\0";
        let model_at = 1u32;
        let skin_at = 24u32;
        assert_eq!(&strings[skin_at as usize..skin_at as usize + 4], b"Wolf");

        // CreatureDisplayInfo record: id 42 -> model 7, scale 2.0, one skin.
        let mut display = vec![0u32; 12];
        display[0] = 42;
        display[display_fields::MODEL_ID] = 7;
        display[display_fields::SCALE] = 2.0f32.to_bits();
        display[display_fields::TEXTURE_VARIATION] = skin_at;

        // CreatureModelData record: id 7, the path, modelScale 0.5.
        let mut model = vec![0u32; 16];
        model[0] = 7;
        model[display_fields::MODEL_NAME] = model_at;
        model[display_fields::MODEL_SCALE] = 0.5f32.to_bits();

        // No `CreatureDisplayInfoExtra`, geoset, item display or helmet tables:
        // the optional tables are absent because this test does not use them.
        let tables = tables(&[
            ("CreatureDisplayInfo", build_wide(&[display], 12, strings)),
            ("CreatureModelData", build_wide(&[model], 16, strings)),
            (
                "GameObjectDisplayInfo",
                build_wide(&[vec![0; 12]], 12, b"\0"),
            ),
        ])
        .unwrap();

        let resolved = tables.creature(42).expect("display 42");
        // .mdx in the DBC, .m2 in the archive.
        assert_eq!(resolved.path, "Creature\\Wolf\\Wolf.m2");
        // A skin is a bare name beside the model, not a path.
        assert_eq!(resolved.skins[0], "Creature\\Wolf\\WolfSkinGrey.blp");
        assert_eq!(resolved.skins[1], "", "an absent variation keeps its slot");
        // scale is the product, matching Unit::GetScaleForDisplayId.
        assert_eq!(resolved.scale, 1.0);

        assert_eq!(tables.creature(43), None, "unknown display id");
    }

    /// An NPC wearing a character model: no texture variations, and its skin
    /// reached through `extendedDisplayInfoId` instead.
    #[test]
    fn a_character_model_npc_takes_its_baked_skin_into_slot_zero() {
        let strings = b"\0Character\\Human\\Male\\HumanMale.mdx\0973e54e79012eea3f2658f2897e681d9.blp\0";
        let bake_at = 36u32;
        assert_eq!(&strings[bake_at as usize..bake_at as usize + 3], b"973");

        // CreatureDisplayInfo 1234 -> model 49, extra 800, and no variations.
        let mut display = vec![0u32; 12];
        display[0] = 1234;
        display[display_fields::MODEL_ID] = 49;
        display[display_fields::EXTENDED_DISPLAY_INFO_ID] = 800;

        let mut model = vec![0u32; 16];
        model[0] = 49;
        model[display_fields::MODEL_NAME] = 1;

        let mut extra = vec![0u32; 19];
        extra[0] = 800;
        extra[display_fields::BAKE_NAME] = bake_at;

        let tables = tables(&[
            ("CreatureDisplayInfo", build_wide(&[display], 12, strings)),
            ("CreatureModelData", build_wide(&[model], 16, strings)),
            (
                "GameObjectDisplayInfo",
                build_wide(&[vec![0; 12]], 12, b"\0"),
            ),
            (
                "CreatureDisplayInfoExtra",
                build_wide(&[extra], 19, strings),
            ),
        ])
        .unwrap();

        let resolved = tables.creature(1234).expect("display 1234");
        assert_eq!(resolved.path, "Character\\Human\\Male\\HumanMale.m2");
        // The bake is named without a directory and lives in one shared
        // directory, unlike a texture variation, which lives beside its model.
        assert_eq!(
            resolved.skins[0],
            "Textures\\BakedNpcTextures\\973e54e79012eea3f2658f2897e681d9.blp"
        );
        assert_eq!(resolved.skins[1], "", "the other slots stay empty");
    }

    /// The bake must not replace a texture variation that already fills slot
    /// 0. The two texture types share the slot, so the wrong precedence would
    /// draw a wolf with a character's baked skin.
    #[test]
    fn a_texture_variation_outranks_a_bake_for_the_same_slot() {
        let strings = b"\0Creature\\Wolf\\Wolf.mdx\0WolfSkinGrey\0baked.blp\0";
        let skin_at = 24u32;
        let bake_at = 37u32;
        assert_eq!(&strings[skin_at as usize..skin_at as usize + 4], b"Wolf");
        assert_eq!(&strings[bake_at as usize..bake_at as usize + 5], b"baked");

        let mut display = vec![0u32; 12];
        display[0] = 7;
        display[display_fields::MODEL_ID] = 7;
        display[display_fields::EXTENDED_DISPLAY_INFO_ID] = 3;
        display[display_fields::TEXTURE_VARIATION] = skin_at;

        let mut model = vec![0u32; 16];
        model[0] = 7;
        model[display_fields::MODEL_NAME] = 1;

        let mut extra = vec![0u32; 19];
        extra[0] = 3;
        extra[display_fields::BAKE_NAME] = bake_at;

        let tables = tables(&[
            ("CreatureDisplayInfo", build_wide(&[display], 12, strings)),
            ("CreatureModelData", build_wide(&[model], 16, strings)),
            (
                "GameObjectDisplayInfo",
                build_wide(&[vec![0; 12]], 12, b"\0"),
            ),
            (
                "CreatureDisplayInfoExtra",
                build_wide(&[extra], 19, strings),
            ),
        ])
        .unwrap();

        assert_eq!(
            tables.creature(7).unwrap().skins[0],
            "Creature\\Wolf\\WolfSkinGrey.blp"
        );
    }

    /// A chain whose `CreatureDisplayInfoExtra` is damaged still resolves
    /// everything else, with the same degradation as a chain that lacks it.
    /// The two cases look the same to a caller, and neither may be fatal.
    #[test]
    fn a_missing_extra_table_costs_only_the_bake() {
        let strings = b"\0Character\\Human\\Male\\HumanMale.mdx\0";
        let mut display = vec![0u32; 12];
        display[0] = 1234;
        display[display_fields::MODEL_ID] = 49;
        display[display_fields::EXTENDED_DISPLAY_INFO_ID] = 800;
        let mut model = vec![0u32; 16];
        model[0] = 49;
        model[display_fields::MODEL_NAME] = 1;

        let tables = tables(&[
            ("CreatureDisplayInfo", build_wide(&[display], 12, strings)),
            ("CreatureModelData", build_wide(&[model], 16, strings)),
            (
                "GameObjectDisplayInfo",
                build_wide(&[vec![0; 12]], 12, b"\0"),
            ),
            ("CreatureDisplayInfoExtra", b"not a dbc".to_vec()),
        ])
        .unwrap();

        let resolved = tables.creature(1234).expect("display 1234");
        assert_eq!(resolved.path, "Character\\Human\\Male\\HumanMale.m2");
        assert_eq!(resolved.skins[0], "");
    }

    #[test]
    fn a_game_object_display_id_holds_its_path_directly() {
        let strings = b"\0World\\Generic\\Chest02.mdx\0";
        let mut row = vec![0u32; 12];
        row[0] = 5;
        row[display_fields::OBJECT_MODEL_NAME] = 1;
        let mut named = empty_required();
        named[2].1 = build_wide(&[row], 12, strings);
        let tables = tables(&named).unwrap();
        let resolved = tables.game_object(5).expect("display 5");
        assert_eq!(resolved.path, "World\\Generic\\Chest02.m2");
        assert!(resolved.skins.is_empty());
    }

    /// A transport is a WMO building, and its row names a `.wmo` file.
    /// Appending `.m2` yields `transport_zeppelin.wmo.m2`, a path in no
    /// archive, which would show up as a broken model instead of as a request
    /// for the wrong kind of file.
    #[test]
    fn a_game_object_that_is_a_building_keeps_its_extension() {
        let name = b"World\\wmo\\transports\\transport_zeppelin\\transport_zeppelin.wmo";
        let mut strings = vec![0u8];
        strings.extend_from_slice(name);
        strings.push(0);
        let mut row = vec![0u32; 12];
        row[0] = 5;
        row[display_fields::OBJECT_MODEL_NAME] = 1;
        let mut named = empty_required();
        named[2].1 = build_wide(&[row], 12, &strings);
        let tables = tables(&named).unwrap();
        assert_eq!(
            tables.game_object(5).expect("display 5").path,
            String::from_utf8_lossy(name)
        );
    }

    /// The three tables without which nothing resolves are required; every
    /// other one is a documented degradation.
    ///
    /// The reader in [`DisplayTables::load`] replaced eight positional
    /// arguments. A caller can no longer transpose two tables, but it can omit
    /// one entirely, so this test fixes which tables are required instead of
    /// leaving it to the doc comment.
    #[test]
    fn only_the_three_tables_that_resolve_a_model_are_required() {
        for missing in [
            "CreatureDisplayInfo",
            "CreatureModelData",
            "GameObjectDisplayInfo",
        ] {
            let named: Vec<_> = empty_required()
                .into_iter()
                .filter(|(name, _)| *name != missing)
                .collect();
            match tables(&named) {
                Err(AssetError::NotFound(path)) => assert!(
                    path.contains(missing),
                    "{missing} was reported as {path} missing"
                ),
                Err(e) => panic!("{missing} gave {e}, not a NotFound naming it"),
                Ok(_) => panic!("{missing} is mandatory and its absence was accepted"),
            }
        }
        // The three required tables alone are enough. Everything the optional
        // tables supply degrades: no bake, no hair, no equipment, no helmet
        // rule.
        let bare = tables(&empty_required()).expect("the three are enough");
        assert!(bare.items().is_none(), "no ItemDisplayInfo was supplied");
        assert!(bare.helmet_visibility().is_none());
        // A chain with no `ChrRaces` returns no race display, which leaves the
        // character-select plinth empty instead of showing a wrong model.
        assert_eq!(bare.race_display(1, 0), None);
    }

    /// `ChrRaces` fields 4 and 5, filled with the values from the real rows.
    ///
    /// The values are what 5875's table holds (human 49/50, night elf 55/56),
    /// so the test records the measurement as well as the lookup. A transposed
    /// pair fails on the gender check instead of drawing every character male
    /// without an error.
    #[test]
    fn a_race_and_a_gender_resolve_to_the_display_the_wire_never_sends() {
        let mut named = empty_required();
        named.push((
            "ChrRaces",
            build_wide(
                &[
                    // id, flags, faction, sound, male, female, ...
                    vec![1, 12, 1, 4140, 49, 50],
                    vec![4, 4, 4, 4145, 55, 56],
                    // A race whose female column is zero. 1.12 has no such row;
                    // it must not resolve to a display id of 0.
                    vec![9, 0, 0, 0, 61, 0],
                ],
                8,
                b"\0",
            ),
        ));
        let t = tables(&named).expect("loads");
        assert_eq!(t.race_display(1, 0), Some(49), "human male");
        assert_eq!(t.race_display(1, 1), Some(50), "human female");
        assert_eq!(t.race_display(4, 1), Some(56), "night elf female");
        // The server's gender byte is 0 or 1. Any other value is an unset byte
        // and reads as male, not as no character.
        assert_eq!(t.race_display(4, 7), Some(55));
        assert_eq!(t.race_display(9, 1), None, "a zero column is not an answer");
        assert_eq!(t.race_display(11, 0), None, "no such race in 1.12");
    }

    /// `AnimationData.dbc`'s three read columns, and the behaviour when the
    /// table is absent.
    ///
    /// A chain with no such table answers zero flags for every animation, the
    /// same answer an id with no policy gets. The sheath reconcile then
    /// degrades to "only combat and the server's sheath byte move the weapons",
    /// not to a wrong rule. The type's doc states this degradation, and this
    /// test keeps it true as the code changes.
    #[test]
    fn the_animation_policy_degrades_to_no_opinion() {
        // id, name, WeaponFlags, _, _, _, fallback: three real 5875 rows.
        let strings = b"\0Attack1H\0Swim\0Stand\0";
        let raw = build_wide(
            &[
                vec![17, 1, 0x20, 0, 0, 0, 16],
                vec![42, 10, 4, 0, 0, 0, 0],
                vec![0, 15, 0, 0, 0, 0, 0],
            ],
            7,
            strings,
        );
        let table = AnimationData::parse(&raw).expect("parses");
        assert_eq!(table.weapon_flags(17), 0x20, "Attack1H draws the melee weapon");
        assert_eq!(table.weapon_flags(42), 4, "Swim stows");
        assert_eq!(table.weapon_flags(0), 0, "Stand has no opinion");
        assert_eq!(table.weapon_flags(9999), 0, "…and neither has an unknown id");
        assert_eq!(table.name(17), Some("Attack1H"));
        assert_eq!(table.fallback(17), Some(16));
        assert_eq!(
            table.fallback(42),
            None,
            "0 in the column is `no substitute stated`, not `fall back to Stand`"
        );

        let mut named = empty_required();
        named.push(("AnimationData", raw));
        let with = tables(&named).expect("loads");
        assert_eq!(with.weapon_flags(17), 0x20);
        // Without the table, every animation has no sheath policy.
        let without = tables(&empty_required()).expect("loads");
        assert_eq!(without.weapon_flags(17), 0);
        assert!(without.animations().is_none());
    }

    #[test]
    fn out_of_range_access_is_none() {
        let dbc = Dbc::parse(&build(&[[7, 1]], b"\0x\0")).unwrap();
        assert_eq!(dbc.u32_at(0, 0), Some(7));
        assert_eq!(dbc.u32_at(5, 0), None);
        assert_eq!(dbc.u32_at(0, 9), None);
    }
}
