//! How a spell looks on the caster: `Spell` -> `SpellVisual` ->
//! `SpellVisualKit` -> an `AnimationData` id.
//!
//! `SMSG_SPELL_START` and `SMSG_SPELL_GO` say who is casting which spell id and
//! nothing else, and every cast in the game has its own pose. The animation is
//! stated in three DBCs. A client that does not read them plays one wind-up and
//! one release for every spell; this client did, and a character opening a
//! chest raised both hands over its head.
//!
//! The chain is three hops, one field each:
//!
//! ```text
//!   Spell.dbc          [115] SpellVisualID
//!   SpellVisual.dbc    [1] precastKit  [2] castKit  [5] channelKit
//!   SpellVisualKit.dbc [2] animID       (-1, and 0, mean none)
//! ```
//!
//! Every index here is measured, and the measurement checks itself in the same
//! way as the `Emotes.dbc` animation column: the tables name their rows
//! independently, so a wrong column cannot match the spell's own name fifty
//! times. The results match what the retail client shows on screen:
//!
//! ```text
//!   Fireball        visual  67  precast  30 -> 51 ReadySpellDirected
//!                                cast    38 -> 53 SpellCastDirected
//!   Lesser Heal     visual 285  precast  99 -> 52 ReadySpellOmni
//!                                cast   270 -> 54 SpellCastOmni
//!   Opening         visual 180  precast 518 -> 123 UseStandingLoop
//!                                cast  4629 -> 129 UseStandingEnd
//!   Arcane Missiles visual 262  channel 729 -> 124 ChannelCastDirected
//!   First Aid       visual 100  cast    173 -> 123 UseStandingLoop
//! ```
//!
//! A spell thrown at a target is directed and one cast on the caster is omni.
//! Opening a chest and bandaging a wound are neither. This is why no character
//! model in the game carries the two ids `AnimationData` calls `SpellPrecast`
//! and `SpellCast`: the client never asks for them.
//!
//! Only the caster's animation is read. The `impactKit` animation is the pose
//! played on the victim (Fireball's is `CombatWound`), and this client already
//! plays a flinch on `SMSG_ATTACKERSTATEUPDATE`. The `impactKit` models are
//! read; see below.
//!
//! ## Kit model columns
//!
//! A pose alone is a character miming a fireball. The fireball itself comes
//! from the five model columns beside the animation. Each names a
//! `SpellVisualEffectName` row, a `.mdx` under `Spells\` or `Particles\` that
//! contains only emitters, and each hangs from a different attachment point on
//! the caster:
//!
//! ```text
//!   SpellVisualKit.dbc  [3] head  [4] chest  [5] base
//!                       [6] left hand  [7] right hand
//!   SpellVisualEffectName.dbc  [2] model path   [4] scale
//! ```
//!
//! The same five columns are read from the two kits that are not about the
//! caster. `impactKit` (field 3 of `SpellVisual`) hangs its models on the
//! victim when the spell lands. `stateKit` (field 4) hangs its models on a unit
//! for as long as the aura is on it. These two are most of the visible part of
//! the chain: 8,751 spells have impact models and 1,923 have state models,
//! against 898 that throw a missile. Most spells therefore land the moment they
//! are released, including every self-cast buff, where the caster is also the
//! victim and the impact is the only thing on screen.
//!
//! The columns are identified by what they resolve to. Each of the five holds
//! `-1`, `0`, or an id inside `SpellVisualEffectName`'s 782 rows. A sound id
//! (field 13, 1484 for Fireball) falls outside that range, which separates the
//! model block from the rest of the row. `vale spell` reports the share per
//! column, and the model names match their columns: Fireball's cast kit puts
//! `Fireball_Cast_Hand` on both hands and nothing anywhere else.
//!
//! ## `SpellVisual` fields 11 to 13: the persistent area
//!
//! Fields 11, 12 and 13 of `SpellVisual` describe a spell's persistent area,
//! which is a different subject from the kits. Field 12 is the area's model and
//! field 13 its kit; together they replace the fallback chain
//! [`SpellVisuals::area`] used before. See [`fields::AREA_FLAG`],
//! [`fields::AREA_MODEL`] and [`fields::AREA_KIT`]:
//!
//! ```text
//!   SpellVisual.dbc [11] hasAreaEffect   [12] SpellVisualEffectName  [13] SpellVisualKit
//! ```
//!
//! The 1.12.1 client reads all three together. For a dynamic object it takes
//! the spell from `DYNAMICOBJECT_SPELLID`, follows `Spell` -> `SpellVisual`,
//! and draws no area unless field 11 is non-zero. It then resolves field 12 in
//! `SpellVisualEffectName` and draws that model as the object, and resolves
//! field 13 in `SpellVisualKit`.
//!
//! The earlier fallback chain looked at the five caster kits, filtered to
//! attachment 19, and preferred the kit that describes a condition that holds.
//! It drew the wrong model for Flamestrike, and that report is why fields 11 to
//! 13 are read now. Flamestrike's `SpellVisual` (33) names
//! `Spells\Flamestrike_Impact_Base.mdx` at field 12: 454 vertices, a
//! lava-ground quad and four ribbons. The chain picked its `stateKit`'s
//! `Spells\Immolate_State_Base.m2` instead, which has zero vertices and two
//! emitters.
//!
//! The population checks the reading: 217 of 2,167 `SpellVisual` rows set field
//! 11, covering 722 spells, and all 217 field-12 ids resolve in
//! `SpellVisualEffectName`. A column read one along would not.
//!
//! ## The falling-impact procedural, `charProc` 9
//!
//! A Blizzard is not one model on the ground. `SpellVisualKit` has four
//! `charProc` slots (fields 15..18) with three parallel parameter columns after
//! them. `charProc == 9` is the falling-impact procedural: it spawns one
//! instance of a model per unit of an accumulator that ticks at a rate the kit
//! states.
//!
//! ```text
//!   SpellVisualKit.dbc [15..18] charProc  [19..22] charParamZero  [23..26] charParamOne
//!   charProc     9  -> the falling-impact procedural
//!   charParamZero   -> an index, held as a float, into a seven-model list the client holds
//!   charParamOne    -> impacts per second
//! ```
//!
//! The model list is [`AREA_RAIN_MODELS`], seven entries, and no file contains
//! it. The parameters match their columns: Blizzard's kit (609) is `charProc 9,
//! charParamZero 0.0, charParamOne 5.0`, which is index 0,
//! `Spells\Blizzard_Impact_Base.mdx`, at five per second. That model's declared
//! box is `[-12.5, -12.6, -5.0]..[11.9, 11.8, 30.7]`: one shard falling thirty
//! yards, not a field of snow. Eight kits in the shipped data carry the
//! procedural, used by Blizzard, Rain of Fire, Volley, Hurricane, Death &
//! Decay, Lightning Cloud, Starfall, Manastorm and Chill.
//!
//! Index 3 of the seven, `FlamestrikeSmall_Impact_Base`, is named by no shipped
//! kit. It stays in the list because the list's length is the client's, not
//! this project's.

use crate::tables::dbc::Dbc;
use crate::world::m2::{attach, model_path};
use std::collections::HashMap;

/// The value of a `SpellVisualEffectName` scale column, with zero read as
/// unset.
///
/// 197 of the table's 782 rows hold 0.0, Fireball's own precast and cast hands
/// among them. A renderer that multiplied by it would draw nothing for a
/// quarter of the game's spell effects. Zero means the column is unset, not a
/// size, and so does any value that is not finite; both give 1.0.
///
/// It is a function rather than a line inside the parser because two places
/// read the column: the chain below, and an editor that shows the number in the
/// file. Both must agree on what the file means; otherwise the editor shows
/// "x0.00" beside a model that is visible in the game.
pub fn effect_scale(raw: Option<f32>) -> f32 {
    raw.filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or(1.0)
}

/// One model a kit hangs on its caster.
///
/// The path is already through [`model_path`], so it is the `.m2` the archive
/// holds rather than the `.mdx` the table names.
#[derive(Debug, Clone, PartialEq)]
pub struct KitEffect {
    /// Which [`attach`] point on the caster.
    pub point: u32,
    pub path: String,
    /// `SpellVisualEffectName` field 4. Applied to the effect model and nothing
    /// else: it is the effect's size, not the caster's.
    pub scale: f32,
}

/// What a persistent area of a spell is drawn as: a Blizzard's patch of sky, a
/// Flamestrike's fire, a Consecration, a Rain of Fire.
///
/// The server puts one `DynamicObject` in the world per area. It states a
/// caster, a spell id and a radius; nothing on the wire says what it looks
/// like. Both parts of the answer come from the spell's own `SpellVisual` row.
/// The module comment describes the three fields and how they were identified.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaEffect {
    /// The model the object is drawn as, from `SpellVisual` field 12 through
    /// `SpellVisualEffectName`, already through [`model_path`].
    pub path: String,
    /// That row's field 4, as for a [`KitEffect`]'s scale.
    pub scale: f32,
    /// The second part, for the ten visuals that have one: a rain of impacts
    /// inside the area rather than one model standing in it.
    pub rain: Option<AreaRain>,
}

/// The falling-impact procedural, `charProc == 9`.
///
/// It is not one model with a long animation. The client keeps a fractional
/// accumulator per area, adds `dt * rate` to it every frame, and spawns one
/// instance of [`Self::path`] for each whole unit in it, each at its own place
/// inside the radius and on its own clock. Drawing the model once instead shows
/// a single snowflake landing at the start of a Blizzard and nothing after it.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaRain {
    /// One of [`AREA_RAIN_MODELS`], through [`model_path`].
    pub path: String,
    /// `charParamOne`, in impacts per second. 0.7 for a Lightning Cloud, 5.0
    /// for a Blizzard, 15.0 for Chill.
    pub rate: f32,
}

/// A bolt of lightning strung between two units: one row of
/// `SpellChainEffects.dbc`. It is the only table in this crate that describes a
/// shape rather than naming a model.
///
/// Chain Lightning, Chain Heal, Drain Life, Mind Flay, Health Funnel and the
/// Rallying Cry buffs all use it. No `.m2` is involved: the client builds the
/// geometry from these seven numbers, so a client that reads every other column
/// of every other spell table still draws nothing for these spells.
///
/// ```text
///   SpellChainEffects.dbc  [1] avgSegLen      [2] width       [3] noiseScale
///                          [4] texCoordScale  [5] segDuration [6] segDelay
///                          [7] texture
/// ```
///
/// The 1.12.1 client uses the first four numbers as they are and the fifth and
/// sixth as milliseconds; see [`Self::seg_duration_ms`].
#[derive(Debug, Clone, PartialEq)]
pub struct ChainEffect {
    /// `Textures\SpellChainEffects\Lightning.blp` and seven siblings, as the
    /// archive spells it. A texture, not a model: the client generates the
    /// strip it is wrapped around.
    pub texture: String,
    /// The length of one straight segment of the bolt, in yards: 2.78 on all
    /// eighteen rows. It sets how jagged the polyline is: a 30-yard cast at
    /// 2.78 has eleven kinks.
    pub avg_seg_len: f32,
    /// Half-width of the drawn strip, in yards. 0.5 for the lightning, 0.25 for
    /// the beams.
    pub width: f32,
    /// How far the kinks wander off the straight line, as a fraction of its
    /// length. 0.04 for the lightning and 0.001 for `HealBeam`, so the
    /// lightning crackles and the beam is a straight ribbon.
    pub noise_scale: f32,
    /// How often the texture repeats along the bolt. Negative on three rows,
    /// which flips it. Kept signed rather than clamped, because the sign is in
    /// the file.
    pub tex_coord_scale: f32,
    /// How long one hop of the chain lasts, in milliseconds: 1000 for the
    /// lightning, 2000 for `HealBeam`. The client adds it to the hop's start
    /// time, so it is a duration, not a rate.
    pub seg_duration_ms: u32,
    /// How long each hop waits after the one before it: 300 ms for the
    /// lightning. A hop starts at `now + hop_index * segDelay`, which makes a
    /// Chain Lightning pass from target to target instead of forking.
    pub seg_delay_ms: u32,
}

/// A [`ChainEffect`] and what one spell's kit says to do with it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainVisual {
    pub effect: ChainEffect,
    /// `charParamOne`, clamped to 3, the most the 1.12.1 client draws. Every
    /// shipped kit but one states 1.
    pub bolts: u32,
    /// `charParamTwo != 0`: the bolt is held rather than fired once.
    ///
    /// The flag is in the data; reading it as "held" is this project's
    /// interpretation, and the population supports it: 40 of the 48 kits that
    /// carry a chain set it, all 40 are a `channelKit`, and every kit that
    /// leaves it clear is a cast kit. Drain Life is the first kind and Chain
    /// Lightning the second.
    pub held: bool,
}

/// The largest number of bolts one chain may have. The 1.12.1 client draws no
/// more.
pub const MAX_CHAIN_BOLTS: u32 = 3;

/// The seven models the falling-impact procedural can rain, in the client's
/// order, indexed by `charParamZero`.
///
/// No file contains this list. `charParamZero` is a bare float index and the
/// model paths are held by the client, so a list in a different order draws the
/// wrong spell's impact, with no error, for the six reachable entries. It keeps
/// seven entries although no shipped kit names index 3, because the list's
/// length is the client's and not this project's.
///
/// The prefix on every `SpellVisualEffectName` row the engine reaches by name
/// rather than through `SpellVisual`; see [`SpellVisuals::hardcoded`].
pub const HARDCODED: &str = "HARDCODED ";

/// The row name of the sparkle on a lootable corpse. In 5875 it is
/// `Particles\LootFX.m2`: zero render batches, two textures (`Flare.blp` and
/// `Star5A.blp`), four particle emitters and one sequence, `3333..5733 ms`,
/// flagged to loop. It is made only of emitters and it loops rather than
/// playing once, so it is a state rather than a one-shot. Measured with `vale
/// model 'Particles\LootFX.m2'`.
pub const LOOT_ART: &str = "HARDCODED Loot Art";

pub const AREA_RAIN_MODELS: [&str; 7] = [
    "Spells\\Blizzard_Impact_Base.mdx",
    "Spells\\RainOfFire_Impact_Base.mdx",
    "Spells\\CallLightning_Impact.mdx",
    "Spells\\FlamestrikeSmall_Impact_Base.mdx",
    "Spells\\DeathAndDecay_Area_Base.mdx",
    "Spells\\ArcaneShot_Area.mdx",
    "Spells\\StarShards_Impact_Base.mdx",
];

