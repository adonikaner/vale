//! A minimal object manager: the client's view of the world, keyed by GUID.
//!
//! The server does not send the whole world state. It sends deltas
//! (`SMSG_UPDATE_OBJECT`) that create objects, change fields, and drop objects
//! out of range. The client holds the accumulated state, and this module is
//! where it is held.

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

/// One extrapolated stride, clipped at the point where the world's collision
/// stops it.
///
/// A free function rather than a method because [`ObjectManager::advance`]
/// holds the entity map mutably while it runs and borrows the world beside it.
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
/// Public because two crates test this bit and only one of them has an
/// [`Entity`]: the renderer's snapshot carries the whole flag word. See
/// [`Entity::is_stunned`] for what the client does with it.
pub const UNIT_FLAG_STUNNED: u32 = 0x0004_0000;

/// `UNIT_FLAG_TAXI_FLIGHT`, bit 20 of `UNIT_FIELD_FLAGS`. See
/// [`Entity::is_on_taxi`] for how it is read and why.
///
/// Public for the same reason as [`UNIT_FLAG_STUNNED`]: `game::taxi::on_taxi`
/// answers `UnitOnTaxi(unit)` from a snapshot that carries the flag word
/// rather than an [`Entity`].
pub const UNIT_FLAG_TAXI_FLIGHT: u32 = 0x0010_0000;

/// `UNIT_VIS_FLAGS_GHOST`, bit 0 of the fourth byte of `UNIT_FIELD_BYTES_1`:
/// the spirit walking back from the graveyard (`SPELL_AURA_GHOST`).
///
/// Public for the same reason as the two flags above: the renderer's snapshot
/// carries the whole byte and tests it directly rather than through an
/// [`Entity`]. See [`Entity::vis_flags`].
pub const UNIT_VIS_FLAGS_GHOST: u8 = 0x01;

/// `UNIT_VIS_FLAGS_CREEP`, bit 1 of the same byte: stealth, in every form the
/// game has.
///
/// vmangos sets it from `Aura::HandleModStealth` and nowhere else, so a rogue's
/// Stealth, a druid's Prowl, a night elf's Shadowmeld and every hidden creature
/// all arrive as this one bit. The two rules that depend on stealth therefore
/// test this bit, not a spell id.
pub const UNIT_VIS_FLAGS_CREEP: u8 = 0x02;

/// `UNIT_VIS_FLAGS_UNTRACKABLE`, bit 2: `SPELL_AURA_UNTRACKABLE`, which keeps
/// a unit off the minimap's tracking blips.
///
/// Nothing reads it yet. It is defined so that all three meanings of the byte
/// are documented in one place.
pub const UNIT_VIS_FLAGS_UNTRACKABLE: u8 = 0x04;

/// `MAX_VISIBLE_ITEM_OFFSET` (`ItemDefines.h:157`): how far apart two slots'
/// blocks are in the player's field block. It is 12 from 1.6 onwards and 11
/// before, because the enchantment array grew. A value copied from the wrong
/// version reads the neighbouring slot's data without any error.
pub const VISIBLE_ITEM_STRIDE: u16 = 12;

/// One item in a unit's hand, as the server describes it.
///
/// Two sources carry the same five numbers, and this type holds them in one
/// form. A creature's arrive in its update fields, already resolved:
/// `Creature::SetVirtualItem` writes the display id into
/// `UNIT_VIRTUAL_ITEM_SLOT_DISPLAY` and packs the rest into
/// `UNIT_VIRTUAL_ITEM_INFO` beside it. A player's arrive as an item entry in
/// `PLAYER_VISIBLE_ITEM_n_0`, and the same five come back from
/// `SMSG_ITEM_QUERY_SINGLE_RESPONSE` a round trip later.
/// [`ObjectManager::weapons_of`] joins the two.
///
/// Nothing here is interpreted. What the numbers mean (which attachment point
/// a sheathed weapon hangs from, which swing its wielder plays) is a game rule
/// in `vale_assets::tables::item`, so this type holds five plain numbers and
/// not an enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeldItem {
    /// `ItemDisplayInfo` row. Zero means an empty hand, and is the only test
    /// for one.
    pub display_id: u32,
    /// `ItemPrototype::Class`: 2 weapon, 4 armour.
    pub class: u8,
    pub subclass: u8,
    /// `ItemPrototype::Material`: what the weapon is made of. It is used only
    /// for sound, not for the model: `WeaponImpactSounds` has a metal and a
    /// non-metal row per subclass. See
    /// `vale_assets::tables::item::Weapon::material`.
    pub material: u8,
    pub inventory_type: u8,
    /// `ItemPrototype::Sheath`: where the item hangs when it is put away.
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
    /// A create block has been applied, so a missing field is zero rather than
    /// unknown.
    ///
    /// `Object::_SetCreateBits` sets a bit only `if (m_uint32Values[index] != 0)`,
    /// so a create block states all of the object's values and expresses every
    /// zero by omitting the field. A values update is different: it carries the
    /// fields that changed, and an absent field means "unchanged". That is why
    /// this is a flag on the entity rather than a rule in [`Entity::field`].
    ///
    /// Without it a corpse drew as a live creature. `UNIT_FIELD_HEALTH` of a
    /// unit that was already dead when it came into view is 0, so it is never
    /// sent, so [`Entity::is_dead`] answered `None` and the renderer's
    /// `unwrap_or(false)` drew the unit standing in its idle. See
    /// [`Entity::health`], the one accessor that reads this flag, and the note
    /// there on why the other accessors do not.
    pub created: bool,
    /// Set on the object this session controls.
    pub is_self: bool,
    /// Walk/run/run-back/swim/swim-back/turn-rate, when the server sent a
    /// living movement block. The server's movement anticheat checks the
    /// player against their own run speed, so this value affects more than
    /// display.
    pub speeds: Option<Speeds>,
    /// Server-driven movement currently being animated.
    pub spline: Option<Spline>,
    /// How far into its cycle a moving platform is, in milliseconds, or `None`
    /// for any object that is not a moving platform. Only a few dozen
    /// elevators and the tram have one.
    ///
    /// Seeded from the trailing word of `UPDATEFLAG_TRANSPORT` (see
    /// [`crate::state::update::MovementUpdate::path_progress`]) and advanced by
    /// [`ObjectManager::advance`] on the world clock. The 1.12.1 client stores
    /// `progress - now` and adds `now` back each frame, which is the same
    /// quantity.
    ///
    /// The value is a phase, not a timestamp, and it is not wrapped here. The
    /// cycle length comes from the table (`vale_assets::tables::transport`),
    /// which this crate does not read, so the reader wraps it.
    /// `Transports::offset_at` does the wrapping.
    ///
    /// No other packet mentions an elevator: there is no
    /// `SMSG_MONSTER_MOVE`, no position update and no state change. This word
    /// and the shipped table are the only inputs. A client that ignores the
    /// word draws every platform at its spawn point.
    pub transport_phase_ms: Option<u64>,
    /// The last movement block the server broadcast for this entity, from a
    /// `MSG_MOVE_*` packet. Kept, not only applied, because its flags drive the
    /// dead reckoning until the next packet. Another player sends a start
    /// packet and then only heartbeats, so between them the client simulates
    /// that player the same way it simulates itself.
    pub movement: Option<MovementInfo>,
    /// The movement flags set by the twelve `SMSG_SPLINE_MOVE_*` packets. For
    /// a server-controlled unit these packets are the only source of its flags
    /// after the create block.
    ///
    /// Held beside [`Self::movement`] rather than inside it because it must
    /// survive the two things that replace that field whole: a spline (which
    /// hides the flags entirely; see [`Self::move_flags`]) and a new create
    /// block. If a re-create cleared a root, nothing would set it again.
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
    /// Taken from the packet's `fallTime` rather than accumulated locally.
    /// Every heartbeat during a jump restates both the position and the phase,
    /// so the client re-anchors the arc on each one and does not drift.
    ///
    /// Without this field a jumping player was dead-reckoned horizontally with
    /// z held at the take-off height, and the real height arrived only with
    /// each heartbeat. Other players appeared to teleport up and back down.
    pub fall_secs: f32,
    /// How many times the server has stated this entity's position.
    ///
    /// The live session dead-reckons the player between packets and writes the
    /// result back here, so comparing positions cannot tell a server correction
    /// (teleport, knockback, anticheat resync) from the client's own guess.
    /// A counter can.
    pub position_updates: u32,
    /// The value of [`Self::position_updates`] at which this entity's standing
    /// height was last snapped to the ground. See
    /// [`ObjectManager::stand_on_the_ground`].
    ///
    /// A latch rather than a flag, so the correction runs once per server
    /// position rather than once per simulation step. A city's idle population
    /// is hundreds of entities, and the terrain query is the only costly step
    /// in `advance`. `None` means "never checked", which is also the state a
    /// new create block leaves.
    pub grounded_for: Option<u32>,
    /// How many melee swings this unit has thrown, and how many have landed on
    /// it.
    ///
    /// Counters rather than timestamps, for the same reason as
    /// `position_updates`: the object manager has no clock, and a renderer that
    /// polls at its own rate needs to know whether anything happened since its
    /// last poll, not when. A swing has no duration. The server reports that a
    /// blow landed and sends nothing more about it, so only the occurrence is
    /// recorded.
    pub swings_thrown: u32,
    /// The unit this unit is auto-attacking, per `SMSG_ATTACKSTART` and
    /// `SMSG_ATTACKSTOP` about it, or `None` when it is not attacking anybody.
    ///
    /// This field, not `UNIT_FLAG_IN_COMBAT`, puts a unit in its combat-ready
    /// stance. The client shows the weapon-class Ready idle only while the
    /// auto-attack target guid is set; the combat flag and the sheath state do
    /// not decide it. The combat flag would be wrong in two cases: a caster
    /// being hit has `IN_COMBAT` set for the whole fight and never raises a
    /// weapon, and a player who has aggro from across a room is in combat
    /// before reaching anything to swing at.
    ///
    /// Written for every unit, unlike [`ObjectManager::attacking`], which holds
    /// the same two packets filtered to the local player and drives the Attack
    /// button's highlight.
    pub attacking: Option<u64>,
    /// This client has been given control of this unit: the byte of
    /// `SMSG_CLIENT_CONTROL_UPDATE`, stored per unit.
    ///
    /// The 1.12.1 client keeps this as a flag on the unit named by the packet's
    /// packed guid, not as a session-wide "current mover". The mover is derived
    /// each frame from this flag and from `PLAYER_FARSIGHT`, so a stale value
    /// is corrected on the next frame. That is why the flag is stored here.
    ///
    /// Written by a packet handler rather than by an update block, like
    /// [`Self::attacking`], and discarded with the object, as in the 1.12.1
    /// client.
    pub client_controlled: bool,
    /// `HitInfo` of the swing that last moved [`Self::swings_thrown`].
    ///
    /// The attacker-side counterpart of [`Self::last_victim_state`], from the
    /// same packet. The counter says a swing is due to be drawn and this field
    /// says which swing: `HITINFO_LEFTSWING` makes it the off-hand's and
    /// `HITINFO_CRITICALHIT` makes it a critical. The server sets both bits on
    /// every blow; this client previously ignored them.
    pub last_swing_info: u32,
    /// The victim's response to the last swing (`VictimState`), recorded on the
    /// attacker.
    ///
    /// The same word as [`Self::last_victim_state`], stored on the other unit.
    /// A dodge, a parry or a block is the victim's reaction and also decides
    /// the attacker's impact sound, because the sound depends on what the blow
    /// hit. A parried swing and a swing that hit plate use two different columns
    /// of the attacker's weapon row, so the attacker needs this value.
    pub last_swing_state: u32,
    /// The target of the last swing, so the impact sound plays where the blow
    /// landed rather than where the swing started.
    pub last_swing_victim: u64,
    /// The damage of the last swing: the number that floats over the victim's
    /// head.
    ///
    /// Zero for a miss, a dodge, a parry or a full absorb, as the server sends
    /// it. The word drawn for those cases comes from
    /// [`Self::last_swing_state`] and [`Self::last_swing_info`].
    pub last_swing_damage: u32,
    /// Damage dealt to this unit by the local player, as a monotonic counter
    /// like the other combat counters here.
    ///
    /// Increased by both a melee swing and a spell, in one counter, because the
    /// reader only needs to know that a number landed on this unit. The 1.12.1
    /// client shows these numbers for both sources and only when the source is
    /// the player or the player's pet, so the source test is made here rather
    /// than by the reader.
    ///
    /// See `client::ui::worldtext`, which compares successive values.
    pub damage_taken: u32,
    /// The amount of the blow that last moved [`Self::damage_taken`]. Zero for
    /// a miss, a dodge, a parry or a full absorb, as the server sends it. The
    /// word drawn for those cases comes from the two fields below.
    pub last_damage: u32,
    /// `HitInfo` for a swing and `SpellHitType` for a spell. These are two
    /// different flag sets, so [`Self::last_damage_spell`] records which one
    /// this is. A swing's crit is `0x80` and a spell's is `0x02`; reading one
    /// as the other reports a critical on ordinary hits.
    pub last_damage_info: u32,
    /// `VictimState` for a swing, and zero for a spell. A spell has no such
    /// field; its misses and resists are `SpellHitType` bits.
    pub last_damage_state: u32,
    /// The spell that did the damage, or `None` for a weapon swing. It selects
    /// which of the two flag sets above applies, which is why swings and
    /// spells share one channel: the reader needs to know which set it holds.
    pub last_damage_spell: Option<u32>,
    /// Whether the last event was a heal rather than a hit. Neither the amount
    /// nor the flags record this.
    pub healed: bool,
    pub blows_taken: u32,
    /// `VictimState` of the blow that last moved [`Self::blows_taken`].
    ///
    /// The counter says a reaction is due and this field says which one: a
    /// flinch, a dodge, a parry or a block. It is stored beside the counter,
    /// not as a separate counter, because both describe one event. A poller
    /// reading two independent counters could pair the state of the next blow
    /// with the count of this one.
    pub last_victim_state: u32,
    /// The `HitInfo` of the same blow, which says whether it was a critical.
    ///
    /// A second field beside [`Self::last_victim_state`] because the packet
    /// carries them as two separate fields and the client uses them for two
    /// decisions. The victim state chooses between a flinch, a dodge, a parry
    /// and a block. This field chooses between the normal flinch and the
    /// critical flinch, `AnimationData.dbc` id 10, `CombatCritical`. See
    /// `pose::reaction`, where the selection rule is documented.
    pub last_blow_info: u32,
    /// How many `SMSG_AI_REACTION`s this unit has sent, and the last one's
    /// reason.
    ///
    /// The same counter-plus-payload pair as the swing, because an AI reaction
    /// is also an event that the update stream never restates.
    /// `AI_REACTION_HOSTILE` arrives on every attack, so the counter moves many
    /// times in a fight. The client's sound priority channel keeps that from
    /// playing a voice line on each one; see `vale_client::sound::combat`.
    pub reactions: u32,
    /// The [`crate::play::action::ai_reaction`] value that last moved
    /// [`Self::reactions`].
    pub last_reaction: u32,
    /// How many one-shot emotes this unit has played, and the `Emotes.dbc` id
    /// of the last.
    ///
    /// The same counter-plus-payload pair, for the same reason. The id is an
    /// `Emotes.dbc` row, not an animation: resolving it needs the game
    /// archives, which this crate does not read.
    pub emotes: u32,
    pub last_emote: u32,
    /// How many `SpellVisualKit`s the server has told this client to play on
    /// this unit, and the last of them (`SMSG_PLAY_SPELL_VISUAL`).
    ///
    /// The same counter-plus-payload pair as the emote, for the same reason:
    /// the event has no duration and no state, so only its occurrence is
    /// recorded. The id is a `SpellVisualKit` row rather than a spell, so the
    /// tables this crate cannot read need an entry point keyed by kit; see
    /// [`crate::play::sound`].
    ///
    /// Eating and drinking arrive this way. vmangos sends kit 406 (food) and
    /// 438 (drink) on every regeneration tick while a character sits with
    /// either, and no other packet says that a character is eating.
    pub spell_visuals: u32,
    pub last_spell_visual: u32,
    /// The same pair for `SMSG_PLAY_SPELL_IMPACT`. That packet has the same body
    /// and the same kind of id, but names the unit the effect happened to rather
    /// than the unit that caused it.
    ///
    /// Two counters rather than one with a flag, because the two go to the
    /// two effect slots a unit has (a caster's kit and a victim's). Sharing one
    /// counter would let a new impact remove a cast visual that is still
    /// running.
    pub spell_impacts: u32,
    pub last_spell_impact: u32,
    /// How many casts this unit has begun and how many it has released, with
    /// the cast bar's length in milliseconds beside the first.
    ///
    /// Two counters because they are two packets and the renderer does two
    /// different things with them: `SMSG_SPELL_START` holds the wind-up pose
    /// for `cast_time_ms`, and `SMSG_SPELL_GO` plays the release. An instant
    /// spell sends only the second, so a client that waited for both would
    /// never animate one.
    pub casts_begun: u32,
    pub casts_released: u32,
    /// The last four spells this unit released, most recent written last.
    ///
    /// `last_spell` holds one spell, which fails when two releases arrive
    /// between two polls. Charge always does this: the server sends
    /// `SMSG_SPELL_GO` for Charge (100), then a second for the Charge Stun
    /// (7922) it triggers on the victim, both inside one 25 ms tick. Measured
    /// against a live server, the renderer saw only `spell = 7922`, which
    /// names no caster models, so the red trail and the dust cloud named by
    /// Charge's kit were never drawn. Every spell that triggers a second spell
    /// behaves the same way.
    ///
    /// A fixed array rather than a `Vec` because it is copied into every entity
    /// snapshot on every poll, and an allocation per unit per frame is not
    /// justified for an event that happens seconds apart. The size four is a
    /// renderer's depth, not a protocol limit (unlike [`MAX_AURA_SLOTS`]). A
    /// burst of more than four drops the oldest, which costs one frame of one
    /// glow.
    pub recent_spells: [u32; RECENT_SPELLS],
    /// How many of those releases were a channel starting.
    ///
    /// `MSG_CHANNEL_START` is the only packet that states a channelled spell's
    /// length, and vmangos sends it after `SendSpellGo`. A channel therefore
    /// arrives as a release followed by a begin, inside one poll. Without a way
    /// to tell them apart, the release is tested last and cancels the wind-up
    /// the channel had just started: Evocation played a release animation it
    /// does not have and then stood in its idle loop for eight seconds. This
    /// counter lets the later event take effect.
    pub casts_channelled: u32,
    /// How many releases the server has stated. It counts the same event as
    /// [`Self::casts_released`], but only from packets.
    ///
    /// The two are equal for every unit except the local player. For the local
    /// player they differ on purpose, because they answer two questions:
    ///
    /// * `casts_released` says what to draw on the caster. It moves at the key
    ///   press so the arm does not wait a round trip
    ///   ([`ObjectManager::predict_own_cast`]).
    /// * this counter says what the cast hit, which only `SMSG_SPELL_GO` can
    ///   answer: [`Self::last_spell_target`] is the first entry of the packet's
    ///   hit list, and it does not exist until the packet arrives.
    ///
    /// When the two were one counter, no implicitly targeted spell drew its
    /// impact. A cone or area spell sends no target block
    /// (`CastTarget::SelfImplicit`, the targeting value of 14,002 of the 22,360
    /// rows in `Spell.dbc`), so the prediction wrote `last_spell_target = 0`,
    /// the release counter moved at the key press, and the consumer concluded
    /// one frame later that nothing was hit. The server's `SMSG_SPELL_GO` then
    /// arrived with the real victim and did not move the counter, so nothing
    /// downstream looked again. Cone of Cold lost most of its visuals: its
    /// release is one glow on the hand, and everything else it draws is the
    /// impact on the units it hits.
    pub casts_landed: u32,
    /// How many casts on this unit have ended early: refused, interrupted or
    /// cancelled. No other counter here records that end of a cast.
    ///
    /// A third counter rather than a decrement of the two above, because every
    /// consumer acts when a counter moves, so subtracting from `casts_begun`
    /// would look like another cast beginning. A cancel means that the cast
    /// being drawn has ended, which is an event of its own.
    ///
    /// It is needed because this client predicts its own casts and the 1.12.1
    /// client does not. The local player's cast is drawn at the key press
    /// ([`Self::casts_begun`]), so the arm is already moving and the release
    /// art is already shown when the server answers. If the answer is a
    /// refusal, nothing else removes them: the wind-up would run for its
    /// `HOLD_GRACE_SECS` and the release art for its `RELEASE_SECS`, so a spell
    /// that could not be cast still played its animation.
    ///
    /// Three packets move it, and only the first concerns the prediction:
    /// `SMSG_CAST_RESULT` carrying a failure (the local player's),
    /// `SMSG_SPELL_FAILED_OTHER` (any unit's; an interrupted cast stops, where
    /// previously the pose was held until its timer expired), and the local
    /// player's `CMSG_CANCEL_CAST`.
    pub casts_cancelled: u32,
    pub cast_time_ms: u32,
    /// The `Spell.dbc` id of the last cast this unit began or released.
    ///
    /// The animation depends on the spell, not on the act of casting.
    /// `Spell.dbc` names a `SpellVisual`, which names two `SpellVisualKit`s,
    /// which name the wind-up and the release. A fireball is thrown from the
    /// shoulder, a heal is raised overhead, and opening a chest is a crouch:
    /// three poses from one packet type whose only distinguishing field is
    /// this one. Resolving it needs the game archives, which this crate does
    /// not read, so it is carried as an id, like `last_emote`.
    pub last_spell: u32,
    /// The unit the last released cast landed on, or 0 when it hit nothing the
    /// client can identify.
    ///
    /// A separate field from [`Self::last_spell`] because only `SMSG_SPELL_GO`
    /// supplies it, and only for the release: a wind-up has no hit list. It is
    /// the destination of a missile (a fireball leaves the caster's hand and
    /// travels to it), and it is the only part of a cast that describes a unit
    /// other than the caster.
    pub last_spell_target: u64,
    /// Every unit the last released cast landed on. For an area spell this
    /// differs from [`Self::last_spell_target`].
    ///
    /// The missile is one object and flies to one place, so
    /// [`Self::last_spell_target`] stays a single value. The impact is per
    /// victim: the impact kit of `SpellVisualKit` is the flash on each unit hit,
    /// so an Arcane Explosion that hits five creatures draws five. Reading only
    /// the first entry of the `SMSG_SPELL_GO` hit list drew nothing on four of
    /// the five, while every counter reported success.
    ///
    /// Empty for a cast that hit nothing identifiable, and for every wind-up:
    /// `SMSG_SPELL_START` carries no hit list.
    pub last_spell_targets: Vec<u64>,
    /// Pushback: how many times damage has delayed the cast in progress, and
    /// by how long the last delay moved it.
    ///
    /// The same counter-plus-payload pair as the swing and the emote, for the
    /// same reason: a poll cannot tell one delay from two without a counter.
    /// `SMSG_SPELL_DELAYED` is sent only to the caster and only for a cast
    /// already running. It extends everything held for that cast by the given
    /// amount: the wind-up pose, the art on the caster's hands, and the cast
    /// bar. No other packet restates the cast's length after
    /// `SMSG_SPELL_START`.
    pub casts_delayed: u32,
    pub last_cast_delay_ms: u32,
    /// The largest distance any single server statement has moved this entity,
    /// in yards, and how many statements moved it more than [`JUMP_YARDS`].
    ///
    /// This is the diagnostic for creatures that appear to teleport. A jump is
    /// not an error, because the server does relocate things. Frequent jumps
    /// show that the client moved a creature somewhere the server did not and
    /// was then corrected. The counters identify which packet causes the jumps
    /// and how far they are.
    pub worst_jump: f32,
    pub jumps: u32,
}

