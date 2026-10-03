//! **What the world sounds like** — the sound half of the DBC chain, from the
//! id a table carries to the file an archive holds.
//!
//! Nothing about sound crosses the wire. The server never says "play this";
//! every sound in the game is the client's own decision, driven by tables in
//! the archives — which makes this module the same kind of thing as
//! [`crate::tables::light`]: a rulebook the renderer (here: the mixer) consults, with
//! no opinion about how a decoded buffer reaches a speaker.
//!
//! ## The hub: `SoundEntries.dbc`
//!
//! Every other table below answers with a **sound entry id**, and this table is
//! what an id means: a directory, up to ten candidate files with pick weights,
//! a volume, and the two distances. 4,623 rows.
//!
//! ```text
//! SoundEntries.dbc   4,623 rows x 29 fields
//!   [ 0]     id
//!   [ 1]     soundType        1 spells, 2 UI, … — kept raw, grouped by the census
//!   [ 2]     name             "Invisibility Impact", "gsTitleOptionOK", …
//!   [ 3..12] file[10]         "Dispel_Low_Base.wav"
//!   [13..22] freq[10]         pick weight per file, usually 1
//!   [23]     directoryBase    "Sound\Spells"
//!   [24]     volume           f32, 0..1
//!   [25]     flags
//!   [26]     minDistance      f32, 8.0 — inside it, full volume
//!   [27]     distanceCutoff   f32, 45.0 — beyond it, nothing
//!   [28]     EAXDef           unread (environmental reverb preset)
//! ```
//!
//! Measured on record 0 (id 3, "Invisibility Impact", one file, volume 1.0,
//! 8/45 yards) and record 1 (id 7, "BlizzardImpactVariations", six files).
//! **The name column is an API**: `PlaySound("gsTitleOptionOK")` in the game's
//! own GlueXML/FrameXML is a lookup into it — 99 of the `gs*`/`ig*` interface
//! names are rows here, which is what [`SoundBank::entry_named`] is for.
//!
//! ## The tables that point into it
//!
//! * **`AreaTable.dbc`** carries three sound columns this module reads and
//!   [`crate::tables::area`] deliberately does not: `[7]` ambience, `[8]` zone music,
//!   `[9]` intro music (columns pinned by the ids resolving in their own
//!   tables — Dun Morogh reads ambience 42, music 8, intro 0). `[5]`/`[6]` are
//!   the EAX provider preferences, unread.
//! * **`ZoneMusic.dbc`** (99 x 8): id, set name, then **day/night pairs** —
//!   silence-min ms `[2,3]`, silence-max ms `[4,5]`, sound entry `[6,7]`. Music
//!   is a track, then a silence drawn from `min..max`, then another track.
//! * **`SoundAmbience.dbc`** (68 x 3): id, day entry, night entry. Ambience
//!   loops; there is no silence interval.
//! * **`ZoneIntroMusicTable.dbc`** (43 x 5): id, name, sound entry, priority,
//!   min-delay minutes — the fanfare a zone plays on entry, at most once per
//!   delay window.
//! * **`CreatureSoundData.dbc`** (406 x 30): what a *unit* sounds like. Columns
//!   used here: `[1]` exertion (the attack grunt), `[3]` wound, `[4]` critical
//!   wound, `[5]` crushing wound, `[6]` death, `[9]` **footstep — not a sound
//!   entry id** but a key into `FootstepTerrainLookup`, `[10]` aggro, `[13]`
//!   alert, `[19..22]` the four **custom attacks** (a creature's natural
//!   weapon), `[25]` **`CreatureImpactType` — not a sound id either**, but what
//!   the unit is made of. Each passes its structural
//!   pin (`vale sound`: every nonzero value resolves in the table it
//!   should); **column 8, believed to be the stand sound, fails its pin at
//!   10/19 and is deliberately not read.** Reached from a
//!   display id: `CreatureDisplayInfo[2]` overrides, else
//!   `CreatureModelData[13]` (the model's own; pinned by Basilisk — display 1
//!   -> model 1 -> sound data 1, whose death entry 569 resolves).
//!
//!   **The whole row is pinned by name on one creature**, which is worth more
//!   than a range check on a column of small integers: the basilisk's are
//!   `BasiliskAttack` `[1]`, `BasiliskWound` `[3]`, `BasiliskWoundCritical`
//!   `[4]`, `BasiliskDeath` `[6]`, `BasiliskAggro` `[10]` and — the one that
//!   settles the custom-attack block — `BiteMedium` at `[19]`.
//! * **`FootstepTerrainLookup.dbc`** (179 x 5): (creatureFootstep `[1]`,
//!   terrain `[2]`) -> sound entry `[3]`, splash entry `[4]`. The terrain key
//!   comes from the texture layer under the foot — `MCLY`'s own `effectId`
//!   ([`crate::world::adt::TextureLayer`]) -> `GroundEffectTexture.dbc[6]` -> a
//!   **`TerrainType.dbc` row** -> that row's own field `[4]`.
//!   **`TerrainType` is a hop and not a glossary**, which this module read it
//!   as for a round: its row ids are 0-based and its footstep column is not, so
//!   using a row id as a key answers the terrain one along — grass as wood. See
//!   [`SoundBank::terrain_of_ground_effect`].
//! * **`WeaponSwingSounds2.dbc`** (6 x 4): (swing size `[1]` 0/1/2, crit `[2]`)
//!   -> the whoosh `[3]`. Read and censused; **nothing plays it** — see
//!   [`SoundBank::swing`], and [`COMBAT_MISS_1H`] for what a miss actually is.
//! * **`WeaponImpactSounds.dbc`** (30 x 23): **(weapon subclass `[1]`, metal
//!   `[2]`)**, then ten impact entries `[3..12]` and ten crit entries
//!   `[13..22]`. Both halves of that key matter — 11 subclasses carry two rows
//!   — and the ten columns are the ten things a blow can *meet*, named by the
//!   file: see [`impact_slot`].
//! * **`NPCSounds.dbc`** (156 x 5): id then hello/goodbye/pissed/ack, pointed
//!   at by `CreatureDisplayInfo`'s last column.
//! * **The spell chain** re-walked for its sound columns: `Spell[115]` ->
//!   `SpellVisual`, whose kits `[1..5]` each carry a sound at
//!   `SpellVisualKit[13]`, and whose missile block ends in its own sound at
//!   `SpellVisual[10]`. [`crate::tables::spell`] reads *past* both on purpose (they pin
//!   its model block); this module is where they are finally read.
//!
//! Every column above is validated structurally by `vale sound`: a column
//! believed to hold sound entry ids is checked against the id set, the footstep
//! column against the lookup's own key set, and every referenced file against
//! the archives. **Which sound plays when is a game rule and lives here; *when*
//! the client asks is the client's half** — see `crates/client/src/sound/`.
//!
//! ## Day against night
//!
//! The day/night pairs switch on the hour, and **the switch hours are an
//! interpretation, not a measurement**: dawn 6:00 and dusk 21:00, matching the
//! light table's own sunrise and sunset bands ([`crate::tables::light`]) so the
//! crickets start when the stars come out. The client's own switch is not
//! known; if it is found, it replaces [`is_night`].

use crate::tables::dbc::{dbc_path, Dbc};
use crate::AssetError;
use std::collections::HashMap;

