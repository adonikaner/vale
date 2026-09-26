//! What a spell **looks like** on the caster: `Spell` -> `SpellVisual` ->
//! `SpellVisualKit` -> an `AnimationData` id.
//!
//! **The protocol carries a spell id and nothing else, and every cast in the
//! game is a different pose.** `SMSG_SPELL_START` and `SMSG_SPELL_GO` say who
//! is casting what; which animation that *is* lives in three DBCs, and a client
//! that skips them has one wind-up and one release for the whole game — which
//! is what this one had, and it showed as a character raising both hands over
//! their head to open a chest.
//!
//! The chain is three hops and each is a single field:
//!
//! ```text
//!   Spell.dbc          [115] SpellVisualID
//!   SpellVisual.dbc    [1] precastKit  [2] castKit  [5] channelKit
//!   SpellVisualKit.dbc [2] animID       (-1, and 0, mean none)
//! ```
//!
//! **Every index here is measured, and the measurement is self-checking** in
//! the same way the `Emotes.dbc` animation column was: the tables name their
//! rows independently, so a wrong column cannot agree with the spell's own name
//! fifty times over. What comes out is exactly what the retail client does on
//! screen:
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
//! A spell thrown at a target is *directed* and one cast on the caster is
//! *omni*; opening a chest and bandaging a wound are neither, and are the
//! reason the two ids `AnimationData` calls `SpellPrecast` and `SpellCast` are
//! carried by no character model in the game. The client never asks for them.
//!
//! Only the **caster's** half of the *animation* is read. `impactKit`'s is the
//! pose played on the victim — Fireball's is `CombatWound` — and this client
//! already flinches off `SMSG_ATTACKERSTATEUPDATE`. Its *models* are another
//! matter; see below.
//!
//! ## The kit also names *models*, and that is the visible half
//!
//! A pose alone is a character miming a fireball. What makes it a fireball is
//! the five model columns beside the animation, each of which names a
//! `SpellVisualEffectName` row — a `.mdx` under `Spells\` or `Particles\` whose
//! whole content is emitters — and each of which hangs from a **different
//! attachment point on the caster**:
//!
//! ```text
//!   SpellVisualKit.dbc  [3] head  [4] chest  [5] base
//!                       [6] left hand  [7] right hand
//!   SpellVisualEffectName.dbc  [2] model path   [4] scale
//! ```
//!
//! **And the same five columns are asked of the two kits that are not about the
//! caster at all** — `impactKit` (field 3 of `SpellVisual`), whose models hang
//! on the **victim** when the spell lands, and `stateKit` (field 4), whose
//! models a unit wears for as long as the aura is on it. Those two are most of
//! the visible half of the chain: **8,751 spells burst and 1,923 are worn,
//! against 898 that throw a missile** — so the overwhelming majority land the
//! instant they are released, including every self-cast buff, where the caster
//! is also the victim and the impact is the *only* thing on screen.
//!
//! **The columns are pinned by what they resolve to, not by a reference.** Every
//! one of the five holds either `-1`, `0`, or an id inside
//! `SpellVisualEffectName`'s 782 rows — a sound id (field 13, 1484 for Fireball)
//! lands outside it immediately, which is what separates the model block from
//! the tail. `vale spell` reports the share per column, and the model names
//! that come back read as their own columns: Fireball's cast kit puts
//! `Fireball_Cast_Hand` on **both** hands and nothing anywhere else.
//!
//! ## …and `SpellVisual`'s own tail is the *area*, which is a different subject
//!
//! Fields 11, 12 and 13 sat unread for the life of this module, with a note on
//! [`SpellVisuals::area`]'s ancestor saying that if one of them turned out to be
//! the persistent-area kit it would replace the fallback chain that stood in for
//! it. **Both of them are, and they are two halves of one answer** — see
//! [`fields::AREA_FLAG`], [`fields::AREA_MODEL`] and [`fields::AREA_KIT`]:
//!
//! ```text
//!   SpellVisual.dbc [11] hasAreaEffect   [12] SpellVisualEffectName  [13] SpellVisualKit
//! ```
//!
//! The client settles all three at once. A dynamic object's own visual setup
//! reads `DYNAMICOBJECT_SPELLID`,
//! walks `Spell` -> `SpellVisual`, and **refuses outright unless field 11 is
//! non-zero** — logging `SPELLEFFECTNOAREAEFFECT|<spell>`, which is the field's
//! own name. Field 12 is then read against
//! `SpellVisualEffectName`, failing with `SPELLEFFECTIDNOTFOUND`, and its model
//! is what the object is drawn as; field 13 is read against
//! `SpellVisualKit`.
//!
//! **So the ground art is a measurement now, where it used to be a judgement.**
//! What stood here before was a fallback chain over the five *caster* kits,
//! filtered to attachment 19, preferring the kit whose meaning was "a condition
//! that holds" — and it was careful and it was wrong for Flamestrike, which is
//! the report that paid for this. Flamestrike's `SpellVisual` (33) names
//! `Spells\Flamestrike_Impact_Base.mdx` at field 12: 454 vertices, a lava-ground
//! quad and four ribbons. The chain picked its `stateKit`'s
//! `Spells\Immolate_State_Base.m2` instead — **zero vertices and two emitters**,
//! a candle where the file says a bonfire.
//!
//! The population is its own check: **217 of 2,167 `SpellVisual` rows set field
//! 11, covering 722 spells, and all 217 field-12 ids resolve in
//! `SpellVisualEffectName`** — a column read one along would miss.
//!
//! ## And one of those areas *rains*, which is a procedural the client owns
//!
//! A Blizzard is not a model sitting on the ground. `SpellVisualKit` carries
//! four `charProc` slots (fields 15..18) with three parallel parameter columns
//! after them, and **`charProc == 9` is the falling-impact procedural**,
//! which spawns one instance of a model per unit of an accumulator ticking at a
//! rate the kit states.
//!
//! ```text
//!   SpellVisualKit.dbc [15..18] charProc  [19..22] charParamZero  [23..26] charParamOne
//!   charProc     9  -> the falling-impact procedural
//!   charParamZero   -> an index, held as a float, into the client's own seven-model table
//!   charParamOne    -> impacts per second
//! ```
//!
//! The model table is [`AREA_RAIN_MODELS`] — seven entries — and it is **not**
//! in any file. The parameters read as
//! their own columns: Blizzard's kit (609) is `charProc 9, charParamZero 0.0,
//! charParamOne 5.0`, which is index 0 = `Spells\Blizzard_Impact_Base.mdx` at
//! five a second, and that model's declared box is
//! `[-12.5, -12.6, -5.0]..[11.9, 11.8, 30.7]` — **one shard falling thirty
//! yards**, not a field of snow. Eight kits carry the procedural in the shipped
//! data and between them they are Blizzard, Rain of Fire, Volley, Hurricane,
//! Death & Decay, Lightning Cloud, Starfall, Manastorm and Chill.
//!
//! One entry of the seven — index 3, `FlamestrikeSmall_Impact_Base` — is named
//! by no shipped kit. Stated rather than trimmed: the table is the client's and
//! its length is not this project's to choose.

use crate::tables::dbc::Dbc;
use crate::world::m2::{attach, model_path};
use std::collections::HashMap;

/// **What a `SpellVisualEffectName` scale column means**, including when it is
/// zero.
///
/// 197 of the table's 782 rows carry **0.0** — Fireball's own precast and cast
/// hands among them — and a renderer that multiplied by it would draw nothing
/// at all for a quarter of the game's spell effects. Zero is the column's
/// unset, not a size, and so is anything that is not finite.
///
/// It is a function rather than a line inside the parser because it is now read
/// from two places: the chain below, and an editor showing somebody the number
/// in the file. Both have to agree about what the file means, and the one that
/// does not is the one that reads "x0.00" beside a model that is plainly
/// visible in the game.
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
    /// `SpellVisualEffectName` field 4. Applied to the effect model and to
    /// nothing else — it is the *effect's* size, not the caster's.
    pub scale: f32,
}

