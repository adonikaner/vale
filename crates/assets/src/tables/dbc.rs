//! DBC — the client's static database tables (`DBFilesClient\*.dbc`).
//!
//! Format is refreshingly simple:
//! ```text
//! 'WDBC'  magic
//! u32 recordCount, u32 fieldCount, u32 recordSize, u32 stringBlockSize
//! recordCount x recordSize bytes of fixed-width records (all fields u32-sized)
//! stringBlockSize bytes of \0-separated strings
//! ```
//! A "string" field holds a byte offset into that trailing block.
//!
//! Field *meanings* are not in the file — they are per-table conventions. This
//! module gives typed access by index and leaves naming to the caller, which
//! keeps one reader usable for every table.

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

    /// **One record's own bytes** — for the one table in the game whose record is
    /// *narrower* than a field.
    ///
    /// `CharBaseInfo.dbc` is 41 records of **two bytes**, a race and a class, and
    /// [`Self::u32_at`] cannot read it: the last record starts two bytes before
    /// the end of the block, so the four-byte read runs past it and answers
    /// `None`. That is not a hypothetical off-by-one — it silently dropped
    /// `(Troll, Mage)`, the 41st row, and a character-create screen missing one
    /// class button on one race is exactly the kind of wrong this crate is meant
    /// to catch rather than ship.
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
/// name sits further along and is not needed to locate terrain. Reading this
/// rather than hardcoding "0 = Azeroth" means custom maps work for free.
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

/// `Map.dbc`: map id -> the **localised display name** — "Eastern Kingdoms",
/// not "Azeroth".
///
/// A different column from [`map_directories`]'s and a different question: field
/// 1 is the folder under `World\Maps\` and field 4 is the first of eight locale
/// columns holding what a person calls the place. The continent drop-down on the
/// world map shows this one (`+0x10 + locale*4` in the record).
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
/// Measured against maps whose answer is known rather than taken from a name:
/// Azeroth and Kalimdor read 0, Deadmines 1, Ahn'Qiraj 2, Alterac Valley 3.
///
/// **It is read for one reason — [`crate::tables::light`]'s checks only mean anything
/// where there is a sky.** Every shape the light bands hold outdoors (a warm sun
/// over a cool fill, a zenith darker than the sky beneath it) is broken by
/// interiors on purpose: Scarlet Monastery's "sun" is torchlight and its
/// "zenith" is black, because it is a corridor. Grouping by this field turns
/// three counts that look like failures into the statement that they are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapKind {
    /// A continent — the only kind with weather and a horizon.
    World,
    Dungeon,
    Raid,
    Battleground,
}

impl MapKind {
    /// Whether a map is drawn under an actual sky. Battlegrounds are outdoors
    /// and are counted with the world; that they are is checkable, since
    /// Alterac Valley's light is a snowstorm and shares nothing with a dungeon's.
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
    /// Skin textures the model does *not* carry itself, indexed by the slot the
    /// renderer maps each client-supplied texture type onto.
    ///
    /// Slot 0 is the body: a creature M2 declares it as texture type 11 and a
    /// *character* M2 as type 1, and one model is never both, so the two share
    /// the slot. Slots 1 and 2 are creature texture variations 2 and 3 (types
    /// 12 and 13). Which filename fills slot 0 depends on which kind of model
    /// it is — a creature's comes from `CreatureDisplayInfo`'s texture
    /// variations, a character-model NPC's from the baked skin two tables away.
    pub skins: Vec<String>,
    /// `modelScale * displayScale` — the fallback for an entity whose
    /// `OBJECT_FIELD_SCALE_X` never arrived. The field itself is this product
    /// (`Unit::GetScaleForDisplayId`), so the two must not be multiplied
    /// together.
    pub scale: f32,
    /// Set when this display id is an NPC wearing a *character* model, and
    /// carrying the appearance that says which hair and beard to draw.
    ///
    /// It comes from the same `CreatureDisplayInfoExtra` row as the baked skin
    /// — the seven appearance ids beside it, which are otherwise only the
    /// record of what the bake was computed from. The *texture* half of that is
    /// already baked into the file this row names; the *geometry* half is not,
    /// and this is it. `None` for an ordinary creature, which is drawn
    /// [`Dress::Creature`].
    ///
    /// [`Dress::Creature`]: crate::world::m2::Dress::Creature
    pub character: Option<crate::world::m2::CharacterGeosets>,
    /// The appearance behind [`DisplayModel::character`], kept because a
    /// **helm is cut per race** — `Helm_Plate_D_04.mdx` in the row means one of
    /// sixteen files in the archive — so hanging an NPC's gear on it needs the
    /// race and gender, not just the geosets they chose.
    pub appearance: Option<Appearance>,
    /// What this NPC is *wearing*, as `(ItemDisplayInfo id, Slot)`.
    ///
    /// Empty for an ordinary creature and for a player, whose gear comes off the
    /// wire instead. See [`DisplayTables::equipped_items`] for why the bake does
    /// not cover this.
    pub equipment: Vec<crate::tables::item::Equipped>,
}

impl DisplayModel {
    /// Whether a *player* wearing this display id has to have their body texture
    /// **composed** rather than read out of the archive.
    ///
    /// Display ids 49..57 are the bare race models, and the game ships no body
    /// texture for them: a player's skin is built at runtime from `CharSections`
    /// layers, which is the one thing in the world that has no file. Every other
    /// character-model display id is an NPC whose skin was baked offline and
    /// shipped, and slot 0 names it.
    ///
    /// **Asked as "does this display id supply a skin" rather than "is this a
    /// player"**, because those are not the same question and the difference is
    /// visible: a player who is shapeshifted or disguised is wearing an NPC's
    /// display id, and should be drawn with that NPC's bake rather than have
    /// their own face composed over a bear.
    pub fn composes_its_skin(&self) -> bool {
        self.skins.first().is_none_or(|skin| skin.is_empty())
    }
}