/// A colour a kit draws its bearer's own model through: `charProc` 1.
///
/// This is the whole appearance of Stoneform, Ghost, Shadowform, Ice Block,
/// Banish, Petrify and eighty more auras: the unit's own model is drawn through
/// a flat colour for as long as the aura is on it. The five model columns of a
/// kit hang art on a unit; this changes the unit itself. No packet carries it.
///
/// ## Encoding
///
/// `charParamZero` is an `f32` whose value, truncated, is `0xRRGGBB`. An `f32`
/// mantissa holds any 24-bit integer exactly. The client forces the alpha
/// opaque; it is not in the data. With no colour on the unit the client uses
/// white, the identity of a multiply, which is why the colour multiplies the
/// model rather than adding to it. The population agrees: Stone Skin's 50% grey,
/// Shadowform's tenth and Banish's fifth all darken the model.
///
/// Eight shipped rows are named after their own colours, which pins the column:
///
/// ```text
///   Glowy (Red)     #ff0000     Glowy (Green)   #04f010
///   Glowy (Blue)    #2d32ff     Glowy (Yellow)  #f3ff0f
///   Glowy (Orange)  #ff9b06     Glowy (Purple)  #ff09ff
///   Glowy (Black)   #000000     Vertex Color: Light Blue  #aafbff
/// ```
///
/// Those eight are `charProc` 13, [`ModelFlash`], which uses the same encoding.
/// The spells agree with the reading too: Stone Skin `#787878` and Petrify
/// `#969696` are greys, Ghost `#8cb9fd` is pale blue, Immolate `#e75641` is
/// flame, Enrage `#ff1117` is red, Shadowform `#270042` and Banish `#350035` are
/// dark purple, and Stoneform is `#44465e`, a dark blue-grey. `vale spell`
/// decodes every row of both procedurals and each lands inside
/// `0x000000..0xffffff`.
///
/// ## What `charProc` 1 does not do
///
/// * It does not scale the model. Stoneform's kit carries only this
///   procedural, so the client colours a dwarf and does not resize him.
///   `charProc` 14, which an earlier reading took for a scale, is an opacity;
///   see [`KitProcedurals::opacity`].
/// * It reads no timing. The client reads `charParamZero` and nothing else, so
///   Stoneform's `0.2, 5.0` in the next two columns are unused data in the
///   shipped file. The timed colour is `charProc` 13.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelTint {
    /// `0xRRGGBB` as the row states it, unpacked, in the file's byte space.
    pub colour: [u8; 3],
    /// `charParamOne`, `charParamTwo` and `charParamThree` of the same slot.
    /// The client does not read them for `charProc` 1.
    ///
    /// They are kept because they show the slot was found: a colour declared
    /// in slot 2 whose parameters come back as slot 0's is a row read three
    /// columns off, and a test checks exactly that.
    pub params: [f32; 3],
}

/// Reads a `charParamZero` colour: an `f32` whose truncated value is
/// `0xRRGGBB`. A value outside 24 bits is dropped rather than masked into a
/// different colour.
fn packed_colour(value: Option<f32>) -> Option<[u8; 3]> {
    let packed = value
        .filter(|c| c.is_finite() && *c >= 0.0)
        .map(|c| c.trunc() as u32)
        .filter(|c| *c <= 0x00ff_ffff)?;
    Some([(packed >> 16) as u8, (packed >> 8) as u8, packed as u8])
}

/// The weapon trail a kit arms on its bearer: `charProc` 8.
///
/// The kit draws nothing when it plays. It stores a colour, an opacity and a
/// length on the unit, and the next animation the unit starts draws a strip
/// behind each weapon model it carries. A weapon model marks the strip's two
/// edges with its `$WTB` (bottom) and `$WTT` (top) events; see
/// [`crate::look::weapon_trail`]. 34 kits state one, and they are the cast
/// kits of the melee abilities: Charge, Mortal Strike, Cleave, Sinister
/// Strike, Hamstring, Whirlwind.
///
/// ```text
///   charParamZero   0xRRGGBB, as a float's value
///   charParamOne    not read (20 on most rows)
///   charParamTwo    how long the strip is laid down, in milliseconds
///   charParamThree  the strip's starting opacity, out of 255
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponTrail {
    pub colour: [u8; 3],
    pub alpha: u8,
    pub duration_ms: u32,
}

/// A timed colour on the bearer's own model: `charProc` 13.
///
/// The model is drawn through [`Self::colour`] for [`Self::hold_ms`] after the
/// kit plays, then eased back to white over [`Self::fade_ms`]. While it runs it
/// replaces any [`ModelTint`] an aura states; when it ends the aura's colour
/// returns. The clock starts when the kit plays and is not tied to the aura, so
/// a state kit's flash outlives an aura removed before it ends.
///
/// ```text
///   charParamZero   0xRRGGBB, as a float's value
///   charParamOne    hold, in seconds
///   charParamTwo    fade, in seconds
/// ```
///
/// Most of the 18 rows are impact kits holding half a second and fading over
/// half a second (Chain Bolt, Consume, Brain Freeze). The eight `Glowy
/// (<colour>)` test auras hold for 1e7 seconds or more, which is permanent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFlash {
    pub colour: [u8; 3],
    pub hold_ms: u32,
    pub fade_ms: u32,
}

impl ModelFlash {
    /// How much of the colour is on the model `elapsed_ms` after the kit
    /// played: 255 during the hold, falling to 0 across the fade, and `None`
    /// once the fade is over.
    ///
    /// The 1.12.1 client computes the level as `trunc((1 - t) * 255)` with
    /// `t` the fraction of the fade elapsed.
    pub fn level(&self, elapsed_ms: u64) -> Option<u8> {
        let hold = u64::from(self.hold_ms);
        if elapsed_ms < hold {
            return Some(255);
        }
        let into_fade = elapsed_ms - hold;
        let fade = u64::from(self.fade_ms);
        if into_fade >= fade {
            return None;
        }
        let t = into_fade as f64 / fade as f64;
        Some(((1.0 - t) * 255.0) as u8)
    }

    /// The colour at a `level` from [`Self::level`]: white at 0, the flash
    /// colour at 255, and between them the client's integer blend
    /// `white + ((colour - white) * level) >> 8` per channel.
    pub fn blend(&self, level: u8) -> [u8; 3] {
        match level {
            0 => [255; 3],
            255 => self.colour,
            _ => self.colour.map(|c| {
                let white = 255i32;
                (white + (((i32::from(c) - white) * i32::from(level)) >> 8)) as u8
            }),
        }
    }
}

/// The client-side procedurals one kit runs on the unit it plays on, other
/// than the aura colour ([`ModelTint`]), the chain ([`ChainVisual`]) and the
/// area rain ([`AreaRain`]), which are read into their own maps.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KitProcedurals {
    /// `charProc` 8.
    pub trail: Option<WeaponTrail>,
    /// `charProc` 13.
    pub flash: Option<ModelFlash>,
    /// `charProc` 14: a multiplier on the bearer's opacity, eased in over one
    /// second and held until the aura that played the kit is removed, then
    /// eased back over one second. Ghost reads 0.5, Vanish's state kit 0.3,
    /// Shadowform 0.65 and `Quest - Kodo Fade Out (DND)` 0.0. A value of 1.0
    /// or below zero is not stored, because the client does not store it.
    pub opacity: Option<f32>,
}

impl KitProcedurals {
    pub fn is_empty(&self) -> bool {
        self.trail.is_none() && self.flash.is_none() && self.opacity.is_none()
    }
}

/// [`KitProcedurals`] for each of a spell's five kits, by the moment the kit
/// plays.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpellProcedurals {
    /// Plays when the cast begins.
    pub precast: KitProcedurals,
    /// Plays when the cast is released.
    pub cast: KitProcedurals,
    /// Plays when a channel begins.
    pub channel: KitProcedurals,
    /// Plays on each unit the spell lands on.
    pub impact: KitProcedurals,
    /// Plays when the aura appears on a unit.
    pub state: KitProcedurals,
}

impl SpellProcedurals {
    pub fn is_empty(&self) -> bool {
        [self.precast, self.cast, self.channel, self.impact, self.state]
            .iter()
            .all(KitProcedurals::is_empty)
    }
}

/// The projectile a spell throws, when it throws one.
///
/// The missile is described by `SpellVisual` itself, one table before the kits.
/// A `SpellVisualKit` model hangs on the caster; a missile is a model of its
/// own that flies from the caster to the victim. Fireball states `1, 365, 0, 1,
/// 3011` in the five columns after the kits: it has a missile,
/// `Spells\Fireball_Missile_Low`, path type 0, destination attachment 1, and a
/// sound. Neither kit mentions it, so a client that reads only the kits throws
/// a spell with no fireball in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Missile {
    /// Through [`model_path`], so it is the `.m2` the archive holds.
    pub path: String,
    /// `SpellVisualEffectName` field 4, exactly as a kit model's is.
    pub scale: f32,
    /// `Spell.dbc` field 37, in yards per second. It is on the spell rather
    /// than on the visual, the one place this chain reads back from
    /// `Spell.dbc`. Fireball is 24.0, Frostbolt 28.0. Zero for a spell whose
    /// visual names a missile and whose row states no speed; the renderer
    /// treats that as instant, because otherwise the projectile never arrives.
    pub speed: f32,
    /// `SpellVisual` field 8. 0 for the ordinary straight line. This client
    /// does not distinguish the arcing variants and flies every missile
    /// straight. Kept so the count can show whether that matters.
    pub path_type: u32,
    /// `SpellVisual` field 9: the [`attach`] point on the victim the missile is
    /// aimed at. Read and reported; the renderer aims at the target's
    /// mid-height instead, because hitting the point itself needs the victim's
    /// posed skeleton, and the missile is not attached to it.
    pub destination: u32,
}

/// The two poses a spell puts its caster in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CastAnimation {
    /// Held for as long as the cast bar runs: `precastKit`, or the channel kit
    /// for a spell that has no wind-up of its own.
    ///
    /// A channelled spell has no precast kit and no cast kit. Arcane Missiles
    /// states only `channelKit`, so without the fallback every channel would
    /// stand still.
    pub hold: Option<u16>,
    /// The one-shot at the end — `castKit`.
    pub release: Option<u16>,
    /// The pose held while a channel runs: `channelKit`'s own `animID`, read
    /// separately from the fallback in [`Self::hold`].
    ///
    /// A channel has two held poses and they differ. Blizzard (10) states
    /// `precastKit 197` at `ReadySpellOmni` for the wind-up and `channelKit
    /// 717` at `ChannelCastOmni` for the channel. An earlier version read the
    /// channel kit only when there was no precast kit, which left the caster in
    /// the wind-up pose for the whole channel and never loaded the channel
    /// kit's models.
    ///
    /// Measured: 45 `SpellVisual` rows state both kits, and 41 of those give
    /// two different animations. Of the 323 spells `AttributesEx` marks as
    /// channelled, 63 have both, and 61 of them played the wrong pose. The
    /// other 82 rows state a channel kit and no precast kit, as Arcane Missiles
    /// does, which is why [`Self::hold`] keeps its fallback.
    pub channel: Option<u16>,
    /// Set for a ranged weapon attack, the one spell family whose release is
    /// not in any of these tables: its one-shot animation comes from the
    /// wielder's weapon rather than from the spell.
    ///
    /// It is read from `Spell.dbc`'s own `Attributes`
    /// ([`crate::tables::spellbook::SpellInfo::uses_ranged_slot`]), not from
    /// the visual chain. It is a field of the animation because it answers the
    /// same question as [`Self::release`], and a caller must not be able to get
    /// one without the other.
    ///
    /// Auto Shot (75) and the wand's Shoot (5019), the game's only two
    /// auto-repeating ranged attacks, both read `SpellVisual = 0`: no visual,
    /// so no kit and no `animID`. The chain says nothing about a hunter's
    /// commonest attack, and a client that reads only the chain draws an archer
    /// standing still. `world::entities::pose` resolves the animation from the
    /// drawn weapon.
    pub ranged_shot: bool,
}

impl CastAnimation {
    /// Whether the chain said anything at all about this spell. A spell with
    /// neither half falls back to the client's generic cast animations, which
    /// is what every spell used before this table was read.
    pub fn is_empty(&self) -> bool {
        self.hold.is_none() && self.release.is_none()
    }
}