/// **What a persistent area of a spell is drawn as** — a Blizzard's patch of
/// sky, a Flamestrike's fire, a Consecration, a Rain of Fire.
///
/// The server puts one `DynamicObject` in the world per such area and it states
/// a caster, a spell id and a radius; nothing on the wire says what it looks
/// like. This is the whole of the answer, and both halves come off the spell's
/// own `SpellVisual` row — see the module comment for the three fields and the
/// three addresses that pin them.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaEffect {
    /// The model the object *is*, from `SpellVisual` field 12 through
    /// `SpellVisualEffectName` — already through [`model_path`].
    pub path: String,
    /// That row's own field 4, exactly as a [`KitEffect`]'s scale is.
    pub scale: f32,
    /// …and the second half, for the ten visuals that have one: a rain of
    /// impacts inside the area rather than one model standing in it.
    pub rain: Option<AreaRain>,
}

/// The falling-impact procedural — `charProc == 9`.
///
/// **This is not one model with a long animation.** The client keeps a
/// fractional accumulator per area, adds `dt * rate` to it every frame and
/// spawns one instance of [`Self::path`] for each whole unit in it,
/// each at its own place inside the radius and on its own clock.
/// Drawing the model once instead is a single snowflake landing at the start of
/// a Blizzard and nothing afterwards, which is exactly what it looked like.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaRain {
    /// One of [`AREA_RAIN_MODELS`], through [`model_path`].
    pub path: String,
    /// `charParamOne`, in impacts per second. 0.7 for a Lightning Cloud, 5.0
    /// for a Blizzard, 15.0 for Chill.
    pub rate: f32,
}

/// **A bolt of lightning strung between two units** - one row of
/// `SpellChainEffects.dbc`, which is the only table in this crate that
/// describes a *shape* rather than naming a model.
///
/// Chain Lightning, Chain Heal, Drain Life, Mind Flay, Health Funnel and the
/// Rallying Cry buffs are all this. There is no `.m2` anywhere in the chain for
/// them: the client builds the geometry itself out of these seven numbers,
/// which is why a client that reads every other column of every other spell
/// table still draws nothing at all for them.
///
/// ```text
///   SpellChainEffects.dbc  [1] avgSegLen      [2] width       [3] noiseScale
///                          [4] texCoordScale  [5] segDuration [6] segDelay
///                          [7] texture
/// ```
///
/// The client reads seven consecutive four-byte fields and then the texture,
/// and uses the first four as they are and the fifth and sixth as
/// **milliseconds** - see [`Self::seg_duration_ms`].
#[derive(Debug, Clone, PartialEq)]
pub struct ChainEffect {
    /// `Textures\SpellChainEffects\Lightning.blp` and seven siblings, as the
    /// archive spells it. **A texture, not a model** - the strip it is wrapped
    /// around is the client's own geometry.
    pub texture: String,
    /// How long one straightened segment of the bolt is, in yards - 2.78 on
    /// every one of the eighteen rows. What it decides is how *jagged* the
    /// polyline is: a 30-yard cast at 2.78 is eleven kinks.
    pub avg_seg_len: f32,
    /// Half-width of the drawn strip, in yards. 0.5 for the lightning, 0.25 for
    /// the beams.
    pub width: f32,
    /// How far off the straight line the kinks wander, as a fraction of its
    /// length. 0.04 for the lightning, 0.001 for `HealBeam` - which is why one
    /// crackles and the other is a straight ribbon.
    pub noise_scale: f32,
    /// How fast the texture repeats along the bolt. **Negative on three rows**,
    /// which flips it; kept signed rather than clamped, because the sign is the
    /// file's.
    pub tex_coord_scale: f32,
    /// How long one hop of the chain lasts, in milliseconds - 1000 for the
    /// lightning, 2000 for `HealBeam`. The client *adds* it to a start time, which is what makes it a duration
    /// rather than a rate.
    pub seg_duration_ms: u32,
    /// ...and how long each hop waits behind the one before it - 300 ms for the
    /// lightning. `start = now + hop_index * segDelay`, which is
    /// the whole of what makes a Chain Lightning *chain* rather than fork.
    pub seg_delay_ms: u32,
}

/// ...and what one spell's kit says to do with it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainVisual {
    pub effect: ChainEffect,
    /// `charParamOne`, **clamped to 3** - the client's own ceiling
    /// Every shipped kit but one states 1.
    pub bolts: u32,
    /// `charParamTwo != 0` - whether the bolt is **held** rather than fired
    /// once.
    ///
    /// The flag is the client's; reading it as "held" is
    /// this project's, and it is the interpretation the population supports:
    /// 40 of the 48 kits that carry a chain set it, every one of those 40 hangs
    /// off a `channelKit`, and every one that leaves it clear hangs off a cast.
    /// A Drain Life against a Chain Lightning.
    pub held: bool,
}

/// The largest number of bolts one chain may have - the client's own clamp.
pub const MAX_CHAIN_BOLTS: u32 = 3;

/// The seven models the falling-impact procedural can rain, in the client's own
/// order, indexed by `charParamZero`.
///
/// **Not in any file.** `charParamZero` is a bare float index and these strings
/// are built into the client; a client that does not carry the same table in
/// the same order draws the wrong spell's impact, silently, for the six of them
/// that are reachable. Left at seven entries even though index 3 is named by no
/// shipped kit, because the table's length is the client's and not ours.
/// The prefix on every `SpellVisualEffectName` row the engine reaches by name
/// rather than through `SpellVisual` — see [`SpellVisuals::hardcoded`].
pub const HARDCODED: &str = "HARDCODED ";

/// **The lootable-corpse sparkle's row name.** `Particles\LootFX.m2` in 5875:
/// zero render batches, two textures (`Flare.blp` and `Star5A.blp`), four
/// particle emitters and **one sequence, `3333..5733 ms`, flagged to loop** —
/// so it is entirely emitters and it holds rather than playing out, which is
/// what makes it a state rather than a one-shot. Measured with
/// `vale model 'Particles\LootFX.m2'`.
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

/// **A colour a kit paints its bearer's own model with** — `charProc == 1`.
///
/// This is the whole of what a dwarf's Stoneform looks like, and of Ghost,
/// Shadowform, Ice Block, Banish, Petrify, Frost Nova's chill and eighty more:
/// the unit's *own* model is drawn through a flat colour for as long as the kit
/// that named it is live. Nothing else in the chain says so — the five model
/// columns hang art *on* a unit and this changes the unit itself — and no
/// packet carries it either, which is what left every one of those auras
/// looking exactly like an unaffected character.
///
/// ## The colour is a float holding a packed integer
///
/// `charParamZero` is an `f32` whose **value**, truncated, is `0xRRGGBB` — the
/// same trick [`AreaRain`]'s model index uses one column along, and exact for
/// every 24-bit integer because an `f32` mantissa is 24 bits.
///
/// **The whole path is short.** Case **1** truncates `charParamZero` to an
/// integer and ORs in `0xff000000` —
/// **the alpha is the client's, forced opaque, and is not in the data at all**
/// — and the result is pushed on a per-unit list. When it is read back, the
/// three bytes come off high-to-low as red, green, blue, each scaled by
/// `1/255`, and land as the model's colour. The default when the list is
/// empty is `0xffffffff` — **white, which is the identity of a multiply.**
///
/// The shipped table then names itself, which is what says the *column* is
/// right rather than merely the arithmetic — `vale spell` decodes all
/// **106** rows of `charProc` 1 and 13 and every one lands inside
/// `0x000000..0xffffff`:
///
/// ```text
///   Glowy (Red)     #ff0000     Glowy (Green)   #04f010
///   Glowy (Blue)    #2d32ff     Glowy (Yellow)  #f3ff0f
///   Glowy (Orange)  #ff9b06     Glowy (Purple)  #ff09ff
///   Glowy (Black)   #000000     Vertex Color: Light Blue  #aafbff
/// ```
///
/// Eight rows whose names *are* their colours, and the spells agree with them:
/// Stone Skin `#787878` and Petrify `#969696` are greys, Ghost `#8cb9fd` is
/// pale blue, every Freeze, Chill and Frost row is blue, Immolate `#e75641` is
/// flame, Enrage `#ff1117` is red, Shadowform `#270042` and Banish `#350035`
/// are dark purple — and **Stoneform is `#44465e`**, a dark blue-grey, which is
/// the report this was found from.
///
/// ## …and it multiplies, because white is what the absence of it means
///
/// The client uses `0xffffffff` when the list is empty, and a value that is
/// the **identity of a multiply** is not one an additive term would take. The
/// population agrees with it: Stone Skin's 50% grey, Shadowform's tenth and
/// Banish's fifth are all *darker* than the model they are cast on, where an
/// added glow would brighten. The product is taken here as one multiply
/// inside the lit branch.
///
/// ## Two things that are in the same family and are **not** read
///
/// * **The scale**, which is what a report of Stoneform making a dwarf bigger
///   would have to be. It is real and it is a *different* procedural — cases
///   **14** and **15**, which multiply the model's
///   base scale by `charParamZero` and ease it in over 1,000 and 500 ms
///   respectively, beside the tint.
///   **Stoneform's kit carries neither**: its only procedural is case 1, so
///   the client tints the dwarf and does not resize him. 11 kit rows in the
///   whole game state case 14, and none of them is read here.
/// * **A ramp on the colour.** Case 1 reads `charParamZero` and stops — the
///   other three columns of its slot are never touched, so Stoneform's
///   `0.2, 5.0` are dead data in the shipped file. Case **13** is
///   the one that does ramp, in milliseconds off `charParamOne × 1000` and
///   `charParamTwo × 1000`, which is why its eight `Glowy` rows state `1e7`
///   there: a fade so slow it is no fade. This client draws 13's colour flat,
///   which for those rows is what it looks like anyway.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelTint {
    /// `0xRRGGBB` as the row states it, unpacked. Bytes, in the file's own
    /// space — the same space every other colour in this game is added and
    /// multiplied in.
    pub colour: [u8; 3],
    /// `charParamOne`, `charParamTwo` and `charParamThree` of the same slot,
    /// **measured and not read** — and for `charProc` 1 the client does not
    /// read them either.
    ///
    /// They are carried rather than dropped because they are the evidence that
    /// the *slot* was found: a colour declared in slot 2 whose parameters come
    /// back as slot 0's is a row read off by three columns, and there is a test
    /// for exactly that. What they mean depends on which procedural stated the
    /// colour — case 1 ignores them outright (Stoneform's `0.2, 5.0` are dead
    /// data in the shipped file), and case 13 takes the first two as a fade in
    /// milliseconds. Nothing here interpolates.
    pub params: [f32; 3],
}