/// The display tables, parsed once and indexed by id.
///
/// Four tables. A unit's `DISPLAYID` names a `CreatureDisplayInfo` row, which
/// names a `CreatureModelData` row, which finally holds a path; game objects are
/// one hop, `GameObjectDisplayInfo` holding the path directly. The fourth,
/// `CreatureDisplayInfoExtra`, is off to the side and only exists for the NPCs
/// that wear a *character* model — see [`DisplayTables::creature`].
///
/// Field indices below are **measured**, not taken from a wiki: `vale dbc
/// <Table>` prints each column with its plausible types, and a string field is
/// unmistakable once you look (`CreatureModelData` field 2 on record 0 resolves
/// to `Creature\Basilisk\Basilisk.mdx`, and on record 1 to `BogBeast`).
pub struct DisplayTables {
    creature_display: Indexed,
    creature_model: Indexed,
    object_display: Indexed,
    /// Optional: a chain missing this table costs baked NPC skins, nothing else.
    creature_extra: Option<Indexed>,
    /// The two tables that say which hair and facial-hair *geometry* an
    /// appearance selects. They belong here rather than beside `CharSections`
    /// because both the NPCs that wear a character model and the players who
    /// wear one resolve them, and the NPCs reach them through this table.
    char_geosets: CharGeosets,
    /// `ItemDisplayInfo.dbc`. Optional like the bakes: without it a player is
    /// drawn in their underwear, which is what every player was before it was
    /// read.
    items: Option<crate::tables::item::ItemDisplays>,
    /// `HelmetGeosetVisData.dbc`, which says what a helmet hides. Optional on
    /// the same terms: without it a helm is drawn over the wearer's own hair,
    /// which is visible and survivable.
    helmet_vis: Option<crate::tables::item::HelmetVisibility>,
    /// `Emotes.dbc`, which turns the id in `SMSG_EMOTE` into an animation.
    /// Optional on the same terms: without it a `/dance` is a character
    /// standing still, which is what every emote was before it was read.
    emotes: Option<Emotes>,
    /// `EmotesText.dbc` and its two companions — what a text emote says and
    /// which voice line it plays. Optional on the same terms: without it a
    /// `/dance` is the animation alone.
    emote_texts: Option<crate::tables::emotetext::EmoteTexts>,
    /// `AnimationData.dbc` — what each animation is *called*, and what it
    /// implies about the weapons. Optional: without it the sheath reconcile has
    /// no policy to read, so the weapons follow the engaged draw and the
    /// server's byte alone — a character who swims or gossips with the sword
    /// still in hand.
    animations: Option<AnimationData>,
    /// The light chain, which says what the sun, the sky, the fog and the water
    /// are — see [`crate::tables::light`]. Optional: without it every liquid draws
    /// white and the world takes [`crate::tables::light::Atmosphere::PLACEHOLDER`].
    light: Option<crate::tables::light::LightTables>,
    /// `Spell` + `SpellVisual` + `SpellVisualKit`, which between them say what
    /// a caster *does* — see [`crate::tables::spell`]. Optional: without them every
    /// spell in the game is cast with the same two generic poses, which is
    /// what this client did before they were read.
    spells: Option<crate::tables::spell::SpellVisuals>,
    /// `Spell` again, from the other side: what a spell *is* to a button — see
    /// [`crate::tables::spellbook`]. Optional: without it the action bar has ids and no
    /// names, no costs and no cast bars, and the aiming rule has nothing to read
    /// so every cast is sent at the raw selection.
    spellbook: Option<crate::tables::spellbook::Spells>,
    /// `SpellShapeshiftForm.dbc` — **which of the ten action bars is on the
    /// screen**, see [`crate::tables::spellbook::ShapeshiftForms`]. Not optional in the
    /// same way as its neighbours: an absent table answers 0 for every form,
    /// which is the ordinary page, so what it costs is a stance-using class's
    /// whole bar rather than a degraded one.
    shapeshift: crate::tables::spellbook::ShapeshiftForms,
    /// `FactionTemplate.dbc`, which is the whole of what "friend" means — see
    /// [`crate::tables::faction`]. Optional: without it every unit reads neutral, which
    /// errs towards attackable, so Tab-targeting picks up innkeepers and the
    /// server refuses the swing.
    factions: Option<crate::tables::faction::Factions>,
    /// `Faction.dbc` — see [`crate::tables::reputation`]. Named for the panel it
    /// draws rather than for the file, because [`Self::factions`] is already
    /// `FactionTemplate.dbc` one row up and the two are easy to confuse.
    reputation: Option<crate::tables::reputation::Factions>,
    /// `TaxiNodes` + `TaxiPath` + `TaxiPathNode`, joined against
    /// `WorldMapContinent`'s taxi box — the whole of the flight map, none of
    /// which is on the wire. See [`crate::tables::taxi`]. Optional as a set: without
    /// them `NumTaxiNodes()` is 0 and a flight master's window opens empty,
    /// which is what this client did before it read them.
    taxi: Option<crate::tables::taxi::TaxiTables>,
    /// `SkillLineAbility` + `SkillLine` + `SkillRaceClassInfo`, which between
    /// them say which *page* of the spellbook a spell goes on and which pages
    /// exist at all — see [`crate::tables::skills`]. Optional: without the first two
    /// every spell falls to the General tab, so the book is one long list in
    /// name order rather than the game's four pages; without the third every
    /// weapon skill and profession opens a page of its own.
    skills: Option<crate::tables::skills::Skills>,
    /// …and the same file's other reading: the recipe thresholds the two
    /// profession windows colour by. See [`crate::tables::tradeskill`].
    tradeskills: Option<crate::tables::tradeskill::TradeSkills>,
    /// `SpellFocusObject.dbc` — id to name ("Anvil", "Forge"), for the
    /// Requires line under a recipe. Field 0 is the id and field 1 the enUS
    /// name, measured: row 2 reads "Anvil", row 3 "Forge".
    spell_focus: std::collections::HashMap<u32, String>,
    /// `Talent.dbc` and `TalentTab.dbc` — the three trees a class may spend
    /// points in. See [`crate::tables::talent`]. **Both or neither**, which is
    /// that module's own rule rather than a convenience here: a tab list with no
    /// talents under it draws parchment and no buttons. Absent, the talent panel
    /// answers `GetNumTalentTabs() == 0` and takes the reference's own
    /// "classes without talents" branch.
    talents: Option<crate::tables::talent::Talents>,
    /// `AreaTable.dbc` — where the character is, in the game's own words. See
    /// [`crate::tables::area`]. Optional: without it every zone name in the interface is
    /// empty, which is what the minimap's title bar showed before it was read.
    areas: Option<crate::tables::area::Areas>,
    /// The six chat channels and which of them a place is in — see
    /// [`crate::tables::channels`].
    chat_channels: Option<crate::tables::channels::ChatChannels>,
    /// `QuestSort.dbc` — the *other* table a quest log heading can come out of,
    /// chosen by `ZoneOrSort`'s sign. See [`crate::tables::questsort`]. Not optional in
    /// the same way as its neighbours: an absent file is an empty map, which
    /// leaves the handful of profession and "Epic" headings blank and every
    /// zone heading untouched.
    quest_sorts: crate::tables::questsort::QuestSorts,
    /// `Stationery` + `Package` + `MailTemplate` — the paper a letter is
    /// written on, and which of the five the send panel offers. See
    /// [`crate::tables::stationery`], which owns the rule that turns five rows
    /// into the one choice a fresh character has. Not optional in the same way
    /// as its neighbours: absent files are empty tables, and what that costs is
    /// a send panel whose Send button never lights, because a letter with no
    /// stationery is one the client itself refuses to build.
    mail: crate::tables::stationery::MailTables,
    /// `Resistances.dbc` — what a damage school is *called*, which is the last
    /// word of every combat log line that names one. See
    /// [`crate::tables::resistances`]. Not optional in its neighbours' sense:
    /// an absent file is an empty table, and what that costs is the
    /// `…SCHOOL…` half of the log falling back to the short key rather than
    /// drawing a sentence with a hole in it.
    resistances: crate::tables::resistances::Resistances,
    /// **The five tables a pet is made of** — `PetPersonality`,
    /// `CreatureFamily`, `ItemPetFood`, `PetLoyalty` and `StableSlotPrices`. See
    /// [`crate::tables::pet`]. Not optional in its neighbours' sense either:
    /// each of the five is absent-tolerant on its own, and the degradations are
    /// stated there — a happiness icon that never shows, a pet with no family
    /// name, an empty diet line, a blank loyalty label, and a stable slot
    /// priced at nothing.
    pet: crate::tables::pet::PetTables,
    /// `BankBagSlotPrices.dbc` — what the bank window's purchase button
    /// asks. See [`crate::tables::bank`]. Absent-tolerant on the same terms
    /// as the pet's tables: a slot priced at nothing.
    bank: crate::tables::bank::BankPrices,
    /// `WMOAreaTable.dbc` — …and where the character is when the ground under
    /// them is not the answer. See [`crate::tables::wmoarea`]. Optional: without it a
    /// city is whatever zone its dirt belongs to and no building has a name of
    /// its own, which is Ironforge reading "Dun Morogh".
    wmo_areas: Option<crate::tables::wmoarea::WmoAreas>,
    /// `WorldMapArea` + `WorldMapContinent` — see [`crate::tables::worldmap`]. Optional:
    /// without them the world map opens on the cosmic parchment with no zone
    /// under the pointer and no player arrow, which is where it started.
    world_map: Option<crate::tables::worldmap::WorldMap>,
    /// `AreaPOI.dbc` — the flags drawn on that parchment. Optional: without it
    /// `GetNumMapLandmarks` answers 0 and the map is bare of everything but a
    /// guard's own directions, which is where it started. See
    /// [`crate::tables::areapoi`].
    area_pois: Option<crate::tables::areapoi::AreaPois>,
    /// `Map.dbc`'s *localised* names, which is what the continent drop-down
    /// shows — a different column from the directory names
    /// [`map_directories`] reads.
    map_names: HashMap<u32, String>,
    /// **What kind of place each map is** — `Map.dbc` field 2, see [`MapKind`].
    ///
    /// Loaded beside the names because it comes from the same table and the same
    /// read; one consumer so far, [`Self::map_kind`].
    map_kinds: HashMap<u32, MapKind>,
    /// `ChrRaces.dbc` — **which model a race and a gender is, before there is a
    /// world.**
    ///
    /// Everywhere else in this client a unit's model arrives as
    /// `UNIT_FIELD_DISPLAYID` off the wire, so this table is not needed at all;
    /// character select is the one screen where nobody has sent one, because
    /// nothing has been logged into yet. See [`DisplayTables::race_display`].
    /// Optional on the usual terms: without it the plinth is empty, which is
    /// what character select was before it was read.
    races: Option<Indexed>,
    /// `PaperDollItemFrame` + `StringLookups` + `ItemClass` + `ItemSubClass` —
    /// everything about an item that is in a *file* rather than on the wire.
    /// See [`crate::tables::inventory`]. Never `None` as a whole: each of the four
    /// degrades on its own, and the icon directory has a stated fallback.
    item_tables: crate::tables::inventory::ItemTables,
    /// `DurabilityCosts` + `DurabilityQuality` — what a repair costs, which is
    /// the one price in the game the *client* works out. See [`crate::tables::repair`].
    repair: crate::tables::repair::RepairCosts,
    /// `TransportAnimation` — where an elevator, a lift or the tram is in its
    /// own cycle. Optional, and the absence is the behaviour this client had
    /// before it read one: every platform stands at its spawn point. See
    /// [`crate::tables::transport`].
    transports: crate::tables::transport::Transports,
    /// `ItemGroupSounds` — what an item sounds like changing hands. Optional
    /// like every other table below the three: without it items are silent.
    /// See [`crate::tables::itemsound`].
    item_sounds: crate::tables::itemsound::ItemGroupSounds,
    /// `PageTextMaterial` — what a sign or a book is written on, which is a
    /// name rather than a colour. See [`crate::tables::pagetext`].
    page_materials: crate::tables::pagetext::PageMaterials,
    /// `Lock` + `LockType` — what it takes to open a door, a chest or an ore
    /// vein, and therefore which pointer goes over one. See
    /// [`crate::tables::lock`] and [`crate::look::object`].
    locks: crate::tables::lock::Locks,
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
    /// baked texture name — 19 fields, and the last one is the only string.
    ///
    /// The seven appearance ids and the ten item ids are what the *bake* was
    /// computed from; the game ships the result, so this client needs only the
    /// name. (Race in field 1 and gender in field 2 are the cross-check: record
    /// 0 reads race 3 gender 0 — a dwarf male — and its model is
    /// `Character\Dwarf\Male\DwarfMale.mdx`.)
    pub const BAKE_NAME: usize = 18;
    /// …with the exception of the hairstyle and the beard, which are *geometry*
    /// and could not be baked into a texture. Those seven ids are therefore not
    /// only a record of the bake: they are the only statement of which geosets
    /// this NPC's head wears.
    pub const APPEARANCE: usize = 1;
    pub const APPEARANCE_COUNT: usize = 7;