/// `Spell.dbc` joined with `SpellVisual.dbc` and `SpellVisualKit.dbc`, reduced
/// to the one question the renderer asks.
///
/// The three raw tables are not kept. `Spell.dbc` alone is 22,360 records of
/// 173 fields, 15 MB of mostly gameplay rules this client does not use, and the
/// answer is two `u16`s per spell. The reduction happens once, at load.
pub struct SpellVisuals {
    /// Spell id -> what its caster does. Only the spells that resolve to at
    /// least one animation are in here.
    by_spell: HashMap<u32, CastAnimation>,
    /// Spell id -> the models its two kits hang on the caster.
    /// Spell id -> the models its two kits hang on the caster.
    ///
    /// Separate from [`Self::by_spell`] rather than a field on `CastAnimation`,
    /// because different columns answer the two and a spell can have either
    /// without the other: `Fireball`'s precast kit is a pose with no models,
    /// and many buff visuals are models with no pose. One map would make "no
    /// animation" also mean "no effect", and the effect would not be drawn.
    effects: HashMap<u32, CastEffects>,
    /// The visuals the engine spawns, by their own name; see
    /// [`Self::hardcoded`].
    hardcoded: HashMap<String, (String, f32)>,
    /// Spell id -> the projectile it throws. A third map for the same reason
    /// the second one exists: a spell can have a missile and no pose (a trap's
    /// bolt) or a pose and no missile (every heal in the game), so folding them
    /// together would make one absence mean the other.
    missiles: HashMap<u32, Missile>,
    /// Spell id -> the models a unit wears while this spell's aura is on it.
    ///
    /// A fourth map. It is not a field of [`CastEffects`] because it answers a
    /// different question: the other three describe an event, asked once per
    /// cast, and this describes a condition, asked of every unit in view
    /// against its `UNIT_FIELD_AURA` slots. A spell can state a state kit and
    /// no cast kit (most weapon enchants) or a cast kit and no state kit (every
    /// direct damage spell), so one map would make one absence mean the other.
    /// The effects are separate from [`CastAnimation`] for the same reason.
    states: HashMap<u32, Vec<KitEffect>>,
    /// Spell id -> the pose a unit holds for as long as this spell's aura is on
    /// it: the state kit's own `animID`, from the same row as the models above.
    ///
    /// This is the only source of a stun pose. `UNIT_FLAG_STUNNED` says a unit
    /// may not act and `MOVEFLAG_ROOT` says it may not move; neither is a pose,
    /// and neither affects the idle pose the 1.12.1 client chooses for a
    /// stopped unit (swimming -> `SwimIdle`, then `StealthStand`, then `Hover`,
    /// else `Stand`). Hammer of Justice's state kit (349) reads `animID = 14`,
    /// `Stun`, beside the `StunSwirl_State_Head` model this client already
    /// drew, so the cower and the stars over the head are two columns of one
    /// row.
    ///
    /// Separate from [`Self::states`] for the reason [`Self::effects`] is
    /// separate from [`Self::by_spell`]: a kit can state a pose with no models
    /// (a sit, a kneel) or models with no pose (every weapon enchant), and one
    /// map would make one absence mean the other.
    state_anim: HashMap<u32, u16>,
    /// Spell id -> the colour a unit's own model is drawn through while this
    /// spell's aura is on it, from the same kit's `charProc` slot. A seventh
    /// map, for the same reason as the six around it: a kit can state a colour
    /// and no models (Stoneform's does) or models and no colour (most of them),
    /// so one map would make one absence mean the other. See [`ModelTint`].
    state_tint: HashMap<u32, ModelTint>,
    /// Spell id -> what its persistent area is drawn as, for the 722 spells
    /// that have one.
    ///
    /// A fifth map, for the same reason as the four above: three columns none
    /// of the others read answer it, and it is asked of a `DynamicObject`
    /// rather than of a cast or an aura slot. A spell can state an area and no
    /// kits at all (a summoned cloud), and every direct-damage spell states
    /// kits and no area.
    areas: HashMap<u32, AreaEffect>,
    /// Spell id -> the bolt it strings between units, for the 192 spells that
    /// have one.
    ///
    /// An eighth map, for the same reason as the seven above, and the clearest
    /// case of it: it comes from a table (`SpellChainEffects.dbc`) no other map
    /// uses, through a `charProc` slot no other map reads, and it names no
    /// model. Chain Heal states a cast kit whose only content is a chain, and
    /// 22,000 spells state other visuals and no chain.
    chains: HashMap<u32, ChainVisual>,
    /// `SpellVisualKit` id -> the models it hangs, and the pose it holds. Every
    /// spell-keyed map here is built from these two, which are kept after
    /// parsing.
    ///
    /// Two packets name a kit rather than a spell: `SMSG_PLAY_SPELL_VISUAL` and
    /// `SMSG_PLAY_SPELL_IMPACT`, whose body is a guid and a kit id. The
    /// spell-keyed maps cannot answer them. These two were earlier locals
    /// inside [`Self::parse`], dropped once the spell-keyed maps were filled,
    /// and those packets had no lookup.
    ///
    /// They are the whole answer for those two packets, and it is used often:
    /// kits 406 and 438 are eating and drinking, and vmangos sends one of them
    /// on every regeneration tick while a character sits with food.
    /// [`crate::tables::sound::SoundBank::kit_sound`] keeps the kit-keyed sound
    /// data for the same reason.
    kits: HashMap<u32, Vec<KitEffect>>,
    kit_poses: HashMap<u32, u16>,
    /// Kit id -> its [`KitProcedurals`], for the kits that run at least one.
    kit_procedurals: HashMap<u32, KitProcedurals>,
    /// Spell id -> the procedurals of its five kits, for the spells where any
    /// kit runs one.
    procedurals: HashMap<u32, SpellProcedurals>,
    /// The spells with at least one effect that targets the caster.
    ///
    /// A set rather than a map because the question is yes or no, and it is
    /// asked only of a release whose hit list named nobody; see
    /// [`fields::EFFECT_TARGET_A`].
    self_cast: std::collections::HashSet<u32>,
    /// How many spells the table has. `vale spell` reports it beside how many
    /// resolved, and the pair shows whether a column moved.
    spells: usize,
}

/// The models a spell's two kits hang on its caster.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CastEffects {
    /// Shown while the cast bar runs — the precast kit's, or the channel kit's
    /// for a spell that has no wind-up of its own.
    pub hold: Vec<KitEffect>,
    /// Shown at the release, and for as long as the release lasts.
    pub release: Vec<KitEffect>,
    /// Shown on the victim when the spell lands: the `impactKit`'s models.
    ///
    /// The only set here that hangs on a unit other than the caster. It is kept
    /// beside the other two because the same `SpellVisual` row answers it, at
    /// the same time, through the same lookup. The renderer decides which unit
    /// wears it.
    pub impact: Vec<KitEffect>,
    /// Shown while a channel runs: the `channelKit`'s models, on the same terms
    /// as [`CastAnimation::channel`] and for the same reason.
    ///
    /// [`Self::hold`] takes these only when there is no precast kit, so a spell
    /// stating both would otherwise show the wind-up's models during the
    /// channel and never the channel's. Blizzard is the case: its falling ice
    /// hangs off its channel kit.
    pub channel: Vec<KitEffect>,
}

impl CastEffects {
    pub fn is_empty(&self) -> bool {
        self.hold.is_empty()
            && self.release.is_empty()
            && self.impact.is_empty()
            && self.channel.is_empty()
    }
}

/// `(kit field, attachment point)` for the five model columns, so the check
/// that measures them can name the same pairs the parser uses rather than a
/// second copy of them.
pub const EFFECT_POINTS: [(usize, u32); 6] = fields::EFFECTS;

/// Every column index this module measured, public.
///
/// [`crate::tables::schema`] also reads these tables, to describe them to an
/// editor. The indices are shared so that there is one reading of the layout,
/// not two. `schema`'s own test asserts every column it names against the
/// constants here.
pub mod fields {
    /// `Spell.dbc` field 115, `SpellVisualID`.
    ///
    /// Measured against the table it points into rather than counted from a
    /// layout. The four spells sampled read 67, 180, 285 and 5622, and all four
    /// are rows in `SpellVisual.dbc`. 5622 is well past that table's 2,167
    /// records, so a dense column would not match it by chance. The
    /// neighbouring columns, the spell icon (117) and priority, hold small
    /// integers that would also resolve to some row.
    pub const SPELL_VISUAL: usize = 115;

    /// `SpellVisual.dbc`: id, precastKit, castKit, impactKit, stateKit,
    /// channelKit, then the missile description: `hasMissile`, its model, its
    /// path type, its destination attachment and its sound. The missile
    /// description identifies the tail of the row (Fireball reads 1, 365, 0, 1,
    /// 3011 there).
    pub const PRECAST_KIT: usize = 1;
    pub const CAST_KIT: usize = 2;
    /// The kit played on the victim, not the caster: what a spell does when it
    /// lands. Fireball's is 286. Its animation is `CombatWound` (the flinch
    /// this client already plays on `SMSG_ATTACKERSTATEUPDATE`) and its models
    /// are the burst.
    ///
    /// No packet announces the impact. `SMSG_PLAY_SPELL_IMPACT` exists in the
    /// 1.12 opcode table, but vmangos sends it from one place only, the `.debug
    /// sendspellimpact` GM command (`DebugCommands.cpp`), so a client that
    /// waits for it never draws an impact. The 1.12 client times the impact
    /// itself: `SMSG_SPELL_GO`'s hit list says who was hit, and the missile's
    /// arrival says when.
    pub const IMPACT_KIT: usize = 3;
    /// The kit a unit wears for as long as an aura is on it: Ice Armor's hands,
    /// a shield bubble, a weapon enchant's glow.
    ///
    /// Every other kit marks a moment; this one marks a condition. The cast
    /// does not drive it, because a buff cast before the client saw the unit
    /// must still be drawn. `UNIT_FIELD_AURA` drives it instead of
    /// `SMSG_SPELL_GO`, which is why it is a separate lookup below rather than
    /// another field of [`super::CastEffects`].
    pub const STATE_KIT: usize = 4;
    pub const CHANNEL_KIT: usize = 5;

    /// The missile block, which identifies the tail of `SpellVisual`'s 16
    /// columns. Fireball reads `1, 365, 0, 1, 3011` across these five: a flag,
    /// an id well inside `SpellVisualEffectName`'s 782 rows, a small enum, an
    /// attachment id the character models carry, and a sound id outside that
    /// table. The same range check separated the kit's model columns from the
    /// rest of its row.
    pub const HAS_MISSILE: usize = 6;
    pub const MISSILE_MODEL: usize = 7;
    pub const MISSILE_PATH_TYPE: usize = 8;
    pub const MISSILE_DESTINATION: usize = 9;

    /// Whether this spell's persistent area draws anything. It gates the other
    /// two: when it is clear, the 1.12.1 client reads neither of them and draws
    /// no area.
    ///
    /// 217 of the 2,167 rows set it, and every one of those 217 has an
    /// [`AREA_MODEL`] that resolves. That shows the gate and the model are
    /// these two columns and not two dense columns that happen to agree.
    pub const AREA_FLAG: usize = 11;
    /// A `SpellVisualEffectName` id: the model the `DynamicObject` is drawn as.
    ///
    /// This column replaces a fallback chain over the caster's own five kits;
    /// the module comment says what that chain drew for Flamestrike.
    pub const AREA_MODEL: usize = 12;
    /// A `SpellVisualKit` id: the procedural part, which is a rain rather than
    /// a model. Only 10 of the 217 carry a kit whose [`CHAR_PROC`] is 9.
    pub const AREA_KIT: usize = 13;

    /// `SpellVisualKit.dbc` fields 15..18, `charProc[4]`: which client-side
    /// procedural a kit runs, with three parallel parameter columns after it.
    ///
    /// The four slots are searched in order for the wanted value, and the
    /// parameters are read at the slot index found, so a kit that carries two
    /// procedurals reads each one's parameters from its own column.
    ///
    /// The position follows from the columns either side. The five model
    /// columns end at 7, the sound is 13 and the shake 14, all measured, so the
    /// four procedurals are 15..18 and their parameters begin at 19. A row has
    /// 35 fields, with the id at field 0.
    pub const CHAR_PROC: usize = 15;
    /// How many procedural slots a kit has.
    pub const CHAR_PROC_SLOTS: usize = 4;
    /// `charParamZero[4]`, fields 19..22.
    pub const CHAR_PARAM_ZERO: usize = 19;
    /// `charParamOne[4]`, fields 23..26.
    pub const CHAR_PARAM_ONE: usize = 23;
    /// The falling-impact procedural. The other values in the shipped data (8
    /// with 34 rows, 11 with 17, …) are other procedurals, and this module does
    /// not guess at them.
    pub const CHAR_PROC_AREA_RAIN: u32 = 9;
    /// The aura-colour procedural, the commonest of them: 88 slots in 86 kits.
    /// See [`super::ModelTint`].
    pub const CHAR_PROC_MODEL_COLOUR: u32 = 1;
    /// The timed-colour procedural, 18 kits. See [`super::ModelFlash`].
    pub const CHAR_PROC_MODEL_GLOW: u32 = 13;
    /// The weapon-trail procedural; see [`super::WeaponTrail`].
    pub const CHAR_PROC_WEAPON_TRAIL: u32 = 8;
    /// The opacity procedural; see [`super::KitProcedurals::opacity`].
    pub const CHAR_PROC_MODEL_OPACITY: u32 = 14;

    /// The chain procedural: a bolt of lightning strung between units.
    ///
    /// It has two values, `0` and `12`, and the 1.12.1 client treats them as
    /// the same procedural. Reading only one of them loses 14 kits, the half
    /// that contains Chain Lightning, Chain Heal and every `Shock`.
    ///
    /// Zero is also what an all-zero row reads, so the id it names must resolve
    /// before anything is drawn: a kit whose four slots are `0` with all-zero
    /// parameters is a row with no meaning, not four chain effects. The
    /// unused-slot value in this column is `-1` for 6,851 of the 7,112 slots;
    /// the all-zero rows are the rest.
    pub const CHAR_PROC_CHAIN: [u32; 2] = [0, 12];

    /// `SpellChainEffects.dbc`: the seven columns after the id, in order. See
    /// [`super::ChainEffect`].
    pub const CHAIN_AVG_SEG_LEN: usize = 1;
    pub const CHAIN_WIDTH: usize = 2;
    pub const CHAIN_NOISE_SCALE: usize = 3;
    pub const CHAIN_TEX_COORD_SCALE: usize = 4;
    pub const CHAIN_SEG_DURATION: usize = 5;
    pub const CHAIN_SEG_DELAY: usize = 6;
    pub const CHAIN_TEXTURE: usize = 7;

    /// `charParamTwo[4]`, fields 27..30 — see [`super::ModelTint::params`].
    pub const CHAR_PARAM_TWO: usize = 27;
    /// `charParamThree[4]`, fields 31..34.
    pub const CHAR_PARAM_THREE: usize = 31;

    /// `Spell.dbc` fields 82..84, `EffectImplicitTargetA`, one per effect.
    ///
    /// Read for one question: does this spell land on its own caster?
    /// `SMSG_SPELL_GO`'s hit list says who was hit, and a cast that names
    /// nobody is ambiguous: a self-buff whose list came back empty and an area
    /// spell that hit nothing look the same on the wire. The spell's own row
    /// tells them apart. Without this, every Arcane Explosion that hit nothing
    /// burst on the mage's own chest.
    ///
    /// Measured rather than counted from a layout, as [`SPELL_VISUAL`] was:
    /// Frost Armor reads `1, 1, 0` here (vmangos' `TARGET_UNIT_CASTER`), Arcane
    /// Explosion reads `22, 0, 0` (`TARGET_LOCATION_CASTER_SRC`, a place rather
    /// than a unit) and Fireball reads `6, 6, 0` (`TARGET_UNIT_ENEMY`). Three
    /// spells whose targeting is known independently give three different
    /// answers, and only the self-buff reads 1.
    pub const EFFECT_TARGET_A: std::ops::Range<usize> = 82..85;
    /// `TARGET_UNIT_CASTER` in vmangos' `SpellDefines.h`.
    pub const TARGET_SELF: u32 = 1;

    /// `Spell.dbc` field 37, `Speed`, in yards per second.
    ///
    /// The only number in this chain that is not on `SpellVisual`. Projectile
    /// speed belongs to the spell, not to its art: Fireball throws
    /// `Fireball_Missile_Low.m2` at 24 y/s, and other spells that use the same
    /// model throw it at other speeds. Measured as every other index here:
    /// `vale dbc Spell 133` reads `f32 24.000` at 37, and vmangos'
    /// `SpellEntry::speed` carries the comment `// 37`.
    pub const SPELL_SPEED: usize = 37;

    /// `SpellVisualKit.dbc`'s six model columns, and the [`super::attach`]
    /// point each hangs from.
    ///
    /// The first five are contiguous. Three more columns of the same shape
    /// follow them, breath (8) and the two weapon effects (9, 10), and are not
    /// read: a breath effect belongs to a dragon rather than a caster, and a
    /// weapon effect hangs off the weapon's model rather than the wearer's.
    ///
    /// Field 12 is also a model column. An earlier version skipped it, on the
    /// argument that the columns past 10 are not models because field 13 is
    /// 1484 for Fireball, outside `SpellVisualEffectName`'s 782 rows. Field 13
    /// is a `SoundEntries` id, which is why it runs to 9060, but field 12
    /// resolves on all 37 kits that set it:
    ///
    /// ```text
    /// kit   66  ->  EntanglingRoots_State
    /// kit  285  ->  Frost_Nova_state
    /// kit  744  ->  Net_State
    /// kit  746  ->  Web_State
    /// kit  356  ->  ThunderClap_Cast_Base
    /// ```
    ///
    /// These are the roots, the ice, the Booty Bay guard's net, the spider web,
    /// and Thunder Clap's ring. 29 of the 37 set no other model column, so
    /// without field 12 those kits drew nothing on the unit. It was reported as
    /// "root spell effects are not drawn".
    ///
    /// The attachment point is `BASE`, taken from the models because no column
    /// in the row names one: field 13 beside it is the sound and field 14 a
    /// small flag. The model names end in `_State`, `_Base`, `_Ground` and
    /// `_area`, which are effects that sit on the floor under a unit. On 3 of
    /// the 37 it coexists with the real `BASE` column, which puts two models at
    /// the feet; that is allowed.
    pub const EFFECTS: [(usize, u32); 6] = [
        (3, super::attach::HEAD),
        (4, super::attach::CHEST),
        (5, super::attach::BASE),
        (6, super::attach::SPELL_HAND_LEFT),
        (7, super::attach::SPELL_HAND_RIGHT),
        (12, super::attach::BASE),
    ];