/// The projectile a spell throws, when it throws one.
///
/// **This is a different question from the kit's models and it is answered one
/// table earlier.** A `SpellVisualKit` model hangs on the caster; a missile is
/// a model of its own that flies from the caster to the victim and is described
/// by `SpellVisual` itself — Fireball states `1, 365, 0, 1, 3011` in the five
/// columns after the kits, which is *has one*, `Spells\Fireball_Missile_Low`,
/// path type 0, destination attachment 1 and a sound. Nothing in either kit
/// mentions it, which is why a client that reads only the kits throws a spell
/// with no fireball in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Missile {
    /// Through [`model_path`], so it is the `.m2` the archive holds.
    pub path: String,
    /// `SpellVisualEffectName` field 4, exactly as a kit model's is.
    pub scale: f32,
    /// **`Spell.dbc` field 37, in yards per second** — and it is on the *spell*
    /// rather than on the visual, which is the one place this chain crosses
    /// back. Fireball is 24.0, Frostbolt 28.0. Zero for a spell whose visual
    /// names a missile and whose row states no speed; the renderer treats that
    /// as instant, since the alternative is a projectile that never arrives.
    pub speed: f32,
    /// `SpellVisual` field 8. 0 for the ordinary straight line; the arcing
    /// variants are not distinguished by this client, which flies every missile
    /// straight. Kept because the count is what would say whether that matters.
    pub path_type: u32,
    /// `SpellVisual` field 9 — the [`attach`] point on the **victim** the
    /// missile is aimed at. Read and reported; the renderer aims at the
    /// target's mid-height instead, because hitting the point itself needs the
    /// victim's own posed skeleton and the missile is not attached to it.
    pub destination: u32,
}

/// The two poses a spell puts its caster in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CastAnimation {
    /// Held for as long as the cast bar runs — `precastKit`, or the channel
    /// kit for a spell that has no wind-up of its own.
    ///
    /// **A channelled spell has no precast and no cast kit at all**: Arcane
    /// Missiles states only `channelKit`, so treating the two as separate
    /// questions would leave every channel standing still.
    pub hold: Option<u16>,
    /// The one-shot at the end — `castKit`.
    pub release: Option<u16>,
    /// **The pose held while a channel runs** — `channelKit`'s own `animID`,
    /// and its own question rather than a fallback for [`Self::hold`].
    ///
    /// A channel has *two* held poses and they are not the same one. Blizzard
    /// (10) states `precastKit 197` at `ReadySpellOmni` for the wind-up and
    /// `channelKit 717` at `ChannelCastOmni` for the channel; reading the
    /// channel only when there is no precast — which is what this chain did —
    /// left the caster standing in the wind-up for the whole channel and never
    /// asked for the channel kit's models at all.
    ///
    /// **Measured**: 45 `SpellVisual` rows state both kits and 41 of those
    /// give the two different animations. Across the 323 spells
    /// `AttributesEx` calls channelled, 63 have both and **61 played the wrong
    /// pose**. The other 82 rows state a channel kit and no precast, which is
    /// Arcane Missiles and is why [`Self::hold`] keeps its fallback.
    pub channel: Option<u16>,
    /// **…and the one spell family whose release is not in any of these
    /// tables**: a ranged weapon attack, whose one-shot is the *wielder's own
    /// weapon* rather than the spell's.
    ///
    /// This is off `Spell.dbc`'s own `Attributes`
    /// ([`crate::tables::spellbook::SpellInfo::uses_ranged_slot`]) rather than off the
    /// visual chain, and it is here — on the animation rather than beside it —
    /// because it answers the same question [`Self::release`] does and the
    /// caller must not be able to have one without asking about the other.
    ///
    /// **Why it is needed at all is measured**: Auto Shot (75) and the wand's
    /// Shoot (5019), which are the game's only two auto-repeating ranged
    /// attacks, both read `SpellVisual = 0` — no visual, therefore no kit,
    /// therefore no `animID`. So the chain that names every other pose in the
    /// game says *nothing* about the commonest attack a hunter makes, and a
    /// client that only reads the chain draws an archer standing still. See
    /// `world::entities::pose`, which resolves it against the drawn weapon.
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

