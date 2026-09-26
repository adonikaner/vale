//! A minimal object manager: the client's view of the world, keyed by GUID.
//!
//! The server never sends "the world state" — it sends *deltas*
//! (`SMSG_UPDATE_OBJECT`) that create objects, change fields, and drop objects
//! out of range. Holding that state is the client's job, and this is where it
//! lives.

use crate::play::wdb::{Caches, Kind, Learned};
use crate::state::fields;
use crate::state::movement::{
    bearing, distance, fall_elevation, move_flags, wrap_angle, Footing, MonsterMove, MovementInfo,
    Speeds, Spline, SplineFacing, FALL_THRESHOLD,
};
use crate::state::query::{CreatureInfo, GameObjectInfo, ItemInfo, PlayerInfo};
use crate::state::update::{ObjectType, ObjectUpdate, Position, UpdateBlock};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// One extrapolated stride, ended wherever the world says it ends.
///
/// Free rather than a method because [`ObjectManager::advance`] holds the entity
/// map mutably while it runs and the world is borrowed beside it.
fn clipped(world: Option<&dyn Footing>, from: Position, dx: f32, dy: f32) -> [f32; 2] {
    let to = [from.x + dx, from.y + dy, from.z];
    let Some(world) = world else {
        return [to[0], to[1]];
    };
    let end = world.step([from.x, from.y, from.z], to);
    [end[0], end[1]]
}

/// `EQUIPMENT_SLOT_END` (`Player.h:501`) — head through tabard.
pub const EQUIPMENT_SLOTS: usize = 19;

/// `UNIT_FLAG_STUNNED`, bit 18 of `UNIT_FIELD_FLAGS`.
///
/// Public because two crates ask the same question of the same bit and only one
/// of them has an [`Entity`] to ask it of: the renderer's snapshot carries the
/// flag word whole. See [`Entity::is_stunned`] for what the client does with it.
pub const UNIT_FLAG_STUNNED: u32 = 0x0004_0000;

/// `UNIT_FLAG_TAXI_FLIGHT`, bit 20 of the same field — see
/// [`Entity::is_on_taxi`], which is where the reading and the reason are.
///
/// Public on the same terms as the one above, and read by the same two crates
/// for the same reason: `game::taxi::on_taxi` answers `UnitOnTaxi(unit)` off a
/// snapshot that carries the flag word rather than an [`Entity`].
pub const UNIT_FLAG_TAXI_FLIGHT: u32 = 0x0010_0000;

/// `UNIT_VIS_FLAGS_GHOST`, bit 0 of `UNIT_FIELD_BYTES_1`'s **fourth** byte —
/// the spirit walking back from the graveyard (`SPELL_AURA_GHOST`).
///
/// Public on the same terms as the two flags above: the renderer's snapshot
/// carries the byte whole and asks the question of that rather than of an
/// [`Entity`]. See [`Entity::vis_flags`].
pub const UNIT_VIS_FLAGS_GHOST: u8 = 0x01;

/// `UNIT_VIS_FLAGS_CREEP`, bit 1 of the same byte — **stealth, in every form
/// the game has**.
///
/// vmangos sets it from `Aura::HandleModStealth` and from nothing else, so a
/// rogue's Stealth, a druid's Prowl, a night elf's Shadowmeld and every hidden
/// creature in the world arrive here as this one bit. That is why the two rules
/// keyed off it are keyed off *it* and not off a spell id.
pub const UNIT_VIS_FLAGS_CREEP: u8 = 0x02;

/// `UNIT_VIS_FLAGS_UNTRACKABLE`, bit 2 — `SPELL_AURA_UNTRACKABLE`, which keeps
/// a unit off the minimap's tracking blips.
///
/// Nothing reads it yet; it is here so the byte's three meanings are written
/// down together rather than two of three.
pub const UNIT_VIS_FLAGS_UNTRACKABLE: u8 = 0x04;

/// `MAX_VISIBLE_ITEM_OFFSET` (`ItemDefines.h:157`): how far apart two slots'
/// blocks are in the player's field block. **12 from 1.6 onwards and 11 before**
/// — the enchantment array grew — so this is one of the constants that silently
/// reads a neighbour's data if it is copied from the wrong era.
pub const VISIBLE_ITEM_STRIDE: u16 = 12;

/// One thing in a unit's hand, as the **server** describes it.
///
/// **Two packets carry the same five numbers and this is where they stop being
/// two.** A creature's arrive in its update fields, already resolved:
/// `Creature::SetVirtualItem` writes the display id into
/// `UNIT_VIRTUAL_ITEM_SLOT_DISPLAY` and packs the rest into
/// `UNIT_VIRTUAL_ITEM_INFO` beside it. A player's arrive as an item **entry** in
/// `PLAYER_VISIBLE_ITEM_n_0`, and the same five come back from
/// `SMSG_ITEM_QUERY_SINGLE_RESPONSE` a round trip later —
/// [`ObjectManager::weapons_of`] is the join.
///
/// Nothing here is interpreted. What the numbers *mean* — which attachment
/// point a sheathed weapon hangs from, which swing its wielder plays — is a
/// game rule and lives in `vale_assets::tables::item`, which is why this is five
/// plain numbers and not an enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeldItem {
    /// `ItemDisplayInfo` row. **Zero is an empty hand**, and is the only test
    /// for one.
    pub display_id: u32,
    /// `ItemPrototype::Class` — 2 weapon, 4 armour.
    pub class: u8,
    pub subclass: u8,
    /// `ItemPrototype::Material` — what the weapon is made of, which is read
    /// for exactly one thing and it is not a picture: `WeaponImpactSounds`
    /// carries a metal and a non-metal row per subclass. See
    /// `vale_assets::tables::item::Weapon::material`.
    pub material: u8,
    pub inventory_type: u8,
    /// `ItemPrototype::Sheath` — where it hangs when it is put away.
    pub sheath: u8,
}

/// One entity the client currently knows about.
#[derive(Debug, Clone)]
pub struct Entity {
    pub guid: u64,
    pub object_type: Option<ObjectType>,
    pub position: Option<Position>,
    /// Raw update fields by index. Meaning depends on `object_type`; see
    /// vmangos `UpdateFields.h`.
    pub fields: HashMap<u16, u32>,
    /// **A create block has been applied, so a field that is missing is zero
    /// rather than unknown.**
    ///
    /// `Object::_SetCreateBits` sets a bit only `if (m_uint32Values[index] != 0)`,
    /// so a create block is a complete statement of the object's values in which
    /// every zero is expressed by *silence*. Nothing else on the wire works that
    /// way — a values update carries the fields that *changed*, and there an
    /// absence means "unchanged" — which is why this is a flag on the entity
    /// rather than a rule in [`Entity::field`].
    ///
    /// It costs a bool and it is the difference between a corpse and a live
    /// creature. `UNIT_FIELD_HEALTH` of a unit that was already dead when it
    /// streamed into view is 0, so it is never sent at all, so
    /// [`Entity::is_dead`] answered `None` and the renderer's
    /// `unwrap_or(false)` drew it standing in its idle. See [`Entity::health`],
    /// which is the one accessor that reads this, and the note there about why
    /// it is not applied to all of them.
    pub created: bool,
    /// Set on the object this session controls.
    pub is_self: bool,
    /// Walk/run/run-back/swim/swim-back/turn-rate, when the server sent a
    /// living movement block. The player's own run speed is what the movement
    /// anticheat measures us against, so it is not merely informational.
    pub speeds: Option<Speeds>,
    /// Server-driven movement currently being animated.
    pub spline: Option<Spline>,
    /// **How far into its own cycle a moving platform is**, in milliseconds —
    /// or `None` for everything that is not one, which is every object in the
    /// world but a few dozen elevators and the tram.
    ///
    /// Seeded from `UPDATEFLAG_TRANSPORT`'s trailing word (see
    /// [`crate::state::update::MovementUpdate::path_progress`]) and advanced by
    /// [`ObjectManager::advance`] on the world's own clock, exactly as the
    /// client does — it stores `progress - now` and adds `now` back per frame,
    /// which is the same quantity kept the other way up.
    ///
    /// **It is a phase, not a timestamp, and it is not taken modulo anything
    /// here.** The cycle length is a property of the *table*
    /// (`vale_assets::tables::transport`), which this crate does not read,
    /// so wrapping is the reader's — and `Transports::offset_at` does it, so
    /// a reader cannot get it wrong.
    ///
    /// Nothing else on the wire ever mentions an elevator: no
    /// `SMSG_MONSTER_MOVE`, no position update, no state change. This word and
    /// the shipped table are the whole of it, which is why a client that
    /// ignored it drew every platform at its spawn point for ever.
    pub transport_phase_ms: Option<u64>,
    /// The last movement block the server broadcast for this entity, from a
    /// `MSG_MOVE_*` packet. Kept rather than merely applied because its flags
    /// are what the *next* half-second of dead reckoning runs on — another
    /// player sends a start packet and then only heartbeats, so between them the
    /// client has to simulate them exactly as it simulates itself.
    pub movement: Option<MovementInfo>,
    /// **What the twelve `SMSG_SPLINE_MOVE_*` packets have said about this
    /// unit's flags**, which for a server-controlled unit is the only statement
    /// of them that is ever made after the create block.
    ///
    /// Held beside [`Self::movement`] rather than folded into it because it has
    /// to survive the two things that replace that field wholesale: a spline
    /// (which shadows the flags entirely — see [`Self::move_flags`]) and a
    /// fresh create block. A root that a re-create silently un-rooted would be
    /// worse than no root at all, since nothing else would ever put it back.
    ///
    /// Empty for almost every entity in the world; see
    /// [`crate::state::movement::FlagOverride`].
    pub forced_flags: crate::state::movement::FlagOverride,
    /// Where a `Final_Target` spline told this unit to look, resolved to a
    /// position when the packet arrived. Kept because [`ObjectManager::advance`]
    /// only sees one entity at a time and cannot look the target up itself.
    pub facing_target: Option<[f32; 3]>,
    /// How far into an arc this unit is, in seconds, when
    /// [`Self::movement`] carries `MOVEFLAG_JUMPING`.
    ///
    /// **Taken from the packet's own `fallTime` rather than accumulated from
    /// scratch**, which is what makes the parabola self-correcting: every
    /// heartbeat during a jump restates both the position and the phase, so the
    /// client re-anchors on each one instead of drifting away from an arc it
    /// started guessing half a second ago.
    ///
    /// Without any of this a jumping player was dead-reckoned horizontally with
    /// their z frozen at take-off, and their real height arrived only with each
    /// heartbeat — which is the "other players teleport up and back down" report.
    pub fall_secs: f32,
    /// How many times the *server* has stated this entity's position.
    ///
    /// The live session dead-reckons the player between packets and writes the
    /// result back here, so comparing positions cannot tell a server correction
    /// (teleport, knockback, anticheat resync) from the client's own guess.
    /// A counter can.
    pub position_updates: u32,
    /// The [`Self::position_updates`] this entity's **standing** height was last
    /// put on the ground for — see [`ObjectManager::stand_on_the_ground`].
    ///
    /// A latch rather than a flag, so the correction is taken once per statement
    /// the server makes rather than once per simulation step: a city's idle
    /// population is hundreds of entities and the terrain query is the one thing
    /// in `advance` that is not free. `None` is "never looked", which is also
    /// what a fresh create block leaves behind.
    pub grounded_for: Option<u32>,
    /// How many melee swings this unit has *thrown*, and how many have *landed
    /// on* it.
    ///
    /// Counters rather than timestamps, for the same reason `position_updates`
    /// is one: the object manager has no clock, and a renderer that polls at its
    /// own rate needs to know "has anything happened since I last looked?"
    /// rather than "when". A swing is an event with no duration — the server
    /// says a blow landed and says nothing else about it, ever — so the only
    /// thing to carry across is that it happened.
    pub swings_thrown: u32,
    /// **Who this unit is auto-attacking**, per `SMSG_ATTACKSTART` /
    /// `SMSG_ATTACKSTOP` about it, or `None` for one that is not swinging at
    /// anybody.
    ///
    /// **This, and not `UNIT_FLAG_IN_COMBAT`, is what puts a unit in its
    /// combat-ready stance.** The client gates the weapon-class Ready idle on
    /// the auto-attack target guid being set — engagement, not the combat flag
    /// and not the sheath state. The
    /// difference is the whole of what the flag gets wrong: a caster being
    /// beaten on has `IN_COMBAT` set the entire fight and never raises a weapon,
    /// and a player who has aggro from across a room is in combat before they
    /// have reached anything to swing at.
    ///
    /// Written for **every** unit, unlike [`ObjectManager::attacking`], which is
    /// the same two packets filtered to ourselves and is what an Attack button
    /// lights up from.
    pub attacking: Option<u64>,
    /// **This client has been handed control of this body** —
    /// `SMSG_CLIENT_CONTROL_UPDATE`'s byte, latched per unit.
    ///
    /// The client's own shape: it reads the packet's packed guid and its byte,
    /// looks the unit up, and sets a flag on that unit. It is a flag **on the
    /// unit**, not a session-wide "who is the mover" — the mover is derived from
    /// it and from `PLAYER_FARSIGHT` every frame, which is what makes the state
    /// machine self-correcting and is why it lives here.
    ///
    /// Written by a handler rather than by an update block, like
    /// [`Self::attacking`], and it dies with the object exactly as the
    /// reference's does.
    pub client_controlled: bool,
    /// `HitInfo` of the swing that last moved [`Self::swings_thrown`].
    ///
    /// The same counter-plus-payload shape [`Self::last_victim_state`] has, on
    /// the attacker's side of the same packet, and for the same reason: the
    /// counter says a swing is due to be drawn and this says *which* swing —
    /// `HITINFO_LEFTSWING` makes it the off-hand's and `HITINFO_CRITICALHIT`
    /// makes it the critical one. Both are bits the server sets on every blow
    /// and this client read over until now.
    pub last_swing_info: u32,
    /// …and **what the victim did about it**, recorded on the *attacker*.
    ///
    /// The same word as [`Self::last_victim_state`] and a different subject: a
    /// dodge, a parry and a block are the victim's reaction and the attacker's
    /// *sound*, since what a blow sounds like is decided by what it met. A
    /// swing that was parried and one that landed on plate are two different
    /// columns of the swinger's own weapon row, and neither is knowable from
    /// this end without carrying it here.
    pub last_swing_state: u32,
    /// …and who it was aimed at, so the impact can be voiced where the blow
    /// landed rather than where the swing started.
    pub last_swing_victim: u64,
    /// …and **how much it was for**, which is the one thing about a swing that is
    /// not an animation: it is the number that floats over the victim's head.
    ///
    /// Zero for a miss, a dodge, a parry or a full absorb, which is exactly what
    /// the server sends — the *word* those draw instead comes off
    /// [`Self::last_swing_state`] and [`Self::last_swing_info`], not off this.
    pub last_swing_damage: u32,
    /// **What this unit has been hit for, by the local player**, as the same kind
    /// of monotonic counter every other piece of combat news here is.
    ///
    /// Bumped by a melee swing *and* by a spell — one channel rather than two,
    /// because the thing that reads it wants "a number happened to this unit" and
    /// does not care which packet said so. That is the client's own shape: its
    /// producer is reached from both, and refuses outright unless
    /// the **source** is the player or their pet, which is why the source test is
    /// made here rather than by the reader.
    ///
    /// See `client::ui::worldtext`, which differences it.
    pub damage_taken: u32,
    /// How much the blow that last moved [`Self::damage_taken`] was for. Zero for
    /// a miss, a dodge, a parry or a full absorb, which is what the server sends —
    /// the *word* those draw instead comes off the two fields below.
    pub last_damage: u32,
    /// `HitInfo` for a swing and `SpellHitType` for a spell — **two different
    /// tables**, which is why [`Self::last_damage_spell`] says which one this is.
    /// A swing's crit is `0x80` and a spell's is `0x02`; reading one as the other
    /// finds a critical in every eighth ordinary hit.
    pub last_damage_info: u32,
    /// `VictimState` for a swing, and zero for a spell — a spell has no such
    /// field, and its refusals are `SpellHitType` bits instead.
    pub last_damage_state: u32,
    /// The spell that did it, or `None` for a weapon swing. **The switch between
    /// the two flag tables above**, and the reason the pair is one channel rather
    /// than two: a reader needs to know which table it is holding.
    pub last_damage_spell: Option<u32>,
    /// …and whether the last one was a **heal** rather than a hit, which is the
    /// one thing neither the amount nor the flags can say.
    pub healed: bool,
    pub blows_taken: u32,
    /// `VictimState` of the blow that last moved [`Self::blows_taken`].
    ///
    /// **The counter says a reaction is due and this says which one** — a
    /// flinch, a sidestep, a parry or the shield coming up. Kept beside the
    /// counter rather than as a counter of its own because they are one event:
    /// a poller that read them separately could see the state of the *next*
    /// blow against the count of this one.
    pub last_victim_state: u32,
    /// …and the same blow's `HitInfo`, which is what says whether it was a
    /// **critical**.
    ///
    /// A second payload beside [`Self::last_victim_state`] rather than folded
    /// into it, because the two words are two different fields of the packet
    /// and the client reads them for two different questions: the victim state
    /// picks between a flinch, a dodge, a parry and a block, and this picks
    /// between the flinch and the *big* flinch. `AnimationData.dbc` id 10,
    /// `CombatCritical`, is the second one — see `pose::reaction`, where the
    /// client's own chooser is written up.
    pub last_blow_info: u32,
    /// How many `SMSG_AI_REACTION`s this unit has sent, and the last one's
    /// reason.
    ///
    /// The same counter-plus-payload shape as the swing, because it is the same
    /// kind of thing: an *event* about a unit that the update stream never
    /// restates. `AI_REACTION_HOSTILE` arrives on every attack, so the counter
    /// moves repeatedly through a fight and the client's own priority channel is
    /// what stops that being a stream of barks — see
    /// `vale_client::sound::combat`.
    pub reactions: u32,
    /// The [`crate::play::action::ai_reaction`] value that last moved
    /// [`Self::reactions`].
    pub last_reaction: u32,
    /// How many one-shot emotes this unit has played, and the `Emotes.dbc` id
    /// of the last.
    ///
    /// The same counter-plus-payload shape, for the same reason. The id is a
    /// **row**, not an animation — the hop through `Emotes.dbc` needs the
    /// archives and this crate has none.
    pub emotes: u32,
    pub last_emote: u32,
    /// **How many `SpellVisualKit`s the server has told this client to play on
    /// this unit, and the last of them** — `SMSG_PLAY_SPELL_VISUAL`.
    ///
    /// The same counter-plus-payload shape as the emote, and for the same
    /// reason: it is an instant with no duration and no state, so the only
    /// thing to carry across is that it happened. The id is a
    /// **`SpellVisualKit` row** rather than a spell, which is why the tables
    /// this crate cannot read need a kit-keyed entry point — see
    /// [`crate::play::sound`].
    ///
    /// **This is what eating looks like.** vmangos sends kit 406 (food) and 438
    /// (drink) on every regeneration tick while a character sits with either,
    /// and nothing else on the wire says a character is eating at all.
    pub spell_visuals: u32,
    pub last_spell_visual: u32,
    /// …and the same pair for `SMSG_PLAY_SPELL_IMPACT`, which is the same body
    /// and the same kind of id about the unit something **happened to** rather
    /// than the one that did it.
    ///
    /// Two counters rather than one with a flag, because the two land in the
    /// two effect slots a unit already has — a caster's kit and a victim's —
    /// and folding them would let a pushed impact take down a pushed cast
    /// visual that is still running.
    pub spell_impacts: u32,
    pub last_spell_impact: u32,
    /// How many casts this unit has **begun** and how many it has **released**,
    /// with the cast bar's length in milliseconds beside the first.
    ///
    /// Two counters because they are two packets and the renderer does two
    /// different things with them: `SMSG_SPELL_START` holds the wind-up pose
    /// for `cast_time_ms`, `SMSG_SPELL_GO` fires the release. An instant spell
    /// sends only the second, so a client that waited for a pair would never
    /// animate one.
    pub casts_begun: u32,
    pub casts_released: u32,
    /// **The last four spells this unit released, oldest last written.**
    ///
    /// `last_spell` is one field and a release is one packet, which is fine
    /// until two of them arrive between two polls — and one family of spells
    /// guarantees exactly that. **Charge is two casts**: `SMSG_SPELL_GO` for
    /// Charge (100), then a second for the Charge Stun (7922) it triggers on the
    /// victim, both inside one 25 ms tick. Measured against a live server: the
    /// renderer saw `spell = 7922`, which states no caster models at all, so the
    /// red trail and the dust cloud Charge's own kit names were never asked for.
    /// Everything that triggers a second spell has the same shape.
    ///
    /// A fixed array rather than a `Vec` because this is copied into every
    /// entity snapshot every poll and an allocation per unit per frame for an
    /// event that happens seconds apart is not a trade worth making. Four is
    /// chosen the way [`MAX_AURA_SLOTS`] is not — it is a *renderer's* depth
    /// rather than the wire's, and a burst deeper than four loses its oldest,
    /// which is one frame of one glow.
    pub recent_spells: [u32; RECENT_SPELLS],
    /// **How many of those releases were a channel starting.**
    ///
    /// `MSG_CHANNEL_START` is the only statement a channelled spell makes about
    /// its length, and vmangos sends it *after* `SendSpellGo` — so a channel
    /// arrives as a release and a begin in that order, inside one poll. Without
    /// something to tell them apart the release wins by being tested last and
    /// **cancels the wind-up the channel had just armed**: an Evocation played
    /// its release (which it has not got) and stood in its idle loop for eight
    /// seconds. This counter is what lets the later event win.
    pub casts_channelled: u32,
    /// **How many releases the *server* has stated** — the same event as
    /// [`Self::casts_released`], counted from the wire alone.
    ///
    /// The two are equal for every unit but the local player, and for the local
    /// player they are deliberately not, because they answer two different
    /// questions:
    ///
    /// * `casts_released` is **what to draw on the caster**, and it moves at the
    ///   press so the arm does not wait a round trip
    ///   ([`ObjectManager::predict_own_cast`]).
    /// * this one is **what the cast hit**, and only `SMSG_SPELL_GO` can answer
    ///   that: [`Self::last_spell_target`] is the first entry of the packet's own
    ///   hit list, and it does not exist until the packet arrives.
    ///
    /// Folding the two together is what took the impact art off every
    /// implicitly-aimed spell in the game. A cone or an area spell sends **no
    /// target block at all** — `CastTarget::SelfImplicit`, the aiming word
    /// `Spell.dbc` gives 14,002 of its 22,360 rows — so the prediction writes
    /// `last_spell_target = 0`, the release counter moves at the press, and the
    /// consumer decides "nobody was hit" a frame later. The echo then arrives
    /// carrying the real victim, writes it, and moves nothing: swallowing the
    /// counter is precisely the guarantee that nothing downstream looks again.
    /// Cone of Cold lost its whole visible half that way — its release is one
    /// glow on the hand and everything else it draws is the impact on the unit
    /// it hit.
    pub casts_landed: u32,
    /// **How many casts this unit has had taken off it** — refused, interrupted
    /// or cancelled — which is the one end of a cast that no other counter here
    /// can express.
    ///
    /// A third counter rather than a rollback of the two above, and the reason
    /// is the shape of every consumer: they act on a counter *moving*, so
    /// subtracting from `casts_begun` would read as another cast beginning.
    /// What a cancel means is "the thing you are drawing is over now", which is
    /// news in its own right and is exactly what a counter says.
    ///
    /// **It exists because this client predicts and the reference does not.**
    /// Our own cast is drawn at the press ([`Self::casts_begun`]), so the arm is
    /// already moving and the release art is already up by the time the server
    /// answers — and if that answer is a refusal, nothing else would ever take
    /// them down: the wind-up would run out its `HOLD_GRACE_SECS` and the
    /// release art its `RELEASE_SECS`, which is the "a spell that cannot be cast
    /// plays an animation anyway" report exactly.
    ///
    /// Three things move it, and only the first is about the prediction:
    /// `SMSG_CAST_RESULT` carrying a failure (ours), `SMSG_SPELL_FAILED_OTHER`
    /// (anybody's — an interrupted cast simply stops, and before this the pose
    /// was held to its timer), and our own `CMSG_CANCEL_CAST`.
    pub casts_cancelled: u32,
    pub cast_time_ms: u32,
    /// The `Spell.dbc` id of the last cast this unit began or released.
    ///
    /// **The animation is a property of the spell, not of casting**, which is
    /// the fact this field exists to carry: `Spell.dbc` names a `SpellVisual`,
    /// which names two `SpellVisualKit`s, which name the wind-up and the
    /// release. A fireball is thrown from the shoulder, a heal is raised
    /// overhead, and opening a chest is a crouch over it — three different
    /// poses off one packet whose only distinguishing field is this one. The
    /// hop needs the archives and this crate has none, so it travels as an id
    /// exactly as `last_emote` does.
    pub last_spell: u32,
    /// **What the last released cast landed on**, or 0 for one that hit nothing
    /// the client can name.
    ///
    /// A second field beside [`Self::last_spell`] rather than part of it,
    /// because it is answered only by `SMSG_SPELL_GO` and only for the release:
    /// a wind-up has no hit list. It is what a *missile* flies at — a fireball
    /// leaves the caster's hand and has to arrive somewhere — and it is the one
    /// thing about a cast that is not about the caster at all.
    pub last_spell_target: u64,
    /// …and **everything** it landed on, which is a different question from the
    /// one above and has a different answer for every area spell in the game.
    ///
    /// The missile is one object and flies at one place, so
    /// [`Self::last_spell_target`] is a scalar and stays one. The *impact* is
    /// per victim: `SpellVisualKit`'s impact kit is the flash on the thing that
    /// was hit, and an Arcane Explosion catching five creatures owes five of
    /// them. Reading only the head of `SMSG_SPELL_GO`'s hit list gave four of
    /// the five nothing, with every count in the client reporting success.
    ///
    /// Empty for a cast that hit nothing nameable, and empty for every wind-up:
    /// `SMSG_SPELL_START` carries no list at all.
    pub last_spell_targets: Vec<u64>,
    /// **Pushback**: how many times the cast in progress has been knocked back
    /// by damage, and by how long the last one moved it.
    ///
    /// The same counter-plus-payload shape as the swing and the emote, and for
    /// the same reason — a poll cannot tell one delay from two without a
    /// counter. `SMSG_SPELL_DELAYED` is sent only to the caster and only for a
    /// cast already running, so what it means is "everything you are holding
    /// for this cast lasts this much longer": the held wind-up pose, the art on
    /// the caster's hands, and the bar. Nothing else in the protocol restates
    /// the cast's length after `SMSG_SPELL_START` has stated it once.
    pub casts_delayed: u32,
    pub last_cast_delay_ms: u32,
    /// The largest distance any single server statement has moved this entity,
    /// in yards, and how many statements moved it more than [`JUMP_YARDS`].
    ///
    /// **This is the instrument for "mobs teleport around".** A jump is not an
    /// error — the server really does relocate things — but a *frequent* one is
    /// the signature of the client walking a creature somewhere the server did
    /// not, and then being corrected. Without a number, the report is anecdotal
    /// and every plausible cause stays equally plausible; with one, the question
    /// becomes which packet is doing it and how far.
    pub worst_jump: f32,
    pub jumps: u32,
}

/// How far a single server statement may move an entity before it counts as a
/// jump rather than a correction.
///
/// A creature runs at about 7 yards a second and the server states its position
/// twice a second at most, so 10 yards is comfortably more than any legitimate
/// half-second of travel and comfortably less than the "it appeared somewhere
/// else" the report is about.
pub const JUMP_YARDS: f32 = 10.0;