/// Column indices, one block per table — see the module comment for how each
/// was pinned.
pub mod fields {
    pub mod entry {
        pub const KIND: usize = 1;
        pub const NAME: usize = 2;
        pub const FILE: usize = 3;
        pub const FILE_COUNT: usize = 10;
        pub const FREQ: usize = 13;
        pub const DIRECTORY: usize = 23;
        pub const VOLUME: usize = 24;
        pub const FLAGS: usize = 25;
        pub const MIN_DISTANCE: usize = 26;
        pub const CUTOFF: usize = 27;
    }
    pub mod area {
        pub const AMBIENCE: usize = 7;
        pub const ZONE_MUSIC: usize = 8;
        pub const INTRO_MUSIC: usize = 9;
    }
    pub mod zone_music {
        pub const SET_NAME: usize = 1;
        pub const SILENCE_MIN: usize = 2; // [2,3] day/night
        pub const SILENCE_MAX: usize = 4; // [4,5]
        pub const SOUND: usize = 6; // [6,7]
    }
    pub mod intro {
        pub const SOUND: usize = 2;
        pub const MIN_DELAY_MINUTES: usize = 4;
    }
    pub mod ambience {
        pub const SOUND: usize = 1; // [1,2] day/night
    }
    pub mod creature {
        pub const EXERTION: usize = 1;
        pub const WOUND: usize = 3;
        pub const WOUND_CRITICAL: usize = 4;
        /// `SoundInjuryCrushingBlowID` — the third injury column, for
        /// `HITINFO_CRUSHING`.
        pub const WOUND_CRUSHING: usize = 5;
        pub const DEATH: usize = 6;
        pub const FOOTSTEP: usize = 9;
        pub const AGGRO: usize = 10;
        /// `SoundAlertID` — the pre-aggro bark, `AI_REACTION_ALERT`'s.
        pub const ALERT: usize = 13;
        /// `CustomAttack[4]`, `[19..22]` — **the natural weapon's own impact**,
        /// played instead of a `WeaponImpactSounds` row by a creature that is
        /// biting rather than swinging. Pinned by name: the basilisk's first is
        /// entry 7374, `BiteMedium`.
        pub const CUSTOM_ATTACK: usize = 19;
        pub const CUSTOM_ATTACK_COUNT: usize = 4;
        /// `CreatureImpactType` — **what this unit is made of**, and therefore
        /// which of a weapon row's ten slots a blow on it lands in. Not a sound
        /// id: 0..2 over flesh, chain and plate. See [`super::super::impact_slot`].
        pub const IMPACT_TYPE: usize = 25;
        /// **The three pet-talk columns**, pinned twice over: by name — the
        /// only rows that state them are the four warlock demons, and they
        /// resolve to `A_IMP_KILL` / `A_IMP_ORDER` / `A_Imp_Dismiss` and their
        /// three siblings — and by the client. `SMSG_PET_ACTION_SOUND`'s
        /// handler routes talk 1 (attack) to `+0x6c` and talk 0
        /// (a special-spell order) to `+0x70`, and `SMSG_PET_DISMISS_SOUND`'s
        /// reads `+0x74` directly.
        pub const PET_ATTACK: usize = 27;
        pub const PET_ORDER: usize = 28;
        pub const PET_DISMISS: usize = 29;
    }
    pub mod footstep {
        pub const CREATURE_FOOTSTEP: usize = 1;
        pub const TERRAIN: usize = 2;
        pub const SOUND: usize = 3;
        pub const SPLASH: usize = 4;
    }
    pub mod display {
        /// `CreatureDisplayInfo`: the per-display override…
        pub const SOUND_OVERRIDE: usize = 2;
        pub const MODEL_ID: usize = 1;
        /// …and `CreatureModelData`: the model's own.
        pub const MODEL_SOUND: usize = 13;
    }
    pub mod swing {
        pub const SIZE: usize = 1;
        pub const CRIT: usize = 2;
        pub const SOUND: usize = 3;
    }
    pub mod impact {
        pub const SUBCLASS: usize = 1;
        /// **The second half of the key**, and the reason this table is not a
        /// map from subclass alone: 11 of the 21 subclasses have two rows here,
        /// one metal and one not. Pinned by name — subclass 4 is
        /// `Mace1H_ArmorFlesh` at 0 and `Mace1HMetal_ArmorFlesh` at 1.
        pub const METAL: usize = 2;
        pub const HIT: usize = 3; // [3..12]
        pub const CRIT: usize = 13; // [13..22]
        pub const COUNT: usize = 10;
    }
    pub mod ground {
        /// **A `TerrainType.dbc` *row id*, not a footstep key.** See
        /// [`super::super::SoundBank::terrain_of_ground_effect`], which is where
        /// the hop those two need between them is written up.
        pub const TERRAIN_TYPE: usize = 6;
    }
    /// `TerrainType.dbc`: 11 rows x 6 — id, the name, two spray effects, **the
    /// footstep key**, flags.
    pub mod terrain {
        /// Field 4. The column that makes `GroundEffectTexture` and
        /// `FootstepTerrainLookup` speak the same language.
        pub const SOUND: usize = 4;
    }
    pub mod spell {
        pub const VISUAL: usize = 115;
        /// `SpellVisual`'s five kit columns, [1..5]: precast, cast, impact,
        /// state, channel. Same indices as [`crate::tables::spell`]'s `fields`.
        pub const PRECAST_KIT: usize = 1;
        pub const CAST_KIT: usize = 2;
        pub const IMPACT_KIT: usize = 3;
        pub const CHANNEL_KIT: usize = 5;
        pub const MISSILE_SOUND: usize = 10;
        pub const KIT_SOUND: usize = 13;
    }
}

/// Day or night, as an index into the `[2]`-wide columns. See the module
/// comment: the switch hours are an interpretation.
pub fn is_night(hour: u32) -> bool {
    !(6..21).contains(&hour)
}

/// The day/night column index for an hour.
pub fn day_index(hour: u32) -> usize {
    usize::from(is_night(hour))
}

/// One `SoundEntries` row: what a sound entry id *is*.
#[derive(Debug, Clone, Default)]
pub struct SoundEntry {
    pub id: u32,
    /// `soundType`, kept raw — the census groups by it.
    pub kind: u32,
    pub name: String,
    /// The candidate files with their pick weights, empty slots dropped.
    pub files: Vec<(String, u32)>,
    pub directory: String,
    pub volume: f32,
    pub flags: u32,
    /// Inside this range the sound is at full volume…
    pub min_distance: f32,
    /// …and beyond this one it is not started at all.
    pub cutoff_distance: f32,
}

impl SoundEntry {
    /// The archive path of one candidate file.
    ///
    /// **The directory's own trailing separator is trimmed**, and that is a
    /// bug fix rather than tidiness: three of the shipped rows end
    /// `directoryBase` with a `\` — 1519's is `Sound\interface\` — and the
    /// naive join produced `Sound\interface\\igNewTaxiNodeDiscovered.wav`,
    /// which the archive does not have. The failure is silence, which for a
    /// sound is indistinguishable from not having asked, and `vale sound`
    /// counted all three among its missing files.
    pub fn path(&self, file: &str) -> String {
        let directory = self.directory.trim_end_matches(['\\', '/']);
        if directory.is_empty() {
            file.to_string()
        } else {
            format!("{directory}\\{file}")
        }
    }

    /// Pick a file, weighted by the freq column. `roll` is any external
    /// randomness — deterministic here so the pick is testable; a zero-weight
    /// set falls back to equal weights rather than picking nothing.
    pub fn pick(&self, roll: u32) -> Option<String> {
        if self.files.is_empty() {
            return None;
        }
        let total: u32 = self.files.iter().map(|(_, w)| *w).sum();
        if total == 0 {
            let (file, _) = &self.files[roll as usize % self.files.len()];
            return Some(self.path(file));
        }
        let mut at = roll % total;
        for (file, weight) in &self.files {
            if at < *weight {
                return Some(self.path(file));
            }
            at -= weight;
        }
        None
    }
}

/// A zone's three sound ids, off `AreaTable`'s own row — or off a
/// [`crate::tables::wmoarea`] row, which carries the same three columns for a building.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AreaSounds {
    /// A `SoundAmbience` id, 0 for none.
    pub ambience: u32,
    /// A `ZoneMusic` id, 0 for none.
    pub zone_music: u32,
    /// A `ZoneIntroMusicTable` id, 0 for none.
    pub intro_music: u32,
}

impl AreaSounds {
    /// **Fall back a column at a time, never a row at a time.**
    ///
    /// A zero in any of the three means "not stated here", and the answer is
    /// whatever the next place down states — so a subzone that names music and
    /// no ambience keeps its music *and* gets its zone's bed, which is the
    /// commonest shape in the table rather than an edge case. Measured over the
    /// shipped `AreaTable.dbc`: of the 495 rows that state anything at all,
    /// **316 state no ambience and 306 of those have one on their parent**, and
    /// 48 state no music with 34 answered by the parent. Falling back per *row*
    /// — which is what this client did until this was written — silences every
    /// one of them.
    ///
    /// The parent chain does not need walking, incidentally: all 1,081 rows are
    /// at depth 0 or 1 (122 roots, 959 children), so "the area, then its zone"
    /// **is** the whole chain in 1.12.
    #[must_use]
    pub fn or(self, fallback: AreaSounds) -> AreaSounds {
        AreaSounds {
            ambience: or_zero(self.ambience, fallback.ambience),
            zone_music: or_zero(self.zone_music, fallback.zone_music),
            intro_music: or_zero(self.intro_music, fallback.intro_music),
        }
    }
}