    /// The ten `ItemDisplayInfo` ids, at 8..17.
    ///
    /// **The same distinction the appearance ids have, and for the same
    /// reason.** They are usually described as the record of what the bake was
    /// computed from, which is true of the *texture* half of a garment and false
    /// of the geometry half: a pauldron is a separate model and a bootleg is a
    /// geoset, and neither can be painted into a 256x256 body atlas. So a client
    /// that reads only the bake dresses an NPC in every texture it wears and
    /// none of its shapes — which is a Gadgetzan Bruiser with a correctly
    /// coloured leather vest, correctly coloured trousers, and no shoulders.
    pub const EQUIPPED_ITEM: usize = 8;
    pub const EQUIPPED_ITEM_COUNT: usize = 10;
}

impl DisplayTables {
    /// Read every table this needs out of `read`, which answers by **bare table
    /// name** — `"CreatureDisplayInfo"`, not a path — and returns `None` for a
    /// table the chain does not have.
    ///
    /// **A reader rather than a list of byte slices, because the list only ever
    /// grows.** This started as three tables and is eight; each addition used to
    /// be a signature change that every caller had to be edited for, and the
    /// arguments were eight values of one type in a fixed order — which is
    /// precisely the shape that produced the `CharSections` and `geosetGroup`
    /// field-index bugs one level down. Adding a table is now one line here and
    /// nothing anywhere else, and no caller can transpose two of them.
    ///
    /// Three tables are required, because without them no entity in the world
    /// resolves to a model at all. **Every other table is optional and its
    /// absence is a documented degradation** rather than an error: no
    /// `CreatureDisplayInfoExtra` is grey character-model NPCs, no
    /// `CharHairGeosets` is bald ones, no `ItemDisplayInfo` is players in their
    /// underwear, no `HelmetGeosetVisData` is hair drawn through a helm. Each of
    /// those is what this client did before that table was read at all.
    pub fn load(
        mut read: impl FnMut(&str) -> Option<Vec<u8>>,
    ) -> Result<DisplayTables, AssetError> {
        let creature_display = required(&mut read, "CreatureDisplayInfo")?;
        let creature_model = required(&mut read, "CreatureModelData")?;
        let object_display = required(&mut read, "GameObjectDisplayInfo")?;
        // **Read before the struct is built, because the map needs it**: the
        // zone list on a continent is sorted by the `AreaTable` name, so
        // `WorldMap::parse` takes the areas rather than looking them up later.
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
            // The first three or none: the row arithmetic that turns a
            // `LightParams` id into a band row is only checkable with both
            // tables in hand. The fourth degrades on its own — see
            // `LightTables::parse`.
            light: crate::tables::light::LightTables::parse(
                &read("Light").unwrap_or_default(),
                &read("LightParams").unwrap_or_default(),
                &read("LightIntBand").unwrap_or_default(),
                &read("LightFloatBand").unwrap_or_default(),
            ),
            // All three or none again, and for a sharper version of the same
            // reason: a chain with `SpellVisual` but no kits resolves every
            // spell to nothing, which is exactly what a correctly read table
            // for a game with no spell animations would look like.
            spells: crate::tables::spell::SpellVisuals::parse(
                &read("Spell").unwrap_or_default(),
                &read("SpellVisual").unwrap_or_default(),
                &read("SpellVisualKit").unwrap_or_default(),
                // The fourth is optional: without it a cast is posed and has
                // no models, which is a degradation and not a wrong answer.
                &read("SpellVisualEffectName").unwrap_or_default(),
                // …and so is the fifth, which is the only table in the chain
                // that describes a *shape* rather than naming a model: without
                // it the 192 spells that string a bolt between units draw
                // nothing, which is what this client did until it was read.
                &read("SpellChainEffects").unwrap_or_default(),
            ),
            // `Spell.dbc` is required *by this table* and the three it points
            // into are not — see `Spells::parse` for what each absence costs.
            spellbook: crate::tables::spellbook::Spells::parse(
                &read("Spell").unwrap_or_default(),
                &read("SpellCastTimes").unwrap_or_default(),
                &read("SpellRange").unwrap_or_default(),
                &read("SpellIcon").unwrap_or_default(),
                // …and the two the *description* points into, which degrade on
                // their own terms: without them a `$d` reads as no duration and
                // a `$a` as zero yards, so the sentence loses a number rather
                // than the panel losing a spell.
                &read("SpellDuration").unwrap_or_default(),
                &read("SpellRadius").unwrap_or_default(),
                // …and the one the *buff bar* points into: without it every
                // debuff border draws the "none" red, which is the colour the
                // interface uses for an undispellable one anyway — so the
                // absence costs the four coloured borders and nothing else.
                &read("SpellDispelType").unwrap_or_default(),
            )
            .ok(),
            // …and the one that says which *bar* those buttons are on, which
            // for a warrior or a druid is the difference between a full action
            // bar and an empty one. See `ShapeshiftForms`.
            shapeshift: crate::tables::spellbook::ShapeshiftForms::parse(
                &read("SpellShapeshiftForm").unwrap_or_default(),
            ),
            // …with `FactionGroup.dbc` attached, which turns the group *mask*
            // into the word `UnitFactionGroup` answers. Optional on its own —
            // see `Factions::with_groups`.
            factions: read("FactionTemplate")
                .and_then(|raw| crate::tables::faction::Factions::parse(&raw).ok())
                .map(|factions| factions.with_groups(&read("FactionGroup").unwrap_or_default())),
            // …and the *other* faction table, which answers a different
            // question about the same subject: a player's own standing rather
            // than a creature's reaction. Optional on its own — without it the
            // reputation panel is empty, which is what it was before this table
            // was read at all. See `tables::reputation`.
            reputation: read("Faction")
                .and_then(|raw| crate::tables::reputation::Factions::parse(&raw).ok()),
            // All four or none — see `TaxiTables::parse`, where the argument for
            // that is written out: a partial chain draws a working-looking
            // window with wrong routes in it.
            taxi: crate::tables::taxi::TaxiTables::parse(
                &read("TaxiNodes").unwrap_or_default(),
                &read("TaxiPath").unwrap_or_default(),
                &read("TaxiPathNode").unwrap_or_default(),
                &read("WorldMapContinent").unwrap_or_default(),
            ),
            // The first two both or neither — see `Skills::parse`, where the
            // argument for that is written out. `SpellIcon` is shared with the
            // spellbook above and degrades on its own terms (a tab with no art);
            // `SkillRaceClassInfo` degrades on its own too, into the over-tabbed
            // book this client drew before the gate was read.
            skills: crate::tables::skills::Skills::parse(
                &read("SkillLineAbility").unwrap_or_default(),
                &read("SkillLine").unwrap_or_default(),
                &read("SpellIcon").unwrap_or_default(),
                &read("SkillRaceClassInfo").unwrap_or_default(),
            )
            // …and the fifth, which only the skills *panel* needs: the eight
            // headings and the column they are ordered by. Optional on its own
            // terms — see `Skills::with_categories`.
            .map(|skills| skills.with_categories(&read("SkillLineCategory").unwrap_or_default())),
            // The same file again, for the columns the spellbook's reading
            // drops: the profession windows' thresholds.
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
            // …and the talent panel's pair, which shares nothing with the two
            // above but the shape of the question. Both or neither — see the
            // field, and `Talents::parse`, which is where the argument is.
            talents: crate::tables::talent::Talents::parse(
                &read("Talent").unwrap_or_default(),
                &read("TalentTab").unwrap_or_default(),
            ),
            races: read("ChrRaces").and_then(|raw| Indexed::parse(&raw).ok()),
            // Four tables, four independent degradations — see
            // `ItemTables::parse`. Read together because every one of them is
            // about the same subject and no consumer wants three of the four.
            item_tables: crate::tables::inventory::ItemTables::parse(
                &read("PaperDollItemFrame").unwrap_or_default(),
                &read("StringLookups").unwrap_or_default(),
                &read("ItemClass").unwrap_or_default(),
                &read("ItemSubClass").unwrap_or_default(),
            ),
            // Both or neither, and the absence is a *price* rather than a
            // panel: `GetRepairAllCost` quotes nothing and the armourer's
            // tooltip loses its money line. The repair itself is unaffected —
            // the server does that arithmetic again from the same two files.
            // Both or neither, and the absence is a *pointer* rather than a
            // panel — see `Locks::parse`. `LockType` carries only the names, so
            // losing it alone costs the tooltip's "Requires Mining" line and
            // leaves the pick on the cursor.
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
            // …and the one table in the chain that is about a thing that
            // *moves*. Optional on its own, and the degradation is exactly the
            // bug it fixes: no table, and every elevator in the game is drawn
            // and stood on at the position it was spawned at.
            transports: crate::tables::transport::Transports::parse(
                &read("TransportAnimation").unwrap_or_default(),
            ),
        })
    }

    /// **What it takes to open something** — see [`crate::tables::lock`].
    ///
    /// Not an `Option`, because an absent file is an empty table with a stated
    /// degradation rather than a missing answer: every game object reads as
    /// unlocked, which is the plain interact hand over an ore vein and nothing
    /// worse.
    /// `PageTextMaterial.dbc` — see [`crate::tables::pagetext`]. Empty without
    /// the file, which is every page drawn on parchment.
    pub fn page_materials(&self) -> &crate::tables::pagetext::PageMaterials {
        &self.page_materials
    }

    pub fn locks(&self) -> &crate::tables::lock::Locks {
        &self.locks
    }

    /// The four tables an item's *appearance in the interface* comes from —
    /// see [`crate::tables::inventory`].
    pub fn item_tables(&self) -> &crate::tables::inventory::ItemTables {
        &self.item_tables
    }

    /// …and what it costs to repair one — see [`crate::tables::repair`].
    pub fn repair(&self) -> &crate::tables::repair::RepairCosts {
        &self.repair
    }

    /// **Where a moving platform is in its cycle** — see
    /// [`crate::tables::transport`].
    ///
    /// Not an `Option`, on [`Self::locks`]' own terms: an absent file is an
    /// empty table, every game object answers "does not animate", and every
    /// elevator stands still — which is a stated degradation and is what this
    /// client did before the table was read at all.
    pub fn transports(&self) -> &crate::tables::transport::Transports {
        &self.transports
    }

    /// **An item's inventory icon, all the way from a display id to a path.**
    ///
    /// The join between the two tables that own the halves:
    /// `ItemDisplayInfo.dbc` states the bare name and `StringLookups.dbc` row 3
    /// states the folder. One door, because a caller that did the join itself
    /// would have to know that the folder is a table lookup — and the obvious
    /// guess (a literal `Interface\Icons\`) is right today and is not what the
    /// client does.
    /// **A bare icon name as a path**, through `StringLookups` row 3 — see
    /// [`crate::tables::inventory::InventoryTables::icon_path`]. Public for the
    /// one icon that does not come out of `ItemDisplayInfo`:
    /// [`crate::tables::inventory::coin_icon`]'s.
    pub fn icon_path(&self, icon_name: &str) -> Option<String> {
        self.item_tables.icon_path(icon_name)
    }

    pub fn item_icon(&self, display_id: u32) -> Option<String> {
        let name = self.items.as_ref()?.inventory_icon(display_id)?;
        self.item_tables.icon_path(&name)
    }

    pub fn item_sounds(&self) -> &crate::tables::itemsound::ItemGroupSounds {
        &self.item_sounds
    }

    /// **The `SoundEntries` id an item makes changing hands**, all the way from
    /// a display id — the same join the client makes, and one door for the same
    /// reason [`Self::item_icon`] is one: the caller would otherwise have to
    /// know that `ItemDisplayInfo` field 11 is a *row id* of a second table
    /// rather than a sound.
    ///
    /// `None` for a display id with no row, a row whose group is 0, or a group
    /// whose column is empty — all three of which are ordinary and mean
    /// silence.
    pub fn item_sound(
        &self,
        display_id: u32,
        which: crate::tables::itemsound::ItemSound,
    ) -> Option<u32> {
        let group = self.items.as_ref()?.group_sound(display_id)?;
        self.item_sounds.sound(group, which)
    }

    /// **Which `CreatureDisplayInfo` id a race and gender is** — the one lookup
    /// the character-select screen cannot do without and the world never needs.
    ///
    /// `gender` is the wire's byte: 0 male, 1 female, and anything else is read
    /// as male, which is what an unset byte means.
    ///
    /// Fields 4 and 5, **measured**, and the measurement is self-checking in the
    /// way this file's others are: record 0 is race 1 and reads 49 and 50, and
    /// `vale npc 49` resolves `Character\Human\Male\HumanMale.m2`; record 3 is
    /// race 4 and reads 55 and 56, which are the two night-elf models. Nothing
    /// but the right pair of columns produces that twice, and the surrounding
    /// columns are all small integers — field 3 is a sound id in the four
    /// thousands and field 7 is a float — so a wrong index here resolves to
    /// *nothing* rather than to the wrong body.
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

    /// **`CreatureDisplayInfo` field 11, `npcSound`** — the `NPCSounds.dbc`
    /// row a display greets and parts with. 0 for the many that say nothing.
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

    /// **`ChrRaces` field 12, `ResSicknessSpellID`** — the spell the spirit
    /// healer's warning is worded from, which is what `GetResSicknessDuration`
    /// starts at: the player's race row, `[row + 0x30]` (field
    /// 12), then that spell's `DurationIndex` into `SpellDuration.dbc`. 15007
    /// on all nine rows in 5875, and read rather than assumed for the same
    /// reason the display ids above are. See
    /// [`crate::tables::spellbook::Spells::duration_at`], which finishes the
    /// arithmetic.
    pub fn res_sickness_spell(&self, race: u8) -> Option<u32> {
        const RES_SICKNESS_SPELL_ID: usize = 12;
        let races = self.races.as_ref()?;
        let row = races.row(u32::from(race))?;
        races.dbc.u32_at(row, RES_SICKNESS_SPELL_ID).filter(|id| *id != 0)
    }

    /// **The three talent trees**, or `None` with either file missing — see
    /// [`crate::tables::talent`]. `None` is an empty talent panel and nothing
    /// worse: the addon has a branch for a class with no talents at all.
    pub fn talents(&self) -> Option<&crate::tables::talent::Talents> {
        self.talents.as_ref()
    }

    /// Which page of the spellbook a spell goes on — see [`crate::tables::skills`].
    /// `None` puts every spell in General, which is a shape rather than a wrong
    /// answer.
    /// The recipe thresholds, or `None` without `SkillLineAbility.dbc` —
    /// which draws both profession windows empty, the same degradation an
    /// absent `SkillLine.dbc` gives the skills panel.
    pub fn tradeskills(&self) -> Option<&crate::tables::tradeskill::TradeSkills> {
        self.tradeskills.as_ref()
    }

    /// A spell focus object's name — "Anvil", "Forge" — or `None`, which
    /// drops it from the Requires line rather than printing an id.
    pub fn spell_focus_name(&self, id: u32) -> Option<String> {
        self.spell_focus.get(&id).cloned()
    }

    pub fn skills(&self) -> Option<&crate::tables::skills::Skills> {
        self.skills.as_ref()
    }

    /// Where the character is standing, by area id — see [`crate::tables::area`].
    pub fn areas(&self) -> Option<&crate::tables::area::Areas> {
        self.areas.as_ref()
    }

    /// The six chat channels — see [`crate::tables::channels`]. Optional like
    /// every table but the three that resolve a model: without it no zone
    /// channel is joined and `/join` still works on custom ones.
    pub fn chat_channels(&self) -> Option<&crate::tables::channels::ChatChannels> {
        self.chat_channels.as_ref()
    }

    /// …and the other half of a quest log heading — see [`crate::tables::questsort`].
    pub fn quest_sorts(&self) -> &crate::tables::questsort::QuestSorts {
        &self.quest_sorts
    }

    /// …and the three tables the mail window reads — see
    /// [`crate::tables::stationery`].
    pub fn mail(&self) -> &crate::tables::stationery::MailTables {
        &self.mail
    }

    /// …and what a damage school is called — see
    /// [`crate::tables::resistances`].
    pub fn resistances(&self) -> &crate::tables::resistances::Resistances {
        &self.resistances
    }

    /// …and what a pet is: its happiness bands, its family and its diet — see
    /// [`crate::tables::pet`].
    pub fn pet(&self) -> &crate::tables::pet::PetTables {
        &self.pet
    }

    /// What a bank bag slot costs — see [`crate::tables::bank`].
    pub fn bank(&self) -> &crate::tables::bank::BankPrices {
        &self.bank
    }

    /// …and where they are when a *building* is the answer — see
    /// [`crate::tables::wmoarea`].
    pub fn wmo_areas(&self) -> Option<&crate::tables::wmoarea::WmoAreas> {
        self.wmo_areas.as_ref()
    }

    /// The world map's two tables — see [`crate::tables::worldmap`].
    /// **The world map's one non-table input** — see
    /// [`crate::tables::worldmap::WorldMap::load_zone_grids`], which reads
    /// `Interface\WorldMap\<continent>.zmp`.
    ///
    /// A second step rather than a second argument to [`Self::load`], because
    /// that reader answers by *table name* and this is an archive path — and
    /// because `Assets::read` takes `&mut self`, so a caller cannot hand out two
    /// closures over it. Skipping it is a documented degradation: the hover and
    /// the click fall back to the rectangle rule.
    pub fn load_zone_grids(&mut self, read: impl FnMut(&str) -> Option<Vec<u8>>) {
        let DisplayTables {
            world_map, areas, ..
        } = self;
        if let Some(map) = world_map.as_mut() {
            map.load_zone_grids(read, areas.as_ref());
        }
    }

    /// `AreaPOI.dbc`, for the landmark layer — see [`crate::tables::areapoi`].
    pub fn area_pois(&self) -> Option<&crate::tables::areapoi::AreaPois> {
        self.area_pois.as_ref()
    }

    pub fn world_map(&self) -> Option<&crate::tables::worldmap::WorldMap> {
        self.world_map.as_ref()
    }

    /// **What a continent is called** — `Map.dbc`'s localised name, which is
    /// what `GetMapContinents` answers. Empty for a map the table
    /// does not carry, which draws an unnamed drop-down entry rather than
    /// dropping the continent.
    pub fn map_name(&self, map: u32) -> &str {
        self.map_names.get(&map).map_or("", String::as_str)
    }

    /// **What kind of place a map is** — see [`MapKind`]. `None` for a map the
    /// table does not carry, which a caller should read as "not an instance"
    /// rather than as "unknown": the table is the only authority there is and a
    /// map missing from it is a map this client cannot be standing in.
    pub fn map_kind(&self, map: u32) -> Option<MapKind> {
        self.map_kinds.get(&map).copied()
    }

    /// **Every map `Map.dbc` names**, for a census that wants to walk them.
    ///
    /// One caller, `vale zones`, which crosses them with
    /// [`crate::tables::area::Areas::zone_of_map`]. In no order; the caller sorts.
    pub fn map_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.map_names.keys().copied()
    }

    /// What a spell *is* — see [`crate::tables::spellbook`]. `None` when the chain has
    /// no `Spell.dbc`, which costs every name, cost and cast time on the bar.
    pub fn spellbook(&self) -> Option<&crate::tables::spellbook::Spells> {
        self.spellbook.as_ref()
    }

    /// Which action bar a form puts on the screen — see
    /// [`crate::tables::spellbook::ShapeshiftForms`]. Never `None`: an absent table is
    /// the one that answers "the ordinary page" for everything.
    pub fn shapeshift(&self) -> &crate::tables::spellbook::ShapeshiftForms {
        &self.shapeshift
    }

    /// Friend or foe — see [`crate::tables::faction`]. `None` reads as neutral
    /// everywhere, which errs towards attackable.
    pub fn factions(&self) -> Option<&crate::tables::faction::Factions> {
        self.factions.as_ref()
    }

    /// `Faction.dbc`, for the reputation panel. `None` without the table, which
    /// draws an empty panel rather than a wrong one.
    pub fn reputation(&self) -> Option<&crate::tables::reputation::Factions> {
        self.reputation.as_ref()
    }

    /// The flight map's three tables — see [`crate::tables::taxi`]. `None` opens a
    /// flight master's window with no nodes on it.
    pub fn taxi(&self) -> Option<&crate::tables::taxi::TaxiTables> {
        self.taxi.as_ref()
    }

    /// **Which side a faction template is on** — `FactionTemplate.factionGroup`,
    /// the mask the taxi map's own filter compares against two literals. See
    /// [`crate::tables::taxi::Team::of_group`].
    pub fn faction_group(&self, template: u32) -> Option<u32> {
        self.factions.as_ref()?.group(template)
    }

    /// …and the same side **in words** — `UnitFactionGroup`'s two returns. See
    /// [`crate::tables::faction::Factions::group_name`], where the empty-name rule is.
    pub fn faction_group_name(&self, template: u32) -> Option<(&str, &str)> {
        self.factions.as_ref()?.group_name(template)
    }

    /// How one unit stands towards another, by faction template.
    ///
    /// Here rather than only on [`crate::tables::faction::Factions`] so that a caller
    /// holding the tables does not have to branch on the absence: with no table
    /// at all the answer is [`crate::tables::faction::Reaction::Neutral`], which is what
    /// the missing-template case already resolves to.
    pub fn reaction(
        &self,
        template: Option<u32>,
        towards: Option<u32>,
    ) -> crate::tables::faction::Reaction {
        self.template_rank(template, towards).into()
    }

    /// …and the same answer on the client's own eight-rank scale, for the two
    /// callers that compare it against a floor rather than folding it — see
    /// [`crate::look::cursor::can_interact`], whose gate is `>= Neutral` and
    /// therefore refuses Unfriendly, which the three-way fold cannot express.
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

    /// The colour and opacity of one liquid **standing at `at`** on one map, at
    /// `time` half-minutes past midnight. See [`crate::tables::light`] for why this is
    /// not in the texture.
    ///
    /// `None` means "this liquid is drawn from its own texture" — magma and
    /// slime always, since theirs are the only two flipbooks that carry colour.
    /// Water and ocean fall back to [`crate::tables::light::LiquidLight::UNLIT`], a
    /// conspicuous white, when the chain has no light tables at all.
    ///
    /// **`at` is where the *camera* is, and this note used to say the
    /// opposite.** The argument for the surface's own position was that a lake
    /// tinted by where it is looked at from changes colour as you walk around
    /// it, which is true and is the smaller of two evils. A liquid's two colours
    /// are bands of a `LightParams` row — the same row the fog, the sky and the
    /// sun come out of, which the renderer already resolves once a frame at the
    /// character — so per-surface was a second answer to a question the light
    /// chain answers once. And it does not divide the world evenly: a draw is
    /// *per ADT tile per kind*, so a lake crossing a tile boundary was two
    /// centres 533 yards apart, either side of a falloff sphere, and what that
    /// drew was a straight tint seam down the middle of the water.
    ///
    /// So it is asked once a frame now, from the camera's cell — see
    /// `crate::render::water` in `vale-client`, which is the only caller.
    /// `vale water` still passes a tile centre, because what it is reporting
    /// is what the water *there* looks like.
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

    /// **The same liquid as a shallow and a deep colour**, which is how the
    /// shipped minimaps were rendered — see
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
    /// `None` when the chain has no such table, which costs hair through a helm
    /// and nothing else.
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

    /// What a text emote says — see [`crate::tables::emotetext`].
    pub fn emote_texts(&self) -> Option<&crate::tables::emotetext::EmoteTexts> {
        self.emote_texts.as_ref()
    }

    /// What the animation about to be played implies about the weapons — see
    /// [`crate::look::sheath::reconcile`], which is the only caller.
    ///
    /// **Zero for an unknown id and for a chain with no table**, which is the
    /// same answer and deliberately so: zero is "this clip has no opinion", and
    /// most of the table's 208 rows carry it. So a missing `AnimationData.dbc`
    /// degrades to "no clip ever forces a change" rather than to a wrong one.
    pub fn weapon_flags(&self, anim_id: u16) -> u32 {
        self.animations
            .as_ref()
            .map_or(0, |table| table.weapon_flags(anim_id))
    }

    /// The animation table itself — the names, for a check that wants to report
    /// `Attack1H` rather than `17`.
    pub fn animations(&self) -> Option<&AnimationData> {
        self.animations.as_ref()
    }

    /// What the caster of a spell does — the wind-up held while its bar runs
    /// and the release at the end.
    ///
    /// `None` for a spell with no visual and for a chain with no spell tables,
    /// and the caller falls back to the generic cast animations in both cases.
    pub fn cast_animation(&self, spell_id: u32) -> Option<crate::tables::spell::CastAnimation> {
        self.spells.as_ref()?.cast(spell_id)
    }

    /// The models that spell's kits hang on its caster — the *visible* half of
    /// the same chain. Separate from the pose because either can exist without
    /// the other; see [`crate::tables::spell`].
    pub fn cast_effects(&self, spell_id: u32) -> Option<&crate::tables::spell::CastEffects> {
        self.spells.as_ref()?.effects(spell_id)
    }

    /// The projectile that spell throws — the third answer of the same chain,
    /// and the one that is not on the caster at all. See [`crate::tables::spell`].
    pub fn cast_missile(&self, spell_id: u32) -> Option<&crate::tables::spell::Missile> {
        self.spells.as_ref()?.missile(spell_id)
    }

    /// **The bolt that spell strings between units** - the fourth answer of the
    /// same chain, and the only one in this whole file that names no model:
    /// `SpellChainEffects` states a texture and six numbers and the client
    /// builds the geometry. See [`crate::tables::spell::SpellVisuals::chain`].
    pub fn cast_chain(&self, spell_id: u32) -> Option<&crate::tables::spell::ChainVisual> {
        self.spells.as_ref()?.chain(spell_id)
    }

    /// What a **persistent area** of that spell is drawn as — a Blizzard, a
    /// Flamestrike, a Rain of Fire — which is the one appearance in the game
    /// that is not reached through a display id. See
    /// [`crate::tables::spell::SpellVisuals::area`].
    pub fn spell_area(&self, spell_id: u32) -> Option<&crate::tables::spell::AreaEffect> {
        self.spells.as_ref()?.area(spell_id)
    }

    /// **The models a `SpellVisualKit` hangs, asked by kit id.**
    ///
    /// The one entry into this chain that does not begin at a spell, and the
    /// only one that can answer `SMSG_PLAY_SPELL_VISUAL` — whose body is a guid
    /// and a kit with no spell in it. See
    /// [`crate::tables::spell::SpellVisuals::kit`].
    pub fn kit_effects(&self, kit_id: u32) -> Option<&Vec<crate::tables::spell::KitEffect>> {
        self.spells.as_ref()?.kit(kit_id)
    }

    /// …and the pose that kit holds, which for kits 406 and 438 — food and
    /// drink — is the visible half.
    pub fn kit_pose(&self, kit_id: u32) -> Option<u16> {
        self.spells.as_ref()?.kit_pose(kit_id)
    }

    /// The models a unit wears while that spell's **aura** is on it — the
    /// `stateKit`, and the one answer in this chain that is not about a moment.
    /// See [`crate::tables::spell::SpellVisuals::state`].
    pub fn aura_effects(&self, spell_id: u32) -> Option<&Vec<crate::tables::spell::KitEffect>> {
        self.spells.as_ref()?.state(spell_id)
    }

    /// …and the **pose** the same kit states, which is the other column of the
    /// same row: what a stun looks like. See
    /// [`crate::tables::spell::SpellVisuals::aura_pose`].
    pub fn aura_pose(&self, spell_id: u32) -> Option<u16> {
        self.spells.as_ref()?.aura_pose(spell_id)
    }

    /// …and the **colour it paints the bearer's own model**, which is a third
    /// column of the same row: what Stoneform, Ghost and Shadowform look like.
    /// See [`crate::tables::spell::ModelTint`].
    pub fn aura_tint(&self, spell_id: u32) -> Option<crate::tables::spell::ModelTint> {
        self.spells.as_ref()?.aura_tint(spell_id)
    }

    /// Does that spell land on the unit that cast it? See
    /// [`crate::tables::spell::SpellVisuals::is_self_cast`] — asked only of a release
    /// whose hit list named nobody. `false` with no spell tables, which is the
    /// safe way round: an impact drawn on nobody costs nothing, and one drawn
    /// on the wrong unit is a burst on a mage's own chest.
    pub fn is_self_cast(&self, spell_id: u32) -> bool {
        self.spells
            .as_ref()
            .is_some_and(|s| s.is_self_cast(spell_id))
    }

    /// The spell tables themselves, for the survey that checks them.
    pub fn spells(&self) -> Option<&crate::tables::spell::SpellVisuals> {
        self.spells.as_ref()
    }

    /// Which geosets an appearance selects — the entry point for a *player*,
    /// whose appearance arrives on the wire rather than out of a DBC.
    pub fn character_geosets(&self, look: &Appearance) -> crate::world::m2::CharacterGeosets {
        self.char_geosets.geosets(look)
    }

    /// `ItemDisplayInfo.dbc`, if the chain had it — what a piece of equipment
    /// looks like, once the *server* has said which row an item entry means.
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
        // and are kept as such: their *position* is the texture type, so
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

        // An NPC wearing a character model has no texture variations at all —
        // its skin was *baked* offline from a race, a face, a hairstyle and ten
        // equipped items, and the result ships as one texture named only in
        // `CreatureDisplayInfoExtra`. It fills the same slot 0, because the M2
        // asks for it under texture type 1 where a creature asks under type 11.
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
            // Only an NPC in a character model has any: the columns live in the
            // same `CreatureDisplayInfoExtra` row the appearance does.
            equipment: match appearance {
                Some(_) => self.equipped_items(display_id),
                None => Vec::new(),
            },
        })
    }

    /// The appearance behind a display id, for the survey that checks the
    /// geosets against the models — a display id is the only thing that ties a
    /// race and gender to a model path in the data.
    pub fn appearance(&self, display_id: u32) -> Option<Appearance> {
        self.extra_appearance(self.creature_display.row(display_id)?)
    }

    /// What an NPC in a character model is *wearing*, as
    /// `(ItemDisplayInfo id, Slot)` pairs — the geometry half of its gear.
    ///
    /// The ten columns are in equipment-slot order, and the order is measured
    /// rather than transcribed: `vale npc` resolves every column of every row
    /// through `ItemDisplayInfo` and prints which garment each one turns out to
    /// name, from the component textures' own filenames. Column 2 is a shirt in
    /// every row that fills it, column 6 a boot, column 8 a glove, and so on
    /// down the standard list.
    ///
    /// **The textures are deliberately not returned.** They are already in the
    /// bake — that is what the bake *is* — so painting them again would be a
    /// second, unnecessary copy of work the game shipped precomputed, and on a
    /// model whose UVs already address the baked atlas it would change nothing.
    /// What is missing from a bake, and only ever missing, is the geometry: the
    /// attached models and the geosets.
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

    /// The slot each of the ten columns dresses, in order. Public so the survey
    /// can print the mapping it is checking rather than restating it.
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
    /// Zeros matter to the survey and only to the survey: dropping them first
    /// compacts the list and destroys the column index, which is the one thing
    /// the measurement is about.
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
    /// has one. Only the character-model NPCs do, which is what makes the
    /// presence of the row the test for "is this a character model" — and a
    /// better test than the path, which would have to be matched by name.
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

    /// The baked skin for one `CreatureDisplayInfo` *record*, or `None` if that
    /// row has no extended info — which is the common case, since every actual
    /// creature in the game is textured by its own model or its variations.
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

    /// The model for a game object's `DISPLAYID`. Chests, mailboxes, doors.
    ///
    /// **Some of them are `.wmo` files, not M2s** — the transports (zeppelins,
    /// boats, the Deeprun Tram) are buildings, and `GameObjectDisplayInfo` names
    /// them as such. `model_path` would append `.m2` to those and produce
    /// `…\transport_zeppelin.wmo.m2`, which is in no archive: a plausible path
    /// that resolves to nothing. Left alone instead, so a caller can tell what
    /// it is being given by the extension it kept.
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
            // A mailbox has no hairstyle, and nothing on its shoulders.
            character: None,
            appearance: None,
            equipment: Vec::new(),
        })
    }
}