    /// `SpellVisualEffectName.dbc`: id, name, model path, another name, and a
    /// scale. Field 2 is the column `vale particles` takes its spell-effect
    /// population from, so a wrong index there would have shown as 782 models
    /// that do not parse.
    ///
    /// `SpellVisualEffectName` field 1: the row's own name, which the client
    /// uses to find the few visuals no chain names. See
    /// [`super::SpellVisuals::hardcoded`].
    pub const EFFECT_NAME: usize = 1;
    pub const EFFECT_MODEL: usize = 2;
    pub const EFFECT_SCALE: usize = 4;

    /// `SpellVisualKit.dbc` field 2, `animID`: an `AnimationData.dbc` id.
    ///
    /// `-1` is the table's "none" and appears in the neighbouring columns too.
    /// 0 is `Stand`, which is not used as an answer, for the same reason as in
    /// `Emotes.dbc`: a cast that resolved to Stand would interrupt the caster
    /// to make it stand still.
    pub const ANIMATION: usize = 2;
}

impl SpellVisuals {
    /// Build the map, or `None` if any of the three tables is missing or does
    /// not parse.
    ///
    /// All three or none: a `SpellVisual` with no kits to resolve is the wrong
    /// answer, not a partial one. Every spell would come back empty and the
    /// fallback would appear to work.
    ///
    /// `effect_name` is `SpellVisualEffectName.dbc` and `chain_effect` is
    /// `SpellChainEffects.dbc`; these two tables are optional. Without the
    /// first, a cast is still posed and has no models. Without the second, the
    /// 192 spells that string a bolt between units draw nothing. Both are
    /// reduced answers rather than wrong ones. The other three are all or none
    /// for the reason above.
    pub fn parse(
        spell: &[u8],
        visual: &[u8],
        kit: &[u8],
        effect_name: &[u8],
        chain_effect: &[u8],
    ) -> Option<SpellVisuals> {
        let spell = Dbc::parse(spell).ok()?;
        let visual = Dbc::parse(visual).ok()?;
        let kit = Dbc::parse(kit).ok()?;

        // effect id -> the model it names and its scale. A row with an empty
        // path has no art in the shipped data. It loses its own effect and
        // nothing else, as a missing `CharSections` texture loses its own
        // layer.
        let mut effect_model: HashMap<u32, (String, f32)> = HashMap::new();
        let mut hardcoded: HashMap<String, (String, f32)> = HashMap::new();
        if let Ok(names) = Dbc::parse(effect_name) {
            for record in 0..names.record_count {
                let Some(id) = names.u32_at(record, 0) else {
                    continue;
                };
                let path = names
                    .string_at(record, fields::EFFECT_MODEL)
                    .unwrap_or_default();
                if path.is_empty() {
                    continue;
                }
                let scale = effect_scale(names.f32_at(record, fields::EFFECT_SCALE));
                // The rows no spell names, kept by their own name; see
                // [`SpellVisuals::hardcoded`]. Fourteen of the 782 are called
                // `HARDCODED …`. The engine reaches each one directly rather
                // than through `SpellVisual`, so nothing above looks them up by
                // id.
                if let Some(name) = names.string_at(record, fields::EFFECT_NAME) {
                    if name.starts_with(HARDCODED) {
                        hardcoded.insert(name, (model_path(&path), scale));
                    }
                }
                effect_model.insert(id, (model_path(&path), scale));
            }
        }

        // chain id -> the bolt's shape. Optional, like the effect table; see
        // the doc above. Every id below must resolve here before anything is
        // drawn. The 1.12.1 client also draws nothing for an id with no row.
        let mut chain_rows: HashMap<u32, ChainEffect> = HashMap::new();
        if let Ok(rows) = Dbc::parse(chain_effect) {
            for record in 0..rows.record_count {
                let Some(id) = rows.u32_at(record, 0) else {
                    continue;
                };
                let texture = rows
                    .string_at(record, fields::CHAIN_TEXTURE)
                    .unwrap_or_default();
                // A row with no texture has nothing to wrap the strip in and
                // would draw as untextured white, which is more visible than
                // drawing nothing and less correct. Dropped, as `effect_model`
                // drops a row with no path.
                if texture.is_empty() {
                    continue;
                }
                let float = |field: usize, default: f32| {
                    rows.f32_at(record, field)
                        .filter(|v| v.is_finite())
                        .unwrap_or(default)
                };
                chain_rows.insert(
                    id,
                    ChainEffect {
                        texture,
                        // A zero here is a division by zero one layer up, so it
                        // falls back rather than being trusted; every shipped
                        // row reads 2.78.
                        avg_seg_len: float(fields::CHAIN_AVG_SEG_LEN, 1.0).max(0.01),
                        width: float(fields::CHAIN_WIDTH, 0.5).max(0.001),
                        noise_scale: float(fields::CHAIN_NOISE_SCALE, 0.0).max(0.0),
                        tex_coord_scale: float(fields::CHAIN_TEX_COORD_SCALE, 1.0),
                        seg_duration_ms: rows
                            .u32_at(record, fields::CHAIN_SEG_DURATION)
                            .unwrap_or(0),
                        seg_delay_ms: rows
                            .u32_at(record, fields::CHAIN_SEG_DELAY)
                            .unwrap_or(0),
                    },
                );
            }
        }

        // kit id -> the models it hangs, by attachment point.
        let mut kit_effects: HashMap<u32, Vec<KitEffect>> = HashMap::new();
        for record in 0..kit.record_count {
            let Some(id) = kit.u32_at(record, 0) else {
                continue;
            };
            let mut effects = Vec::new();
            for (field, point) in fields::EFFECTS {
                // The table's two ways of saying nothing, the same pair the
                // animation column uses.
                let Some(effect) = kit.u32_at(record, field).filter(|e| *e != 0 && *e != u32::MAX)
                else {
                    continue;
                };
                if let Some((path, scale)) = effect_model.get(&effect) {
                    effects.push(KitEffect {
                        point,
                        path: path.clone(),
                        scale: *scale,
                    });
                }
            }
            if !effects.is_empty() {
                kit_effects.insert(id, effects);
            }
        }

        // kit id -> the bolt it strings, for the 48 kits that have one.
        //
        // The same slot search as `kit_rain` below, for the same reason, plus
        // one check of its own: `charProc == 0` is also what an all-zero row
        // reads, so the id must resolve in the table before the kit counts as
        // carrying a chain. That check drops twenty rows.
        let mut kit_chain: HashMap<u32, ChainVisual> = HashMap::new();
        for record in 0..kit.record_count {
            let Some(id) = kit.u32_at(record, 0) else {
                continue;
            };
            let Some(slot) = (0..fields::CHAR_PROC_SLOTS).find(|slot| {
                kit.u32_at(record, fields::CHAR_PROC + slot)
                    .is_some_and(|kind| fields::CHAR_PROC_CHAIN.contains(&kind))
            }) else {
                continue;
            };
            // `charParamZero` is a float holding an integer id, as the rain's
            // is a float holding an index, and it is read the same way.
            let Some(effect) = kit
                .f32_at(record, fields::CHAR_PARAM_ZERO + slot)
                .filter(|i| i.is_finite() && *i >= 0.0)
                .map(|i| i.round() as u32)
                .and_then(|i| chain_rows.get(&i))
            else {
                continue;
            };
            let bolts = kit
                .f32_at(record, fields::CHAR_PARAM_ONE + slot)
                .filter(|b| b.is_finite() && *b >= 1.0)
                .map(|b| (b.round() as u32).min(MAX_CHAIN_BOLTS))
                .unwrap_or(1);
            let held = kit
                .f32_at(record, fields::CHAR_PARAM_TWO + slot)
                .is_some_and(|f| f.is_finite() && f.round() as i32 != 0);
            kit_chain.insert(
                id,
                ChainVisual {
                    effect: effect.clone(),
                    bolts,
                    held,
                },
            );
        }

        // kit id -> the rain it runs, for the eight kits that have one.
        //
        // The slot is searched and then used as an index: the parameters are
        // three parallel arrays, and a procedural in slot 2 reads its rate from
        // `charParamOne[2]`. Reading slot 0 would give every one of them
        // Blizzard's five per second.
        let mut kit_rain: HashMap<u32, AreaRain> = HashMap::new();
        for record in 0..kit.record_count {
            let Some(id) = kit.u32_at(record, 0) else {
                continue;
            };
            let Some(slot) = (0..fields::CHAR_PROC_SLOTS).find(|slot| {
                kit.u32_at(record, fields::CHAR_PROC + slot) == Some(fields::CHAR_PROC_AREA_RAIN)
            }) else {
                continue;
            };
            // `charParamZero` is a float holding an integer index, which the
            // 1.12.1 client converts to an integer. It is read as a float here
            // and rounded, so 2.0 is index 2, and a value outside the list is
            // dropped rather than clamped to the wrong spell's impact.
            let index = kit
                .f32_at(record, fields::CHAR_PARAM_ZERO + slot)
                .filter(|i| i.is_finite() && *i >= 0.0)
                .map(|i| i.round() as usize)
                .filter(|i| *i < AREA_RAIN_MODELS.len());
            let rate = kit
                .f32_at(record, fields::CHAR_PARAM_ONE + slot)
                .filter(|r| r.is_finite() && *r > 0.0);
            // A rate of zero is an accumulator that never reaches one, so the
            // entry would be a lookup that draws nothing for ever.
            if let (Some(index), Some(rate)) = (index, rate) {
                kit_rain.insert(
                    id,
                    AreaRain {
                        path: model_path(AREA_RAIN_MODELS[index]),
                        rate,
                    },
                );
            }
        }

        // kit id -> the colour it draws its bearer's model through, for the 86
        // kits that state one. The same slot search as the rain above, for the
        // same reason: the parameters are four parallel arrays and a procedural
        // in slot 2 reads its own column.
        let mut kit_tint: HashMap<u32, ModelTint> = HashMap::new();
        for record in 0..kit.record_count {
            let Some(id) = kit.u32_at(record, 0) else {
                continue;
            };
            let Some(slot) = (0..fields::CHAR_PROC_SLOTS).find(|slot| {
                kit.u32_at(record, fields::CHAR_PROC + slot) == Some(fields::CHAR_PROC_MODEL_COLOUR)
            }) else {
                continue;
            };
            // The float's value is the packed `0xRRGGBB`, truncated; see
            // [`ModelTint`]. The 1.12.1 client truncates toward zero, so this
            // is not the `round` used for the rain's model index one column
            // along: that is a small integer written exactly, and this is a
            // colour written as whatever the artist's tool produced. A value
            // outside the 24 bits is dropped rather than masked into another
            // colour.
            let Some(packed) = kit
                .f32_at(record, fields::CHAR_PARAM_ZERO + slot)
                .filter(|c| c.is_finite() && *c >= 0.0)
                .map(|c| c.trunc() as u32)
                .filter(|c| *c <= 0x00ff_ffff)
            else {
                continue;
            };
            let param = |field: usize| {
                kit.f32_at(record, field + slot)
                    .filter(|p| p.is_finite())
                    .unwrap_or(0.0)
            };
            kit_tint.insert(
                id,
                ModelTint {
                    colour: [
                        (packed >> 16) as u8,
                        (packed >> 8) as u8,
                        packed as u8,
                    ],
                    params: [
                        param(fields::CHAR_PARAM_ONE),
                        param(fields::CHAR_PARAM_TWO),
                        param(fields::CHAR_PARAM_THREE),
                    ],
                },
            );
        }

        // kit id -> the weapon trail, the timed colour and the opacity it
        // runs. Every slot is run in order, so a later slot stating the same
        // procedural replaces an earlier one, as it does in the client.
        let mut kit_procs: HashMap<u32, KitProcedurals> = HashMap::new();
        for record in 0..kit.record_count {
            let Some(id) = kit.u32_at(record, 0) else {
                continue;
            };
            let mut procs = KitProcedurals::default();
            for slot in 0..fields::CHAR_PROC_SLOTS {
                let param = |field: usize| {
                    kit.f32_at(record, field + slot)
                        .filter(|p| p.is_finite())
                };
                match kit.u32_at(record, fields::CHAR_PROC + slot) {
                    Some(fields::CHAR_PROC_WEAPON_TRAIL) => {
                        // A length of zero arms nothing: the 1.12.1 client
                        // starts no trail when the stored length is zero.
                        let duration_ms = param(fields::CHAR_PARAM_TWO)
                            .filter(|d| *d >= 1.0)
                            .map(|d| d as u32);
                        if let (Some(colour), Some(duration_ms)) =
                            (packed_colour(param(fields::CHAR_PARAM_ZERO)), duration_ms)
                        {
                            procs.trail = Some(WeaponTrail {
                                colour,
                                // The low byte of the truncated value, which
                                // the 1.12.1 client uses as the alpha.
                                alpha: param(fields::CHAR_PARAM_THREE)
                                    .map_or(0, |a| a.trunc() as i64 as u8),
                                duration_ms,
                            });
                        }
                    }
                    Some(fields::CHAR_PROC_MODEL_GLOW) => {
                        // Seconds to milliseconds, saturating: the test
                        // auras' 1e7 seconds are permanent either way.
                        let millis = |field: usize| {
                            param(field).map_or(0, |s| (s * 1000.0).max(0.0) as u32)
                        };
                        if let Some(colour) = packed_colour(param(fields::CHAR_PARAM_ZERO)) {
                            procs.flash = Some(ModelFlash {
                                colour,
                                hold_ms: millis(fields::CHAR_PARAM_ONE),
                                fade_ms: millis(fields::CHAR_PARAM_TWO),
                            });
                        }
                    }
                    Some(fields::CHAR_PROC_MODEL_OPACITY) => {
                        procs.opacity = param(fields::CHAR_PARAM_ZERO)
                            .filter(|o| *o >= 0.0 && *o != 1.0);
                    }
                    _ => {}
                }
            }
            if !procs.is_empty() {
                kit_procs.insert(id, procs);
            }
        }

        // kit id -> animation, for the two columns that reach the caster.
        let mut kit_anim: HashMap<u32, u16> = HashMap::with_capacity(kit.record_count);
        for record in 0..kit.record_count {
            let (Some(id), Some(anim)) = (kit.u32_at(record, 0), kit.u32_at(record, fields::ANIMATION))
            else {
                continue;
            };
            // -1 is "none" and 0 is Stand; neither is a pose to play.
            if anim == u32::MAX || anim == 0 || anim > u16::MAX as u32 {
                continue;
            }
            kit_anim.insert(id, anim as u16);
        }

        // visual id -> the caster's two animations, and its two sets of models.
        let mut visual_anim: HashMap<u32, CastAnimation> = HashMap::with_capacity(visual.record_count);
        let mut visual_effects: HashMap<u32, CastEffects> = HashMap::new();
        let mut visual_state: HashMap<u32, Vec<KitEffect>> = HashMap::new();
        let mut visual_state_anim: HashMap<u32, u16> = HashMap::new();
        let mut visual_state_tint: HashMap<u32, ModelTint> = HashMap::new();
        let mut visual_missile: HashMap<u32, Missile> = HashMap::new();
        let mut visual_area: HashMap<u32, AreaEffect> = HashMap::new();
        let mut visual_chain: HashMap<u32, ChainVisual> = HashMap::new();
        let mut visual_procs: HashMap<u32, SpellProcedurals> = HashMap::new();
        for record in 0..visual.record_count {
            let Some(id) = visual.u32_at(record, 0) else {
                continue;
            };
            let kit_of =
                |field: usize| visual.u32_at(record, field).filter(|k| *k != 0 && *k != u32::MAX);
            let of = |field: usize| kit_of(field).and_then(|k| kit_anim.get(&k).copied());
            let animation = CastAnimation {
                // The channel is the wind-up for a spell that has no other:
                // Arcane Missiles states only `channelKit`, and its pose is
                // held for the whole duration exactly as a cast bar's is.
                hold: of(fields::PRECAST_KIT).or_else(|| of(fields::CHANNEL_KIT)),
                release: of(fields::CAST_KIT),
                // …and its own column besides, for the 45 visuals that state
                // both — see [`CastAnimation::channel`].
                channel: of(fields::CHANNEL_KIT),
                // Not a property of the visual; set per spell below, from that
                // row's own `Attributes`.
                ranged_shot: false,
            };
            if !animation.is_empty() {
                visual_anim.insert(id, animation);
            }

            // The held models come from the kit that supplied the held pose,
            // and the two are looked up separately: a spell can state a
            // `precastKit` with a pose and no models and a `channelKit` with
            // models and no pose, and taking both from one kit would lose
            // whichever part the other kit had.
            let models = |field: usize| {
                kit_of(field)
                    .and_then(|k| kit_effects.get(&k))
                    .cloned()
                    .unwrap_or_default()
            };
            let channel = models(fields::CHANNEL_KIT);
            let mut hold = models(fields::PRECAST_KIT);
            if hold.is_empty() {
                hold.clone_from(&channel);
            }
            let effects = CastEffects {
                hold,
                release: models(fields::CAST_KIT),
                impact: models(fields::IMPACT_KIT),
                channel,
            };
            if !effects.is_empty() {
                visual_effects.insert(id, effects);
            }
            let state = models(fields::STATE_KIT);
            if !state.is_empty() {
                visual_state.insert(id, state);
            }
            // …and the same row's pose, which is asked separately from its
            // models for the reason `SpellVisuals::state_anim` gives.
            if let Some(pose) = of(fields::STATE_KIT) {
                visual_state_anim.insert(id, pose);
            }
            // The colour the state kit draws the bearer through, asked
            // separately from its models: a kit can state a colour and no
            // models, as Stoneform's does. The state kit only: 75 of the 86
            // kits are state kits. The client files a colour from any other
            // kit under the spell's id and removes it at the next event about
            // that spell on the unit, such as the aura's removal or the
            // spell's next cast, so no length is stated for it. Those six
            // kits are not drawn.
            if let Some(tint) = kit_of(fields::STATE_KIT).and_then(|k| kit_tint.get(&k)) {
                visual_state_tint.insert(id, *tint);
            }
            let procs = |field: usize| {
                kit_of(field)
                    .and_then(|k| kit_procs.get(&k))
                    .copied()
                    .unwrap_or_default()
            };
            let procedurals = SpellProcedurals {
                precast: procs(fields::PRECAST_KIT),
                cast: procs(fields::CAST_KIT),
                channel: procs(fields::CHANNEL_KIT),
                impact: procs(fields::IMPACT_KIT),
                state: procs(fields::STATE_KIT),
            };
            if !procedurals.is_empty() {
                visual_procs.insert(id, procedurals);
            }

            // The area is gated on field 11 first. The 1.12.1 client reads
            // neither of the other two when it is clear, and this matters: 33
            // rows carry a field-12 id with the flag clear, and drawing them
            // would put a model on the ground under every `DynamicObject` whose
            // spell states it should have none.
            let has_area = visual
                .u32_at(record, fields::AREA_FLAG)
                .is_some_and(|f| f != 0 && f != u32::MAX);
            if has_area {
                if let Some((path, scale)) = visual
                    .u32_at(record, fields::AREA_MODEL)
                    .filter(|m| *m != 0 && *m != u32::MAX)
                    .and_then(|m| effect_model.get(&m))
                {
                    visual_area.insert(
                        id,
                        AreaEffect {
                            path: path.clone(),
                            scale: *scale,
                            // The rain comes from the kit and is optional: 207
                            // of the 217 areas are one still model and nothing
                            // else.
                            rain: kit_of(fields::AREA_KIT)
                                .and_then(|k| kit_rain.get(&k))
                                .cloned(),
                        },
                    );
                }
            }

            // The kit that carries the chain decides when it is drawn. A cast
            // or precast kit fires it once; a channel kit holds it. No shipped
            // visual sets both, and the three that set precast and cast use the
            // same kit for both, so the first one found is the answer.
            if let Some(chain) = [
                fields::CAST_KIT,
                fields::PRECAST_KIT,
                fields::CHANNEL_KIT,
                fields::IMPACT_KIT,
                fields::STATE_KIT,
            ]
            .into_iter()
            .find_map(|field| kit_of(field).and_then(|k| kit_chain.get(&k)))
            {
                visual_chain.insert(id, chain.clone());
            }

            // The flag alone is not enough. 1,187 visuals set `hasMissile`, and
            // some of them name a model that resolves to nothing: a row shipped
            // without art, as in `SpellVisualEffectName` above. A missile with
            // no model is not drawn, so the entry is made only when a file is
            // named. The speed is filled in per spell below, since it is on the
            // spell.
            let has_missile = visual
                .u32_at(record, fields::HAS_MISSILE)
                .is_some_and(|f| f != 0 && f != u32::MAX);
            if has_missile {
                if let Some((path, scale)) = visual
                    .u32_at(record, fields::MISSILE_MODEL)
                    .filter(|m| *m != 0 && *m != u32::MAX)
                    .and_then(|m| effect_model.get(&m))
                {
                    visual_missile.insert(
                        id,
                        Missile {
                            path: path.clone(),
                            scale: *scale,
                            // Filled in from the spell's own row.
                            speed: 0.0,
                            path_type: visual
                                .u32_at(record, fields::MISSILE_PATH_TYPE)
                                .unwrap_or(0),
                            destination: visual
                                .u32_at(record, fields::MISSILE_DESTINATION)
                                .filter(|d| *d != u32::MAX)
                                .unwrap_or(0),
                        },
                    );
                }
            }
        }

        let mut by_spell = HashMap::new();
        let mut effects = HashMap::new();
        let mut states = HashMap::new();
        let mut state_anim = HashMap::new();
        let mut state_tint = HashMap::new();
        let mut missiles = HashMap::new();
        let mut areas = HashMap::new();
        let mut chains = HashMap::new();
        let mut procedurals = HashMap::new();
        let mut self_cast = std::collections::HashSet::new();
        for record in 0..spell.record_count {
            if let Some(id) = spell.u32_at(record, 0) {
                if fields::EFFECT_TARGET_A
                    .clone()
                    .any(|f| spell.u32_at(record, f) == Some(fields::TARGET_SELF))
                {
                    self_cast.insert(id);
                }
            }
            let (Some(id), Some(visual_id)) =
                (spell.u32_at(record, 0), spell.u32_at(record, fields::SPELL_VISUAL))
            else {
                continue;
            };
            // A ranged attack is the one release the visual chain does not
            // carry, so it is set here rather than looked up with the
            // animation; see [`CastAnimation::ranged_shot`]. The entry is
            // inserted even for a spell with no visual, which is Auto Shot's
            // case: `SpellVisual = 0`, no kit, nothing to join.
            let ranged_shot = spell
                .u32_at(record, crate::tables::spellbook::spell_fields::ATTRIBUTES)
                .is_some_and(|a| a & crate::tables::spellbook::spell_attributes::USES_RANGED_SLOT != 0);
            match visual_anim.get(&visual_id) {
                Some(animation) => {
                    by_spell.insert(id, CastAnimation { ranged_shot, ..*animation });
                }
                None if ranged_shot => {
                    by_spell.insert(id, CastAnimation { ranged_shot, ..Default::default() });
                }
                None => {}
            }
            if let Some(models) = visual_effects.get(&visual_id) {
                effects.insert(id, models.clone());
            }
            if let Some(models) = visual_state.get(&visual_id) {
                states.insert(id, models.clone());
            }
            if let Some(pose) = visual_state_anim.get(&visual_id) {
                state_anim.insert(id, *pose);
            }
            if let Some(tint) = visual_state_tint.get(&visual_id) {
                state_tint.insert(id, *tint);
            }
            // The one place the chain reads the spell's own row for something
            // other than the visual id: how fast the projectile flies.
            if let Some(missile) = visual_missile.get(&visual_id) {
                let speed = spell
                    .f32_at(record, fields::SPELL_SPEED)
                    .filter(|s| s.is_finite() && *s > 0.0)
                    .unwrap_or(0.0);
                missiles.insert(id, Missile { speed, ..missile.clone() });
            }
            if let Some(area) = visual_area.get(&visual_id) {
                areas.insert(id, area.clone());
            }
            if let Some(chain) = visual_chain.get(&visual_id) {
                chains.insert(id, chain.clone());
            }
            if let Some(procs) = visual_procs.get(&visual_id) {
                procedurals.insert(id, *procs);
            }
        }
        Some(SpellVisuals {
            by_spell,
            effects,
            hardcoded,
            states,
            state_anim,
            state_tint,
            missiles,
            areas,
            chains,
            kits: kit_effects,
            kit_poses: kit_anim,
            kit_procedurals: kit_procs,
            procedurals,
            self_cast,
            spells: spell.record_count,
        })
    }