/// `Spell.dbc` crossed with `SpellVisual.dbc` and `SpellVisualKit.dbc`, reduced
/// to the one question the renderer asks.
///
/// **The three raw tables are not kept.** `Spell.dbc` alone is 22,360 records
/// of 173 fields — 15 MB of mostly gameplay rules this client has no use for —
/// and the answer is two `u16`s per spell. The reduction happens once, at load.
pub struct SpellVisuals {
    /// Spell id -> what its caster does. Only the spells that resolve to at
    /// least one animation are in here.
    by_spell: HashMap<u32, CastAnimation>,
    /// Spell id -> the models its two kits hang on the caster.
    ///
    /// Separate from [`Self::by_spell`] rather than a field on `CastAnimation`
    /// because they are answered by different columns and a spell can have
    /// either without the other: `Fireball`'s precast kit is a pose with no
    /// models, and plenty of buff visuals are models with no pose. Keeping them
    /// together would have made "no animation" mean "no effect" as well, which
    /// is the failure that reads as *nothing happening*.
    effects: HashMap<u32, CastEffects>,
    /// **The visuals the engine spawns**, by their own name — see
    /// [`Self::hardcoded`].
    hardcoded: HashMap<String, (String, f32)>,
    /// Spell id -> the projectile it throws. A third map for the same reason
    /// the second one exists: a spell can have a missile and no pose (a trap's
    /// bolt) or a pose and no missile (every heal in the game), so folding them
    /// together would make one absence mean the other.
    missiles: HashMap<u32, Missile>,
    /// Spell id -> the models a unit wears while this spell's aura is on it.
    ///
    /// A fourth map, and the reason it is not a field of [`CastEffects`] is the
    /// question it answers: the other three are "what happened", asked once per
    /// cast, and this is "what is true", asked of every unit in view against its
    /// `UNIT_FIELD_AURA` slots. A spell can state a state kit and no cast kit
    /// (most weapon enchants) or a cast kit and no state kit (every direct
    /// damage spell in the game), so folding them together would make one
    /// absence mean the other — the same reason the effects are not part of
    /// [`CastAnimation`].
    states: HashMap<u32, Vec<KitEffect>>,
    /// Spell id -> the pose a unit **holds** for as long as this spell's aura is
    /// on it — the state kit's own `animID`, the same column of the same row the
    /// models above come from.
    ///
    /// **This is what a stun looks like, and nothing else in the game says so.**
    /// `UNIT_FLAG_STUNNED` says a unit may not act and `MOVEFLAG_ROOT` says it
    /// may not move; neither is a pose, and the client's own idle cascade
    /// (swimming -> `SwimIdle`, then `StealthStand`, then `Hover`,
    /// else `Stand`) does not mention either. Hammer of Justice's state kit
    /// (349) reads `animID = 14` — `Stun` — beside the `StunSwirl_State_Head`
    /// this client was already drawing, so the cower and the stars over the head
    /// are two columns of one row and only one of them was read.
    ///
    /// Separate from [`Self::states`] for the reason [`Self::effects`] is
    /// separate from [`Self::by_spell`]: a kit can state a pose with no models
    /// (a sit, a kneel) or models with no pose (every weapon enchant), and
    /// folding them together makes one absence mean the other.
    state_anim: HashMap<u32, u16>,
    /// Spell id -> the colour a unit's **own model** is drawn through while
    /// this spell's aura is on it — the same kit's `charProc` slot. A seventh
    /// map on the same terms as the six around it: a kit can state a colour and
    /// no models (Stoneform's does exactly that) or models and no colour (most
    /// of them), so folding them together makes one absence mean the other.
    /// See [`ModelTint`].
    state_tint: HashMap<u32, ModelTint>,
    /// Spell id -> what its **persistent area** is drawn as, for the 722 spells
    /// that have one.
    ///
    /// A fifth map, on the same terms as the four above: it is answered by three
    /// columns none of the others read, and it is asked of a `DynamicObject`
    /// rather than of a cast or of an aura slot. A spell can state an area and
    /// no kits at all (a summoned cloud), and every direct-damage spell in the
    /// game states kits and no area.
    areas: HashMap<u32, AreaEffect>,
    /// Spell id -> **the bolt it strings between units**, for the 192 spells
    /// that have one.
    ///
    /// An eighth map, on the same terms as the seven above and with the
    /// sharpest version of the argument: it is answered by a table
    /// (`SpellChainEffects.dbc`) no other map touches, reached through a
    /// `charProc` slot no other map reads, and it names **no model at all** -
    /// so Chain Heal states a cast kit whose only content is this, and 22,000
    /// spells state everything else and no chain.
    chains: HashMap<u32, ChainVisual>,
    /// **`SpellVisualKit` id -> the models it hangs, and the pose it holds** —
    /// the two maps every other field here is *built out of*, kept rather than
    /// consumed.
    ///
    /// Every other question in this type is asked spell-first, so these two
    /// used to be locals inside [`Self::parse`]: walked once to fill the
    /// spell-keyed maps and dropped. Two packets ask kit-first —
    /// `SMSG_PLAY_SPELL_VISUAL` and `SMSG_PLAY_SPELL_IMPACT`, whose body is a
    /// guid and a kit id with no spell anywhere in it — and there was no way in.
    ///
    /// They are the whole answer for those two, and it is not a small one:
    /// kits 406 and 438 are what eating and drinking look like, and vmangos
    /// sends one of them on every regeneration tick a character spends sitting
    /// with food. See [`crate::tables::sound::SoundBank::kit_sound`], which is
    /// the same keeping-what-was-thrown-away in the sound tables.
    kits: HashMap<u32, Vec<KitEffect>>,
    kit_poses: HashMap<u32, u16>,
    /// The spells at least one of whose effects targets the **caster**.
    ///
    /// A set rather than a map because the question is a yes or no, and it is
    /// asked only of a release that named nobody — see
    /// [`fields::EFFECT_TARGET_A`].
    self_cast: std::collections::HashSet<u32>,
    /// How many spells the table has, against how many resolved — the pair
    /// `vale spell` reports, and the one that says whether a column moved.
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
    /// Shown on **the victim** when the spell lands — the `impactKit`'s models.
    ///
    /// The one set here that hangs on somebody other than the caster, and the
    /// reason it lives beside the other two anyway: it is answered by the same
    /// `SpellVisual` row, at the same time, by the same lookup. Which unit wears
    /// it is the renderer's business.
    pub impact: Vec<KitEffect>,
    /// **Shown while a channel runs** — the `channelKit`'s models, on the same
    /// terms as [`CastAnimation::channel`] and for the same reason.
    ///
    /// [`Self::hold`] takes these only when there is no precast kit, so a
    /// spell stating both hung the wind-up's models for the channel and never
    /// the channel's. Blizzard is the case: its channel kit is what the falling
    /// ice is hung off.
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

/// **Every column index this module measured**, published.
///
/// It was private for as long as this file was the only reader of it. The
/// schema in [`crate::tables::schema`] is the second, and it describes these
/// same tables for an editor — so either the indices are shared or there are
/// two readings of one layout, which is the shape this repository has paid for
/// before. See `schema`'s own test, which asserts every column it names against
/// the constants here.
pub mod fields {
    /// `Spell.dbc` field 115, `SpellVisualID`.
    ///
    /// Measured against the table it points into rather than counted out of a
    /// layout: the four spells sampled read 67, 180, 285 and 5622, and **all
    /// four are rows in `SpellVisual.dbc`** — including 5622, which is well
    /// past that table's 2,167 records and so could not have been a coincidence
    /// of a dense column. The neighbours are the spell icon (117) and priority,
    /// which are small integers that would also have resolved to *something*.
    pub const SPELL_VISUAL: usize = 115;

    /// `SpellVisual.dbc`: id, precastKit, castKit, impactKit, stateKit,
    /// channelKit, then the missile description — `hasMissile`, its model, its
    /// path type, its destination attachment and its sound, which is what pins
    /// the tail (Fireball reads 1, 365, 0, 1, 3011 there).
    pub const PRECAST_KIT: usize = 1;
    pub const CAST_KIT: usize = 2;
    /// **The kit played on the *victim*, not the caster** — what a spell does
    /// when it lands. Fireball's is 286, whose animation is `CombatWound` (the
    /// flinch this client already plays off `SMSG_ATTACKERSTATEUPDATE`) and
    /// whose models are the burst.
    ///
    /// Nothing on the wire announces it. `SMSG_PLAY_SPELL_IMPACT` exists in the
    /// 1.12 opcode table and vmangos sends it from **exactly one place** — the
    /// `.debug sendspellimpact` GM command (`DebugCommands.cpp`) — so a client
    /// waiting to be told when a spell lands waits for ever. The 1.12 client
    /// times the impact itself: `SMSG_SPELL_GO`'s hit list says who was hit, and
    /// the missile's own arrival says when.
    pub const IMPACT_KIT: usize = 3;
    /// The kit a unit **wears for as long as an aura is on it** — Ice Armor's
    /// hands, a shield bubble, a weapon enchant's glow.
    ///
    /// The odd one of the five: every other kit is a moment, and this one is a
    /// condition. It is not answered by the cast at all — a buff cast before the
    /// client ever saw the unit still has to be drawn — so it is driven by
    /// `UNIT_FIELD_AURA` rather than by `SMSG_SPELL_GO`, which is why it is a
    /// separate lookup below rather than another field of [`super::CastEffects`].
    pub const STATE_KIT: usize = 4;
    pub const CHANNEL_KIT: usize = 5;

    /// The missile block, and it is what pins the tail of `SpellVisual`'s 16
    /// columns: Fireball reads `1, 365, 0, 1, 3011` across these five, which is
    /// a flag, an id well inside `SpellVisualEffectName`'s 782 rows, a small
    /// enum, an attachment id the character models carry, and a sound id
    /// **outside** that table — the same arithmetic that separated the kit's
    /// model columns from its own tail.
    pub const HAS_MISSILE: usize = 6;
    pub const MISSILE_MODEL: usize = 7;
    pub const MISSILE_PATH_TYPE: usize = 8;
    pub const MISSILE_DESTINATION: usize = 9;