fn or_zero(value: u32, fallback: u32) -> u32 {
    if value == 0 { fallback } else { value }
}

/// One `ZoneMusic` row: the playlist rule for a zone.
#[derive(Debug, Clone, Default)]
pub struct ZoneMusic {
    pub set_name: String,
    /// Day/night silence between tracks, drawn from `min..=max`, milliseconds.
    pub silence_min: [u32; 2],
    pub silence_max: [u32; 2],
    /// Day/night sound entries.
    pub sounds: [u32; 2],
}

/// One `ZoneIntroMusicTable` row: the fanfare on entering a zone.
#[derive(Debug, Clone, Default)]
pub struct IntroMusic {
    pub name: String,
    pub sound: u32,
    /// At most once per this many minutes.
    pub min_delay_minutes: u32,
}

/// One `SoundAmbience` row: the loop under everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ambience {
    /// Day/night sound entries.
    pub sounds: [u32; 2],
}

/// The columns of `CreatureSoundData` this client acts on. All sound entry
/// ids except `footstep`, which keys `FootstepTerrainLookup`, and
/// `impact_type`, which is a material.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreatureSounds {
    pub exertion: u32,
    pub wound: u32,
    pub wound_critical: u32,
    pub wound_crushing: u32,
    pub death: u32,
    pub footstep: u32,
    pub aggro: u32,
    pub alert: u32,
    /// **What a creature's own attack sounds like**, `$AH0..3` — played
    /// *instead of* a weapon impact by anything that bites, claws or gores.
    /// Without it every creature in the game lands its blows in silence, since
    /// nothing with no item in its hand reaches `WeaponImpactSounds` at all.
    pub custom_attack: [u32; CUSTOM_ATTACKS],
    /// The victim's material, 0..2 — which of a weapon row's ten slots a blow
    /// on this unit lands in. See [`impact_slot`].
    pub impact_type: u32,
    /// **What the pet says when ordered to attack** — `SMSG_PET_ACTION_SOUND`
    /// talk 1, `A_IMP_KILL` and its three siblings. Stated by the four warlock
    /// demons and nothing else in 1.12.
    pub pet_attack: u32,
    /// …when ordered to cast a special spell — talk 0, `A_IMP_ORDER`.
    pub pet_order: u32,
    /// …and when dismissed — `SMSG_PET_DISMISS_SOUND`, `A_Imp_Dismiss`.
    pub pet_dismiss: u32,
}

/// How many `$AHn` columns a creature has.
pub const CUSTOM_ATTACKS: usize = fields::creature::CUSTOM_ATTACK_COUNT;

/// **The ten slots of a `WeaponImpactSounds` row, named by the file itself.**
///
/// Not an interpretation and not a guess: the ids in the Axe1H row resolve to
/// `Axe1H_ArmorFlesh`, `Axe1H_ArmorChain`, `Axe1H_ArmorPlate`, `Shield Metal
/// Impact`, `(DONOTRENAME)ShieldWoodImpact`, `1hParryMetalHitMetal`,
/// `1hParryMetalHitWood`, `Axe1H_HitWood`, `Axe1H_HitStone`, `Ethereal_1H`, in
/// that order. So the slot is **what the blow met**, and four of the ten are
/// the defended cases — which is why a parry and a block used to be silent
/// here: this client only ever read slot 0.
pub mod impact_slot {
    pub const ARMOR_FLESH: usize = 0;
    pub const ARMOR_CHAIN: usize = 1;
    pub const ARMOR_PLATE: usize = 2;
    /// A block on a metal shield…
    pub const SHIELD_METAL: usize = 3;
    /// …and on a wooden one.
    pub const SHIELD_WOOD: usize = 4;
    /// A parry, by what the *victim* parried with.
    pub const PARRY_METAL: usize = 5;
    pub const PARRY_WOOD: usize = 6;
    /// The two the world takes rather than a unit: a door, a tree, a wall.
    pub const HIT_WOOD: usize = 7;
    pub const HIT_STONE: usize = 8;
    pub const ETHEREAL: usize = 9;
    /// The whole set, for the census.
    pub const COUNT: usize = 10;
}

/// **The subclass a weaponless swing is keyed on.** `ITEM_SUBCLASS_WEAPON_FIST`
/// is 13, and row 13 of `WeaponImpactSounds` resolves to entry 1014,
/// `Unarmed_Generic` — so the fist row *is* the bare-hands row, which is what
/// makes a creature's or an unarmed player's blow audible at all.
pub const UNARMED_SUBCLASS: u32 = 13;

/// **The two whooshes the client caches by name**: it walks the loaded
/// `SoundEntries` rows comparing each name against six literals and keeping
/// the matching row's *id*. These are two of the six.
///
/// The choice they are used for is one flag: the 1H entry when it is set
/// and the 2H one when it is clear, played at volume 1.0. **There is no crit
/// variant and no third size**, which is what this client had before: it was choosing among
/// `WeaponSwingSounds2`' six light/medium/heavy × normal/critical rows on a
/// hand-written size table. That table is loaded by the real client too and its
/// own names confirm the reading, but it is not the miss whoosh, and where it
/// *is* played has not been pinned — see [`SoundBank::swing`].
pub const COMBAT_MISS_1H: &str = "(DONOTRENAME)Combat Miss 1H";
pub const COMBAT_MISS_2H: &str = "(DONOTRENAME)Combat Miss 2H";

/// …and a third of the six: **the noise a shield makes when it eats a blow.**
/// `HITINFO_ABSORB`'s own comment in vmangos' `UnitDefines.h` is "plays absorb
/// sound", and this is the sound it means — a positioned play of this one
/// entry and nothing else.
pub const ABSORB_GET_HIT: &str = "(DONOTRENAME)AbsorbGetHit";

/// **The footstep key for plain dirt — `TerrainType` row 0's own field 4.**
///
/// One rather than zero, and that is the whole point of naming it: zero is the
/// key `TerrainType`'s *"None"* row (10) carries, and it is a real key with real
/// sounds behind it rather than an absence. What a texture with no ground-effect
/// row should sound like is dirt, so the fallback has to be this and not the
/// zero that a `unwrap_or` reaches for.
pub const FOOTSTEP_KEY_DIRT: u32 = 1;

/// One `(creatureFootstep, terrain)` cell of `FootstepTerrainLookup`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Footstep {
    pub sound: u32,
    pub splash: u32,
}

/// One weapon subclass's row of `WeaponImpactSounds`.
#[derive(Debug, Clone, Copy, Default)]
pub struct WeaponImpact {
    pub hit: [u32; 10],
    pub crit: [u32; 10],
}

/// The sound columns of a spell's visual chain — each already resolved to a
/// sound entry id, so a cast asks one question.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CastSounds {
    /// The wind-up's kit sound, held while the cast bar runs.
    pub precast: u32,
    /// The release's.
    pub cast: u32,
    /// The impact kit's, at the victim.
    pub impact: u32,
    /// The channel kit's, held for the channel.
    pub channel: u32,
    /// The missile's own, from `SpellVisual` itself.
    pub missile: u32,
}

impl CastSounds {
    pub fn is_silent(&self) -> bool {
        self.precast == 0
            && self.cast == 0
            && self.impact == 0
            && self.channel == 0
            && self.missile == 0
    }
}