    /// A visual the engine spawns, looked up by the name the table gives it.
    ///
    /// Fourteen of `SpellVisualEffectName`'s 782 rows are called `HARDCODED
    /// <something>`, and none is reachable through `SpellVisual`: no spell
    /// names them, so the chain [`Self::effects`] walks never produces one. The
    /// 1.12.1 client finds them by name and plays them on engine events: a
    /// level-up, a mount dismissing, an inebriation, a pet's loyalty changing,
    /// and the loot sparkle this function was written for:
    ///
    /// ```text
    /// id 14  "HARDCODED Loot Art"       Particles\LootFX.mdl
    /// id 21  "HARDCODED Unit Level Up"  Spells\LevelUp\LevelUp.mdl
    /// ```
    ///
    /// The client looks these rows up by name, so this function does too. The
    /// ids are only what the shipped table holds; the names are what the client
    /// asks for.
    ///
    /// Returns the model path, already through [`model_path`] (the `.m2` the
    /// archive holds rather than the `.mdl` the table names), and the row's own
    /// scale.
    pub fn hardcoded(&self, name: &str) -> Option<(&str, f32)> {
        let (path, scale) = self.hardcoded.get(name)?;
        Some((path.as_str(), *scale))
    }

    /// The models `spell_id`'s kits hang on the caster, or `None` for a spell
    /// whose visual names none.
    pub fn effects(&self, spell_id: u32) -> Option<&CastEffects> {
        self.effects.get(&spell_id)
    }

    /// Whether this spell lands on the unit that cast it.
    ///
    /// Asked only of a release whose hit list named nobody. A self-buff's list
    /// usually carries the caster's own guid and sometimes carries nothing, and
    /// both cases must give the same answer; without the fallback, the buffs
    /// whose impact is the visible part (Frost Armor's head, Dampen Magic's
    /// ring) draw nothing. The fallback cannot apply to every spell either: an
    /// area spell that hit no one also names nobody, and giving it the caster
    /// bursts Arcane Explosion on the mage's own chest every time it misses.
    pub fn is_self_cast(&self, spell_id: u32) -> bool {
        self.self_cast.contains(&spell_id)
    }

    /// The models a unit wears while `spell_id`'s aura is on it, or `None` for
    /// the great majority of spells that leave no visible mark.
    ///
    /// Asked per aura slot rather than per cast; see [`fields::STATE_KIT`].
    pub fn state(&self, spell_id: u32) -> Option<&Vec<KitEffect>> {
        self.states.get(&spell_id)
    }

    /// The models a `SpellVisualKit` hangs, by kit id: the one lookup here that
    /// does not start from a spell.
    ///
    /// `SMSG_PLAY_SPELL_VISUAL` and `SMSG_PLAY_SPELL_IMPACT` carry a kit and no
    /// spell, so nothing else here can answer them. See [`Self::kits`].
    pub fn kit(&self, kit_id: u32) -> Option<&Vec<KitEffect>> {
        self.kits.get(&kit_id)
    }

    /// The pose that kit holds. For kits 406 and 438 the pose is the visible
    /// part: both read `animID 61`, `EmoteEat`.
    pub fn kit_pose(&self, kit_id: u32) -> Option<u16> {
        self.kit_poses.get(&kit_id).copied()
    }

    /// How many kits resolved to at least one model, and how many to a pose —
    /// for `vale spell`, which reports what a table produced rather than
    /// what it contains.
    pub fn kit_counts(&self) -> (usize, usize) {
        (self.kits.len(), self.kit_poses.len())
    }

    /// The weapon trail, timed colour and opacity `spell_id`'s kits run, by
    /// the moment each kit plays. `None` for a spell whose kits run none.
    pub fn procedurals(&self, spell_id: u32) -> Option<&SpellProcedurals> {
        self.procedurals.get(&spell_id)
    }

    /// The same, asked by kit id, for the two packets that carry a kit and no
    /// spell. See [`Self::kit`].
    pub fn kit_procedurals(&self, kit_id: u32) -> Option<&KitProcedurals> {
        self.kit_procedurals.get(&kit_id)
    }