/// One of [`DisplayTables::load`]'s three mandatory tables.
///
/// A missing one is [`AssetError::NotFound`] naming the path it would have had,
/// because that is the message that says what to go and look for — the reader
/// itself only ever answers `None`.
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
/// dense or sorted, so the map is built rather than the id used as an offset.
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

/// `Emotes.dbc` — the one hop between what the server says and an animation.
///
/// **`SMSG_EMOTE` is the only packet in the game that comes close to naming an
/// animation**, and this is the table that closes the gap: `u32 emoteId` in the
/// packet is an id here, and field 2 is an `AnimationData.dbc` id.
///
/// **Field 2 is measured, and the measurement is self-checking**, which matters
/// because the surrounding columns are all small integers and a wrong index
/// produces a plausible animation rather than a failure. The two tables name
/// their rows independently and the names agree: row 1 is `ONESHOT_TALK(DNR)`
/// and its field 2 is 60, which `AnimationData` calls `EmoteTalk`; row 2 is
/// `ONESHOT_BOW` and its field 2 is 66, `EmoteBow`. Nothing but the right column
/// produces that correspondence twice.
///
/// The remaining columns are `id`, the name string, then flags, a spec proc,
/// its parameter and a sound id — none of which this client reads.
pub struct Emotes {
    dbc: Dbc,
    /// Emote id -> `AnimationData` id, built once. 78 rows, so the map is as
    /// much about stating the shape as about speed.
    animation: HashMap<u32, u16>,
    /// The rows whose `EmoteType` is non-zero — the *held* emotes, which never
    /// cross the wire as a packet. See [`Emotes::EMOTE_TYPE`].
    states: std::collections::HashSet<u32>,
}