/// How far a single server statement may move an entity before it counts as a
/// jump rather than a correction.
///
/// A creature runs at about 7 yards a second and the server states its position
/// at most twice a second, so 10 yards is well above any legitimate
/// half-second of travel and well below a creature reappearing somewhere else.
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
    /// An impossible position is refused, not adopted. The client is not
    /// authoritative, so its only defence against a misread coordinate is to
    /// detect that the value is out of range. Adopting it would move the entity
    /// thousands of yards, outside every loaded tile, where it is invisible and
    /// nothing moves it back, while the server still places it nearby. Keeping
    /// the last good position leaves a stale entity instead, which stays visible,
    /// and [`ObjectManager::rejected_positions`] counts the refusal.
    ///
    /// Returns `None` if the position was refused, otherwise how far it moved
    /// the entity from the position the client had simulated. The bound is the
    /// one in `MaNGOS::IsValidMapCoord`: `MAX_MAP_COORD` is 64 * 533.33333 / 2.
    ///
    /// The returned distance feeds the teleport diagnostic; see
    /// [`Entity::worst_jump`]. The caller reports it, because only the caller
    /// knows which packet caused it.
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
    /// `None` does nothing rather than clearing: a block without a spline says
    /// nothing about one, and most blocks carry none. The caller clears the
    /// spline when the packet means that.
    ///
    /// `elapsed_ms` is the server's `timePassed`, which is the reason for
    /// reading this block. Starting the path from zero would move the creature
    /// back to the start of a patrol it is half way through, a larger jump than
    /// the one this avoids.
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
            // An update block's spline section carries no transport reference.
            // Only `SMSG_MONSTER_MOVE_TRANSPORT` does.
            on_transport: None,
        });
        // A server-driven move replaces dead reckoning, as in
        // `apply_monster_move`.
        self.movement = None;
    }

    /// Low part of the GUID, which vmangos shows in its console and database.
    pub fn low_guid(&self) -> u32 {
        (self.guid & 0xFFFF_FFFF) as u32
    }

    pub fn field(&self, index: u16) -> Option<u32> {
        self.fields.get(&index).copied()
    }

    pub fn field_f32(&self, index: u16) -> Option<f32> {
        self.field(index).map(f32::from_bits)
    }

    /// Template id: the argument of `CMSG_CREATURE_QUERY`. It identifies the
    /// kind of object, where the GUID identifies the instance.
    /// `object::ENTRY` is at the same index for every object type.
    pub fn entry(&self) -> Option<u32> {
        self.field(fields::object::ENTRY)
    }

    pub fn scale(&self) -> Option<f32> {
        self.field_f32(fields::object::SCALE_X)
    }

    /// `UNIT_FIELD_COMBATREACH`: the part of the distance between two units
    /// that is taken up by the units themselves.
    ///
    /// Every range in the game is measured between surfaces, not centres.
    /// vmangos' `GetCombatDistance` is `GetDistance(target,
    /// SizeFactor::CombatReach)`, which subtracts both units' reaches, and
    /// `Spell::CheckRange` uses that number. A client that checks a spell's
    /// range centre to centre refuses casts at a large creature that the server
    /// would accept. A local range check must never refuse what the server
    /// would accept.
    ///
    /// `None` when the field block has not stated the field. The caller treats
    /// that as [`DEFAULT_COMBAT_REACH`], not as zero.
    pub fn combat_reach(&self) -> Option<f32> {
        self.field_f32(fields::unit::COMBATREACH)
    }

    /// `UNIT_FIELD_BOUNDINGRADIUS`: the radius of the unit's footprint.
    ///
    /// The field next to [`Self::combat_reach`] on the wire, and related in
    /// meaning: the reach is the part of a gap taken up by the unit, and this
    /// is the radius of the unit's footprint on the ground, in world yards,
    /// with the object's scale already applied.
    ///
    /// It is the only footprint the game states. The M2's bounding box covers
    /// every frame of every animation, so it measures something else:
    /// `Creature\Boar\Boar.m2` declares a horizontal half-extent of 2.5 model
    /// yards where the server sends 0.882 for the Rockhide Boar that uses it.
    /// That factor of two and a half made the selection circle far too large.
    ///
    /// vmangos writes it in `Unit::UpdateModelData` from
    /// `creature_display_info_addon.bounding_radius`, scaled by
    /// `GetObjectScale() / nativeScale`. The value on the wire is therefore
    /// already in world space, and a reader must not apply
    /// `OBJECT_FIELD_SCALE_X` to it again.
    ///
    /// `None` when the field block has not stated the field. The caller treats
    /// that as its own `DEFAULT_BOUNDING_RADIUS`, not as zero, because a zero
    /// radius draws an empty ring.
    pub fn bounding_radius(&self) -> Option<f32> {
        self.field_f32(fields::unit::BOUNDINGRADIUS)
    }

    pub fn is_unit_like(&self) -> bool {
        matches!(
            self.object_type,
            Some(ObjectType::Unit) | Some(ObjectType::Player)
        )
    }

    /// Health, level and display id read from the unit field block.
    ///
    /// These return `None` for non-units on purpose: field indices overlap
    /// between object types, so reading `unit::HEALTH` off a game object would
    /// silently return that object's unrelated field 22.
    ///
    /// Health is often a percentage, not hit points. Unless the server runs
    /// with `ShowHealthValues = 1`, vmangos hides real values from players who
    /// cannot "see health of" a unit: it sends max = 100 and current = percent
    /// (clamped to 1 when alive). A whole zone reporting `100/100` is correct
    /// server behaviour, not a parsing bug. See [`Self::health_is_percentage`].
    ///
    /// A unit whose create block did not mention its health has zero health.
    /// Without this rule, a creature that died before it came into view stood
    /// in its idle animation. `_SetCreateBits` sends a field only when it is
    /// non-zero, and a corpse's health is exactly zero, so the server never
    /// states it. A creature killed while in view is handled by a values
    /// update, which carries the change to zero.
    ///
    /// Only this accessor uses [`Self::created`], on purpose. The rule holds
    /// for every field, but its effect differs: `combat_reach` of `None` means
    /// [`DEFAULT_COMBAT_REACH`], where a stated 0.0 would tighten every range
    /// check, and [`Self::target`] of `None` means "nobody", where a stated 0
    /// would be a guid. Treating every missing field as zero would introduce
    /// several bugs to fix one, so it is applied only where zero is the correct
    /// value.
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

    /// `PLAYER_XP` and `PLAYER_NEXT_LEVEL_XP`: `UnitXP` and `UnitXPMax`.
    ///
    /// Zero rather than absent once a create block has been applied, for the
    /// reason given on [`Self::health`]. It matters more here: a character that
    /// has just levelled has exactly zero experience, so a missing field is the
    /// normal case (`_SetCreateBits` omits every zero field).
    ///
    /// The two values must be set together, because
    /// `TextStatusBar_UpdateTextString` hides a bar whose maximum is zero. The
    /// XP bar was missing for that reason: `UnitXPMax` returned a stubbed 0,
    /// `MainMenuExpBar_Update` passed it to `SetMinMaxValues`, and the FrameXML
    /// code hid the bar.
    ///
    /// Both fields are `PRIVATE`, so they are only sent for the local player's
    /// character.
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

    /// `PLAYER_CHARACTER_POINTS1` and `2`, the pair `UnitCharacterPoints("player")`
    /// returns: unspent talent points, then unspent profession points.
    ///
    /// Zero rather than absent, for the reason given on [`Self::experience`].
    /// Zero is common here: a character below level 10 has no talent points,
    /// and `_SetCreateBits` omits the field, so answering `None` for a missing
    /// field would report "unknown" for most of every session.
    ///
    /// This is the only talent-panel counter the server sends. The number of
    /// spent points is derived from the known spells; see
    /// [`crate::play::talents`] for how. `PRIVATE`, so it is only sent for the
    /// local player's character.
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

    /// `PLAYER_REST_STATE_EXPERIENCE`: `GetXPExhaustion()`, the rested pool.
    ///
    /// Returns `None` (`nil` in Lua) when the pool is zero, because FrameXML
    /// tests for `nil`: `ExhaustionTick_Update` reads `if ( not
    /// exhaustionThreshold ) then ExhaustionTick:Hide()`. A character with no
    /// rest left must return nothing rather than 0, or the tick is drawn at the
    /// left edge of the bar.
    pub fn rested_experience(&self) -> Option<u32> {
        (self.object_type == Some(ObjectType::Player))
            .then(|| self.field(fields::player::REST_STATE_EXPERIENCE))
            .flatten()
            .filter(|rested| *rested > 0)
    }

    /// `UNIT_FIELD_BYTES_0` byte 3: which of the five powers this unit runs on.
    ///
    /// There are five power fields and a unit uses only one, so this byte
    /// selects which to read. A warrior's `POWER1` (mana) is always zero, as is
    /// a rogue's, so a bar drawn from the first field is empty for half the
    /// classes. See [`power_type`] for the names.
    pub fn power_type(&self) -> Option<u8> {
        let bytes = self
            .is_unit_like()
            .then(|| self.field(fields::unit::BYTES_0))
            .flatten()?;
        Some(((bytes >> 24) & 0xFF) as u8)
    }

    /// `UNIT_FIELD_BYTES_0` bytes 0 and 1: race and class, the one-based ids
    /// that key `ChrRaces.dbc` and `ChrClasses.dbc`.
    ///
    /// The same field [`Self::power_type`] and [`Self::appearance`] read. It is
    /// a separate accessor because the callers differ: dressing a character
    /// needs the appearance block and the spellbook needs only these two. Class
    /// is not in the appearance struct, because a character's appearance does
    /// not depend on it.
    ///
    /// Race 0 is not a race, so an absent field and a zero field give the same
    /// result. [`Self::appearance`] relies on that; see the note there.
    pub fn race_and_class(&self) -> Option<(u8, u8)> {
        let bytes = self
            .is_unit_like()
            .then(|| self.field(fields::unit::BYTES_0))
            .flatten()?;
        Some((bytes as u8, (bytes >> 8) as u8))
    }

    /// `UNIT_FIELD_BYTES_0` byte 2: the unit's gender, as `UnitSex` returns it.
    ///
    /// The third byte of the field [`Self::race_and_class`] reads, and one that
    /// [`Self::appearance`] also unpacks. It is a separate accessor because
    /// `UnitSex` accepts any unit, while the appearance block returns `None`
    /// for anything that is not a player. A boar has a gender byte and the
    /// interface may ask for it.
    ///
    /// `UNIT_FIELD_BYTES_0` is field index 36 (6 + 30); gender is its third
    /// byte. Zero is male and a valid value, so an absent field is `None`
    /// rather than zero. See [`crate::play::stats`] for the same distinction.
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

    /// `POWER1`/`MAXPOWER1` plus the power type's offset. The five fields are
    /// consecutive in both blocks, so this is an addition rather than a match.
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
    /// Health decides this, not the stand state. `UNIT_STAND_STATE_DEAD` in
    /// `UNIT_FIELD_BYTES_1` exists and is set on some corpses, but a creature
    /// killed in view keeps its previous stand state. The server writes that
    /// byte only when it wants a pose, and it always writes the health. Zero
    /// health means dead in both encodings: with `ShowHealthValues = 0` the
    /// field is a percentage clamped to 1 while the unit is alive.
    ///
    /// `None` for anything that has no health, such as a game object or an
    /// item.
    pub fn is_dead(&self) -> Option<bool> {
        Some(self.health()? == 0)
    }

    /// `PLAYER_FARSIGHT`: the object this character's camera is viewing
    /// through.
    ///
    /// One field covers two kinds of spell, and it is the client's only input
    /// for both. `Player::ScheduleCameraUpdate` writes it, and sends it by
    /// itself with `DirectSendPublicValueUpdate(PLAYER_FARSIGHT, 2)`, whenever
    /// `Camera::SetView` moves the server's view point. That happens in two
    /// places:
    ///
    /// * Far sight (Eagle Eye, Far Sight, Bird's Eye): `SetLongSight` creates a
    ///   `DynamicObject` at the map's visibility distance in front of the
    ///   caster and points the camera at it. The guid therefore names a
    ///   `DynamicObject`, which this client already receives and places; a
    ///   Blizzard's ring uses the same object type.
    /// * Possess (Eye of Kilrogg, Mind Control, Eyes of the Beast):
    ///   `Unit::ModPossess` points the camera at the possessed unit.
    ///
    /// `None` in the ordinary case. The 1.12.1 client treats a zero guid (both
    /// halves zero) as no far sight and does nothing.
    ///
    /// Only a player has this field (index 712 on a creature belongs to a
    /// different block), and in practice only the local player, because the
    /// field is not in an update group sent to other clients.
    pub fn farsight(&self) -> Option<u64> {
        if self.object_type != Some(ObjectType::Player) {
            return None;
        }
        let low = u64::from(self.field(fields::player::FARSIGHT)?);
        let high = u64::from(self.field(fields::player::FARSIGHT + 1).unwrap_or(0));
        let guid = (high << 32) | low;
        (guid != 0).then_some(guid)
    }

    /// The spirit has been released: `PLAYER_FLAGS & PLAYER_FLAGS_GHOST`.
    ///
    /// Health cannot tell this apart, which is why the flag is read. A corpse
    /// has health 0, and `Player::BuildPlayerRepop` sets the ghost to health 1,
    /// the same value a nearly dead living player has. The two states allow
    /// different next actions (release, or walk to the body), and no other
    /// field distinguishes them.
    ///
    /// `false` for anything that is not a player: the field is `PLAYER_FLAGS`,
    /// and index 190 on a creature belongs to a different block.
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

    /// `PLAYER_DUEL_ARBITER`: the duel flag object both duellists reference,
    /// or `0` when there is no duel.
    ///
    /// Read for the friend-or-foe decision, not for the duel window. The
    /// client decides the reaction between two players from the arbiter and the
    /// team before it checks either faction. This is the only way two members
    /// of one faction become hostile without a PvP flag.
    pub fn duel_arbiter(&self) -> u64 {
        if self.object_type != Some(ObjectType::Player) {
            return 0;
        }
        let low = u64::from(self.field(fields::player::DUEL_ARBITER).unwrap_or(0));
        let high = u64::from(self.field(fields::player::DUEL_ARBITER + 1).unwrap_or(0));
        (high << 32) | low
    }

    /// `PLAYER_DUEL_TEAM`: which side of the duel, `0` for neither.
    pub fn duel_team(&self) -> u32 {
        (self.object_type == Some(ObjectType::Player))
            .then(|| self.field(fields::player::DUEL_TEAM))
            .flatten()
            .unwrap_or(0)
    }

    /// `PLAYER_FIELD_BYTES` byte 0: the flag byte into which `Player::KillPlayer`
    /// writes the release-window policy.
    ///
    /// This is a different field from `PLAYER_BYTES`, which holds the
    /// appearance. The field table's name shortening once mapped both to the
    /// same name, and `PLAYER_FIELD_BYTES` was dropped from the table. That
    /// caused no compile error; the field was simply unreadable.
    ///
    /// `PRIVATE`, so it is only sent for the local player's character. See
    /// [`crate::play::death::RELEASE_TIMER`] and
    /// [`crate::play::death::NO_RELEASE_WINDOW`] for the two bits used here.
    pub fn player_field_flags(&self) -> u32 {
        (self.object_type == Some(ObjectType::Player))
            .then(|| self.field(fields::player::FIELD_BYTES))
            .flatten()
            .unwrap_or(0)
            & 0xFF
    }

    /// `PLAYER_FIELD_BYTES` byte 2: which of the four extra action bars the
    /// character has enabled, one bit per bar.
    ///
    /// The same field whose byte 0 [`Self::player_field_flags`] reads, and
    /// `PRIVATE` for the same reason: only the local player needs to know which
    /// bars are shown. It is the only place in the protocol that records this.
    /// The four bars are frames in `Interface\FrameXML\MultiActionBars.xml`,
    /// and the server stores nothing else about them. Exactly one packet writes
    /// it, [`crate::play::spells::set_actionbar_toggles_body`], whose body is
    /// this byte.
    ///
    /// See [`crate::play::spells::multi_bar`] for the meaning of each bit. The
    /// 1.12.1 client reports this byte as four values, one per bit. vmangos'
    /// `HandleSetActionBarTogglesOpcode` stores it at
    /// `PLAYER_FIELD_BYTES_OFFSET_ACTION_BARS`.
    pub fn action_bar_toggles(&self) -> u8 {
        (self.object_type == Some(ObjectType::Player))
            .then(|| self.field(fields::player::FIELD_BYTES))
            .flatten()
            .map_or(0, |v| ((v >> 16) & 0xFF) as u8)
    }

    /// `PLAYER_FIELD_BYTES` byte 1: how many combo points are on the target.
    /// Every rogue and druid finisher is greyed out when this is zero.
    ///
    /// The byte next to the one [`Self::action_bar_toggles`] reads, from the
    /// same field. It is the count only. The unit the points are on is
    /// `PLAYER_FIELD_COMBO_TARGET`, which the interface never reads: a finisher
    /// with points on a different unit is refused by the server, not greyed by
    /// the client.
    pub fn combo_points(&self) -> u8 {
        (self.object_type == Some(ObjectType::Player))
            .then(|| self.field(fields::player::FIELD_BYTES))
            .flatten()
            .map_or(0, |v| ((v >> 8) & 0xFF) as u8)
    }

    /// `UNIT_FIELD_AURASTATE`: the states a spell may require.
    ///
    /// A bitmask in which the bit for state n is `1 << (n - 1)`: vmangos'
    /// `ModifyAuraState` writes `SetFlag(UNIT_FIELD_AURASTATE, 1 << (flag - 1))`.
    /// The states are `AURA_STATE_*` in `SpellDefines.h`, and the ones marked
    /// `C` there are the caster-side states a `Spell.dbc` row can require:
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
    /// This field lets the client grey out Judgement without knowing what a
    /// Seal is: the server sets the bit when the aura is applied and clears it
    /// when the aura ends, and the button follows the bit.
    pub fn aura_state(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::AURASTATE))
            .flatten()
            .unwrap_or(0)
    }

    /// `UNIT_FIELD_BYTES_1` byte 0 (`UNIT_BYTES_1_OFFSET_STAND_STATE`): 0 stand,
    /// 1..6 the sitting variants, 7 dead, 8 kneeling.
    ///
    /// Many stationary NPCs use it, such as an innkeeper on a stool or a guard
    /// asleep at a post. Drawn standing, they intersect the furniture.
    pub fn stand_state(&self) -> Option<u8> {
        self.is_unit_like()
            .then(|| self.field(fields::unit::BYTES_1))
            .flatten()
            .map(|v| (v & 0xFF) as u8)
    }

    /// `UNIT_FIELD_BYTES_1` byte 3 (`UNIT_BYTES_1_OFFSET_VIS_FLAG`): three bits
    /// that mark a unit as not drawn as an ordinary solid body.
    ///
    /// ```text
    /// 0x01  GHOST        SPELL_AURA_GHOST — the spirit at the graveyard
    /// 0x02  CREEP        SPELL_AURA_MOD_STEALTH — every stealth and prowl
    /// 0x04  UNTRACKABLE  SPELL_AURA_UNTRACKABLE
    /// ```
    ///
    /// vmangos' `UnitVisFlags` names them, and `Aura::HandleModStealth` sets
    /// the middle one (`SpellAuras.cpp:3614`). No packet and no other field
    /// reports stealth: `Spell.dbc` gives Stealth (1784) no visual (no kit, no
    /// pose, no models). This byte is the only place in the protocol that says
    /// a rogue has entered stealth, and everything the client does about it
    /// depends on these three bits alone.
    ///
    /// The 1.12.1 client uses bit `0x02` for two animation choices: the moving
    /// animation (`-> StealthWalk`) and the idle one (`-> StealthStand`). See
    /// `crate::state::objects::WorldEntity::creeping` and the pose chooser in
    /// the renderer.
    pub fn vis_flags(&self) -> u8 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::BYTES_1))
            .flatten()
            .map_or(0, |v| ((v >> 24) & 0xFF) as u8)
    }

    /// `UNIT_FIELD_BYTES_1` byte 2 (`UNIT_BYTES_1_OFFSET_SHAPESHIFT_FORM`):
    /// which form this unit is in, as a `SpellShapeshiftForm.dbc` row.
    ///
    /// 0 is no form. The warrior stances are 17, 18 and 19
    /// (`FORM_BATTLESTANCE`..`FORM_BERSERKERSTANCE` in vmangos'
    /// `SharedDefines.h`), the druid forms 1..5, and there are 32 rows in all.
    ///
    /// This value alone decides which action bar is shown. The row's
    /// `bonusActionBar` column is `GetBonusBarOffset()`, to which
    /// `ActionButton_GetPagedID` adds 72. A warrior in Battle Stance therefore
    /// uses slots 73..84, and a character whose form has no bonus bar uses the
    /// ordinary page. See `vale_assets::tables::spellbook::ShapeshiftForms`.
    pub fn shapeshift_form(&self) -> u8 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::BYTES_1))
            .flatten()
            .map_or(0, |v| ((v >> 16) & 0xFF) as u8)
    }

    /// `GAMEOBJECT_STATE`: whether a door, a chest or a lever is closed or
    /// open. It is the only statement the server makes about how a game object
    /// should look.
    ///
    /// `GO_STATE_ACTIVE` (0) is the used state (a door standing open, a chest
    /// with its lid up), `GO_STATE_READY` (1) is the reset state, and
    /// `GO_STATE_ACTIVE_ALTERNATIVE` (2) is a second used state
    /// (`GameObjectDefines.h`). Because the open state is zero, the absent case
    /// is correct: vmangos omits a zero field, so a game object with no state
    /// field is open, and that is what this returns.
    ///
    /// `None` for anything that is not a game object, because the index
    /// belongs to another type's field: 14 is `UNIT_FIELD_POWER4` on a unit.
    pub fn game_object_state(&self) -> Option<u8> {
        (self.object_type == Some(ObjectType::GameObject))
            .then(|| self.field(fields::game_object::STATE))
            .flatten()
            .map(|v| (v & 0xFF) as u8)
    }

    /// `GAMEOBJECT_FLAGS`: three bits that decide whether clicking the object
    /// does anything, and one that decides whether the nameplate says
    /// "Locked".
    ///
    /// `None` for anything that is not a game object, for the reason given on
    /// [`Self::game_object_state`]: index 9 is `UNIT_FIELD_POWER1` on a unit.
    /// See `vale_assets::look::object::go_flags`, which defines the bits, and
    /// [`Self::game_object_dyn_flags`], which one of them is read together
    /// with.
    pub fn game_object_flags(&self) -> Option<u32> {
        (self.object_type == Some(ObjectType::GameObject))
            .then(|| self.field(fields::game_object::FLAGS))
            .flatten()
    }

    /// `GAMEOBJECT_DYN_FLAGS`: the server's per-player statement of whether
    /// this character may use this object.
    ///
    /// It completes `GO_FLAG_INTERACT_COND`: the template flag says the use is
    /// conditional, and this field, sent in the private part of the update
    /// block, gives the result. A chest this character may open and one it may
    /// not differ only in this word.
    ///
    /// Absent means zero, and the code depends on that. `_SetCreateBits` omits
    /// a zero field, so an object that may not be used sends no value, which
    /// has the same meaning as the bit being clear.
    pub fn game_object_dyn_flags(&self) -> Option<u32> {
        (self.object_type == Some(ObjectType::GameObject))
            .then(|| self.field(fields::game_object::DYN_FLAGS))
            .flatten()
    }

    /// `GAMEOBJECT_LEVEL`: the rank used by any of the five lock slots whose
    /// required rank is 0.
    ///
    /// Nearly always absent, which reads as zero and is correct: vmangos writes
    /// the field only for transports (`transport.pause`). See
    /// `vale_assets::look::object::can_open`, its only reader.
    pub fn game_object_level(&self) -> Option<u32> {
        (self.object_type == Some(ObjectType::GameObject))
            .then(|| self.field(fields::game_object::LEVEL))
            .flatten()
    }

    /// What a creature holds, as `ItemDisplayInfo` ids: main hand, off hand,
    /// ranged.
    ///
    /// No query round trip is needed. `Creature::SetVirtualItem` writes
    /// `proto->DisplayInfoID` directly into `UNIT_VIRTUAL_ITEM_SLOT_DISPLAY`, so
    /// a creature's display id is in its update fields. A player's
    /// `PLAYER_VISIBLE_ITEM_n_0` instead carries an item entry that must be
    /// resolved with `CMSG_ITEM_QUERY_SINGLE`. The same function also writes
    /// the item's class, subclass, material and inventory type into
    /// `UNIT_VIRTUAL_ITEM_INFO`, and its sheath type into the second word of
    /// each pair. For players the sheath type is only in the item query
    /// response; for creatures it is here.
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

    /// `UNIT_NPC_EMOTESTATE`: an `Emotes.dbc` id the unit is holding.
    ///
    /// The second part of the emote system, and the more visible one.
    /// `Unit::HandleEmote` checks the row's `EmoteType`: a one-shot emote is
    /// sent as `SMSG_EMOTE`, and a state emote is written to this field, where
    /// it stays. `/wave` is a packet and `/dance` is this field. So is every
    /// innkeeper working at a bar, every guard standing in a ready pose, and
    /// every stunned creature.
    ///
    /// Zero means no held emote. Most units have none, so the field is usually
    /// absent from an update block.
    pub fn emote_state(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::NPC_EMOTESTATE))
            .flatten()
            .unwrap_or(0)
    }

    /// Everything a creature holds: the display id and the four item
    /// properties stored beside it.
    ///
    /// `UNIT_VIRTUAL_ITEM_INFO` is two words per slot, and
    /// `Creature::SetVirtualItem` packs them with `SetByteValue`: class,
    /// subclass, material and inventory type into the bytes of the first, and
    /// sheath type into byte 0 of the second. That is six fields for three
    /// slots, starting at index 40.
    ///
    /// Empty for a player. A player has these indices but the server never
    /// writes them, because a player's gear is in `PLAYER_VISIBLE_ITEM_n_0` and
    /// needs the query round trip. [`ObjectManager::weapons_of`] handles both.
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
                // Byte 2 is the material. It decides the sound of a blow and
                // nothing that is drawn. When it was not read, a mace could
                // play the impact sound of the other material's row.
                // `VIRTUAL_ITEM_INFO_0_OFFSET_MATERIAL` is 2.
                material: ((info >> 16) & 0xFF) as u8,
                inventory_type: ((info >> 24) & 0xFF) as u8,
                sheath: (sheath & 0xFF) as u8,
            };
        }
        out
    }

    /// The item entries in the three weapon slots: main hand, off hand,
    /// ranged.
    ///
    /// `EQUIPMENT_SLOT_MAINHAND` is 15 and the three are consecutive, in the
    /// same order [`Self::virtual_items`] uses. The two sources of a unit's
    /// weapons therefore match slot for slot, and the caller does not need to
    /// know which one it used.
    pub fn weapon_entries(&self) -> Option<[u32; 3]> {
        const EQUIPMENT_SLOT_MAINHAND: usize = 15;
        let equipment = self.equipment()?;
        Some([
            equipment[EQUIPMENT_SLOT_MAINHAND],
            equipment[EQUIPMENT_SLOT_MAINHAND + 1],
            equipment[EQUIPMENT_SLOT_MAINHAND + 2],
        ])
    }

    /// The enchantment ids on the three weapon slots' items, in the order
    /// [`Self::weapon_entries`] uses, seven per slot.
    ///
    /// The seven words after `PLAYER_VISIBLE_ITEM_n_0` are the item's
    /// enchantment slots. vmangos writes the first two, the permanent and the
    /// temporary enchantment, in `Player::SetVisibleItemSlot` and again in
    /// `Player::ApplyEnchantment` whenever one is applied or removed, so a
    /// shaman's Flametongue reaches every player in view. All zero for a unit
    /// that is not a player: a creature's weapon fields carry no enchantment.
    pub fn weapon_enchantments(&self) -> [[u32; 7]; 3] {
        const EQUIPMENT_SLOT_MAINHAND: u16 = 15;
        let mut out = [[0u32; 7]; 3];
        if self.object_type != Some(ObjectType::Player) {
            return out;
        }
        for (hand, slots) in out.iter_mut().enumerate() {
            let base = fields::player::VISIBLE_ITEM_1_0
                + (EQUIPMENT_SLOT_MAINHAND + hand as u16) * VISIBLE_ITEM_STRIDE;
            for (slot, id) in slots.iter_mut().enumerate() {
                *id = self.field(base + 1 + slot as u16).unwrap_or(0);
            }
        }
        out
    }

    /// What this unit is riding, as a `CreatureDisplayInfo` id:
    /// `UNIT_FIELD_MOUNTDISPLAYID`. It is the only mount indicator in the
    /// protocol, and it resolves to a model the same way as every other
    /// display id.
    ///
    /// Zero means "not mounted", not "display id 0", so this filter is all
    /// that [`Self::mounted`] needs. vmangos dismounts with
    /// `SetUInt32Value(UNIT_FIELD_MOUNTDISPLAYID, 0)`, and `_SetCreateBits`
    /// omits a zero field, so an unmounted unit sends no value.
    pub fn mount_display_id(&self) -> Option<u32> {
        self.is_unit_like()
            .then(|| self.field(fields::unit::MOUNTDISPLAYID))
            .flatten()
            .filter(|v| *v != 0)
    }

    /// Whether this unit is riding a mount.
    ///
    /// Used for a second purpose besides drawing the mount: a mounted rider's
    /// weapons are sheathed for as long as the rider is mounted (the persistent
    /// draw block in `vale_assets::look::sheath::reconcile`). Without it, a
    /// character who takes a griffon with a sword drawn holds the sword for the
    /// whole flight.
    pub fn mounted(&self) -> bool {
        self.mount_display_id().is_some()
    }

    /// `UNIT_FIELD_BYTES_2` byte 0: 0 unarmed, 1 melee drawn, 2 ranged drawn.
    ///
    /// For the local player's character this value echoes the client's own
    /// request. In vmangos only `HandleSetSheathedOpcode` writes a `Player`'s
    /// value, so what arrives is the last `CMSG_SETSHEATHED` this client sent.
    /// The client therefore keeps its own committed state and adopts this
    /// value only when it changes independently. See `vale_assets::look::sheath`.
    /// A creature's value comes from the server: `Creature::Create` sets melee.
    ///
    /// This value says whether a weapon is in the hand or sheathed. Where a
    /// sheathed weapon hangs is decided on the client, by the item's sheath
    /// type and a table of attachment points; the server never sends it.
    pub fn sheath_state(&self) -> u8 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::BYTES_2))
            .flatten()
            .map(|v| (v & 0xFF) as u8)
            .unwrap_or(0)
    }

    /// The unit this unit's `pet` token names: `UNIT_FIELD_CHARM` if it is
    /// set, otherwise `UNIT_FIELD_SUMMON`.
    ///
    /// The 1.12.1 client resolves `"pet"` in this order: the charm guid
    /// ([`fields::unit::CHARM`]) when it is non-zero, otherwise the summon guid
    /// ([`fields::unit::SUMMON`]).
    ///
    /// Checking charm first matters for more than warlocks. A mind-controlled
    /// creature is the caster's `pet` while the control lasts, and a client
    /// that reads only `SUMMON` shows the warlock's imp on the pet frame while
    /// a different unit is under the player's control.
    ///
    /// `None` rather than `Some(0)` for a unit with neither, which makes
    /// `UnitExists("pet")` false. See [`crate::state::fields`] for why a zero
    /// guid is never a valid one.
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

    /// `UNIT_FIELD_SUMMONEDBY`: the unit that summoned this unit, which is not
    /// necessarily the unit commanding it.
    ///
    /// The counterpart of [`Self::pet_guid`]. The two differ for a
    /// mind-controlled creature: it is the caster's charm and was summoned by
    /// nobody, so it answers the `pet` token while this field stays empty.
    /// `PetCanBeAbandoned` requires this field to equal the local player's
    /// guid, so the pet menu does not offer to abandon a creature the player
    /// did not summon.
    pub fn summoned_by(&self) -> Option<u64> {
        if !self.is_unit_like() {
            return None;
        }
        let low = u64::from(self.field(fields::unit::SUMMONEDBY)?);
        let high = u64::from(self.field(fields::unit::SUMMONEDBY + 1).unwrap_or(0));
        let guid = (high << 32) | low;
        (guid != 0).then_some(guid)
    }

    /// The five values a hunter's pet panel displays, or `None` for anything
    /// that is not a pet.
    ///
    /// One accessor rather than five because the panel reads them together and
    /// the condition is the same for each: `UNIT_FIELD_PETNUMBER` is non-zero,
    /// which separates a real pet from a totem or a guardian. See the module
    /// comment of `vale_assets::tables::pet` for how each value was confirmed.
    ///
    /// ```text
    /// UNIT_FIELD_POWER5        27   the happiness value
    /// UNIT_FIELD_BYTES_1 b1   138   the loyalty level, 1..6
    /// UNIT_FIELD_PETEXPERIENCE 141
    /// UNIT_FIELD_PETNEXTLEVELEXP 142
    /// UNIT_FIELD_TRAINING_POINTS 149  two shorts: total, spent
    /// ```
    ///
    /// The training points word holds the total in its high half and the spent
    /// count in its low half. The 1.12.1 client reports the high half as the
    /// first value, and `PetPaperDollFrame` subtracts the second value from
    /// the first, so the high half is the total.
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

    /// `UNIT_FIELD_CREATED_BY_SPELL`: the spell that summoned this unit, or
    /// `None` for a unit no spell summoned.
    ///
    /// It is used for one word of text: the combat log says "%s is destroyed."
    /// rather than "%s dies." for a unit whose summoning spell has one of ten
    /// `Effect[0]` values, all of them object summons. See
    /// `vale_assets::interface::combatlog::is_destroyed`. The 1.12.1 client
    /// uses this same field for the same decision.
    pub fn created_by_spell(&self) -> Option<u32> {
        if !self.is_unit_like() {
            return None;
        }
        self.field(fields::unit::CREATED_BY_SPELL).filter(|id| *id != 0)
    }

    /// `UNIT_FIELD_PETNUMBER`: the identity of a pet that persists when it is
    /// dismissed and called back. Here it is used as the only field that marks
    /// a unit as a pet.
    ///
    /// `HasPetUI` is true when the unit exists, is not a player, and has a
    /// non-zero pet number. A guardian or a totem has no pet number and gets
    /// no pet panel, which is why the check uses this field rather than "the
    /// player has a summon".
    pub fn pet_number(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::PETNUMBER))
            .flatten()
            .unwrap_or(0)
    }

    /// `UNIT_FIELD_PET_NAME_TIMESTAMP`: when the pet was last named.
    ///
    /// The server writes `time(nullptr)` into it on a successful
    /// `CMSG_PET_RENAME` (vmangos `PetHandler.cpp`, `HandlePetRename`), and
    /// `SMSG_PET_NAME_QUERY_RESPONSE` echoes the same value. A value the name
    /// cache has not seen means the name must be queried again. Zero for a
    /// unit whose field never arrived.
    pub fn pet_name_timestamp(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::PET_NAME_TIMESTAMP))
            .flatten()
            .unwrap_or(0)
    }

    /// `UNIT_FIELD_TARGET`: the unit this unit is attacking or looking at.
    ///
    /// This is the only information the server sends about which way a
    /// creature in melee faces. `Unit::SetInFront` only calls `SetOrientation`
    /// and sends no packet, and vmangos' `TotemAI` gives the reason in a
    /// comment beside the call: "client change orientation by self". The facing
    /// rule is therefore implemented on the client, with this field as its
    /// input; see [`ObjectManager::face_targets`].
    ///
    /// It is not always the unit's attack target. For a creature that is
    /// casting, `Object::BuildValuesUpdate` substitutes `m_castingTargetGuid`
    /// into this field, so the value is the unit to look at rather than the
    /// unit being attacked. That suits facing, but it is wrong for any use that
    /// treats it as the attack target.
    ///
    /// Two `u32` fields, low word first. Zero is a valid value: the server
    /// writes `SetTargetGuid(ObjectGuid())` when a creature loses its victim
    /// and again on death. A client that treats a missing half as "unchanged"
    /// leaves a corpse facing its killer.
    pub fn target_guid(&self) -> Option<u64> {
        if !self.is_unit_like() {
            return None;
        }
        let low = u64::from(self.field(fields::unit::TARGET)?);
        let high = u64::from(self.field(fields::unit::TARGET + 1).unwrap_or(0));
        let guid = (high << 32) | low;
        (guid != 0).then_some(guid)
    }

    /// Whether this unit is auto-attacking the unit it is looking at. This is
    /// the condition for the combat-ready stance, and it requires both fields,
    /// not [`Self::attacking`] alone.
    ///
    /// The two fields usually agree. They differ in one visible case: clearing
    /// the target does not stop the attack. `ClearTarget()` sends
    /// `CMSG_SET_SELECTION` and fires the target-changed event, and does not
    /// stop the attack. vmangos' `HandleSetSelectionOpcode` cancels only an
    /// auto-repeat spell, never melee. A character whose target is cleared
    /// mid-fight therefore keeps hitting the creature, which is correct, and
    /// without this test it stood in the combat-ready stance with nothing
    /// selected.
    ///
    /// For the local player the second field is the selection, because
    /// vmangos' `SetSelectionGuid` is `SetTargetGuid`. The stance therefore
    /// drops one round trip after the click rather than on it, which the pose's
    /// 150 ms cross-fade covers.
    ///
    /// Confidence: the two field readings above are confirmed. The
    /// combination of the two is inferred from the observed behaviour; the
    /// 1.12.1 client's own condition for the stance depends on client-side
    /// state this project has not identified.
    pub fn engaged(&self) -> bool {
        self.attacking.is_some() && self.attacking == self.target_guid()
    }

    /// The spells currently on this unit: the 48 slots of `UNIT_FIELD_AURA`,
    /// with the empty ones dropped.
    ///
    /// The only part of the protocol that says a buff is active, as opposed to
    /// having been cast. `SMSG_SPELL_GO` is a single event. A client that uses
    /// only that packet knows Ice Armor was cast but not that it is still on,
    /// so a buff applied before the unit came into view is not drawn, and one
    /// that expires or is dispelled stays drawn. This field is the condition
    /// for the persistent visual; see the `stateKit` of `SpellVisualKit`.
    ///
    /// The slot count of 48 comes from the field table, not from an
    /// assumption. `UNIT_FIELD_AURAFLAGS` starts at 95, so the spell id array
    /// before it spans 95 − 47 = 48 indices, one `u32` each. A slot the server
    /// has never sent is absent from the field map, which is not the same as
    /// zero, so this filters rather than indexing directly: vmangos omits a
    /// field whose value is zero. The same rule once caused a player to be
    /// drawn magenta.
    pub fn auras(&self) -> Vec<u32> {
        self.aura_slots().into_iter().map(|aura| aura.spell).collect()
    }

    /// The same slots with everything the update fields say about each one:
    /// its slot index, its flags, the caster's level and the stack count.
    ///
    /// [`Self::auras`] serves the renderer, which needs only the spell ids.
    /// This serves the interface, which needs three more things:
    ///
    /// * The slot index, because it is the only indication of whether an aura
    ///   is a buff or a debuff. The aura blocks have no sign bit: the server
    ///   places a positive aura in the first [`POSITIVE_AURA_SLOTS`] and a
    ///   negative one above them (`SpellAuraHolder::_AddSpellAuraHolder`, two
    ///   loops with those bounds), and `UnitBuff`/`UnitDebuff` read the two
    ///   ranges.
    /// * The flags. The only bit this client reads is
    ///   [`aura_flags::CANCELABLE`]: the `CANCELABLE` / `NOT_CANCELABLE` filter
    ///   of `GetPlayerBuff`, which decides whether right-clicking an icon
    ///   removes the aura.
    /// * The applications: the number drawn on a stacking debuff. The field
    ///   holds count − 1 (stated in the comment on vmangos'
    ///   `UpdateAuraApplication`), so a single aura is stored as 0 and this adds
    ///   one. The interface tests `count > 1`.
    ///
    /// The three parallel blocks are packed differently. The levels and the
    /// applications are one byte per slot (12 `u32`s each, `slot / 4` and
    /// `(slot % 4) * 8`), and the flags are one nibble per slot (6 `u32`s,
    /// `slot >> 3` and `(slot & 7) << 2`). The field table confirms the sizes:
    /// 95 − 47 = 48 aura slots, 101 − 95 = 6 flag words, 113 − 101 = 12 level
    /// words.
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

    /// The whole `UNIT_FIELD_FLAGS` word, which decides whether a unit can be
    /// selected or attacked.
    ///
    /// Five of its bits make a unit unattackable regardless of faction (a quest
    /// giver, a flight master in flight, a creature still spawning). The list
    /// is `vale_assets::tables::faction::UNATTACKABLE_FLAGS`, because which bits
    /// they are is a game rule rather than a packet layout. Zero for a unit
    /// whose field has not arrived, which means nothing is forbidden.
    pub fn unit_flags(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::FLAGS))
            .flatten()
            .unwrap_or(0)
    }

    /// `UNIT_NPC_FLAGS`: the services this unit offers, and the only input
    /// for the cursor shown over it.
    ///
    /// Sixteen service bits (gossip, quests, vendor, trainer, flight master,
    /// innkeeper, banker, auctioneer, and so on). The client uses them for the
    /// cursor before it uses them for any panel; see
    /// `vale_assets::look::cursor::over_npc` for the confirmed order of checks.
    ///
    /// Zero for a unit that offers no service, which is most creatures, so an
    /// absent field and an ordinary creature give the same result.
    pub fn npc_flags(&self) -> u32 {
        self.is_unit_like()
            .then(|| self.field(fields::unit::NPC_FLAGS))
            .flatten()
            .unwrap_or(0)
    }

    /// The quest log: twenty slots of three update fields each.
    ///
    /// `PLAYER_QUEST_LOG_1_1` is field 198 and each slot is `(id, packed,
    /// timer)`. See [`crate::play::quest::QuestSlot::from_fields`], which
    /// decodes the packed word's four six-bit counters and explains why they
    /// are not bytes.
    ///
    /// The slots are sparse. A quest is abandoned by writing zero into its
    /// slot, so slot 4 can be empty while slot 5 holds a quest. The interface
    /// counts entries (`GetNumQuestLogEntries`), so the empty slots are removed
    /// once, here, rather than in each of the eight readers.
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

    /// `UNIT_DYNAMIC_FLAGS` bit 0: this corpse has loot.
    ///
    /// The only field that says a corpse can be looted, and it is not in
    /// `UNIT_FIELD_FLAGS`: vmangos' `UNIT_DYNFLAG_LOOTABLE` is `0x0001` of the
    /// dynamic word, set by `Creature::PrepareBodyLootState` when the loot is
    /// generated and cleared when the loot is empty. No other field
    /// distinguishes a looted corpse from a full one.
    ///
    /// The value is per viewer. The field is in the `UF_FLAG_DYNAMIC` group,
    /// so the server sends each player its own value. A corpse that another
    /// group member has already looted can read zero here while still reading
    /// one for that member. The value therefore says whether a right-click by
    /// this player would do anything, which is what the cursor needs.
    ///
    /// False for anything with no dynamic field, which includes every player.
    pub fn lootable(&self) -> bool {
        const UNIT_DYNFLAG_LOOTABLE: u32 = 0x0000_0001;
        self.is_unit_like()
            .then(|| self.field(fields::unit::DYNAMIC_FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_DYNFLAG_LOOTABLE != 0)
    }

    /// `UNIT_DYNAMIC_FLAGS` bit 1: a hunter's mark.
    ///
    /// `SPELL_AURA_MOD_STALKED` sets `UNIT_DYNFLAG_TRACK_UNIT` (`0x0002`), and
    /// the client's minimap shows any unit with it as a tracked dot, whatever
    /// the character is tracking. See `vale_assets::look::blips::tracked_unit`.
    pub fn hunters_marked(&self) -> bool {
        const UNIT_DYNFLAG_TRACK_UNIT: u32 = 0x0000_0002;
        self.is_unit_like()
            .then(|| self.field(fields::unit::DYNAMIC_FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_DYNFLAG_TRACK_UNIT != 0)
    }

    /// `UNIT_FIELD_CHARMEDBY`, read the same way as [`Self::summoned_by`]: the
    /// guid of the unit controlling a mind-controlled creature. The minimap
    /// checks it before the summoner when it classifies a unit.
    pub fn charmed_by(&self) -> Option<u64> {
        if !self.is_unit_like() {
            return None;
        }
        let low = u64::from(self.field(fields::unit::CHARMEDBY)?);
        let high = u64::from(self.field(fields::unit::CHARMEDBY + 1).unwrap_or(0));
        let guid = (high << 32) | low;
        (guid != 0).then_some(guid)
    }

    /// What the character is tracking: `PLAYER_TRACK_CREATURES`,
    /// `PLAYER_TRACK_RESOURCES` and the stealth-tracking bit of
    /// `PLAYER_FIELD_BYTES` (byte 0, bit 1). All three are `PRIVATE` fields
    /// that only arrive for the local player's character. `None` for any other
    /// unit.
    ///
    /// The three are returned together because the only consumer
    /// (`vale_assets::look::blips`) uses them together.
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

    /// `UNIT_DYNAMIC_FLAGS` bit 5: this unit is feigning death.
    ///
    /// This bit is the only thing the server sends about Feign Death.
    /// `Unit::SetFeignDeath` sets `UNIT_DYNFLAG_DEAD` (`0x0020`) on apply and
    /// clears it on expiry, and nothing else changes: the health stays the
    /// same, no bit of `UNIT_FIELD_FLAGS` changes, and there is no opcode for
    /// it. `SMSG_FEIGN_DEATH_RESISTED` reports only a failure.
    ///
    /// The 1.12.1 client draws a unit as dead when its health is zero, when
    /// this bit (bit 5 of unit field 143) is set, or when it is a corpse
    /// (`OBJECT_TYPE` 7). This applies to its display decisions, including the
    /// pose and the display blend.
    ///
    /// It is therefore a drawing rule, not a state. The client draws a
    /// feigning hunter as it draws a corpse, and everything else about the
    /// hunter (alive, can be healed, no release dialog) follows the health.
    /// That is why this is a separate accessor rather than an `||` inside
    /// [`Self::is_dead`], whose readers include the death handling.
    pub fn is_feigning(&self) -> bool {
        const UNIT_DYNFLAG_DEAD: u32 = 0x0000_0020;
        self.is_unit_like()
            .then(|| self.field(fields::unit::DYNAMIC_FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_DYNFLAG_DEAD != 0)
    }

    /// `UNIT_FLAG_IN_COMBAT`: the unit is in combat.
    ///
    /// The server reports a swing only when a blow lands, so this flag is what
    /// says a creature is in a fight between swings rather than idle.
    pub fn in_combat(&self) -> bool {
        const UNIT_FLAG_IN_COMBAT: u32 = 0x0008_0000;
        self.is_unit_like()
            .then(|| self.field(fields::unit::FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_FLAG_IN_COMBAT != 0)
    }

    /// `UNIT_FLAG_STUNNED`: the unit may not turn. `MOVEFLAG_ROOT` is a
    /// different rule: the unit may not travel.
    ///
    /// A stun sets both; other effects set one or the other. The 1.12.1 client
    /// applies them as two independent conditions on the active mover before
    /// it handles the movement keys:
    ///
    /// * Can move: health > 0, stand state not 7 (`DEAD`), and movement flags
    ///   `& 0x1200` clear (`0x1000` is `MOVEFLAG_ROOT`). This controls walking
    ///   and strafing.
    /// * Can turn: health > 0 and bit 18 of `UNIT_FIELD_FLAGS` clear, which is
    ///   `0x40000` = `UNIT_FLAG_STUNNED`. This controls turning.
    ///
    /// When `UNIT_FIELD_FLAGS` changes and bit 0x40000 is among the changed
    /// bits, the client re-evaluates its movement input at once.
    ///
    /// On the server only `HandleAuraModStun` sets it (`SpellAuras.cpp`): a
    /// root aura calls only `SetRooted(true)`, and a stun sets this flag as
    /// well. vmangos' comment on the constant says "Turn and strafe movement
    /// disabled", but the client blocks strafing under the can-move condition
    /// with the root, so only turning depends on this flag.
    pub fn is_stunned(&self) -> bool {
        self.is_unit_like()
            .then(|| self.field(fields::unit::FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_FLAG_STUNNED != 0)
    }

    /// Whether the server is flying this character on a flight path:
    /// `UNIT_FIELD_FLAGS & UNIT_FLAG_TAXI_FLIGHT` (0x00100000).
    ///
    /// It is the only taxi state that persists across a session, which is why
    /// it is read here rather than inferred from the flight. A character who
    /// logged out in flight logs back in with the flag set, and no window, key
    /// press or packet sent by this client knows about that flight.
    ///
    /// It is set only for a taxi, so honouring it cannot block walking for
    /// any other reason. vmangos sets and clears it in exactly two places,
    /// `Initialize` and `Finalize` of `FlightPathMovementGenerator`
    /// (`WaypointMovementGenerator.cpp` 396 and 363), and both times together
    /// with `UNIT_FLAG_REMOVE_CLIENT_CONTROL`, whose comment in `UnitDefines.h`
    /// says it is there to "disable player movement".
    pub fn is_on_taxi(&self) -> bool {
        self.is_unit_like()
            .then(|| self.field(fields::unit::FLAGS))
            .flatten()
            .is_some_and(|flags| flags & UNIT_FLAG_TAXI_FLIGHT != 0)
    }

    /// How a player looks: race, gender, skin tone, face, hair and beard.
    ///
    /// A player has no baked body texture: display ids 49..57 are the bare
    /// race models and `CreatureDisplayInfoExtra` has no row for them, so a
    /// client that can only look up a baked texture draws every player
    /// magenta. These are the seven numbers the 1.12.1 client composes the skin
    /// from instead; see `vale_assets::look::character`.
    ///
    /// Race and gender come from the unit block and the rest from the player
    /// block. vmangos packs each as bytes of one `u32` with
    /// `SetByteValue(field, offset)`, which indexes the value's bytes, so
    /// offset 0 is the low byte. Returns `None` for anything that is not a
    /// player, because a creature has no player block and index 187 there
    /// means something else.
    ///
    /// An absent field means zero, and zero is a valid appearance.
    /// `Object::_SetCreateBits` sets a bit only for `m_uint32Values[index] != 0`
    /// (`Object.cpp:1131`), so a create block omits every zero field. An
    /// all-zero `PLAYER_BYTES` is skin 0, face 0, hair style 0 and hair colour
    /// 0, the first option on the character-creation screen and therefore
    /// common. Requiring the field would give those characters no skin, which
    /// draws magenta. `UNIT_FIELD_BYTES_0` is required, because race 0 is not
    /// a player race, so its absence does mean the block never arrived.
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

    /// The item entries in the nineteen equipment slots, zero where empty.
    ///
    /// `Player::SetVisibleItemSlot` writes `pItem->GetEntry()` to
    /// `PLAYER_VISIBLE_ITEM_1_0 + slot * MAX_VISIBLE_ITEM_OFFSET`, and the
    /// offset is 12: the block is a creator GUID, the entry, seven
    /// enchantments, a properties pair and a pad. An entry is not a display
    /// id. The display id of a piece of armour is in the server's
    /// `item_template`, because `Item.dbc` is not in the 1.12 archives, so each
    /// distinct entry costs a `CMSG_ITEM_QUERY_SINGLE`.
    ///
    /// `None` for anything that is not a player, for the same reason as
    /// [`Self::appearance`]: a creature has no player block and those indices
    /// mean something else there. An NPC's equipment is not here either. It is
    /// baked into its `CreatureDisplayInfoExtra` texture, so an NPC in plate
    /// needs no query.
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

    /// Model id. It is at a different index for units and game objects; the
    /// per-type field modules exist to prevent reading the wrong one.
    pub fn display_id(&self) -> Option<u32> {
        match self.object_type {
            Some(ObjectType::Unit) | Some(ObjectType::Player) => {
                self.field(fields::unit::DISPLAYID)
            }
            Some(ObjectType::GameObject) => self.field(fields::game_object::DISPLAYID),
            _ => None,
        }
    }

    /// The spell of this persistent area object, and the area's radius.
    ///
    /// The server creates a `DynamicObject` for a Blizzard, a Flamestrike, a
    /// Rain of Fire or a Consecration. It has no display id and no model, and
    /// these two fields are all it states about itself. Its appearance is the
    /// spell's art, resolved from the archives
    /// (`vale_assets::tables::spell::SpellVisuals::ground_art`). A client that
    /// reads neither field draws nothing where the ground effect should be.
    ///
    /// `None` for every other object type, for the same reason
    /// [`Self::display_id`] is per type: the index tables overlap and index 9
    /// means something different in each.
    ///
    /// A radius of zero is valid. This accessor used to require a positive
    /// radius, on the assumption that zero would shrink the art to nothing,
    /// but the radius does not scale the art. The first paragraph of
    /// `world::entities::effects` explains this: the client scales the art only
    /// for a `DYNAMICOBJECT_BYTES` that is neither 1 nor 2, and vmangos writes
    /// 1 for every area aura. The radius affects only how impacts are
    /// distributed.
    ///
    /// The old filter hid all of a far-sight spell's visible effect.
    /// `Player::SetLongSight` creates its `DynamicObject` with a radius of zero
    /// (`DynamicObject::Create(..., 0, 0, DYNAMIC_OBJECT_FARSIGHT_FOCUS)`), and
    /// Eagle Eye's `SpellVisual` names `Spells\FarSight_Impact_Base.m2`, so the
    /// object arrived, was placed and resolved to art, and was then discarded
    /// before it was drawn.
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
    /// A heuristic, because the wire format is the same in both modes and the
    /// client is not told which mode the server uses. A max health of exactly
    /// 100 is the indicator, since vmangos substitutes that constant when it
    /// hides real values.
    pub fn health_is_percentage(&self) -> bool {
        self.max_health() == Some(100)
    }

    /// Is the server currently moving this entity?
    ///
    /// It checks the two sources [`ObjectManager::advance`] dead-reckons from:
    /// a spline the server put the unit on, or the movement flags of its last
    /// `MSG_MOVE_*` broadcast. Nothing else moves an entity, so this says
    /// whether its position is changing, from the stated movement rather than
    /// from observed positions.
    ///
    /// A renderer should use this rather than the difference between two
    /// positions it is interpolating. That difference is zero for the last
    /// frames of every interpolation window, because the window ends when the
    /// next snapshot is due, not when it arrives, so an animation chosen from
    /// it restarts several times a second. Not true for the local player: the
    /// session's [`crate::state::movement::Mover`] owns that one and reports it
    /// separately.
    pub fn is_moving(&self) -> bool {
        self.spline.is_some() || self.effective_movement().is_some_and(|m| m.is_moving())
    }

    /// Whether this unit is off the ground, and whether it jumped.
    ///
    /// This client never treats a unit on a server spline as airborne:
    /// `SMSG_MONSTER_MOVE` says where a creature will be and never why, so a
    /// leaping creature follows a path through the air with nothing to
    /// distinguish it. This applies to other players, whose `MSG_MOVE_JUMP`
    /// broadcasts state both facts.
    ///
    /// The vertical speed separates a jump from a fall. The wire is
    /// down-positive, so a jump reports a `jump.zspeed` of minus
    /// [`crate::state::movement::JUMP_SPEED`]. A step off a ledge reports zero,
    /// and a knockback reports whatever speed it applied.
    pub fn is_airborne(&self) -> bool {
        self.spline.is_none()
            && self
                .effective_movement()
                .is_some_and(|m| m.has(move_flags::JUMPING))
    }

    pub fn is_jumping(&self) -> bool {
        self.is_airborne() && self.effective_movement().is_some_and(|m| m.jump.z_speed < 0.0)
    }

    /// How fast, in yards per second, the client is moving this entity. Zero
    /// when it is not moving, which selects Stand over Walk.
    ///
    /// The same number the dead reckoning uses, so the animation and the drawn
    /// position agree about whether a unit is running.
    pub fn ground_speed(&self) -> f32 {
        if let Some(spline) = self.spline.as_ref() {
            return spline.speed();
        }
        match self.effective_movement() {
            Some(info) if info.is_moving() => info.speed(&self.speeds.unwrap_or_default()),
            _ => 0.0,
        }
    }

    /// The movement flags the gait is chosen from. They come from the same
    /// block [`Self::ground_speed`] and [`Self::is_moving`] read, so the gait,
    /// the speed and the direction always agree.
    ///
    /// Flags rather than a direction, because the client's gait rules depend
    /// on the flags and the precedence differs between ground and water. On
    /// the ground backward takes precedence over a strafe; in water a strafe
    /// takes precedence over backward and a turn over both. A four-way
    /// direction enum cannot express that, and the enum this replaced applied
    /// the ground order in both places.
    ///
    /// A unit on a spline reads as `FORWARD`: `SMSG_MONSTER_MOVE` states a
    /// path and no flags, and a creature walks its patrol facing the direction
    /// of travel. Without this substitution a patrolling creature has no
    /// direction bit set and is drawn standing while it slides along its route.
    ///
    /// A flying spline also reads as `FLYING`, by the same substitution.
    /// `MoveSplineFlag::Flying` is how the server marks a taxi flight and a
    /// flying creature's path, and the renderer picks the animation from it.
    /// Without it every flying unit plays its run animation in the air, at its
    /// spline speed divided by the 6.9 authored for `Run`.
    ///
    /// The server's separate flag packets take precedence over both
    /// substitutions; [`Self::forced_flags`] holds them.
    /// `SMSG_SPLINE_MOVE_ROOT` about a creature arrives with no movement block,
    /// and without this the spline substitution above would keep reporting
    /// `FORWARD` for a unit the server has just rooted.
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

    /// The moving platform this unit is riding, or `None` for a unit on the
    /// ground.
    ///
    /// The transport guid under `MOVEFLAG_ONTRANSPORT`, read directly from the
    /// movement block. The server writes both halves for every passenger it
    /// broadcasts, and `HandleMoverRelocation` treats a passenger's `t_pos` as
    /// its position. See [`crate::state::movement::Ferry`].
    ///
    /// Not used for the local player, whose movement the server never echoes:
    /// that is `SessionStatus::ferry`, and the caller combines the two.
    ///
    /// The flag is tested rather than trusting the block, because
    /// [`crate::state::movement::MovementInfo::transport`] is only filled in
    /// under the flag and a stale value would name a platform the unit has
    /// left.
    pub fn platform_guid(&self) -> Option<u64> {
        let info = self.movement?;
        if info.flags & crate::state::movement::move_flags::ONTRANSPORT == 0 {
            return None;
        }
        info.transport.map(|t| t.guid)
    }

    /// [`Self::movement`] with [`Self::forced_flags`] applied: the block the
    /// dead reckoning runs on.
    ///
    /// Drawing and movement both use this accessor, so the flags a unit is
    /// drawn with and the flags it is moved with cannot disagree. Before it
    /// existed, `move_flags` applied the override and `advance` read the raw
    /// field, so a rooted player was drawn standing and still moved.
    pub fn effective_movement(&self) -> Option<MovementInfo> {
        let mut info = self.movement?;
        if !self.forced_flags.is_empty() {
            info.flags = self.forced_flags.over(info.flags);
        }
        Some(info)
    }

    /// Is this unit in water deep enough to swim in?
    ///
    /// `MOVEFLAG_SWIMMING`, which the server sets on a creature it moves
    /// through water and which a player's own client sets and broadcasts. It is
    /// a flag rather than a depth, so this is the stated state rather than an
    /// estimate from the liquid surface. It is also the field the gait comes
    /// from, so the two agree about what a unit is doing.
    ///
    /// A unit on a spline has no movement block of its own, and the result is
    /// then false: `SMSG_MONSTER_MOVE` carries no flags. This is a known gap: a
    /// swimming creature on a spline does not play its swim animation. A
    /// swimming player is not affected.
    ///
    /// It reads [`Self::effective_movement`] rather than the raw block, so the
    /// two packets that would set it for a creature,
    /// `SMSG_SPLINE_MOVE_START_SWIM` and `_STOP_SWIM`, take effect if they are
    /// ever sent. vmangos never sends them, which is why the gap remains. It
    /// deliberately does not read [`Self::move_flags`], which substitutes a
    /// spline's flags for the block's and would stop a swimmer's swim animation
    /// as soon as the server put it on a path.
    pub fn is_swimming(&self) -> bool {
        self.effective_movement()
            .is_some_and(|m| m.flags & crate::state::movement::move_flags::SWIMMING != 0)
    }

    /// The body's vertical angle, in radians, up-positive.
    ///
    /// The movement block's `pitch`, which the packet carries only under
    /// `MOVEFLAG_SWIMMING`. It is zero for any unit not in water; reading it
    /// unconditionally would tilt creatures by whatever value the field held.
    /// The check is here rather than at the call site so that the rule exists
    /// in one place.
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

/// Templates answered since the last check. This is the only event this
/// manager records; everything else in it is state.
///
/// A template arriving changes no field a reader polls, so a panel that read
/// a name once and got nothing cannot learn that it should read again. That
/// is why an item seen for the first time had no name, and the same applies
/// to a quest objective naming a creature that has never been in view. See
/// `crate::game::templates` on the client side, which documents how the
/// 1.12.1 client notifies a panel when a template arrives.
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

    /// Whether any template a quest log row is built from has just arrived.
    pub fn names_a_quest_objective(&self) -> bool {
        !self.creatures.is_empty() || !self.gameobjects.is_empty() || !self.items.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct ObjectManager {
    entities: HashMap<u64, Entity>,
    /// Which simulation step the standing pass is on. See [`STAND_BEAT`].
    stand_beat: u8,
    /// GUID of the object flagged `UPDATEFLAG_SELF`: the local player.
    pub player_guid: Option<u64>,
    /// Creature templates resolved by `CMSG_CREATURE_QUERY`, keyed by entry.
    /// Cached because entries repeat often: every wolf in a zone shares one.
    pub creatures: HashMap<u32, CreatureInfo>,
    /// The same, for `CMSG_GAMEOBJECT_QUERY`.
    pub gameobjects: HashMap<u32, GameObjectInfo>,
    /// Players resolved by `CMSG_NAME_QUERY`, keyed by GUID rather than entry:
    /// a player has no shared template, so there is nothing to deduplicate.
    pub players: HashMap<u64, PlayerInfo>,
    /// Every player the last `SMSG_GROUP_LIST` named, whether or not they are
    /// in the world.
    ///
    /// [`Self::unresolved_player_guids`] cannot find these by walking the
    /// entities: a raid member on another continent has no entity, and the
    /// roster packet carries a name and a status byte but no class. The class
    /// is needed for a raid button's class label and its `RAID_CLASS_COLORS`
    /// colour. The 1.12.1 client's `GetRaidRosterInfo` takes both from its
    /// name cache, so a group member is resolved by name query like any other
    /// player.
    pub group_guids: Vec<u64>,
    /// Every player on the friends and ignore lists, for the same reason as
    /// [`Self::group_guids`].
    ///
    /// Neither `SMSG_FRIEND_LIST` nor `SMSG_IGNORE_LIST` carries a name (see
    /// [`crate::play::social`]), and a friend is usually on another continent,
    /// so the entity walk cannot find them either.
    ///
    /// Two lists rather than one, because the two packets arrive separately
    /// and each replaces its own list. In a single vector, an ignore list
    /// arriving after a friends list would remove the friends. A guid on both
    /// lists is queried once, because the query pass filters on the name
    /// cache, not on these lists.
    pub friend_guids: Vec<u64>,
    pub ignore_guids: Vec<u64>,
    /// Guids named by a friends or ignore list change, for which neither list
    /// has been sent again.
    ///
    /// `SMSG_FRIEND_STATUS` is the only announcement of an addition and the
    /// server does not resend the list, so without this a friend added
    /// mid-session has no name until the next login.
    pub wanted_social: HashSet<u64>,
    /// Item templates resolved by `CMSG_ITEM_QUERY_SINGLE`, keyed by entry.
    /// This deduplicates well: every guard in a city wears the same
    /// breastplate.
    pub items: HashMap<u32, ItemInfo>,
    /// Entries the server refused to describe, so they are queried once and
    /// not every frame. vmangos answers `entry | 0x80000000` for an item it
    /// will not describe, which is a valid answer rather than a lost packet.
    pub unknown_items: HashSet<u32>,
    /// Item entries that code outside the world state asked to have named.
    ///
    /// Everything else in this manager comes from the server; this set is
    /// filled by the interface. A spell's reagents are item entries
    /// (`Spell.dbc` holds 17031, not "Rune of Teleportation", and `Item.dbc`
    /// is not in the archives), so a tooltip that names one must use the same
    /// query queue as a worn breastplate. See [`Self::want_item`].
    pub wanted_items: HashSet<u32>,
    /// Something arrived that may need a name. Cleared by the session's query
    /// pass; see [`Self::take_query_hint`] for the reason.
    queries_wanted: bool,
    /// The same kind of set for the other tables a name can come from. Only
    /// the quest log fills these: an objective is `ReqCreatureOrGOId` plus a
    /// count, and a name such as "Kobold Vermin" exists only in the answer to
    /// `CMSG_CREATURE_QUERY`. See [`Self::want_creature`].
    pub wanted_creatures: HashSet<u32>,
    pub wanted_gameobjects: HashSet<u32>,
    /// Query answers learned this session that are not on disk yet, as the
    /// response bodies they arrived as. The session loop drains them into
    /// [`crate::play::wdb`].
    ///
    /// Bodies rather than parsed records, because the cache replays them
    /// through the same parser. See that module, which also explains why a
    /// persistent cache is the only fix for a blank loot row.
    ///
    /// Off unless something drains it. A snapshot pump has no cache to write
    /// to and would otherwise keep a copy of every answer it received.
    pub record_cache: bool,
    pub learned: Vec<Learned>,
    /// Every `(kind, key)` on disk at start-up or recorded this session, so
    /// an answer the server restates is not written twice. See
    /// [`Self::remember`].
    cache_known: HashSet<(Kind, u64)>,
    /// The bodies of the on-demand kinds ([`Kind::on_demand`]), from which the
    /// session loop answers a query instead of sending it. See
    /// [`Self::cached_answer`].
    on_demand: HashMap<(Kind, u64), Vec<u8>>,
    /// Entries whose template answer arrived since the last check, refusals
    /// included. Drained by [`Self::take_item_arrivals`].
    ///
    /// This is the only event this manager records. Everything else here is
    /// state that a reader compares against. A template arriving changes no
    /// polled field, so a panel that read an item's name once and got nothing
    /// cannot learn that it should read again. That is why the first item of a
    /// kind had no name: the query was sent, the answer arrived, and the window
    /// still showed the blank.
    ///
    /// The 1.12.1 client notifies the waiting panel when the answer arrives and
    /// re-raises the panel's event, so the panel reads again. This client uses
    /// a queue for the same purpose, because a packet handler here runs under
    /// the world lock and cannot raise a Bevy message.
    ///
    /// A refusal also counts as an arrival. Otherwise a panel waiting on an
    /// entry the server will never describe is never told to stop waiting.
    arrivals: Arrivals,
    /// The quest marker over each quest giver's head, by guid. See
    /// [`Self::set_quest_status`] for why this is state rather than an event.
    quest_status: HashMap<u64, crate::play::quest::DialogStatus>,
    /// Every guid that has been queried, which is a different set: a giver
    /// with nothing for this character answers `None` and is left out of the
    /// map above, and without this set the client would query it again every
    /// frame.
    asked_quest_status: HashSet<u64>,
    /// Whether each flight master's node is one this character has
    /// discovered, by guid. This is the other source of a model over a unit's
    /// head. See [`Self::set_taxi_status`].
    ///
    /// A separate map from the quest one, rather than one "marker on this
    /// unit" map, because the two answers arrive independently and either can
    /// be known while the other has not been queried. On the rare unit that
    /// has both, the renderer decides which is shown; the 1.12.1 client shows
    /// whichever packet arrived last.
    taxi_status: HashMap<u64, bool>,
    /// Every flight master already queried, on the same terms as
    /// [`Self::asked_quest_status`]: a discovered node answers `true` and is
    /// kept, and without this set the client would query once a frame.
    asked_taxi_status: HashSet<u64>,
    /// How many positions were refused as impossible. See
    /// [`Entity::set_server_position`]. It should be zero; any other value
    /// means a packet is being read at the wrong offset. The count is exposed
    /// because otherwise the only symptom is an entity that disappears.
    pub rejected_positions: u32,
    /// How many server statements moved an entity more than [`JUMP_YARDS`], and
    /// a capped sample of them naming the packet responsible.
    ///
    /// The purpose is attribution. Creatures that appear to teleport can have
    /// many causes (a misparsed spline, a dropped packet, an interpolation
    /// fault), and watching the screen cannot rule any of them out. A creature
    /// name, a distance and the opcode identify the cause.
    pub jump_count: u32,
    pub jump_log: Vec<String>,
    /// Chat that has arrived and not yet been shown, oldest first.
    ///
    /// Drained by its reader, not cloned. Everything else here is state (a
    /// position, a template, a count) and can be read repeatedly. A line of
    /// chat is an event that must be shown exactly once, so the reader takes it
    /// ([`Self::take_chat`]) and this becomes empty. `swings_thrown` handles
    /// the same distinction by being a counter.
    ///
    /// Bounded, because a reader is not guaranteed: `vale live` polls a few
    /// times a second and the renderer every frame, but a caller that only
    /// enters the world and waits would otherwise accumulate a city's combat
    /// log for the whole session.
    chat: Vec<crate::play::chat::ChatMessage>,
    /// The world clock, as the server stated it at login.
    ///
    /// State, and the only state the server states exactly once. It is held
    /// here rather than in the session loop because it belongs to the world,
    /// not to this connection (the same reason the creature templates are
    /// here), and because `SMSG_NEW_WORLD` does not restate it, so a teleport
    /// across a continent must not lose it. See [`crate::play::time`] for what
    /// the packet carries, and [`crate::play::time::GameTime::advanced`] for
    /// how it advances afterwards.
    pub game_time: Option<crate::play::time::GameTime>,
    /// The current weather: the last `SMSG_WEATHER`, kept as state for the
    /// same reason as [`Self::game_time`]. The server sends it on a zone
    /// change and then only when it rolls a new grade, minutes apart, so a
    /// reader that missed the packet would draw a clear sky during a storm.
    /// `None` until the first packet, which means a clear sky.
    pub weather: Option<crate::play::weather::Weather>,
    /// Where the hearthstone returns the character to.
    ///
    /// Held here for the same reason as the clock: `SMSG_BINDPOINTUPDATE`
    /// arrives once in the login burst and again only when the home is
    /// changed, so a reader that missed it shows a hearthstone tooltip with a
    /// missing location and no error. `None` before the login burst has
    /// arrived. See [`crate::play::bindpoint`].
    pub bind_point: Option<crate::play::bindpoint::BindPoint>,
    /// The character's spellbook, as the server stated it at login and has
    /// amended since.
    ///
    /// State rather than an event, held here for the same reason as the
    /// clock: it is sent once, in the login burst, and never restated, so a
    /// reader that missed it shows an empty action bar and no error. See
    /// [`crate::play::spells::parse_initial_spells`].
    pub spellbook: crate::play::spells::Spellbook,
    /// The action bar the server stores for this character, occupied slots
    /// only. Cosmetic state: the client owns the bar, and this is only how the
    /// last session left it.
    pub action_buttons: Vec<crate::play::spells::ActionButton>,
    /// Incremented whenever either of the two above changes, so a reader can
    /// detect a new spellbook without comparing the lists.
    pub spellbook_version: u32,
    /// The pet's action bar, as `SMSG_PET_SPELLS` last stated it. See
    /// [`crate::play::pet`].
    ///
    /// Empty until there is a pet, and emptied again by the eight-byte
    /// dismissal the server sends when it goes. It is held here rather than on
    /// the pet's [`Entity`] because of the packet order: on a summon the packet
    /// arrives before the pet's create block, and on a dismissal after the pet
    /// has left the world, so at both moments there is no entity to store it
    /// on.
    pub pet: crate::play::pet::PetSpells,
    /// The name the player gave the pet, keyed as the server keys it: by
    /// `UNIT_FIELD_PETNUMBER` rather than a guid, so the name survives a
    /// dismissal. The second value is the response's copy of
    /// `UNIT_FIELD_PET_NAME_TIMESTAMP`. When the field moves past it, a rename
    /// has happened, so [`Self::unresolved_pet_names`] queries again.
    pub pet_names: HashMap<u32, (String, u32)>,
    /// Pet numbers a name has been queried for, with the field timestamp the
    /// query was made against. One query per number per timestamp, so a
    /// server that does not answer is not queried again until the field moves.
    pub wanted_pet_names: HashMap<u32, u32>,
    /// The same kind of version counter as [`Self::spellbook_version`]:
    /// resolving the bar needs a `Spell.dbc` lookup per slot, which is cheap
    /// once and too costly sixty times a second.
    pub pet_version: u32,
    /// The pet was told to attack and has not been told to stop. In the
    /// 1.12.1 client, `CastPetAction` sets this when the attack command is
    /// used on a valid target, and `IsPetAttackActive` returns it. Cleared by
    /// `CMSG_PET_STOP_ATTACK` and by the next `SMSG_PET_SPELLS`. See
    /// [`Self::apply_pet_press`].
    pub pet_attacking: bool,
    /// Incremented whenever any part of the inventory changes: a create or
    /// values block for an `Item` or a `Container`, or one that touches the
    /// local player's three slot ranges.
    ///
    /// The same kind of version counter as [`Self::spellbook_version`], and
    /// more necessary here: [`crate::play::items::Inventory::read`] follows
    /// three levels of GUID and allocates, so rebuilding it every frame would
    /// cost work sixty times a second for bags nobody opened. No packet
    /// announces an inventory change, so this counter serves as the
    /// announcement. See [`crate::play::items`], whose module comment explains
    /// how the inventory is derived.
    pub inventory_version: u32,
    /// The clocks the server has started on carried items: temporary
    /// enchantments and expiring items. See [`crate::play::items::ItemTimers`].
    /// Setting one bumps [`Self::inventory_version`], because the snapshot
    /// copies the expiry times.
    pub item_timers: crate::play::items::ItemTimers,
    /// The unit the local player is auto-attacking, per `SMSG_ATTACKSTART` and
    /// `SMSG_ATTACKSTOP` about the local player.
    ///
    /// The server's decision, not the client's request. Pressing Attack sends
    /// a request and the server decides whether the attack starts; this field
    /// holds the result, and the Attack button's highlight follows it. `None`
    /// before the player object exists, since the guid to compare against is
    /// not known yet.
    pub attacking: Option<u64>,
    /// The server's responses to the local player's actions, oldest first.
    /// Drained by the interface, in the same way as [`Self::take_chat`].
    events: Vec<crate::play::spells::PlayerEvent>,
    /// Combat events, oldest first: the combat log's queue, drained by the
    /// code that composes its lines. See [`Self::note_combat`] for why this is
    /// separate from the field above.
    combat: Vec<crate::play::combatlog::CombatEvent>,
    /// Remaining duration of the local player's own auras, keyed by
    /// `UNIT_FIELD_AURA` slot.
    ///
    /// `SMSG_UPDATE_AURA_DURATION` is the only source of this in the 1.12
    /// protocol, and it is sent to the aura's target only when that target is
    /// a player (`SpellAuraHolder::UpdateAuraDuration`). This therefore holds
    /// the local player's buff timers, and there is no equivalent for other
    /// units. That is not a gap in this client: the game's target frame and
    /// party frames show no timers either, for the same reason.
    ///
    /// It is state rather than an event, unlike the other responses to the
    /// player's actions, because the interface asks for the remaining time
    /// every frame and never asks whether a duration arrived. Keyed by slot
    /// because the packet carries a slot and no spell id.
    aura_durations: HashMap<u8, AuraDuration>,
}

/// One `SMSG_UPDATE_AURA_DURATION`, as [`ObjectManager`] keeps it.
#[derive(Debug, Clone, Copy)]
pub struct AuraDuration {
    /// How long the aura had left when the packet arrived. On an apply or a
    /// refresh this is the whole duration; the packet is also sent on a
    /// pushback, where it is not.
    pub remaining_ms: u32,
    /// When the packet arrived, so a reader can subtract. `Instant` rather than
    /// a session clock, because this crate has no frame clock and the
    /// consumer's time base (`GetTime()`) differs. The reader converts with one
    /// subtraction.
    pub received: Instant,
    /// Incremented once per packet for this slot, so a reader can tell a
    /// refresh from the same value read again. Without it a buff re-applied at
    /// its full duration looks the same as one that has been running.
    pub seq: u32,
}

/// How far a standing unit may be pulled down onto the surface below it. See
/// [`ObjectManager::stand_on_the_ground`] for the full reasoning.
///
/// Two yards, chosen by judgement rather than measured. It must exceed the
/// world database's largest ordinary spawn error: Eagan Peltskinner's is 0.77,
/// and among the 14,193 stationary ground spawns on maps 0 and 1 the errors
/// reach about 1.9 before the next values are a whole storey. It must also
/// stay well below the height of anything a unit could really be standing on
/// for which this client has no collision hull. A dais, a gangway or a flying
/// creature is metres up, not one.
///
/// The 1.12.1 client very probably has no such tolerance, because it loads a
/// collision hull for everything it draws, so its floor query always finds the
/// real surface. This band compensates for missing collision in this client;
/// the proper fix is to add the missing hulls, not to raise this value.
const STAND_BAND: f32 = 2.0;

/// How many simulation steps apart the standing pass runs.
///
/// It exists for entities whose ground has not arrived yet. Those cannot be
/// latched (see [`ObjectManager::stand_on_the_ground`]), so they are checked
/// again until the tile under them loads. At login every entity in view is
/// created before any terrain, so that is every entity at once. Eight steps
/// is five times a second, which settles a spawn within a fifth of a second of
/// its ground arriving; the delay is not visible.
const STAND_BEAT: u8 = 8;

/// The smallest height above the ground worth correcting. Below this the drop
/// is invisible and applying it only changes the position for anything
/// watching it. The database's systematic offset is +0.18, so this value is
/// deliberately below it: the most common spawn is a fifth of a yard up and
/// should be put down.
const STAND_EPSILON: f32 = 0.05;

/// How long an arc may be extrapolated before the client stops.
///
/// `Unit::ExtrapolateMovement` stops after ten seconds for the same
/// reason: after that the landing packet has been lost, and continuing to
/// integrate only drives the unit into the ground.
const MAX_ARC_SECS: f32 = 10.0;

/// How many jump samples to keep. Enough to see a pattern, and few enough
/// that a faulty session does not grow a `Vec` without bound.
const JUMP_SAMPLES: usize = 16;

/// How many undrained chat lines to keep. A busy city's combat log produces a
/// few a second, and a reader that has not read in a hundred lines will not
/// be helped by the hundred and first.
const CHAT_BACKLOG: usize = 100;

/// The same bound for [`ObjectManager::note_event`]. Smaller, because these are
/// responses to the player's actions: a player produces a few a second at
/// most, and a reader that has not read thirty-two of them is no longer
/// reading.
const EVENT_BACKLOG: usize = 32;

/// The same bound for [`ObjectManager::note_combat`], the largest of the three.
///
/// It is a separate queue rather than part of [`EVENT_BACKLOG`] because a
/// combat log line can be about any unit in sight, not only the local
/// player's actions. A five-player pull in a city produces tens a second
/// while the action queue produces one. A shared queue would let a fight push
/// every response to an action out before anything read it, and that loss
/// would look like an unrelated bug.
const COMBAT_BACKLOG: usize = 200;

/// How many releases [`Entity::recent_spells`] remembers.
///
/// A renderer's depth, not a protocol value. It must cover a spell that
/// triggers another within one poll, which is two. Four leaves room for a
/// chain and costs sixteen bytes in a snapshot.
pub const RECENT_SPELLS: usize = 4;

/// How many `UNIT_FIELD_AURA` slots a unit has: 48, computed from the field
/// table rather than assumed (`UNIT_FIELD_AURAFLAGS` at 95 minus
/// `UNIT_FIELD_AURA` at 47).
pub const MAX_AURA_SLOTS: u8 = (fields::unit::AURAFLAGS - fields::unit::AURA) as u8;

/// How many of the aura slots are buffs. The first 32 slots hold positive
/// auras and the 16 above them negative ones. The slot is the only way a
/// client can tell a buff from a debuff; none of the four aura blocks has a
/// sign bit.
///
/// vmangos allocates by exactly these bounds
/// (`SpellAuraHolder::_AddSpellAuraHolder`: `for i in 0..MAX_POSITIVE_AURAS`
/// for a positive holder, `for i in MAX_POSITIVE_AURAS..MAX_AURAS` for a
/// negative one), and `MAX_POSITIVE_AURAS` is 32 in `SpellAuraDefines.h`.
pub const POSITIVE_AURA_SLOTS: u8 = 32;

/// The nibble of `UNIT_FIELD_AURAFLAGS`: `AFLAG_*` in vmangos'
/// `SpellAuraDefines.h`. This client reads one bit of it.
pub mod aura_flags {
    /// `AFLAG_CANCELABLE`: right-clicking the icon removes the aura.
    ///
    /// The server sets it for a positive aura without
    /// `SPELL_ATTR_NO_AURA_CANCEL` (`SpellAuraHolder::SetAuraFlag`), so it
    /// means both "this is a buff" and "the player may remove it". The
    /// `CANCELABLE` / `NOT_CANCELABLE` filter of `GetPlayerBuff` selects on it.
    pub const CANCELABLE: u8 = 0x01;
    /// The three effect-slot bits, `AFLAG_EFF_INDEX_0`..`2`. Nothing here
    /// reads them; they are named so that a printed flag word is readable.
    pub const EFF_INDEX_0: u8 = 0x08;
    pub const EFF_INDEX_1: u8 = 0x04;
    pub const EFF_INDEX_2: u8 = 0x02;
}

/// One occupied aura slot on a unit. See [`Entity::aura_slots`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuraSlot {
    /// Which of the 48 slots, which also says whether this is a buff. See
    /// [`POSITIVE_AURA_SLOTS`].
    pub slot: u8,
    /// The `Spell.dbc` id.
    pub spell: u32,
    /// [`aura_flags`].
    pub flags: u8,
    /// The caster's level. Kept because the packet carries it. Nothing in this
    /// client reads it yet, and the 1.12 interface has no function that
    /// exposes it.
    pub level: u8,
    /// The stack count, one-based. The field holds count − 1.
    pub applications: u8,
}

impl AuraSlot {
    /// A buff rather than a debuff, by the slot it is in.
    pub fn helpful(&self) -> bool {
        self.slot < POSITIVE_AURA_SLOTS
    }

    /// Whether right-clicking it removes the aura ([`aura_flags::CANCELABLE`]).
    pub fn cancelable(&self) -> bool {
        self.flags & aura_flags::CANCELABLE != 0
    }
}

/// The values a hunter's pet panel reads from the pet's unit fields. See
/// [`Entity::pet_stats`], which lists every field index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PetStats {
    /// `UNIT_FIELD_POWER5`. Signed, because the client compares it against the
    /// happiness thresholds as a signed value.
    pub happiness: i32,
    /// 1..6, indexing `PetLoyalty.dbc`. Zero for a pet that has none yet.
    pub loyalty_level: u32,
    pub experience: u32,
    pub next_level_experience: u32,
    /// The high half of `UNIT_FIELD_TRAINING_POINTS`. See
    /// [`Entity::pet_stats`].
    pub training_total: u16,
    pub training_spent: u16,
}

/// `enum Powers` (`SharedDefines.h`). The value is both the fourth byte of
/// `UNIT_FIELD_BYTES_0` and the offset from `UNIT_FIELD_POWER1`, which is why
/// [`Entity::power_field`] can compute the field index by addition.
pub mod power_type {
    pub const MANA: u32 = 0;
    pub const RAGE: u32 = 1;
    pub const FOCUS: u32 = 2;
    pub const ENERGY: u32 = 3;
    /// A hunter pet's power. The player frame never draws it.
    pub const HAPPINESS: u32 = 4;

    /// Rage is stored in tenths, and the protocol does not indicate this.
    ///
    /// A warrior's `UNIT_FIELD_POWER2` runs 0..1000 for the hundred points the
    /// interface shows, and the cost column of `Spell.dbc` uses the same units:
    /// 100 for Shield Block's ten rage. vmangos multiplies explicitly
    /// (`ModifyPower(POWER_RAGE, addRage * 10)`), so a client that shows the
    /// raw field reports a warrior at "1000/1000 rage" and every ability at ten
    /// times its cost.
    ///
    /// Mana, focus and energy are one for one; happiness belongs to a pet and
    /// is not drawn. Applied to both a bar and a cost, which is why it takes a
    /// plain number rather than belonging to either.
    pub fn display(kind: u8, value: u32) -> u32 {
        if u32::from(kind) == RAGE {
            value / 10
        } else {
            value
        }
    }

    /// The label for a bar. The keys are the game's `GlobalStrings.lua` keys
    /// `MANA`, `RAGE`, `FOCUS`, `ENERGY` and `HAPPINESS`.
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

/// The kind of object a guid names, from its top sixteen bits.
///
/// The high word of `ObjectGuid`, which the server builds into every guid it
/// sends. It is the only way to tell an item from a creature without a
/// lookup. The four values 1.12 sends to this client:
///
/// ```text
/// 0x0000  a player          — the low bits are the character's own id
/// 0x4000  an item           — in a bag, and therefore nowhere in the world
/// 0xF110  a game object     — a door, a chest, an ore vein
/// 0xF130  a creature        — an NPC, and the entry is in the middle bits
/// ```
///
/// This function exists because an item has no position. The server treats
/// an item as an ordinary quest giver (`HandleQuestgiverQueryQuestOpcode`
/// looks a guid up through `TYPEMASK_CREATURE_GAMEOBJECT_OR_ITEM`), so a quest
/// page can come from an object that is not in the world. Every client-side
/// rule that measures the distance to the current quest giver must allow for
/// that.
pub fn guid_high(guid: u64) -> u16 {
    (guid >> 48) as u16
}

/// `HIGHGUID_ITEM`. See [`guid_high`].
pub const HIGHGUID_ITEM: u16 = 0x4000;
/// `HIGHGUID_GAMEOBJECT`.
pub const HIGHGUID_GAMEOBJECT: u16 = 0xF110;
/// `HIGHGUID_UNIT`.
pub const HIGHGUID_UNIT: u16 = 0xF130;

/// Whether this guid names an object that has a position in the world.
///
/// `false` for an item, which is in a bag; `true` for everything else,
/// including a player (whose high word is zero). A range check must make this
/// test before it concludes that a guid which resolves to nothing means the
/// object has moved away.
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
    /// Never removes the local player: the server marks the player out of
    /// range during a teleport, and dropping that entry would lose the
    /// session's anchor, leaving the live loop nothing to dead-reckon from.
    ///
    /// An item being removed is an inventory change, and this is the path
    /// items are removed by. `SMSG_DESTROY_OBJECT` is a bare guid handled
    /// outside [`Self::apply`] (`socket::handler::world::destroy`), so the
    /// version bump in the out-of-range branch never ran for items. An item is
    /// not in the grid, so no `OUT_OF_RANGE` block names one, while every
    /// server-side destruction of a carried item (a stack merged into another,
    /// a charge used up, a quest item taken, anything moved out of the
    /// inventory) sends this opcode. Incrementing the version here clears the
    /// slot on the frame the packet arrives rather than at the next field
    /// change.
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
    /// A far teleport (`SMSG_NEW_WORLD`) does not destroy the old map's objects
    /// one by one. The player is removed from that map, so no
    /// `SMSG_DESTROY_OBJECT` and no out-of-range block arrives for any of them.
    /// If they are not removed here they stay in the world: a Stormwind guard
    /// is drawn in the Barrens, at coordinates that are valid on the map he is
    /// no longer on, dead-reckoned along a spline that ended elsewhere. There
    /// is no warning, because the server simply stops mentioning them.
    ///
    /// The player and the template caches are kept. The player is the
    /// session's anchor ([`ObjectManager::remove`] refuses to drop it for the
    /// same reason). A creature template, an item's appearance or another
    /// player's name is keyed by entry or GUID and is valid on any map, so
    /// discarding them would only cause a burst of repeated queries on arrival.
    ///
    /// The transport the character is standing on is also kept.
    /// `Map::SendInitTransports` builds a create block for every transport on
    /// the new map except `player->GetTransport()`, so the boat a character
    /// crossed on is the one object the new map never states. The 1.12.1
    /// client keeps its transport object across `SMSG_NEW_WORLD`, so it does
    /// not need one. Dropping it here would lose the entry and the path
    /// progress that locate a continent transport, and would leave the
    /// passenger standing at deck height over open water until
    /// `PLATFORM_HOLD` runs out.
    ///
    /// `riding` is the ferry's guid when `SMSG_TRANSFER_PENDING` said this
    /// teleport keeps the transport, and `None` for every other teleport. See
    /// `crate::socket::handler::acks::new_world`, which distinguishes the two.
    pub fn leave_map(&mut self, riding: Option<u64>) {
        let player = self.player_guid;
        let kept = |guid: u64| Some(guid) == player || Some(guid) == riding;
        // Every item and container object is dropped with the map, but the
        // player object, whose slot fields still name them, is not. A snapshot
        // taken before this and not rebuilt would show a full inventory of
        // guids that resolve to nothing, so the version is incremented. See
        // [`Self::remove`], which does the same for a single packet.
        if self
            .entities
            .values()
            .any(|entity| Some(entity.guid) != player && is_carried_object(entity))
        {
            self.inventory_version = self.inventory_version.wrapping_add(1);
        }
        self.entities.retain(|guid, _| kept(*guid));
        // Any movement the player had on the old map has ended.
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
    /// The template holds everything the nameplate shows about a creature
    /// (its `<Innkeeper>` tag, its `CreatureType.dbc` row and its
    /// classification). None of it is in an update block, so this is a lookup
    /// by entry rather than a field. `None` for a player, which has no
    /// template, and during the round trip before
    /// `SMSG_CREATURE_QUERY_RESPONSE` arrives.
    pub fn creature_of(&self, entity: &Entity) -> Option<&CreatureInfo> {
        if entity.object_type == Some(ObjectType::Player) {
            return None;
        }
        entity.entry().and_then(|entry| self.creatures.get(&entry))
    }

    /// The same for a game object: the template `CMSG_GAMEOBJECT_QUERY`
    /// returned, or `None` while the query is outstanding.
    ///
    /// The type check mirrors the one in [`ObjectManager::creature_of`], for
    /// the same reason: both tables are keyed by entry and the id spaces
    /// overlap (entry 1731 is a Copper Vein in one and a creature in the
    /// other), so a lookup that did not check the entity's type could return a
    /// wolf's template for a chest.
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
            // Players are named by GUID and have no creature template to fall
            // back on; an unresolved player would otherwise read "entry 0".
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

    /// The name alone: what `UnitName` returns and what a unit frame's
    /// nameplate shows.
    ///
    /// [`Self::name_of`] is the listing form: a player's includes race and
    /// class, a creature's its `<Innkeeper>` tag and its rank. That suits a CLI
    /// line but not a nameplate; the 1.12.1 client puts the subname on a
    /// tooltip's second line, never inside the name. The fallbacks are the
    /// same as in [`Self::name_of`], so the output is never blank.
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
                // A pet is named by its owner's name for it, not its species:
                // the creature template says "Wolf" and the nameplate,
                // `UnitName` and the paper doll all show "Growlfang". Keyed by
                // pet number, so the name survives a dismissal.
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

    /// Players whose name has not been queried yet. Keyed by GUID, so there is
    /// nothing to deduplicate. The local player is excluded, because its name
    /// is already known from the character list.
    pub fn unresolved_player_guids(&self) -> Vec<u64> {
        self.entities
            .values()
            .filter(|e| e.object_type == Some(ObjectType::Player))
            .map(|e| e.guid)
            // Group members, who are not all in the entity walk. See
            // [`Self::group_guids`]: a member out of range has no entity, and a
            // raid frame still has to show their class.
            .chain(self.group_guids.iter().copied())
            // The friends and ignore lists, which carry only guids. See
            // [`Self::friend_guids`].
            .chain(self.friend_guids.iter().copied())
            .chain(self.ignore_guids.iter().copied())
            .chain(self.wanted_social.iter().copied())
            .filter(|g| Some(*g) != self.player_guid && !self.players.contains_key(g))
            .collect()
    }

    /// Item entries worn by a player in sight that have not been looked up yet.
    ///
    /// Deduplicated across players and slots, which matters most here:
    /// nineteen slots for every player in an inn is many packets for a few
    /// distinct items. Entries the server has already refused are not queried
    /// again.
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
        // Everything in the local player's bags, which is a different and much
        // larger set than the one above. A worn item is visible to everyone and
        // arrives through `PLAYER_VISIBLE_ITEM_n`. A stack of linen in the third
        // bag is visible to nobody else and exists only as an `Item` object the
        // server created for this client. Without this loop the inventory had
        // GUIDs and entries but no names, icons or tooltips, because the
        // equipment loop above only sees worn items.
        //
        // Every item the local player owns is already in `entities`, so this
        // loop walks those rather than traversing the bags again. It
        // deliberately does not use `items::Inventory`, because an item whose
        // container has not arrived yet still needs its name.
        for e in self.entities.values() {
            if matches!(
                e.object_type,
                Some(ObjectType::Item) | Some(ObjectType::Container)
            ) {
                want(e.entry().unwrap_or(0), &mut seen);
            }
        }
        // Entries the interface asked about. See [`Self::wanted_items`].
        for entry in &self.wanted_items {
            want(*entry, &mut seen);
        }
        seen.into_iter().collect()
    }

    /// Request an item's template because code needs its name, rather than
    /// because a unit is wearing it.
    ///
    /// Idempotent and cheap enough to call from a tooltip: an entry already
    /// resolved, already refused or already queued does nothing. It never
    /// sends a packet. The query pass in [`crate::socket::session`] sends on
    /// its own interval, so hovering, however often, sends no packet directly.
    pub fn want_item(&mut self, entry: u32) {
        if entry == 0 || self.items.contains_key(&entry) || self.unknown_items.contains(&entry) {
            return;
        }
        self.queries_wanted |= self.wanted_items.insert(entry);
    }

    /// Fill the tables from the on-disk caches before a session starts.
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

    /// Keep an answer's body for the on-disk cache, once per key.
    ///
    /// Called by the handler of each cached response with the packet body
    /// unchanged. A key already on disk or already learned this session is
    /// skipped, so a template the server restates (every login re-sends the
    /// equipment of everyone in view) is not appended every session. Nothing is
    /// queued for disk while `record_cache` is off; an on-demand kind is still
    /// held for [`Self::cached_answer`], so a book opened twice in one session
    /// is queried once.
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

    /// Keep an answer that replaces an earlier answer under the same key. A
    /// pet's name changes under its pet number; the cache file uses the last
    /// record for a key, so the new name is appended rather than skipped.
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
    /// session loop runs it through the packet dispatch instead of sending the
    /// query. See `SessionLoop::answer_from_cache`.
    pub fn cached_answer(&self, kind: Kind, key: u64) -> Option<&[u8]> {
        self.on_demand.get(&(kind, key)).map(Vec::as_slice)
    }

    /// How many on-demand answers are held.
    pub fn cached_answers(&self) -> usize {
        self.on_demand.len()
    }

    /// Record that an entry has been answered, with a template or a refusal.
    /// See [`Arrivals`] and the `arrivals` field for why this event is needed.
    pub(crate) fn note_item_arrival(&mut self, entry: u32) {
        self.arrivals.items.push(entry);
    }

    pub(crate) fn note_creature_arrival(&mut self, entry: u32) {
        self.arrivals.creatures.push(entry);
    }

    pub(crate) fn note_gameobject_arrival(&mut self, entry: u32) {
        self.arrivals.gameobjects.push(entry);
    }

    /// Take the arrivals since the last call, leaving the lists empty.
    ///
    /// A drain rather than a peek on purpose: with a peek, two readers would
    /// each see the arrival and the second would raise events about a template
    /// already applied, which on a busy login means a redraw per item per
    /// frame.
    pub fn take_arrivals(&mut self) -> Arrivals {
        std::mem::take(&mut self.arrivals)
    }

    /// What one unit holds in its three weapon slots, from whichever source
    /// supplies it.
    ///
    /// This is the only place that distinguishes creatures from players for
    /// weapons, so that `vale_assets::look::dress` does not have to. A creature
    /// carries the whole record in its update fields; a player carries three
    /// item entries whose templates arrive from `CMSG_ITEM_QUERY_SINGLE` a
    /// round trip later. A player whose queries have not returned yet has
    /// empty hands here. That is correct and temporary, the same delay as the
    /// rest of their equipment, and the renderer rebuilds when the answer
    /// arrives.
    ///
    /// An entry the server refused to describe stays empty permanently,
    /// because there is no display id to draw.
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
                // `-1` is a valid value here ("no material") and it is not
                // metal. The cast turns it into 255, which is still not 1, and
                // readers only ever test for equality with 1, so the cast is
                // safe.
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
        // Entries the interface asked about, which is a separate set: a quest
        // objective often names a creature that is not in view ("Kobold Vermin
        // slain: 3/8" read in a log opened in Stormwind), so the loop above
        // cannot find it. See [`Self::want_creature`].
        //
        // The guid is zero, which is correct: vmangos'
        // `HandleCreatureQueryOpcode` reads the entry and the guid, then uses
        // only `GetCreatureTemplate(entry)`. The game-object handler does the
        // same.
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

    /// Request a creature's template because code needs its name.
    ///
    /// The counterpart of [`Self::want_item`], with the same contract:
    /// idempotent, never touches the socket, drained by the session's query
    /// pass. The caller is the quest log, whose objective lines combine a
    /// counter from the log with a name that only this table has.
    pub fn want_creature(&mut self, entry: u32) {
        if entry != 0 && !self.creatures.contains_key(&entry) {
            self.queries_wanted |= self.wanted_creatures.insert(entry);
        }
    }

    /// The same for a game object, the other kind of object a quest objective
    /// can target. The top bit of `ReqCreatureOrGOId` decides which.
    pub fn want_gameobject(&mut self, entry: u32) {
        if entry != 0 && !self.gameobjects.contains_key(&entry) {
            self.queries_wanted |= self.wanted_gameobjects.insert(entry);
        }
    }

    /// Set the current group members, replacing the previous list. See
    /// [`Self::group_guids`].
    ///
    /// Replaced rather than merged, because the roster packet is the complete
    /// state: a member who left must no longer be queried. The hint is raised
    /// only when a guid is new to the name cache, so the ordinary resend of an
    /// unchanged roster costs nothing.
    pub fn note_group(&mut self, guids: Vec<u64>) {
        self.queries_wanted |= guids
            .iter()
            .any(|guid| *guid != 0 && !self.players.contains_key(guid));
        self.group_guids = guids;
    }

    /// Set the current friends list, replacing the previous one. See
    /// [`Self::friend_guids`].
    ///
    /// Replaced rather than merged, for the reason given on
    /// [`Self::note_group`]: a removed player must no longer be queried. The
    /// hint is raised only for a guid the name cache has never answered, so
    /// the packet arriving twice costs nothing.
    pub fn note_social_friends(&mut self, guids: Vec<u64>) {
        self.queries_wanted |= self.any_unnamed(&guids);
        self.friend_guids = guids;
    }

    /// The same for the ignore list.
    pub fn note_social_ignores(&mut self, guids: Vec<u64>) {
        self.queries_wanted |= self.any_unnamed(&guids);
        self.ignore_guids = guids;
    }

    /// Add one guid named by a mid-session list change. See
    /// [`Self::wanted_social`].
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

    /// Whether the query pass should walk the world for things to query. Taken
    /// and cleared by the query pass in [`crate::socket::session`].
    ///
    /// The pass walks every entity in view four times. It runs on the session
    /// tick rather than every two seconds, so that a name a window is waiting
    /// for is queried on the next tick instead of up to two seconds later: 25
    /// ms instead of 2,000. Running four walks forty times a second under the
    /// world lock, while the renderer takes the same lock sixty times a
    /// second, would replace one cost with another.
    ///
    /// This flag answers "has anything changed since the last pass" in O(1).
    /// Exactly two things raise it:
    ///
    /// * one of the `want_*` calls above inserted an entry (a loot row, a
    ///   vendor row, a quest objective, a tooltip's reagent);
    /// * an update block created an object or changed something in the
    ///   character's inventory. These are the only two ways an entry this
    ///   client has never seen can arrive.
    ///
    /// A movement block does not raise it. Movement is most of the traffic in
    /// a busy zone and cannot name anything new. An equipment change on an
    /// existing entity whose count is unchanged does not raise it either; that
    /// case waits for the periodic pass, which is kept because a query gets no
    /// acknowledgement and an unanswered one must be sent again anyway.
    pub fn take_query_hint(&mut self) -> bool {
        std::mem::take(&mut self.queries_wanted)
    }

    /// Raise the query hint from outside this manager.
    ///
    /// Used for `SMSG_PET_SPELLS`: it names no new object, so the create-block
    /// rule above does not apply, and the name it makes queryable is derived
    /// rather than queued. Without this the pet's name waits for the next
    /// two-second pass.
    pub fn hint_queries(&mut self) {
        self.queries_wanted = true;
    }

    /// Start animating a server-driven move.
    ///
    /// A stop packet (no destination) places the unit where the server says it
    /// is, which also corrects any drift the interpolation accumulated.
    pub fn apply_monster_move(&mut self, mm: &MonsterMove) {
        // Resolved here rather than in `advance`, because only the manager can
        // look up another entity. A creature told to face its victim needs the
        // victim's position, which is the usual case in combat.
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
            // `SMSG_MONSTER_MOVE_TRANSPORT`: every waypoint above is in this
            // transport's frame. See `Spline::in_world`.
            on_transport: mm.transport,
        });
        // A server-driven move replaces the client's dead reckoning, as
        // `apply_movement` replaces a spline in the other direction.
        //
        // Without this both remain: a player who is knocked back, charged or
        // dragged gets a spline on top of a `MovementInfo` that still says
        // FORWARD. When the spline ends, `advance` finds that stale block and
        // moves the player again, in the direction they were running before,
        // at run speed, indefinitely. The server never corrects it, because on
        // the server the player is standing still. The entity appears to drift
        // away on its own.
        entity.movement = None;

        // Facing. A unit walks forwards, so while it is moving it faces the
        // direction of travel. A stop packet has no travel, so the unit takes
        // the facing the packet asks for, or keeps its previous one.
        //
        // Without this a creature keeps the orientation it was created with
        // for the whole session and walks sideways or backwards. The animation
        // is correct and the model is turned the wrong way, which looks like
        // an animation bug.
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
                // Every branch above depends on the start point, so an
                // impossible one invalidates the spline too. A stale creature is
                // preferable to one walking a path from an invalid point.
                if let Some(entity) = self.entities.get_mut(&mm.guid) {
                    entity.spline = None;
                }
                self.rejected_positions = self.rejected_positions.saturating_add(1);
            }
        }
    }

    /// Record that a server statement moved an entity further than it could
    /// have walked, and which packet did it.
    ///
    /// Called by every site that writes a server position, because only the
    /// call site knows the opcode. The local player is excluded: the live
    /// session dead-reckons it and writes the result back here, so a jump on
    /// the player is the resync working, not an entity moving.
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

    /// Queue a line of chat, dropping the oldest if nothing is reading.
    pub fn note_chat(&mut self, message: crate::play::chat::ChatMessage) {
        if self.chat.len() >= CHAT_BACKLOG {
            self.chat.remove(0);
        }
        self.chat.push(message);
    }

    /// Take everything that has arrived since the last call.
    ///
    /// A line is an event and must be shown once, so this empties the queue
    /// (see the field). Two readers in one process would each see part of the
    /// conversation, so there is only one reader.
    pub fn take_chat(&mut self) -> Vec<crate::play::chat::ChatMessage> {
        std::mem::take(&mut self.chat)
    }

    /// Set the quest marker over a quest giver's head, by guid.
    ///
    /// State rather than an event; it is the only quest data kept as state.
    /// The marker stays until the server changes it, so the pass that draws it
    /// must be able to read it at any time rather than having to receive the
    /// answer when it arrived. A marker kept only as an event would be missing
    /// for every unit that came into view before the interface was ready.
    ///
    /// Cleared with the entity it belongs to ([`Self::forget_quest_status`]),
    /// so a guid the server reuses in another zone cannot inherit one.
    pub fn set_quest_status(&mut self, guid: u64, status: crate::play::quest::DialogStatus) {
        match status {
            // `None` means no marker and is not stored. Storing it would grow
            // the map by one entry per giver per session and would not help
            // answer whether a giver has been queried; that is
            // [`Self::wants_quest_status`], which stops the client querying
            // the same giver twice a second.
            crate::play::quest::DialogStatus::None => {
                self.quest_status.remove(&guid);
            }
            status => {
                self.quest_status.insert(guid, status);
            }
        }
        self.asked_quest_status.insert(guid);
    }

    /// The quest marker for a giver, or `None` for one that has not been
    /// queried or has no marker.
    pub fn quest_status(&self, guid: u64) -> Option<crate::play::quest::DialogStatus> {
        self.quest_status.get(&guid).copied()
    }

    /// Whether this giver still needs a `CMSG_QUESTGIVER_STATUS_QUERY`.
    ///
    /// Once per guid, until the quest log changes. See
    /// [`Self::requery_quest_status`].
    pub fn wants_quest_status(&self, guid: u64) -> bool {
        !self.asked_quest_status.contains(&guid)
    }

    /// Query every giver again, because the answers are out of date.
    ///
    /// vmangos sends `SMSG_QUESTGIVER_STATUS` from exactly one place,
    /// `HandleQuestgiverStatusQueryOpcode`, as a reply, so no packet announces
    /// that a marker changed. The marker depends on the character's quest log,
    /// so the only reliable trigger is a change to the log: accepting a quest
    /// turns a giver's gold `!` into a grey `?`, killing the eighth kobold
    /// turns that into a gold `?`, and handing in removes it. Without this a
    /// marker stays as it was when the unit first came into view for the rest
    /// of the session. That caused both a turn-in NPC that still showed a `!`
    /// and another NPC that showed nothing.
    ///
    /// The answers are kept and only the record of queries is cleared, so the
    /// markers do not disappear during the round trip. A giver whose new
    /// answer is `None` is cleared by [`Self::set_quest_status`] when it
    /// arrives.
    pub fn requery_quest_status(&mut self) {
        self.asked_quest_status.clear();
    }

    /// Forget the quest and taxi status of an entity that has left the world.
    pub fn forget_quest_status(&mut self, guid: u64) {
        self.quest_status.remove(&guid);
        self.asked_quest_status.remove(&guid);
        self.taxi_status.remove(&guid);
        self.asked_taxi_status.remove(&guid);
    }

    /// Set whether this flight master's node has been discovered by this
    /// character, from `SMSG_TAXINODE_STATUS`.
    ///
    /// State rather than an event, for the same reason as the quest status:
    /// the green `!` over an undiscovered flight master stays until something
    /// changes it, so the pass that draws it must be able to read it at any
    /// time.
    ///
    /// Both answers are kept, unlike the quest map, which drops `None`. Here
    /// `false` is the significant value, because it shows the marker, so the
    /// map stores it, and the queried set only stops the query repeating.
    pub fn set_taxi_status(&mut self, guid: u64, known: bool) {
        self.taxi_status.insert(guid, known);
        self.asked_taxi_status.insert(guid);
    }

    /// The taxi status for a flight master, or `None` for one that has not
    /// been queried.
    pub fn taxi_status(&self, guid: u64) -> Option<bool> {
        self.taxi_status.get(&guid).copied()
    }

    /// Whether this flight master still needs a `CMSG_TAXINODE_STATUS_QUERY`.
    ///
    /// Once per guid, with no requery. The quest marker changes with the
    /// character's quest log and no packet announces it, but this status is
    /// announced: vmangos' `SendLearnNewTaxiNode` sends `SMSG_NEW_TAXI_PATH`
    /// and then a new `SMSG_TAXINODE_STATUS` carrying 1 for the flight master
    /// just discovered, so the marker is removed at the same time as the
    /// message is shown.
    pub fn wants_taxi_status(&self, guid: u64) -> bool {
        !self.asked_taxi_status.contains(&guid)
    }

    /// Queue the server's response to one of the local player's actions, on
    /// the same terms as [`Self::note_chat`]: bounded, oldest dropped.
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

    /// Queue one combat event for the combat log.
    ///
    /// Bounded at [`COMBAT_BACKLOG`], oldest dropped. See that constant for
    /// why this is separate from [`Self::note_event`].
    pub fn note_combat(&mut self, event: crate::play::combatlog::CombatEvent) {
        if self.combat.len() >= COMBAT_BACKLOG {
            self.combat.remove(0);
        }
        self.combat.push(event);
    }

    /// Take the queued combat events, in arrival order.
    ///
    /// The order matters. A swing, the spell that followed it and the death it
    /// caused are three packets that only make sense in the order they were
    /// sent, and a reader that took each kind from a separate queue would
    /// interleave them differently every frame.
    pub fn take_combat(&mut self) -> Vec<crate::play::combatlog::CombatEvent> {
        std::mem::take(&mut self.combat)
    }

    /// `SMSG_UPDATE_AURA_DURATION`: how long the aura in this slot has left.
    ///
    /// Records are never deleted. The packet is only sent for a live aura, so
    /// a record whose slot has since been reused is stale, and the reader
    /// rejects it. See [`Self::aura_durations`] and, in the client crate,
    /// `game::auras`, which ignores a reading older than the aura it would be
    /// matched to.
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

    /// The recorded durations of the local player's auras, as
    /// `(slot, reading)`.
    ///
    /// Read rather than drained, unlike the chat and action queues: a duration
    /// is state that stays valid between frames, and a reader that missed the
    /// packet still needs to draw the timer. At most 48 entries, one per slot,
    /// so nothing needs pruning.
    pub fn aura_durations(&self) -> impl Iterator<Item = (u8, AuraDuration)> + '_ {
        self.aura_durations.iter().map(|(slot, held)| (*slot, *held))
    }

    /// `SMSG_INITIAL_SPELLS`: the whole spellbook, replacing whatever was held.
    ///
    /// Replaced rather than merged, because the packet is the server's complete
    /// statement. It is sent again after a talent reset, where a merge would
    /// leave the unlearned ranks on the bar permanently.
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

    /// Apply an `SMSG_PET_SPELLS`: the pet bar, or the dismissal that removes
    /// it.
    ///
    /// The dismissal needs no special handling: a `PetSpells` with a zero guid
    /// is the empty bar, so storing it is correct.
    pub fn apply_pet_spells(&mut self, spells: crate::play::pet::PetSpells) {
        self.pet = spells;
        self.pet_attacking = false;
        self.pet_version = self.pet_version.wrapping_add(1);
    }

    /// The client-side effect of a `CMSG_PET_ACTION`, applied when it is sent.
    ///
    /// The server keeps the state and sends no reply: vmangos'
    /// `HandlePetAction` ends in `SetReactState` for a reaction and
    /// `HandlePetCommand` ends in `SetCommandState` for stay and follow, and
    /// neither sends `SMSG_PET_MODE` (`Pet::SetEnabled` is its only sender).
    /// The 1.12.1 client therefore updates its own copy before the packet is
    /// sent, and this does the same:
    ///
    /// * a reaction slot writes the low byte of the state word, the react
    ///   state;
    /// * a stay or follow command writes the second byte, the command state;
    /// * an attack command with a target sets the attack flag and writes no
    ///   state byte, which matches the server: the `COMMAND_ATTACK` case of
    ///   `HandlePetCommand` calls `SetIsCommandAttack` and leaves the command
    ///   state unchanged;
    /// * dismiss and a spell slot change nothing here.
    ///
    /// Each of the first three fires `PET_BAR_UPDATE`, which is why the version
    /// is incremented when anything changed. Without this update the three
    /// mode buttons and the two command buttons kept the old button pressed
    /// until the next `SMSG_PET_SPELLS`, which only a teleport sends.
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

    /// The client-side effect of `CMSG_PET_SET_ACTION`, applied when it is
    /// sent.
    ///
    /// The server stores the move and sends no reply (`HandlePetSetAction`
    /// ends at `SetActionBar`). The drag must therefore be recorded in this
    /// copy, or the next rebuild restores the bar as it was before the drop.
    /// Each entry is `(position, packed)`, the same pair the packet carries:
    /// one entry for a removal, two for a move. The rule that the body length
    /// gives the entry count is at [`crate::play::pet::pet_set_action_body`].
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

    /// The client-side effect of `CMSG_PET_STOP_ATTACK`: the attack flag is
    /// cleared and the bar redraws with the attack button released.
    pub fn apply_pet_stop_attack(&mut self) {
        if self.pet_attacking {
            self.pet_attacking = false;
            self.pet_version = self.pet_version.wrapping_add(1);
        }
    }

    /// `SMSG_PET_MODE`: the four state bytes without the rest of the bar.
    ///
    /// Applied only to the pet the bar belongs to. The packet also arrives for
    /// a charmed unit this client is not controlling, and writing that over the
    /// bar's state would make the panel show another unit's react and command
    /// state.
    pub fn apply_pet_mode(&mut self, mode: crate::play::pet::PetMode) {
        if self.pet.pet != mode.pet || self.pet.pet == 0 {
            return;
        }
        if self.pet.react == mode.react
            && self.pet.command == mode.command
            && self.pet.flags == mode.flags
        {
            // A repeat of the current state does not rebuild the panel. This
            // is the same rule `apply_spell_change` follows.
            return;
        }
        self.pet.react = mode.react;
        self.pet.command = mode.command;
        self.pet.flags = mode.flags;
        self.pet_version = self.pet_version.wrapping_add(1);
    }

    /// The client-side effect of `CMSG_PET_SPELL_AUTOCAST`, applied when it is
    /// sent.
    ///
    /// vmangos records the toggle and sends no reply
    /// (`HandlePetSpellAutocastOpcode` ends at `SetSpellAutocast`). A client
    /// that waits for the server therefore never shows the autocast marker
    /// change, and the next press reads the old state and sends the same
    /// toggle again. The bar and the spellbook list both hold the spell, so
    /// both are updated. A passive spell is not changed, which applies the
    /// server's `IsAutocastable` check locally.
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

    /// Records the pet's given name, keyed by pet number.
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

    /// The pet whose name must be queried, as `(pet_number, guid)`. The query
    /// packet needs both; the cache is keyed by the number only.
    ///
    /// The answer is derived from current state rather than taken from a
    /// queue. `SMSG_PET_SPELLS` arrives before the pet's create block, so a
    /// set filled when the bar arrived would hold a number with no entity
    /// yet. The create block alone is not enough either, because a pet that is
    /// only in view does not belong to this player. Deriving the answer from
    /// both costs one map lookup per query beat and cannot fire too early.
    ///
    /// Each number is queried once per timestamp and not retried. A rename
    /// changes `UNIT_FIELD_PET_NAME_TIMESTAMP` (vmangos `HandlePetRename`
    /// writes `time(nullptr)` into it after the name change), and that field
    /// is the only signal on the wire that the cached name is out of date. A
    /// timestamp the cache does not hold is queried once; a server that does
    /// not answer is not asked again until the field changes. If the query is
    /// missed, the panel keeps showing the species name it already has.
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

    /// `SMSG_SUPERCEDED_SPELL`: a spell rank replaced by a higher one. This is
    /// two edits: the spellbook, and every bar slot that holds the old id.
    ///
    /// Updating only the spellbook leaves a bar button that still shows the
    /// icon but no longer casts: the server refuses a spell the character no
    /// longer has active, and sends no error when it does. See
    /// [`crate::play::spells::parse_superceded_spell`].
    ///
    /// The old id is removed rather than merely deactivated. vmangos keeps it
    /// with `active = false` because it has to remember it for the database;
    /// nothing on this side can press an inactive spell, and `known` is what
    /// both the book and the bar filter against.
    ///
    /// The slots that changed are reported in
    /// [`crate::play::spells::PlayerEvent::SpellSuperceded`], and the caller
    /// must send them to the server.
    pub fn apply_superceded_spell(&mut self, old: u32, new: u32) {
        use crate::play::spells::action_kind;

        // Only spell buttons: an item entry or a macro index can equal a spell
        // id and still refer to something else.
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
        // Incremented even when no slot changed, unlike `set_action_button`:
        // the spellbook changed, and readers rebuild from the spellbook when
        // the version changes.
        self.spellbook_version = self.spellbook_version.wrapping_add(1);
        self.note_event(crate::play::spells::PlayerEvent::SpellSuperceded { old, new, slots });
    }

    /// `SMSG_ACTION_BUTTONS`: the whole bar, likewise replacing.
    pub fn apply_action_buttons(&mut self, buttons: Vec<crate::play::spells::ActionButton>) {
        self.action_buttons = buttons;
        self.spellbook_version = self.spellbook_version.wrapping_add(1);
    }

    /// One action bar slot changed by this client. The server never states
    /// this change.
    ///
    /// The bar is client state that the server only stores:
    /// `CMSG_SET_ACTION_BUTTON` has no reply, and `SMSG_ACTION_BUTTONS` arrives
    /// once, at login. A drop onto a button must therefore be recorded here as
    /// well as sent. Otherwise the next rebuild from this list, for example at
    /// a level-up, loses the change, although the bar looked correct until
    /// then.
    ///
    /// A `kind` of `None` empties the slot. The version is not incremented. A
    /// reader rebuilds the whole bar when the version changes, and this caller
    /// has already applied its own one-slot change; incrementing it would make
    /// each drop cost 120 DBC lookups.
    pub fn set_action_button(&mut self, slot: u8, action: u32, kind: Option<u8>) {
        self.action_buttons.retain(|button| button.slot != slot);
        if let Some(kind) = kind {
            self.action_buttons.push(crate::play::spells::ActionButton { slot, action, kind });
            // Ascending, the order `parse_action_buttons` produces. Nothing
            // reads the list by position today; sorting here keeps both code
            // paths producing the same order so that nothing comes to depend
            // on a difference.
            self.action_buttons.sort_by_key(|button| button.slot);
        }
    }

    /// `SMSG_ATTACKSTART` / `SMSG_ATTACKSTOP`: recorded on the attacker, and
    /// also on the manager itself when the attacker is the local player.
    ///
    /// Both packets are broadcast to every client in sight, and both are kept
    /// for every unit. They are the only statement in the protocol that a unit
    /// is swinging at something, and the ready stance is drawn from them
    /// ([`Entity::attacking`]). This code previously dropped them for every
    /// unit except the local player and used `UNIT_FLAG_IN_COMBAT` for the
    /// rest. That put every unit near a fight into its combat stance,
    /// including a unit being shot at from across a room and a unit running
    /// away.
    pub fn apply_attack_state(&mut self, attacker: u64, victim: Option<u64>) {
        let victim = victim.filter(|guid| *guid != 0);
        if let Some(unit) = self.entities.get_mut(&attacker) {
            unit.attacking = victim;
        }
        if self.player_guid == Some(attacker) {
            self.attacking = victim;
        }
    }

    /// `MSG_CHANNEL_START`: a channelled spell has begun on the local player.
    ///
    /// A channel is a duration, and this packet carries it:
    /// `{u32 spell, u32 milliseconds}`, sent by `Spell::SendChannelStart` with
    /// `SendDirectMessage`, so only the caster receives it. `SMSG_SPELL_GO` has
    /// already arrived by then, and the cast bar holds what `SMSG_SPELL_START`
    /// stated, which for a channel is nothing. Without this packet an
    /// Evocation was drawn as an instant release and the held pose was never
    /// shown.
    ///
    /// The channel is recorded in the cast counters rather than in a third
    /// pair of its own, because for the animation it is a wind-up held for a
    /// stated time. The pose comes from `SpellVisual`'s channel kit; see
    /// `vale_assets::tables::spell`.
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
        // This counter records that the begin came after a release, which is
        // what makes the channel drawn at all; see
        // [`Entity::casts_channelled`].
        unit.casts_channelled = unit.casts_channelled.wrapping_add(1);
    }

    /// `MSG_CHANNEL_UPDATE`: the time left in the channel, in milliseconds.
    ///
    /// Zero means the channel is over, which is what an interrupted Evocation
    /// sends. Any other value restates the remaining time without restarting
    /// the pose. It is not a new cast: the begin counter is unchanged and only
    /// `cast_time_ms` is written.
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
    /// A player's chat line carries a GUID and no name (`BuildChatPacket`
    /// writes a name only for the monster types). The server assumes the
    /// client already knows the sender, which is true of anyone in view and
    /// false of a guild member on another continent. The lookup order is: the
    /// name in the packet, the `CMSG_NAME_QUERY` cache, the entity if it is in
    /// view, and finally the GUID itself. The result is never blank and never
    /// names the wrong sender.
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
    /// victim flinches. A blow that missed still swings, so that the miss is
    /// visible. Only the flinch is conditional, and the condition is the
    /// server's (`HITINFO_AFFECTS_VICTIM`, whose comment in `UnitDefines.h`
    /// reads "no being hit animation on victim without it").
    ///
    /// Neither unit is created if it is not already known: a swing landing on
    /// something out of sight is not a reason to invent an entity with no
    /// position, which would be drawn at the centre of the map.
    pub fn apply_attack(&mut self, attack: &crate::play::action::AttackUpdate) {
        use crate::play::action::victim_state;
        if let Some(attacker) = self.entities.get_mut(&attack.attacker) {
            attacker.swings_thrown = attacker.swings_thrown.wrapping_add(1);
            attacker.last_swing_info = attack.hit_info;
            // The victim state is stored on both ends, because each end uses it
            // for something different: the victim's copy picks its reaction
            // animation, and the attacker's copy picks which column of its
            // weapon's sound row the blow plays.
            attacker.last_swing_state = attack.victim_state;
            attacker.last_swing_victim = attack.victim;
            attacker.last_swing_damage = attack.damage;
        }
        // A dodge, a parry and a block are not hits, but each has a reaction
        // animation. `hit_the_victim` tests `HITINFO_AFFECTS_VICTIM`, which the
        // server does not set for a blow that did not connect. A victim counter
        // driven by that test alone leaves a character standing still through
        // every blow it defended against, which is most of a fight.
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
        // The floating combat number has its own counter, stored on the
        // victim. It is separate from `blows_taken` above, which is an
        // animation counter and requires the blow to have connected: a miss
        // and a dodge are drawn as words and must reach the reader. This
        // counter tests the attacker instead, because the 1.12.1 client shows
        // these numbers only when the source is the player or the player's
        // pet; see [`Entity::damage_taken`].
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

    /// `SMSG_SPELLNONMELEEDAMAGELOG`: a spell hit a unit, or a damage-over-time
    /// effect ticked.
    ///
    /// Recorded in the same counter as a swing, for the reason given at
    /// [`Entity::damage_taken`], and under the same condition: the 1.12.1
    /// client shows the number only when the source is the player or the
    /// player's pet, so a fight elsewhere in view records nothing.
    ///
    /// A full absorb or resist still counts. The damage is zero and the flags
    /// give the reason, which is drawn as a word. Dropping it here would lose
    /// every "Immune" and "Resist" a spell draws.
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

    /// `SMSG_SPELLHEALLOG`: a heal landed on a unit, drawn as a number over
    /// its head like damage.
    ///
    /// Recorded in the same counter, with the amount as the damage and the
    /// spell id set. The reader tells a heal apart by [`Entity::healed`], and
    /// this method is the only one that sets it.
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
    /// The creature is not created if it is not already known, for the same
    /// reason as in [`Self::apply_attack`]. `SMSG_AI_REACTION` is broadcast to
    /// every client in sight of the creature, which is a larger set than the
    /// clients the creature is in sight of, and a sound from a unit with no
    /// position would play at the map origin.
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

    /// A `SpellVisualKit` the server wants played on a unit:
    /// `SMSG_PLAY_SPELL_VISUAL` when `impact` is false, `SMSG_PLAY_SPELL_IMPACT`
    /// when it is true. See [`crate::play::sound`] for the two packet bodies
    /// and the two server callers that are not a GM command.
    ///
    /// Unlike the appliers that create a missing entity, this one does not,
    /// which matches the 1.12.1 client. vmangos' comment on the sibling packet,
    /// "ignored by client if unit is not loaded", says the client drops a
    /// visual for a guid it does not know. There is no position to draw it at
    /// and no model to attach it to. An entity created from a kit id would be
    /// a unit with no position, no model and no create block, which the
    /// renderer would have to filter out. [`Self::apply_emote`] handles its
    /// packet the same way.
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

    /// A cast this unit was making ended without landing: refused,
    /// interrupted, or cancelled by the player.
    ///
    /// See [`Entity::casts_cancelled`] for why this is a separate counter
    /// rather than a rollback of the two counters the press moved, and for the
    /// three packets that call it.
    ///
    /// It only ever concerns cast art the server started. This client does not
    /// draw its own cast at the press (see [`Self::apply_cast`]), so a refusal
    /// that arrives before any `SMSG_SPELL_START` has nothing to remove. In
    /// that case `unit.last_spell` still names the previous cast, and the
    /// guard below ignores the refusal.
    pub fn apply_cast_cancelled(&mut self, caster: u64, spell_id: u32) {
        let Some(unit) = self.entities.get_mut(&caster) else {
            return;
        };
        // Only the cast currently shown is cancelled. A failure for a spell
        // this unit is not showing refers to a cast that has already finished,
        // for example a refusal that arrives after the release was drawn.
        // Removing the art on it would cut off whatever is playing now.
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
    /// The local player is handled the same way as every other unit. An
    /// earlier `predict_own_cast` incremented both counters at the press, so
    /// the arm moved without waiting a round trip, and kept a slot to ignore
    /// the server's echo so the cast did not play twice. The 1.12.1 client
    /// (build 5875) does not draw a cast at the press. It raises
    /// `SPELLCAST_START` only on receiving `SMSG_SPELL_START`, and vmangos
    /// comments its `SendSpellStart()` with `// will show cast bar`. At the
    /// press the client records the pending cast, starts the global cooldown
    /// and sends the request, and draws nothing. With the prediction, a
    /// refused cast played a wind-up and a release for a spell that was never
    /// cast.
    pub fn apply_cast(&mut self, cast: &crate::play::action::SpellCast, start: bool) {
        let Some(unit) = self.entities.get_mut(&cast.caster) else {
            return;
        };
        // Written for both the start and the release, because the wind-up and
        // the release are two animations from the same spell's visual.
        unit.last_spell = cast.spell_id;
        if start {
            // The server's `m_timer` decides how long the pose is held, whether
            // or not the animation has already started. At the press only the
            // spell's base cast time was known.
            unit.cast_time_ms = cast.cast_time_ms;
            // The wind-up carries no hit list at all, so writing the target for
            // both would clear it the moment the next spell started.
            unit.casts_begun = unit.casts_begun.wrapping_add(1);
            return;
        }
        unit.last_spell_target = cast.target();
        unit.last_spell_targets.clear();
        unit.last_spell_targets.extend_from_slice(&cast.hits);
        // The spell is also pushed into the ring, so that a second release
        // arriving before the renderer next reads does not replace the first
        // one's art. See [`Entity::recent_spells`], where Charge is the
        // measured case.
        unit.recent_spells.rotate_left(1);
        unit.recent_spells[RECENT_SPELLS - 1] = cast.spell_id;
        // Two counters for one packet, used for different purposes. The hit
        // list is read when [`Entity::casts_landed`] changes.
        unit.casts_landed = unit.casts_landed.wrapping_add(1);
        unit.casts_released = unit.casts_released.wrapping_add(1);
    }

    /// `SMSG_SPELL_DELAYED`: the cast in progress was knocked back.
    ///
    /// This is the only packet that restates a cast's length after it has
    /// begun. vmangos' `Spell::Delayed` adds `GetNextDelayAtDamageMsTime()` to
    /// `m_timer` when the caster takes damage (500 ms, then 1,000, then 3,000)
    /// and sends this packet to the caster only. Everything this client holds
    /// for the cast runs on a clock started from `SMSG_SPELL_START`'s
    /// `m_timer`. Without this packet the wind-up pose, the art on the caster's
    /// hands and the cast bar all end while the server is still casting. The
    /// report of a character stuck casting a spell that was interrupted
    /// describes the same mismatch.
    ///
    /// Recorded as a counter rather than a new absolute length, because the
    /// packet states a difference and each consumer holds its own deadline;
    /// see [`Entity::casts_delayed`].
    pub fn apply_cast_delayed(&mut self, guid: u64, delay_ms: u32) {
        let Some(unit) = self.entities.get_mut(&guid) else {
            return;
        };
        unit.casts_delayed = unit.casts_delayed.wrapping_add(1);
        unit.last_cast_delay_ms = delay_ms;
        // The wind-up was started with the server's `m_timer`, so the restated
        // length is stored here as well. A later reader of the field gets the
        // cast's current length rather than its original one.
        unit.cast_time_ms = unit.cast_time_ms.saturating_add(delay_ms);
    }

    /// Apply a `MSG_MOVE_*` broadcast: another player moved.
    ///
    /// The position in the block is correct as of that packet, and the flags
    /// say what the player is doing next, which [`Self::advance`] then
    /// dead-reckons. A player's own broadcast is not sent back to them
    /// (`SendMovementMessageToSet` excludes the sender). The guard costs
    /// little, and without it a half-second-old server position would
    /// conflict with the live simulation and cause jitter that is hard to
    /// diagnose.
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
                // A rejected position here would otherwise leave the flags in
                // place, and `advance` would dead-reckon onwards from an
                // invalid position. The movement block is cleared instead.
                if let Some(entity) = self.entities.get_mut(&guid) {
                    entity.movement = None;
                }
                self.rejected_positions = self.rejected_positions.saturating_add(1);
            }
        }
    }

    /// One of the twelve `SMSG_SPLINE_MOVE_*` packets: the server setting or
    /// clearing a movement flag on a unit it controls.
    ///
    /// See [`crate::state::movement::SplineFlagChange`] for what each opcode
    /// means, and why this packet family exists beside the two older ones.
    ///
    /// It creates the entity if it has not been seen, as the movement
    /// appliers here do, for the same reason: the order of an update block and
    /// a state packet about the same guid is not guaranteed, and a root
    /// dropped because the create block had not arrived yet would never be
    /// restated.
    ///
    /// The local player is not excluded. The server sends these packets only
    /// about units no player is moving, so one naming the local player should
    /// not arrive. If one did, it would be a statement worth keeping. This
    /// differs from [`Self::apply_movement`], where this client owns the
    /// position and must not receive an older copy of it.
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
    /// ([`Spline::position_and_heading`]) rather than in a straight line from
    /// end to end. The client is not authoritative, but cutting the corner of
    /// a patrol route puts a creature visibly off the path it was sent along.
    /// Players are dead-reckoned instead: their heartbeats are 500 ms apart,
    /// so without extrapolation every other player jumps twice a second.
    ///
    /// ## Why dead reckoning uses the same `world` as the local character
    ///
    /// The dead reckoning here reproduces a stride that another client has
    /// already taken, and that client applied its own collision to it. A
    /// player holding forward against a wall stops, and their heartbeats keep
    /// reporting the stop. Extrapolating the same flags in a straight line
    /// therefore simulates a different client, and the disagreement has no
    /// bound where it is most visible. Half a second of run speed is 3.5
    /// yards, so the observed player walks through the wall and is pulled back
    /// on the next heartbeat, twice a second, for as long as they push against
    /// it. This matches the report "they appear to constantly teleport thru it
    /// and back". The ground has the same problem: a runner going up a hill
    /// has their height restated by their own client every stride and by this
    /// one never.
    ///
    /// The stride is therefore proposed and the world decides where it ends,
    /// in the same order [`crate::state::movement::Mover::advance`] uses for
    /// the local character: the wall first, then the floor at the end of the
    /// stride that was actually taken. `None` is passed by the CLI's snapshot
    /// pump and by the renderer before its first tile, and gives the earlier
    /// straight-line behaviour.
    ///
    /// Three cases are not corrected. A spline is left alone, because the
    /// server computed that path over its own geometry and restates it. A
    /// swimmer's height follows the water, not the floor. An entity that walks
    /// off a ledge is held at its height rather than dropped, because nothing
    /// here simulates its fall, it never announced one, and the server will
    /// state the result within half a second. Only the positions this function
    /// extrapolates are corrected.
    ///
    /// A step longer than a tick is split by the caller, not clamped here.
    /// `dt_ms` is walked as one stride: the wall test, the floor lookup and
    /// the arc each happen once for it. Passing a whole five-second stall
    /// would produce a 38-yard step through a `Footing::step` written for
    /// 0.2-yard steps. `socket::session`'s `tick_world` instead calls this
    /// repeatedly in tick-sized slices, which is what a thread that had not
    /// stalled would have done.
    ///
    /// This runs under the world lock, and the set of entities queried is kept
    /// small. The two world queries are made once per moving dead-reckoned
    /// entity per step, which means other players walking in view and nothing
    /// else: a creature moves on a spline, and a standing unit is skipped
    /// before either query. The lock order is one-way (this lock, then the
    /// terrain's and the collision world's, neither of which can reach an
    /// `ObjectManager`), so no deadlock cycle exists. The cost in a crowded
    /// capital has not been measured; the session thread's step time is where
    /// it would show.
    pub fn advance(&mut self, dt_ms: u32, world: Option<&dyn crate::state::movement::Footing>) {
        let dt = dt_ms as f32 / 1000.0;
        let player_guid = self.player_guid;

        for entity in self.entities.values_mut() {
            // Transports are the only entities here whose motion no packet
            // drives. They are advanced at the top of this loop rather than in
            // a loop of their own, and before every guard below, because none
            // of those guards applies: a transport has no movement block, no
            // speeds and no spline, so it would `continue` past all of them. A
            // separate pass measured +3.0 us per step over 2,000 entities,
            // which is one more walk of the map for one `Option` test.
            //
            // The phase is not clamped, as elsewhere in this function: it is
            // the world's clock and the server advanced all of it. The reader
            // applies the wrap, in
            // `vale_assets::tables::transport::Transports::offset_at`, so the
            // wrap is implemented in one place only.
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
            // Rooted units are not dead-reckoned. The guard is here rather than
            // left to the moving flags because the root and the flags arrive
            // in different packets and in either order. The server clears the
            // moving bits itself when it roots a unit it controls (`SetRooted`
            // calls `StopMoving` first), so for a player this is usually
            // redundant. A root that arrives between a start packet and its
            // heartbeat has nothing to clear the bits, and without this guard
            // the unit would keep walking for half a second after the spell
            // stopped it.
            if info.has(move_flags::ROOT) || !info.is_moving() {
                continue;
            }
            // A unit in the air is simulated separately. The horizontal
            // velocity is the one fixed at take-off; the flags do not describe
            // it, and a player who jumps while running forward and releases W
            // keeps travelling. The height follows the parabola that
            // `Unit::ExtrapolateMovement` computes. This code previously moved
            // an airborne unit at run speed along its facing, with z held at
            // the take-off height until the next heartbeat corrected it.
            if info.has(move_flags::JUMPING) {
                let t0 = entity.fall_secs;
                // The server does not extrapolate an arc past ten seconds, and
                // neither does this code: after that the landing packet has
                // been lost, and extrapolating further only moves the unit
                // below the ground.
                if t0 > MAX_ARC_SECS {
                    continue;
                }
                let t1 = t0 + dt;
                let safe = info.has(move_flags::SAFE_FALL);
                // The jump block's `zspeed` is already in `fall_elevation`'s
                // frame, positive downwards, so a jump arrives as -7.9558. The
                // wire value is the fall's start velocity (`Unit::KnockBack`
                // sends `-verticalSpeed` in this field). Negating it here moved
                // every jumping player straight down through the floor.
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
                    // Only the arc's height is applied. The jumper's client
                    // follows the same parabola, and a floor lookup here would
                    // place them on a roof they are jumping over.
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
                // A swimmer's height is not taken from the floor. Their own
                // client holds them at a depth this client receives no packet
                // for, and the ground under a lake is not where they are.
                if !swimming {
                    if let Some(h) = world.and_then(|w| w.floor(x, y, position.z)) {
                        // The unit follows the ground up a slope, and down
                        // only by less than `FALL_THRESHOLD`, so it is not
                        // dropped off a ledge. A drop is left for the server
                        // to state: nothing here simulates the fall, the unit
                        // never announced one, and inventing a fall would put
                        // a unit running across a bridge under it whenever the
                        // hull below answered first.
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
            // The whole step, not clamped: this is the server's path and the
            // server walked all of it. See the note at the top.
            spline.elapsed_ms = spline.elapsed_ms.saturating_add(dt_ms);
            // A cyclic path wraps rather than ending — see `Spline::wrap`. A
            // step longer than a whole lap would need more than one wrap, which
            // only happens if the thread stalled for the length of a patrol.
            while spline.wrap() {}
            // The position in world coordinates, which for a passenger's
            // spline differ from the path's coordinates.
            // `SMSG_MONSTER_MOVE_TRANSPORT` states the path in the transport's
            // frame. `in_world` converts it, and returns `None` for a
            // transport this client is not tracking; the unit then stays where
            // it was rather than being placed at the map's origin. See
            // [`crate::state::movement::Spline::in_world`].
            let Some((position, heading)) = spline.in_world(world) else {
                continue;
            };
            let finished = spline.finished();

            // A unit faces the leg it is walking, and on arrival turns to the
            // facing the packet requested. The `Target` case is a creature that
            // has run up to a player and faces them. It is resolved when the
            // packet arrives, because only the manager can see another entity.
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

            // The leg's height is taken from the ground it crosses. The server
            // states a straight line in three dimensions between two points it
            // placed on the ground. The terrain between them is not straight,
            // so a creature drawn on the line floats over hollows and sinks
            // into rises. The rule and the measurement behind it are at
            // [`Spline::grounded_z`], which returns `None` for every spline
            // whose height does not follow the ground: a flight, a knock-back
            // and a map with no terrain loaded all keep the straight line.
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

    /// Puts a standing unit on the surface under it, once per server statement
    /// of its position.
    ///
    /// ## The floating-NPC report and the measurements behind this rule
    ///
    /// The report was "Many NPCs appear to float. Example: Peltskinner
    /// questgiver in Northshire." Eagan Peltskinner is `creature.guid` 79971, a
    /// spawn with `movement_type = 0` and `wander_distance = 0`. He never
    /// moves, so [`crate::state::movement::Spline::grounded_z`] does not apply
    /// to him. Four measurements, none of them taken from a screenshot:
    ///
    /// * The world database spawns him at z = 80.9719. vmangos'
    ///   `Creature::LoadFromDB` places a living DB spawn at that value
    ///   unchanged (`CreatureCreatePos::SelectFinalPoint` does nothing without
    ///   a `m_closeObject`), so that value is what the server sends.
    /// * This client's terrain gives 80.2051 at that point. vmangos' extracted
    ///   `0004832.map`, decoded by hand with `GridMap::getHeightFromUint16`,
    ///   also gives 80.2051. The client and server agree about the ground to
    ///   four decimal places.
    /// * `vale collision Azeroth 32 48 -8869.22 -163.237 80.9719` finds nothing
    ///   under him: no building hull and no doodad hull. The nearest drawn
    ///   prop is a barrel 1.6 yards away, on the ground at 80.24.
    /// * He therefore stands 0.767 yards above bare grass, and this client
    ///   drew him where the server placed him.
    ///
    /// The rest of the population shows a smaller version of the same gap.
    /// Over the 14,193 stationary ground spawns on maps 0 and 1, the most
    /// common gap is +0.18, an offset that whatever generated the table added
    /// to every spawn. About 7% are more than 0.3 above the terrain.
    ///
    /// ## What the 1.12.1 client does with the same spawn
    ///
    /// A first version of this rule was marked as a deviation from the 1.12.1
    /// client, on the assumption that the client does not ground units. That
    /// assumption was wrong. Logged into the same vmangos server, the 1.12.1
    /// client shows Eagan Peltskinner standing on the grass, although it
    /// receives the same 80.9719. The client therefore grounds a standing
    /// unit. The rule below reproduces that observed behaviour; its exact
    /// conditions in the client are not known.
    ///
    /// The bound remains, for a different reason: this client's collision is
    /// less complete than the 1.12.1 client's. 25 of the 75 doodad models on
    /// Northshire's tile have no collision hull, so a unit the server placed on
    /// one of them has nothing here to be compared against. Beyond the band,
    /// assuming the server placed the unit there on purpose is safer.
    ///
    /// [`STAND_BAND`] and these conditions complete the rule:
    ///
    /// * Only a standing unit: no spline, no movement flags that say it is
    ///   moving, not swimming, not airborne. Moving units are handled where
    ///   they move.
    /// * Only downwards, and only within the band. A unit the server placed
    ///   more than a band above the surface under it is there on purpose (a
    ///   flying creature, a platform this client has no hull for, a boss on a
    ///   dais) and is left alone. No unit is ever raised.
    /// * The surface is [`crate::state::movement::Footing::floor`]'s answer, so
    ///   a unit on a building's floor is compared against that floor and not
    ///   against the terrain under the building. A building that has not
    ///   streamed in yet answers `None`, which leaves the unit where the server
    ///   put it.
    /// * The unit's movement flags then adjust the surface, through
    ///   [`crate::state::movement::standing_surface`]: a water-walker's surface
    ///   is the water, and a hovering unit's is a yard above whichever surface
    ///   it stands on. This client held both flags correctly but did not draw
    ///   them, and for a standing unit the previous rule pulled a water-walker
    ///   down to the lake bed.
    ///
    /// One known case is wrong: a unit standing on a prop with no collision
    /// hull (a crate, a haystack, a rock the game lets players walk through) is
    /// lowered to the ground under it. The player also walks through that prop
    /// in this client, so the rest of this client already treats the unit as
    /// standing on nothing.
    ///
    /// Cost: one [`Footing::floor`] per entity per server position statement,
    /// not per step. A standing creature's position is stated once, so each
    /// idle unit in a city pays this once. [`Entity::grounded_for`] is
    /// therefore a latch on [`Entity::position_updates`] rather than a bool.
    fn stand_on_the_ground(&mut self, world: Option<&dyn crate::state::movement::Footing>) {
        let Some(world) = world else { return };
        // The beat limits how often entities with no answer yet are retried;
        // see [`STAND_BEAT`]. An entity already grounded costs one comparison
        // regardless, so the gate only matters for the login burst.
        self.stand_beat = self.stand_beat.wrapping_add(1);
        if !self.stand_beat.is_multiple_of(STAND_BEAT) {
            return;
        }
        let player_guid = self.player_guid;
        for entity in self.entities.values_mut() {
            // The live session owns the player's height and runs a full mover
            // for it, including jumps and falls.
            if Some(entity.guid) == player_guid || entity.spline.is_some() {
                continue;
            }
            if entity.grounded_for == Some(entity.position_updates) {
                continue;
            }
            // The two flags that change which surface the unit stands on,
            // taken from the same accessor the rest of the code reads; see
            // [`crate::state::movement::standing_surface`]. Zero for a unit
            // with no movement block, which has neither flag.
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
            // The latch is set only when the world answers. A login burst
            // creates every entity in view before the tiles under them have
            // streamed in, so latching on `None` would leave all of them
            // floating for the rest of the session. The retry is paced by
            // [`STAND_BEAT`] instead.
            //
            // The liquid is queried only for units with
            // `MOVEFLAG_WATERWALKING`, which are very few. A second world
            // query per entity per statement for that flag would double the
            // cost of this pass with no effect on the other units.
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
    /// No packet requests this turn; the client is expected to make it. A
    /// creature chasing a player faces the way it is walking, which is roughly
    /// towards them. When the chase spline finishes,
    /// `TargetedMovementGenerator` calls `owner.SetInFront(i_target.getTarget())`,
    /// which only calls `SetOrientation`: it sends no packet, no spline and no
    /// field. Without this pass a creature keeps facing wherever its last
    /// spline ended, and a player who strafes around it is hit by a creature
    /// facing sideways. That looks like an animation or interpolation fault
    /// but is neither.
    ///
    /// vmangos states this expectation twice and never sends anything for it.
    /// `TotemAI` writes it beside the call:
    /// `SetInFront(victim); // client change orientation by self`.
    /// `Object::BuildValuesUpdate` writes it beside the field, where a casting
    /// creature's casting target is substituted into `UNIT_FIELD_TARGET` for
    /// this reason: "This is done to make creatures face the target they are
    /// casting on." The server expects the client to turn the creature
    /// towards the value in that field.
    ///
    /// Measured on a live vmangos with `vale live` and `goto`: a Carrion Lurker
    /// finished its chase spline 169 degrees off the bearing to the player
    /// standing 0.2 yards from it, and stayed there. With this pass it turns
    /// through those 169 degrees in the 0.9 s the turn rate allows and then
    /// holds at zero.
    ///
    /// Creatures only. A player's orientation arrives in their own
    /// `MSG_MOVE_*` broadcasts, and vmangos sends a player a `SetFacingTo`
    /// spline where it sends a creature nothing (`TargetedMovementGenerator.cpp`,
    /// the two branches on either side of the same `Finalized()` check).
    /// Turning players here would conflict with those packets.
    ///
    /// Stationary units only, for the same reason `apply_monster_move` gives
    /// travel direction priority: a unit walks forwards, so while it moves it
    /// faces the leg it is walking, not its target.
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
            // This client cannot place a target that is out of sight, and
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
            // Rate-limited rather than set at once. The rate is the server's
            // `MOVE_TURN_RATE`, pi rad/s by default and sent in every movement
            // block, so a unit whose turn rate is changed turns at the changed
            // rate. Setting the facing at once would show as a single
            // 180-degree jump: `Motion` interpolates facing, but only across
            // one 25 ms simulation step.
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
        // The inventory latch is also collected and applied at the end, since
        // the block that changes it is often the block that created the object.
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
                    // A creature that comes into view mid-walk is still walking,
                    // and the create block's inline spline is the only statement
                    // of it: `SMSG_MONSTER_MOVE` was sent when the move started,
                    // to the clients that could see it then. The spline is
                    // adopted with the server's `timePassed`, so the unit
                    // continues from the server's current point on the path
                    // rather than from its start.
                    entity.adopt_spline(movement.spline.as_ref());
                    // `path_progress` is the only field that states an
                    // elevator's position. It replaces the local value rather
                    // than being added to it: a new block is the server's
                    // current phase, so it overrides any drift on this side.
                    // See [`Entity::transport_phase_ms`].
                    if let Some(progress) = movement.path_progress {
                        entity.transport_phase_ms = Some(u64::from(progress));
                    }
                    if let Some(speeds) = movement.speeds {
                        entity.speeds = Some(Speeds(speeds));
                    }
                    for (index, value) in &values.fields {
                        entity.fields.insert(*index, *value);
                    }
                    // A field the create block did not carry is zero rather than
                    // unknown; see [`Entity::created`]. The flag is set after
                    // the fields, so it never disagrees with the values.
                    entity.created = true;
                    // A create block is the only way a new entry arrives, so it
                    // is one of the two events that request a query pass; see
                    // [`Self::take_query_hint`].
                    self.queries_wanted = true;
                    if movement.is_self() {
                        entity.is_self = true;
                        self.player_guid = Some(*guid);
                    }
                    inventory_moved |= touches_inventory(entity, &values.fields);
                }

                UpdateBlock::Values { guid, values } => {
                    // A values update can arrive for an object whose create
                    // block this client never received (for example after
                    // joining mid-stream). Tracking it keeps data that would
                    // otherwise be lost.
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
                        // An item leaving is also an inventory change.
                        // [`ObjectManager::remove`] decides that, for this arm
                        // and for `SMSG_DESTROY_OBJECT`, which is the packet
                        // items normally leave by.
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
            // The second event that requests a query pass: something the
            // character carries changed, including the entry of a slot an item
            // was just looted into.
            self.queries_wanted = true;
        }
        for (guid, moved, source) in jumped {
            self.note_jump(guid, moved, source);
        }
    }
}

/// Does this block change what the character is carrying?
///
/// Two cases with different tests. Any field on an `Item` or a `Container`
/// counts (a stack count, a durability, the entry itself), because every one
/// of them is drawn. On the local player only the three slot ranges count, because
/// a player's block also carries health, power, position and twenty other
/// fields that change constantly. Treating those as inventory changes would
/// rebuild the bags every tick and make the latch useless.
///
/// The test errs towards rebuilding. The ranges are checked by index rather
/// than by meaning, so an unnamed field inside one of them still triggers a
/// rebuild. A redundant rebuild costs a hash walk; a missed one leaves a bag
/// that never updates.
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
            // The buyback is the third range, made of two runs. A sale
            // changes no field a bag square draws: the item is not destroyed
            // but moved into `PLAYER_FIELD_VENDORBUYBACK_SLOT_n`. Without these
            // runs the vendor's buyback tab would update only when an
            // unrelated field changed. The price and the timestamp are
            // adjacent runs of twelve and are covered as one range.
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

/// Whether this object is part of what the character is carrying.
///
/// The test behind both the destroy latch in [`ObjectManager::remove`] and the
/// first arm of [`touches_inventory`]. Any field on an `Item` or a
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