    /// The opacity multiplier a unit is drawn at while `spell_id`'s aura is on
    /// it: its state kit's `charProc` 14. See [`KitProcedurals::opacity`].
    pub fn aura_opacity(&self, spell_id: u32) -> Option<f32> {
        self.procedurals.get(&spell_id)?.state.opacity
    }

    /// `(kits with a weapon trail, with a timed colour, with an opacity)`, and
    /// the spells reaching each — what `vale spell` prints.
    pub fn procedural_counts(&self) -> ([usize; 3], [usize; 3]) {
        let kits = |f: fn(&KitProcedurals) -> bool| self.kit_procedurals.values().filter(|k| f(k)).count();
        let spells = |f: fn(&KitProcedurals) -> bool| {
            self.procedurals
                .values()
                .filter(|s| [s.precast, s.cast, s.channel, s.impact, s.state].iter().any(f))
                .count()
        };
        let trail: fn(&KitProcedurals) -> bool = |k| k.trail.is_some();
        let flash: fn(&KitProcedurals) -> bool = |k| k.flash.is_some();
        let opacity: fn(&KitProcedurals) -> bool = |k| k.opacity.is_some();
        (
            [kits(trail), kits(flash), kits(opacity)],
            [spells(trail), spells(flash), spells(opacity)],
        )
    }

    /// The pose a unit holds for as long as `spell_id`'s aura is on it, or
    /// `None` for most auras, which do not change how their bearer stands.
    ///
    /// Asked per aura slot, as [`Self::state`] is, for the same reason: a stun
    /// cast before this client saw the unit must still be drawn, so the cast
    /// cannot be the trigger. See [`SpellVisuals::state_anim`] for how the
    /// column was identified.
    pub fn aura_pose(&self, spell_id: u32) -> Option<u16> {
        self.state_anim.get(&spell_id).copied()
    }

    /// The colour a unit's own model is drawn through while `spell_id`'s aura
    /// is on it: Stoneform's stone, a ghost's pallor, Shadowform's purple.
    ///
    /// Asked per aura slot, as [`Self::state`] and [`Self::aura_pose`] are, for
    /// the same reason: an aura applied before this client saw the unit must
    /// still be drawn, so the cast cannot be the trigger. See [`ModelTint`] for
    /// the encoding and for the eight rows named after their own colours.
    pub fn aura_tint(&self, spell_id: u32) -> Option<ModelTint> {
        self.state_tint.get(&spell_id).copied()
    }

    /// `(spells whose aura paints their bearer, the distinct colours)` — what
    /// `vale spell` reports, and the pair that moves if the column does.
    ///
    /// The colours are listed rather than counted because the list is the
    /// check: a wrong column does not produce `#ff0000` for a row called `Glowy
    /// (Red)`.
    pub fn aura_tint_counts(&self) -> (usize, Vec<[u8; 3]>) {
        let colours: std::collections::BTreeSet<[u8; 3]> =
            self.state_tint.values().map(|t| t.colour).collect();
        (self.state_tint.len(), colours.into_iter().collect())
    }

    /// `(spells whose aura states a pose, the census of which poses)`, the
    /// second sorted commonest first — what `vale spell` prints, and the
    /// measurement the renderer's precedence for the pose is judged against.
    ///
    /// The census matters more than the total: a column read as a pose should
    /// name poses a stopped unit holds, and it does; see the check's own
    /// output.
    pub fn aura_pose_counts(&self) -> (usize, Vec<(u16, usize)>) {
        let mut per: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
        for pose in self.state_anim.values() {
            *per.entry(*pose).or_default() += 1;
        }
        let mut census: Vec<(u16, usize)> = per.into_iter().collect();
        census.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        (self.state_anim.len(), census)
    }

    /// What a persistent area of `spell_id` is drawn as: a Blizzard, a
    /// Flamestrike, a Rain of Fire, a Consecration.
    ///
    /// The server puts one `DynamicObject` in the world per area, carrying only
    /// the spell id and a radius, and nothing on the wire says what it looks
    /// like. Three columns of the spell's own `SpellVisual` row do; the module
    /// comment describes them and their population.
    ///
    /// This replaced a fallback chain over the caster's five kits. That chain
    /// read attachment 19 from the state kit, then the impact, then the
    /// release, then the hold. It matched the file for Blizzard by chance (both
    /// name `Blizzard_Impact_Base`) and not for Flamestrike, where it drew a
    /// two-emitter `Immolate_State_Base` in place of the 454-vertex
    /// `Flamestrike_Impact_Base` that field 12 names. Nothing reported the
    /// error, because a smaller fire still looks like a fire.
    pub fn area(&self, spell_id: u32) -> Option<&AreaEffect> {
        self.areas.get(&spell_id)
    }

    /// The bolt this spell strings between units, or `None` for the 22,168
    /// spells that string none; see [`ChainVisual`].
    ///
    /// This is the one visual in the chain with no model: every other answer
    /// here is a path into the archives, but `SpellChainEffects` states a
    /// texture and six numbers and the client builds the geometry. A renderer
    /// that reads only the model columns draws nothing for Chain Lightning,
    /// Chain Heal, Drain Life, Mind Flay and the Rallying Cry buffs, as this
    /// one did before this table was read.
    pub fn chain(&self, spell_id: u32) -> Option<&ChainVisual> {
        self.chains.get(&spell_id)
    }

    /// `(spells with a chain, of which held, distinct shapes, the textures)` -
    /// the population behind [`Self::chain`].
    ///
    /// The middle number shows the `charParamTwo` reading is right: it must
    /// count the channels, and a reading with the flag inverted reports the
    /// complement.
    pub fn chain_counts(&self) -> (usize, usize, usize, Vec<&str>) {
        let held = self.chains.values().filter(|c| c.held).count();
        let mut shapes: Vec<&ChainEffect> = Vec::new();
        let mut textures: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for chain in self.chains.values() {
            textures.insert(chain.effect.texture.as_str());
            if !shapes.iter().any(|e| **e == chain.effect) {
                shapes.push(&chain.effect);
            }
        }
        (
            self.chains.len(),
            held,
            shapes.len(),
            textures.into_iter().collect(),
        )
    }

    /// `(spells with an area, of which rain, distinct models, distinct rains)`:
    /// the population behind [`Self::area`], so that a column read one along
    /// shows as a changed number.
    ///
    /// The last is a census rather than a count for the same reason as
    /// [`Self::aura_pose_counts`]: `charParamZero` is an index into a list that
    /// is in no file, so the check that it was read correctly is that the
    /// models it picks match the names of the spells that ask for them.
    ///
    /// Every distinct model a persistent area names: the object's own and the
    /// rain's. A caller checking them against the archive must ask about both,
    /// and eight of the ten rains name a different file from the area they fall
    /// in.
    pub fn area_models(&self) -> Vec<&str> {
        let mut all: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for area in self.areas.values() {
            all.insert(area.path.as_str());
            if let Some(rain) = &area.rain {
                all.insert(rain.path.as_str());
            }
        }
        all.into_iter().collect()
    }

    pub fn area_counts(&self) -> (usize, usize, usize, Vec<(String, usize)>) {
        let models: std::collections::BTreeSet<&str> =
            self.areas.values().map(|a| a.path.as_str()).collect();
        let mut rains: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for area in self.areas.values() {
            if let Some(rain) = &area.rain {
                *rains.entry(rain.path.as_str()).or_default() += 1;
            }
        }
        let raining = self.areas.values().filter(|a| a.rain.is_some()).count();
        let mut census: Vec<(String, usize)> =
            rains.into_iter().map(|(p, n)| (p.to_string(), n)).collect();
        census.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        (self.areas.len(), raining, models.len(), census)
    }

    /// `(spells whose aura is visible, distinct models those states name)` —
    /// what `vale spell` reports beside the cast effects.
    pub fn state_counts(&self) -> (usize, usize) {
        let models: std::collections::BTreeSet<&str> = self
            .states
            .values()
            .flatten()
            .map(|e| e.path.as_str())
            .collect();
        (self.states.len(), models.len())
    }

    /// `(spells with an impact, distinct models those impacts name)`.
    pub fn impact_counts(&self) -> (usize, usize) {
        let with = self.effects.values().filter(|e| !e.impact.is_empty()).count();
        let models: std::collections::BTreeSet<&str> = self
            .effects
            .values()
            .flat_map(|e| &e.impact)
            .map(|e| e.path.as_str())
            .collect();
        (with, models.len())
    }

    /// The projectile `spell_id` throws, or `None` for the great majority that
    /// throw nothing.
    pub fn missile(&self, spell_id: u32) -> Option<&Missile> {
        self.missiles.get(&spell_id)
    }

    /// `(spells that throw a missile, distinct missile models, how many state
    /// no speed)`, which `vale spell` reports. The third matters most: a
    /// missile with no speed cannot be flown, and a large count there would
    /// mean the speed column had moved rather than that a few rows are instant.
    pub fn missile_counts(&self) -> (usize, usize, usize) {
        let models: std::collections::BTreeSet<&str> =
            self.missiles.values().map(|m| m.path.as_str()).collect();
        let speedless = self.missiles.values().filter(|m| m.speed <= 0.0).count();
        (self.missiles.len(), models.len(), speedless)
    }

    /// `(spells with effect models, distinct models named)` — the pair
    /// `vale spell` reports beside the animation counts.
    pub fn effect_counts(&self) -> (usize, usize) {
        let mut models: std::collections::BTreeSet<&str> = Default::default();
        for effect in self.effects.values() {
            for e in effect.hold.iter().chain(&effect.release).chain(&effect.impact) {
                models.insert(e.path.as_str());
            }
        }
        (self.effects.len(), models.len())
    }

    /// What the caster of `spell_id` does, or `None` for a spell with no
    /// visual — a passive, a proc, or one of the many rows that are pure
    /// gameplay.
    pub fn cast(&self, spell_id: u32) -> Option<CastAnimation> {
        self.by_spell.get(&spell_id).copied()
    }

    /// `(spells in the table, spells that resolve to an animation)` — what
    /// `vale spell` reports, and the number that moves if a column does.
    ///
    /// It counts poses, not entries: [`Self::by_spell`] also holds the ranged
    /// attacks, which state no kit and whose release comes from the wielder's
    /// weapon ([`CastAnimation::ranged_shot`]). Counting entries would shift
    /// this baseline by the size of an unrelated population and hide a column
    /// regression.
    pub fn counts(&self) -> (usize, usize) {
        let with_a_pose = self
            .by_spell
            .values()
            .filter(|a| a.hold.is_some() || a.release.is_some())
            .count();
        (self.spells, with_a_pose)
    }

    /// How many spells fire the wielder's own ranged weapon rather than a pose
    /// from the visual chain. It is reported beside the count above and shows
    /// that the `Attributes` bit was found.
    pub fn ranged_shots(&self) -> usize {
        self.by_spell.values().filter(|a| a.ranged_shot).count()
    }

    /// How many spells land on each animation id, commonest first.
    ///
    /// This makes the column checkable without a server: the animations the
    /// game's spells resolve to should be the casting ones, and a wrong column
    /// gives a spread over ids that mean nothing.
    pub fn histogram(&self) -> Vec<(u16, usize)> {
        let mut counts: HashMap<u16, usize> = HashMap::new();
        for animation in self.by_spell.values() {
            for id in [animation.hold, animation.release].into_iter().flatten() {
                *counts.entry(id).or_default() += 1;
            }
        }
        let mut out: Vec<(u16, usize)> = counts.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }
}