/// Every sound table, loaded and cross-linked. See the module comment.
#[derive(Debug, Default)]
pub struct SoundBank {
    entries: HashMap<u32, SoundEntry>,
    /// Lower-cased name -> id, for `PlaySound`'s lookup.
    by_name: HashMap<String, u32>,
    area_sounds: HashMap<u32, AreaSounds>,
    zone_music: HashMap<u32, ZoneMusic>,
    intro_music: HashMap<u32, IntroMusic>,
    ambience: HashMap<u32, Ambience>,
    creatures: HashMap<u32, CreatureSounds>,
    /// Display id -> (override sound data id, model id), and model id -> its
    /// own — the two halves of [`SoundBank::unit_sounds`].
    display_sound: HashMap<u32, (u32, u32)>,
    model_sound: HashMap<u32, u32>,
    footsteps: HashMap<(u32, u32), Footstep>,
    /// `GroundEffectTexture` id -> **`TerrainType` row id** (0..10).
    ground_terrain: HashMap<u32, u32>,
    /// …and `TerrainType` row id -> the **footstep key** the lookup is keyed on
    /// (its own field 4). The hop between the two, without which grass sounds
    /// like wood — see [`SoundBank::terrain_of_ground_effect`].
    terrain_footstep_key: HashMap<u32, u32>,
    /// (size 0/1/2, crit) -> the whoosh.
    swings: HashMap<(u32, bool), u32>,
    /// **(weapon subclass, metal) -> the impacts.** Two-part on purpose; see
    /// [`fields::impact::METAL`].
    impacts: HashMap<(u32, bool), WeaponImpact>,
    /// NPCSounds id -> hello/goodbye/pissed/ack.
    npc: HashMap<u32, [u32; 4]>,
    spell_sounds: HashMap<u32, CastSounds>,
    /// **`SpellVisualKit` id -> its own sound**, kept rather than consumed.
    ///
    /// The same keeping [`crate::tables::spell::SpellVisuals::kit`] does and for
    /// the same reason: `SMSG_PLAY_SPELL_VISUAL` carries a kit id and no spell,
    /// so the spell-keyed map above cannot answer it. Kit 406 reads sound 45
    /// and kit 438 reads 3373 — the two halves of eating and drinking.
    ///
    /// Silent kits are absent rather than present as zero, so a caller does not
    /// have to know that 0 means silence in this table.
    kit_sounds: HashMap<u32, u32>,
}