impl Entity {
    fn new(guid: u64) -> Self {
        Entity {
            guid,
            object_type: None,
            position: None,
            fields: HashMap::new(),
            created: false,
            is_self: false,
            speeds: None,
            spline: None,
            transport_phase_ms: None,
            movement: None,
            forced_flags: crate::state::movement::FlagOverride::default(),
            facing_target: None,
            fall_secs: 0.0,
            position_updates: 0,
            grounded_for: None,
            attacking: None,
            client_controlled: false,
            swings_thrown: 0,
            last_swing_info: 0,
            last_swing_state: crate::play::action::victim_state::UNAFFECTED,
            last_swing_victim: 0,
            last_swing_damage: 0,
            damage_taken: 0,
            last_damage: 0,
            last_damage_info: 0,
            last_damage_state: 0,
            last_damage_spell: None,
            healed: false,
            blows_taken: 0,
            last_victim_state: crate::play::action::victim_state::UNAFFECTED,
            last_blow_info: 0,
            reactions: 0,
            last_reaction: 0,
            emotes: 0,
            last_emote: 0,
            spell_visuals: 0,
            last_spell_visual: 0,
            spell_impacts: 0,
            last_spell_impact: 0,
            casts_begun: 0,
            casts_released: 0,
            recent_spells: [0; RECENT_SPELLS],
            casts_channelled: 0,
            casts_landed: 0,
            casts_cancelled: 0,
            cast_time_ms: 0,
            last_spell: 0,
            last_spell_target: 0,
            last_spell_targets: Vec::new(),
            casts_delayed: 0,
            last_cast_delay_ms: 0,
            worst_jump: 0.0,
            jumps: 0,
        }
    }

    /// Record a position the server stated, as opposed to one the client
    /// simulated.
    ///
    /// **An impossible position is refused rather than adopted.** The client is
    /// not authoritative, so its only defence against a mis-read coordinate is to
    /// notice that it cannot be one: adopting it moves the entity thousands of
    /// yards, where it is outside every loaded tile and therefore invisible, and
    /// nothing brings it back — the entity has effectively vanished while the
    /// server still thinks it is standing next to you. Keeping the last good
    /// position instead degrades to a stale entity, which is visible and
    /// obviously wrong, and [`ObjectManager::rejected_positions`] counts it.
    ///
    /// Returns `None` if the position was refused, otherwise how far it moved
    /// the entity from wherever the client had walked it to. The bound is
    /// `MaNGOS::IsValidMapCoord`'s: `MAX_MAP_COORD` is 64 * 533.33333 / 2.
    ///
    /// That distance is the instrument for "mobs teleport around" — see
    /// [`Entity::worst_jump`]. The caller reports it, because only the caller
    /// knows which packet did it.
    fn set_server_position(&mut self, position: Position) -> Option<f32> {
        if !position.is_plausible() {
            return None;
        }
        let moved = self
            .position
            .map(|was| distance([was.x, was.y, was.z], [position.x, position.y, position.z]))
            .unwrap_or(0.0);
        if moved > JUMP_YARDS {
            self.jumps = self.jumps.saturating_add(1);
            self.worst_jump = self.worst_jump.max(moved);
        }
        self.position = Some(position);
        self.position_updates = self.position_updates.wrapping_add(1);
        Some(moved)
    }

    /// Take up the in-flight spline an update block carried, at the point the
    /// server is already at.
    ///
    /// `None` does nothing rather than clearing: a block without a spline is a
    /// block that says nothing about one, and most of them do not carry one at
    /// all. The caller clears where clearing is what the packet means.
    ///
    /// **`elapsed_ms` is the server's `timePassed`, and that is the whole point
    /// of reading this block.** Starting the path from zero would walk the
    /// creature back to the beginning of a patrol it is half way through, which
    /// is a worse teleport than the one this fixes.
    fn adopt_spline(&mut self, spline: Option<&crate::state::update::SplineUpdate>) {
        let Some(update) = spline else { return };
        if update.duration_ms == 0 || update.path.len() < 2 {
            return;
        }
        self.spline = Some(Spline {
            path: update.path.clone(),
            duration_ms: update.duration_ms,
            elapsed_ms: update.elapsed_ms.min(update.duration_ms),
            facing: update.facing,
            cyclic: update.flags & crate::state::movement::spline_flags::CYCLIC != 0,
            flying: update.flags & crate::state::movement::spline_flags::FLYING != 0,
            falling: update.flags & crate::state::movement::spline_flags::FALLING != 0,
            // An update block's spline section carries no transport reference —
            // only `SMSG_MONSTER_MOVE_TRANSPORT` does.
            on_transport: None,
        });
        // A server-driven move supersedes dead reckoning, the same way
        // `apply_monster_move`'s does.
        self.movement = None;
    }

    /// GUID low part — what vmangos shows in its console and database.
    pub fn low_guid(&self) -> u32 {
        (self.guid & 0xFFFF_FFFF) as u32
    }

    pub fn field(&self, index: u16) -> Option<u32> {
        self.fields.get(&index).copied()
    }

    pub fn field_f32(&self, index: u16) -> Option<f32> {
        self.field(index).map(f32::from_bits)
    }

    /// Template id — what `CMSG_CREATURE_QUERY` takes and what identifies
    /// *what* this thing is, as opposed to *which instance* the GUID names.
    /// `object::ENTRY` is at the same index for every object type.
    pub fn entry(&self) -> Option<u32> {
        self.field(fields::object::ENTRY)
    }

    pub fn scale(&self) -> Option<f32> {
        self.field_f32(fields::object::SCALE_X)
    }

    /// **`UNIT_FIELD_COMBATREACH` — how much of the distance between two units
    /// is not distance.**
    ///
    /// Every range in the game is measured between *surfaces* rather than
    /// centres: vmangos' `GetCombatDistance` is `GetDistance(target,
    /// SizeFactor::CombatReach)`, which subtracts both units' reaches, and
    /// `Spell::CheckRange` is written against that number. So a client checking
    /// a spell's range centre-to-centre refuses casts at a large creature that
    /// the server would have taken — which is the one-sided error a local range
    /// check must not make.
    ///
    /// `None` for anything the field block has not stated, which the caller
    /// reads as [`DEFAULT_COMBAT_REACH`] rather than as zero.
    pub fn combat_reach(&self) -> Option<f32> {
        self.field_f32(fields::unit::COMBATREACH)
    }

    /// **`UNIT_FIELD_BOUNDINGRADIUS` — how wide the thing standing there is.**
    ///
    /// [`Self::combat_reach`]'s neighbour on the wire and its neighbour in
    /// meaning: the reach is how much of a *gap* is bulk, and this is the bulk
    /// itself — the radius of the unit's footprint on the ground, in world
    /// yards, with the object's scale already folded in.
    ///
    /// It is the only footprint the game states. The M2's own bounding box is
    /// authored to cover **every frame of every animation** it has, so it is a
    /// different number about a different question: `Creature\Boar\Boar.m2`
    /// declares a horizontal half-extent of 2.5 model yards where the server
    /// sends 0.882 for the Rockhide Boar that wears it — a factor of two and a
    /// half, and the whole of "the selection circle is far too large".
    ///
    /// vmangos writes it in `Unit::UpdateModelData` from
    /// `creature_display_info_addon.bounding_radius`, scaled by
    /// `GetObjectScale() / nativeScale` — so the value on the wire is already
    /// world-space and a reader must **not** apply `OBJECT_FIELD_SCALE_X` to it
    /// a second time.
    ///
    /// `None` for anything the field block has not stated, which the caller
    /// reads as its own `DEFAULT_BOUNDING_RADIUS` rather than as zero — a zero
    /// here is a ring of nothing.
    pub fn bounding_radius(&self) -> Option<f32> {
        self.field_f32(fields::unit::BOUNDINGRADIUS)
    }

    pub fn is_unit_like(&self) -> bool {
        matches!(
            self.object_type,
            Some(ObjectType::Unit) | Some(ObjectType::Player)
        )
    }

    /// Health, level and display id read from the *unit* field block.
    ///
    /// These return `None` for non-units on purpose: field indices overlap
    /// between object types, so reading `unit::HEALTH` off a game object would
    /// silently return that object's unrelated field 22.
    ///
    /// **Health is often a percentage, not hit points.** Unless the server runs
    /// with `ShowHealthValues = 1`, vmangos hides real values from players who
    /// cannot "see health of" a unit: it sends max = 100 and current = percent
    /// (clamped to 1 when alive). A whole zone reporting `100/100` is the
    /// server behaving correctly, not a parsing bug — see
    /// [`Self::health_is_percentage`].
    ///
    /// **A unit whose create block did not mention its health has none**, and
    /// that is the whole of "a creature that died before I arrived stands there
    /// idling". `_SetCreateBits` sends a field only when it is non-zero, and the
    /// health of a corpse is exactly zero — so the one value that matters most
    /// here is the one value the server never states. A creature killed *in*
    /// view is a different packet and was always right: a values update carries
    /// what changed, and a change to zero is a change.
    ///
    /// **Only this accessor takes [`Self::created`] into account, and that is
    /// deliberate rather than lazy.** The rule is true of every field, but its
    /// consequences are not: `combat_reach` of `None` means
    /// [`DEFAULT_COMBAT_REACH`] where a stated 0.0 would tighten every range
    /// check, and [`Self::target`] of `None` means "nobody" where a stated 0
    /// would be a guid. Turning silence into zero across the board trades one
    /// class of bug for several, so it is applied where zero is *the answer*.
    pub fn health(&self) -> Option<u32> {
        if !self.is_unit_like() {
            return None;
        }
        match self.field(fields::unit::HEALTH) {
            Some(health) => Some(health),
            None => self.created.then_some(0),
        }
    }

    pub fn max_health(&self) -> Option<u32> {
        self.is_unit_like()
            .then(|| self.field(fields::unit::MAXHEALTH))
            .flatten()
    }

    pub fn level(&self) -> Option<u32> {
        self.is_unit_like()
            .then(|| self.field(fields::unit::LEVEL))
            .flatten()
    }

    /// `PLAYER_XP` and `PLAYER_NEXT_LEVEL_XP` — `UnitXP` and `UnitXPMax`.
    ///
    /// **Zero rather than absent for a player the server has finished
    /// describing**, on [`Self::health`]'s argument and for a sharper reason: a
    /// character that has just dinged has *exactly* zero experience, so silence
    /// is the ordinary case here rather than the edge one (`_SetCreateBits`
    /// omits every zero field).
    ///
    /// And the pair has to move together, because `TextStatusBar_UpdateTextString`
    /// **hides a bar whose maximum is zero** — which is the whole of why the XP
    /// bar was missing: `UnitXPMax` answered a stubbed 0, `MainMenuExpBar_Update`
    /// fed it to `SetMinMaxValues`, and the game's own code took the bar off the
    /// screen. Working as designed on a wrong answer.
    ///
    /// `PRIVATE`, so both are our own character's and nobody else's.
    pub fn experience(&self) -> Option<(u32, u32)> {
        if self.object_type != Some(ObjectType::Player) {
            return None;
        }
        let xp = self.field(fields::player::XP);
        let next = self.field(fields::player::NEXT_LEVEL_XP);
        match (xp, next) {
            (None, None) if !self.created => None,
            _ => Some((xp.unwrap_or(0), next.unwrap_or(0))),
        }
    }

    /// `PLAYER_CHARACTER_POINTS1` and `2` — `UnitCharacterPoints("player")`'s
    /// pair: **unspent talent points, then unspent profession points.**
    ///
    /// Zero-rather-than-absent on [`Self::experience`]'s own argument, and here
    /// the ordinary case is even more strongly zero: a character below level 10
    /// has no talent points at all, and `_SetCreateBits` omits the field, so a
    /// reader that answered `None` for silence would report "we do not know"
    /// for the majority of every session.
    ///
    /// **This is the only counter in the talent panel that crosses the wire.**
    /// How many points are *spent* is worked back out of the known spells — see
    /// [`crate::play::talents`], which is where that is argued. `PRIVATE`, so it
    /// is our own character's and nobody else's.
    pub fn character_points(&self) -> Option<(u32, u32)> {
        if self.object_type != Some(ObjectType::Player) {
            return None;
        }
        let talent = self.field(fields::player::CHARACTER_POINTS1);
        let profession = self.field(fields::player::CHARACTER_POINTS2);
        match (talent, profession) {
            (None, None) if !self.created => None,
            _ => Some((talent.unwrap_or(0), profession.unwrap_or(0))),
        }
    }

    /// `PLAYER_REST_STATE_EXPERIENCE` — `GetXPExhaustion()`, the rested pool.
    ///
    /// **`nil` when it is zero, and that is the interface's own test**:
    /// `ExhaustionTick_Update` reads `if ( not exhaustionThreshold ) then
    /// ExhaustionTick:Hide()`, so a rested character with no rest left has to
    /// answer nothing rather than 0 or the tick sits at the left edge of the bar.
    pub fn rested_experience(&self) -> Option<u32> {
        (self.object_type == Some(ObjectType::Player))
            .then(|| self.field(fields::player::REST_STATE_EXPERIENCE))
            .flatten()
            .filter(|rested| *rested > 0)
    }

    /// `UNIT_FIELD_BYTES_0` byte 3: which of the five powers this unit runs on.
    ///
    /// **There are five power fields and only one of them is the unit's**, so
    /// this byte decides which to read — a warrior's `POWER1` (mana) is
    /// permanently zero and a rogue's is too, which is why a bar drawn off the
    /// first field is empty for half the classes in the game. See
    /// [`power_type`] for the names.
    pub fn power_type(&self) -> Option<u8> {
        let bytes = self
            .is_unit_like()
            .then(|| self.field(fields::unit::BYTES_0))
            .flatten()?;
        Some(((bytes >> 24) & 0xFF) as u8)
    }

    /// `UNIT_FIELD_BYTES_0` bytes 0 and 1: **race and class**, the one-based
    /// ids `ChrRaces.dbc` and `ChrClasses.dbc` are keyed by.
    ///
    /// The same field [`Self::power_type`] takes its byte from, and the same
    /// field [`Self::appearance`] takes the race from — kept apart because the
    /// callers are: dressing a character wants the appearance block and the
    /// spellbook wants only these two. Class is *not* in the appearance struct,
    /// because nothing about what a character looks like depends on it.
    ///
    /// Race 0 is not a race, so an absent field and a zeroed one are the same
    /// answer here — see the note on [`Self::appearance`], which relies on that.
    pub fn race_and_class(&self) -> Option<(u8, u8)> {
        let bytes = self
            .is_unit_like()
            .then(|| self.field(fields::unit::BYTES_0))
            .flatten()?;
        Some((bytes as u8, (bytes >> 8) as u8))
    }

    /// `UNIT_FIELD_BYTES_0` byte **2**: the unit's gender, as `UnitSex` reads it.
    ///
    /// The third byte of the same field [`Self::race_and_class`] takes the first
    /// two from, and the one [`Self::appearance`] already unpacks — kept apart
    /// for the reason those two are: `UnitSex` is asked about *any* unit, and
    /// the appearance block answers `None` for everything that is not a player.
    /// A boar has a gender byte and the interface may ask about it.
    ///
    /// Read at `[unitFields + 0x7a]`, which is index 6 + 30 = 36 —
    /// `UNIT_FIELD_BYTES_0` — with the byte-2 offset that makes it the third of
    /// the four. Zero is male and a real answer, so an absent field is `None`
    /// rather than zero: see [`crate::play::stats`] for the same distinction.
    pub fn gender(&self) -> Option<u8> {
        let bytes = self
            .is_unit_like()
            .then(|| self.field(fields::unit::BYTES_0))
            .flatten()?;
        Some((bytes >> 16) as u8)
    }

    /// The unit's current power, from whichever of the five fields
    /// [`Self::power_type`] names.
    pub fn power(&self) -> Option<u32> {
        self.power_field(fields::unit::POWER1)
    }

    pub fn max_power(&self) -> Option<u32> {
        self.power_field(fields::unit::MAXPOWER1)
    }

    /// `POWER1`/`MAXPOWER1` plus the power type's offset. The five are
    /// consecutive in both blocks, which is what makes this arithmetic rather
    /// than a match.
    fn power_field(&self, first: u16) -> Option<u32> {
        let kind = self.power_type()?;
        if u32::from(kind) > power_type::HAPPINESS {
            return None;
        }
        self.field(first + u16::from(kind))
    }

    pub fn faction(&self) -> Option<u32> {
        self.is_unit_like()
            .then(|| self.field(fields::unit::FACTIONTEMPLATE))
            .flatten()
    }

    /// Is this unit dead?
    ///
    /// **Health is the signal, not the stand state.** `UNIT_FIELD_BYTES_1`'s
    /// `UNIT_STAND_STATE_DEAD` exists and is set on some corpses, but a creature
    /// killed in front of you keeps whatever stand state it was in — the server
    /// only writes that byte where it wants a *pose*, and the thing it always
    /// writes is the health. Zero is unambiguous either way: with
    /// `ShowHealthValues = 0` the field is a percentage clamped to 1 while the
    /// unit is alive, so 0 means dead in both encodings.
    ///
    /// `None` for anything that never had health — a game object, an item.
    pub fn is_dead(&self) -> Option<bool> {
        Some(self.health()? == 0)
    }

    /// **`PLAYER_FARSIGHT` — what this character is looking through.**
    ///
    /// One field for two spells' worth of behaviour, and the client's whole
    /// input for both. `Player::ScheduleCameraUpdate` writes it — and sends it
    /// on its own, `DirectSendPublicValueUpdate(PLAYER_FARSIGHT, 2)` — whenever
    /// `Camera::SetView` moves the server's view point, which happens in two
    /// places:
    ///
    /// * **Far sight** (Eagle Eye, Far Sight, Bird's Eye): `SetLongSight`
    ///   creates a `DynamicObject` at the map's visibility distance in front of
    ///   the caster and points the camera at it. So the guid names a
    ///   `DynamicObject` this client already streams in and already places —
    ///   the same object type a Blizzard's ring rides on.
    /// * **Possess** (Eye of Kilrogg, Mind Control, Eyes of the Beast):
    ///   `Unit::ModPossess` points the camera at the possessed *unit*.
    ///
    /// `None` for the ordinary case, which is what the client tests: it ors the
    /// two halves of this field and does nothing at all when the result is zero.
    ///
    /// Only a player has one — the field is `PLAYER_FARSIGHT` and index 712 on
    /// a creature is another block entirely — and in practice only ever the
    /// local one, since the field is not in a group any other client is sent.
    pub fn farsight(&self) -> Option<u64> {
        if self.object_type != Some(ObjectType::Player) {
            return None;
        }
        let low = u64::from(self.field(fields::player::FARSIGHT)?);
        let high = u64::from(self.field(fields::player::FARSIGHT + 1).unwrap_or(0));
        let guid = (high << 32) | low;
        (guid != 0).then_some(guid)
    }

    /// **The spirit has been released** — `PLAYER_FLAGS & PLAYER_FLAGS_GHOST`.
    ///
    /// Not derivable from the health, which is the reason this is read at all: a
    /// corpse is health 0, and `Player::BuildPlayerRepop` sets the *ghost* to
    /// health 1 — the same 1 a nearly-dead living player has. The two states
    /// differ in what may be done next (release, against walk to the body) and
    /// nothing else says which one this is.
    ///
    /// `false` for anything that is not a player: the field is `PLAYER_FLAGS`
    /// and index 190 on a creature is another block entirely.
    pub fn is_ghost(&self) -> bool {
        self.player_flags() & crate::play::death::PLAYER_FLAGS_GHOST != 0
    }

    /// `PLAYER_FLAGS` whole, zero for anything that is not a player.
    pub fn player_flags(&self) -> u32 {
        (self.object_type == Some(ObjectType::Player))
            .then(|| self.field(fields::player::FLAGS))
            .flatten()
            .unwrap_or(0)
    }

    /// `PLAYER_DUEL_ARBITER` — **the flag object two duellists both point at**,
    /// and `0` when there is no duel.
    ///
    /// Read for friend-or-foe rather than for the duel window: the client
    /// settles the reaction between two players on the arbiter and the team
    /// before it looks at either faction, which is the only way two members of
    /// one faction become hostile without a flag.
    pub fn duel_arbiter(&self) -> u64 {
        if self.object_type != Some(ObjectType::Player) {
            return 0;
        }
        let low = u64::from(self.field(fields::player::DUEL_ARBITER).unwrap_or(0));
        let high = u64::from(self.field(fields::player::DUEL_ARBITER + 1).unwrap_or(0));
        (high << 32) | low
    }

    /// `PLAYER_DUEL_TEAM` — which side of that duel, `0` for neither.
    pub fn duel_team(&self) -> u32 {
        (self.object_type == Some(ObjectType::Player))
            .then(|| self.field(fields::player::DUEL_TEAM))
            .flatten()
            .unwrap_or(0)
    }

    /// `PLAYER_FIELD_BYTES` byte 0 — the flag byte `Player::KillPlayer` writes
    /// the release-window policy into.
    ///
    /// **A different field from `PLAYER_BYTES`**, which is the appearance, and
    /// the two once collided in the field table's name shortening:
    /// `PLAYER_FIELD_BYTES` was being dropped from the table outright, which is
    /// not a compile error anywhere — it is a field nobody can ask about.
    ///
    /// `PRIVATE`, so it only ever arrives for our own character. See
    /// [`crate::play::death::RELEASE_TIMER`] and
    /// [`crate::play::death::NO_RELEASE_WINDOW`] for the two bits that matter here.
    pub fn player_field_flags(&self) -> u32 {
        (self.object_type == Some(ObjectType::Player))
            .then(|| self.field(fields::player::FIELD_BYTES))
            .flatten()
            .unwrap_or(0)
            & 0xFF
    }

    /// `PLAYER_FIELD_BYTES` byte **2** — **which of the four extra action bars
    /// the character has switched on**, as a bit per bar.
    ///
    /// The same field [`Self::player_field_flags`] reads byte 0 of, and
    /// `PRIVATE` for the same reason: it is nobody else's business which bars
    /// are on your screen. It is the *only* thing in this protocol that says so
    /// — the four bars are `Interface\FrameXML\MultiActionBars.xml`'s frames and
    /// the server has no other opinion about them — and it is written by exactly
    /// one packet, [`crate::play::spells::set_actionbar_toggles_body`], whose whole
    /// body is this byte.
    ///
    /// See [`crate::play::spells::multi_bar`] for what each bit is. Checked
    /// against the client's own reader, which reaches this byte through the
    /// descriptor at `+0xe68` and then returns four values, one per bit — and
    /// against vmangos'
    /// `HandleSetActionBarTogglesOpcode`, which stores it at
    /// `PLAYER_FIELD_BYTES_OFFSET_ACTION_BARS`.
    pub fn action_bar_toggles(&self) -> u8 {
        (self.object_type == Some(ObjectType::Player))
            .then(|| self.field(fields::player::FIELD_BYTES))
            .flatten()
            .map_or(0, |v| ((v >> 16) & 0xFF) as u8)
    }

    /// **`PLAYER_FIELD_BYTES` byte 1 — how many combo points are on the
    /// target**, which is what greys every rogue and druid finisher.
    ///
    /// The byte next door to [`Self::action_bar_toggles`], and reached the same
    /// way. It is the *count*; who they are on is
    /// `PLAYER_FIELD_COMBO_TARGET`, and the interface never asks — a finisher
    /// with points on somebody else is refused by the server, not greyed by the
    /// client.
    pub fn combo_points(&self) -> u8 {
        (self.object_type == Some(ObjectType::Player))
            .then(|| self.field(fields::player::FIELD_BYTES))
            .flatten()
            .map_or(0, |v| ((v >> 8) & 0xFF) as u8)
    }