/// A `Dbc` this crate can build for a test without an archive.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing;

    /// One row per table, wired so that spell 133 reaches animation 53.
    fn chain() -> Option<SpellVisuals> {
        let mut spell = vec![0u32; 173];
        spell[0] = 133;
        spell[fields::SPELL_VISUAL] = 67;

        let mut visual = vec![0u32; 16];
        visual[0] = 67;
        visual[fields::PRECAST_KIT] = 30;
        visual[fields::CAST_KIT] = 38;

        let mut precast = vec![0u32; 35];
        precast[0] = 30;
        precast[fields::ANIMATION] = 51;
        let mut cast = vec![0u32; 35];
        cast[0] = 38;
        cast[fields::ANIMATION] = 53;

        SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[precast, cast], 35, b"\0"),
            &[],
            &[],
        )
    }

    /// The three hops, end to end. A wrong index at any of them resolves to
    /// nothing rather than to something wrong, which is why this is one test.
    #[test]
    fn a_spell_reaches_its_two_animations() {
        let visuals = chain().expect("three tables");
        let animation = visuals.cast(133).expect("Fireball has a visual");
        assert_eq!(animation.hold, Some(51), "ReadySpellDirected");
        assert_eq!(animation.release, Some(53), "SpellCastDirected");
        assert_eq!(visuals.cast(134), None, "a spell that is not in the table");
    }

    /// The visuals the engine spawns are found by name, because no
    /// `SpellVisual` names them. Their ids are only what the shipped table
    /// holds; the names are what the 1.12.1 client asks for. `HARDCODED Loot
    /// Art` and `HARDCODED Unit Level Up` are both names the client uses; see
    /// [`SpellVisuals::hardcoded`].
    #[test]
    fn the_engine_spawned_visuals_are_found_by_their_own_name() {
        let mut spell = vec![0u32; 173];
        spell[0] = 1;
        let mut loot = vec![0u32; 5];
        loot[0] = 14;
        loot[fields::EFFECT_NAME] = 1;
        loot[fields::EFFECT_MODEL] = 20;
        // The scale column is an f32; 1.0 as bits.
        loot[fields::EFFECT_SCALE] = 1.0_f32.to_bits();
        let strings = b"\0HARDCODED Loot Art\0Particles\\LootFX.mdl\0";
        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[vec![0u32; 16]], 16, b"\0"),
            &testing::dbc(&[vec![0u32; 35]], 35, b"\0"),
            &testing::dbc(&[loot], 5, strings),
            &[],
        )
        .expect("the chain parses");
        let (path, scale) = visuals.hardcoded(LOOT_ART).expect("the loot art row");
        // `.mdl` in the table, `.m2` in the archive: the same change as every
        // other effect path in this module.
        assert_eq!(path, "Particles\\LootFX.m2");
        assert_eq!(scale, 1.0);
        assert_eq!(visuals.hardcoded("HARDCODED Nothing"), None);
    }

    /// The colour is the float's value, packed `0xRRGGBB`. A wrong reading of
    /// [`ModelTint`] gives a plausible colour rather than none, since any float
    /// decodes to some colour.
    ///
    /// Stoneform's shipped row is the case: `charParamZero` holds 4,474,718.0,
    /// which is `#44465e`, the dark blue-grey a dwarf turns. Reading the bits
    /// instead would give the top bytes of `0x4a888cbc` and paint him
    /// mid-green.
    #[test]
    fn an_aura_colour_is_the_float_s_value_and_not_its_bits() {
        let mut spell = vec![0u32; 173];
        spell[0] = 20594;
        spell[fields::SPELL_VISUAL] = 5787;

        let mut visual = vec![0u32; 16];
        visual[0] = 5787;
        visual[fields::STATE_KIT] = 5152;

        let mut kit = vec![0u32; 35];
        kit[0] = 5152;
        // Slot 0 of the four, exactly as the shipped row has it.
        kit[fields::CHAR_PROC] = fields::CHAR_PROC_MODEL_COLOUR;
        kit[fields::CHAR_PARAM_ZERO] = (0x0044_465e as f32).to_bits();
        kit[fields::CHAR_PARAM_ONE] = 0.2_f32.to_bits();
        kit[fields::CHAR_PARAM_TWO] = 5.0_f32.to_bits();

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[kit], 35, b"\0"),
            &[],
            &[],
        )
        .expect("the chain parses");
        let tint = visuals.aura_tint(20594).expect("Stoneform paints its bearer");
        assert_eq!(tint.colour, [0x44, 0x46, 0x5e]);
        assert_eq!(tint.params, [0.2, 5.0, 0.0]);
        assert_eq!(visuals.aura_tint(133), None, "a spell with no colour");
    }

    /// The parameters are read at the procedural's own slot, as [`AreaRain`]'s
    /// rate is: the four `charProc`s and their four parameter arrays are
    /// parallel, so a colour declared in slot 2 whose parameters are read from
    /// slot 0 gets some other procedural's values.
    #[test]
    fn an_aura_colour_reads_the_slot_its_procedural_is_in() {
        let mut spell = vec![0u32; 173];
        spell[0] = 7;
        spell[fields::SPELL_VISUAL] = 8;
        let mut visual = vec![0u32; 16];
        visual[0] = 8;
        visual[fields::STATE_KIT] = 9;
        let mut kit = vec![0u32; 35];
        kit[0] = 9;
        // Slot 2 carries the colour; slots 0 and 1 carry a different procedural
        // whose parameters must not be taken for this one's.
        kit[fields::CHAR_PROC] = fields::CHAR_PROC_AREA_RAIN;
        kit[fields::CHAR_PROC + 2] = fields::CHAR_PROC_MODEL_COLOUR;
        kit[fields::CHAR_PARAM_ZERO] = (0x00ff_0000 as f32).to_bits();
        kit[fields::CHAR_PARAM_ZERO + 2] = (0x0000_ff00 as f32).to_bits();
        kit[fields::CHAR_PARAM_ONE + 2] = 3.0_f32.to_bits();

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[kit], 35, b"\0"),
            &[],
            &[],
        )
        .expect("the chain parses");
        let tint = visuals.aura_tint(7).expect("the kit states a colour");
        assert_eq!(tint.colour, [0x00, 0xff, 0x00], "slot 2's colour, not slot 0's");
        assert_eq!(tint.params[0], 3.0);
    }

    /// Kit 44, Charge's cast kit, as shipped: `charProc 8` with
    /// `14355216.0, 20.0, 1000.0, 100.0`. The colour is `#db0b10`, the length
    /// 1,000 ms and the opacity 100; the 20 is not read.
    #[test]
    fn a_weapon_trail_reads_colour_length_and_opacity_from_its_slot() {
        let mut spell = vec![0u32; 173];
        spell[0] = 100;
        spell[fields::SPELL_VISUAL] = 50;
        let mut visual = vec![0u32; 16];
        visual[0] = 50;
        visual[fields::CAST_KIT] = 44;
        let mut kit = vec![0u32; 35];
        kit[0] = 44;
        // Slot 1, behind an unused slot, so the parameters must come from
        // column 1 of each array.
        kit[fields::CHAR_PROC] = u32::MAX;
        kit[fields::CHAR_PROC + 1] = fields::CHAR_PROC_WEAPON_TRAIL;
        kit[fields::CHAR_PARAM_ZERO + 1] = 14_355_216.0_f32.to_bits();
        kit[fields::CHAR_PARAM_ONE + 1] = 20.0_f32.to_bits();
        kit[fields::CHAR_PARAM_TWO + 1] = 1000.0_f32.to_bits();
        kit[fields::CHAR_PARAM_THREE + 1] = 100.0_f32.to_bits();

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[kit], 35, b"\0"),
            &[],
            &[],
        )
        .expect("the chain parses");
        let procs = visuals.procedurals(100).expect("Charge's cast kit runs one");
        assert_eq!(
            procs.cast.trail,
            Some(WeaponTrail { colour: [0xdb, 0x0b, 0x10], alpha: 100, duration_ms: 1000 })
        );
        assert_eq!(procs.impact, KitProcedurals::default());
        assert_eq!(visuals.kit_procedurals(44), Some(&procs.cast));
    }

    /// `charProc` 13 is a timed colour, not an aura colour: kit 68 as shipped
    /// holds for half a second and fades over half a second, and it does not
    /// answer [`SpellVisuals::aura_tint`].
    #[test]
    fn a_timed_colour_holds_then_fades_to_white() {
        let mut spell = vec![0u32; 173];
        spell[0] = 1;
        spell[fields::SPELL_VISUAL] = 2;
        let mut visual = vec![0u32; 16];
        visual[0] = 2;
        visual[fields::STATE_KIT] = 68;
        let mut kit = vec![0u32; 35];
        kit[0] = 68;
        kit[fields::CHAR_PROC] = fields::CHAR_PROC_MODEL_GLOW;
        kit[fields::CHAR_PARAM_ZERO] = (0x0000_ff00 as f32).to_bits();
        kit[fields::CHAR_PARAM_ONE] = 0.5_f32.to_bits();
        kit[fields::CHAR_PARAM_TWO] = 0.5_f32.to_bits();

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[kit], 35, b"\0"),
            &[],
            &[],
        )
        .expect("the chain parses");
        assert_eq!(visuals.aura_tint(1), None);
        let flash = visuals.procedurals(1).and_then(|p| p.state.flash).expect("a flash");
        assert_eq!(flash, ModelFlash { colour: [0, 255, 0], hold_ms: 500, fade_ms: 500 });
        assert_eq!(flash.level(0), Some(255));
        assert_eq!(flash.level(499), Some(255));
        assert_eq!(flash.level(750), Some(127));
        assert_eq!(flash.level(1000), None);
        assert_eq!(flash.blend(255), [0, 255, 0]);
        assert_eq!(flash.blend(0), [255, 255, 255]);
        // 255 + ((0 - 255) * 127 >> 8), with the shift flooring.
        assert_eq!(flash.blend(127), [128, 255, 128]);
    }

    /// `charProc` 14 on a state kit is the aura's opacity. 1.0 is the identity
    /// and is not stored.
    #[test]
    fn an_aura_opacity_is_the_state_kit_s_multiplier() {
        let row = |id: u32, visual: u32| {
            let mut spell = vec![0u32; 173];
            spell[0] = id;
            spell[fields::SPELL_VISUAL] = visual;
            spell
        };
        let visual = |id: u32, kit: u32| {
            let mut row = vec![0u32; 16];
            row[0] = id;
            row[fields::STATE_KIT] = kit;
            row
        };
        let kit = |id: u32, opacity: f32| {
            let mut row = vec![0u32; 35];
            row[0] = id;
            row[fields::CHAR_PROC + 1] = fields::CHAR_PROC_MODEL_OPACITY;
            row[fields::CHAR_PARAM_ZERO + 1] = opacity.to_bits();
            row
        };
        let visuals = SpellVisuals::parse(
            &testing::dbc(&[row(8326, 10), row(3989, 11)], 173, b"\0"),
            &testing::dbc(&[visual(10, 989), visual(11, 3989)], 16, b"\0"),
            &testing::dbc(&[kit(989, 0.5), kit(3989, 1.0)], 35, b"\0"),
            &[],
            &[],
        )
        .expect("the chain parses");
        assert_eq!(visuals.aura_opacity(8326), Some(0.5), "Ghost");
        assert_eq!(visuals.aura_opacity(3989), None, "Possess states 1.0");
    }

    /// A channelled spell states only its channel kit, and that kit's pose is
    /// the held one. Reading only `precastKit` would leave every channel in the
    /// game standing still with a bar running.
    #[test]
    fn a_channel_supplies_the_held_pose() {
        let mut spell = vec![0u32; 173];
        spell[0] = 5143;
        spell[fields::SPELL_VISUAL] = 262;
        let mut visual = vec![0u32; 16];
        visual[0] = 262;
        visual[fields::CHANNEL_KIT] = 729;
        let mut kit = vec![0u32; 35];
        kit[0] = 729;
        kit[fields::ANIMATION] = 124;

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[kit], 35, b"\0"),
            &[],
            &[],
        )
        .expect("three tables");
        assert_eq!(
            visuals.cast(5143),
            Some(CastAnimation {
                hold: Some(124),
                release: None,
                // The same kit twice: a spell with only a channel holds it
                // through both, which is what the fallback in `hold` is for.
                channel: Some(124),
                ranged_shot: false
            })
        );
    }

    /// A spell with both a precast and a channel kit keeps them apart.
    ///
    /// Blizzard's layout: `precastKit` at `ReadySpellOmni` for the wind-up,
    /// `channelKit` at `ChannelCastOmni` for the channel. An earlier version
    /// read the channel only when there was no precast, so `hold` answered 52
    /// and nothing answered 125, and 61 of the game's channelled spells stood
    /// in their wind-up pose for the whole channel. See
    /// [`CastAnimation::channel`].
    #[test]
    fn a_channel_with_a_precast_before_it_states_two_different_poses() {
        let mut spell = vec![0u32; 173];
        spell[0] = 10;
        spell[fields::SPELL_VISUAL] = 259;

        let mut visual = vec![0u32; 16];
        visual[0] = 259;
        visual[fields::PRECAST_KIT] = 197;
        visual[fields::CHANNEL_KIT] = 717;

        let mut wind_up = vec![0u32; 35];
        wind_up[0] = 197;
        wind_up[fields::ANIMATION] = 52;
        let mut channel = vec![0u32; 35];
        channel[0] = 717;
        channel[fields::ANIMATION] = 125;

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[wind_up, channel], 35, b"\0"),
            &[],
            &[],
        )
        .expect("three tables");
        assert_eq!(
            visuals.cast(10),
            Some(CastAnimation {
                hold: Some(52),
                release: None,
                channel: Some(125),
                ranged_shot: false
            })
        );
    }

    /// A ranged attack with no visual still resolves. The chain is joined on
    /// `SpellVisual`, and Auto Shot's is 0, so without
    /// [`CastAnimation::ranged_shot`] a hunter's commonest attack gave a
    /// `cast()` of `None`, the same as a passive.
    ///
    /// The second row checks the other case: a ranged ability that names a kit
    /// keeps the kit's own release, because the file is more specific than the
    /// weapon.
    #[test]
    fn a_ranged_attack_resolves_with_and_without_a_kit() {
        use crate::tables::spellbook::spell_attributes::USES_RANGED_SLOT;
        let mut auto_shot = vec![0u32; 173];
        auto_shot[0] = 75;
        auto_shot[crate::tables::spellbook::spell_fields::ATTRIBUTES] = USES_RANGED_SLOT;
        // …and `SPELL_VISUAL` left at 0, which is the shipped value.

        let mut aimed = vec![0u32; 173];
        aimed[0] = 19434;
        aimed[crate::tables::spellbook::spell_fields::ATTRIBUTES] = USES_RANGED_SLOT;
        aimed[fields::SPELL_VISUAL] = 67;
        let mut fireball = vec![0u32; 173];
        fireball[0] = 133;
        fireball[fields::SPELL_VISUAL] = 67;

        let mut visual = vec![0u32; 16];
        visual[0] = 67;
        visual[fields::CAST_KIT] = 38;
        let mut cast = vec![0u32; 35];
        cast[0] = 38;
        cast[fields::ANIMATION] = 53;

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[auto_shot, aimed, fireball], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[cast], 35, b"\0"),
            &[],
            &[],
        )
        .expect("three tables");

        let shot = visuals.cast(75).expect("a spell with no visual still answers");
        assert!(shot.ranged_shot && shot.release.is_none() && shot.hold.is_none());
        let ability = visuals.cast(19434).expect("Aimed Shot");
        assert!(ability.ranged_shot);
        assert_eq!(ability.release, Some(53), "the file outranks the weapon");
        assert!(!visuals.cast(133).expect("Fireball").ranged_shot);
        // The baseline count is poses, not entries. Two of the three rows reach
        // a kit; the third is in `by_spell` and must not be counted.
        assert_eq!(visuals.counts().1, 2);
        assert_eq!(visuals.ranged_shots(), 2);
    }

    /// `-1` and `0` both mean "no animation", and neither may reach the
    /// renderer: -1 truncates to a `u16` id nothing has, and 0 is Stand, which
    /// would interrupt the caster in order to stand still.
    #[test]
    fn the_tables_two_ways_of_saying_nothing_are_both_refused() {
        let mut spell = vec![0u32; 173];
        spell[0] = 1;
        spell[fields::SPELL_VISUAL] = 9;
        let mut visual = vec![0u32; 16];
        visual[0] = 9;
        visual[fields::PRECAST_KIT] = 5;
        visual[fields::CAST_KIT] = 6;
        let mut none = vec![0u32; 35];
        none[0] = 5;
        none[fields::ANIMATION] = u32::MAX;
        let mut stand = vec![0u32; 35];
        stand[0] = 6;
        stand[fields::ANIMATION] = 0;

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[none, stand], 35, b"\0"),
            &[],
            &[],
        )
        .expect("three tables");
        assert_eq!(visuals.cast(1), None, "an empty visual was stored anyway");
    }

    /// The models, and the two ways they differ from the animation.
    ///
    /// A kit hangs one model per body part, each at its own attachment point,
    /// so the five columns are five different answers rather than a fallback
    /// chain. The effect table is optional, so a chain without it still poses
    /// the caster. Both are asserted here because each fails by drawing
    /// nothing.
    #[test]
    fn a_kits_models_reach_their_own_attachment_points() {
        let mut spell = vec![0u32; 173];
        spell[0] = 133;
        spell[fields::SPELL_VISUAL] = 67;
        let mut visual = vec![0u32; 16];
        visual[0] = 67;
        visual[fields::CAST_KIT] = 38;
        let mut kit = vec![0u32; 35];
        kit[0] = 38;
        kit[fields::ANIMATION] = 53;
        kit[3] = u32::MAX; // head: the table's own "none"
        kit[6] = 7; // left hand
        kit[7] = 7; // right hand
        // …and a sound id where the models stop, which marks the end of the
        // block: 1484 is outside the effect table.
        kit[13] = 1484;

        // `SpellVisualEffectName`: id, name, model, name, scale.
        let effect: Vec<u32> = vec![7, 0, 1, 0, 2.5f32.to_bits()];
        let names = testing::dbc(&[effect], 5, b"\0Spells\\Fire_Cast_Hand.mdx\0");

        let three = || {
            (
                testing::dbc(&[spell.clone()], 173, b"\0"),
                testing::dbc(&[visual.clone()], 16, b"\0"),
                testing::dbc(&[kit.clone()], 35, b"\0"),
            )
        };
        let (s, v, k) = three();
        let visuals = SpellVisuals::parse(&s, &v, &k, &names, &[]).expect("three tables");

        let effects = visuals.effects(133).expect("Fireball hangs models");
        assert!(effects.hold.is_empty(), "the precast kit named none");
        assert_eq!(
            effects.release,
            vec![
                KitEffect {
                    point: attach::SPELL_HAND_LEFT,
                    path: "Spells\\Fire_Cast_Hand.m2".to_string(),
                    scale: 2.5,
                },
                KitEffect {
                    point: attach::SPELL_HAND_RIGHT,
                    path: "Spells\\Fire_Cast_Hand.m2".to_string(),
                    scale: 2.5,
                },
            ],
            "the model went to the wrong point, or the sound was read as one"
        );
        // The pose is unaffected either way.
        assert_eq!(visuals.cast(133).and_then(|c| c.release), Some(53));

        // …and with no effect table at all, the cast is still posed.
        let (s, v, k) = three();
        let posed = SpellVisuals::parse(&s, &v, &k, &[], &[]).expect("three tables");
        assert_eq!(posed.cast(133).and_then(|c| c.release), Some(53));
        assert_eq!(posed.effects(133), None, "models from nowhere");
    }

    /// Blizzard's three area columns, end to end: the model, the rain, and the
    /// gate that must pass before either is read.
    ///
    /// Every number here is from the shipped row: `SpellVisual` 259 reads `1,
    /// 398, 609` at fields 11..13 and `SpellVisualKit` 609 reads `charProc 9,
    /// charParamZero 0.0, charParamOne 5.0`. A column read one along either way
    /// gives `None`; the old fallback chain hid that.
    #[test]
    fn a_persistent_area_is_its_own_three_columns() {
        let mut spell = vec![0u32; 173];
        spell[0] = 10;
        spell[fields::SPELL_VISUAL] = 259;
        let mut visual = vec![0u32; 16];
        visual[0] = 259;
        visual[fields::AREA_FLAG] = 1;
        visual[fields::AREA_MODEL] = 398;
        visual[fields::AREA_KIT] = 609;
        let mut kit = vec![0u32; 35];
        kit[0] = 609;
        kit[fields::CHAR_PROC] = fields::CHAR_PROC_AREA_RAIN;
        kit[fields::CHAR_PARAM_ZERO] = 0.0f32.to_bits();
        kit[fields::CHAR_PARAM_ONE] = 5.0f32.to_bits();

        let effect: Vec<u32> = vec![398, 0, 1, 0, 1.0f32.to_bits()];
        let names = testing::dbc(&[effect], 5, b"\0Spells\\Blizzard_Impact_Base.mdx\0");
        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[kit], 35, b"\0"),
            &names,
            &[],
        )
        .expect("three tables");

        let area = visuals.area(10).expect("Blizzard states an area");
        assert_eq!(area.path, "Spells\\Blizzard_Impact_Base.m2");
        assert_eq!(area.scale, 1.0);
        assert_eq!(
            area.rain,
            Some(AreaRain {
                path: "Spells\\Blizzard_Impact_Base.m2".to_string(),
                rate: 5.0,
            }),
            "charParamZero is a float index into AREA_RAIN_MODELS"
        );
        assert_eq!(visuals.area(11), None, "a spell that is not in the table");
    }

    /// The gate is checked before the other two are read, as the 1.12.1 client
    /// does. 33 shipped rows carry a field-12 id with field 11 clear, and
    /// reading them would put a model on the ground under every `DynamicObject`
    /// whose spell says it has none.
    #[test]
    fn an_area_model_with_the_flag_clear_is_refused() {
        let mut spell = vec![0u32; 173];
        spell[0] = 133;
        spell[fields::SPELL_VISUAL] = 67;
        let mut visual = vec![0u32; 16];
        visual[0] = 67;
        visual[fields::AREA_FLAG] = 0;
        visual[fields::AREA_MODEL] = 398;
        let effect: Vec<u32> = vec![398, 0, 1, 0, 1.0f32.to_bits()];
        let names = testing::dbc(&[effect], 5, b"\0Spells\\Blizzard_Impact_Base.mdx\0");
        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[vec![0u32; 35]], 35, b"\0"),
            &names,
            &[],
        )
        .expect("three tables");
        assert_eq!(visuals.area(133), None);
    }

    /// The rain reads its parameters at the slot the procedural was found in.
    /// The three parameter columns are parallel arrays, and a kit whose
    /// `charProc` is in slot 2 takes its rate from `charParamOne[2]`. Reading
    /// slot 0 would give every one of them Blizzard's five per second.
    #[test]
    fn the_rain_reads_the_slot_its_procedural_is_in() {
        let mut kit = vec![0u32; 35];
        kit[0] = 700;
        // Slot 0 holds a procedural this client does not run; the rain is in 2.
        kit[fields::CHAR_PROC] = 1;
        kit[fields::CHAR_PROC + 2] = fields::CHAR_PROC_AREA_RAIN;
        kit[fields::CHAR_PARAM_ZERO] = 6.0f32.to_bits();
        kit[fields::CHAR_PARAM_ZERO + 2] = 2.0f32.to_bits();
        kit[fields::CHAR_PARAM_ONE] = 5.0f32.to_bits();
        kit[fields::CHAR_PARAM_ONE + 2] = 0.7f32.to_bits();

        let mut spell = vec![0u32; 173];
        spell[0] = 421;
        spell[fields::SPELL_VISUAL] = 125;
        let mut visual = vec![0u32; 16];
        visual[0] = 125;
        visual[fields::AREA_FLAG] = 1;
        visual[fields::AREA_MODEL] = 328;
        visual[fields::AREA_KIT] = 700;
        let effect: Vec<u32> = vec![328, 0, 1, 0, 1.0f32.to_bits()];
        let names = testing::dbc(&[effect], 5, b"\0Spells\\LightningStorm_Cloud_State.mdx\0");

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[kit], 35, b"\0"),
            &names,
            &[],
        )
        .expect("three tables");
        let rain = visuals
            .area(421)
            .and_then(|a| a.rain.clone())
            .expect("Chain Lightning's cloud rains");
        assert_eq!(rain.path, "Spells\\CallLightning_Impact.m2", "index 2");
        assert_eq!(rain.rate, 0.7, "slot 0's 5.0 was read instead");
    }

    /// A `charParamZero` outside the seven-entry list draws nothing rather than
    /// the nearest entry.
    ///
    /// The list's length is the client's, not this project's, so an index past
    /// the end is a row this client does not understand. Clamping it would rain
    /// Star Shards on a spell that asked for something else: a wrong result
    /// that looks right.
    #[test]
    fn a_rain_index_past_the_table_is_dropped() {
        let mut kit = vec![0u32; 35];
        kit[0] = 1;
        kit[fields::CHAR_PROC] = fields::CHAR_PROC_AREA_RAIN;
        kit[fields::CHAR_PARAM_ZERO] = 7.0f32.to_bits();
        kit[fields::CHAR_PARAM_ONE] = 5.0f32.to_bits();

        let mut spell = vec![0u32; 173];
        spell[0] = 1;
        spell[fields::SPELL_VISUAL] = 1;
        let mut visual = vec![0u32; 16];
        visual[0] = 1;
        visual[fields::AREA_FLAG] = 1;
        visual[fields::AREA_MODEL] = 5;
        visual[fields::AREA_KIT] = 1;
        let effect: Vec<u32> = vec![5, 0, 1, 0, 1.0f32.to_bits()];
        let names = testing::dbc(&[effect], 5, b"\0Spells\\Anything.mdx\0");

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&[kit], 35, b"\0"),
            &names,
            &[],
        )
        .expect("three tables");
        let area = visuals.area(1).expect("the model still stands");
        assert_eq!(area.rain, None, "index 7 is past the table's seven entries");
    }

    /// The two kits that are not about the caster. If their columns move, they
    /// fail in opposite directions.
    ///
    /// `impactKit` hangs on the victim and `stateKit` on the unit carrying the
    /// aura. Both are read from columns 3 and 4, between the cast kit and the
    /// channel kit, which are already identified. Swapping the two would not
    /// fail visibly: an impact drawn as a state is a burst that never ends, and
    /// a state drawn as an impact is a buff glow that shows for a second and
    /// disappears. Neither looks like an error, so this test asserts that the
    /// right models come from the right column, not only that some models came
    /// out.
    #[test]
    fn the_impact_and_the_state_are_read_from_their_own_columns() {
        let mut spell = vec![0u32; 173];
        spell[0] = 7302;
        spell[fields::SPELL_VISUAL] = 300;

        let mut visual = vec![0u32; 16];
        visual[0] = 300;
        visual[fields::CAST_KIT] = 10;
        visual[fields::IMPACT_KIT] = 11;
        visual[fields::STATE_KIT] = 12;

        // Three kits, each naming one model at a point of its own — so a column
        // read as its neighbour lands on an attachment that is visibly wrong.
        let kit = |id: u32, field: usize, effect: u32| {
            let mut kit = vec![0u32; 35];
            kit[0] = id;
            kit[fields::ANIMATION] = u32::MAX;
            kit[field] = effect;
            kit
        };
        let kits = [
            kit(10, 7, 1),  // cast: right hand
            kit(11, 3, 2),  // impact: head
            kit(12, 6, 3),  // state: left hand
        ];
        // The string block, built rather than written out, so a model's offset
        // cannot drift from where its bytes actually are.
        let mut strings: Vec<u8> = vec![0];
        let mut names: Vec<Vec<u32>> = Vec::new();
        for (id, name) in [
            "Ice_Cast_Hand.mdx",
            "IceArmor_Low_Head.mdx",
            "IceArmor_State_Hand.mdx",
        ]
        .into_iter()
        .enumerate()
        {
            names.push(vec![id as u32 + 1, 0, strings.len() as u32, 0, 1.0f32.to_bits()]);
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
        }

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[visual], 16, b"\0"),
            &testing::dbc(&kits, 35, b"\0"),
            &testing::dbc(&names, 5, &strings),
            &[],
        )
        .expect("three tables");

        let effects = visuals.effects(7302).expect("a visual");
        let paths = |list: &[KitEffect]| -> Vec<(u32, String)> {
            list.iter().map(|e| (e.point, e.path.clone())).collect()
        };
        assert_eq!(
            paths(&effects.release),
            vec![(attach::SPELL_HAND_RIGHT, "Ice_Cast_Hand.m2".to_string())]
        );
        assert_eq!(
            paths(&effects.impact),
            vec![(attach::HEAD, "IceArmor_Low_Head.m2".to_string())],
            "the impact came out of the wrong column"
        );
        assert_eq!(
            paths(visuals.state(7302).expect("a state")),
            vec![(attach::SPELL_HAND_LEFT, "IceArmor_State_Hand.m2".to_string())],
            "the state came out of the wrong column"
        );
        assert_eq!(visuals.impact_counts(), (1, 1));
        assert_eq!(visuals.state_counts(), (1, 1));

        // The state is not part of the cast, and asking for one must not return
        // the other: a spell whose only visual is a worn aura has no cast
        // effects, and one with a cast and no aura has no state.
        assert_eq!(visuals.state(999), None);
        assert!(visuals.effects(999).is_none());
    }

    /// The missile is described in `SpellVisual`, one table before the kits,
    /// and its speed in `Spell.dbc`, one table after. `SpellVisual` says that
    /// there is one and which model; only `Spell.dbc` says how fast it flies. A
    /// reader that takes the whole description from one row gets a projectile
    /// that either never leaves the caster's hand or arrives before it is
    /// drawn.
    #[test]
    fn a_missile_crosses_two_tables_for_its_model_and_its_speed() {
        let mut spell = vec![0u32; 173];
        spell[0] = 133;
        spell[fields::SPELL_VISUAL] = 67;
        spell[fields::SPELL_SPEED] = 24.0f32.to_bits();

        let mut visual = vec![0u32; 16];
        visual[0] = 67;
        visual[fields::CAST_KIT] = 38;
        // Fireball's own five: has one, model 365, path 0, attachment 1, and a
        // sound id after the block, which shows where the block ends.
        visual[fields::HAS_MISSILE] = 1;
        visual[fields::MISSILE_MODEL] = 365;
        visual[fields::MISSILE_PATH_TYPE] = 0;
        visual[fields::MISSILE_DESTINATION] = 1;
        visual[10] = 3011;

        let mut kit = vec![0u32; 35];
        kit[0] = 38;
        kit[fields::ANIMATION] = 53;

        let effect: Vec<u32> = vec![365, 0, 1, 0, 1.0f32.to_bits()];
        let names = testing::dbc(&[effect], 5, b"\0Spells\\Fireball_Missile_Low.mdx\0");

        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell.clone()], 173, b"\0"),
            &testing::dbc(&[visual.clone()], 16, b"\0"),
            &testing::dbc(&[kit.clone()], 35, b"\0"),
            &names,
            &[],
        )
        .expect("three tables");
        assert_eq!(
            visuals.missile(133),
            Some(&Missile {
                path: "Spells\\Fireball_Missile_Low.m2".to_string(),
                scale: 1.0,
                speed: 24.0,
                path_type: 0,
                destination: 1,
            })
        );
        assert_eq!(visuals.missile_counts(), (1, 1, 0));

        // The flag alone is not a missile. A visual that sets it and names no
        // model with shipped art has nothing to throw, and an entry built from
        // the flag would make the renderer look up an empty path on every cast.
        let mut no_art = visual.clone();
        no_art[fields::MISSILE_MODEL] = 999;
        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell.clone()], 173, b"\0"),
            &testing::dbc(&[no_art], 16, b"\0"),
            &testing::dbc(&[kit.clone()], 35, b"\0"),
            &names,
            &[],
        )
        .expect("three tables");
        assert_eq!(visuals.missile(133), None);

        // …and a spell whose visual throws nothing throws nothing, whatever
        // speed its own row states.
        let mut none = visual;
        none[fields::HAS_MISSILE] = 0;
        let visuals = SpellVisuals::parse(
            &testing::dbc(&[spell], 173, b"\0"),
            &testing::dbc(&[none], 16, b"\0"),
            &testing::dbc(&[kit], 35, b"\0"),
            &names,
            &[],
        )
        .expect("three tables");
        assert_eq!(visuals.missile(133), None);
        assert_eq!(visuals.missile_counts(), (0, 0, 0));
    }

    /// A chain missing one table is no chain: the answer would be "no spell in
    /// the game has an animation", which is indistinguishable from the table
    /// being read correctly and the game having none.
    #[test]
    fn all_three_tables_or_none() {
        assert!(SpellVisuals::parse(&[], &[], &[], &[], &[]).is_none());
    }
}