impl Emotes {
    /// Which `AnimationData.dbc` id each emote plays.
    ///
    /// Field 2, and **zero is not an answer**: `ONESHOT_NONE` is row 0 with
    /// animation 0, and animation 0 is `Stand`. An emote that resolved to Stand
    /// would interrupt whatever the character was doing to stand still, which is
    /// worse than ignoring it.
    const ANIMATION: usize = 2;

    /// `EmoteType`: **0 is a one-shot and anything else is a state**, and the
    /// distinction decides which half of the protocol carries the emote.
    ///
    /// `Unit::HandleEmote` branches on exactly this: a one-shot goes out as
    /// `SMSG_EMOTE` and a state is written into `UNIT_NPC_EMOTESTATE`, an
    /// update field that is simply *held*. So a client that reads only the
    /// packet animates `/wave` and not `/dance` — and not the innkeeper stuck
    /// permanently in `STATE_WORK` either, which is the more visible half.
    ///
    /// Measured rather than transcribed, and the data is unambiguous: every
    /// `ONESHOT_*` row reads 0 here and every `STATE_*` row reads 2.
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

    /// Whether this emote is a **held state** rather than a one-shot.
    pub fn is_state(&self, emote_id: u32) -> bool {
        self.states.contains(&emote_id)
    }