    /// **Does this spell's persistent area draw anything at all** — the gate on
    /// the other two, and the client refuses before reading either of them.
    ///
    /// Named by its own failure: a dynamic object's setup logs
    /// `SPELLEFFECTNOAREAEFFECT|<spell>` and returns when it is clear.
    /// 217 of the 2,167 rows set it, and every one of those 217
    /// has a [`AREA_MODEL`] that resolves — which is the check that says the
    /// gate and the model are the same two columns rather than a coincidence of
    /// two dense ones.
    pub const AREA_FLAG: usize = 11;
    /// A `SpellVisualEffectName` id — **the model the `DynamicObject` is**,
    /// failing with `SPELLEFFECTIDNOTFOUND`.
    ///
    /// This is the column that replaces a fallback chain over the caster's own
    /// five kits; see the module comment for what that chain drew for
    /// Flamestrike.
    pub const AREA_MODEL: usize = 12;
    /// …and a `SpellVisualKit` id — the *procedural* half,
    /// which is a rain rather than a model. Only 10 of the 217 carry one whose
    /// [`CHAR_PROC`] says 9.
    pub const AREA_KIT: usize = 13;

    /// `SpellVisualKit.dbc` fields 15..18, `charProc[4]` — which client-side
    /// procedural a kit runs, with three parallel parameter columns after it.
    ///
    /// The four slots are searched in order for the wanted value and the *slot
    /// index* is what the parameters are read at, so a kit that
    /// carries two procedurals answers each from its own column.
    ///
    /// **Pinned by the layout either side of it.** The five model columns end at
    /// 7, the sound is 13 and the shake 14 — measured already — so the four
    /// procedurals are 15..18 and their parameters begin at 19. They sit at
    /// `+0x3c`, `+0x4c` and `+0x5c` of the record, which is
    /// exactly fields 15, 19 and 23 of a 35-field row whose id is field 0.
    pub const CHAR_PROC: usize = 15;
    /// How many procedural slots a kit has.
    pub const CHAR_PROC_SLOTS: usize = 4;
    /// `charParamZero[4]`, fields 19..22.
    pub const CHAR_PARAM_ZERO: usize = 19;
    /// `charParamOne[4]`, fields 23..26.
    pub const CHAR_PARAM_ONE: usize = 23;
    /// The falling-impact procedural. The other values in
    /// the shipped data (8 with 34 rows, 11 with 17, …) are other procedurals
    /// and are **not** guessed at.
    pub const CHAR_PROC_AREA_RAIN: u32 = 9;
    /// **The model-colour procedural** — the commonest of them all at 88 rows,
    /// and the one that paints a unit's own model. See [`super::ModelTint`],
    /// where the eight rows that name their own colours are.
    pub const CHAR_PROC_MODEL_COLOUR: u32 = 1;
    /// …and its sibling at 18 rows, which is **a colour with a fade** —
    /// taking `charParamOne × 1000` and `charParamTwo × 1000` as
    /// milliseconds where case 1 reads no timing at all.
    ///
    /// Read here as the same thing as case 1, which is a stated deviation and a
    /// small one: its population is the eight `Glowy (<colour>)` test auras,
    /// `Vertex Color: Light Blue` and a handful of impacts, and every one of
    /// the eight states `1e7` for both durations — a fade of nearly three
    /// hours, which is a fade nobody sees. Drawing them flat is what they look
    /// like; dropping them would leave a spell called `Vertex Color: Light
    /// Blue` with no colour on it.
    pub const CHAR_PROC_MODEL_GLOW: u32 = 13;

    /// **The chain procedural - a bolt of lightning strung between units.**
    ///
    /// Two values, `0` and `12`, and they reach the *same* case in the client's
    /// dispatch. So they are one procedural with
    /// two names in the data, and reading only one of them loses 14 kits - the
    /// half Chain Lightning, Chain Heal and every `Shock` is in.
    ///
    /// **Zero is also what an all-zero row reads as**, which is why the id it
    /// names has to resolve before anything is drawn: a kit whose four slots
    /// are `0` with all-zero parameters is not four chain effects, it is a row
    /// that means nothing. The unused-slot convention in this column is `-1`
    /// for 6,851 of the 7,112 slots; the all-zero rows are the residue.
    pub const CHAR_PROC_CHAIN: [u32; 2] = [0, 12];

    /// `SpellChainEffects.dbc`, whose seven columns after the id are read in
    /// order by the client - see [`super::ChainEffect`].
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

    /// `Spell.dbc` fields 82..84, `EffectImplicitTargetA` — one per effect.
    ///
    /// Read for exactly one question: **does this spell land on its own
    /// caster?** `SMSG_SPELL_GO`'s hit list is what says who was hit, and a
    /// cast that names nobody is ambiguous — a self-buff whose list came back
    /// empty and an area spell that caught nothing look identical on the wire.
    /// The spell's own row is what tells them apart, and getting it wrong is
    /// visible: without this, every Arcane Explosion that hit nothing burst on
    /// the mage's own chest.
    ///
    /// Measured rather than counted out of a layout, the way
    /// [`SPELL_VISUAL`] was: Frost Armor reads `1, 1, 0` here — vmangos'
    /// `TARGET_UNIT_CASTER` — where Arcane Explosion reads `22, 0, 0`
    /// (`TARGET_LOCATION_CASTER_SRC`, a place rather than a unit) and Fireball
    /// reads `6, 6, 0` (`TARGET_UNIT_ENEMY`). Three spells whose targeting is
    /// known independently, three different answers, and the self-buff is the
    /// only one that reads 1.
    pub const EFFECT_TARGET_A: std::ops::Range<usize> = 82..85;
    /// `TARGET_UNIT_CASTER` in vmangos' `SpellDefines.h`.
    pub const TARGET_SELF: u32 = 1;

    /// `Spell.dbc` field 37, `Speed`, in yards per second.
    ///
    /// **The only number in this chain that is not on `SpellVisual`**, and it
    /// has to be: how fast a projectile flies is a property of the spell, not
    /// of its art — the same `Fireball_Missile_Low.m2` is thrown at 24 y/s by
    /// Fireball and at other speeds by everything that borrows it. Measured the
    /// way every other index here was: `vale dbc Spell 133` reads `f32
    /// 24.000` at 37, and vmangos' `SpellEntry::speed` is commented `// 37`
    /// beside it.
    pub const SPELL_SPEED: usize = 37;

    /// `SpellVisualKit.dbc`'s **six** model columns, and the [`super::attach`]
    /// point each hangs from.
    ///
    /// The first five are contiguous and are followed by three more the same
    /// shape — breath (8) and the two weapon effects (9, 10) — which are
    /// deliberately not read: a breath effect belongs to a dragon rather than
    /// to a caster, and a weapon effect wants the *weapon's* model to hang off
    /// rather than the wearer's.
    ///
    /// **And then there is field 12, which is a model column too, and skipping
    /// it is why nothing rooted looked rooted.** The note that stood here
    /// argued the tail past 10 is not models, "and the check is arithmetic:
    /// field 13 is 1484 for Fireball, well outside `SpellVisualEffectName`'s
    /// 782 rows". Field 13 is indeed not a model — it is a `SoundEntries` id,
    /// which is why it runs to 9060 — but the check stepped straight over 12,
    /// and 12 resolves cleanly on every one of the 37 kits that set it:
    ///
    /// ```text
    /// kit   66  ->  EntanglingRoots_State
    /// kit  285  ->  Frost_Nova_state
    /// kit  744  ->  Net_State
    /// kit  746  ->  Web_State
    /// kit  356  ->  ThunderClap_Cast_Base
    /// ```
    ///
    /// The roots, the ice, the Booty Bay guard's net, the spider web, and
    /// Thunder Clap's ring. **29 of the 37 set no other model column at all**,
    /// so for those kits this was the whole of the visual and the unit wore
    /// nothing — reported as "root spell effects are not drawn".
    ///
    /// **`BASE` is the point, and it is read off the population rather than off
    /// a companion column**, because there is none: field 13 beside it is the
    /// sound and field 14 a small flag, and nothing in the row names an
    /// attachment. What names it is the art — `_State`, `_Base`, `_Ground` and
    /// `_area` throughout, effects that sit on the floor under a unit. It
    /// coexists with the real `BASE` column on 3 of the 37, which is two models
    /// at the feet and is allowed.
    pub const EFFECTS: [(usize, u32); 6] = [
        (3, super::attach::HEAD),
        (4, super::attach::CHEST),
        (5, super::attach::BASE),
        (6, super::attach::SPELL_HAND_LEFT),
        (7, super::attach::SPELL_HAND_RIGHT),
        (12, super::attach::BASE),
    ];