impl SoundBank {
    /// Read every table off `read`, which answers by **bare table name** — the
    /// same contract as `DisplayTables::load`, for the same reason.
    ///
    /// `SoundEntries` is required: without it no id in any other table means
    /// anything. **Every other table is optional and its absence is a
    /// documented degradation**: no `ZoneMusic` is a silent world, no
    /// `FootstepTerrainLookup` is silent feet, no `CreatureSoundData` is mute
    /// creatures — each of which is what this client was before this module.
    pub fn load(mut read: impl FnMut(&str) -> Option<Vec<u8>>) -> Result<SoundBank, AssetError> {
        let raw = read("SoundEntries")
            .ok_or_else(|| AssetError::NotFound(dbc_path("SoundEntries")))?;
        let dbc = Dbc::parse(&raw)?;
        let mut bank = SoundBank::default();
        for r in 0..dbc.record_count {
            let Some(id) = dbc.u32_at(r, 0) else { continue };
            let mut files = Vec::new();
            for i in 0..fields::entry::FILE_COUNT {
                let file = dbc.string_at(r, fields::entry::FILE + i).unwrap_or_default();
                if file.is_empty() {
                    continue;
                }
                let freq = dbc.u32_at(r, fields::entry::FREQ + i).unwrap_or(0);
                files.push((file, freq));
            }
            let entry = SoundEntry {
                id,
                kind: dbc.u32_at(r, fields::entry::KIND).unwrap_or(0),
                name: dbc.string_at(r, fields::entry::NAME).unwrap_or_default(),
                files,
                directory: dbc.string_at(r, fields::entry::DIRECTORY).unwrap_or_default(),
                volume: dbc.f32_at(r, fields::entry::VOLUME).unwrap_or(1.0),
                flags: dbc.u32_at(r, fields::entry::FLAGS).unwrap_or(0),
                min_distance: dbc.f32_at(r, fields::entry::MIN_DISTANCE).unwrap_or(8.0),
                cutoff_distance: dbc.f32_at(r, fields::entry::CUTOFF).unwrap_or(45.0),
            };
            if !entry.name.is_empty() {
                bank.by_name.insert(entry.name.to_ascii_lowercase(), id);
            }
            bank.entries.insert(id, entry);
        }

        if let Some(raw) = read("AreaTable") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    let sounds = AreaSounds {
                        ambience: dbc.u32_at(r, fields::area::AMBIENCE).unwrap_or(0),
                        zone_music: dbc.u32_at(r, fields::area::ZONE_MUSIC).unwrap_or(0),
                        intro_music: dbc.u32_at(r, fields::area::INTRO_MUSIC).unwrap_or(0),
                    };
                    if sounds != AreaSounds::default() {
                        bank.area_sounds.insert(id, sounds);
                    }
                }
            }
        }

        if let Some(raw) = read("ZoneMusic") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    let at = |base: usize, i: usize| dbc.u32_at(r, base + i).unwrap_or(0);
                    bank.zone_music.insert(
                        id,
                        ZoneMusic {
                            set_name: dbc
                                .string_at(r, fields::zone_music::SET_NAME)
                                .unwrap_or_default(),
                            silence_min: [
                                at(fields::zone_music::SILENCE_MIN, 0),
                                at(fields::zone_music::SILENCE_MIN, 1),
                            ],
                            silence_max: [
                                at(fields::zone_music::SILENCE_MAX, 0),
                                at(fields::zone_music::SILENCE_MAX, 1),
                            ],
                            sounds: [
                                at(fields::zone_music::SOUND, 0),
                                at(fields::zone_music::SOUND, 1),
                            ],
                        },
                    );
                }
            }
        }

        if let Some(raw) = read("ZoneIntroMusicTable") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    bank.intro_music.insert(
                        id,
                        IntroMusic {
                            name: dbc.string_at(r, 1).unwrap_or_default(),
                            sound: dbc.u32_at(r, fields::intro::SOUND).unwrap_or(0),
                            min_delay_minutes: dbc
                                .u32_at(r, fields::intro::MIN_DELAY_MINUTES)
                                .unwrap_or(0),
                        },
                    );
                }
            }
        }

        if let Some(raw) = read("SoundAmbience") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    bank.ambience.insert(
                        id,
                        Ambience {
                            sounds: [
                                dbc.u32_at(r, fields::ambience::SOUND).unwrap_or(0),
                                dbc.u32_at(r, fields::ambience::SOUND + 1).unwrap_or(0),
                            ],
                        },
                    );
                }
            }
        }

        if let Some(raw) = read("CreatureSoundData") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                use fields::creature::*;
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    let at = |f: usize| dbc.u32_at(r, f).unwrap_or(0);
                    let mut custom_attack = [0u32; CUSTOM_ATTACKS];
                    for (i, slot) in custom_attack.iter_mut().enumerate() {
                        *slot = at(CUSTOM_ATTACK + i);
                    }
                    bank.creatures.insert(
                        id,
                        CreatureSounds {
                            exertion: at(EXERTION),
                            wound: at(WOUND),
                            wound_critical: at(WOUND_CRITICAL),
                            wound_crushing: at(WOUND_CRUSHING),
                            death: at(DEATH),
                            footstep: at(FOOTSTEP),
                            aggro: at(AGGRO),
                            alert: at(ALERT),
                            custom_attack,
                            impact_type: at(IMPACT_TYPE),
                            pet_attack: at(PET_ATTACK),
                            pet_order: at(PET_ORDER),
                            pet_dismiss: at(PET_DISMISS),
                        },
                    );
                }
            }
        }

        if let Some(raw) = read("CreatureDisplayInfo") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    bank.display_sound.insert(
                        id,
                        (
                            dbc.u32_at(r, fields::display::SOUND_OVERRIDE).unwrap_or(0),
                            dbc.u32_at(r, fields::display::MODEL_ID).unwrap_or(0),
                        ),
                    );
                }
            }
        }

        if let Some(raw) = read("CreatureModelData") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    let sound = dbc.u32_at(r, fields::display::MODEL_SOUND).unwrap_or(0);
                    if sound != 0 {
                        bank.model_sound.insert(id, sound);
                    }
                }
            }
        }

        if let Some(raw) = read("FootstepTerrainLookup") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                use fields::footstep::*;
                for r in 0..dbc.record_count {
                    let creature = dbc.u32_at(r, CREATURE_FOOTSTEP).unwrap_or(0);
                    let terrain = dbc.u32_at(r, TERRAIN).unwrap_or(0);
                    bank.footsteps.insert(
                        (creature, terrain),
                        Footstep {
                            sound: dbc.u32_at(r, SOUND).unwrap_or(0),
                            splash: dbc.u32_at(r, SPLASH).unwrap_or(0),
                        },
                    );
                }
            }
        }

        // **`TerrainType` before `GroundEffectTexture`**, because the second is
        // resolved *through* the first — see
        // [`SoundBank::terrain_of_ground_effect`].
        if let Some(raw) = read("TerrainType") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    let key = dbc.u32_at(r, fields::terrain::SOUND).unwrap_or(0);
                    bank.terrain_footstep_key.insert(id, key);
                }
            }
        }

        if let Some(raw) = read("GroundEffectTexture") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    let terrain_type = dbc.u32_at(r, fields::ground::TERRAIN_TYPE).unwrap_or(0);
                    // **Row 0 of `TerrainType` is Dirt and is a real answer**, so
                    // the store keeps every texture that names a row — including
                    // 0 — and only a texture that names none is absent. The old
                    // `!= 0` filter here was reading a row id as though it were
                    // a "no sound" sentinel.
                    bank.ground_terrain.insert(id, terrain_type);
                }
            }
        }

        if let Some(raw) = read("WeaponSwingSounds2") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                use fields::swing::*;
                for r in 0..dbc.record_count {
                    let size = dbc.u32_at(r, SIZE).unwrap_or(0);
                    let crit = dbc.u32_at(r, CRIT).unwrap_or(0) != 0;
                    let sound = dbc.u32_at(r, SOUND).unwrap_or(0);
                    bank.swings.insert((size, crit), sound);
                }
            }
        }

        if let Some(raw) = read("WeaponImpactSounds") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                use fields::impact::*;
                for r in 0..dbc.record_count {
                    let subclass = dbc.u32_at(r, SUBCLASS).unwrap_or(0);
                    let metal = dbc.u32_at(r, METAL).unwrap_or(0) != 0;
                    let mut row = WeaponImpact::default();
                    for i in 0..COUNT {
                        row.hit[i] = dbc.u32_at(r, HIT + i).unwrap_or(0);
                        row.crit[i] = dbc.u32_at(r, CRIT + i).unwrap_or(0);
                    }
                    // **Keyed on both**, or 11 of the game's 21 weapon
                    // subclasses lose a row to whichever of their two the
                    // iteration reached last.
                    bank.impacts.insert((subclass, metal), row);
                }
            }
        }

        if let Some(raw) = read("NPCSounds") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    bank.npc.insert(
                        id,
                        [
                            dbc.u32_at(r, 1).unwrap_or(0),
                            dbc.u32_at(r, 2).unwrap_or(0),
                            dbc.u32_at(r, 3).unwrap_or(0),
                            dbc.u32_at(r, 4).unwrap_or(0),
                        ],
                    );
                }
            }
        }

        // The spell chain, re-walked for the columns `crate::tables::spell` reads
        // past. All three tables or nothing: a chain with kits but no spells
        // resolves every cast to silence indistinguishably from a game whose
        // spells are silent.
        if let (Some(spell), Some(visual), Some(kit)) =
            (read("Spell"), read("SpellVisual"), read("SpellVisualKit"))
        {
            if let (Ok(spell), Ok(visual), Ok(kit)) =
                (Dbc::parse(&spell), Dbc::parse(&visual), Dbc::parse(&kit))
            {
                use fields::spell::*;
                let mut kit_sound = HashMap::new();
                for r in 0..kit.record_count {
                    let Some(id) = kit.u32_at(r, 0) else { continue };
                    kit_sound.insert(id, kit.u32_at(r, KIT_SOUND).unwrap_or(0));
                }
                // **Kept as well as consumed**, for the two packets that name a
                // kit and no spell — see [`SoundBank::kit_sound`]. The silent
                // rows are dropped rather than stored as zeros: `kit_sound` is
                // built over every row because the loop below indexes it for a
                // kit column that may be anything, and the kept copy answers a
                // question where "no sound" and "no such kit" are the same
                // answer.
                bank.kit_sounds = kit_sound
                    .iter()
                    .filter(|(_, sound)| **sound != 0)
                    .map(|(kit, sound)| (*kit, *sound))
                    .collect();
                let mut visual_row = HashMap::new();
                for r in 0..visual.record_count {
                    let Some(id) = visual.u32_at(r, 0) else { continue };
                    let of_kit = |f: usize| {
                        let kit = visual.u32_at(r, f).unwrap_or(0);
                        kit_sound.get(&kit).copied().unwrap_or(0)
                    };
                    visual_row.insert(
                        id,
                        CastSounds {
                            precast: of_kit(PRECAST_KIT),
                            cast: of_kit(CAST_KIT),
                            impact: of_kit(IMPACT_KIT),
                            channel: of_kit(CHANNEL_KIT),
                            missile: visual.u32_at(r, MISSILE_SOUND).unwrap_or(0),
                        },
                    );
                }
                for r in 0..spell.record_count {
                    let Some(id) = spell.u32_at(r, 0) else { continue };
                    let visual_id = spell.u32_at(r, VISUAL).unwrap_or(0);
                    if let Some(sounds) = visual_row.get(&visual_id) {
                        if !sounds.is_silent() {
                            bank.spell_sounds.insert(id, *sounds);
                        }
                    }
                }
            }
        }

        Ok(bank)
    }

    pub fn entry(&self, id: u32) -> Option<&SoundEntry> {
        self.entries.get(&id)
    }

    /// `PlaySound`'s lookup: by the name column, case-insensitively — which is
    /// how `"igMainMenuOption"` and `"gsTitleOptionOK"` resolve.
    pub fn entry_named(&self, name: &str) -> Option<&SoundEntry> {
        self.entries.get(self.by_name.get(&name.to_ascii_lowercase())?)
    }

    /// A zone's ambience/music/intro ids. `None` for an area with none of the
    /// three, which most sub-areas are — the caller walks up to the zone.
    pub fn area_sounds(&self, area_id: u32) -> Option<&AreaSounds> {
        self.area_sounds.get(&area_id)
    }

    pub fn zone_music(&self, id: u32) -> Option<&ZoneMusic> {
        self.zone_music.get(&id)
    }

    pub fn intro_music(&self, id: u32) -> Option<&IntroMusic> {
        self.intro_music.get(&id)
    }

    pub fn ambience(&self, id: u32) -> Option<&Ambience> {
        self.ambience.get(&id)
    }

    /// What a unit sounds like, from its display id: the display's override
    /// when it names one, else the model's own.
    pub fn unit_sounds(&self, display_id: u32) -> Option<&CreatureSounds> {
        let (override_id, model_id) = self.display_sound.get(&display_id)?;
        let sound_data = if *override_id != 0 {
            *override_id
        } else {
            *self.model_sound.get(model_id)?
        };
        self.creatures.get(&sound_data)
    }

    /// …and the same row asked by **`CreatureModelData` id**, which is the only
    /// way in for `SMSG_PET_DISMISS_SOUND`: the packet carries a model-data id
    /// and a position, no guid and no display id. The reference walks the
    /// model's own sound column (`+0x34`, field 13) into `CreatureSoundData`
    /// and never consults a display's override.
    pub fn model_data_sounds(&self, model_id: u32) -> Option<&CreatureSounds> {
        self.creatures.get(self.model_sound.get(&model_id)?)
    }

    /// The footstep for a creature-footstep key on a terrain, falling back to
    /// [`FOOTSTEP_KEY_DIRT`] — the ground a texture with no ground effect
    /// reads as.
    pub fn footstep(&self, creature_footstep: u32, terrain: u32) -> Option<&Footstep> {
        self.footsteps
            .get(&(creature_footstep, terrain))
            .or_else(|| self.footsteps.get(&(creature_footstep, FOOTSTEP_KEY_DIRT)))
    }

    /// **`MCLY`'s `effectId` -> the footstep key, through `TerrainType`.**
    ///
    /// The hop in the middle is the whole of this function and it was missing:
    /// `GroundEffectTexture[6]` is a `TerrainType` **row id** and
    /// `FootstepTerrainLookup[2]` is that row's **own field 4**, and the two
    /// numberings are off by one because `TerrainType` is 0-based and its sound
    /// column is not:
    ///
    /// ```text
    /// row  name         field 4
    ///   0  Dirt            1
    ///   1  Metallic        2
    ///   2  Stone           3
    ///   3  Snow            4
    ///   4  Wood            5
    ///   5  Grass           6
    ///   6  Leaves          7
    ///   7  Sand            8
    ///   8  Soggy           9
    ///   9  DustyGrass      6      <- the same key as Grass
    ///  10  None            0
    /// ```
    ///
    /// So a grass texture (row 5) used as a lookup key directly resolves
    /// terrain 5, which is **Wood** — which is what a walk across Northshire
    /// sounded like. `DustyGrass` is the second proof: it is its own row and it
    /// deliberately shares Grass's key, which a straight-through reading cannot
    /// express at all (it would resolve Soggy).
    ///
    /// The structural check that let this stand is worth naming, because it is
    /// the failure mode this whole module is written against: both columns hold
    /// small integers in overlapping ranges, so "every value lands in the
    /// lookup's key set" passed at 1,827 of 1,830. **The three that missed were
    /// the tell** — they are exactly the `None` rows at id 10, which is one past
    /// the largest key the lookup has, and a column that can exceed its
    /// supposed key space is not that key space.
    ///
    /// Falls back to [`FOOTSTEP_KEY_DIRT`] for a texture with no ground-effect
    /// row and for a chain with no `TerrainType`.
    pub fn terrain_of_ground_effect(&self, ground_effect_id: u32) -> u32 {
        let Some(terrain_type) = self.ground_terrain.get(&ground_effect_id) else {
            return FOOTSTEP_KEY_DIRT;
        };
        self.terrain_footstep_key
            .get(terrain_type)
            .copied()
            .unwrap_or(FOOTSTEP_KEY_DIRT)
    }

    /// **The whoosh a swing that met nothing makes** — the two entries the
    /// client caches by name, chosen by handedness and nothing else. See
    /// [`COMBAT_MISS_1H`].
    pub fn miss_whoosh(&self, two_handed: bool) -> Option<u32> {
        let name = if two_handed {
            COMBAT_MISS_2H
        } else {
            COMBAT_MISS_1H
        };
        self.entry_named(name).map(|entry| entry.id)
    }

    /// `WeaponSwingSounds2`'s six rows: size 0/1/2 (light/medium/heavy) crossed
    /// with crit, and the table's own entry names say exactly that
    /// (`LightWeaponNormal`, `HeavyWeaponCritical`).
    ///
    /// **Nothing in this client plays it and that is deliberate.** The real
    /// client loads the table, so it is played by *something*, but the swing
    /// sounds this project has been able to pin (on the `$CSS` animation tag)
    /// are the two named
    /// whooshes above and not these. Kept read, kept censused, and left
    /// uncalled rather than being wired to a trigger that would be a guess.
    pub fn swing(&self, size: u32, crit: bool) -> Option<u32> {
        self.swings.get(&(size, crit)).copied().filter(|&s| s != 0)
    }

    /// **What a blow sounds like**: a weapon subclass and its material crossed
    /// with what the blow *met* — one of [`impact_slot`]'s ten — and whether it
    /// was critical.
    ///
    /// The material falls back to the other row rather than to nothing: four
    /// subclasses state only one of the two (bow, gun, the exotics and the fist
    /// row every unarmed blow uses), so a caller that insisted on an exact
    /// match would silence exactly the weapons with no choice to make.
    pub fn impact(&self, subclass: u32, metal: bool, slot: usize, crit: bool) -> Option<u32> {
        let row = self
            .impacts
            .get(&(subclass, metal))
            .or_else(|| self.impacts.get(&(subclass, !metal)))?;
        let column = if crit { &row.crit } else { &row.hit };
        let sound = *column.get(slot)?;
        (sound != 0).then_some(sound)
    }

    pub fn npc_sounds(&self, id: u32) -> Option<&[u32; 4]> {
        self.npc.get(&id)
    }

    /// A spell's five sound columns, resolved. `None` for a spell whose whole
    /// chain is silent — 0 is not an id in `SoundEntries`.
    pub fn cast_sounds(&self, spell_id: u32) -> Option<&CastSounds> {
        self.spell_sounds.get(&spell_id)
    }

    /// …and the same column asked by **kit** rather than by spell, which is the
    /// only way in for the two packets that carry one. See [`Self::kit_sounds`].
    pub fn kit_sound(&self, kit_id: u32) -> Option<u32> {
        self.kit_sounds.get(&kit_id).copied()
    }

    // --- census, for `vale sound` ---

    pub fn entries_iter(&self) -> impl Iterator<Item = &SoundEntry> {
        self.entries.values()
    }

    pub fn footsteps_iter(&self) -> impl Iterator<Item = (&(u32, u32), &Footstep)> {
        self.footsteps.iter()
    }

    pub fn creatures_iter(&self) -> impl Iterator<Item = (&u32, &CreatureSounds)> {
        self.creatures.iter()
    }

    /// Every display id `CreatureDisplayInfo` names, with the two halves of the
    /// join behind it: the display's own override and its model. The census
    /// crosses these with [`Self::unit_sounds`] to say **how many of the game's
    /// units have a voice at all**, which is the only number that says whether
    /// a silent fight is a missing table or a missing caller.
    pub fn displays_iter(&self) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
        self.display_sound
            .iter()
            .map(|(&display, &(over, model))| (display, over, model))
    }

    pub fn area_sounds_iter(&self) -> impl Iterator<Item = (&u32, &AreaSounds)> {
        self.area_sounds.iter()
    }

    pub fn zone_music_iter(&self) -> impl Iterator<Item = (&u32, &ZoneMusic)> {
        self.zone_music.iter()
    }

    pub fn ambience_iter(&self) -> impl Iterator<Item = (&u32, &Ambience)> {
        self.ambience.iter()
    }

    pub fn intro_music_iter(&self) -> impl Iterator<Item = (&u32, &IntroMusic)> {
        self.intro_music.iter()
    }

    pub fn ground_terrain_iter(&self) -> impl Iterator<Item = (&u32, &u32)> {
        self.ground_terrain.iter()
    }

    pub fn spell_sounds_iter(&self) -> impl Iterator<Item = (&u32, &CastSounds)> {
        self.spell_sounds.iter()
    }

    pub fn impacts_iter(&self) -> impl Iterator<Item = (&(u32, bool), &WeaponImpact)> {
        self.impacts.iter()
    }

    pub fn counts(&self) -> SoundCounts {
        SoundCounts {
            entries: self.entries.len(),
            named: self.by_name.len(),
            area_sounds: self.area_sounds.len(),
            zone_music: self.zone_music.len(),
            intro_music: self.intro_music.len(),
            ambience: self.ambience.len(),
            creatures: self.creatures.len(),
            footsteps: self.footsteps.len(),
            ground_terrain: self.ground_terrain.len(),
            spells: self.spell_sounds.len(),
        }
    }
}