    pub fn animation(&self, emote_id: u32) -> Option<u16> {
        self.animation.get(&emote_id).copied()
    }

    /// How many rows the table has, and how many resolve to an animation —
    /// the pair `vale emote` reports.
    pub fn counts(&self) -> (usize, usize) {
        (self.dbc.record_count, self.animation.len())
    }

    /// Every row, as `(emote id, name, animation id)`. The name is field 1 and
    /// is what makes the animation column checkable by eye: `ONESHOT_BOW` had
    /// better resolve to something called `EmoteBow`.
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

/// `AnimationData.dbc` — the table every other animation in this project is
/// named by, and the one column that is a **rule** rather than a label.
///
/// 208 rows of seven columns: `id`, the name, `WeaponFlags`, `BodyFlags`, two
/// unidentified, and a fallback animation. This client reads the first three.
///
/// **Column 2 is the sheath policy** — see [`crate::look::sheath`], which is what
/// consumes it and where the bits are documented. The identification follows
/// the client's sheath reconcile; what makes it
/// checkable *here* is that the column's entire value set in 5875 is
/// `{0, 4, 16, 20, 32}`, which is exactly the three tested bits and their one
/// combination and is a shape no mis-indexed column would have. The names in
/// the row beside it close it: 17 is `Attack1H` and reads 32, 16 is
/// `AttackUnarmed` and reads 16, 42 is `Swim` and reads 4.
///
/// **Column 6 is the game's own missing-clip fallback**, and it is read here
/// because it costs nothing to keep the row — but nothing uses it yet.
/// `entities::fallbacks` is a hand-written table that agrees with it case after
/// case, and adopting this one is its own piece of work: the file contains a
/// genuine cycle (`Open -> Close -> Open`), and this client is deliberately more
/// generous in a few places. [`Self::fallback`] is what a check compares
/// against meanwhile.
pub struct AnimationData {
    /// Animation id -> `WeaponFlags`. Only the non-zero rows, since zero is
    /// both the default and the commonest value.
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
            // Zero is `Stand`, which the file uses as "no substitute stated"
            // rather than as an answer, and a row that falls back to itself is
            // not a chain.
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