    /// `SpellVisualEffectName.dbc`: id, name, **model path**, another name, and
    /// a scale. Field 2 is the same column `vale particles` takes its
    /// spell-effect population from, so a wrong index there would have shown up
    /// as 782 models that will not parse.
    /// `SpellVisualEffectName` field 1 — the row's own name, which is how the
    /// client reaches the handful of visuals no chain names. See
    /// [`super::SpellVisuals::hardcoded`].
    pub const EFFECT_NAME: usize = 1;
    pub const EFFECT_MODEL: usize = 2;
    pub const EFFECT_SCALE: usize = 4;

    /// `SpellVisualKit.dbc` field 2, `animID` — an `AnimationData.dbc` id.
    ///
    /// **-1 is the table's own "none"** and appears in the columns either side
    /// of it too; 0 is `Stand`, which is not an answer for the same reason it
    /// is not one in `Emotes.dbc` — a cast that resolved to Stand would
    /// interrupt the caster in order to stand still.
    pub const ANIMATION: usize = 2;
}

impl SpellVisuals {
    /// Build the map, or `None` if any of the three tables is missing or will
    /// not parse.
    ///
    /// All three or none: a `SpellVisual` with no kits to resolve is not a
    /// partial answer, it is the *wrong* answer — every spell would come back
    /// empty and the fallback chain would look like it was working.
    /// `effect_name` is `SpellVisualEffectName.dbc` and `chain_effect` is
    /// `SpellChainEffects.dbc`; those two are the **optional** tables in the
    /// chain. Without the first a cast is still posed and simply has no models,
    /// which is what this client did until they were read; without the second
    /// the 192 spells that string a bolt between units draw nothing, which is
    /// what this client did until *this* round. Both are degradations rather
    /// than wrong answers. The other three are all-or-none for the reason
    /// above.
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