/// The table sizes, for the census.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SoundCounts {
    pub entries: usize,
    pub named: usize,
    pub area_sounds: usize,
    pub zone_music: usize,
    pub intro_music: usize,
    pub ambience: usize,
    pub creatures: usize,
    pub footsteps: usize,
    pub ground_terrain: usize,
    pub spells: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// **A column at a time, not a row at a time.** The shape this exists for is
    /// a subzone that names its own music and no ambience: it must keep the
    /// music *and* pick up the bed from underneath it. Falling back per row
    /// answers one of the two and silences the other, which is 306 of the
    /// game's areas.
    #[test]
    fn a_missing_column_falls_through_while_the_others_stay() {
        let subzone = AreaSounds {
            ambience: 0,
            zone_music: 156,
            intro_music: 0,
        };
        let zone = AreaSounds {
            ambience: 35,
            zone_music: 1,
            intro_music: 61,
        };
        assert_eq!(
            subzone.or(zone),
            AreaSounds {
                ambience: 35,
                zone_music: 156,
                intro_music: 61,
            }
        );
        assert_eq!(zone.or(subzone), zone, "a stated column is never overridden");
        assert_eq!(
            AreaSounds::default().or(zone),
            zone,
            "…and a row that states nothing is the fallback outright"
        );
        assert_eq!(subzone.or(AreaSounds::default()), subzone);
    }

    /// A string block with each name at a known offset, returned beside it.
    fn strings(names: &[&str]) -> (Vec<u8>, Vec<u32>) {
        let mut block = vec![0u8]; // offset 0 is the empty string
        let mut offsets = Vec::new();
        for name in names {
            offsets.push(block.len() as u32);
            block.extend_from_slice(name.as_bytes());
            block.push(0);
        }
        (block, offsets)
    }

    fn bank_with(tables: Vec<(&str, Vec<u8>)>) -> SoundBank {
        SoundBank::load(|name| {
            tables
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, raw)| raw.clone())
        })
        .expect("SoundEntries present")
    }

    fn entries_table() -> Vec<u8> {
        let (block, off) = strings(&[
            "Invisibility Impact",
            "Dispel_Low_Base.wav",
            "Dispel_High.wav",
            "Sound\\Spells",
        ]);
        let mut row = vec![0u32; 29];
        row[0] = 3;
        row[fields::entry::KIND] = 1;
        row[fields::entry::NAME] = off[0];
        row[fields::entry::FILE] = off[1];
        row[fields::entry::FILE + 1] = off[2];
        row[fields::entry::FREQ] = 1;
        row[fields::entry::FREQ + 1] = 3;
        row[fields::entry::DIRECTORY] = off[3];
        row[fields::entry::VOLUME] = 1.0f32.to_bits();
        row[fields::entry::MIN_DISTANCE] = 8.0f32.to_bits();
        row[fields::entry::CUTOFF] = 45.0f32.to_bits();
        dbc(&[row], 29, &block)
    }

    /// The entry is the hub: files with weights, a directory that prefixes
    /// them, and a name that resolves case-insensitively — which is how the
    /// interface's `PlaySound("igMainMenuOption")` strings reach a row.
    #[test]
    fn an_entry_is_files_with_weights_under_a_directory() {
        let bank = bank_with(vec![("SoundEntries", entries_table())]);
        let entry = bank.entry(3).expect("row 3");
        assert_eq!(entry.name, "Invisibility Impact");
        assert_eq!(entry.files.len(), 2);
        assert_eq!(entry.volume, 1.0);
        assert_eq!(entry.min_distance, 8.0);
        assert_eq!(entry.cutoff_distance, 45.0);

        // Weighted pick: weights 1 and 3, so rolls 0 and 1.. split 1:3.
        assert_eq!(
            entry.pick(0).as_deref(),
            Some("Sound\\Spells\\Dispel_Low_Base.wav")
        );
        assert_eq!(
            entry.pick(1).as_deref(),
            Some("Sound\\Spells\\Dispel_High.wav")
        );
        assert_eq!(
            entry.pick(3).as_deref(),
            Some("Sound\\Spells\\Dispel_High.wav")
        );
        assert_eq!(entry.pick(4).as_deref(), entry.pick(0).as_deref());

        assert_eq!(bank.entry_named("INVISIBILITY impact").map(|e| e.id), Some(3));
        assert!(bank.entry_named("no such name").is_none());
    }

    /// **A directory column that ends in its own separator does not double
    /// it**, which three of the shipped rows need and which nothing would ever
    /// have reported: the join produced `Sound\interface\\igNew….wav`, the
    /// archive answered "no such file", and a sound that does not play is
    /// indistinguishable from one nobody asked for.
    ///
    /// Row 1519 is the real case — it is the sound `ERR_NEWTAXIPATH` names, so
    /// this is what stands between "New flight path discovered!" and its noise.
    #[test]
    fn a_directory_that_already_ends_in_a_separator_is_not_doubled() {
        let mut entry = SoundEntry {
            directory: r"Sound\interface\".to_string(),
            files: vec![("igNewTaxiNodeDiscovered.wav".to_string(), 1)],
            ..SoundEntry::default()
        };
        assert_eq!(
            entry.pick(0).as_deref(),
            Some(r"Sound\interface\igNewTaxiNodeDiscovered.wav")
        );
        // …and the ordinary row, which is every other one, is untouched.
        entry.directory = r"Sound\Spells".to_string();
        assert_eq!(
            entry.pick(0).as_deref(),
            Some(r"Sound\Spells\igNewTaxiNodeDiscovered.wav")
        );
        // A row with no directory at all is still the bare file name.
        entry.directory = String::new();
        assert_eq!(entry.pick(0).as_deref(), Some("igNewTaxiNodeDiscovered.wav"));
    }

    /// The display->sound resolution: the display's override wins, the model's
    /// own answers otherwise, and a display naming neither is mute.
    #[test]
    fn a_units_voice_is_the_displays_override_else_the_models_own() {
        let display = dbc(
            &[
                vec![1, 10, 0], // display 1 -> model 10, no override
                vec![2, 10, 77], // display 2 -> override 77
                vec![3, 11, 0], // display 3 -> model 11, which names nothing
            ],
            12,
            b"\0",
        );
        let mut model_row = vec![0u32; 16];
        model_row[0] = 10;
        model_row[fields::display::MODEL_SOUND] = 55;
        let mut mute_row = vec![0u32; 16];
        mute_row[0] = 11;
        let model = dbc(&[model_row, mute_row], 16, b"\0");
        let sounds = dbc(
            &[
                {
                    let mut r = vec![0u32; 30];
                    r[0] = 55;
                    r[fields::creature::DEATH] = 569;
                    r[fields::creature::FOOTSTEP] = 8;
                    r
                },
                {
                    let mut r = vec![0u32; 30];
                    r[0] = 77;
                    r[fields::creature::DEATH] = 700;
                    r
                },
            ],
            30,
            b"\0",
        );
        let bank = bank_with(vec![
            ("SoundEntries", entries_table()),
            ("CreatureDisplayInfo", display),
            ("CreatureModelData", model),
            ("CreatureSoundData", sounds),
        ]);
        assert_eq!(bank.unit_sounds(1).map(|s| s.death), Some(569));
        assert_eq!(bank.unit_sounds(1).map(|s| s.footstep), Some(8));
        assert_eq!(bank.unit_sounds(2).map(|s| s.death), Some(700));
        assert!(bank.unit_sounds(3).is_none(), "model 11 names no sound data");
        assert!(bank.unit_sounds(9).is_none(), "unknown display");
    }

    /// **The footstep join, and the hop in the middle of it**: a ground effect
    /// names a `TerrainType` *row*, the row names the *key*, and the key
    /// crosses the creature's own to reach a cell.
    ///
    /// The fixture keeps the shipped table's own shape — row ids 0-based, the
    /// footstep column one along, and `DustyGrass` deliberately sharing Grass's
    /// key — because that shape is the whole bug: reading a row id as a key
    /// answers the terrain next door, and for grass the terrain next door is
    /// wood.
    #[test]
    fn a_footstep_is_a_creature_key_crossed_with_a_terrain() {
        // (creature 8, key) -> sound. The keys are `TerrainType`'s field 4:
        // 1 dirt, 5 wood, 6 grass.
        let lookup = dbc(
            &[
                vec![21, 8, 6, 650, 1063],
                vec![22, 8, 1, 640, 1050],
                vec![23, 8, 5, 900, 1050],
            ],
            5,
            b"\0",
        );
        // id, name, sprayRun, sprayWalk, **footstep key**, flags — the shipped
        // rows for Dirt, Wood, Grass and DustyGrass.
        let terrain = dbc(
            &[
                vec![0, 0, 0, 0, 1, 0],
                vec![4, 0, 0, 0, 5, 0],
                vec![5, 0, 0, 0, 6, 0],
                vec![9, 0, 0, 0, 6, 0],
            ],
            6,
            b"\0",
        );
        let ground = dbc(
            &[
                vec![100, 0, 0, 0, 0, 0, 5], // a grass texture: TerrainType row 5
                vec![101, 0, 0, 0, 0, 0, 0], // a dirt texture: row 0
                vec![102, 0, 0, 0, 0, 0, 9], // dusty grass: row 9
            ],
            7,
            b"\0",
        );
        let bank = bank_with(vec![
            ("SoundEntries", entries_table()),
            ("FootstepTerrainLookup", lookup),
            ("TerrainType", terrain),
            ("GroundEffectTexture", ground),
        ]);
        // **The whole of the bug in one line**: grass is key 6, not row 5.
        assert_eq!(bank.terrain_of_ground_effect(100), 6);
        assert_eq!(
            bank.footstep(8, bank.terrain_of_ground_effect(100))
                .map(|f| f.sound),
            Some(650)
        );
        assert_eq!(
            bank.footstep(8, 5).map(|f| f.sound),
            Some(900),
            "key 5 really is wood, which is what grass used to resolve to"
        );
        // Row 0 is Dirt and is a real row, not a "no sound" sentinel.
        assert_eq!(bank.terrain_of_ground_effect(101), FOOTSTEP_KEY_DIRT);
        // DustyGrass is its own row and shares Grass's key — which a
        // straight-through reading cannot express at all.
        assert_eq!(bank.terrain_of_ground_effect(102), 6);
        // A texture with no ground-effect row at all is dirt.
        assert_eq!(bank.terrain_of_ground_effect(9999), FOOTSTEP_KEY_DIRT);
        assert_eq!(
            bank.footstep(8, 3).map(|f| f.sound),
            Some(640),
            "an uncovered terrain falls back to dirt"
        );
        assert!(bank.footstep(9, 6).is_none(), "unknown creature key");
    }

    /// The zone tables: the three AreaTable columns, the day/night pairs, and
    /// the hour rule that indexes them.
    #[test]
    fn a_zones_sound_is_three_columns_and_a_day_night_pair() {
        let mut area_row = vec![0u32; 25];
        area_row[0] = 1;
        area_row[fields::area::AMBIENCE] = 42;
        area_row[fields::area::ZONE_MUSIC] = 8;
        let area = dbc(&[area_row, vec![0u32; 25]], 25, b"\0");
        let (block, off) = strings(&["Zone-Forest"]);
        let mut music_row = vec![0u32; 8];
        music_row[0] = 8;
        music_row[fields::zone_music::SET_NAME] = off[0];
        music_row[fields::zone_music::SILENCE_MIN] = 180_000;
        music_row[fields::zone_music::SILENCE_MIN + 1] = 120_000;
        music_row[fields::zone_music::SILENCE_MAX] = 300_000;
        music_row[fields::zone_music::SILENCE_MAX + 1] = 240_000;
        music_row[fields::zone_music::SOUND] = 2523;
        music_row[fields::zone_music::SOUND + 1] = 2524;
        let music = dbc(&[music_row], 8, &block);
        let ambience = dbc(&[vec![42, 4162, 4204]], 3, b"\0");
        let bank = bank_with(vec![
            ("SoundEntries", entries_table()),
            ("AreaTable", area),
            ("ZoneMusic", music),
            ("SoundAmbience", ambience),
        ]);
        let sounds = bank.area_sounds(1).expect("area 1");
        assert_eq!(sounds.zone_music, 8);
        assert_eq!(sounds.ambience, 42);
        assert!(bank.area_sounds(0).is_none(), "an all-zero row is not kept");

        let music = bank.zone_music(8).expect("music 8");
        assert_eq!(music.sounds, [2523, 2524]);
        assert_eq!(music.silence_min, [180_000, 120_000]);
        let ambience = bank.ambience(42).expect("ambience 42");
        assert_eq!(ambience.sounds, [4162, 4204]);

        // Noon is day, midnight is night; dawn 6 and dusk 21 per the module
        // comment's stated interpretation.
        assert_eq!(day_index(12), 0);
        assert_eq!(day_index(0), 1);
        assert_eq!(day_index(6), 0);
        assert_eq!(day_index(21), 1);
    }

    /// The spell chain re-walked: each kit column resolved to its kit's sound,
    /// the missile's own kept, and an all-silent chain dropped.
    #[test]
    fn a_casts_sounds_are_the_kits_resolved() {
        let mut spell_row = vec![0u32; 120];
        spell_row[0] = 133;
        spell_row[fields::spell::VISUAL] = 55;
        let mut silent_row = vec![0u32; 120];
        silent_row[0] = 134;
        silent_row[fields::spell::VISUAL] = 56;
        let spell = dbc(&[spell_row, silent_row], 120, b"\0");
        let mut visual_row = vec![0u32; 12];
        visual_row[0] = 55;
        visual_row[fields::spell::PRECAST_KIT] = 1;
        visual_row[fields::spell::CAST_KIT] = 2;
        visual_row[fields::spell::MISSILE_SOUND] = 3011;
        let mut silent_visual = vec![0u32; 12];
        silent_visual[0] = 56;
        let visual = dbc(&[visual_row, silent_visual], 12, b"\0");
        let kits = dbc(
            &[
                {
                    let mut r = vec![0u32; 14];
                    r[0] = 1;
                    r[fields::spell::KIT_SOUND] = 1484;
                    r
                },
                {
                    let mut r = vec![0u32; 14];
                    r[0] = 2;
                    r[fields::spell::KIT_SOUND] = 1485;
                    r
                },
            ],
            14,
            b"\0",
        );
        let bank = bank_with(vec![
            ("SoundEntries", entries_table()),
            ("Spell", spell),
            ("SpellVisual", visual),
            ("SpellVisualKit", kits),
        ]);
        let sounds = bank.cast_sounds(133).expect("spell 133");
        assert_eq!(sounds.precast, 1484);
        assert_eq!(sounds.cast, 1485);
        assert_eq!(sounds.impact, 0);
        assert_eq!(sounds.missile, 3011);
        assert!(
            bank.cast_sounds(134).is_none(),
            "an all-silent chain is not kept"
        );
    }

    /// Without `SoundEntries` nothing means anything; with only it, every
    /// other lookup degrades to `None` rather than failing.
    #[test]
    fn sound_entries_is_required_and_everything_else_degrades() {
        assert!(SoundBank::load(|_| None).is_err());
        let bank = bank_with(vec![("SoundEntries", entries_table())]);
        assert!(bank.area_sounds(1).is_none());
        assert!(bank.unit_sounds(1).is_none());
        assert!(bank.footstep(8, 2).is_none());
        assert!(bank.cast_sounds(133).is_none());
        assert!(bank.swing(0, false).is_none());
        assert!(bank.impact(0, true, impact_slot::ARMOR_FLESH, false).is_none());
        // The whoosh is a *name* lookup, so it degrades with the entries
        // themselves rather than with a table of its own.
        assert!(bank.miss_whoosh(false).is_none());
    }

    /// **The impact table is keyed on two things, and the ten columns are ten
    /// materials.**
    ///
    /// The fixture is the shipped table's own shape: subclass 4 twice, once
    /// non-metal and once metal, with different sounds — which is what a key of
    /// subclass alone loses. The unarmed row (13) is there because it is the one
    /// every creature in the game and every bare-handed player lands on, and it
    /// states only the non-metal variant, which is why the lookup falls back to
    /// the other row rather than answering nothing.
    #[test]
    fn an_impact_is_a_weapon_a_material_and_what_it_met() {
        let row = |id: u32, subclass: u32, metal: u32, base: u32| {
            let mut r = vec![0u32; 23];
            r[0] = id;
            r[fields::impact::SUBCLASS] = subclass;
            r[fields::impact::METAL] = metal;
            for i in 0..fields::impact::COUNT {
                r[fields::impact::HIT + i] = base + i as u32;
                r[fields::impact::CRIT + i] = base + 100 + i as u32;
            }
            r
        };
        let impacts = dbc(
            &[
                row(5, 4, 0, 941),  // Mace1H
                row(14, 4, 1, 969), // Mace1HMetal
                row(13, 13, 0, 1014), // Unarmed_Generic — one row only
            ],
            23,
            b"\0",
        );
        let bank = bank_with(vec![
            ("SoundEntries", entries_table()),
            ("WeaponImpactSounds", impacts),
        ]);
        use impact_slot::*;
        // **The whole of the key bug in two lines**: one subclass, two rows.
        assert_eq!(bank.impact(4, false, ARMOR_FLESH, false), Some(941));
        assert_eq!(bank.impact(4, true, ARMOR_FLESH, false), Some(969));
        // The slot is what the blow met, and the defended ones are real slots.
        assert_eq!(bank.impact(4, false, ARMOR_PLATE, false), Some(943));
        assert_eq!(bank.impact(4, false, SHIELD_WOOD, false), Some(945));
        assert_eq!(bank.impact(4, false, PARRY_METAL, false), Some(946));
        assert_eq!(bank.impact(4, false, ARMOR_FLESH, true), Some(1041));
        // A subclass with one row answers it whichever material is asked for.
        assert_eq!(bank.impact(UNARMED_SUBCLASS, true, ARMOR_FLESH, false), Some(1014));
        assert_eq!(bank.impact(UNARMED_SUBCLASS, false, ARMOR_CHAIN, false), Some(1015));
        assert!(bank.impact(99, false, ARMOR_FLESH, false).is_none());
        assert!(
            bank.impact(4, false, COUNT, false).is_none(),
            "a slot past the ten is refused rather than wrapping"
        );
    }

    /// **The miss whoosh is two entries looked up by name**, which is the
    /// client's own mechanism (it caches them out of the loaded table)
    /// and not a row of `WeaponSwingSounds2`.
    #[test]
    fn a_miss_is_one_of_two_named_whooshes() {
        let (block, off) = strings(&[COMBAT_MISS_1H, COMBAT_MISS_2H, "MissWhoosh1Handed.wav"]);
        let named = |id: u32, name: u32| {
            let mut r = vec![0u32; 29];
            r[0] = id;
            r[fields::entry::NAME] = name;
            r[fields::entry::FILE] = off[2];
            r
        };
        let entries = dbc(&[named(7080, off[0]), named(7081, off[1])], 29, &block);
        let bank = bank_with(vec![("SoundEntries", entries)]);
        assert_eq!(bank.miss_whoosh(false), Some(7080));
        assert_eq!(bank.miss_whoosh(true), Some(7081));
    }
}