    /// **`UNIT_FIELD_AURASTATE` — the states a spell may require.**
    ///
    /// A bitmask, and the bit for state *n* is `1 << (n - 1)`: vmangos'
    /// `ModifyAuraState` writes `SetFlag(UNIT_FIELD_AURASTATE, 1 << (flag - 1))`.
    /// The states are `AURA_STATE_*` in `SpellDefines.h` and the ones marked
    /// `C` there are the caster-side ones a `Spell.dbc` row asks for:
    ///
    /// ```text
    /// 1  DEFENSE                    you blocked, parried or dodged  -> Revenge
    /// 2  HEALTHLESS_20_PERCENT      caster or target under a fifth  -> Execute
    /// 3  BERSERKING
    /// 5  JUDGEMENT                  a Seal is up                    -> Judgement
    /// 7  HUNTER_PARRY                                               -> Riposte
    /// 8  ROGUE_ATTACK_FROM_STEALTH
    /// ```
    ///
    /// This is the field that makes "why is Judgement grey" answerable without
    /// the client knowing what a Seal is: the server sets the bit when the aura
    /// lands and clears it when it goes, and the button follows the bit.
    pub fn aura_state(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::AURASTATE))
            .flatten()
            .unwrap_or(0)
    }

    /// `UNIT_FIELD_BYTES_1` byte 0 (`UNIT_BYTES_1_OFFSET_STAND_STATE`): 0 stand,
    /// 1..6 the sitting variants, 7 dead, 8 kneeling.
    ///
    /// What makes this worth reading is the innkeeper on a stool and the guard
    /// asleep at his post: they are numerous, they are stationary, and drawn
    /// standing they read as models floating through the furniture.
    pub fn stand_state(&self) -> Option<u8> {
        self.is_unit_like()
            .then(|| self.field(fields::unit::BYTES_1))
            .flatten()
            .map(|v| (v & 0xFF) as u8)
    }

    /// `UNIT_FIELD_BYTES_1` byte **3** (`UNIT_BYTES_1_OFFSET_VIS_FLAG`) — the
    /// three bits that say a unit is **not to be drawn as an ordinary solid
    /// body**.
    ///
    /// ```text
    /// 0x01  GHOST        SPELL_AURA_GHOST — the spirit at the graveyard
    /// 0x02  CREEP        SPELL_AURA_MOD_STEALTH — every stealth and prowl
    /// 0x04  UNTRACKABLE  SPELL_AURA_UNTRACKABLE
    /// ```
    ///
    /// vmangos' `UnitVisFlags` names them and `Aura::HandleModStealth` is what
    /// sets the middle one (`SpellAuras.cpp:3614`). **There is no packet and no
    /// other field**: `Spell.dbc` gives Stealth (1784) no visual at all — no
    /// kit, no pose, no models — so this byte is the only thing in the protocol
    /// that says a rogue has gone into the shadows, and everything the client
    /// does about it is keyed off these three bits and nothing else.
    ///
    /// The client reads bit `0x02` in exactly two places and they are the two
    /// halves of the gait: the moving cascade (`-> StealthWalk`) and the idle
    /// one (`-> StealthStand`). See
    /// `crate::state::objects::WorldEntity::creeping` and the pose chooser in
    /// the renderer.
    pub fn vis_flags(&self) -> u8 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::BYTES_1))
            .flatten()
            .map_or(0, |v| ((v >> 24) & 0xFF) as u8)
    }

    /// `UNIT_FIELD_BYTES_1` byte **2** (`UNIT_BYTES_1_OFFSET_SHAPESHIFT_FORM`):
    /// which form this unit is in, as a `SpellShapeshiftForm.dbc` row.
    ///
    /// 0 is no form. The warrior stances are 17, 18 and 19
    /// (`FORM_BATTLESTANCE`..`FORM_BERSERKERSTANCE` in vmangos'
    /// `SharedDefines.h`), the druid forms 1..5, and there are 32 rows in all.
    ///
    /// **It is the whole of what decides which action bar is on screen.** The
    /// row's `bonusActionBar` column is `GetBonusBarOffset()`, which is what
    /// `ActionButton_GetPagedID` adds 72 to — so a warrior in Battle Stance
    /// presses slots 73..84 and a character whose form has no bonus bar presses
    /// the ordinary page. See `vale_assets::tables::spellbook::ShapeshiftForms`.
    pub fn shapeshift_form(&self) -> u8 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::BYTES_1))
            .flatten()
            .map_or(0, |v| ((v >> 16) & 0xFF) as u8)
    }

    /// `GAMEOBJECT_STATE` — whether a door, a chest or a lever is *closed* or
    /// *open*, and the only thing the server ever says about how a game object
    /// should look.
    ///
    /// `GO_STATE_ACTIVE` (0) is the used state — a door standing open, a chest
    /// with its lid up — `GO_STATE_READY` (1) is the reset one, and
    /// `GO_STATE_ACTIVE_ALTERNATIVE` (2) is a second used state
    /// (`GameObjectDefines.h`). **Zero being the open state is what makes the
    /// absent case correct**: vmangos omits a field whose value is zero, so a
    /// game object that says nothing about its state is one that is open, which
    /// is exactly what this returns.
    ///
    /// `None` for everything that is not a game object, because the index is
    /// another type's field entirely — 14 is `UNIT_FIELD_POWER4` on a unit.
    pub fn game_object_state(&self) -> Option<u8> {
        (self.object_type == Some(ObjectType::GameObject))
            .then(|| self.field(fields::game_object::STATE))
            .flatten()
            .map(|v| (v & 0xFF) as u8)
    }

    /// `GAMEOBJECT_FLAGS` — the three bits that decide whether a click on this
    /// does anything, and one that decides whether the plate says "Locked".
    ///
    /// `None` for everything that is not a game object, on
    /// [`Self::game_object_state`]'s own terms: index 9 is `UNIT_FIELD_POWER1`
    /// on a unit. See `vale_assets::look::object::go_flags`, which owns what
    /// the bits mean, and [`Self::game_object_dyn_flags`], which is the word
    /// one of them is read together with.
    pub fn game_object_flags(&self) -> Option<u32> {
        (self.object_type == Some(ObjectType::GameObject))
            .then(|| self.field(fields::game_object::FLAGS))
            .flatten()
    }

    /// `GAMEOBJECT_DYN_FLAGS` — **the server's per-player answer** to whether
    /// this particular character may act on this object.
    ///
    /// It is the other half of `GO_FLAG_INTERACT_COND`: the template says "ask
    /// first", and this is the answer, sent in the private part of the update
    /// block. A chest that is yours to open and one that is somebody else's
    /// differ in exactly this word and in nothing else.
    ///
    /// **Absent means zero and that is load-bearing here.** `_SetCreateBits`
    /// omits a zero field, so an object nobody may touch says nothing at all —
    /// which is the same thing the bit being clear says.
    pub fn game_object_dyn_flags(&self) -> Option<u32> {
        (self.object_type == Some(ObjectType::GameObject))
            .then(|| self.field(fields::game_object::DYN_FLAGS))
            .flatten()
    }

    /// `GAMEOBJECT_LEVEL` — the rank a lock slot wanting **0** falls back to,
    /// five times over.
    ///
    /// Nearly always absent, which reads as zero and is correct: vmangos writes
    /// the field for transports (`transport.pause`) and for nothing else. See
    /// `vale_assets::look::object::can_open`, its one reader.
    pub fn game_object_level(&self) -> Option<u32> {
        (self.object_type == Some(ObjectType::GameObject))
            .then(|| self.field(fields::game_object::LEVEL))
            .flatten()
    }

    /// What a **creature** has in its hands, as `ItemDisplayInfo` ids: main
    /// hand, off hand, ranged.
    ///
    /// **No round trip.** `Creature::SetVirtualItem` writes
    /// `proto->DisplayInfoID` straight into `UNIT_VIRTUAL_ITEM_SLOT_DISPLAY`, so
    /// for a creature the display id is already on the wire — where a *player's*
    /// `PLAYER_VISIBLE_ITEM_n_0` carries an item entry that has to be resolved by
    /// `CMSG_ITEM_QUERY_SINGLE`. The same row also writes the item's class,
    /// subclass, material and inventory type into `UNIT_VIRTUAL_ITEM_INFO`, and
    /// its **sheath type** into the second word of each pair — which is the
    /// thing once thought to live only deep in the item-query tail. It
    /// does, for players; for creatures it is here.
    pub fn weapon_displays(&self) -> [u32; 3] {
        let mut out = [0u32; 3];
        for (slot, id) in out.iter_mut().enumerate() {
            *id = self
                .is_unit_like()
                .then(|| self.field(fields::unit::VIRTUAL_ITEM_SLOT_DISPLAY + slot as u16))
                .flatten()
                .unwrap_or(0);
        }
        out
    }

    /// What a **creature** is carrying, in full: the display id and the four
    /// facts about the item beside it.
    ///
    /// `UNIT_VIRTUAL_ITEM_INFO` is *two* words per slot, and
    /// `Creature::SetVirtualItem` packs them with `SetByteValue`: class,
    /// subclass, material and inventory type into the bytes of the first,
    /// **sheath type** into byte 0 of the second. Six fields for three slots,
    /// starting at index 40.
    ///
    /// Empty for a **player** — the indices exist on one but are never written,
    /// because a player's gear is `PLAYER_VISIBLE_ITEM_n_0` and needs the query
    /// round trip. [`ObjectManager::weapons_of`] is the one that knows both.
    /// `UNIT_NPC_EMOTESTATE` — an `Emotes.dbc` id the unit is **holding**.
    ///
    /// **The other half of the emote system, and the more visible one.**
    /// `Unit::HandleEmote` branches on the row's `EmoteType`: a one-shot goes
    /// out as `SMSG_EMOTE` and a *state* is written here instead, where it
    /// simply stays. So `/wave` is a packet and `/dance` is this field — and so
    /// is every innkeeper permanently at work, every guard standing in a ready
    /// pose, and every stunned creature.
    ///
    /// Zero is "no held emote", which is the overwhelming majority and is why
    /// the field is usually absent from an update block altogether.
    pub fn emote_state(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::NPC_EMOTESTATE))
            .flatten()
            .unwrap_or(0)
    }

    pub fn virtual_items(&self) -> [HeldItem; 3] {
        let mut out = [HeldItem::default(); 3];
        if !self.is_unit_like() {
            return out;
        }
        for (slot, weapon) in out.iter_mut().enumerate() {
            let slot = slot as u16;
            let display = self
                .field(fields::unit::VIRTUAL_ITEM_SLOT_DISPLAY + slot)
                .unwrap_or(0);
            if display == 0 {
                continue;
            }
            let info = self
                .field(fields::unit::VIRTUAL_ITEM_INFO + slot * 2)
                .unwrap_or(0);
            let sheath = self
                .field(fields::unit::VIRTUAL_ITEM_INFO + slot * 2 + 1)
                .unwrap_or(0);
            *weapon = HeldItem {
                display_id: display,
                class: (info & 0xFF) as u8,
                subclass: ((info >> 8) & 0xFF) as u8,
                // **Byte 2 is the material**, which decides the sound a blow
                // makes and nothing that is drawn — which is why it was read
                // past for eight rounds and why every mace in the game had a
                // one-in-two chance of sounding like the other kind of mace.
                // `VIRTUAL_ITEM_INFO_0_OFFSET_MATERIAL` is 2.
                material: ((info >> 16) & 0xFF) as u8,
                inventory_type: ((info >> 24) & 0xFF) as u8,
                sheath: (sheath & 0xFF) as u8,
            };
        }
        out
    }

    /// The item **entries** in the three slots a weapon can be worn in:
    /// main hand, off hand, ranged.
    ///
    /// `EQUIPMENT_SLOT_MAINHAND` is 15 and the three are consecutive, which is
    /// the same order [`Self::virtual_items`] uses — so the two routes to a
    /// unit's weapons line up slot for slot and the caller does not have to
    /// care which one it took.
    pub fn weapon_entries(&self) -> Option<[u32; 3]> {
        const EQUIPMENT_SLOT_MAINHAND: usize = 15;
        let equipment = self.equipment()?;
        Some([
            equipment[EQUIPMENT_SLOT_MAINHAND],
            equipment[EQUIPMENT_SLOT_MAINHAND + 1],
            equipment[EQUIPMENT_SLOT_MAINHAND + 2],
        ])
    }

    /// **What this unit is riding**, as a `CreatureDisplayInfo` id —
    /// `UNIT_FIELD_MOUNTDISPLAYID`, which is the wire's one and only mounted
    /// signal and resolves through the same hop every other model takes.
    ///
    /// Zero is "not mounted" rather than "display id 0", which is what makes
    /// the filter here the whole of [`Self::mounted`]: vmangos writes the field
    /// with `SetUInt32Value(UNIT_FIELD_MOUNTDISPLAYID, 0)` to dismount, and
    /// `_SetCreateBits` omits a zero field entirely, so an unmounted unit says
    /// nothing at all about it.
    pub fn mount_display_id(&self) -> Option<u32> {
        self.is_unit_like()
            .then(|| self.field(fields::unit::MOUNTDISPLAYID))
            .flatten()
            .filter(|v| *v != 0)
    }

    /// **Is this unit riding something?**
    ///
    /// Read for a second reason beside drawing the mount: a mounted rider's
    /// weapons are **stowed** and stay stowed for as long as they are up there
    /// (`vale_assets::look::sheath::reconcile`'s persistent draw-block), so a
    /// character who takes a griffon with a sword out otherwise rides the whole
    /// way holding it.
    pub fn mounted(&self) -> bool {
        self.mount_display_id().is_some()
    }

    /// `UNIT_FIELD_BYTES_2` byte 0: 0 unarmed, 1 melee drawn, 2 ranged drawn.
    ///
    /// **For our own character this is an echo, not a source.** Nothing in
    /// vmangos writes a `Player`'s except `HandleSetSheathedOpcode`, so what
    /// arrives here is the last `CMSG_SETSHEATHED` *we* sent — which is why the
    /// client keeps its own committed state and only adopts this when it
    /// changes underneath it. See `vale_assets::look::sheath`. A creature's is
    /// genuinely the server's: `Creature::Create` sets melee.
    ///
    /// This is what says whether a weapon is in the hand or on the back, and it
    /// is the only part of the question this client can answer — *where* a
    /// sheathed weapon hangs is the item's sheath type against a client-side
    /// table of attachment points, which is a rule that never crosses the wire.
    pub fn sheath_state(&self) -> u8 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::BYTES_2))
            .flatten()
            .map(|v| (v & 0xFF) as u8)
            .unwrap_or(0)
    }

    /// **What this unit's `pet` token names** — `UNIT_FIELD_CHARM` if it is
    /// set, otherwise `UNIT_FIELD_SUMMON`.
    ///
    /// The precedence is the client's own and it is measured rather than
    /// guessed: the token resolver's `"pet"` branch reads the unit
    /// descriptor block at `[obj+0x110]`, tests the 64-bit value at `+0x00` and
    /// falls through to the one at `+0x08` only when it is zero. That block
    /// begins at `UNIT_FIELD_CHARM` — `OBJECT_END` is 6, so charm is unit field
    /// zero — which puts `+0x00` at [`fields::unit::CHARM`] and `+0x08` at
    /// [`fields::unit::SUMMON`]; the same base makes the `target` suffix's
    /// `+0x28` land on [`fields::unit::TARGET`], which is the cross-check that
    /// the offsets were read the right way round.
    ///
    /// **Charm first matters**, and not only for warlocks: a mind-controlled
    /// creature is the caster's `pet` for as long as the control lasts, and a
    /// client reading `SUMMON` alone shows the warlock's imp on the pet frame
    /// while the thing actually under their command is somebody else.
    ///
    /// `None` rather than `Some(0)` for a unit with neither, which is what
    /// makes `UnitExists("pet")` false — see [`crate::state::fields`] for why a
    /// zero guid is never a real one.
    pub fn pet_guid(&self) -> Option<u64> {
        if !self.is_unit_like() {
            return None;
        }
        let pair = |first: u16| {
            let low = u64::from(self.field(first)?);
            let high = u64::from(self.field(first + 1).unwrap_or(0));
            let guid = (high << 32) | low;
            (guid != 0).then_some(guid)
        };
        pair(fields::unit::CHARM).or_else(|| pair(fields::unit::SUMMON))
    }

    /// `UNIT_FIELD_SUMMONEDBY` — **who summoned this unit**, which is not the
    /// same question as who is commanding it.
    ///
    /// The pair with [`Self::pet_guid`], and the two come apart in exactly the
    /// case that matters: a mind-controlled creature is the caster's *charm* and
    /// was summoned by nobody, so it answers the `pet` token while this stays
    /// empty. `PetCanBeAbandoned` is written on that difference —
    /// it compares this field against the local player's own guid before it
    /// looks at the flag — which is what stops the pet menu from offering to
    /// abandon somebody else's mob.
    pub fn summoned_by(&self) -> Option<u64> {
        if !self.is_unit_like() {
            return None;
        }
        let low = u64::from(self.field(fields::unit::SUMMONEDBY)?);
        let high = u64::from(self.field(fields::unit::SUMMONEDBY + 1).unwrap_or(0));
        let guid = (high << 32) | low;
        (guid != 0).then_some(guid)
    }

    /// **The five numbers a hunter's pet panel is drawn from**, or `None` for
    /// anything that is not a pet.
    ///
    /// One accessor rather than five because the panel reads all of them
    /// together and because the gate is the same for each: `UNIT_FIELD_PETNUMBER`
    /// non-zero, which is what separates a real pet from a totem or a guardian.
    /// Every one is checked against the client's own getters — see
    /// `vale_assets::tables::pet`'s module comment.
    ///
    /// ```text
    /// UNIT_FIELD_POWER5        27   the happiness value
    /// UNIT_FIELD_BYTES_1 b1   138   the loyalty level, 1..6
    /// UNIT_FIELD_PETEXPERIENCE 141
    /// UNIT_FIELD_PETNEXTLEVELEXP 142
    /// UNIT_FIELD_TRAINING_POINTS 149  two shorts: total, spent
    /// ```
    ///
    /// **The training points are read high-half-first**, which is the one thing
    /// here a reader gets backwards without noticing: the client pushes the
    /// short at `+0x23e` before the one at `+0x23c`, and `PetPaperDollFrame`
    /// subtracts the second from the first — so the *high* half is the total.
    pub fn pet_stats(&self) -> Option<PetStats> {
        if !self.is_unit_like() || self.pet_number() == 0 {
            return None;
        }
        let bytes_1 = self.field(fields::unit::BYTES_1).unwrap_or(0);
        let training = self.field(fields::unit::TRAINING_POINTS).unwrap_or(0);
        Some(PetStats {
            happiness: self.field(fields::unit::POWER5).unwrap_or(0) as i32,
            loyalty_level: (bytes_1 >> 8) & 0xFF,
            experience: self.field(fields::unit::PETEXPERIENCE).unwrap_or(0),
            next_level_experience: self.field(fields::unit::PETNEXTLEVELEXP).unwrap_or(0),
            training_total: (training >> 16) as u16,
            training_spent: (training & 0xFFFF) as u16,
        })
    }

    /// `UNIT_FIELD_CREATED_BY_SPELL` — **which spell summoned this unit**, or
    /// `None` for one nothing did.
    ///
    /// Read for one thing and it is a *word*: the combat log says "%s is
    /// destroyed." rather than "%s dies." for a unit whose summoning spell has
    /// one of ten `Effect[0]` values, all of them object summons — see
    /// `vale_assets::interface::combatlog::is_destroyed`. The client reads the
    /// same field at the same index (`+0x230` off a block that starts at
    /// `UNIT_FIELD_CHARM`).
    pub fn created_by_spell(&self) -> Option<u32> {
        if !self.is_unit_like() {
            return None;
        }
        self.field(fields::unit::CREATED_BY_SPELL).filter(|id| *id != 0)
    }

    /// `UNIT_FIELD_PETNUMBER` — **the stable identity of a pet across being
    /// dismissed and called back**, and, more usefully here, the one field that
    /// says a unit is a pet at all.
    ///
    /// `HasPetUI` is three tests and this is the last of them:
    /// the unit exists, it is not a player, and this is non-zero. A guardian or
    /// a totem has no pet number and gets no pet panel, which is why the check
    /// is this rather than "the player has a summon".
    pub fn pet_number(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::PETNUMBER))
            .flatten()
            .unwrap_or(0)
    }

    /// `UNIT_FIELD_PET_NAME_TIMESTAMP` — when the pet was last named.
    ///
    /// The server writes `time(nullptr)` into it on a successful
    /// `CMSG_PET_RENAME` (vmangos `PetHandler.cpp`, `HandlePetRename`), and
    /// `SMSG_PET_NAME_QUERY_RESPONSE` echoes the same field — so a field value
    /// the name cache has not seen is the cue to ask again. Zero for a unit the
    /// field never arrived for.
    pub fn pet_name_timestamp(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::PET_NAME_TIMESTAMP))
            .flatten()
            .unwrap_or(0)
    }

    /// `UNIT_FIELD_TARGET` — who this unit is currently attacking or looking at.
    ///
    /// **This is the only statement the server ever makes about which way a
    /// creature in melee is turned.** `Unit::SetInFront` is `SetOrientation` and
    /// nothing else — no packet — and vmangos' `TotemAI` says why in a comment
    /// beside the call: *"client change orientation by self"*. So the facing
    /// rule lives here, and this field is its input; see
    /// [`ObjectManager::face_targets`].
    ///
    /// It is also **not always the unit's target**: a creature that is casting
    /// has `m_castingTargetGuid` substituted into this field on the way out
    /// (`Object::BuildValuesUpdate`), so what arrives is "the thing to look at"
    /// rather than "the thing being attacked". That is convenient here and a
    /// trap for anything that reads it as a threat table.
    ///
    /// Two `u32` fields, low word first, and **zero is a real answer**: the
    /// server writes `SetTargetGuid(ObjectGuid())` when a creature loses its
    /// victim and again on death, and a client that treats a missing half as
    /// "unchanged" leaves a corpse staring at whoever killed it.
    pub fn target_guid(&self) -> Option<u64> {
        if !self.is_unit_like() {
            return None;
        }
        let low = u64::from(self.field(fields::unit::TARGET)?);
        let high = u64::from(self.field(fields::unit::TARGET + 1).unwrap_or(0));
        let guid = (high << 32) | low;
        (guid != 0).then_some(guid)
    }

    /// **Swinging at the unit it is looking at** — the gate on the combat-ready
    /// stance, and a conjunction rather than [`Self::attacking`] alone.
    ///
    /// Two fields that agree most of the time and come apart in exactly one
    /// place a player notices: **clearing the target does not stop the swing.**
    /// `ClearTarget()` is `SetTarget(0)` and that function sends `CMSG_SET_SELECTION`, fires the target-changed event and
    /// contains no attack-stop of any kind; vmangos'
    /// `HandleSetSelectionOpcode` cancels only an *auto-repeat*, never the
    /// melee. So a character whose target is cleared mid-fight goes on hitting
    /// the mob — correctly — while standing in the guard with nothing selected,
    /// which is the report this closes.
    ///
    /// For the local player the second field *is* the selection, because
    /// vmangos' `SetSelectionGuid` is `SetTargetGuid` — so the guard drops a
    /// round trip after the click rather than on it, which the pose's own
    /// 150 ms cross-fade covers.
    ///
    /// **Which half is which.** The two readings above are measured; the
    /// conjunction is taken from the report rather than from the client's own
    /// stance chooser, whose gate is a client-side flag byte (`0xd58 & 0x60`) this project has not
    /// identified.
    pub fn engaged(&self) -> bool {
        self.attacking.is_some() && self.attacking == self.target_guid()
    }

    /// **The spells currently on this unit** — `UNIT_FIELD_AURA`'s 48 slots,
    /// with the empty ones dropped.
    ///
    /// The only thing in the protocol that says a buff is *active* rather than
    /// that one was cast. `SMSG_SPELL_GO` is a moment and a client that draws
    /// nothing else knows Ice Armor was cast but not that it is on — so a buff
    /// applied before the unit streamed into view is invisible, and one that
    /// expires or is dispelled is drawn for ever. This is the condition; see
    /// `SpellVisualKit`'s `stateKit`.
    ///
    /// **48 slots, and the count is pinned by the table rather than assumed.**
    /// The three parallel blocks after it are packed one byte per aura —
    /// `UNIT_FIELD_AURAFLAGS` at 95 is 12 `u32`s of flags, so the array before
    /// it spans 95 − 47 = 48 indices, one `u32` spell id each. A slot the server
    /// has never sent is absent from the field map, which is not the same as
    /// zero and is why this filters rather than indexing blindly: vmangos omits
    /// a field whose value is zero, the same rule that once drew a player
    /// magenta.
    pub fn auras(&self) -> Vec<u32> {
        self.aura_slots().into_iter().map(|aura| aura.spell).collect()
    }

    /// …and the same slots with everything the wire says **about** each one:
    /// which slot it is in, its flags, the caster's level and the stack count.
    ///
    /// [`Self::auras`] is the renderer's question ("what art is this unit
    /// wearing"); this is the interface's, and it needs three things that one
    /// throws away:
    ///
    /// * **the slot number**, because that is the only thing that says whether
    ///   an aura is a *buff* or a *debuff*. Nothing in the aura blocks is a
    ///   sign bit: the server places a positive aura in the first
    ///   [`POSITIVE_AURA_SLOTS`] and a negative one above them
    ///   (`SpellAuraHolder::_AddSpellAuraHolder`, two loops with those bounds),
    ///   and `UnitBuff`/`UnitDebuff` are the two halves of that split.
    /// * **the flags**, whose one bit this client reads is
    ///   [`aura_flags::CANCELABLE`] — `GetPlayerBuff`'s `CANCELABLE` /
    ///   `NOT_CANCELABLE` filter, and what decides whether right-clicking an
    ///   icon does anything.
    /// * **the applications**, which is the number drawn on a stacking debuff.
    ///   The field holds **count − 1** (`UpdateAuraApplication` says so in its
    ///   own comment), so a lone aura is stored as 0 and this adds the one
    ///   back — the interface compares `count > 1`.
    ///
    /// The three parallel blocks are packed differently from each other and the
    /// packing is the part worth pinning rather than assuming: the *levels* and
    /// the *applications* are one byte per slot (12 `u32`s each, `slot / 4` and
    /// `(slot % 4) * 8`), and the **flags are one nibble** (6 `u32`s,
    /// `slot >> 3` and `(slot & 7) << 2`). The field table agrees from the other
    /// end — 95 − 47 = 48 aura slots, 101 − 95 = 6 flag words, 113 − 101 = 12
    /// level words.
    pub fn aura_slots(&self) -> Vec<AuraSlot> {
        if !self.is_unit_like() {
            return Vec::new();
        }
        (0..MAX_AURA_SLOTS)
            .filter_map(|slot| {
                let spell = self.field(fields::unit::AURA + u16::from(slot))?;
                if spell == 0 {
                    return None;
                }
                let byte = |base: u16| {
                    let word = self.field(base + u16::from(slot) / 4).unwrap_or(0);
                    ((word >> ((u32::from(slot) % 4) * 8)) & 0xFF) as u8
                };
                let flags = {
                    let word = self.field(fields::unit::AURAFLAGS + u16::from(slot) / 8).unwrap_or(0);
                    ((word >> ((u32::from(slot) % 8) * 4)) & 0x0F) as u8
                };
                Some(AuraSlot {
                    slot,
                    spell,
                    flags,
                    level: byte(fields::unit::AURALEVELS),
                    // Stored as count − 1, and an absent word is a lone aura.
                    applications: byte(fields::unit::AURAAPPLICATIONS).saturating_add(1),
                })
            })
            .collect()
    }

    /// `UNIT_FIELD_FLAGS` whole, which is what decides whether a unit can be
    /// **selected or attacked at all**.
    ///
    /// Five of its bits disqualify a unit whatever its faction says — a quest
    /// giver, a flight master mid-flight, a creature still spawning — and the
    /// list is `vale_assets::tables::faction::UNATTACKABLE_FLAGS`, because which bits
    /// those are is a game rule rather than a packet layout. Zero for anything
    /// whose field has not arrived, which reads as "nothing forbidden".
    pub fn unit_flags(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::FLAGS))
            .flatten()
            .unwrap_or(0)
    }

    /// `UNIT_NPC_FLAGS` — **what this unit is *for***, and the whole of what the
    /// pointer says about it.
    ///
    /// Sixteen bits of service — gossip, quests, a vendor, a trainer, a flight
    /// master, an innkeeper, a banker, an auctioneer — and the client reads them
    /// for the *cursor* long before it reads them for a panel —
    /// `vale_assets::look::cursor::over_npc` is the measured chain.
    ///
    /// Zero for a unit that is nobody's business, which is every creature in the
    /// world, so an absent field and a plain mob are the same answer.
    pub fn npc_flags(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::NPC_FLAGS))
            .flatten()
            .unwrap_or(0)
    }

    /// **The quest log**, twenty slots of three update fields.
    ///
    /// `PLAYER_QUEST_LOG_1_1` is field 198 and each slot is `(id, packed,
    /// timer)`; see [`crate::play::quest::QuestSlot::from_fields`], which is where the
    /// packed word's four **six-bit** counters are decoded and where the reason
    /// that is not a byte read lives.
    ///
    /// **Sparse and kept sparse.** A quest is abandoned by writing a zero into
    /// its slot, so the twenty are not a list with a length — slot 4 can be
    /// empty while slot 5 holds something. The interface counts *entries*
    /// (`GetNumQuestLogEntries`), so the compaction happens exactly once, here,
    /// rather than at each of the eight reads that would otherwise each have to
    /// know it.
    pub fn quest_log(&self) -> Vec<crate::play::quest::QuestSlot> {
        let base = fields::player::QUEST_LOG_1_1;
        (0..crate::play::quest::MAX_QUESTS)
            .filter_map(|slot| {
                let at = base + (slot * crate::play::quest::SLOT_FIELDS) as u16;
                let id = self.field(at)?;
                (id != 0).then(|| {
                    crate::play::quest::QuestSlot::from_fields(
                        id,
                        self.field(at + 1).unwrap_or(0),
                        self.field(at + 2).unwrap_or(0),
                    )
                })
            })
            .collect()
    }

    /// **`UNIT_DYNAMIC_FLAGS` bit 0 — there is something on this body.**
    ///
    /// The one field that says a corpse is worth right-clicking, and it is not
    /// `UNIT_FIELD_FLAGS`: vmangos' `UNIT_DYNFLAG_LOOTABLE` is `0x0001` of the
    /// *dynamic* word, set by `Creature::PrepareBodyLootState` when the loot is
    /// generated and cleared the moment it is empty. Nothing else on the wire
    /// distinguishes a looted corpse from a full one.
    ///
    /// **It is per-viewer**, which is the part that matters: the field is in the
    /// `UF_FLAG_DYNAMIC` group, so the server sends each player their own value
    /// and a corpse another group member has already stripped reads zero here
    /// while still reading one for them. So this is the honest answer to "would
    /// a right-click do anything", which is exactly what the pointer needs.
    ///
    /// Zero for anything with no dynamic field at all, which is every player.
    pub fn lootable(&self) -> bool {
        const UNIT_DYNFLAG_LOOTABLE: u32 = 0x0000_0001;
        self.is_unit_like()
            .then(|| self.field(fields::unit::DYNAMIC_FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_DYNFLAG_LOOTABLE != 0)
    }

    /// **`UNIT_DYNAMIC_FLAGS` bit 1 — a hunter's mark.**
    ///
    /// `SPELL_AURA_MOD_STALKED` sets `UNIT_DYNFLAG_TRACK_UNIT` (`0x0002`), and
    /// the client's minimap test shows any unit carrying it as a
    /// tracked dot whatever the character is tracking. See
    /// `vale_assets::look::blips::tracked_unit`.
    pub fn hunters_marked(&self) -> bool {
        const UNIT_DYNFLAG_TRACK_UNIT: u32 = 0x0000_0002;
        self.is_unit_like()
            .then(|| self.field(fields::unit::DYNAMIC_FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_DYNFLAG_TRACK_UNIT != 0)
    }

    /// `UNIT_FIELD_CHARMEDBY`, as [`Self::summoned_by`] reads its field — the
    /// guid a mind-controlled creature answers to, and the one the minimap's
    /// classifier tests *before* the summoner's.
    pub fn charmed_by(&self) -> Option<u64> {
        if !self.is_unit_like() {
            return None;
        }
        let low = u64::from(self.field(fields::unit::CHARMEDBY)?);
        let high = u64::from(self.field(fields::unit::CHARMEDBY + 1).unwrap_or(0));
        let guid = (high << 32) | low;
        (guid != 0).then_some(guid)
    }

    /// **What the character is tracking** — `PLAYER_TRACK_CREATURES`,
    /// `PLAYER_TRACK_RESOURCES` and the stealth-tracking bit of
    /// `PLAYER_FIELD_BYTES`, all `PRIVATE` fields that only ever arrive for our
    /// own character. `None` for anybody else.
    ///
    /// The three are read together because the one consumer
    /// (`vale_assets::look::blips`) reads them together, at the client's own
    /// offsets: `+0xe50` off the player-field base, `+0xe54`, and `+0x1028`
    /// (byte 0, bit 1).
    pub fn tracking(&self) -> Option<(u32, u32, bool)> {
        const PLAYER_FIELD_BYTE_TRACK_STEALTHED: u32 = 0x02;
        if !self.is_self || self.object_type != Some(ObjectType::Player) {
            return None;
        }
        Some((
            self.field(fields::player::TRACK_CREATURES).unwrap_or(0),
            self.field(fields::player::TRACK_RESOURCES).unwrap_or(0),
            self.player_field_flags() & PLAYER_FIELD_BYTE_TRACK_STEALTHED != 0,
        ))
    }

    /// **`UNIT_DYNAMIC_FLAGS` bit 5 — this body is playing dead.**
    ///
    /// Feign Death, and it is the whole of what crosses the wire about it.
    /// `Unit::SetFeignDeath` sets `UNIT_DYNFLAG_DEAD` (`0x0020`) on apply and
    /// clears it on expiry, and nothing else moves: the health stays where it
    /// was, no flag on `UNIT_FIELD_FLAGS` changes, and there is no opcode for
    /// it. `SMSG_FEIGN_DEATH_RESISTED` is the *failure* and says nothing about
    /// a success.
    ///
    /// **The client reads it in the same breath as the health**: its test takes
    /// the unit's descriptor, answers true if the health is zero, then true
    /// again if this bit is set, then true for an `OBJECT_TYPE` of 7 (a corpse).
    /// Seven display paths read it — the pose cascade and the display blend
    /// among them — all of them as bit 5 of unit field 143.
    ///
    /// So it is a *drawing* fact rather than a state: the client draws a
    /// feigning hunter exactly as it draws a corpse, and everything else about
    /// them — that they are alive, that they may be healed, that no release box
    /// is owed — stays what the health says. That is why this is its own
    /// accessor rather than an `||` inside [`Self::is_dead`], whose readers
    /// include the death machinery.
    pub fn is_feigning(&self) -> bool {
        const UNIT_DYNFLAG_DEAD: u32 = 0x0000_0020;
        self.is_unit_like()
            .then(|| self.field(fields::unit::DYNAMIC_FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_DYNFLAG_DEAD != 0)
    }

    /// `UNIT_FLAG_IN_COMBAT` — the unit has a fight on.
    ///
    /// The client is never told "swing now" for anything but a landed blow, so
    /// this is what says a creature should be standing in its combat-ready pose
    /// between swings rather than idling at a target it is trying to kill.
    pub fn in_combat(&self) -> bool {
        const UNIT_FLAG_IN_COMBAT: u32 = 0x0008_0000;
        self.is_unit_like()
            .then(|| self.field(fields::unit::FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_FLAG_IN_COMBAT != 0)
    }

    /// `UNIT_FLAG_STUNNED` — **the unit may not turn**, which is a different
    /// statement from `MOVEFLAG_ROOT`'s "may not travel".
    ///
    /// The two arrive together for a stun and separately for everything else,
    /// and the client obeys them in two different places. Its input tick
    /// computes two independent booleans off the active mover before it does
    /// anything with the keys:
    ///
    /// * *can move* — health > 0, stand state not 7 (`DEAD`), and movement
    ///   flags `& 0x1200` clear (`0x1000` is `MOVEFLAG_ROOT`). This gates the
    ///   strides.
    /// * *can turn* — health > 0 and bit 18 of `UNIT_FIELD_FLAGS` clear, which
    ///   is `0x40000` = `UNIT_FLAG_STUNNED`. This gates the turn.
    ///
    /// The handler that runs when `UNIT_FIELD_FLAGS` changes re-runs the input
    /// tick precisely when bit 0x40000 is among the bits that moved.
    ///
    /// Server side it is `HandleAuraModStun` and nothing else
    /// (`SpellAuras.cpp`): a root aura calls `SetRooted(true)` alone, a stun
    /// sets this flag as well. vmangos' own comment on the constant says "Turn
    /// and strafe movement disabled"; the client puts the *strafe* under
    /// `canMove` with the root, so only the turn belongs here.
    pub fn is_stunned(&self) -> bool {
        self.is_unit_like()
            .then(|| self.field(fields::unit::FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_FLAG_STUNNED != 0)
    }

    /// **Is the server flying this character on a flight path?** —
    /// `UNIT_FIELD_FLAGS & UNIT_FLAG_TAXI_FLIGHT` (0x00100000), out of the same
    /// field one bit-shift along.
    ///
    /// **The one piece of taxi state that outlives a session**, which is the
    /// whole reason it is read here rather than inferred from the ride: a
    /// character who logged out mid-flight logs back in wearing it, and no
    /// window, no press and no packet of ours knows anything about that flight.
    ///
    /// Exclusively a taxi, so obeying it cannot lock a character out of walking
    /// for any other reason: vmangos sets and clears it in exactly two places,
    /// `FlightPathMovementGenerator`'s own `Initialize` and `Finalize`
    /// (`WaypointMovementGenerator.cpp` 396 and 363), and both times it is
    /// paired with `UNIT_FLAG_REMOVE_CLIENT_CONTROL` — whose own comment in
    /// `UnitDefines.h` says it is there to *"disable player movement"*.
    pub fn is_on_taxi(&self) -> bool {
        self.is_unit_like()
            .then(|| self.field(fields::unit::FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_FLAG_TAXI_FLIGHT != 0)
    }

    /// How a *player* looks: race, gender, skin tone, face, hair and beard.
    ///
    /// A player has no baked body texture — display ids 49..57 are the bare race
    /// models and `CreatureDisplayInfoExtra` has no row for them — so a client
    /// that only knows how to look one up draws every player magenta. These are
    /// the seven numbers the real client composes the skin from instead; see
    /// `vale_assets::look::character`.
    ///
    /// **Race and gender come out of the *unit* block, the rest out of the
    /// player block**, and vmangos packs each as bytes of one `u32` with
    /// `SetByteValue(field, offset)` — which indexes the value's bytes, so
    /// offset 0 is the low byte. Returns `None` for anything that is not a
    /// player, because the player block does not exist on a creature and index
    /// 187 there means something else entirely.
    ///
    /// **An absent field means zero, and here that is a real appearance.**
    /// `Object::_SetCreateBits` sets a bit only for `m_uint32Values[index] != 0`
    /// (`Object.cpp:1131`), so a create block simply omits every field that is
    /// zero — and `PLAYER_BYTES` all-zero is skin 0, face 0, hair style 0,
    /// hair colour 0, which is the first option on the character-creation
    /// screen and therefore extremely common. Requiring the field instead makes
    /// exactly those characters fall back to no skin at all, which draws
    /// magenta: the appearance the server was most confident about is the one
    /// that goes missing. `UNIT_FIELD_BYTES_0` is required because race 0 is not
    /// a player race, so its absence really would mean the block never arrived.
    pub fn appearance(&self) -> Option<[u32; 3]> {
        if self.object_type != Some(ObjectType::Player) {
            return None;
        }
        Some([
            self.field(fields::unit::BYTES_0)?,
            self.field(fields::player::BYTES).unwrap_or(0),
            self.field(fields::player::BYTES_2).unwrap_or(0),
        ])
    }

    /// The item **entries** in the nineteen equipment slots, zero where empty.
    ///
    /// `Player::SetVisibleItemSlot` writes `pItem->GetEntry()` to
    /// `PLAYER_VISIBLE_ITEM_1_0 + slot * MAX_VISIBLE_ITEM_OFFSET`, and the
    /// offset is 12 — the block is a creator GUID, the entry, seven
    /// enchantments, a properties pair and a pad. An entry is not a display id:
    /// what a piece of armour looks like is in the server's `item_template`,
    /// because `Item.dbc` is not in the 1.12 archives at all, so each distinct
    /// entry costs a `CMSG_ITEM_QUERY_SINGLE`.
    ///
    /// `None` for anything that is not a player, for the same reason
    /// [`Self::appearance`] is: the player block does not exist on a creature
    /// and those indices mean something else there. **An NPC's equipment is not
    /// here either** — it is baked into its `CreatureDisplayInfoExtra` texture,
    /// which is why an NPC in plate needs no query at all.
    pub fn equipment(&self) -> Option<[u32; EQUIPMENT_SLOTS]> {
        if self.object_type != Some(ObjectType::Player) {
            return None;
        }
        let mut out = [0u32; EQUIPMENT_SLOTS];
        for (slot, entry) in out.iter_mut().enumerate() {
            let index = fields::player::VISIBLE_ITEM_1_0 + (slot as u16) * VISIBLE_ITEM_STRIDE;
            *entry = self.field(index).unwrap_or(0);
        }
        Some(out)
    }

    /// Model id. Lives at a different index for units and game objects, which
    /// is exactly the overlap trap the per-type modules exist to prevent.
    pub fn display_id(&self) -> Option<u32> {
        match self.object_type {
            Some(ObjectType::Unit) | Some(ObjectType::Player) => {
                self.field(fields::unit::DISPLAYID)
            }
            Some(ObjectType::GameObject) => self.field(fields::game_object::DISPLAYID),
            _ => None,
        }
    }

    /// **The spell whose persistent area this object is, and how wide it is.**
    ///
    /// A `DynamicObject` is what the server puts in the world for a Blizzard, a
    /// Flamestrike, a Rain of Fire or a Consecration: it has no display id and
    /// no model of its own, and these two fields are the whole of what it says
    /// about itself. What it *looks* like is the spell's own art, one archive
    /// lookup away (`vale_assets::tables::spell::SpellVisuals::ground_art`) — so a
    /// client that reads neither field draws nothing where the ground effect
    /// should be, which is what this one did.
    ///
    /// `None` for every other object type, for the reason [`Self::display_id`]
    /// is per type: the index tables overlap and 9 means something else in each
    /// of them.
    ///
    /// **The radius may be zero and the object is still one.** This used to
    /// require a positive radius, on the grounds that a zero would collapse the
    /// art it scales — and the art is not scaled by it. `world::entities::effects`
    /// says so in its own first paragraph (the client takes the scale branch
    /// only for a `DYNAMICOBJECT_BYTES` that is neither 1 nor 2, and vmangos writes 1 for every area aura), so
    /// the radius drives the *impact distribution* and nothing else.
    ///
    /// What the filter cost is the whole of a far-sight spell's visible effect.
    /// `Player::SetLongSight` creates its `DynamicObject` with a radius of
    /// literally zero (`DynamicObject::Create(..., 0, 0, DYNAMIC_OBJECT_FARSIGHT_FOCUS)`),
    /// and Eagle Eye's `SpellVisual` names `Spells\FarSight_Impact_Base.m2` —
    /// so the object arrived, was placed, resolved to art, and was refused one
    /// line before it drew.
    pub fn persistent_area(&self) -> Option<(u32, f32)> {
        if self.object_type != Some(ObjectType::DynamicObject) {
            return None;
        }
        let spell = self.field(fields::dynamic_object::SPELLID)?;
        let radius = self.field_f32(fields::dynamic_object::RADIUS).unwrap_or(0.0);
        Some((spell, radius))
    }

    pub fn is_alive(&self) -> bool {
        self.health().is_some_and(|h| h > 0)
    }

    /// Whether [`Self::health`] is a percentage rather than hit points.
    ///
    /// A heuristic, and necessarily so: the wire format is identical either
    /// way, and the client is not told which mode the server is in. Max health
    /// of exactly 100 is the tell, since that is the constant vmangos
    /// substitutes when hiding real values.
    pub fn health_is_percentage(&self) -> bool {
        self.max_health() == Some(100)
    }

    /// Is the server currently moving this entity?
    ///
    /// The two sources are the same two [`ObjectManager::advance`] dead-reckons
    /// from: a spline the server put the unit on, or the movement flags of its
    /// last `MSG_MOVE_*` broadcast. Nothing else moves an entity, so this is
    /// exactly "is its position changing", stated rather than inferred.
    ///
    /// A renderer wants this rather than the difference between two positions it
    /// is interpolating. That difference reads zero for the last frames of every
    /// interpolation window — the window ends when the next snapshot is *due*,
    /// not when it arrives — and an animation chosen from it therefore restarts
    /// several times a second. **Not** true for the player: the session's
    /// [`crate::state::movement::Mover`] owns that one, and reports it separately.
    pub fn is_moving(&self) -> bool {
        self.spline.is_some() || self.effective_movement().is_some_and(|m| m.is_moving())
    }

    /// Is this unit off the ground — and did it *jump*?
    ///
    /// A server-driven unit on a spline is never airborne as far as this client
    /// is concerned: `SMSG_MONSTER_MOVE` says where a creature will be and never
    /// why, so a leaping creature is walking a path through the air and there is
    /// nothing to distinguish. This is about the other players, whose own
    /// `MSG_MOVE_JUMP` broadcasts say both outright.
    ///
    /// **The upward speed is what separates the two arcs.** The wire is
    /// down-positive, so a jump reports `jump.zspeed` of *minus*
    /// [`crate::state::movement::JUMP_SPEED`]; a step off a ledge reports zero, and a
    /// knockback whatever it was thrown at.
    pub fn is_airborne(&self) -> bool {
        self.spline.is_none()
            && self
                .effective_movement()
                .is_some_and(|m| m.has(move_flags::JUMPING))
    }

    pub fn is_jumping(&self) -> bool {
        self.is_airborne() && self.effective_movement().is_some_and(|m| m.jump.z_speed < 0.0)
    }

    /// How fast, in yards per second, the client is moving this entity — zero
    /// when it is not moving at all, which is what chooses Stand over Walk.
    ///
    /// The same number the dead reckoning uses, so the animation and the drawn
    /// position cannot disagree about whether something is running.
    pub fn ground_speed(&self) -> f32 {
        if let Some(spline) = self.spline.as_ref() {
            return spline.speed();
        }
        match self.effective_movement() {
            Some(info) if info.is_moving() => info.speed(&self.speeds.unwrap_or_default()),
            _ => 0.0,
        }
    }

    /// The movement flags the gait is chosen from — the same block
    /// [`Self::ground_speed`] and [`Self::is_moving`] read, so the gait, the
    /// speed and the direction are one answer taken three ways and cannot
    /// disagree.
    ///
    /// **The flags rather than a direction**, because the client's own gait
    /// rules are written on them and the two surfaces do not agree about
    /// precedence: on the ground backward beats a strafe, in the water a strafe
    /// beats backward and a turn beats both. A four-way direction enum cannot
    /// say that, and the one this replaced said the ground's order in both
    /// places.
    ///
    /// **A unit on a spline reads as `FORWARD`**: `SMSG_MONSTER_MOVE` states a
    /// path and no flags at all, and a creature walks its patrol facing the way
    /// it goes. Without the substitution a patrolling creature has no direction
    /// bit set and is drawn standing while it slides along its route.
    /// **…and a *flying* spline reads as `FLYING` too**, which is the same
    /// substitution one flag along: `MoveSplineFlag::Flying` is what the
    /// server says about a taxi flight and a flying creature's path, and it is
    /// what the renderer picks the clip from. Without it every flying thing in
    /// the world runs on air at whatever its spline's speed divided by `Run`'s
    /// authored 6.9 comes to.
    ///
    /// **…and the server's separate statements outrank both**, which is what
    /// [`Self::forced_flags`] is for: `SMSG_SPLINE_MOVE_ROOT` about a creature
    /// arrives with no block at all, and the spline substitution above would
    /// otherwise go on reporting `FORWARD` for a unit the server has just
    /// nailed to the floor.
    pub fn move_flags(&self) -> u32 {
        let base = if let Some(spline) = self.spline.as_ref() {
            let mut flags = crate::state::movement::move_flags::FORWARD;
            if spline.flying {
                flags |= crate::state::movement::move_flags::FLYING;
            }
            flags
        } else {
            self.movement.map(|m| m.flags).unwrap_or(0)
        };
        self.forced_flags.over(base)
    }

    /// **Which moving platform this unit is riding**, or `None` for one on the
    /// ground.
    ///
    /// `MOVEFLAG_ONTRANSPORT`'s guid, straight off the movement block — the
    /// server writes both halves for every passenger it broadcasts, and a
    /// passenger's `t_pos` is the position as far as `HandleMoverRelocation`
    /// is concerned. See [`crate::state::movement::Ferry`].
    ///
    /// **Not the local player's**, whose movement never comes back from the
    /// server: that one is `SessionStatus::ferry`, and the two are joined by
    /// the caller.
    ///
    /// The flag is tested rather than trusting the block, because
    /// [`crate::state::movement::MovementInfo::transport`] is only filled in
    /// under the flag and a stale one would name a deck the unit has left.
    pub fn platform_guid(&self) -> Option<u64> {
        let info = self.movement?;
        if info.flags & crate::state::movement::move_flags::ONTRANSPORT == 0 {
            return None;
        }
        info.transport.map(|t| t.guid)
    }

    /// [`Self::movement`] with [`Self::forced_flags`] applied — the block the
    /// dead reckoning should actually run on.
    ///
    /// **The one accessor, so that the flags a unit is drawn by and the flags it
    /// is moved by cannot disagree.** They did once, for the whole life of the
    /// stand-in that preceded this: `move_flags` was patched and `advance` read
    /// the raw field, so a rooted player was drawn standing and walked anyway.
    pub fn effective_movement(&self) -> Option<MovementInfo> {
        let mut info = self.movement?;
        if !self.forced_flags.is_empty() {
            info.flags = self.forced_flags.over(info.flags);
        }
        Some(info)
    }

    /// Is this unit in water deep enough to swim in?
    ///
    /// `MOVEFLAG_SWIMMING`, which the server sets on a creature it is moving
    /// through water and which a *player's* own client sets and broadcasts. It
    /// is a flag rather than a depth, so this is the server's answer rather than
    /// a guess from the liquid surface — and it is the same field the gait comes
    /// from, so the two cannot disagree about what a unit is doing.
    ///
    /// A unit on a spline has no movement block of its own, and the answer is
    /// then false: `SMSG_MONSTER_MOVE` carries no flags. That is a real gap and
    /// it costs a swimming *creature* its stroke, not a swimming player.
    ///
    /// It is asked of [`Self::effective_movement`] rather than of the raw block,
    /// so the two packets that would answer it for a creature —
    /// `SMSG_SPLINE_MOVE_START_SWIM` and `_STOP_SWIM` — reach it if they are
    /// ever sent. **On vmangos they are not**, by any code path, which is why
    /// the gap above is still a gap. Deliberately *not* asked of
    /// [`Self::move_flags`]: that one substitutes a spline's own flags for the
    /// block's, which would take the stroke off a swimmer the moment the server
    /// put them on a path.
    pub fn is_swimming(&self) -> bool {
        self.effective_movement()
            .is_some_and(|m| m.flags & crate::state::movement::move_flags::SWIMMING != 0)
    }

    /// **Which way the body is pointed in the vertical**, radians, up-positive.
    ///
    /// The movement block's `pitch`, which the wire carries **only** under
    /// `MOVEFLAG_SWIMMING` — so it is zero for everything that is not in the
    /// water, and reading it unconditionally would tilt every creature in the
    /// world by whatever happened to be in the field. Gated here rather than at
    /// the call site so that there is one copy of the rule.
    pub fn pitch(&self) -> f32 {
        match self.movement {
            Some(m) if m.flags & crate::state::movement::move_flags::SWIMMING != 0 => m.pitch,
            _ => 0.0,
        }
    }

    /// Health rendered for display, disambiguating percent from hit points.
    pub fn health_label(&self) -> Option<String> {
        let hp = self.health()?;
        let max = self.max_health()?;
        Some(if self.health_is_percentage() {
            format!("{hp}%")
        } else {
            format!("{hp}/{max} hp")
        })
    }
}

/// **Templates answered since the last look** — the one *edge* this manager
/// carries, as against the states everything else in it is.
///
/// A template arriving moves no field a reader polls, so a panel that read a
/// name once and got nothing has no way to learn it could ask again. That is
/// the whole of "an item seen for the first time has no name", and the same
/// sentence covers a quest objective naming a creature that has never been in
/// view. See `crate::game::templates` on the client side, which is where the
/// reference's own callback mechanism is written up.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Arrivals {
    pub items: Vec<u32>,
    pub creatures: Vec<u32>,
    pub gameobjects: Vec<u32>,
}

impl Arrivals {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.creatures.is_empty() && self.gameobjects.is_empty()
    }

    /// Whether anything a *quest log row* is composed from just landed.
    pub fn names_a_quest_objective(&self) -> bool {
        !self.creatures.is_empty() || !self.gameobjects.is_empty() || !self.items.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct ObjectManager {
    entities: HashMap<u64, Entity>,
    /// Which simulation step the standing pass is on — see [`STAND_BEAT`].
    stand_beat: u8,
    /// GUID of the object flagged `UPDATEFLAG_SELF` — the player.
    pub player_guid: Option<u64>,
    /// Creature templates resolved by `CMSG_CREATURE_QUERY`, keyed by entry.
    /// Cached because entries repeat heavily — every wolf in a zone shares one.
    pub creatures: HashMap<u32, CreatureInfo>,
    /// The same, for `CMSG_GAMEOBJECT_QUERY`.
    pub gameobjects: HashMap<u32, GameObjectInfo>,
    /// Players resolved by `CMSG_NAME_QUERY`, keyed by **GUID** rather than
    /// entry: a player has no template to share, so there is nothing to
    /// deduplicate against.
    pub players: HashMap<u64, PlayerInfo>,
    /// **Everybody the last `SMSG_GROUP_LIST` named**, whether or not they are
    /// in the world.
    ///
    /// The one population [`Self::unresolved_player_guids`] cannot find by
    /// walking the entities: a raid member across the continent has no entity at
    /// all, and the roster packet carries a name and a status byte but **no
    /// class** — which is what a raid button's class label and its
    /// `RAID_CLASS_COLORS` colour are made of. The reference reads both out of
    /// its own name cache for exactly this reason (the lookup at the top of
    /// `GetRaidRosterInfo`), so a group member is asked about by name
    /// query like anybody else.
    pub group_guids: Vec<u64>,
    /// **…and everybody on the friends and ignore lists**, on exactly the same
    /// terms and for the same reason.
    ///
    /// Neither `SMSG_FRIEND_LIST` nor `SMSG_IGNORE_LIST` carries a name — see
    /// [`crate::play::social`] — and a friend is usually on another continent, so
    /// this is the second population the entity walk cannot find.
    ///
    /// **Two lists rather than one**, because the two packets arrive separately
    /// and each replaces its own: merged into a single vector, an ignore list
    /// landing after a friends list would take the friends off it. A guid on
    /// both is asked about once anyway — the query pass filters on the name
    /// cache, not on this.
    pub friend_guids: Vec<u64>,
    pub ignore_guids: Vec<u64>,
    /// …and the ones a *change* named, which neither list has been re-sent for.
    ///
    /// `SMSG_FRIEND_STATUS` is the only announcement an add makes and the server
    /// never repeats the list, so without this a friend added mid-session has no
    /// name until the next login.
    pub wanted_social: HashSet<u64>,
    /// Item templates resolved by `CMSG_ITEM_QUERY_SINGLE`, keyed by entry —
    /// which deduplicates hard, since a city full of guards is one breastplate.
    pub items: HashMap<u32, ItemInfo>,
    /// Entries the server refused to describe, so they are asked for once and
    /// not once a frame. vmangos answers `entry | 0x80000000` for an item it
    /// will not talk about, and that is a real answer rather than a lost packet.
    pub unknown_items: HashSet<u32>,
    /// **Item entries something outside the world asked to have named.**
    ///
    /// Everything else in this manager is a fact the server volunteered; this
    /// is the one thing the *interface* puts in. A spell's reagents are item
    /// entries — `Spell.dbc` holds 17031 and not "Rune of Teleportation", and
    /// `Item.dbc` is not in the archives — so a tooltip that wants to name one
    /// has to get in the same query queue a worn breastplate does. See
    /// [`Self::want_item`].
    pub wanted_items: HashSet<u32>,
    /// **Something arrived that may need a name**, cleared by the session's own
    /// query pass — see [`Self::take_query_hint`], which is where the whole of
    /// the reason lives.
    queries_wanted: bool,
    /// …and the same two for the other tables a *name* can come out of. Only
    /// the quest log fills these: an objective is `ReqCreatureOrGOId` plus a
    /// count, and the words "Kobold Vermin" live nowhere but
    /// `CMSG_CREATURE_QUERY`'s answer. See [`Self::want_creature`].
    pub wanted_creatures: HashSet<u32>,
    pub wanted_gameobjects: HashSet<u32>,
    /// **Query answers learned this session that are not on disk yet**, as the
    /// response bodies they arrived as — drained by the session loop into
    /// [`crate::play::wdb`].
    ///
    /// The bodies rather than the parsed records, because the cache replays
    /// them through the one parser; see that module, which is also where the
    /// reason a persistent cache is the only fix for a blank loot row is.
    ///
    /// Off unless somebody is draining it. A snapshot pump has no cache to
    /// write to and would otherwise accumulate a copy of every answer it ever
    /// received.
    pub record_cache: bool,
    pub learned: Vec<Learned>,
    /// Every `(kind, key)` on disk at start-up or recorded this session, so
    /// an answer the server restates is not written twice. See
    /// [`Self::remember`].
    cache_known: HashSet<(Kind, u64)>,
    /// The bodies of the on-demand kinds — [`Kind::on_demand`] — which the
    /// session loop answers a query out of instead of sending it. See
    /// [`Self::cached_answer`].
    on_demand: HashMap<(Kind, u64), Vec<u8>>,
    /// **Entries whose template answer landed since the last look**, refusals
    /// included — drained by [`Self::take_item_arrivals`].
    ///
    /// This is the edge nothing else in this manager carries. Every other
    /// population here is a *state* a reader compares against; a template
    /// arriving changes no field anybody polls, so a panel that read an item's
    /// name once and got nothing has no way to learn it could ask again. That
    /// is the whole of "the first item of a kind has no name": the query goes
    /// out, the answer lands, and the window is still holding the blank.
    ///
    /// The client's own mechanism is a **completion callback** rather than a
    /// queue — its item-cache lookup returns the record on a hit and, on a miss, files the callback and queries; the
    /// callback re-raises the panel's own event so the read happens again. The
    /// queue is this client's shape for the same thing, because a handler here
    /// runs under the world lock and cannot raise a Bevy message.
    ///
    /// **A refusal is an arrival too.** Without it a panel waiting on an entry
    /// the server will never describe is never told to stop waiting.
    arrivals: Arrivals,
    /// **What is over each quest giver's head**, by guid — see
    /// [`Self::set_quest_status`], which is where the reason this is state
    /// rather than an edge is.
    quest_status: HashMap<u64, crate::play::quest::DialogStatus>,
    /// …and every guid that has been *asked* about, which is not the same set:
    /// a giver with nothing for this character answers `None` and is kept out
    /// of the map above, and without this the client would ask again every
    /// frame it saw one.
    asked_quest_status: HashSet<u64>,
    /// **…and whether each flight master's own node is one this character has
    /// stood at**, by guid — the other thing that puts a model over a head. See
    /// [`Self::set_taxi_status`].
    ///
    /// A separate map from the quest one rather than a shared "what mark is on
    /// this unit", because the two arrive on their own schedules and either can
    /// be answered while the other has not been asked. Which of them *wins* on
    /// the rare unit that is both is the renderer's, and the reference's answer
    /// is "whichever packet came last" — there is one slot at `unit+0xcb8`.
    taxi_status: HashMap<u64, bool>,
    /// …and every flight master already asked about, on exactly the terms
    /// [`Self::asked_quest_status`] is: a *known* node answers `true` and is
    /// kept, and without this the client would ask once a frame.
    asked_taxi_status: HashSet<u64>,
    /// How many positions were refused as impossible — see
    /// [`Entity::set_server_position`]. Should be zero; anything else says a
    /// packet is being read at the wrong offset, and the number is surfaced
    /// rather than silently swallowed because the symptom otherwise is an entity
    /// that quietly vanishes.
    pub rejected_positions: u32,
    /// How many server statements moved an entity more than [`JUMP_YARDS`], and
    /// a capped sample of them naming the packet that did it.
    ///
    /// **The point is attribution.** "Mobs teleport around" is a report that
    /// every plausible cause fits equally well — a mis-parsed spline, a dropped
    /// packet, an interpolation fault — and none of them can be ruled out by
    /// looking harder at the screen. A creature name, a distance and the opcode
    /// that carried it turns the question into one that can be answered.
    pub jump_count: u32,
    pub jump_log: Vec<String>,
    /// Chat that has arrived and not yet been shown, oldest first.
    ///
    /// **Drained by its reader, not cloned.** Every other thing in here is
    /// *state* — a position, a template, a count — and can be read again as often
    /// as anyone likes. A line of chat is an **event**: it has to be shown
    /// exactly once, so the reader takes it ([`Self::take_chat`]) and this is
    /// empty again. That is the same distinction `swings_thrown` makes by being a
    /// counter, arrived at from the other side.
    ///
    /// Bounded, because nothing guarantees a reader: `vale live` polls a few
    /// times a second and the renderer every frame, but a caller that only
    /// enters the world and waits would otherwise accumulate a city's worth of
    /// combat log for the life of the session.
    chat: Vec<crate::play::chat::ChatMessage>,
    /// The world's clock, as the server stated it at login.
    ///
    /// **State, and the only piece of it the server says exactly once.** It is
    /// held here rather than in the session loop because it is a property of the
    /// world and not of this connection to it — the same reason the creature
    /// templates are here — and because `SMSG_NEW_WORLD` does *not* restate it,
    /// so a teleport across a continent must not lose it. See
    /// [`crate::play::time`] for what the one packet carries, and
    /// [`crate::play::time::GameTime::advanced`] for who runs it afterwards.
    pub game_time: Option<crate::play::time::GameTime>,
    /// **What the sky is doing** — the last `SMSG_WEATHER`, kept as state
    /// for the reason [`Self::game_time`] is: the server states it on a zone
    /// change and then only when it rolls a new grade, which is minutes
    /// apart, and a reader that missed the one packet would draw a clear sky
    /// through a storm. `None` until the first one, which is a clear sky.
    pub weather: Option<crate::play::weather::Weather>,
    /// **Where the hearthstone returns the character to.**
    ///
    /// Here for the same reason the clock is: `SMSG_BINDPOINTUPDATE` arrives
    /// once in the login burst and again only when the home is changed, and
    /// nothing ever restates it — so a reader that missed it has a hearthstone
    /// tooltip with a hole in the sentence and no error anywhere. `None` before
    /// the burst has landed. See [`crate::play::bindpoint`].
    pub bind_point: Option<crate::play::bindpoint::BindPoint>,
    /// **What this character can do**: the spellbook the server stated at login
    /// and has amended since.
    ///
    /// State rather than an event, and held here for the same reason the clock
    /// is: it is said once, in the login burst, and nothing ever restates it —
    /// so a reader that missed it has an empty action bar and no error anywhere.
    /// See [`crate::play::spells::parse_initial_spells`].
    pub spellbook: crate::play::spells::Spellbook,
    /// The action bar the server remembers for this character, occupied slots
    /// only. Cosmetic state: the *bar* is the client's, and this is only where
    /// the last session left it.
    pub action_buttons: Vec<crate::play::spells::ActionButton>,
    /// Bumped whenever either of the two above changes, so a reader can tell a
    /// new spellbook from the same one without comparing the lists.
    pub spellbook_version: u32,
    /// **The pet's own bar**, as `SMSG_PET_SPELLS` last stated it — see
    /// [`crate::play::pet`].
    ///
    /// Empty until there is a pet, and emptied again by the eight-byte
    /// dismissal the server sends when one goes. It is held here rather than on
    /// the pet's [`Entity`] for a reason the wire forces: the packet arrives
    /// **before** the pet's create block on a summon and **after** the pet has
    /// left the world on a dismissal, so an entity to hang it on is exactly
    /// what is missing at both moments.
    pub pet: crate::play::pet::PetSpells,
    /// …and the name the player gave it, keyed the way the server keys it —
    /// `UNIT_FIELD_PETNUMBER` rather than a guid, which is what makes the name
    /// survive a dismissal. The second half is the response's copy of
    /// `UNIT_FIELD_PET_NAME_TIMESTAMP`: the field moving past it is the one
    /// statement on the wire that a rename took, so it is what
    /// [`Self::unresolved_pet_names`] re-asks on.
    pub pet_names: HashMap<u32, (String, u32)>,
    /// Pet numbers a name has been asked for, with the field timestamp the ask
    /// was made against — one query per number per timestamp, so a server that
    /// declines to answer is not asked again until the field moves.
    pub wanted_pet_names: HashMap<u32, u32>,
    /// The same latch [`Self::spellbook_version`] is: resolving the bar means a
    /// `Spell.dbc` lookup per slot, which is nothing once and unacceptable
    /// sixty times a second.
    pub pet_version: u32,
    /// **The pet was told to attack and has not been told to stop** — the
    /// reference's own flag, which `CastPetAction` sets when the
    /// attack command is pressed at a valid target and `IsPetAttackActive`
    /// reads. Cleared by `CMSG_PET_STOP_ATTACK` and by the next
    /// `SMSG_PET_SPELLS`. See [`Self::apply_pet_press`].
    pub pet_attacking: bool,
    /// **Bumped whenever anything the inventory is made of moves** — a create
    /// or values block for an `Item` or a `Container`, or one that touches the
    /// local player's three slot runs.
    ///
    /// The same latch [`Self::spellbook_version`] is, and for a sharper reason:
    /// [`crate::play::items::Inventory::read`] chases three levels of GUID and
    /// allocates, so a client that rebuilt per frame would pay for a bag it
    /// never opened sixty times a second. Nothing on the wire announces an
    /// inventory change, so this counter *is* the announcement — see
    /// [`crate::play::items`], whose module comment is the whole reason it can be
    /// computed at all.
    pub inventory_version: u32,
    /// Who the local player is auto-attacking, per `SMSG_ATTACKSTART` /
    /// `SMSG_ATTACKSTOP` about ourselves.
    ///
    /// **The server's opinion, not the client's request.** Pressing Attack sends
    /// a swing and the server decides whether it took; this is the answer, and
    /// it is what an Attack button should light up from. `None` before the
    /// player object exists, since the guid to compare against is not known yet.
    pub attacking: Option<u64>,
    /// What the server said about our own presses, oldest first — drained by
    /// whoever is showing the interface, exactly as [`Self::take_chat`] is.
    events: Vec<crate::play::spells::PlayerEvent>,
    /// …and what happened in the fight, oldest first — the combat log's own
    /// queue, drained by whoever is composing its lines. See
    /// [`Self::note_combat`] for why this is not the field above.
    combat: Vec<crate::play::combatlog::CombatEvent>,
    /// **How long our own buffs have left**, keyed by `UNIT_FIELD_AURA` slot.
    ///
    /// `SMSG_UPDATE_AURA_DURATION` is the only thing in the 1.12 protocol that
    /// says so, and it is sent **to the aura's target and only when that target
    /// is a player** (`SpellAuraHolder::UpdateAuraDuration`) — so this is our own
    /// buff bar's timers and there is nothing equivalent for anybody else. That
    /// is not a gap in this client: the game's own target frame and party frames
    /// draw no timers either, for exactly this reason.
    ///
    /// It is **state rather than an event**, unlike everything else the server
    /// says about our presses, because the interface asks "how long is left"
    /// once a frame and never "did a duration arrive". Slot-keyed because the
    /// packet carries a slot and no spell id at all.
    aura_durations: HashMap<u8, AuraDuration>,
}

/// One `SMSG_UPDATE_AURA_DURATION`, as [`ObjectManager`] keeps it.
#[derive(Debug, Clone, Copy)]
pub struct AuraDuration {
    /// How long the aura had left when the packet arrived. At an apply or a
    /// refresh this **is** the whole duration; the packet is also sent on a
    /// pushback, where it is not.
    pub remaining_ms: u32,
    /// When it arrived, so a reader can subtract. `Instant` rather than a
    /// session clock because this crate has no frame clock and the consumer's
    /// base (`GetTime()`) is not this one's — the conversion is one subtraction
    /// at the reader.
    pub received: Instant,
    /// Bumped once per packet for this slot, so a reader can tell a **refresh**
    /// from the same reading seen again. Without it a buff re-applied at its own
    /// full duration is indistinguishable from one that has been running.
    pub seq: u32,
}

/// How many jump samples to keep. Enough to see a pattern, few enough that a
/// pathological session does not grow a `Vec` without bound.
/// How long an arc may be extrapolated before the client gives up on it.
///
/// `Unit::ExtrapolateMovement` refuses past ten seconds for the same reason:
/// past that the landing packet has plainly been lost, and continuing to
/// integrate only drives the unit into the ground.
/// How far above what is under it a **standing** unit may be pulled down — see
/// [`ObjectManager::stand_on_the_ground`], which is where the whole argument is.
///
/// Two yards, and the number is a judgement rather than a measurement. It has
/// to clear the world database's worst ordinary spawn error — Eagan
/// Peltskinner's is 0.77 and the tail of the 14,193 stationary ground spawns on
/// maps 0 and 1 runs to about 1.9 before it stops being a graze and starts
/// being a storey — and it has to stay well under the height of anything a unit
/// could really be standing on that this client has no hull for. A dais, a
/// gangway or a flying creature is metres up, not one.
///
/// **The reference almost certainly has no such number**, because it does not
/// need one: it loads a hull for everything it draws, so its floor query always
/// has the real answer. This band is the price of a collision world with holes
/// in it, and the honest place to spend the next hour on this subject is
/// closing those holes rather than raising this.
const STAND_BAND: f32 = 2.0;

/// How many simulation steps apart the standing pass runs.
///
/// It exists for one population: the entities whose ground has not arrived yet.
/// Those cannot be latched (see [`ObjectManager::stand_on_the_ground`]), so
/// they are re-asked until the tile under them streams in — and at a login,
/// with every entity in view created before any terrain, that is the whole
/// crowd at once. Eight steps is five times a second, which settles a spawn
/// within a fifth of a second of its ground arriving and cannot be seen.
const STAND_BEAT: u8 = 8;

/// …and how far off the ground is worth moving at all. Below this the drop is
/// invisible and taking it only churns the position for anything watching it.
/// The database's own systematic lift is +0.18, so this deliberately sits under
/// that: the commonest spawn in the game is a fifth of a yard up and it is
/// worth putting down.
const STAND_EPSILON: f32 = 0.05;

const MAX_ARC_SECS: f32 = 10.0;

const JUMP_SAMPLES: usize = 16;

/// How many undrained chat lines to keep. A busy city's combat log is a few a
/// second, and a reader that has not looked in a hundred lines' time is not
/// going to be helped by the hundred and first.
const CHAT_BACKLOG: usize = 100;

/// The same bound for [`ObjectManager::note_event`]. Smaller, because these are
/// answers to presses: a player generates a handful a second at most, and a
/// reader that has not looked in thirty-two of them has stopped looking.
const EVENT_BACKLOG: usize = 32;

/// …and for [`ObjectManager::note_combat`], which is the largest of the three.
///
/// **The reason it is a queue of its own rather than more of
/// [`EVENT_BACKLOG`]**: a combat log line is about anybody in sight, not about
/// one of our own presses, so a five-man pull in a city produces them at tens a
/// second while the press queue produces one. Sharing would let a fight push
/// every answer to a press out of the queue before anything read it, which is
/// the sort of loss that looks like an unrelated bug.
const COMBAT_BACKLOG: usize = 200;

/// How many releases back [`Entity::recent_spells`] remembers.
///
/// A renderer's depth rather than anything the wire states: what it has to cover
/// is a spell that triggers another inside one poll, which is two. Four leaves
/// room for a chain and still costs sixteen bytes in a snapshot.
pub const RECENT_SPELLS: usize = 4;

/// How many `UNIT_FIELD_AURA` slots a unit has — 48, pinned by the table rather
/// than assumed: `UNIT_FIELD_AURAFLAGS` at 95 minus `UNIT_FIELD_AURA` at 47.
pub const MAX_AURA_SLOTS: u8 = (fields::unit::AURAFLAGS - fields::unit::AURA) as u8;

/// **…and how many of them are buffs.** The first 32 slots hold positive auras
/// and the 16 above them negative ones, and that split is the *whole* of how a
/// client tells a buff from a debuff — nothing in any of the four aura blocks is
/// a sign.
///
/// vmangos allocates by exactly these bounds
/// (`SpellAuraHolder::_AddSpellAuraHolder`: `for i in 0..MAX_POSITIVE_AURAS`
/// for a positive holder, `for i in MAX_POSITIVE_AURAS..MAX_AURAS` for a
/// negative one), and `MAX_POSITIVE_AURAS` is 32 in `SpellAuraDefines.h`.
pub const POSITIVE_AURA_SLOTS: u8 = 32;

/// `UNIT_FIELD_AURAFLAGS`' nibble — `AFLAG_*` in vmangos'
/// `SpellAuraDefines.h`, of which this client reads one bit.
pub mod aura_flags {
    /// `AFLAG_CANCELABLE` — right-clicking the icon will be honoured.
    ///
    /// The server sets it for a **positive** aura that is not
    /// `SPELL_ATTR_NO_AURA_CANCEL` (`SpellAuraHolder::SetAuraFlag`), so it is
    /// both "this is a buff" and "you may drop it". It is what
    /// `GetPlayerBuff`'s `CANCELABLE` / `NOT_CANCELABLE` filter selects on.
    pub const CANCELABLE: u8 = 0x01;
    /// The three effect-slot bits, `AFLAG_EFF_INDEX_0`..`2`. Read by nothing
    /// here; named so that a flag word printed in a trace is legible.
    pub const EFF_INDEX_0: u8 = 0x08;
    pub const EFF_INDEX_1: u8 = 0x04;
    pub const EFF_INDEX_2: u8 = 0x02;
}

/// One occupied aura slot on a unit — see [`Entity::aura_slots`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuraSlot {
    /// Which of the 48, and therefore whether this is a buff — see
    /// [`POSITIVE_AURA_SLOTS`].
    pub slot: u8,
    /// The `Spell.dbc` id.
    pub spell: u32,
    /// [`aura_flags`].
    pub flags: u8,
    /// The caster's level. Carried because the wire carries it; nothing in this
    /// client reads it yet, and 1.12's own interface has no function that
    /// exposes it either.
    pub level: u8,
    /// The stack count, **one-based** — the field holds count − 1.
    pub applications: u8,
}

impl AuraSlot {
    /// A buff rather than a debuff, by the slot it is in.
    pub fn helpful(&self) -> bool {
        self.slot < POSITIVE_AURA_SLOTS
    }

    /// Whether right-clicking it will be honoured — [`aura_flags::CANCELABLE`].
    pub fn cancelable(&self) -> bool {
        self.flags & aura_flags::CANCELABLE != 0
    }
}

/// **What a hunter's pet panel reads off its own unit fields** — see
/// [`Entity::pet_stats`], which is where every index is pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PetStats {
    /// `UNIT_FIELD_POWER5`. Signed because the client's band comparison is.
    pub happiness: i32,
    /// 1..6, indexing `PetLoyalty.dbc`. Zero for a pet that has none yet.
    pub loyalty_level: u32,
    pub experience: u32,
    pub next_level_experience: u32,
    /// The **high** half of `UNIT_FIELD_TRAINING_POINTS` — see
    /// [`Entity::pet_stats`].
    pub training_total: u16,
    pub training_spent: u16,
}

/// `enum Powers` (`SharedDefines.h`), which is both the value of
/// `UNIT_FIELD_BYTES_0`'s fourth byte and the offset from `UNIT_FIELD_POWER1`.
/// The two facts are the same fact, which is what makes
/// [`Entity::power_field`] arithmetic.
pub mod power_type {
    pub const MANA: u32 = 0;
    pub const RAGE: u32 = 1;
    pub const FOCUS: u32 = 2;
    pub const ENERGY: u32 = 3;
    /// A hunter pet's, and the one the player frame never draws.
    pub const HAPPINESS: u32 = 4;

    /// **Rage is stored in tenths, and nothing on the wire says so.**
    ///
    /// A warrior's `UNIT_FIELD_POWER2` runs 0..1000 for the hundred points the
    /// interface shows, and `Spell.dbc`'s cost column is in the same units — 100
    /// for Shield Block's ten rage. vmangos does the multiplying openly
    /// (`ModifyPower(POWER_RAGE, addRage * 10)`), so a client that shows the
    /// field raw reports a warrior at "1000/1000 rage" and every ability costing
    /// ten times what it does.
    ///
    /// Mana, focus and energy are one for one; happiness is a pet's and is not
    /// drawn. Applied to *both* a bar and a cost, which is why it takes a plain
    /// number rather than living on either.
    pub fn display(kind: u8, value: u32) -> u32 {
        if u32::from(kind) == RAGE {
            value / 10
        } else {
            value
        }
    }

    /// What to call it on a bar. The words are the game's — `GlobalStrings.lua`
    /// keys `MANA`, `RAGE`, `FOCUS`, `ENERGY`, `HAPPINESS`.
    pub fn key(kind: u8) -> &'static str {
        match u32::from(kind) {
            RAGE => "RAGE",
            FOCUS => "FOCUS",
            ENERGY => "ENERGY",
            HAPPINESS => "HAPPINESS",
            _ => "MANA",
        }
    }
}

/// **What kind of thing a guid names**, off its top sixteen bits.
///
/// `ObjectGuid`'s high word, which the server builds into every guid it sends
/// and which is the only thing that tells an item from a creature without a
/// lookup. The four values 1.12 puts on the wire in front of this client:
///
/// ```text
/// 0x0000  a player          — the low bits are the character's own id
/// 0x4000  an item           — in a bag, and therefore nowhere in the world
/// 0xF110  a game object     — a door, a chest, an ore vein
/// 0xF130  a creature        — an NPC, and the entry is in the middle bits
/// ```
///
/// **The reason this exists is the one thing on that list with no position.**
/// An item is a perfectly ordinary quest giver as far as the server is
/// concerned — `HandleQuestgiverQueryQuestOpcode` looks a guid up through
/// `TYPEMASK_CREATURE_GAMEOBJECT_OR_ITEM` — so a quest page can be *about* a
/// thing that is not in the world, and every client-side rule that measures the
/// distance to whoever the player is talking to has to know that.
pub fn guid_high(guid: u64) -> u16 {
    (guid >> 48) as u16
}

/// `HIGHGUID_ITEM`. See [`guid_high`].
pub const HIGHGUID_ITEM: u16 = 0x4000;
/// `HIGHGUID_GAMEOBJECT`.
pub const HIGHGUID_GAMEOBJECT: u16 = 0xF110;
/// `HIGHGUID_UNIT`.
pub const HIGHGUID_UNIT: u16 = 0xF130;

/// **Is this guid something that stands somewhere?**
///
/// `false` for an item, which is in a bag; `true` for everything else,
/// including a player (whose high word is zero). It is the test a range check
/// has to make before it decides that a guid resolving to nothing means the
/// thing walked away.
pub fn guid_is_in_the_world(guid: u64) -> bool {
    guid_high(guid) != HIGHGUID_ITEM
}

impl ObjectManager {

    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn get(&self, guid: u64) -> Option<&Entity> {
        self.entities.get(&guid)
    }

    pub fn get_mut(&mut self, guid: u64) -> Option<&mut Entity> {
        self.entities.get_mut(&guid)
    }

    /// Forget an entity the server says is gone or out of range.
    ///
    /// Never forgets the player: the server sends us out of range during a
    /// teleport, and dropping that entry would lose the session's anchor —
    /// the live loop would have nothing left to dead-reckon from.
    ///
    /// **An item leaving is an inventory change, and this is the door it
    /// actually leaves by.** `SMSG_DESTROY_OBJECT` is a bare guid handled
    /// outside [`Self::apply`] entirely (`socket::handler::world::destroy`), so
    /// the latch that the out-of-range arm sets was never reached for the case
    /// that happens: an item is not in the grid, so nothing ever names one in an
    /// `OUT_OF_RANGE` block, while *every* server-side destruction of a carried
    /// item — a stack merged into another, a charge used up, a quest item taken,
    /// anything moved out of the inventory — sends this opcode. Bumping the
    /// version here is what makes the slot clear on the frame the packet lands
    /// rather than on whatever moves a field next.
    pub fn remove(&mut self, guid: u64) {
        if Some(guid) == self.player_guid {
            return;
        }
        let Some(entity) = self.entities.remove(&guid) else {
            return;
        };
        if is_carried_object(&entity) {
            self.inventory_version = self.inventory_version.wrapping_add(1);
        }
    }

    pub fn player(&self) -> Option<&Entity> {
        self.player_guid.and_then(|g| self.entities.get(&g))
    }

    /// Forget the map the player has just left.
    ///
    /// A far teleport (`SMSG_NEW_WORLD`) does **not** destroy the old map's
    /// objects packet by packet — the player is simply removed from that map, so
    /// no `SMSG_DESTROY_OBJECT` and no out-of-range block ever arrives for any of
    /// them. Left alone they stay in the world forever: a Stormwind guard drawn
    /// standing in the Barrens, at coordinates that are perfectly valid on the
    /// map he is no longer on, dead-reckoned along a spline that finished
    /// somewhere else. Nothing warns, because from this client's point of view
    /// the server merely stopped mentioning them.
    ///
    /// **The player survives, and the template caches survive.** The player is
    /// the session's anchor — [`ObjectManager::remove`] already refuses to drop
    /// it for the same reason — and a creature template, an item's appearance or
    /// another player's name is keyed by entry or GUID and is true on any map, so
    /// throwing those away would only buy a re-query storm on arrival.
    ///
    /// **…and so does the deck the character is standing on.**
    /// `Map::SendInitTransports` builds a create block for every transport on
    /// the new map *except* `player->GetTransport()`, so the boat a character
    /// crossed on is the one object the far side never states. The reference
    /// client does not need it stated, because `SMSG_NEW_WORLD` does not
    /// destroy its transport object; dropping it here loses the entry and the
    /// path progress that are the whole of where a continent transport is, and
    /// leaves the passenger standing at deck height over open water until
    /// `PLATFORM_HOLD` runs out.
    ///
    /// `riding` is the ferry's guid when `SMSG_TRANSFER_PENDING` said this
    /// teleport keeps the transport, and `None` for every other teleport — see
    /// `crate::socket::handler::acks::new_world`, which is where the two are
    /// told apart.
    pub fn leave_map(&mut self, riding: Option<u64>) {
        let player = self.player_guid;
        let kept = |guid: u64| Some(guid) == player || Some(guid) == riding;
        // **The bags go with the map.** Every item and container object is
        // dropped here, and the player object — whose slot fields still name
        // them — is not, so a snapshot taken before this and not rebuilt shows
        // a full inventory of guids that resolve to nothing. See
        // [`Self::remove`], which is the same argument one packet at a time.
        if self
            .entities
            .values()
            .any(|entity| Some(entity.guid) != player && is_carried_object(entity))
        {
            self.inventory_version = self.inventory_version.wrapping_add(1);
        }
        self.entities.retain(|guid, _| kept(*guid));
        // Whatever the old map had this entity doing is over.
        if let Some(entity) = player.and_then(|g| self.entities.get_mut(&g)) {
            entity.spline = None;
            entity.facing_target = None;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    /// Entities of a given type.
    pub fn of_type(&self, ty: ObjectType) -> impl Iterator<Item = &Entity> {
        self.entities
            .values()
            .filter(move |e| e.object_type == Some(ty))
    }

    /// The creature template behind an entity, once it has been queried.
    ///
    /// **The template is where everything the plate says about a creature
    /// lives** — its `<Innkeeper>` tag, its `CreatureType.dbc` row and its
    /// classification — and none of it is in an update block, which is why this
    /// is a lookup by entry rather than a field. `None` for a player (which has
    /// no template at all) and for the round trip before
    /// `SMSG_CREATURE_QUERY_RESPONSE` lands.
    pub fn creature_of(&self, entity: &Entity) -> Option<&CreatureInfo> {
        if entity.object_type == Some(ObjectType::Player) {
            return None;
        }
        entity.entry().and_then(|entry| self.creatures.get(&entry))
    }

    /// …and the same for a **game object**: the template `CMSG_GAMEOBJECT_QUERY`
    /// brought back, or `None` while it is still in flight.
    ///
    /// The type check is the mirror of [`ObjectManager::creature_of`]'s and it
    /// matters for the same reason: the two tables are keyed by entry and the
    /// id spaces overlap outright — entry 1731 is a Copper Vein in one and a
    /// creature in the other — so a lookup that did not first ask what the
    /// entity *is* would answer a chest with a wolf's template.
    pub fn gameobject_of(&self, entity: &Entity) -> Option<&GameObjectInfo> {
        if entity.object_type != Some(ObjectType::GameObject) {
            return None;
        }
        entity.entry().and_then(|entry| self.gameobjects.get(&entry))
    }

    /// Name for an entity, once its creature template has been queried.
    /// Falls back to the entry id, then the type, so output is never blank.
    pub fn name_of(&self, entity: &Entity) -> String {
        match entity.object_type {
            // Players are named by GUID, and have no creature template to fall
            // back on — an unresolved one would otherwise read "entry 0".
            Some(ObjectType::Player) => {
                return match self.players.get(&entity.guid) {
                    Some(info) => info.display_name(),
                    None => format!("Player {}", entity.low_guid()),
                };
            }
            Some(ObjectType::GameObject) => {
                if let Some(info) = entity.entry().and_then(|e| self.gameobjects.get(&e)) {
                    return info.name.clone();
                }
            }
            _ => {
                if let Some(info) = entity.entry().and_then(|e| self.creatures.get(&e)) {
                    return info.display_name();
                }
            }
        }
        match entity.entry() {
            Some(entry) => format!("entry {entry}"),
            None => format!("{:?}", entity.object_type),
        }
    }

    /// **The name alone** — what `UnitName` answers and what a unit frame's
    /// name plate shows.
    ///
    /// [`Self::name_of`] is the *listing* form: a player carries its race and
    /// class, a creature its `<Innkeeper>` tag and its rank. Both are right on
    /// a CLI line and wrong on a name plate — the real client puts the subname
    /// on a tooltip's second line, never inside the name. The fallbacks are
    /// [`Self::name_of`]'s own, so output is still never blank.
    pub fn unit_name_of(&self, entity: &Entity) -> String {
        match entity.object_type {
            Some(ObjectType::Player) => {
                return match self.players.get(&entity.guid) {
                    Some(info) => info.name.clone(),
                    None => format!("Player {}", entity.low_guid()),
                };
            }
            Some(ObjectType::GameObject) => {
                if let Some(info) = entity.entry().and_then(|e| self.gameobjects.get(&e)) {
                    return info.name.clone();
                }
            }
            _ => {
                // **A pet answers what its owner called it**, not its species —
                // the creature template says "Wolf" and the plate, `UnitName`
                // and the paper doll all show "Growlfang". Keyed by pet number,
                // which is what makes the answer survive a dismissal.
                if entity.pet_number() != 0 {
                    if let Some((name, _)) = self.pet_names.get(&entity.pet_number()) {
                        return name.clone();
                    }
                }
                if let Some(info) = entity.entry().and_then(|e| self.creatures.get(&e)) {
                    return info.name.clone();
                }
            }
        }
        match entity.entry() {
            Some(entry) => format!("entry {entry}"),
            None => format!("{:?}", entity.object_type),
        }
    }

    /// Entries that have a creature template we have not fetched yet.
    ///
    /// Deduplicated, so a field of forty identical wolves costs one query.
    pub fn unresolved_creature_entries(&self) -> Vec<(u32, u64)> {
        self.unresolved(ObjectType::Unit, &self.creatures)
    }

    /// The same for game objects, which have their own template table.
    pub fn unresolved_gameobject_entries(&self) -> Vec<(u32, u64)> {
        self.unresolved(ObjectType::GameObject, &self.gameobjects)
    }

    /// Players whose name we have not asked for yet. Keyed by GUID, so there is
    /// nothing to deduplicate — but the player's own name is already known from
    /// the character list, so asking about ourselves would be a wasted trip.
    pub fn unresolved_player_guids(&self) -> Vec<u64> {
        self.entities
            .values()
            .filter(|e| e.object_type == Some(ObjectType::Player))
            .map(|e| e.guid)
            // **…and the group, which is not in the entity walk.** See
            // [`Self::group_guids`]: a member out of range has no entity and a
            // raid frame still has to say what class they are.
            .chain(self.group_guids.iter().copied())
            // …and the friends and ignore lists, which are guids and nothing
            // else at all. See [`Self::friend_guids`].
            .chain(self.friend_guids.iter().copied())
            .chain(self.ignore_guids.iter().copied())
            .chain(self.wanted_social.iter().copied())
            .filter(|g| Some(*g) != self.player_guid && !self.players.contains_key(g))
            .collect()
    }

    /// Item entries worn by a player in sight that have not been looked up yet.
    ///
    /// Deduplicated across players and slots, which matters more here than
    /// anywhere else: nineteen slots times everyone in the inn is a lot of
    /// packets for a handful of distinct items. Entries the server has already
    /// refused are not asked for again.
    pub fn unresolved_item_entries(&self) -> Vec<u32> {
        let mut seen: HashSet<u32> = HashSet::new();
        let want = |entry: u32, seen: &mut HashSet<u32>| {
            if entry != 0 && !self.items.contains_key(&entry) && !self.unknown_items.contains(&entry)
            {
                seen.insert(entry);
            }
        };
        for e in self.entities.values() {
            let Some(equipment) = e.equipment() else {
                continue;
            };
            for entry in equipment {
                want(entry, &mut seen);
            }
        }
        // **…and everything in our own bags**, which is a different set from
        // the one above and much the larger of the two. A worn item is visible
        // to everyone and comes through `PLAYER_VISIBLE_ITEM_n`; a stack of
        // linen in the third bag is visible to nobody and exists only as an
        // `Item` object the server created for us. Without this the inventory
        // has GUIDs and entries and no names, icons or tooltips at all — which
        // is exactly what it had, because the equipment walk above cannot see
        // an item that is not being *worn*.
        //
        // Every item the local player owns is already in `entities`, so this is
        // a walk of what is there rather than a second traversal of the bags —
        // and it deliberately does not go through `items::Inventory`, since an
        // item whose container has not arrived yet still has a name worth
        // asking for.
        for e in self.entities.values() {
            if matches!(
                e.object_type,
                Some(ObjectType::Item) | Some(ObjectType::Container)
            ) {
                want(e.entry().unwrap_or(0), &mut seen);
            }
        }
        // …and whatever the interface asked about — see [`Self::wanted_items`].
        for entry in &self.wanted_items {
            want(*entry, &mut seen);
        }
        seen.into_iter().collect()
    }

    /// **Ask for an item's template because something wants its name**, rather
    /// than because a unit is wearing it.
    ///
    /// Idempotent and cheap enough to call from a tooltip: an entry already
    /// resolved, already refused or already queued does nothing. It never
    /// *sends* — [`crate::socket::session`]'s own query pass does that on its own
    /// interval, so a hover cannot put a packet on the wire and a hundred
    /// hovers cannot put a hundred.
    pub fn want_item(&mut self, entry: u32) {
        if entry == 0 || self.items.contains_key(&entry) || self.unknown_items.contains(&entry) {
            return;
        }
        self.queries_wanted |= self.wanted_items.insert(entry);
    }

    /// **Fill the tables from the on-disk caches**, before a session starts.
    ///
    /// The state kinds are parsed into the same tables the socket's answers go
    /// into, through the same parsers, so the query pass never asks for them
    /// and the first window of the session already has its names. The
    /// on-demand kinds are held as bodies for [`Self::cached_answer`]. Every
    /// key read is marked known, so a restated answer is not written again.
    /// Recording is switched on when the caches have anywhere to write.
    pub fn seed_cache(&mut self, caches: &Caches) {
        self.record_cache = caches.enabled();
        for (_, body) in caches.seed(Kind::Item) {
            if let Some(info) = crate::state::query::parse_item_response(body) {
                self.items.insert(info.entry, info);
            }
        }
        for (_, body) in caches.seed(Kind::Creature) {
            if let Some(info) = crate::state::query::parse_creature_response(body) {
                self.creatures.insert(info.entry, info);
            }
        }
        for (_, body) in caches.seed(Kind::GameObject) {
            if let Some(info) = crate::state::query::parse_gameobject_response(body) {
                self.gameobjects.insert(info.entry, info);
            }
        }
        for (_, body) in caches.seed(Kind::Name) {
            if let Some(info) = crate::state::query::parse_name_response(body) {
                self.players.insert(info.guid, info);
            }
        }
        for (_, body) in caches.seed(Kind::PetName) {
            if let Some(name) = crate::play::pet::parse_pet_name(body) {
                self.pet_names
                    .insert(name.pet_number, (name.name, name.timestamp));
            }
        }
        for kind in Kind::ALL {
            for (key, body) in caches.seed(kind) {
                self.cache_known.insert((kind, *key));
                if kind.on_demand() {
                    self.on_demand.insert((kind, *key), body.clone());
                }
            }
        }
    }

    /// **Keep an answer's body for the on-disk cache**, once per key.
    ///
    /// Called by the handler of each cached response with the packet body
    /// verbatim. A key already on disk or already learned this session is
    /// skipped, which is what stops a template the server restates — every
    /// login re-sends the equipment of everyone in view — from being appended
    /// on every session. Nothing is queued for disk while `record_cache` is
    /// off; an on-demand kind is still held for [`Self::cached_answer`], so a
    /// book opened twice in one session is asked for once.
    pub fn remember(&mut self, kind: Kind, key: u64, body: &[u8]) {
        if key == 0 || body.is_empty() {
            return;
        }
        if kind.on_demand() {
            self.on_demand.insert((kind, key), body.to_vec());
        }
        if self.record_cache && self.cache_known.insert((kind, key)) {
            self.learned.push(Learned {
                kind,
                key,
                body: body.to_vec(),
            });
        }
    }

    /// …and keep one that replaces an earlier answer under the same key. A
    /// pet's name changes under its pet number; the file reads the last record
    /// for a key, so the new name is appended rather than skipped.
    pub fn remember_again(&mut self, kind: Kind, key: u64, body: &[u8]) {
        if key == 0 || body.is_empty() {
            return;
        }
        if kind.on_demand() {
            self.on_demand.insert((kind, key), body.to_vec());
        }
        if !self.record_cache {
            return;
        }
        self.cache_known.insert((kind, key));
        self.learned.push(Learned {
            kind,
            key,
            body: body.to_vec(),
        });
    }

    /// The cached body of an on-demand answer, if the disk had one. The
    /// session loop runs it through the packet dispatch in place of sending
    /// the query — see `SessionLoop::answer_from_cache`.
    pub fn cached_answer(&self, kind: Kind, key: u64) -> Option<&[u8]> {
        self.on_demand.get(&(kind, key)).map(Vec::as_slice)
    }

    /// How many on-demand answers are held.
    pub fn cached_answers(&self) -> usize {
        self.on_demand.len()
    }

    /// **Record that an entry has been answered**, either way — see
    /// [`Arrivals`], and the `arrivals` field, for why the edge exists at all.
    pub(crate) fn note_item_arrival(&mut self, entry: u32) {
        self.arrivals.items.push(entry);
    }

    pub(crate) fn note_creature_arrival(&mut self, entry: u32) {
        self.arrivals.creatures.push(entry);
    }

    pub(crate) fn note_gameobject_arrival(&mut self, entry: u32) {
        self.arrivals.gameobjects.push(entry);
    }

    /// …and take the ones since the last look, leaving the lists empty.
    ///
    /// Deliberately a drain rather than a peek: two readers would each see the
    /// arrival and the second would be raising events about a template that had
    /// already been folded in, which on a busy login is a redraw per item per
    /// frame.
    pub fn take_arrivals(&mut self) -> Arrivals {
        std::mem::take(&mut self.arrivals)
    }

    /// What one unit has in its three weapon slots, whichever route the facts
    /// took to get here.
    ///
    /// **This is the only place the creature/player split for weapons is made,
    /// and it exists so that `vale_assets::look::dress` never has to.** A creature
    /// carries the whole record in its update fields; a player carries three
    /// item *entries* whose templates arrive from `CMSG_ITEM_QUERY_SINGLE` a
    /// round trip later. A player whose queries have not come back yet has empty
    /// hands here, which is correct and temporary — the same lateness the rest
    /// of their wardrobe has, and the renderer rebuilds when it lands.
    ///
    /// An entry the server *refused* to describe stays empty for good, which is
    /// the honest outcome: there is no display id to draw.
    pub fn weapons_of(&self, entity: &Entity) -> [HeldItem; 3] {
        let Some(entries) = entity.weapon_entries() else {
            return entity.virtual_items();
        };
        let mut out = [HeldItem::default(); 3];
        for (weapon, entry) in out.iter_mut().zip(entries) {
            let Some(info) = self.items.get(&entry) else {
                continue;
            };
            if info.display_id == 0 {
                continue;
            }
            *weapon = HeldItem {
                display_id: info.display_id,
                class: info.class as u8,
                subclass: info.subclass as u8,
                // **`-1` is a real value here** ("no material") and it is not
                // metal, so the cast that matters is the one that does not turn
                // it into 255-and-therefore-not-1 by accident. It does not:
                // only equality with 1 is ever asked.
                material: info.material as u8,
                inventory_type: info.inventory_type as u8,
                sheath: info.sheath as u8,
            };
        }
        out
    }

    fn unresolved<T>(&self, ty: ObjectType, known: &HashMap<u32, T>) -> Vec<(u32, u64)> {
        let mut seen: HashMap<u32, u64> = HashMap::new();
        for e in self.entities.values() {
            if e.object_type != Some(ty) {
                continue;
            }
            if let Some(entry) = e.entry() {
                if !known.contains_key(&entry) {
                    seen.entry(entry).or_insert(e.guid);
                }
            }
        }
        // **…and whatever the interface asked about**, which is a different set
        // entirely: a quest objective names a creature that is very often not in
        // view — "Kobold Vermin slain: 3/8" is read in a log opened in Stormwind
        // — so the walk above can never resolve it. See [`Self::want_creature`].
        //
        // **The guid is zero and that is correct**: `HandleCreatureQueryOpcode`
        // reads the entry, reads the guid and then never uses it —
        // `GetCreatureTemplate(entry)` is the whole lookup. The same is true of
        // the game-object handler.
        let asked = match ty {
            ObjectType::GameObject => &self.wanted_gameobjects,
            _ => &self.wanted_creatures,
        };
        for entry in asked {
            if !known.contains_key(entry) {
                seen.entry(*entry).or_insert(0);
            }
        }
        seen.into_iter().collect()
    }

    /// **Ask for a creature's template because something wants its name.**
    ///
    /// The sibling of [`Self::want_item`], with the same contract: idempotent,
    /// never touches the socket, drained by the session's own query pass. The
    /// caller is the quest log, whose objective lines are the client's own join
    /// of a counter the log carries and a name only this table has.
    pub fn want_creature(&mut self, entry: u32) {
        if entry != 0 && !self.creatures.contains_key(&entry) {
            self.queries_wanted |= self.wanted_creatures.insert(entry);
        }
    }

    /// …and the same for a game object, which is the other half of what an
    /// objective can be aimed at — `ReqCreatureOrGOId`'s top bit decides which.
    pub fn want_gameobject(&mut self, entry: u32) {
        if entry != 0 && !self.gameobjects.contains_key(&entry) {
            self.queries_wanted |= self.wanted_gameobjects.insert(entry);
        }
    }

    /// **Who is in the group now**, replacing whoever was — see
    /// [`Self::group_guids`].
    ///
    /// Replaced rather than merged, because the roster packet is the whole
    /// state: somebody who left must stop being asked about. The hint is raised
    /// only when a guid is *new* to the name cache, so the ordinary re-send of
    /// an unchanged roster costs nothing.
    pub fn note_group(&mut self, guids: Vec<u64>) {
        self.queries_wanted |= guids
            .iter()
            .any(|guid| *guid != 0 && !self.players.contains_key(guid));
        self.group_guids = guids;
    }

    /// **Who is on the friends list now**, replacing whoever was — see
    /// [`Self::friend_guids`].
    ///
    /// Replaced rather than merged, for the reason [`Self::note_group`] gives:
    /// somebody removed must stop being asked about. The hint is raised only for
    /// a guid the name cache has never answered, so the packet arriving twice
    /// costs nothing.
    pub fn note_social_friends(&mut self, guids: Vec<u64>) {
        self.queries_wanted |= self.any_unnamed(&guids);
        self.friend_guids = guids;
    }

    /// …and the same for the ignore list.
    pub fn note_social_ignores(&mut self, guids: Vec<u64>) {
        self.queries_wanted |= self.any_unnamed(&guids);
        self.ignore_guids = guids;
    }

    /// **One guid a mid-session change named** — see [`Self::wanted_social`].
    pub fn want_social_guid(&mut self, guid: u64) {
        if guid != 0 && !self.players.contains_key(&guid) {
            self.queries_wanted |= self.wanted_social.insert(guid);
        }
    }

    fn any_unnamed(&self, guids: &[u64]) -> bool {
        guids
            .iter()
            .any(|guid| *guid != 0 && !self.players.contains_key(guid))
    }

    /// **Is it worth walking the world for things to ask about?** — taken and
    /// cleared by [`crate::socket::session`]'s query pass.
    ///
    /// The pass itself is four walks of every entity in view, and it now runs on
    /// the session tick rather than on a two-second beat, so that a name a
    /// window is waiting on leaves on the next packet instead of up to two
    /// seconds later. That is 25 ms against 2,000, and paying four walks for it
    /// forty times a second — **under the world lock**, against a renderer that
    /// wants the same lock sixty times a second — would be trading one cost for
    /// another.
    ///
    /// This is the O(1) answer to "has anything changed since the last pass".
    /// It is raised by exactly two things and both are the truth:
    ///
    /// * one of the `want_*` calls above actually inserted — a loot row, a
    ///   vendor row, a quest objective, a tooltip's reagent;
    /// * an update block **created an object** or moved something in the
    ///   character's inventory — the only two ways an entry this client has
    ///   never seen can arrive on the wire.
    ///
    /// A *movement* block does not raise it, which is the point: those are most
    /// of the traffic in a busy zone and none of them can name anything new.
    /// Nor does an equipment change on an entity that already exists and whose
    /// count is unchanged — that one waits for the beat, and a beat is still
    /// there because a query is fire-and-forget and an unanswered one has to be
    /// re-asked anyway.
    pub fn take_query_hint(&mut self) -> bool {
        std::mem::take(&mut self.queries_wanted)
    }

    /// **…and the one door for raising it from outside.**
    ///
    /// `SMSG_PET_SPELLS` is the case: it names nothing new by itself, so the
    /// create-block rule above does not fire for it, and the name it makes
    /// askable is derived rather than queued. Without this the pet's name waits
    /// for the next two-second beat.
    pub fn hint_queries(&mut self) {
        self.queries_wanted = true;
    }

    /// Start animating a server-driven move.
    ///
    /// A stop packet (no destination) parks the unit where the server says it
    /// is, which also corrects any drift our interpolation accumulated.
    pub fn apply_monster_move(&mut self, mm: &MonsterMove) {
        // Resolved here rather than in `advance`, because only the manager can
        // look another entity up — a creature told to face its victim needs the
        // victim's position, and that is the combat case.
        let facing_target = match mm.facing {
            SplineFacing::Target(guid) => self
                .entities
                .get(&guid)
                .and_then(|e| e.position)
                .map(|p| [p.x, p.y, p.z]),
            _ => None,
        };

        let entity = self
            .entities
            .entry(mm.guid)
            .or_insert_with(|| Entity::new(mm.guid));
        let previous = entity.position.map(|p| p.orientation).unwrap_or(0.0);

        let moving = mm.duration_ms > 0 && mm.path.len() >= 2;
        entity.spline = moving.then(|| Spline {
            path: mm.path.clone(),
            duration_ms: mm.duration_ms,
            elapsed_ms: 0,
            facing: mm.facing,
            cyclic: mm.flags & crate::state::movement::spline_flags::CYCLIC != 0,
            flying: mm.flags & crate::state::movement::spline_flags::FLYING != 0,
            falling: mm.flags & crate::state::movement::spline_flags::FALLING != 0,
            // `SMSG_MONSTER_MOVE_TRANSPORT` — every waypoint above is in this
            // transport's own frame. See `Spline::in_world`.
            on_transport: mm.transport,
        });
        // A server-driven move supersedes the client's dead reckoning, exactly
        // as `apply_movement` supersedes a spline in the other direction.
        //
        // Without this the two survive each other: a player who is knocked back,
        // charged or dragged gets a spline over the top of a `MovementInfo` that
        // still says FORWARD, and the moment the spline retires, `advance` finds
        // that stale block and starts walking them again — off in the direction
        // they were running before, at run speed, forever, with the server never
        // saying otherwise because as far as it is concerned they are standing
        // still. It reads as an entity drifting away on its own.
        entity.movement = None;

        // Facing. A unit walks forwards, so the direction of travel is the
        // answer while it is moving; a stop packet has no travel, so it keeps
        // what the packet asks for, or what it had.
        //
        // Without this a creature keeps whatever orientation it was created
        // with for the whole session and walks sideways or backwards for all of
        // it — the animation is right and the model is turned the wrong way,
        // which reads as an animation bug.
        let orientation = match (&entity.spline, mm.facing) {
            (Some(spline), _) => spline.position_and_heading().1,
            (None, SplineFacing::Angle(a)) => a,
            (None, SplineFacing::Spot(spot)) => bearing(mm.start, spot),
            (None, SplineFacing::Target(_)) => match facing_target {
                Some(at) => bearing(mm.start, at),
                None => previous,
            },
            (None, SplineFacing::Travel) => previous,
        };
        entity.facing_target = facing_target;

        let moved = entity.set_server_position(Position {
            x: mm.start[0],
            y: mm.start[1],
            z: mm.start[2],
            orientation,
        });
        match moved {
            Some(moved) => self.note_jump(mm.guid, moved, "SMSG_MONSTER_MOVE"),
            None => {
                // The start point is the one thing every branch above depends
                // on, so an impossible one invalidates the spline too — better a
                // stale creature than one walking a path from nowhere.
                if let Some(entity) = self.entities.get_mut(&mm.guid) {
                    entity.spline = None;
                }
                self.rejected_positions = self.rejected_positions.saturating_add(1);
            }
        }
    }

    /// Note that a server statement moved an entity further than it could have
    /// walked, and say which packet did it.
    ///
    /// Called by every site that writes a server position, because only the
    /// call site knows the opcode. The player is excluded: the live session
    /// dead-reckons it and writes the result back here, so a "jump" on the
    /// player is the resync working rather than an entity moving.
    fn note_jump(&mut self, guid: u64, moved: f32, source: &str) {
        if moved <= JUMP_YARDS || Some(guid) == self.player_guid {
            return;
        }
        self.jump_count = self.jump_count.saturating_add(1);
        if self.jump_log.len() < JUMP_SAMPLES {
            let what = self
                .entities
                .get(&guid)
                .map(|e| self.name_of(e))
                .unwrap_or_else(|| format!("guid {guid:#x}"));
            self.jump_log
                .push(format!("{what} moved {moved:.0}y on {source}"));
        }
    }

    /// Note a line of chat, dropping the oldest if nobody is reading.
    pub fn note_chat(&mut self, message: crate::play::chat::ChatMessage) {
        if self.chat.len() >= CHAT_BACKLOG {
            self.chat.remove(0);
        }
        self.chat.push(message);
    }

    /// Take everything that has arrived since the last call.
    ///
    /// A line is an event and must be shown once, so this **empties** the queue
    /// — see the field. Two readers in one process would therefore each see half
    /// the conversation, which is why there is only ever one.
    pub fn take_chat(&mut self) -> Vec<crate::play::chat::ChatMessage> {
        std::mem::take(&mut self.chat)
    }

    /// **What is over a quest giver's head**, by guid.
    ///
    /// State rather than an edge, and the only member of the quest family that
    /// is: the mark stays there until the server says otherwise, so the pass
    /// that draws it has to be able to *ask* rather than to have been listening
    /// when the answer arrived — a mark that only existed as an event would be
    /// missing for every unit that streamed into view before the interface did.
    ///
    /// Cleared with the entity it is about ([`Self::forget_quest_status`]), so
    /// a guid the server reuses in another zone cannot inherit one.
    pub fn set_quest_status(&mut self, guid: u64, status: crate::play::quest::DialogStatus) {
        match status {
            // **Nothing is an absence, not a value.** Keeping a `None` in the
            // map would make this grow by one entry per giver per session and
            // would make "have we asked?" unanswerable — see
            // [`Self::wants_quest_status`], which is what stops the client
            // asking the same giver twice a second.
            crate::play::quest::DialogStatus::None => {
                self.quest_status.remove(&guid);
            }
            status => {
                self.quest_status.insert(guid, status);
            }
        }
        self.asked_quest_status.insert(guid);
    }

    /// …and the answer, or `None` for a giver nobody has asked about.
    pub fn quest_status(&self, guid: u64) -> Option<crate::play::quest::DialogStatus> {
        self.quest_status.get(&guid).copied()
    }

    /// **Whether this giver is still worth a `CMSG_QUESTGIVER_STATUS_QUERY`.**
    ///
    /// Once per guid, until the log moves — see [`Self::requery_quest_status`].
    pub fn wants_quest_status(&self, guid: u64) -> bool {
        !self.asked_quest_status.contains(&guid)
    }

    /// **Ask everyone again**, because the answers just went stale.
    ///
    /// `SMSG_QUESTGIVER_STATUS` is sent from exactly one place in vmangos —
    /// `HandleQuestgiverStatusQueryOpcode`, in reply — so **nothing on the wire
    /// ever announces that a mark changed**. What is over a head is a function
    /// of the character's own quest log, and the only honest trigger is
    /// therefore the log itself moving: accepting turns a giver's gold `!` into
    /// a grey `?`, killing the eighth kobold turns that into a gold one, and
    /// handing in takes it away. Without this a mark is whatever it was when
    /// the unit first came into view, for the rest of the session — which is
    /// "the turn-in NPC still has a `!`" and "the other one has nothing at all",
    /// both reported off the same screenshot.
    ///
    /// **The answers are kept and only the asks are forgotten**, so the marks
    /// do not blink out for the round trip. A giver whose new answer is
    /// `None` is cleared by [`Self::set_quest_status`] when it lands.
    pub fn requery_quest_status(&mut self) {
        self.asked_quest_status.clear();
    }

    /// Forget both, for an entity that has left the world.
    pub fn forget_quest_status(&mut self, guid: u64) {
        self.quest_status.remove(&guid);
        self.asked_quest_status.remove(&guid);
        self.taxi_status.remove(&guid);
        self.asked_taxi_status.remove(&guid);
    }

    /// **Whether this flight master's own node is one we have stood at**, from
    /// `SMSG_TAXINODE_STATUS`.
    ///
    /// State rather than an edge for the same reason the quest status is: the
    /// green `!` over an undiscovered master stays there until something says
    /// otherwise, so the pass that draws it has to be able to *ask*.
    ///
    /// **Both answers are kept, and that is the difference from the quest
    /// map**, which drops a `None`. Here `false` is the interesting value — it
    /// is what puts the mark up — so the map holds it and the asked set exists
    /// only to stop the query repeating.
    pub fn set_taxi_status(&mut self, guid: u64, known: bool) {
        self.taxi_status.insert(guid, known);
        self.asked_taxi_status.insert(guid);
    }

    /// …and the answer, or `None` for a master nobody has asked about.
    pub fn taxi_status(&self, guid: u64) -> Option<bool> {
        self.taxi_status.get(&guid).copied()
    }

    /// **Whether this master is still worth a `CMSG_TAXINODE_STATUS_QUERY`.**
    ///
    /// Once per guid, and **there is no requery**: unlike the quest mark, whose
    /// answer changes with the character's own log and which nothing on the wire
    /// announces, this one is announced. `SendLearnNewTaxiNode` writes
    /// `SMSG_NEW_TAXI_PATH` and then a fresh `SMSG_TAXINODE_STATUS` carrying 1
    /// for the very master that was just discovered, so the mark takes itself
    /// down in the same breath as the message goes up.
    pub fn wants_taxi_status(&self, guid: u64) -> bool {
        !self.asked_taxi_status.contains(&guid)
    }

    /// Note what the server said about one of our own presses, on the same
    /// terms as [`Self::note_chat`]: bounded, oldest dropped.
    pub fn note_event(&mut self, event: crate::play::spells::PlayerEvent) {
        if self.events.len() >= EVENT_BACKLOG {
            self.events.remove(0);
        }
        self.events.push(event);
    }

    /// Take everything that has happened since the last call, emptying the
    /// queue. Same contract as [`Self::take_chat`], and the same single reader.
    pub fn take_events(&mut self) -> Vec<crate::play::spells::PlayerEvent> {
        std::mem::take(&mut self.events)
    }

    /// Note one thing that happened in a fight, for the combat log.
    ///
    /// Bounded at [`COMBAT_BACKLOG`], oldest dropped — see that constant for
    /// why this is not [`Self::note_event`].
    pub fn note_combat(&mut self, event: crate::play::combatlog::CombatEvent) {
        if self.combat.len() >= COMBAT_BACKLOG {
            self.combat.remove(0);
        }
        self.combat.push(event);
    }

    /// …and take them, in arrival order.
    ///
    /// **Arrival order is the whole point.** A swing, the spell that followed
    /// it and the death it caused are three packets whose sense depends on
    /// being read in the order they were sent, and a reader that took each kind
    /// from its own queue would interleave them differently every frame.
    pub fn take_combat(&mut self) -> Vec<crate::play::combatlog::CombatEvent> {
        std::mem::take(&mut self.combat)
    }

    /// `SMSG_UPDATE_AURA_DURATION`: how long the aura in this slot has left.
    ///
    /// **Never a deletion.** The packet is only ever sent for a live aura, so a
    /// record whose slot has since been recycled is stale rather than wrong, and
    /// the reader is what rejects it — see [`Self::aura_durations`] and, in the
    /// client, `game::auras`, which will not join a reading that predates the
    /// aura it would be joined to.
    pub fn note_aura_duration(&mut self, slot: u8, remaining_ms: u32) {
        let seq = self
            .aura_durations
            .get(&slot)
            .map_or(0, |held| held.seq.wrapping_add(1));
        self.aura_durations.insert(
            slot,
            AuraDuration {
                remaining_ms,
                received: Instant::now(),
                seq,
            },
        );
    }

    /// What has been said about our own buffs' clocks, as `(slot, reading)`.
    ///
    /// **Read rather than drained**, unlike the chat and the press answers: a
    /// duration is state that stays true between frames, and a reader that
    /// missed the packet still has to be able to draw the timer. Bounded at 48
    /// entries by the field layout, so there is nothing to prune.
    pub fn aura_durations(&self) -> impl Iterator<Item = (u8, AuraDuration)> + '_ {
        self.aura_durations.iter().map(|(slot, held)| (*slot, *held))
    }

    /// `SMSG_INITIAL_SPELLS`: the whole spellbook, replacing whatever was held.
    ///
    /// Replacing rather than merging, because the packet is the server's
    /// complete statement — and it is sent again after a talent reset, where a
    /// merge would leave the un-learned ranks on the bar forever.
    pub fn apply_spellbook(&mut self, book: crate::play::spells::Spellbook) {
        for cooldown in &book.cooldowns {
            self.note_event(crate::play::spells::PlayerEvent::CooldownStarted {
                spell_id: cooldown.spell_id,
                ms: cooldown.spell_ms.max(cooldown.category_duration_ms()),
            });
        }
        self.spellbook = book;
        self.spellbook_version = self.spellbook_version.wrapping_add(1);
    }

    /// **`SMSG_PET_SPELLS` landed** — the bar, or the dismissal that takes it
    /// away.
    ///
    /// The dismissal is not a special case here beyond the name it is given:
    /// a `PetSpells` with a zero guid *is* the empty bar, so storing it is
    /// storing the truth.
    pub fn apply_pet_spells(&mut self, spells: crate::play::pet::PetSpells) {
        self.pet = spells;
        self.pet_attacking = false;
        self.pet_version = self.pet_version.wrapping_add(1);
    }

    /// **The client's half of a `CMSG_PET_ACTION` press**, applied at the send.
    ///
    /// The server keeps the state and answers nothing: `HandlePetAction` ends
    /// in `SetReactState` for a reaction and `HandlePetCommand` ends in
    /// `SetCommandState` for stay and follow, and neither sends `SMSG_PET_MODE`
    /// (`Pet::SetEnabled` is its only sender). So the reference mirrors the
    /// press into its own copy before the packet goes, and this does the same:
    ///
    /// * a reaction slot writes the low byte of the state word — the react
    ///   state;
    /// * a stay or follow command writes the second byte — the command state;
    /// * an attack command at a target sets the attack flag and writes no
    ///   state byte, which matches the server:
    ///   `HandlePetCommand`'s `COMMAND_ATTACK` arm calls `SetIsCommandAttack`
    ///   and leaves the command state alone;
    /// * dismiss and a spell slot change nothing here.
    ///
    /// Each of the first three fires `PET_BAR_UPDATE` (event `0x161`), which
    /// is why the version is bumped when anything moved. Without the mirror the
    /// three mode buttons and the two command buttons kept the old one pressed
    /// until the next `SMSG_PET_SPELLS`, which a teleport sends and nothing
    /// else does.
    ///
    /// `data` is the slot's packed word and `had_target` is whether the press
    /// carried a target guid.
    pub fn apply_pet_press(&mut self, data: u32, had_target: bool) {
        use crate::play::pet::{active_state, command_state, PetAction};
        if self.pet.pet == 0 {
            return;
        }
        let action = PetAction::unpack(data);
        let value = (action.action & 0xFF) as u8;
        let changed = match action.state {
            active_state::REACTION => {
                let moved = self.pet.react != value;
                self.pet.react = value;
                moved
            }
            active_state::COMMAND => match value {
                command_state::STAY | command_state::FOLLOW => {
                    let moved = self.pet.command != value || self.pet_attacking;
                    self.pet.command = value;
                    self.pet_attacking = false;
                    moved
                }
                command_state::ATTACK if had_target => {
                    let moved = !self.pet_attacking;
                    self.pet_attacking = true;
                    moved
                }
                _ => false,
            },
            _ => false,
        };
        if changed {
            self.pet_version = self.pet_version.wrapping_add(1);
        }
    }

    /// **The client's half of `CMSG_PET_SET_ACTION`**, applied at the send.
    ///
    /// The server stores the move and answers nothing (`HandlePetSetAction`
    /// ends at `SetActionBar`; no packet), so the drag has to land in this copy
    /// or the next rebuild restates the bar as it was before the drop. Each
    /// entry is `(position, packed)` — the same pair the wire carries, one for
    /// a removal and two for a move. See [`crate::play::pet::pet_set_action_body`],
    /// where the length-is-the-count rule is.
    pub fn apply_pet_set_action(&mut self, moves: &[(u32, u32)]) {
        use crate::play::pet::PetAction;
        if self.pet.pet == 0 {
            return;
        }
        let mut changed = false;
        for &(position, packed) in moves {
            let Some(slot) = self.pet.bar.get_mut(position as usize) else {
                continue;
            };
            let action = PetAction::unpack(packed);
            if *slot != action {
                *slot = action;
                changed = true;
            }
        }
        if changed {
            self.pet_version = self.pet_version.wrapping_add(1);
        }
    }

    /// **`CMSG_PET_STOP_ATTACK`'s half** — the attack flag goes, and the bar
    /// redraws with the attack button up.
    pub fn apply_pet_stop_attack(&mut self) {
        if self.pet_attacking {
            self.pet_attacking = false;
            self.pet_version = self.pet_version.wrapping_add(1);
        }
    }

    /// **`SMSG_PET_MODE`** — the four state bytes on their own.
    ///
    /// Applied only to the pet the bar is about. The packet also arrives for a
    /// charm this client is not driving, and writing that over the bar's own
    /// state would leave the panel showing another unit's mood.
    pub fn apply_pet_mode(&mut self, mode: crate::play::pet::PetMode) {
        if self.pet.pet != mode.pet || self.pet.pet == 0 {
            return;
        }
        if self.pet.react == mode.react
            && self.pet.command == mode.command
            && self.pet.flags == mode.flags
        {
            // The server restating what it already said must not rebuild the
            // panel — the same rule `apply_spell_change` keeps.
            return;
        }
        self.pet.react = mode.react;
        self.pet.command = mode.command;
        self.pet.flags = mode.flags;
        self.pet_version = self.pet_version.wrapping_add(1);
    }

    /// **The client's half of `CMSG_PET_SPELL_AUTOCAST`**, applied at the send.
    ///
    /// vmangos records the toggle and answers nothing
    /// (`HandlePetSpellAutocastOpcode` ends at `SetSpellAutocast`; no packet),
    /// so a client that waits for the server never sees the dot move — and the
    /// next press reads the stale state and sends the same toggle again. The
    /// bar and the spellbook list both carry the spell, so both are flipped.
    /// A passive is left alone, which is the server's own refusal
    /// (`IsAutocastable`) applied locally.
    pub fn apply_pet_autocast(&mut self, spell_id: u32, on: bool) {
        use crate::play::pet::active_state;
        let wanted = match on {
            true => active_state::ENABLED,
            false => active_state::DISABLED,
        };
        let mut changed = false;
        for slot in self.pet.bar.iter_mut().chain(self.pet.spells.iter_mut()) {
            if slot.is_spell()
                && slot.action & 0xFFFF == spell_id
                && slot.state != active_state::PASSIVE
                && slot.state != wanted
            {
                slot.state = wanted;
                changed = true;
            }
        }
        if changed {
            self.pet_version = self.pet_version.wrapping_add(1);
        }
    }

    /// **The pet's given name**, keyed by pet number.
    ///
    /// The timestamp is stored even when the name did not change, because it is
    /// what settles the re-ask latch in [`Self::unresolved_pet_names`].
    pub fn apply_pet_name(&mut self, name: crate::play::pet::PetName) {
        self.wanted_pet_names.remove(&name.pet_number);
        let changed = self
            .pet_names
            .insert(name.pet_number, (name.name.clone(), name.timestamp))
            .is_none_or(|(old, _)| old != name.name);
        if changed {
            self.pet_version = self.pet_version.wrapping_add(1);
        }
    }

    /// **The pet's name, if it needs asking for** — `(pet_number, guid)`,
    /// because the packet wants both and only the *number* is the key.
    ///
    /// **Derived rather than queued**, and that is what makes it correct at both
    /// ends of a summon: `SMSG_PET_SPELLS` arrives *before* the pet's create
    /// block, so a set filled when the bar landed would be filled with a number
    /// nothing knew yet — and the create block alone is not enough either,
    /// because a pet that is merely in view is not ours. Asking off the two
    /// together costs one map lookup on the query beat and cannot be early.
    ///
    /// **Asked once per number per timestamp and not retried.** A rename moves
    /// `UNIT_FIELD_PET_NAME_TIMESTAMP` (vmangos `HandlePetRename` writes
    /// `time(nullptr)` into it after the name change), which is the one
    /// statement on the wire that the cached name went stale — so a timestamp
    /// the cache does not hold is asked about once, and a server that declines
    /// to answer is not asked again until the field moves again. The cost of a
    /// miss is a panel showing the species name it already has.
    pub fn unresolved_pet_names(&mut self) -> Vec<(u32, u64)> {
        let guid = self.pet.pet;
        if guid == 0 {
            return Vec::new();
        }
        let Some(entity) = self.get(guid) else {
            return Vec::new();
        };
        let number = entity.pet_number();
        if number == 0 {
            return Vec::new();
        }
        let stamp = entity.pet_name_timestamp();
        if self
            .pet_names
            .get(&number)
            .is_some_and(|(_, cached)| *cached == stamp)
        {
            return Vec::new();
        }
        if self.wanted_pet_names.insert(number, stamp) == Some(stamp) {
            return Vec::new();
        }
        vec![(number, guid)]
    }

    /// One spell learned or unlearned after login.
    pub fn apply_spell_change(&mut self, spell_id: u32, learned: bool) {
        let known = self.spellbook.known.contains(&spell_id);
        match (learned, known) {
            (true, false) => self.spellbook.known.push(spell_id),
            (false, true) => self.spellbook.known.retain(|id| *id != spell_id),
            // Nothing changed, so nothing is bumped: a reader keyed on the
            // version must not rebuild the bar because the server restated
            // something it had already said.
            _ => return,
        }
        self.spellbook_version = self.spellbook_version.wrapping_add(1);
        self.note_event(if learned {
            crate::play::spells::PlayerEvent::SpellLearned(spell_id)
        } else {
            crate::play::spells::PlayerEvent::SpellRemoved(spell_id)
        });
    }

    /// **A rank replaced by a higher one** — `SMSG_SUPERCEDED_SPELL`, which is
    /// two edits rather than one: the book *and* every bar slot holding the old
    /// id.
    ///
    /// Doing only the book is what this client did by omission for its whole
    /// life, and it is worse than doing nothing: the button keeps working as a
    /// picture and stops working as a button, because the server refuses a spell
    /// the character no longer has *active* and refuses it **silently**. See
    /// [`crate::play::spells::parse_superceded_spell`].
    ///
    /// The old id is removed rather than merely deactivated. vmangos keeps it
    /// with `active = false` because it has to remember it for the database;
    /// nothing on this side can press an inactive spell, and `known` is what
    /// both the book and the bar filter against.
    ///
    /// Returns the slots that changed, which the caller owes the server — see
    /// [`crate::play::spells::PlayerEvent::SpellSuperceded`].
    pub fn apply_superceded_spell(&mut self, old: u32, new: u32) {
        use crate::play::spells::action_kind;

        // Only spell buttons: an item entry or a macro index that happens to
        // equal a spell id is a different thing wearing the same number.
        let mut slots: Vec<u8> = Vec::new();
        for button in &mut self.action_buttons {
            if button.kind == action_kind::SPELL && button.action == old {
                button.action = new;
                slots.push(button.slot);
            }
        }

        self.spellbook.known.retain(|id| *id != old);
        if !self.spellbook.known.contains(&new) {
            self.spellbook.known.push(new);
        }
        // **Bumped even when no slot moved**, unlike `set_action_button`: the
        // book itself changed, and the book is what the version latches a
        // rebuild on.
        self.spellbook_version = self.spellbook_version.wrapping_add(1);
        self.note_event(crate::play::spells::PlayerEvent::SpellSuperceded { old, new, slots });
    }

    /// `SMSG_ACTION_BUTTONS`: the whole bar, likewise replacing.
    pub fn apply_action_buttons(&mut self, buttons: Vec<crate::play::spells::ActionButton>) {
        self.action_buttons = buttons;
        self.spellbook_version = self.spellbook_version.wrapping_add(1);
    }

    /// **…and one slot the *client* just changed**, which is the other half of
    /// the same field and the only one the server never states.
    ///
    /// The bar is client state that the server merely stores: `CMSG_SET_ACTION_BUTTON`
    /// is acknowledged with nothing at all and `SMSG_ACTION_BUTTONS` arrives once,
    /// at login. So a drop onto a button has to be recorded here as well as sent,
    /// or the next time anything rebuilds from this list the change is gone — and
    /// "gone at the next level-up" is exactly the shape of bug this project keeps
    /// paying for, because it looks like it worked.
    ///
    /// A `kind` of `None` empties the slot. **The version is deliberately not
    /// bumped**: it is the latch a reader rebuilds the *whole* bar on, and this
    /// caller has already applied its own one-slot change — bumping it would make
    /// every such drop cost 120 DBC lookups.
    pub fn set_action_button(&mut self, slot: u8, action: u32, kind: Option<u8>) {
        self.action_buttons.retain(|button| button.slot != slot);
        if let Some(kind) = kind {
            self.action_buttons.push(crate::play::spells::ActionButton { slot, action, kind });
            // Ascending, the order `parse_action_buttons` produces — nothing
            // reads it positionally today and a list that is sorted in one code
            // path and not the other is a difference waiting to be depended on.
            self.action_buttons.sort_by_key(|button| button.slot);
        }
    }

    /// `SMSG_ATTACKSTART` / `SMSG_ATTACKSTOP` — recorded on the **attacker**, and
    /// on ourselves as well when the attacker is us.
    ///
    /// **Both are broadcast to everyone in sight**, and that is the point rather
    /// than noise to be filtered out: this is the only thing in the protocol
    /// that says a unit is *swinging at* something, and it is what the ready
    /// stance is drawn from ([`Entity::attacking`]). It used to be dropped for
    /// everyone but the local player, on the reasoning that
    /// `UNIT_FLAG_IN_COMBAT` covered the rest — which put every unit that had a
    /// fight anywhere near it into its combat guard, including one being shot at
    /// from across a room and one running away.
    pub fn apply_attack_state(&mut self, attacker: u64, victim: Option<u64>) {
        let victim = victim.filter(|guid| *guid != 0);
        if let Some(unit) = self.entities.get_mut(&attacker) {
            unit.attacking = victim;
        }
        if self.player_guid == Some(attacker) {
            self.attacking = victim;
        }
    }

    /// `MSG_CHANNEL_START` — a channelled spell has begun on the local player.
    ///
    /// **A channel is a duration, not an animation**, and this is the packet
    /// that carries it: `{u32 spell, u32 milliseconds}`, sent by
    /// `Spell::SendChannelStart` with `SendDirectMessage` and therefore **only
    /// to the caster**. `SMSG_SPELL_GO` has already fired by the time it
    /// arrives and put whatever `SMSG_SPELL_START` said in the cast bar — which
    /// for a channel is nothing at all, so an Evocation was drawn as an instant
    /// release and the held pose was never seen.
    ///
    /// Folded into the *cast* counters rather than given a third pair of its
    /// own, because from the animation's side that is exactly what it is: a
    /// wind-up held for a stated time. `SpellVisual`'s channel kit is what
    /// supplies the pose — see `vale_assets::tables::spell`.
    pub fn apply_channel_start(&mut self, spell_id: u32, duration_ms: u32) {
        let Some(guid) = self.player_guid else {
            return;
        };
        let Some(unit) = self.entities.get_mut(&guid) else {
            return;
        };
        unit.casts_begun = unit.casts_begun.wrapping_add(1);
        unit.cast_time_ms = duration_ms;
        unit.last_spell = spell_id;
        // **The half that says this begin came *after* a release**, which is the
        // whole of why a channel is drawn at all — see
        // [`Entity::casts_channelled`].
        unit.casts_channelled = unit.casts_channelled.wrapping_add(1);
    }

    /// `MSG_CHANNEL_UPDATE` — how much of the channel is left, in milliseconds.
    ///
    /// Zero is the server saying it is over, which is what an interrupted
    /// Evocation sends. Anything else re-states the remaining time without
    /// restarting the pose, so it is **not** a new cast: the counter is left
    /// alone and only the clock moves.
    pub fn apply_channel_update(&mut self, remaining_ms: u32) {
        let Some(guid) = self.player_guid else {
            return;
        };
        let Some(unit) = self.entities.get_mut(&guid) else {
            return;
        };
        if remaining_ms == 0 {
            unit.casts_released = unit.casts_released.wrapping_add(1);
        }
        unit.cast_time_ms = remaining_ms;
    }

    /// Who a chat line is from, resolved as far as this client can.
    ///
    /// **A player's line carries a GUID and no name** (`BuildChatPacket` writes
    /// one only for the monster types), on the assumption that the client already
    /// knows who that is — true of anyone in view, and false of a guild member on
    /// another continent. So the chain is: the name in the packet, then the
    /// `CMSG_NAME_QUERY` cache, then the entity if it is one we can see, and
    /// finally the GUID itself. Never blank, and never a lie.
    pub fn chat_sender(&self, message: &crate::play::chat::ChatMessage) -> String {
        if let Some(name) = &message.sender_name {
            return name.clone();
        }
        if message.sender == 0 {
            return String::new();
        }
        if let Some(info) = self.players.get(&message.sender) {
            return info.name.clone();
        }
        self.entities
            .get(&message.sender)
            .map(|e| self.name_of(e))
            .unwrap_or_else(|| format!("guid {:#x}", message.sender))
    }

    /// Record one melee swing.
    ///
    /// Both ends are counted, because both animate: the attacker swings and the
    /// victim flinches. A blow that missed still swings — that is the whole
    /// point of a miss being visible — so only the flinch is conditional, and
    /// the condition is the server's own (`HITINFO_AFFECTS_VICTIM`, whose
    /// comment in `UnitDefines.h` reads "no being hit animation on victim
    /// without it").
    ///
    /// Neither unit is created if it is not already known: a swing landing on
    /// something out of sight is not a reason to invent an entity with no
    /// position, which would be drawn at the centre of the map.
    pub fn apply_attack(&mut self, attack: &crate::play::action::AttackUpdate) {
        use crate::play::action::victim_state;
        if let Some(attacker) = self.entities.get_mut(&attack.attacker) {
            attacker.swings_thrown = attacker.swings_thrown.wrapping_add(1);
            attacker.last_swing_info = attack.hit_info;
            // **The victim state goes on both ends**, because the two ends read
            // it for two different questions: the victim's picks its reaction
            // animation, the attacker's picks which column of its weapon's
            // sound row the blow lands in.
            attacker.last_swing_state = attack.victim_state;
            attacker.last_swing_victim = attack.victim;
            attacker.last_swing_damage = attack.damage;
        }
        // **A dodge, a parry and a block are not "hits", and they are exactly
        // the reactions worth drawing.** `hit_the_victim` gates on
        // `HITINFO_AFFECTS_VICTIM`, which the server does not set for a blow
        // that never connected — so a victim counter driven by it alone leaves
        // a character standing perfectly still through everything they
        // successfully defended against, which is most of a fight.
        let defended = matches!(
            attack.victim_state,
            victim_state::DODGE | victim_state::PARRY | victim_state::BLOCKS
        );
        if attack.hit_the_victim() || defended {
            if let Some(victim) = self.entities.get_mut(&attack.victim) {
                victim.blows_taken = victim.blows_taken.wrapping_add(1);
                victim.last_victim_state = attack.victim_state;
                victim.last_blow_info = attack.hit_info;
            }
        }
        // **The floating number's own channel, and it is the *victim's*.**
        // Separate from `blows_taken` above, which is an animation counter and
        // is gated on the blow having connected: a miss and a dodge draw a word
        // and must reach the reader. Gated on the *attacker* instead, which is
        // the reference's own refusal — see [`Entity::damage_taken`].
        if self.player_guid == Some(attack.attacker) {
            if let Some(victim) = self.entities.get_mut(&attack.victim) {
                victim.damage_taken = victim.damage_taken.wrapping_add(1);
                victim.last_damage = attack.damage;
                victim.last_damage_info = attack.hit_info;
                victim.last_damage_state = attack.victim_state;
                victim.last_damage_spell = None;
                victim.healed = false;
            }
        }
    }

    /// `SMSG_SPELLNONMELEEDAMAGELOG` — **a spell landed on somebody**, or a
    /// damage-over-time ticked.
    ///
    /// Recorded on the same channel a swing is, for [`Entity::damage_taken`]'s
    /// reason, and under the same gate: the reference's producer refuses unless
    /// the source is the player or their pet, so a fight across the
    /// field raises nothing.
    ///
    /// **A full absorb or resist still counts.** The damage is zero and the
    /// flags say why, which is a *word* rather than a number — dropping it here
    /// would silently lose every "Immune" and every "Resist" a spell draws.
    pub fn apply_spell_damage(&mut self, log: &crate::play::action::SpellDamage) {
        if self.player_guid != Some(log.caster) {
            return;
        }
        let Some(victim) = self.entities.get_mut(&log.victim) else {
            return;
        };
        victim.damage_taken = victim.damage_taken.wrapping_add(1);
        victim.last_damage = log.damage;
        victim.last_damage_info = log.hit_info;
        victim.last_damage_state = 0;
        victim.last_damage_spell = Some(log.spell_id);
        victim.healed = false;
    }

    /// `SMSG_SPELLHEALLOG` — **a heal landed on somebody**, which is a number
    /// over their head like any other.
    ///
    /// The same channel again, with the amount as the damage and the spell
    /// named. What tells the reader it is a heal is [`Entity::healed`]: this is
    /// the only door that sets it.
    pub fn apply_spell_heal(&mut self, log: &crate::play::action::SpellHeal) {
        if self.player_guid != Some(log.healer) {
            return;
        }
        let Some(victim) = self.entities.get_mut(&log.victim) else {
            return;
        };
        victim.damage_taken = victim.damage_taken.wrapping_add(1);
        victim.last_damage = log.amount;
        victim.last_damage_info = if log.critical {
            crate::play::action::spell_hit::CRIT
        } else {
            0
        };
        victim.last_damage_state = 0;
        victim.last_damage_spell = Some(log.spell_id);
        victim.healed = true;
    }

    /// Record an AI reaction against the creature that had it.
    ///
    /// **Not created if it is not already known**, for the same reason a swing
    /// is not: `SMSG_AI_REACTION` is broadcast to everyone in sight of the
    /// creature, which is a larger set than everyone the creature is in sight
    /// of, and a bark from an unplaced unit would be voiced at the map origin.
    pub fn apply_ai_reaction(&mut self, reaction: &crate::play::action::AiReaction) {
        if let Some(unit) = self.entities.get_mut(&reaction.guid) {
            unit.reactions = unit.reactions.wrapping_add(1);
            unit.last_reaction = reaction.reaction;
        }
    }

    /// Record a one-shot emote against the unit that played it.
    pub fn apply_emote(&mut self, emote: &crate::play::action::Emote) {
        if let Some(unit) = self.entities.get_mut(&emote.guid) {
            unit.emotes = unit.emotes.wrapping_add(1);
            unit.last_emote = emote.emote_id;
        }
    }

    /// **A `SpellVisualKit` the server wants played on a unit** —
    /// `SMSG_PLAY_SPELL_VISUAL` when `impact` is false, `SMSG_PLAY_SPELL_IMPACT`
    /// when it is. See [`crate::play::sound`] for the two bodies and the two
    /// callers that are not a GM command.
    ///
    /// **Unlike every other applier here it does not create the entity**, and
    /// that is the reference's own behaviour rather than a shortcut: vmangos'
    /// comment on the sibling packet — *"ignored by client if unit is not
    /// loaded"* — says the 1.12 client drops a visual for a guid it has never
    /// heard of. There is nowhere to draw it and nothing to attach it to, and
    /// an entity conjured from a kit id would be a unit with no position, no
    /// model and no create block, which the renderer would have to filter out
    /// again one layer up. Matches [`Self::apply_emote`], which is the same
    /// shape of statement.
    pub fn apply_spell_visual(&mut self, guid: u64, kit: u32, impact: bool) {
        let Some(unit) = self.entities.get_mut(&guid) else {
            return;
        };
        if impact {
            unit.spell_impacts = unit.spell_impacts.wrapping_add(1);
            unit.last_spell_impact = kit;
        } else {
            unit.spell_visuals = unit.spell_visuals.wrapping_add(1);
            unit.last_spell_visual = kit;
        }
    }

    /// **A cast this unit was doing is over without having landed** — refused,
    /// interrupted, or cancelled by the player.
    ///
    /// See [`Entity::casts_cancelled`] for why this is a counter of its own
    /// rather than a rollback of the two the press moved, and for the three
    /// packets that reach it.
    ///
    /// **It is now only ever about art the *server* started**, which is what
    /// makes the guard below the whole of it: since this client stopped drawing
    /// its own cast at the press (see [`Self::apply_cast`]), a refusal that
    /// arrives before any `SMSG_SPELL_START` has nothing to take down, and
    /// `unit.last_spell` still naming the previous cast is exactly the case the
    /// guard exists for.
    pub fn apply_cast_cancelled(&mut self, caster: u64, spell_id: u32) {
        let Some(unit) = self.entities.get_mut(&caster) else {
            return;
        };
        // **Only about the cast that is actually up.** A failure for a spell
        // this unit is not showing is an answer to something already finished —
        // a refusal that arrives after the release has been drawn and adopted —
        // and taking the art down on it would cut off whatever *is* playing.
        if unit.last_spell != spell_id {
            return;
        }
        unit.casts_cancelled = unit.casts_cancelled.wrapping_add(1);
    }

    /// Record a cast beginning (`start`) or being released.
    ///
    /// Both are counters for the same reason a swing is: the renderer polls,
    /// and "has anything happened since I last looked" is the only question a
    /// clockless world state can answer.
    ///
    /// **The local player is in here on the same terms as everybody else**, and
    /// that is a rule this client got wrong for several rounds. There used to be
    /// a `predict_own_cast` beside this, bumping both counters at the *press* so
    /// the arm moved without waiting a round trip, with an echo-swallowing slot
    /// to stop the server's answer playing it twice. 5875 does not do that:
    /// `SPELLCAST_START` is raised in exactly one place and that
    /// place is `SMSG_SPELL_START`'s handler, vmangos comments its own
    /// `SendSpellStart()` with `// will show cast bar`, and the press path
    /// sets a pending record, starts the global cooldown and sends —
    /// and draws nothing. Predicting it meant a refused cast played a wind-up
    /// and a release that never happened, which is what "the animation plays but
    /// the spell was not really cast" is.
    pub fn apply_cast(&mut self, cast: &crate::play::action::SpellCast, start: bool) {
        let Some(unit) = self.entities.get_mut(&cast.caster) else {
            return;
        };
        // Written for both halves, because both need it: the wind-up and the
        // release are two animations off the same spell's visual.
        unit.last_spell = cast.spell_id;
        if start {
            // The server's own `m_timer` is authoritative for how long the pose
            // is held, whether or not the animation was already played — the
            // press only had the spell's base cast time to go on.
            unit.cast_time_ms = cast.cast_time_ms;
            // The wind-up carries no hit list at all, so writing the target for
            // both would clear it the moment the next spell started.
            unit.casts_begun = unit.casts_begun.wrapping_add(1);
            return;
        }
        unit.last_spell_target = cast.target();
        unit.last_spell_targets.clear();
        unit.last_spell_targets.extend_from_slice(&cast.hits);
        // …and the same spell into the ring, so that a second release arriving
        // before the renderer next looks does not take the first one's art with
        // it. See [`Entity::recent_spells`], where Charge is the measurement.
        unit.recent_spells.rotate_left(1);
        unit.recent_spells[RECENT_SPELLS - 1] = cast.spell_id;
        // Two counters for one packet, and they answer different questions —
        // see [`Entity::casts_landed`], which is what the *hit* list is read on.
        unit.casts_landed = unit.casts_landed.wrapping_add(1);
        unit.casts_released = unit.casts_released.wrapping_add(1);
    }

    /// `SMSG_SPELL_DELAYED`: the cast in progress was knocked back.
    ///
    /// **The one packet that restates a cast's length after it has begun.**
    /// vmangos' `Spell::Delayed` adds `GetNextDelayAtDamageMsTime()` to
    /// `m_timer` when the caster takes damage — 500 ms, then 1,000, then 3,000
    /// — and sends this to the caster alone. Everything the client is holding
    /// for that cast is running on a clock started by `SMSG_SPELL_START`'s
    /// `m_timer`, so without this the wind-up pose, the art on the caster's
    /// hands and the bar all end while the server is still casting: the "stuck
    /// casting a spell that was interrupted" report is that gap seen from the
    /// other side.
    ///
    /// A counter rather than a new absolute length, because the packet is a
    /// *difference* and the consumers each hold their own deadline — see
    /// [`Entity::casts_delayed`].
    pub fn apply_cast_delayed(&mut self, guid: u64, delay_ms: u32) {
        let Some(unit) = self.entities.get_mut(&guid) else {
            return;
        };
        unit.casts_delayed = unit.casts_delayed.wrapping_add(1);
        unit.last_cast_delay_ms = delay_ms;
        // The server's own `m_timer` is what the wind-up was armed with, so the
        // restated length is kept here too: anything that reads the field after
        // this reads the cast's true remaining shape rather than its original.
        unit.cast_time_ms = unit.cast_time_ms.saturating_add(delay_ms);
    }

    /// Apply a `MSG_MOVE_*` broadcast: another player moved.
    ///
    /// The position in the block is authoritative *as of that packet*, and the
    /// flags say what they are doing next, which [`Self::advance`] then
    /// dead-reckons. A player's own broadcast never comes back to them
    /// (`SendMovementMessageToSet` excludes the sender), but the guard is cheap
    /// and the alternative — the server's half-second-old position fighting the
    /// live simulation — would be a jitter that is miserable to diagnose.
    pub fn apply_movement(&mut self, guid: u64, info: &MovementInfo) {
        if Some(guid) == self.player_guid {
            return;
        }
        let entity = self
            .entities
            .entry(guid)
            .or_insert_with(|| Entity::new(guid));
        entity.movement = Some(*info);
        // The phase of any arc in flight, restated by every packet that carries
        // the flag — see `Entity::fall_secs`.
        entity.fall_secs = info.fall_time as f32 / 1000.0;
        let moved = entity.set_server_position(info.position);
        // A player packet supersedes any spline the server had this unit on.
        entity.spline = None;
        match moved {
            Some(moved) => self.note_jump(guid, moved, "MSG_MOVE_* broadcast"),
            None => {
                // A garbage position here is the more dangerous kind: the flags
                // stay, so `advance` would dead-reckon *onwards* from nowhere.
                if let Some(entity) = self.entities.get_mut(&guid) {
                    entity.movement = None;
                }
                self.rejected_positions = self.rejected_positions.saturating_add(1);
            }
        }
    }

    /// **One of the twelve `SMSG_SPLINE_MOVE_*` packets** — the server stating a
    /// movement flag about a unit it controls.
    ///
    /// See [`crate::state::movement::SplineFlagChange`] for which opcode says
    /// what, and why this family exists beside the two that already did.
    ///
    /// **It creates the entity if it has not been seen.** Every other applier
    /// here does, and for the same reason: the ordering between an update block
    /// and a state packet about the same guid is not guaranteed, and a root
    /// dropped because the create block had not landed yet would never be
    /// restated.
    ///
    /// **The local player is not excluded.** The server sends these only about
    /// units no player is moving, so one naming us cannot arrive — but if one
    /// did it would be a statement about us worth keeping, unlike
    /// [`Self::apply_movement`], whose position we own and must not be given
    /// back a stale copy of.
    pub fn apply_spline_flag(&mut self, guid: u64, change: crate::state::movement::SplineFlagChange) {
        let entity = self
            .entities
            .entry(guid)
            .or_insert_with(|| Entity::new(guid));
        entity.forced_flags.state(change);
    }

    /// Advance every in-flight spline by `dt_ms`, and dead-reckon every entity
    /// whose last movement block said it was moving.
    ///
    /// A spline is walked along its own path at constant speed
    /// ([`Spline::position_and_heading`]) rather than straight from end to end;
    /// the client is not authoritative, but cutting the corner of a patrol route
    /// puts a creature visibly off the path it was told to walk. The players are
    /// dead-reckoned instead: their heartbeats are 500 ms apart, so without
    /// extrapolation everyone else teleports twice a second.
    ///
    /// ## `world` is the same ground the local character walks on, and it is
    /// not an optimisation
    ///
    /// The dead reckoning here reproduces a stride that **another client has
    /// already taken**, and that client ran it through its own collision: a
    /// player holding forward against a wall stops, and their heartbeats go on
    /// saying so. Extrapolating the same flags in a straight line is therefore
    /// not "a small error the next packet corrects" — it is simulating a
    /// different client, and the disagreement is unbounded in the one place it
    /// is most visible. Half a second of run speed is 3.5 yards, so the observed
    /// player walks *through* the wall and is yanked back on the heartbeat,
    /// twice a second, for as long as they keep pushing at it. That is the "they
    /// appear to constantly teleport thru it and back" report verbatim, and the
    /// same argument covers the ground under them: a runner going up a hill has
    /// their height re-stated by their own client every stride and by ours never.
    ///
    /// So the stride is *proposed* and the world decides where it ends, in
    /// exactly the order [`crate::state::movement::Mover::advance`] does it for the
    /// local character — the wall first, then the floor at the end of the stride
    /// that actually happened. `None` is the CLI's snapshot pump and the
    /// renderer before its first tile, and gives the straight-line behaviour
    /// this had before.
    ///
    /// **What is deliberately not asked**: a spline is left alone (the server
    /// authored that path over its own geometry and is restating it), a
    /// swimmer's height is the water's rather than the floor's, and an entity
    /// that walks off a ledge is *held* rather than dropped — nothing here owns
    /// their arc, they never announced one, and the server will say so within
    /// the half-second. Only the population that is genuinely being invented
    /// here is corrected.
    ///
    /// **A step longer than a tick is sliced by the caller, not clamped here.**
    /// `dt_ms` is walked as one stride — the wall test, the floor lookup and the
    /// arc all happen once for it — so handing this the whole of a five-second
    /// stall would invent a thirty-eight-yard step through a `Footing::step`
    /// written for 0.2-yard ones. `socket::session`'s `tick_world` calls this
    /// repeatedly in tick-sized slices instead, which is exactly what an
    /// unstalled thread would have done and needs no rule here.
    ///
    /// **It runs under the world lock, and the population is small on purpose.**
    /// The two queries are asked once per *moving dead-reckoned* entity per
    /// step, which is other players actively walking in view and nothing else —
    /// a creature is on a spline, and a standing unit is skipped before either
    /// question is reached. The lock order is one-way (this lock, then the
    /// terrain's and the collision world's, neither of which can see an
    /// `ObjectManager`), so there is no cycle to deadlock on; what is *not*
    /// measured is what a crowded capital costs here, and the honest place to
    /// notice it would be the session thread's step time.
    pub fn advance(&mut self, dt_ms: u32, world: Option<&dyn crate::state::movement::Footing>) {
        let dt = dt_ms as f32 / 1000.0;
        let player_guid = self.player_guid;

        for entity in self.entities.values_mut() {
            // **The platforms, which are the one population here with no packet
            // behind them.** Advanced at the top of this walk rather than in one
            // of its own, and *before* every guard below, because none of those
            // guards applies: a transport has no movement block, no speeds and
            // no spline, so it would `continue` past all of them. A separate
            // pass measured +3.0 us a step over 2,000 entities, which is one
            // more walk of the map for one `Option` test.
            //
            // Unclamped, on this function's own terms — it is the world's clock
            // and the server advanced all of it. The **wrap** is the reader's
            // and is taken by
            // `vale_assets::tables::transport::Transports::offset_at`, so it
            // cannot be got wrong in two places.
            if let Some(phase) = entity.transport_phase_ms.as_mut() {
                *phase = phase.saturating_add(u64::from(dt_ms));
            }
            // The live session owns the player's position; it dead-reckons the
            // same way and writes the result back here every tick.
            if Some(entity.guid) == player_guid || entity.spline.is_some() {
                continue;
            }
            let Some(info) = entity.effective_movement() else {
                continue;
            };
            // **Rooted units are not dead-reckoned**, and the guard is here
            // rather than left to the moving flags because the two arrive on
            // different packets and in either order. The server clears the
            // moving bits itself when it roots somebody it controls
            // (`SetRooted` calls `StopMoving` first), so for a player this is
            // usually redundant — but a root that lands *between* a start
            // packet and its heartbeat has nothing to clear them, and the
            // reckoning would walk the unit on for half a second under the
            // spell that stopped it.
            if info.has(move_flags::ROOT) || !info.is_moving() {
                continue;
            }
            // **In the air is a different simulation, not a faster one.** The
            // horizontal velocity is the one frozen at take-off (the flags say
            // nothing about it — a player who jumps while running forward and
            // lets go of W is still travelling), and the height is the parabola
            // `Unit::ExtrapolateMovement` walks. The alternative is what this
            // client did until now: run speed along the facing, with z held at
            // the take-off height until the next heartbeat corrected it.
            if info.has(move_flags::JUMPING) {
                let t0 = entity.fall_secs;
                // The server refuses to extrapolate an arc past ten seconds and
                // neither do we: past that the landing packet has been lost and
                // guessing further only buries the unit.
                if t0 > MAX_ARC_SECS {
                    continue;
                }
                let t1 = t0 + dt;
                let safe = info.has(move_flags::SAFE_FALL);
                // The jump block's `zspeed` is already in `fall_elevation`'s
                // own frame — down-positive, so a jump arrives as -7.9558 —
                // because the wire value *is* the fall's start velocity
                // (`Unit::KnockBack` sends `-verticalSpeed` into this field).
                // Negating it here is what drove every real client's jumper
                // straight down through the floor.
                let drop = fall_elevation(t1, safe, info.jump.z_speed)
                    - fall_elevation(t0, safe, info.jump.z_speed);
                entity.fall_secs = t1;
                if let Some(position) = entity.position.as_mut() {
                    let (dx, dy) = (
                        info.jump.cos_angle * info.jump.xy_speed * dt,
                        info.jump.sin_angle * info.jump.xy_speed * dt,
                    );
                    let [x, y] = clipped(world, *position, dx, dy);
                    position.x = x;
                    position.y = y;
                    // The arc's own height and nothing else: the jumper's client
                    // is on this parabola too, and a floor lookup here would
                    // stand them on the roof they are sailing over.
                    position.z -= drop;
                }
                continue;
            }
            let speeds = entity.speeds.unwrap_or_default();
            let speed = info.speed(&speeds);
            let heading = info.heading();
            let swimming = info.has(move_flags::SWIMMING);
            if let Some(position) = entity.position.as_mut() {
                let [x, y] = clipped(world, *position, heading.cos() * speed * dt, heading.sin() * speed * dt);
                position.x = x;
                position.y = y;
                // A swimmer's height is the water's business — their own client
                // is floating them at a depth this one has no packet for — and
                // the ground under a lake is not where they are.
                if !swimming {
                    if let Some(h) = world.and_then(|w| w.floor(x, y, position.z)) {
                        // **Up a slope, never off a ledge.** A drop is left for
                        // the server to state: nothing here owns their fall,
                        // they never announced one, and inventing an arc would
                        // put them under a bridge they are running across the
                        // moment the hull below it answers first.
                        if h > position.z - FALL_THRESHOLD {
                            position.z = h;
                        }
                    }
                }
            }
        }

        for entity in self.entities.values_mut() {
            let Some(spline) = entity.spline.as_mut() else {
                continue;
            };
            // The whole of the step, unclamped: this is the server's own path
            // and the server walked all of it. See the note at the top.
            spline.elapsed_ms = spline.elapsed_ms.saturating_add(dt_ms);
            // A cyclic path wraps rather than ending — see `Spline::wrap`. A
            // step longer than a whole lap would need more than one wrap, which
            // only happens if the thread stalled for the length of a patrol.
            while spline.wrap() {}
            // **In world coordinates, which for a passenger's spline is not
            // where the path says.** `SMSG_MONSTER_MOVE_TRANSPORT` states the
            // path in the transport's own frame; `in_world` is the conversion
            // and answers `None` for a transport this client is not holding, in
            // which case the unit is left where it was rather than placed at the
            // map's origin. See [`crate::state::movement::Spline::in_world`].
            let Some((position, heading)) = spline.in_world(world) else {
                continue;
            };
            let finished = spline.finished();

            // A unit faces the leg it is walking, and on arrival turns to
            // whatever the packet asked for. The `Target` case is a creature
            // that has just run up to you and looks at you — resolved when the
            // packet arrived, since only the manager can see another entity.
            let orientation = if finished {
                match spline.facing {
                    SplineFacing::Angle(a) => a,
                    SplineFacing::Spot(spot) => bearing(position, spot),
                    SplineFacing::Target(_) => match entity.facing_target {
                        Some(at) => bearing(position, at),
                        None => heading,
                    },
                    SplineFacing::Travel => heading,
                }
            } else {
                heading
            };

            // **Put the leg back on the ground it crosses.** The server
            // states a straight line in three dimensions between two points it
            // grounded; the hill between them is not straight, and the drawn
            // creature floats over the hollows and wades through the rises.
            // See [`Spline::grounded_z`], which is where the rule and the
            // measurement behind it are — and which answers `None` for every
            // spline whose vertical is not the ground's, so a flight, a
            // knock-back and a map with no terrain loaded all keep the chord.
            let z = world
                .and_then(|world| spline.grounded_z(world, position))
                .unwrap_or(position[2]);
            entity.position = Some(Position {
                x: position[0],
                y: position[1],
                z,
                orientation,
            });
            if finished {
                entity.spline = None;
            }
        }

        self.stand_on_the_ground(world);
        self.face_targets(dt);
    }

    /// **Put a standing unit on whatever is under it**, once per statement the
    /// server makes about where it is.
    ///
    /// ## The report, and what was measured before anything was written
    ///
    /// *"Many NPCs appear to float. Example: Peltskinner questgiver in
    /// Northshire."* Eagan Peltskinner is `creature.guid` 79971, a spawn with
    /// `movement_type = 0` and `wander_distance = 0` — he never moves, so
    /// [`crate::state::movement::Spline::grounded_z`] cannot reach him. Four
    /// measurements, none of them from a picture:
    ///
    /// * the world database spawns him at **z = 80.9719**, and vmangos'
    ///   `Creature::LoadFromDB` relocates an alive DB spawn to that number
    ///   verbatim — `CreatureCreatePos::SelectFinalPoint` is a no-op without a
    ///   `m_closeObject`. So that is what crosses the wire.
    /// * this client's terrain answers **80.2051** there, and vmangos' own
    ///   extracted `0004832.map`, decoded by hand through
    ///   `GridMap::getHeightFromUint16`, answers **80.2051** as well. The two
    ///   sides of the wire agree about the ground to four decimal places.
    /// * `vale collision Azeroth 32 48 -8869.22 -163.237 80.9719` says
    ///   nothing is under him: no building hull, no doodad hull, and the nearest
    ///   drawn prop is a barrel 1.6 yards away sitting on the ground at 80.24.
    /// * so he stands **0.767 yards in the air over bare grass**, and the
    ///   client was drawing exactly what it was told.
    ///
    /// The mode of the whole population is the same story one notch smaller:
    /// over the 14,193 stationary ground spawns on maps 0 and 1, the commonest
    /// gap is **+0.18**, which is the epsilon whatever generated the table
    /// lifted every spawn by. About 7% are more than 0.3 above the terrain.
    ///
    /// ## …and the reference puts him down, which was measured rather than read
    ///
    /// The first draft of this shipped as a *labelled deviation*, on the
    /// grounds that nothing in the client could be found that grounds a unit:
    /// its own ground query is reached only from map and camera code.
    ///
    /// **That reading was wrong, and the experiment that settled it is the
    /// cheap one this project keeps forgetting it can run**: log into the same
    /// vmangos server with the 1.12.1 client and look. Eagan Peltskinner stands on
    /// the grass in the reference, receiving the same 80.9719 over the same
    /// wire. So the client does ground a standing unit; what has not been found
    /// is *where*, and until it is, the rule below is a reconstruction of a
    /// behaviour that is known to exist rather than an invention.
    ///
    /// The bound stays, and its reason changes with it. It is not there because
    /// the reference might not do this — it is there because **this client's
    /// collision is less complete than the reference's**: 25 of the 75 doodad
    /// models on Northshire's tile carry no hull at all, so a unit the server
    /// stood on something this client walks through has nothing to be compared
    /// against. Past the band, "the server meant it" is the safer reading.
    ///
    /// [`STAND_BAND`] and the exclusions are the rest of the design:
    ///
    /// * **only a unit that is standing** — no spline, no movement flags that
    ///   say it is moving, not swimming, not airborne. Everything that moves is
    ///   already handled where it moves.
    /// * **only downwards, and only within the band.** A unit the server has
    ///   put more than a band above what is under it is somewhere on purpose —
    ///   a flying creature, a platform this client has no hull for, a boss on a
    ///   dais — and is left alone. Nothing is ever lifted.
    /// * **[`crate::state::movement::Footing::floor`]'s own answer**, so a unit
    ///   on a building's floor is compared against *that* floor and not against
    ///   the terrain under the building; and a building that has not streamed in
    ///   yet answers `None`, which leaves the unit where the server put it.
    /// * …**and what the unit's own movement flags make of it**, which is
    ///   [`crate::state::movement::standing_surface`]: a water-walker's surface
    ///   is the water and a hovering unit's is a yard over whichever of the two
    ///   it stands on. Both were state this client held correctly and drew
    ///   nothing for, and for a *standing* unit the old rule was worse than
    ///   nothing — it pulled a water-walker down to the lake bed.
    ///
    /// The one case it gets wrong is stated rather than hidden: a unit standing
    /// on a **hull-less prop** — a crate, a haystack, a rock the game means to
    /// be walked through — is sunk to the ground under it. That prop is one the
    /// player also walks through in this client, so the unit was already
    /// standing on nothing as far as everything else here is concerned.
    ///
    /// **Cost: one [`Footing::floor`] per entity per *server position
    /// statement***, not per step. A standing creature is stated once and then
    /// never again, so a city's idle population pays this exactly once each —
    /// which is why [`Entity::grounded_for`] is a latch on
    /// [`Entity::position_updates`] rather than a bool.
    fn stand_on_the_ground(&mut self, world: Option<&dyn crate::state::movement::Footing>) {
        let Some(world) = world else { return };
        // The beat is what bounds the *unanswered* population — see
        // [`STAND_BEAT`]. An entity that has been put down costs one comparison
        // whatever this does, so the gate is about the login burst and nothing
        // else.
        self.stand_beat = self.stand_beat.wrapping_add(1);
        if !self.stand_beat.is_multiple_of(STAND_BEAT) {
            return;
        }
        let player_guid = self.player_guid;
        for entity in self.entities.values_mut() {
            // The live session owns the player's height and runs a whole mover
            // over it, arcs and all.
            if Some(entity.guid) == player_guid || entity.spline.is_some() {
                continue;
            }
            if entity.grounded_for == Some(entity.position_updates) {
                continue;
            }
            // **The two flags that change what "the ground" means**, kept from
            // the same accessor everything else reads — see
            // [`crate::state::movement::standing_surface`]. Zero for a unit the
            // server has said nothing about, which is neither of them.
            let mut flags = 0;
            if let Some(info) = entity.effective_movement() {
                if info.is_moving()
                    || info.has(move_flags::SWIMMING)
                    || info.has(move_flags::JUMPING)
                {
                    continue;
                }
                flags = info.flags & (move_flags::WATERWALKING | move_flags::HOVER);
            }
            let Some(position) = entity.position.as_mut() else {
                continue;
            };
            // **Latched only on an answer.** A login burst creates every
            // entity in view before the tiles under them have streamed, so
            // latching a `None` would leave that whole population floating for
            // the rest of the session — the one failure mode a latch must not
            // have. The retry is paced by [`STAND_BEAT`] instead.
            //
            // **The liquid is asked only of the units that could care**, which
            // is the near-zero fraction carrying `MOVEFLAG_WATERWALKING`: a
            // second world query per entity per statement for a flag almost
            // nothing in the game sets would double the cost of this pass for
            // nobody.
            let water = match flags & move_flags::WATERWALKING {
                0 => None,
                _ => world.liquid(position.x, position.y),
            };
            let floor = world.floor(position.x, position.y, position.z);
            let Some(surface) = crate::state::movement::standing_surface(flags, floor, water)
            else {
                continue;
            };
            entity.grounded_for = Some(entity.position_updates);
            let gap = position.z - surface;
            if gap > STAND_EPSILON && gap <= STAND_BAND {
                position.z = surface;
            }
        }
    }

    /// Turn every stationary creature towards what it is attacking.
    ///
    /// **Nothing on the wire asks for this, and that is the point.** A creature
    /// chasing a player faces the way it is walking, which is roughly at them —
    /// and then the chase spline finalises and `TargetedMovementGenerator`
    /// calls `owner.SetInFront(i_target.getTarget())`, which is
    /// `SetOrientation` and *nothing else*. No packet, no spline, no field.
    /// Without this pass a mob is left facing wherever its last spline parked
    /// it, and a player who strafes round it is beaten up by its shoulder —
    /// which reads as an animation or an interpolation fault and is neither.
    ///
    /// vmangos states the contract twice, and never sends anything to honour
    /// it. `TotemAI` writes it beside the call —
    /// `SetInFront(victim); // client change orientation by self` — and
    /// `Object::BuildValuesUpdate` writes it beside the *field*, where a
    /// creature that is casting has its casting target substituted into
    /// `UNIT_FIELD_TARGET` for exactly this reason: *"This is done to make
    /// creatures face the target they are casting on."* The server is
    /// describing what it expects the receiver to do with the number.
    ///
    /// Measured on a live vmangos, `vale live` → `goto`: a Carrion Lurker
    /// finished its chase spline **169 degrees off** the bearing to the player
    /// standing 0.2 yards from it, and stayed there. With this pass it turns
    /// through those 169 degrees in the 0.9 s the turn rate allows and holds at
    /// zero.
    ///
    /// **Creatures only.** A player's orientation arrives for real, in their own
    /// `MSG_MOVE_*` broadcasts, and vmangos sends a *player* a genuine
    /// `SetFacingTo` spline where it sends a creature nothing
    /// (`TargetedMovementGenerator.cpp`, the two branches either side of the
    /// same `Finalized()` check). Turning them here would fight the packets.
    ///
    /// **And stationary only**, for the same reason `apply_monster_move` puts
    /// travel first: a unit walks forwards, so while it is moving the leg it is
    /// walking is the answer and its target is not.
    fn face_targets(&mut self, dt: f32) {
        // Copied out first, because turning one entity needs another's
        // position and `values_mut` holds the map — the same reason
        // `facing_target` is resolved in `apply_monster_move` rather than here.
        let mut turns: Vec<(u64, [f32; 3])> = Vec::new();
        for entity in self.entities.values() {
            if entity.object_type != Some(ObjectType::Unit)
                || entity.is_moving()
                || !entity.is_alive()
            {
                continue;
            }
            let Some(target) = entity.target_guid() else {
                continue;
            };
            // A target out of sight is a target this client cannot place, and
            // guessing would be worse than leaving the creature as it is.
            if let Some(at) = self.entities.get(&target).and_then(|e| e.position) {
                turns.push((entity.guid, [at.x, at.y, at.z]));
            }
        }

        for (guid, at) in turns {
            let Some(entity) = self.entities.get_mut(&guid) else {
                continue;
            };
            let Some(position) = entity.position else {
                continue;
            };
            let wanted = bearing([position.x, position.y, position.z], at);
            let delta = crate::state::movement::shortest_turn(position.orientation, wanted);
            // Rate-limited rather than snapped, and the rate is the server's own
            // `MOVE_TURN_RATE` — pi rad/s by default, sent in every movement
            // block, so a unit hasted or slowed to turn faster does. A snap
            // would arrive as a single 180-degree pop: `Motion` interpolates
            // facing, but only across one 25 ms simulation step.
            let limit = entity.speeds.unwrap_or_default().turn_rate() * dt;
            let step = delta.clamp(-limit, limit);
            if let Some(position) = entity.position.as_mut() {
                position.orientation = wrap_angle(position.orientation + step);
            }
        }
    }

    /// Fold one parsed update into the world state.
    pub fn apply(&mut self, update: &ObjectUpdate) {
        // Counted up here and added once at the end: the entity is mutably
        // borrowed out of `self.entities` inside the loop.
        let mut rejected = 0u32;
        let mut jumped: Vec<(u64, f32, &'static str)> = Vec::new();
        // …and the same for the inventory latch, since the block that moves it
        // is often the same block that created the object it is about.
        let mut inventory_moved = false;
        for block in &update.blocks {
            match block {
                UpdateBlock::Create {
                    guid,
                    object_type,
                    movement,
                    values,
                    ..
                } => {
                    let entity = self
                        .entities
                        .entry(*guid)
                        .or_insert_with(|| Entity::new(*guid));
                    entity.object_type = *object_type;
                    if let Some(position) = movement.position {
                        match entity.set_server_position(position) {
                            Some(moved) => jumped.push((*guid, moved, "a create block")),
                            None => rejected += 1,
                        }
                    }
                    // A creature that streams into view mid-walk is walking, and
                    // the create block's inline spline is the only thing that
                    // says so — `SMSG_MONSTER_MOVE` went out when the move
                    // *started*, to whoever could see it then. Adopt it with the
                    // server's own `timePassed`, so the unit picks the path up
                    // where the server already is rather than at its start.
                    entity.adopt_spline(movement.spline.as_ref());
                    // **The one word that says anything about an elevator.**
                    // Restated rather than accumulated: a fresh block is the
                    // server's own reading of the phase, so it replaces
                    // whatever this side had drifted to. See
                    // [`Entity::transport_phase_ms`].
                    if let Some(progress) = movement.path_progress {
                        entity.transport_phase_ms = Some(u64::from(progress));
                    }
                    if let Some(speeds) = movement.speeds {
                        entity.speeds = Some(Speeds(speeds));
                    }
                    for (index, value) in &values.fields {
                        entity.fields.insert(*index, *value);
                    }
                    // …and what it did *not* carry is zero rather than unknown —
                    // see [`Entity::created`]. Set after the fields, so the flag
                    // and the values it qualifies are never out of step.
                    entity.created = true;
                    // **A create block is the only way a new entry arrives**,
                    // so it is one of the two things that make the query pass
                    // worth running — see [`Self::take_query_hint`].
                    self.queries_wanted = true;
                    if movement.is_self() {
                        entity.is_self = true;
                        self.player_guid = Some(*guid);
                    }
                    inventory_moved |= touches_inventory(entity, &values.fields);
                }

                UpdateBlock::Values { guid, values } => {
                    // A values update can arrive for an object we never saw
                    // created (e.g. we joined mid-stream); tracking it anyway is
                    // strictly better than dropping the data.
                    let entity = self
                        .entities
                        .entry(*guid)
                        .or_insert_with(|| Entity::new(*guid));
                    for (index, value) in &values.fields {
                        entity.fields.insert(*index, *value);
                    }
                    inventory_moved |= touches_inventory(entity, &values.fields);
                }

                UpdateBlock::Movement { guid, movement } => {
                    let entity = self
                        .entities
                        .entry(*guid)
                        .or_insert_with(|| Entity::new(*guid));
                    if let Some(position) = movement.position {
                        match entity.set_server_position(position) {
                            Some(moved) => {
                                // An authoritative position supersedes whatever
                                // the spline was interpolating towards.
                                entity.spline = None;
                                jumped.push((*guid, moved, "a movement block"));
                            }
                            None => rejected += 1,
                        }
                    }
                    entity.adopt_spline(movement.spline.as_ref());
                    if let Some(progress) = movement.path_progress {
                        entity.transport_phase_ms = Some(u64::from(progress));
                    }
                    if let Some(speeds) = movement.speeds {
                        entity.speeds = Some(Speeds(speeds));
                    }
                }

                UpdateBlock::OutOfRange { guids } => {
                    for g in guids {
                        // **An item leaving is an inventory change too**, and
                        // [`ObjectManager::remove`] is where that is decided —
                        // for this arm and for the bare `SMSG_DESTROY_OBJECT`
                        // alike, which is the door items actually leave by.
                        self.remove(*g);
                    }
                }

                // "Near objects" is informational; it does not create anything.
                UpdateBlock::NearObjects { .. } => {}
            }
        }
        self.rejected_positions = self.rejected_positions.saturating_add(rejected);
        if inventory_moved {
            self.inventory_version = self.inventory_version.wrapping_add(1);
            // …and the other one: something the character carries changed, which
            // includes the entry of a slot that has just been looted into.
            self.queries_wanted = true;
        }
        for (guid, moved, source) in jumped {
            self.note_jump(guid, moved, source);
        }
    }
}

/// Does this block change what the character is carrying?
///
/// Two cases, and they are different questions. **Any** field on an `Item` or a
/// `Container` counts — a stack count, a durability, the entry itself — because
/// every one of them is drawn. On the **local player** only the three slot runs
/// count, because a player's block also carries health, power, position and
/// twenty other things that move constantly, and treating those as an inventory
/// change would rebuild the bags every tick and defeat the latch entirely.
///
/// It errs towards rebuilding: the ranges are checked by index rather than by
/// meaning, so a field this client has not named that happens to sit inside one
/// of them still triggers. That is the safe direction — a redundant rebuild
/// costs a hash walk, a missed one costs a bag that never updates.
fn touches_inventory(entity: &Entity, fields: &[(u16, u32)]) -> bool {
    if is_carried_object(entity) {
        return true;
    }
    match entity.object_type {
        Some(ObjectType::Player) if entity.is_self => {
            use crate::state::fields::player::{
                BUYBACK_PRICE_1, BYTES_2, INV_SLOT_HEAD, KEYRING_SLOT_1, VENDORBUYBACK_SLOT_1,
            };
            // `INV_SLOT_HEAD` runs into `PACK_SLOT_1`, `BANK_SLOT_1` and
            // `BANKBAG_SLOT_1` with no gap, so the worn slots, the four bag
            // slots, the backpack, the bank's squares and the bags in it are
            // one contiguous range up to the buyback; the key ring is the
            // second. `BYTES_2` is the bank's bought-slot count, one byte of
            // a field whose other three move rarely (the rest state is one).
            //
            // **The buyback is the third, and it is two runs.** A sale moves
            // nothing a bag square draws *and* nothing else — the item is not
            // destroyed, it is re-parented into
            // `PLAYER_FIELD_VENDORBUYBACK_SLOT_n` — so without these the
            // vendor's second tab would fill only when something unrelated
            // moved. The price and the timestamp are adjacent runs of twelve
            // and are covered as one.
            fields.iter().any(|(index, _)| {
                (INV_SLOT_HEAD..VENDORBUYBACK_SLOT_1).contains(index)
                    || *index == BYTES_2
                    || (KEYRING_SLOT_1..KEYRING_SLOT_1 + 32).contains(index)
                    || (VENDORBUYBACK_SLOT_1
                        ..VENDORBUYBACK_SLOT_1 + 2 * crate::play::items::BUYBACK_SLOTS as u16)
                        .contains(index)
                    || (BUYBACK_PRICE_1..BUYBACK_PRICE_1 + 2 * crate::play::items::BUYBACK_SLOTS as u16)
                        .contains(index)
            })
        }
        _ => false,
    }
}

/// **Is this object part of what the character is carrying?**
///
/// The one test behind both the destroy latch in [`ObjectManager::remove`] and
/// the first arm of [`touches_inventory`]. Any field on an `Item` or a
/// `Container` counts — a stack count, a durability, the entry itself — because
/// every one of them is drawn.
fn is_carried_object(entity: &Entity) -> bool {
    matches!(
        entity.object_type,
        Some(ObjectType::Item) | Some(ObjectType::Container)
    )
}

#[cfg(test)]
mod tests;