        // effect id -> the model it names and how big. A row whose path is
        // empty is a row Blizzard shipped without art; it costs its own effect
        // and nothing else, exactly as a missing `CharSections` texture costs
        // its own layer.
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
                // **The rows nothing names**, kept by their own name — see
                // [`SpellVisuals::hardcoded`]. Fourteen of the 782 are called
                // `HARDCODED …` and each is reached by the engine rather than
                // through `SpellVisual`, so nothing above would ever look them
                // up by id.
                if let Some(name) = names.string_at(record, fields::EFFECT_NAME) {
                    if name.starts_with(HARDCODED) {
                        hardcoded.insert(name, (model_path(&path), scale));
                    }
                }
                effect_model.insert(id, (model_path(&path), scale));
            }
        }

        // chain id -> the bolt's shape. Optional on its own terms - see the
        // doc above - and every id below has to land in here before anything is
        // drawn, which is the client's own guard too (it bounds the
        // index against the store and then tests the row for null).
        let mut chain_rows: HashMap<u32, ChainEffect> = HashMap::new();
        if let Ok(rows) = Dbc::parse(chain_effect) {
            for record in 0..rows.record_count {
                let Some(id) = rows.u32_at(record, 0) else {
                    continue;
                };
                let texture = rows
                    .string_at(record, fields::CHAIN_TEXTURE)
                    .unwrap_or_default();
                // A row with no texture has nothing to wrap the strip in, which
                // would draw as untextured white - louder than drawing nothing
                // and less true. Dropped, on `effect_model`'s own terms.
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

        // kit id -> the bolt it strings, for the 48 kits that string one.
        //
        // The same search-then-index shape `kit_rain` has below and for the
        // same reason, plus one guard of its own: `charProc == 0` is also what
        // an all-zero row reads, so the id has to land in the table before the
        // kit counts as carrying a chain. Twenty rows are dropped by that.
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
            // `charParamZero` is a float holding an integer id, exactly as the
            // rain's is a float holding an index - the same trick,
            // and the same reading of it here.
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

        // kit id -> the rain it runs, for the eight kits that run one.
        //
        // **The slot is searched and then indexed**, which is the client's own
        // shape: the parameters are three parallel arrays and a
        // procedural in slot 2 reads its rate from `charParamOne[2]`. Taking
        // slot 0's would give every one of them Blizzard's five a second.
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
            // **`charParamZero` is a float holding an integer index**, which is
            // the client's trick: it adds 512.0 and takes the bits
            // it lands in rather than converting. Read as a float here and
            // rounded, so a 2.0 is index 2 and a row outside the table is
            // dropped rather than clamped into the wrong spell's impact.
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

        // kit id -> the colour it paints its bearer's model, for the 106 rows
        // that state one. The same slot search the rain above does and for the
        // same reason: the parameters are four parallel arrays and a procedural
        // in slot 2 reads its own column.
        let mut kit_tint: HashMap<u32, ModelTint> = HashMap::new();
        for record in 0..kit.record_count {
            let Some(id) = kit.u32_at(record, 0) else {
                continue;
            };
            let Some(slot) = (0..fields::CHAR_PROC_SLOTS).find(|slot| {
                matches!(
                    kit.u32_at(record, fields::CHAR_PROC + slot),
                    Some(fields::CHAR_PROC_MODEL_COLOUR) | Some(fields::CHAR_PROC_MODEL_GLOW)
                )
            }) else {
                continue;
            };
            // **The float's value is the packed `0xRRGGBB`, truncated** — see
            // [`ModelTint`]. The client's own conversion truncates
            // (round-to-zero), so this **is not**
            // the `round` the rain's model index one column along takes: that
            // one is a small integer written exactly and this is a colour
            // written as whatever the artist's tool emitted. A value outside
            // the 24 bits is dropped rather than masked into some other colour.
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
                // Not a property of the visual at all — filled in per *spell*
                // below, off that row's own `Attributes`.
                ranged_shot: false,
            };
            if !animation.is_empty() {
                visual_anim.insert(id, animation);
            }

            // **The held models come from whichever kit supplied the held
            // pose**, and the two questions are asked separately: a spell can
            // state a `precastKit` with a pose and no models and a `channelKit`
            // with models and no pose, and taking both from one kit would drop
            // whichever half the other one had.
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
            // …and the colour it paints the bearer, which is a third column of
            // the same row and is asked on the same terms — a kit can state a
            // colour with no models at all, which is exactly what Stoneform's
            // does. **The state kit only**: 82 of the 106 rows are worn ones,
            // and what the other 24 do — a colour on a cast, a channel or an
            // impact — is a *moment* with a length nothing here states. They
            // are read into the table and no caller asks for them yet.
            if let Some(tint) = kit_of(fields::STATE_KIT).and_then(|k| kit_tint.get(&k)) {
                visual_state_tint.insert(id, *tint);
            }

            // **The area, gated on field 11 first.** The client refuses before
            // reading either of the other two, and obeying that matters rather
            // than being tidy: 33 rows carry a field-12 id with the flag clear,
            // and drawing them would put a model on the ground under every
            // `DynamicObject` whose spell states it should have none.
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
                            // The rain is the *kit*'s, and it is optional in its
                            // own right: 207 of the 217 areas are a model
                            // standing still and nothing else.
                            rain: kit_of(fields::AREA_KIT)
                                .and_then(|k| kit_rain.get(&k))
                                .cloned(),
                        },
                    );
                }
            }

            // **Which of the five kits carries the chain decides *when* it
            // is drawn, and that is the whole of the distinction.** A cast or
            // precast kit fires it once; a channel kit holds it. No shipped
            // visual sets both, and the three that set precast *and* cast set
            // the same kit in both - so the first found is the answer rather
            // than a choice between two.
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

            // **The flag is not enough on its own.** 1,187 visuals set
            // `hasMissile` and a share of them name a model that resolves to
            // nothing — a row Blizzard shipped without art, exactly as in
            // `SpellVisualEffectName` above. A missile with no model is not a
            // missile, so the entry is only made when the file is named; the
            // speed is filled in per *spell* below, since it lives on the spell.
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
            // **A ranged attack is the one release the visual chain does not
            // carry**, so it is folded in here rather than looked up beside the
            // animation — see [`CastAnimation::ranged_shot`]. Note the entry is
            // inserted for a spell with *no* visual at all, which is exactly the
            // case Auto Shot is: `SpellVisual = 0`, no kit, nothing to join.
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
            self_cast,
            spells: spell.record_count,
        })
    }

    /// **A visual the engine spawns, by the name the table gives it.**
    ///
    /// Fourteen of `SpellVisualEffectName`'s 782 rows are called
    /// `HARDCODED <something>` and none of them is reachable through
    /// `SpellVisual`: no spell names them, so the chain [`Self::effects`] walks
    /// can never produce one. The client reaches them by *name* at boot and
    /// then plays them off engine conditions — a level-up, a mount dismissing,
    /// an inebriation, a pet's loyalty moving, and the one this exists for:
    ///
    /// ```text
    /// id 14  "HARDCODED Loot Art"       Particles\LootFX.mdl
    /// id 21  "HARDCODED Unit Level Up"  Spells\LevelUp\LevelUp.mdl
    /// ```
    ///
    /// Both names are the client's own, which is what says the
    /// lookup is the client's and not a convention invented here — and the
    /// reason to resolve by name rather than by the id is the same: the id is
    /// this repo reading the shipped table, the name is what the client holds.
    ///
    /// Answers the model path — already through [`model_path`], so it is the
    /// `.m2` the archive holds rather than the `.mdl` the table names — and the
    /// row's own scale.
    pub fn hardcoded(&self, name: &str) -> Option<(&str, f32)> {
        let (path, scale) = self.hardcoded.get(name)?;
        Some((path.as_str(), *scale))
    }

    /// The models `spell_id`'s kits hang on the caster, or `None` for a spell
    /// whose visual names none.
    pub fn effects(&self, spell_id: u32) -> Option<&CastEffects> {
        self.effects.get(&spell_id)
    }

    /// Does this spell land on the unit that cast it?
    ///
    /// Asked of a release whose hit list named nobody, and only then. A
    /// self-buff's list usually carries the caster's own guid and sometimes
    /// carries nothing at all, and the two must not be different answers —
    /// without the fallback, exactly the buffs whose impact *is* the visible
    /// half (Frost Armor's head, Dampen Magic's ring) draw nothing. But the
    /// fallback cannot be unconditional either: an **area** spell that caught
    /// no one also names nobody, and giving it the caster bursts Arcane
    /// Explosion on the mage's own chest every time it misses.
    pub fn is_self_cast(&self, spell_id: u32) -> bool {
        self.self_cast.contains(&spell_id)
    }

    /// The models a unit wears while `spell_id`'s aura is on it, or `None` for
    /// the great majority of spells that leave no visible mark.
    ///
    /// Asked per *aura slot* rather than per cast — see [`fields::STATE_KIT`].
    pub fn state(&self, spell_id: u32) -> Option<&Vec<KitEffect>> {
        self.states.get(&spell_id)
    }

    /// **The models a `SpellVisualKit` hangs, asked by kit id** — the one way
    /// into this chain that does not start at a spell.
    ///
    /// `SMSG_PLAY_SPELL_VISUAL` and `SMSG_PLAY_SPELL_IMPACT` carry a kit and no
    /// spell, so nothing else here can answer them. See [`Self::kits`].
    pub fn kit(&self, kit_id: u32) -> Option<&Vec<KitEffect>> {
        self.kits.get(&kit_id)
    }

    /// …and the pose that kit holds, which for the two kits this exists for is
    /// the visible half: 406 and 438 both read `animID 61`, `EmoteEat`.
    pub fn kit_pose(&self, kit_id: u32) -> Option<u16> {
        self.kit_poses.get(&kit_id).copied()
    }

    /// How many kits resolved to at least one model, and how many to a pose —
    /// for `vale spell`, which reports what a table produced rather than
    /// what it contains.
    pub fn kit_counts(&self) -> (usize, usize) {
        (self.kits.len(), self.kit_poses.len())
    }

    /// The pose a unit holds for as long as `spell_id`'s aura is on it, or
    /// `None` for the overwhelming majority of auras, which change nothing about
    /// how their bearer stands.
    ///
    /// Asked per *aura slot*, exactly as [`Self::state`] is, and for the same
    /// reason: a stun cast before this client ever saw the unit still has to be
    /// drawn, so nothing about the cast can be the trigger. See
    /// [`SpellVisuals::state_anim`] for what pins the column.
    pub fn aura_pose(&self, spell_id: u32) -> Option<u16> {
        self.state_anim.get(&spell_id).copied()
    }

    /// **The colour a unit's own model is drawn through while `spell_id`'s aura
    /// is on it** — Stoneform's stone, a ghost's pallor, Shadowform's purple.
    ///
    /// Asked per *aura slot*, exactly as [`Self::state`] and [`Self::aura_pose`]
    /// are, and for the same reason: an aura applied before this client ever saw
    /// the unit still has to be drawn, so nothing about the cast can be the
    /// trigger. See [`ModelTint`] for the encoding and for the eight rows whose
    /// names are their own colours.
    pub fn aura_tint(&self, spell_id: u32) -> Option<ModelTint> {
        self.state_tint.get(&spell_id).copied()
    }

    /// `(spells whose aura paints their bearer, the distinct colours)` — what
    /// `vale spell` reports, and the pair that moves if the column does.
    ///
    /// The colours are listed rather than counted because **the list is the
    /// evidence**: a wrong column does not come back naming `#ff0000` for a row
    /// called `Glowy (Red)`.
    pub fn aura_tint_counts(&self) -> (usize, Vec<[u8; 3]>) {
        let colours: std::collections::BTreeSet<[u8; 3]> =
            self.state_tint.values().map(|t| t.colour).collect();
        (self.state_tint.len(), colours.into_iter().collect())
    }

    /// `(spells whose aura states a pose, the census of which poses)`, the
    /// second sorted commonest first — what `vale spell` prints, and the
    /// measurement the renderer's precedence for the pose is judged against.
    ///
    /// The census is the point rather than the total: a column read as a pose
    /// had better come back naming *poses a stopped unit holds*, and it does —
    /// see the check's own output.
    pub fn aura_pose_counts(&self) -> (usize, Vec<(u16, usize)>) {
        let mut per: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
        for pose in self.state_anim.values() {
            *per.entry(*pose).or_default() += 1;
        }
        let mut census: Vec<(u16, usize)> = per.into_iter().collect();
        census.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        (self.state_anim.len(), census)
    }

    /// **What a persistent area of `spell_id` is drawn as** — a Blizzard, a
    /// Flamestrike, a Rain of Fire, a Consecration.
    ///
    /// The server puts one `DynamicObject` in the world per such area, carrying
    /// nothing but the spell id and a radius, and nothing on the wire says what
    /// it looks like. Three columns of the spell's own `SpellVisual` row do —
    /// see the module comment, which names the addresses and the population.
    ///
    /// **This used to be a fallback chain over the caster's five kits and it is
    /// a measurement now.** The chain read attachment 19 out of the state kit,
    /// then the impact, then the release, then the hold; it agreed with the file
    /// for Blizzard by luck (both name `Blizzard_Impact_Base`) and disagreed for
    /// Flamestrike, where it drew a two-emitter `Immolate_State_Base` in place of
    /// the 454-vertex `Flamestrike_Impact_Base` field 12 names. Nothing reported
    /// that, because a smaller fire is still a fire.
    pub fn area(&self, spell_id: u32) -> Option<&AreaEffect> {
        self.areas.get(&spell_id)
    }

    /// **The bolt this spell strings between units**, or `None` for the 22,168
    /// that string none - see [`ChainVisual`].
    ///
    /// This is the one visual in the whole chain with **no model behind it**:
    /// where every other answer here is a path into the archives,
    /// `SpellChainEffects` states a texture and six numbers and the client
    /// builds the geometry. A renderer that walks the model columns and stops
    /// draws nothing at all for Chain Lightning, Chain Heal, Drain Life, Mind
    /// Flay and the Rallying Cry buffs - which is exactly what this one did.
    pub fn chain(&self, spell_id: u32) -> Option<&ChainVisual> {
        self.chains.get(&spell_id)
    }

    /// `(spells with a chain, of which held, distinct shapes, the textures)` -
    /// the population behind [`Self::chain`].
    ///
    /// The middle number is what says the `charParamTwo` reading is right
    /// rather than merely plausible: it has to be the *channels*, and a reading
    /// with the flag inverted reports the complement.
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

    /// `(spells with an area, of which rain, distinct models, distinct rains)` —
    /// the population behind [`Self::area`], so that a column read one along is
    /// a number rather than a picture somebody eventually notices.
    ///
    /// The last is a census rather than a count for the same reason
    /// [`Self::aura_pose_counts`]' is: `charParamZero` is an index into a table
    /// that is **not in any file**, so what says it was read correctly is that
    /// the models it picks out are the ones whose names match the spells asking
    /// for them.
    /// Every distinct model a persistent area names — both the object's own and
    /// the rain's, since a caller checking them against the archive has to ask
    /// about both and eight of the ten rains name a different file from the area
    /// they fall in.
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
    /// no speed)` — what `vale spell` reports. The third is the one that
    /// matters: a missile with no speed cannot be flown, and a *large* count
    /// there would say the speed column had moved rather than that a handful of
    /// rows are instant.
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
    /// **A pose, specifically**, not an entry: [`Self::by_spell`] also carries
    /// the ranged attacks, which state no kit at all and whose release is the
    /// wielder's weapon ([`CastAnimation::ranged_shot`]). Counting entries
    /// would move this baseline by the size of an unrelated population and hide
    /// a column regression behind it.
    pub fn counts(&self) -> (usize, usize) {
        let with_a_pose = self
            .by_spell
            .values()
            .filter(|a| a.hold.is_some() || a.release.is_some())
            .count();
        (self.spells, with_a_pose)
    }

    /// How many spells fire the wielder's own ranged weapon rather than a pose
    /// out of the visual chain — the count beside the one above, and the one
    /// that says the `Attributes` bit was found.
    pub fn ranged_shots(&self) -> usize {
        self.by_spell.values().filter(|a| a.ranged_shot).count()
    }

    /// How many spells land on each animation id, commonest first.
    ///
    /// The survey that makes the column checkable without a server: the
    /// animations a game full of spells resolves to had better be the casting
    /// ones, and a wrong column produces a spread over ids that mean nothing.
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

    /// **The visuals the engine spawns are reached by name**, because no
    /// `SpellVisual` names them — so a lookup by id would be this repo's
    /// reading of the shipped table where the name is the client's own string.
    /// Both `HARDCODED Loot Art` and `HARDCODED Unit Level Up` are the
    /// client's; see [`SpellVisuals::hardcoded`].
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
        // **`.mdl` in the table, `.m2` in the archive** — the same swap every
        // other effect path in this module goes through.
        assert_eq!(path, "Particles\\LootFX.m2");
        assert_eq!(scale, 1.0);
        assert_eq!(visuals.hardcoded("HARDCODED Nothing"), None);
    }

    /// **The colour is the float's own value, packed `0xRRGGBB`** — the one
    /// thing about [`ModelTint`] that a wrong reading gets plausibly wrong
    /// rather than emptily wrong, since any float decodes to *some* colour.
    ///
    /// Stoneform's shipped row is the case: `charParamZero` holds
    /// `4474718.0`, which is `0x44465e`, which is the dark blue-grey a dwarf
    /// turns. Reading the bits instead would give `0x4a888cbc`'s top bytes and
    /// paint him mid-green.
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

    /// **The parameters are read at the procedural's own slot**, which is the
    /// same trap [`AreaRain`]'s rate has: the four `charProc`s and their four
    /// parameter arrays are parallel, so a colour declared in slot 2 whose
    /// parameters are read from slot 0 comes back as some other row's.
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

    /// **A channelled spell states only its channel kit**, and its pose is the
    /// held one. Reading `precastKit` and stopping leaves every channel in the
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

    /// **A spell with both a precast and a channel kit keeps them apart.**
    ///
    /// Blizzard's shape: `precastKit` at `ReadySpellOmni` for the wind-up,
    /// `channelKit` at `ChannelCastOmni` for the channel. The chain used to
    /// read the channel only when there was no precast, so `hold` answered 52
    /// and nothing ever answered 125 — 61 of the game's channelled spells
    /// standing in their wind-up for the whole channel. See
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

    /// **A ranged attack with no visual at all still resolves to something**,
    /// and that is the whole point of the field: the chain is joined on
    /// `SpellVisual`, Auto Shot's is **0**, so before this the commonest attack
    /// a hunter makes was a `cast()` of `None` — indistinguishable from a
    /// passive.
    ///
    /// The second row is the half that must not regress: a *ranged ability*
    /// that does name a kit keeps the kit's own release, because the file is
    /// more specific than the weapon.
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
        // **The baseline count is poses, not entries.** Two of the three rows
        // reach a kit; the third is in `by_spell` and must not inflate it.
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

    /// **The models, and the two ways they are unlike the animation.**
    ///
    /// A kit hangs one model per body part and each goes to its own attachment
    /// point, so the five columns are five *different* answers rather than a
    /// fallback chain; and the effect table is optional, so a chain without it
    /// still poses the caster. Both are asserted here because both are the kind
    /// of thing that fails by drawing nothing.
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
        // …and a sound id where the models stop, which is the arithmetic that
        // pins the block: 1484 is outside the effect table entirely.
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

    /// Blizzard's own three columns, end to end — the model, the rain, and the
    /// gate that has to pass before either is read.
    ///
    /// Every number here is the shipped row: `SpellVisual` 259 reads
    /// `1, 398, 609` at fields 11..13 and `SpellVisualKit` 609 reads `charProc
    /// 9, charParamZero 0.0, charParamOne 5.0`. A column read one along either
    /// way gives `None`, which is what the old fallback chain silently covered
    /// for.
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
            "charParamZero is a float index into the client's own table"
        );
        assert_eq!(visuals.area(11), None, "a spell that is not in the table");
    }

    /// **The gate is obeyed before either of the other two is read**, which is
    /// the client's own order and not a tidiness choice: 33 shipped rows carry a
    /// field-12 id with field 11 clear, and reading them would put a model on
    /// the ground under every `DynamicObject` whose spell says it has none.
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

    /// **The rain reads its parameters at the slot the procedural was found
    /// in**, which is the client's own shape: the three parameter columns are
    /// parallel arrays, and a kit whose `charProc` sits in slot 2 takes its rate
    /// from `charParamOne[2]`. Taking slot 0's would give every one of them
    /// Blizzard's five a second.
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

    /// A `charParamZero` outside the seven-entry table draws **nothing** rather
    /// than the nearest entry.
    ///
    /// The table is the client's and its length is not this project's to choose,
    /// so an index past the end is a row this client does not understand — and
    /// clamping it would rain Star Shards on a spell that asked for something
    /// else, which is the plausible-wrong failure this repo keeps recording.
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

    /// **The two kits that are not about the caster**, and they fail in
    /// opposite directions if their columns move.
    ///
    /// `impactKit` hangs on the *victim* and `stateKit` on whoever is carrying
    /// the aura, so both are read out of columns 3 and 4 — between the cast kit
    /// and the channel kit, which are already pinned. A transposition of the two
    /// would not fail: an impact drawn as a state is a burst that never comes
    /// off, and a state drawn as an impact is a buff glow that flashes for a
    /// second and vanishes. Neither reads as an error, which is why this test
    /// asserts that the *right* models come out of the *right* column rather
    /// than that something came out.
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

        // **The state is not part of the cast**, and asking for one must not
        // answer with the other: a spell whose *only* visual is a worn aura has
        // no cast effects at all, and one with a cast and no aura has no state.
        assert_eq!(visuals.state(999), None);
        assert!(visuals.effects(999).is_none());
    }

    /// **The missile is described one table earlier than the kits, and its
    /// speed one table later.** `SpellVisual` says *that* there is one and
    /// which model; only `Spell.dbc` says how fast it flies. A reader that
    /// takes the whole description from one row gets a projectile that either
    /// never leaves the caster's hand or arrives before it is drawn.
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
        // sound id past the block — which is what says the block ends where it
        // does rather than running on.
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

        // **The flag alone is not a missile.** A visual that sets it and names
        // no model Blizzard shipped art for has nothing to throw, and an entry
        // built from the flag would leave the renderer looking up an empty
        // path once per cast.
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