    /// The sheath policy for an animation. **Zero for an unknown id**, which is
    /// the same as "no opinion" — see [`DisplayTables::weapon_flags`].
    pub fn weapon_flags(&self, id: u16) -> u32 {
        self.weapon_flags.get(&id).copied().unwrap_or(0)
    }

    pub fn name(&self, id: u16) -> Option<&str> {
        self.names.get(&id).map(String::as_str)
    }

    /// The substitute the **file** names for a model that lacks this clip, for
    /// a check that compares it against the compiled chains. Not used to resolve
    /// anything — see the type's own comment.
    pub fn fallback(&self, id: u16) -> Option<u16> {
        self.fallback.get(&id).copied()
    }

    /// How many rows carry each `WeaponFlags` value, ascending — the survey
    /// line that says the column has been read correctly. The set must be
    /// `{4, 16, 20, 32}` (zero is not stored) and nothing else.
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

/// Hand-built DBCs, for tests in *other* modules that need a [`DisplayTables`]
/// but are not about the tables themselves.
///
/// [`crate::look::dress`] is the one that needs it: the dressing rule is worth testing
/// without an archive, and it takes tables as its input. Keeping the builders
/// here rather than copied there means a change to a field layout breaks one
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
    /// Anything not listed is absent, which is what the reader's `None` means —
    /// so a test states only the tables it is about and the degradations for the
    /// rest are the ones [`DisplayTables::load`] documents.
    pub fn tables(named: &[(&str, Vec<u8>)]) -> Result<DisplayTables, AssetError> {
        DisplayTables::load(|table| {
            named
                .iter()
                .find(|(name, _)| *name == table)
                .map(|(_, raw)| raw.clone())
        })
    }

    /// The three mandatory tables, as empty-but-valid DBCs, for a test that is
    /// about one of the optional ones.
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

        // No `CreatureDisplayInfoExtra`, no geoset tables, no wardrobe and no
        // helmet table: bald, beardless and in its underwear, which is not what
        // this test is about.
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
        // The bake is named bare and lives in one shared directory — unlike a
        // texture variation, which lives beside its model.
        assert_eq!(
            resolved.skins[0],
            "Textures\\BakedNpcTextures\\973e54e79012eea3f2658f2897e681d9.blp"
        );
        assert_eq!(resolved.skins[1], "", "the other slots stay empty");
    }

    /// The bake must not displace a creature that already has a variation for
    /// slot 0 — the two texture types share the slot, so a wrong precedence
    /// would repaint a wolf with somebody's face.
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

    /// A chain whose table is *damaged* still resolves everything else — the
    /// same degradation as one that lacks it, which is the point: the two are
    /// indistinguishable to a caller and neither may be fatal.
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

    /// A transport is a building, and its row says so. Appending `.m2` to it
    /// yields `transport_zeppelin.wmo.m2` — a path in no archive, which reads as
    /// a broken model rather than as the wrong kind of file being asked for.
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
    /// This is the contract the reader replaced eight positional arguments to
    /// state. A caller cannot transpose two tables any more, but it *can* now
    /// forget one entirely — so the split has to be pinned rather than left to
    /// the doc comment.
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
        // …and the three alone are enough. Everything the optional tables
        // supply degrades: no bake, no hair, no wardrobe, no helmet rule.
        let bare = tables(&empty_required()).expect("the three are enough");
        assert!(bare.items().is_none(), "no ItemDisplayInfo was supplied");
        assert!(bare.helmet_visibility().is_none());
        // …including the one this file's newest reader wants: a chain with no
        // `ChrRaces` answers nothing, which is an empty plinth on character
        // select rather than a wrong body on it.
        assert_eq!(bare.race_display(1, 0), None);
    }

    /// **`ChrRaces` fields 4 and 5, with the real rows' own numbers in them.**
    ///
    /// The values are what 5875's table actually holds — human 49/50, night elf
    /// 55/56 — so the test states the measurement as well as the plumbing, and a
    /// transposed pair fails on the gender rather than silently drawing every
    /// character male.
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
                    // A race whose female column is zero — not a shape 1.12 has,
                    // and the one that must not resolve to a display id of 0.
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
        // The wire's gender byte is 0 or 1 and nothing else; anything else is
        // the unset byte and reads as male rather than as no character at all.
        assert_eq!(t.race_display(4, 7), Some(55));
        assert_eq!(t.race_display(9, 1), None, "a zero column is not an answer");
        assert_eq!(t.race_display(11, 0), None, "no such race in 1.12");
    }

    /// `AnimationData.dbc`'s three read columns, and the **absence** that
    /// matters more than any of them.
    ///
    /// A chain with no such table answers zero flags for every animation, which
    /// is the same answer an id with no policy gets — so the sheath reconcile
    /// degrades to "only combat and the server's byte move the weapons" rather
    /// than to a wrong rule. That is the degradation the type's doc claims, and
    /// it is exactly the sort of claim that is true when written and false two
    /// rounds later.
    #[test]
    fn the_animation_policy_degrades_to_no_opinion() {
        // id, name, WeaponFlags, _, _, _, fallback — three real 5875 rows.
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
        // …and without the table at all, every animation is silent.
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
