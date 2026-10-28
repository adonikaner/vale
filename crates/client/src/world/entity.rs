//! The world's objects as ECS state: [`WorldEntity`], one per guid the server
//! has described, and the two systems that keep them current.
//!
//! [`poll_world`] reconciles the session's snapshot into the components once
//! per simulation step. [`place_entities`] writes each entity's interpolated
//! position onto its `Transform` every frame. Both are registered by
//! [`super::session::SessionPlugin`], which states their order.

use crate::world::motion::Motion;
use crate::world::session::Session;
use crate::world::status::{Counters, WorldStatus};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use vale_assets::tile_for_position;

/// A unit's combat reach when the field block has not set one: 1.5 yards.
/// This is vmangos' `DEFAULT_COMBAT_REACH` (`Objects/ObjectDefines.h:51`),
/// which vmangos writes for a unit whose display has no addon
/// (`Unit.cpp:9656`).
///
/// A default of zero would make every unit a point, and a range check must not
/// treat a unit as a point.
const DEFAULT_COMBAT_REACH: f32 = 1.5;

/// A unit's bounding radius when the field block has not set one: 0.389
/// yards. This is vmangos' `DEFAULT_WORLD_OBJECT_SIZE`
/// (`Objects/ObjectDefines.h:42`), which its comment describes as "actually
/// the bounding_radius, like player/creature from creature_model_data".
///
/// A default of zero would give a selection ring with no radius.
/// `Unit::UpdateModelData` falls back to 1.5 for a unit whose display has no
/// addon at all. 0.389 is the size the game uses for an object of ordinary
/// size, and it is the safer of the two to draw a circle from.
const DEFAULT_BOUNDING_RADIUS: f32 = 0.389;

/// One entity as the renderer wants it: the ECS form of the old
/// `EntitySnapshot`, without the serialisation.
///
/// `PartialEq` is required, not a convenience. [`poll_world`] rebuilds one of
/// these per entity per simulation step. It used to write it over the component
/// unconditionally. The Tauri renderer needed that, because every snapshot
/// crossed an IPC boundary anyway, and the code survived the migration that
/// removed the boundary. In-process, it made Bevy's `Changed<WorldEntity>` true
/// for every entity on every step whether or not anything changed, so no
/// consumer could use it to skip work, and each had to write its own comparison
/// (`EntityModel::matches` is one). Writing only on a real difference makes
/// Bevy's change detection accurate: a city of two hundred standing guards
/// reports no change.
/// The kind of object a [`WorldEntity`] is, re-exported.
///
/// The field is public, so the type has to be nameable. Without the re-export,
/// anything that builds a `WorldEntity` (a test, the crowd, a host with no
/// server) would have to depend on `vale-protocol` to name the kind.
pub use vale_protocol::state::update::ObjectType;

/// One of a unit's aura slots, re-exported for the same reason: the field is
/// public. The state half of the spell chain is driven by the aura list rather
/// than by a counter.
pub use vale_protocol::state::objects::AuraSlot;

/// `Default` is a real constructor, not a test convenience. It is an object of
/// no type, with no level and no name, which describes a guid this client has
/// seen and been told nothing else about. Tests build from it so that a new
/// field does not have to be added to fifty literals that do not use it.
#[derive(Component, Clone, PartialEq, Default)]
pub struct WorldEntity {
    pub guid: u64,
    pub kind: ObjectType,
    pub name: String,
    /// The `<Innkeeper>` tag under the name, and the two numbers beside it:
    /// the `CreatureType.dbc` row and the classification (0 normal, 1 elite,
    /// 2 rare elite, 3 boss, 4 rare).
    ///
    /// All three come from the creature template, and none is in an update
    /// block. They are absent for the round trip `CMSG_CREATURE_QUERY` takes,
    /// and empty or zero for every player. They are carried because the unit
    /// tooltip's middle and right cells are built from them; see
    /// [`crate::interface::api::UnitTip`].
    pub sub_name: String,
    pub creature_type: u32,
    /// The `CreatureFamily.dbc` row, from the template: wolf, cat, boar; 0 for
    /// anything that is not a tameable beast.
    ///
    /// It sits beside `creature_type` because it comes from the same packet and
    /// is resolved at the same moment, and because `UnitCreatureFamily` asks
    /// about any unit, not only the pet; see [`crate::lua::panels::pet`]. It is
    /// zero until `CMSG_CREATURE_QUERY` returns, so a freshly summoned pet has
    /// no family name for a frame or two. `PetPaperDollFrame_Update` checks for
    /// that and omits the word.
    pub pet_family: u32,
    pub classification: u32,
    pub entry: Option<u32>,
    pub level: Option<u32>,
    /// `(current, maximum)`: what a bar is filled from, and all that this
    /// client reads about a unit's health.
    ///
    /// There is no pre-formatted string beside it. A `health: Option<String>`
    /// field used to hold `Entity::health_label`'s `"1234/5678 hp"` or `"57%"`,
    /// and `poll_world` built it with a `format!` for every entity on every poll
    /// while nothing in the renderer read it. The nameplate, the unit frames and
    /// the tooltip all fill from this pair, and the label that the server's
    /// `ShowHealthValues` setting selects is composed where it is displayed. The
    /// field cost one heap allocation per unit per poll, which grows with the
    /// number of NPCs in one place, so it was removed. `Entity::health_label`
    /// remains for the CLI, which does want a formatted line (`vale live`,
    /// `vale login`). `None` for anything that never had health.
    pub health_value: Option<(u32, u32)>,
    /// `(current, maximum, power type)`: mana, rage, focus or energy.
    ///
    /// A byte decides which of the five power fields to read. A bar drawn from
    /// `POWER1` regardless is empty for every warrior and rogue. See
    /// `vale_protocol::state::objects::power_type`.
    pub power_value: Option<(u32, u32, u8)>,
    /// `DISPLAYID`: which model to draw. Resolved once per distinct id.
    pub display_id: Option<u32>,
    /// A player's own appearance, which its skin is composed from.
    ///
    /// `None` for everything that is not a player. A creature's skin is a file
    /// the display tables name; only a player's has to be built.
    pub appearance: Option<vale_assets::look::character::Appearance>,
    /// `(race, class)`, the one-based ids, from the same `UNIT_FIELD_BYTES_0`
    /// the appearance takes its race from.
    ///
    /// It sits beside the appearance rather than in it, because class does not
    /// change how a character looks and dressing one does not use it. The
    /// spellbook uses it: the two masks on a `SkillLineAbility` row decide which
    /// page a spell goes on. See [`crate::interface::spellbook`].
    pub race_class: Option<(u8, u8)>,
    /// The third byte of the same field, which `UnitSex` returns; see
    /// [`vale_protocol::state::objects::Entity::gender`]. It is separate from
    /// [`Self::race_class`] because the two cover different units: every unit
    /// has a gender byte, and only a player has an [`Self::appearance`] to read
    /// one from.
    pub gender: Option<u8>,
    /// What a player is wearing: `(ItemDisplayInfo id, InventoryType)` per
    /// visible slot, empty for everything that is not a player. A player
    /// whose guild has an emblem has one more pair after the items, which is
    /// the emblem and not an item; see [`vale_assets::look::emblem`].
    ///
    /// Both numbers come from the server. `PLAYER_VISIBLE_ITEM_n_0` carries an
    /// item entry, and `Item.dbc` is not in the 1.12 archives, so the display
    /// id and the slot arrive by `CMSG_ITEM_QUERY_SINGLE`. A player is therefore
    /// drawn undressed briefly and then dressed, as a creature is briefly
    /// nameless. The alternative is holding the whole model back for a round
    /// trip.
    pub equipment: Vec<(u32, u32)>,
    /// `OBJECT_FIELD_SCALE_X`, which already includes the display and model
    /// scales from the DBCs.
    pub scale: Option<f32>,
    /// `UNIT_FIELD_COMBATREACH`: how much of the gap between two units is
    /// their own bulk rather than distance.
    ///
    /// Every range the server checks is surface to surface, so a client that
    /// measures centre to centre reads a kodo at melee range as five yards
    /// away. Defaults to vmangos' `DEFAULT_COMBAT_REACH` for a unit that has not
    /// stated one, which is the value vmangos writes for a unit with no display
    /// addon. Its only reader is
    /// [`vale_assets::tables::spellbook::check_cast`].
    pub combat_reach: f32,
    /// `UNIT_FIELD_BOUNDINGRADIUS`: the radius of the unit's footprint on the
    /// ground, in world yards, scale included.
    ///
    /// The reach above says how much of a gap is bulk; this says how wide the
    /// bulk is. It is the only footprint the game states. The M2's own box is
    /// authored to cover every frame of every animation and answers a different
    /// question. [`crate::render::selection`] draws the ring from this.
    ///
    /// The value is already in world space. `place_entities` puts
    /// `OBJECT_FIELD_SCALE_X` on the transform, and the server has already
    /// folded the same scale into this, so code working in a scaled frame must
    /// divide the scale back out rather than let the transform apply it twice.
    pub bounding_radius: f32,
    /// Whether this entity is moving, and how fast in yards per second.
    ///
    /// Both are stated, not computed from position differences. They come from
    /// the flags or the spline that the client's dead reckoning already advances
    /// the entity by, so they cannot disagree with the drawn motion. The
    /// difference between two interpolated positions reads zero for the last
    /// frames of every snapshot interval, and using it restarted an animation
    /// twenty times a second.
    pub moving: bool,
    pub speed: f32,
    /// `MOVE_TURN_RATE`, the sixth speed: pi rad/s by default, and the rate at
    /// which the drawn body turns towards the aim. See [`super::facing`], which
    /// multiplies it by eight.
    ///
    /// Carried rather than assumed because the server sends it in every
    /// movement block and changes it: a unit whose turn rate is raised or
    /// lowered turns its body at the new rate too. `Speeds::default` is only
    /// the fallback for an entity whose block has not arrived.
    pub turn_rate: f32,
    /// The movement flags, unchanged. With `moving` and `speed` they are the
    /// three inputs that decide a gait.
    ///
    /// Without the flags, a character backing out of a fight and a character
    /// charging into one have the same `(moving, speed)` pair and are drawn
    /// identically: the walk cycle running forwards while the body slides
    /// backwards.
    ///
    /// The field holds the flags rather than a direction, because the client's
    /// rules are stated in terms of the flags and three separate rules read
    /// this: the gait, whose precedence differs between ground and water; the
    /// turn-in-place shuffle, which is a flag no direction enum has room for;
    /// and `movement::strafe_body_offset`, which must tell a pure strafe from a
    /// diagonal. See `vale_protocol::state::objects::Entity::move_flags` for the
    /// one substitution made when the flags are read.
    pub move_flags: u32,
    /// The moving platform this unit is riding, as a guid, or `None` for
    /// everything standing on the ground.
    ///
    /// For other units this is the `MOVEFLAG_ONTRANSPORT` guid from the unit's
    /// own movement block. For the local player it is `SessionStatus::ferry`,
    /// because the player's movement never comes back from the server. See
    /// [`vale_protocol::state::movement::Ferry`].
    ///
    /// Read by [`crate::world::facing`], which must turn the body in the deck's
    /// frame. A passenger standing still on a turning boat has a world heading
    /// that rotates; measured in world axes, that reads as the character turning
    /// on the spot, and the feet shuffle for the whole crossing.
    pub platform: Option<u64>,
    /// Off the ground, and whether it began with a jump.
    ///
    /// The pair has two sources, because the player is the one entity whose
    /// movement never comes back from the server. For every other unit it is
    /// `MOVEFLAG_JUMPING` on the last broadcast, plus an upward `zspeed` in the
    /// jump block, which separates a jump from a step off a ledge. For the
    /// player it is the mover's own arc.
    pub airborne: bool,
    pub jumping: bool,
    pub is_self: bool,
    /// The item in the main hand, as an item guid. Set only for the local
    /// player, the only unit anything asks about.
    ///
    /// One field rather than a walk of the bags, because the caller is the
    /// aiming rule and it runs on every press. A spell carrying
    /// `SPELL_ATTR_HELD_ITEM_ONLY` targets this item without asking the player,
    /// and when this is empty the rule reports "Your weapon hand is empty".
    /// See [`vale_assets::tables::spellbook::CastAim::Item`] and
    /// [`vale_protocol::state::objects::items::equipped_guid`].
    pub main_hand_item: Option<u64>,
    /// What this character is looking through, set only for the local player:
    /// `PLAYER_FARSIGHT`, the guid of a far-sight `DynamicObject` or of a
    /// possessed unit. See [`vale_protocol::state::objects::Entity::farsight`],
    /// which covers both kinds of spell.
    ///
    /// One field for Eagle Eye, Far Sight, Bird's Eye, Eye of Kilrogg, Mind
    /// Control and Eyes of the Beast, because the server states all six the
    /// same way. `None` for a whole session that casts none of them. Read only
    /// by [`crate::world::camera::follow_player`].
    pub farsight: Option<u64>,
    /// Health has reached zero. A corpse holds the last frame of Death, because
    /// no 1.12 creature model carries a sequence for `Dead` (id 6), which is in
    /// `AnimationData.dbc` and in none of the 411 models.
    pub dead: bool,
    /// Feign Death: `UNIT_DYNAMIC_FLAGS` bit 5, which is the only thing the
    /// wire says about it. See
    /// [`vale_protocol::state::objects::Entity::is_feigning`].
    ///
    /// Kept separate from [`Self::dead`] because the two describe the same
    /// picture differently: this body is drawn as a corpse and is alive in every
    /// other respect, with no release box, no ghost, and a full health bar.
    /// `wanted_animation` is the one consumer that treats them alike, as the
    /// 1.12.1 client does: it combines the two for the display only.
    pub feigning: bool,
    /// The spirit has been released: `PLAYER_FLAGS_GHOST`, the one thing that
    /// separates a ghost from a corpse.
    ///
    /// Carried because it cannot be derived from [`Self::dead`]: a released
    /// ghost has health 1 and reads as alive. See
    /// [`vale_protocol::play::death`] and [`crate::interface::death`].
    pub is_ghost: bool,
    /// `PLAYER_FIELD_BYTES`' `RELEASE_TIMER` bit: whether the release box
    /// counts down at all. It is clear inside an instance, which makes
    /// `GetReleaseTimeRemaining()` return `-1`. `PRIVATE`, so it is only ever
    /// true for the local player.
    pub timed_release: bool,
    /// `PLAYER_XP` and `PLAYER_NEXT_LEVEL_XP`: `UnitXP` / `UnitXPMax`, and the
    /// pair the main bar is drawn from. `PRIVATE`: `None` for everyone else.
    pub experience: Option<(u32, u32)>,
    /// `PLAYER_CHARACTER_POINTS1` and `2`: unspent talent points and unspent
    /// profession points, the pair `UnitCharacterPoints("player")` returns.
    /// `PRIVATE`: `None` for everyone else. See
    /// [`crate::interface::talents`].
    pub character_points: Option<(u32, u32)>,
    /// `PLAYER_REST_STATE_EXPERIENCE`: `GetXPExhaustion()`, `None` when there
    /// is none, which is the test the interface makes. See
    /// [`vale_protocol::state::objects::Entity::rested_experience`].
    pub rested: Option<u32>,
    /// `UNIT_FLAG_IN_COMBAT`: the unit is in a fight. Read by the interface
    /// (`UnitAffectingCombat`) and by Tab-targeting, and not by the pose; the
    /// ready stance is drawn from [`Self::attacking`].
    pub in_combat: bool,
    /// This unit is auto-attacking the unit it is looking at:
    /// `SMSG_ATTACKSTART` since the last `SMSG_ATTACKSTOP` about it, and
    /// [`Self::target`] still names the same guid.
    ///
    /// This gates the combat-ready stance. The 1.12.1 client draws that stance
    /// from an engagement state, not from `UNIT_FLAG_IN_COMBAT` and not from the
    /// sheath state. A unit that is only in combat idles.
    ///
    /// The second half of the test fixes a report that the attack animation
    /// stayed on after the target was cleared. Clearing the target does not
    /// stop the swing in either direction. `ClearTarget` is `SetTarget(0)`,
    /// which sends `CMSG_SET_SELECTION`, fires the target-changed event, and
    /// sends no attack-stop of any kind; vmangos' `HandleSetSelectionOpcode`
    /// cancels only an auto-repeat, never the melee. So the swing continues,
    /// and the report is about the pose: the guard stance is held while a unit
    /// is fighting and looking at its opponent. For the local player that is
    /// the selection, since `SetSelectionGuid` writes `UNIT_FIELD_TARGET`.
    ///
    /// The two packets and the `ClearTarget` behaviour are measured. The
    /// combination of the two conditions is taken from the report, because the
    /// client-side state the 1.12.1 client uses has not been identified.
    pub attacking: bool,
    /// `UNIT_FIELD_FACTIONTEMPLATE` and `UNIT_FIELD_FLAGS`.
    ///
    /// Together these decide whether the unit may be attacked, and neither can
    /// be derived from anything else the snapshot carries. The template is a
    /// number whose meaning is 314 rows of `FactionTemplate.dbc`
    /// (`vale_assets::tables::faction`), and the flags carry the five bits that
    /// rule a unit out whatever its faction says. Read by [`crate::interface`]
    /// for Tab-targeting, for the target frame's colour, and for choosing a
    /// cast's target.
    pub faction: Option<u32>,
    pub unit_flags: u32,
    /// `PLAYER_FLAGS`, `0` for anything that is not a player.
    ///
    /// Three of its bits decide friend or foe: free-for-all, contested and
    /// ghost. They are read together with the two duel fields below, because
    /// `vale_assets::tables::faction::Party` takes all five at once. See that
    /// module: without them a free-for-all player of the character's own
    /// faction draws green and cannot be clicked, while the server has the two
    /// at war.
    pub player_flags: u32,
    /// `PLAYER_GUILDID` and the guild's name, for a player. The id is 0 for a
    /// player in no guild. The name is empty for that player and until
    /// `SMSG_GUILD_QUERY_RESPONSE` answers for the id; see
    /// [`vale_protocol::state::objects::ObjectManager::guilds`]. The floating
    /// name's second line is the name.
    pub guild_id: u32,
    pub guild: String,
    /// The honor rank and city title bytes of `PLAYER_BYTES_3`; see
    /// [`vale_protocol::state::objects::Entity::pvp_rank_and_medal`]. The
    /// floating name and the unit tooltip put the rank's title in front of a
    /// player's name.
    pub pvp_rank: u8,
    pub pvp_medal: u8,
    /// The line that names this unit's owner, as a title and the owner's
    /// name: a pet, a minion, a guardian or a summoned object. `None` for a
    /// player, for a unit nobody owns, and for one whose owner is not in the
    /// world. See [`vale_assets::look::unitname::OwnerTitle`].
    pub owner: Option<(vale_assets::look::unitname::OwnerTitle, String)>,
    /// `PLAYER_DUEL_ARBITER` and `PLAYER_DUEL_TEAM`: `0`/`0` for almost every
    /// whole session. The 1.12.1 client decides a player-versus-player reaction
    /// on this pair before it looks at either faction.
    pub duel_arbiter: u64,
    pub duel_team: u32,
    /// `UNIT_NPC_FLAGS`: the services this unit offers, such as gossip,
    /// quests, a shop, a stable, a flight path.
    ///
    /// Read for the pointer rather than for a panel. The 1.12.1 client checks
    /// these bits for the cursor before it asks whether the unit can be
    /// attacked, so a vendor or a quest giver never shows a sword whatever the
    /// faction table says. See [`vale_assets::look::cursor::over_unit`].
    pub npc_flags: u32,
    /// Whether this body has loot: `UNIT_DYNAMIC_FLAGS` bit 0, the only thing
    /// on the wire that tells a full corpse from a stripped one. It is set per
    /// viewer, so it already says whether a right-click would do anything for
    /// this character; see [`vale_protocol::state::objects::Entity::lootable`].
    pub lootable: bool,
    /// The unit this unit's `pet` token names, charm before summon; see
    /// [`vale_protocol::state::objects::Entity::pet_guid`], where the
    /// precedence was measured.
    ///
    /// Carried for every unit, not only the player, because `partypet<n>` is
    /// read from the group member. The 1.12.1 client uses the owner's live
    /// charm or summon when the owner is in view, and falls back to the group
    /// packet's cached guid only when the owner is not. This field is the first
    /// of the two sources and [`vale_protocol::play::group::PartyPetStats`] is
    /// the second.
    pub pet: Option<u64>,
    /// `UNIT_FIELD_SUMMONEDBY` and `UNIT_FIELD_PETNUMBER`: whether a pet
    /// belongs to this character and whether it is a pet. The pet menu and the
    /// pet panel check both. See
    /// [`vale_protocol::state::objects::Entity::summoned_by`] and its sibling,
    /// and [`crate::lua::panels::pet`], their only reader.
    pub summoned_by: Option<u64>,
    /// `UNIT_FIELD_CHARMEDBY`. The one rule that reads it together with
    /// [`Self::summoned_by`] is the minimap's classifier: it tests the charmer
    /// first and the summoner only when there is no charmer, which keeps a
    /// mind-controlled creature off the character's own map. See
    /// [`vale_assets::look::blips`].
    pub charmed_by: Option<u64>,
    /// `UNIT_DYNAMIC_FLAGS` bit 1: a hunter's mark, which the minimap shows
    /// whatever the character is tracking.
    pub hunters_marked: bool,
    /// The last `SMSG_QUESTGIVER_STATUS` for this unit said
    /// `DIALOG_STATUS_REWARD2`: a quest to hand in, which is the one status
    /// the minimap draws a dot for.
    pub quest_turn_in: bool,
    /// The character's own three tracking fields, `None` for every other unit:
    /// `(PLAYER_TRACK_CREATURES, PLAYER_TRACK_RESOURCES, TRACK_STEALTHED)`.
    /// See [`vale_protocol::state::objects::Entity::tracking`].
    pub tracking: Option<(u32, u32, bool)>,
    pub pet_number: u32,
    /// The five numbers a hunter's pet panel is drawn from, or `None` for
    /// anything that is not a pet; see
    /// [`vale_protocol::state::objects::Entity::pet_stats`], which fixes every
    /// index.
    ///
    /// One `Option` rather than five fields, and carried on the ordinary unit
    /// mirror rather than stored beside the bar, because the happiness value
    /// changes every few seconds and the panel reads it on `UNIT_HAPPINESS`.
    /// The condition is `pet_number != 0`, so this is `None` for every unit in
    /// the world but one.
    pub pet_stats: Option<vale_protocol::state::objects::PetStats>,
    /// What this unit has selected: `UNIT_FIELD_TARGET`.
    ///
    /// For the local player this is the server's echo of this client's
    /// `CMSG_SET_SELECTION`, which is useful as a cross-check: the client's
    /// selection is authoritative and immediate, so a disagreement means a
    /// `Target` command was dropped.
    pub target: Option<u64>,
    /// `UNIT_FIELD_BYTES_1` byte 0: for example, the innkeeper on his stool.
    pub stand_state: u8,
    /// `UNIT_FIELD_BYTES_1` byte 3: ghost, creep and untrackable, the three
    /// bits that say this unit is not an ordinary solid body.
    ///
    /// See [`vale_protocol::state::objects::Entity::vis_flags`] for what each
    /// bit is and for why nothing else is read: Stealth has no spell visual, so
    /// this byte is all the wire says. It has two consumers, the same two the
    /// 1.12.1 client has: the pose (`StealthWalk`/`StealthStand`) and how solid
    /// the body is drawn.
    pub vis_flags: u8,
    /// This character has a loot window open, and so is drawn crouched over the
    /// body with its arm out.
    ///
    /// It is the one field on this snapshot that no packet states. There is no
    /// loot row in `Emotes.dbc` and no unit field for it: the looting pose is a
    /// rule of the client, driven by a window only the client knows is open.
    /// Everything else here is the server's answer to something.
    ///
    /// So it is only ever true for the local player, because no other
    /// character's loot window is visible from here. [`poll_world`] sets it from
    /// [`crate::interface::loot::LootWindow`] rather than from the object
    /// manager. See `crate::world::entities::pose::wanted_animation`, the
    /// consumer, and `vale_assets::world::m2::anim::LOOT`, the measurement of
    /// the clip.
    pub looting: bool,
    /// `GAMEOBJECT_STATE`: a door open or shut, a chest looted or not.
    ///
    /// It is the only thing the server says about a game object's appearance.
    /// A game object's model carries `Closed`/`Opened` and their two one-shots
    /// and no `Stand`, so without this field the gait chooser's default found
    /// no sequence and a chest was drawn in its bind pose. `None` for everything
    /// that is not a game object.
    pub object_state: Option<u8>,
    /// `GameObjectInfo::type`: what kind of thing a game object is, which is
    /// the only input that says whether a click on it is worth sending.
    ///
    /// Zero for everything that is not a game object and for a game object
    /// whose template has not arrived yet. For the pointer those are the same
    /// answer: type 0 is `Door`, and a door with no lock and no state is what an
    /// unresolved entry looks like. See [`vale_assets::look::object`], which
    /// defines what the number means.
    pub object_kind: u32,
    /// The lock id held in the template word that this type uses for its lock,
    /// or zero for an unlocked object.
    ///
    /// Resolved at the snapshot rather than carried as the whole 24-word union,
    /// because which word holds the lock is a per-type rule
    /// ([`vale_assets::look::object::Kind::lock_word`]) and every consumer
    /// wants the answer rather than the array. It separates an ore vein from a
    /// strongbox, which are both `Chest`.
    pub object_lock: u32,
    /// What the object is doing now, which the template cannot say:
    /// `GAMEOBJECT_FLAGS`.
    ///
    /// Three of its bits decide whether a click is worth sending at all and a
    /// fourth puts "Locked" on the plate; see
    /// [`vale_assets::look::object::go_flags`]. A chest another player's kill
    /// produced, a door mid-swing and an event object standing inert between
    /// events are all ordinary templates whose field says not to touch them.
    /// A pointer judged from the type alone showed a hand over every one of
    /// them.
    pub object_flags: u32,
    /// The server's per-player answer to the one of those bits that depends on
    /// the viewer: `GAMEOBJECT_DYN_FLAGS`. See
    /// [`vale_assets::look::object::go_dyn_flags`].
    pub object_dyn_flags: u32,
    /// `GAMEOBJECT_LEVEL`: the rank that each of the five lock slots wanting 0
    /// falls back to.
    ///
    /// It is nearly always zero, and zero is correct, not missing data. vmangos
    /// writes the field for transports only, and the 1.12.1 client uses this
    /// field rather than the chest template's `level` word. So an `Open`-type
    /// lock with no rank can be opened by anybody, which is what an unlocked
    /// quest goober should allow.
    pub object_level: u32,
    /// The page id and material word held in this type's template union, or
    /// `(0, 0)` for a type that has none.
    ///
    /// Resolved here for the same reason as [`Self::object_lock`]: which words
    /// hold a page is a per-type rule
    /// ([`vale_assets::look::object::Kind::page_words`]) and the consumer
    /// wants the answer. A page id of zero is the ordinary case; most goobers
    /// are braziers with nothing written on them. See
    /// [`crate::interface::pagetext`].
    pub object_page: (u32, u32),
    /// What hovering over the object does, which is a separate question from
    /// whether a click does anything: a street sign is a game object that can
    /// only be looked at, and its plate follows the pointer. Resolved at the
    /// snapshot from the template's words; see
    /// [`vale_assets::look::object::hover_of`].
    pub object_hover: vale_assets::look::object::Hover,
    /// The one-shot animations the server has asked this game object to play,
    /// as a counter, and the last one; see
    /// [`vale_protocol::state::objects::Entity::object_anims`]. Zero and
    /// `None` for anything that is not a game object.
    pub object_anims: u32,
    pub object_anim: Option<vale_protocol::play::object::ObjectAnim>,
    /// In water deep enough to swim in: `MOVEFLAG_SWIMMING`, the server's
    /// answer rather than a guess from the liquid surface.
    pub swimming: bool,
    /// How far the body is tipped nose-up, in radians.
    ///
    /// Zero for everything that is not swimming, because the wire carries the
    /// field only under `MOVEFLAG_SWIMMING`; see
    /// [`vale_protocol::state::objects::Entity::pitch`], which applies that
    /// condition. `place_entities` turns it into the drawn rotation through
    /// [`crate::render::axes::body`].
    pub pitch: f32,
    /// Swings thrown and blows taken, as counters. A renderer compares them
    /// against what it last saw and fires a one-shot on the difference; see
    /// `Entity::swings_thrown` for why they are counters and not timestamps.
    pub swings_thrown: u32,
    /// `HitInfo` of the swing that last moved `swings_thrown`, which says
    /// which of the three swings is due: the main hand's, the off hand's, or
    /// the critical.
    pub last_swing_info: u32,
    /// What the victim did about that swing, as seen on the attacker's side,
    /// which decides the sound the swing makes. See
    /// `vale_protocol::state::objects::Entity::last_swing_state`.
    pub last_swing_state: u32,
    /// Who that swing was aimed at, so the impact sound plays at the victim.
    pub last_swing_victim: u64,
    /// How much damage that swing did, which is the number shown over the
    /// victim's head. See `vale_protocol::state::objects::Entity::last_swing_damage`
    /// and [`crate::ui::worldtext`].
    pub last_swing_damage: u32,
    /// What the local player has done to this unit, as one counter over both
    /// melee and spells; see
    /// `vale_protocol::state::objects::Entity::damage_taken`, which explains
    /// why it is one channel, and `crate::ui::worldtext`, which compares it
    /// against the last value it saw.
    pub damage_taken: u32,
    pub last_damage: u32,
    /// `HitInfo` when [`Self::last_damage_spell`] is `None` and `SpellHitType`
    /// when it is not. The two use different bit values: a swing's crit is
    /// `0x80` and a spell's is `0x02`.
    pub last_damage_info: u32,
    pub last_damage_state: u32,
    pub last_damage_spell: Option<u32>,
    pub healed: bool,
    pub blows_taken: u32,
    /// `VictimState` of the blow that last moved `blows_taken`, which says
    /// which of the four reactions is due: a flinch, a sidestep, a parry, a
    /// block.
    pub last_victim_state: u32,
    /// That blow's `HitInfo`, for the one bit that changes which reaction is
    /// played: `HITINFO_CRITICALHIT`. See `pose::reaction`.
    pub last_blow_info: u32,
    /// The number of `SMSG_AI_REACTION`s this unit has had, and the last one's
    /// reason. These are the aggro and alert barks, the one message in the
    /// protocol whose only purpose is a sound. See `crate::sound::combat`.
    pub reactions: u32,
    pub last_reaction: u32,
    /// One-shot emotes played, as a counter, and the `Emotes.dbc` id of the
    /// last one. The same shape as the swing, for the same reason.
    pub emotes: u32,
    pub last_emote: u32,
    /// `SpellVisualKit`s the server has told this client to play on this
    /// unit, as a counter and the last id: `SMSG_PLAY_SPELL_VISUAL` about what
    /// the unit is doing, `SMSG_PLAY_SPELL_IMPACT` about what happened to it.
    ///
    /// The packets name a kit rather than a spell, which is why
    /// `DisplayTables::kit_effects` exists: nothing else in this client reaches
    /// the visual tables without a spell id. See [`vale_protocol::play::sound`].
    pub spell_visuals: u32,
    pub last_spell_visual: u32,
    pub spell_impacts: u32,
    pub last_spell_impact: u32,
    /// `UNIT_FIELD_BYTES_1` byte 2: which `SpellShapeshiftForm.dbc` form this
    /// unit is in, 0 for none.
    ///
    /// Read for one purpose only, and it is not the model. The form's
    /// `bonusActionBar` column is `GetBonusBarOffset()`, which decides which
    /// twelve of the 120 action slots the bar shows. A warrior's stances are
    /// forms 17..19 and their bars are slots 73..108, so page one of a warrior's
    /// bar is empty. See `vale_assets::tables::spellbook::ShapeshiftForms`.
    pub shapeshift_form: u8,
    /// `UNIT_FIELD_AURASTATE`: the states a `Spell.dbc` row may require, as a
    /// mask. It greys out Judgement with no Seal active, Revenge with nothing
    /// blocked and Execute on a healthy target, without the client knowing what
    /// any of those three are. See
    /// [`vale_protocol::state::objects::Entity::aura_state`] and
    /// [`vale_assets::tables::spellbook::SpellInfo::castable_now`].
    ///
    /// Decoded for every unit, not only the local player, because a
    /// target-side condition reads the target's value.
    pub aura_state: u32,
    /// `PLAYER_FIELD_BYTES` byte 1: combo points on the target, which grey out
    /// every rogue and druid finisher. Zero for any unit that is not a player,
    /// because nothing else has combo points.
    pub combo_points: u8,
    /// `PLAYER_FIELD_BYTES` byte 2: which of the four extra action bars this
    /// character has switched on, as a mask of
    /// `vale_protocol::play::spells::multi_bar` bits.
    ///
    /// Zero for every unit but the local player. No filter here does that: the
    /// field is `PRIVATE`, so it never arrives for anyone else. Read for one
    /// purpose only: `GetActionBarToggles()`, which `UIParent.lua` calls once
    /// per `PLAYER_ENTERING_WORLD` and passes to `MultiActionBar_Update()`.
    pub action_bar_toggles: u8,
    /// `UNIT_NPC_EMOTESTATE`: an `Emotes.dbc` id the unit is holding, the
    /// other half of the emote system. A `/dance` is this and a `/wave` is the
    /// counter above; so is the innkeeper permanently at work.
    pub emote_state: u32,
    /// Casts begun and released, as two counters, with the cast bar's length
    /// beside the first. There are two because they come from two packets that
    /// do different things, and because an instant spell sends only the second,
    /// so a client that waited for both would animate nothing.
    pub casts_begun: u32,
    pub casts_released: u32,
    /// The last four spells released, oldest first; see
    /// `vale_protocol::state::objects::Entity::recent_spells`. Charge is two
    /// casts inside one poll, and this keeps the first one's art.
    pub recent_spells: [u32; vale_protocol::state::objects::RECENT_SPELLS],
    /// How many of those releases were the start of a channel. This lets a
    /// channel's wind-up outlive the release that arrived a microsecond before
    /// it. See `vale_protocol::state::objects::Entity::casts_channelled`.
    pub casts_channelled: u32,
    /// The releases the server stated. This is a third counter, not a copy of
    /// the second; see `vale_protocol::state::objects::Entity::casts_landed`.
    ///
    /// The caster's own art changes at the press ([`Self::casts_released`]).
    /// Only `SMSG_SPELL_GO` says what the cast hit, so the impact art is driven
    /// by this counter. Equal to `casts_released` for every unit but the local
    /// player.
    pub casts_landed: u32,
    /// The casts ended on this unit without completing: refused, interrupted,
    /// cancelled. Neither counter above can state this end of a cast. See
    /// `vale_protocol::state::objects::Entity::casts_cancelled`; `Playback`
    /// drops the held wind-up when it changes and `spell_effects` removes the
    /// art.
    pub casts_cancelled: u32,
    pub cast_time_ms: u32,
    /// The `Spell.dbc` id of that cast, which decides the pose: `SpellVisual`
    /// names a wind-up kit and a release kit, and they are why a fireball is
    /// thrown from the shoulder, a heal is raised overhead and a chest is
    /// opened with a crouch. See `vale_assets::tables::spell`.
    pub last_spell: u32,
    /// The guid the last released cast landed on, or 0. A missile flies at
    /// this; see `vale_protocol::state::objects::Entity::last_spell_target`.
    pub last_spell_target: u64,
    /// Every unit the last cast landed on, each of which gets impact art. An
    /// area spell names several here and one above: the burst is per victim and
    /// the projectile is one object. See
    /// `vale_protocol::state::objects::Entity::last_spell_targets`.
    ///
    /// A `Vec` copied per snapshot, the same trade-off the auras below make.
    /// It costs less, because it is empty for every unit that is not mid-cast,
    /// and an empty `Vec` does not allocate, either when built or when cloned.
    pub last_spell_targets: Vec<u64>,
    /// Pushback: how many times the cast in progress has been pushed back, and
    /// how far the last pushback moved it. This is `SMSG_SPELL_DELAYED`, the
    /// only packet that restates a cast's length after it has begun; see
    /// `vale_protocol::state::objects::Entity::casts_delayed`. The held wind-up
    /// and the art on the caster's hands each extend their own deadline from
    /// it.
    pub casts_delayed: u32,
    pub last_cast_delay_ms: u32,
    /// What is in this unit's hands: main hand, off hand, ranged. A creature's
    /// weapons come from its update fields and a player's from the item query;
    /// `ObjectManager` has already combined the two, so this field is the same
    /// for both.
    pub weapons: [vale_assets::tables::item::Weapon; 3],
    /// `UNIT_FIELD_BYTES_2` byte 0: 0 unarmed, 1 melee drawn, 2 ranged drawn.
    /// This decides whether a weapon hangs from a hand or from its sheath
    /// point, not whether it is drawn at all.
    ///
    /// This is what the server said, which for the local character is an echo
    /// of what this client sent. Nothing draws from it directly; see
    /// [`crate::world::entities::Sheath`], the client's committed state, which
    /// this seeds and which adopts a change in this value.
    pub sheath_state: u8,
    /// `UNIT_FIELD_MOUNTDISPLAYID` is non-zero. A mounted rider's weapons are
    /// stowed and cannot be drawn (see `vale_assets::look::sheath::reconcile`).
    ///
    /// Kept beside [`Self::mount_display_id`] rather than derived from it by
    /// each reader, because it answers a different question (whether this unit
    /// is riding, which is what the sheath rule asks) and because three of the
    /// four readers do not use what it is riding.
    pub mounted: bool,
    /// What the unit is riding, as a `CreatureDisplayInfo` id, for the one
    /// reader that uses it: `crate::world::entities::mount`, which draws it.
    pub mount_display_id: Option<u32>,
    /// The auras on this unit now: the occupied slots of `UNIT_FIELD_AURA`, in
    /// ascending order, as `ObjectManager` holds them.
    ///
    /// A `Vec` copied per snapshot rather than a diff, the same trade-off the
    /// wardrobe makes: 48 slots is a handful of words, the set changes rarely,
    /// and a diff would need the previous set kept somewhere anyway. The
    /// renderer compares it against what it built from; see
    /// `entities::spell_effects`.
    ///
    /// The whole slot rather than the spell id, because the interface needs
    /// three things the renderer does not: the slot index (the only thing that
    /// separates a buff from a debuff; see
    /// [`vale_protocol::state::objects::POSITIVE_AURA_SLOTS`]), the cancelable
    /// flag, and the stack count.
    pub auras: Vec<vale_protocol::state::objects::AuraSlot>,
    /// Everything the character sheet shows. `None` for every unit but the
    /// local player, because the server sends it for no one else: every field
    /// [`vale_protocol::play::stats::UnitStats`] reads is `PRIVATE` or
    /// `OWNER_ONLY`.
    ///
    /// Boxed because it is about 320 bytes that one entity in the world has,
    /// and this struct is rebuilt per entity per simulation step and compared
    /// field by field. A `Box` is one word for every other entity and one
    /// allocation per step for the player.
    pub stats: Option<Box<vale_protocol::play::stats::UnitStats>>,
    /// Where the character has been: `PLAYER_EXPLORED_ZONES`, the only server
    /// state the world map draws. `None` for every unit but the local player,
    /// like [`Self::stats`] and for the same reason: the field is `PRIVATE`.
    ///
    /// Boxed for the same reason: 256 bytes that one entity has, against one
    /// word for every other entity, in a struct rebuilt and compared per entity
    /// per step. See [`vale_protocol::play::explored`].
    pub explored: Option<Box<vale_protocol::play::explored::Explored>>,
    /// Every skill the character has: `PLAYER_SKILL_INFO_1_1`. `None` for
    /// every unit but the local player, because the field is `PRIVATE`, as for
    /// [`Self::explored`].
    ///
    /// Boxed for the same reason: about 50 entries that one entity has, against
    /// one word for every other entity, in a struct rebuilt and compared per
    /// entity per step. See [`vale_protocol::play::skills`].
    pub skills: Option<Box<vale_protocol::play::skills::Skills>>,
    /// Which faction's reputation bar sits over the action bar:
    /// `PLAYER_FIELD_WATCHED_FACTION_INDEX`, a reputation-list id or `-1`.
    ///
    /// `None` for every unit but the local player, as for [`Self::explored`]:
    /// the field is `PRIVATE`. Not boxed, because it is one word either way. It
    /// is here rather than in a resource because it is the server's only
    /// acknowledgement of `CMSG_SET_WATCHED_FACTION`. See
    /// [`crate::interface::reputation`].
    pub watched_faction: Option<i32>,
    /// `(spell id, radius in yards)` for a `DynamicObject`, and `None` for
    /// everything else: a Blizzard's ring, a Flamestrike's patch, a
    /// Consecration.
    ///
    /// It is the one kind of object whose appearance is not a display id. It
    /// has no `CreatureDisplayInfo` row and no `GameObjectDisplayInfo` row: it
    /// looks like its spell's art, and these two numbers are all the server
    /// says about it. See `vale_assets::tables::spell::SpellVisuals::ground_art`
    /// for how they become a model, and `entities::persistent_areas` for what
    /// draws it.
    pub area: Option<(u32, f32)>,
}

impl WorldEntity {
    /// `UNIT_FLAG_STUNNED`: the server has taken away control of this unit.
    /// This differs from `MOVEFLAG_ROOT`, which only forbids travel, and it is
    /// enforced in a different place. See
    /// [`vale_protocol::state::objects::Entity::is_stunned`], and
    /// `vale_protocol::state::movement::Restraint`, which holds both.
    ///
    /// Derived rather than stored as a field, because [`Self::unit_flags`] is
    /// already carried whole and this struct is rebuilt per entity per
    /// simulation step and compared field by field.
    pub(crate) fn stunned(&self) -> bool {
        self.unit_flags & vale_protocol::state::objects::UNIT_FLAG_STUNNED != 0
    }

    /// Stealth, Prowl, Shadowmeld: anything that set `UNIT_VIS_FLAGS_CREEP`.
    ///
    /// Derived from [`Self::vis_flags`] for the same reason [`Self::stunned`]
    /// is derived from `unit_flags`: the byte is carried whole and this struct
    /// is rebuilt and compared field by field every simulation step.
    ///
    /// It never checks a spell id. The rules that read this are the same for a
    /// rogue's Stealth, a druid's Prowl, a night elf's Shadowmeld and every
    /// creature the server hides, because all of them are
    /// `SPELL_AURA_MOD_STEALTH` and all of them set this one bit.
    pub(crate) fn creeping(&self) -> bool {
        self.vis_flags & vale_protocol::state::objects::UNIT_VIS_FLAGS_CREEP != 0
    }

    /// This body is not solid: `UNIT_VIS_FLAGS_GHOST | CREEP`. The 1.12.1
    /// client treats these two bits as one condition; the pairing is not a
    /// choice made here.
    ///
    /// Kept separate from [`Self::is_ghost`] because the two cover overlapping
    /// sets of units. `is_ghost` is `PLAYER_FLAGS_GHOST` and is only true of a
    /// player who has released; this bit is `SPELL_AURA_GHOST` on any unit. The
    /// two agree on a released player, and only this one applies to other
    /// units.
    pub(crate) fn not_solid(&self) -> bool {
        use vale_protocol::state::objects::{UNIT_VIS_FLAGS_CREEP, UNIT_VIS_FLAGS_GHOST};
        self.vis_flags & (UNIT_VIS_FLAGS_GHOST | UNIT_VIS_FLAGS_CREEP) != 0
    }
}

/// Marks the entity the session is driving.
#[derive(Component)]
pub(crate) struct LocalPlayer;

/// GUID -> the ECS entity representing it.
#[derive(Resource, Default)]
pub struct EntityIndex(pub HashMap<u64, Entity>);

/// How far the session thread's simulation has got, as this pass last saw it.
///
/// The renderer polls every frame and reconciles when the simulation has
/// advanced, rather than polling on a timer of its own. A timer is a second
/// free-running clock beside the session thread's, and two clocks at similar
/// rates beat against each other: some polls catch no new step and stall the
/// world, and the next catches two and jumps it forward. The effect is large (a
/// step is 0.19 yards at a run) and looks the same on screen as a rendering
/// fault. Keying on the simulation's own `world_ms` removes the second clock.
/// The cost is one mutex read of a `u64` per frame, which is why
/// `LiveSession::world_ms` exists separately from `status()`.
#[derive(Resource, Default)]
pub(super) struct WorldClock {
    /// `None` until the first reconcile, so the first step is not the whole
    /// elapsed life of the session thread.
    last_ms: Option<u64>,
}

/// The title and name of a unit's owner, for the line under the unit's name.
///
/// The owner is the charmer when there is one and the creator otherwise, and
/// it must be an object in the world: an owner out of sight gives no line.
/// A player has no owner line. The creating spell's first effect chooses
/// between a guardian, a created object and the pet-or-minion default; see
/// [`vale_assets::look::unitname::OwnerTitle::of`].
fn owner_of(
    world: &vale_protocol::state::objects::ObjectManager,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    unit: &vale_protocol::state::objects::Entity,
) -> Option<(vale_assets::look::unitname::OwnerTitle, String)> {
    if unit.object_type != Some(ObjectType::Unit) {
        return None;
    }
    let charmer = unit.charmed_by();
    let owner = world.get(charmer.or_else(|| unit.created_by())?)?;
    let effect = unit.created_by_spell().and_then(|spell| {
        Some(tables?.spellbook()?.info(spell)?.effects[0].kind)
    });
    let creature_type = world.creature_of(unit).map_or(0, |c| c.creature_type);
    Some((
        vale_assets::look::unitname::OwnerTitle::of(charmer.is_some(), effect, creature_type),
        world.unit_name_of(owner),
    ))
}

/// One thing in a unit's hand, from the protocol's shape into the assets'.
///
/// The rule needs two crates that do not depend on each other.
/// `vale-protocol` says what the server stated and `vale-assets` says what
/// that means for the drawing. Neither depends on the other, which keeps the
/// packet parsers testable without an archive and the file parsers testable
/// without a socket. So the numbers are converted here, as a player's
/// `(display id, inventory type)` pairs are further down.
///
/// Written as a struct literal rather than a positional constructor because
/// every field is a small integer and a transposition would compile, so the
/// names must be visible on both sides. `vale dress` has the same six lines,
/// and they only rename fields.
fn held(
    item: vale_protocol::play::items::HeldItem,
    enchantments: [u32; 7],
) -> vale_assets::tables::item::Weapon {
    vale_assets::tables::item::Weapon {
        display_id: item.display_id,
        class: item.class,
        subclass: item.subclass,
        inventory_type: item.inventory_type,
        sheath: item.sheath,
        material: item.material,
        enchantments,
    }
}

/// Read the world and reconcile it into entities.
pub(super) fn poll_world(
    time: Res<Time>,
    mut session: ResMut<Session>,
    // The one input to the snapshot that does not come from the server. It is
    // read here rather than in `interface/` because this is the only function
    // that builds a `WorldEntity`. See [`WorldEntity::looting`].
    loot: Res<crate::interface::loot::LootWindow>,
    // The one table that decides where an object is rather than what it looks
    // like: see [`crate::world::entities::transport`]. Read once per step,
    // outside the loop over entities.
    assets: Res<crate::assets::GameAssets>,
    mut clock: ResMut<WorldClock>,
    // The routes built from those tables, kept because a boat's position is a
    // spline over thirty waypoints and the game has nine boats. See
    // [`crate::world::entities::transport::ShipRoutes`].
    mut ship_routes: ResMut<crate::world::entities::transport::ShipRoutes>,
    mut index: ResMut<EntityIndex>,
    mut motion: ResMut<Motion>,
    mut status: ResMut<WorldStatus>,
    mut commands: Commands,
    mut existing: Query<&mut WorldEntity>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::PollWorld);
    let Some(active) = session.active.as_mut() else {
        return;
    };

    // The simulation has not advanced since the last frame, so there is
    // nothing to reconcile and no new interpolation leg to start. Re-tracking
    // an unchanged snapshot would restart every entity's interpolation towards
    // a target it is already approaching, which slows it to a crawl and then
    // jumps when a real step lands. See `WorldClock`.
    let world_ms = active.live.world_ms();
    if clock.last_ms == Some(world_ms) {
        return;
    }
    clock.last_ms = Some(world_ms);

    let session_status = active.live.status();
    // Read once, outside the per-entity loop: it is one bool about one
    // character and the loop runs over everything in view.
    //
    // Not `is_holding()`'s half-open state. A window whose rows are still
    // being named has not been drawn yet (see [`crate::interface::loot::hold`]),
    // but the character is reaching into the body either way. What matters
    // here is that there is a body being looted, which `get` reports.
    let looting = loot.get().is_some();

    // A far teleport has landed. The session thread has already emptied the
    // object manager of the map the character left, so the entities reconcile
    // below. The terrain does not: it streams from a directory name rather
    // than from an id, so it is re-pointed here.
    if session_status.map_id != active.map_id {
        let directory = active
            .terrain
            .directory(session_status.map_id)
            .map(str::to_string);
        match directory {
            Some(name) => {
                info!(
                    "far teleport: map {} ({}) -> map {} ({name})",
                    active.map_id, active.map_name, session_status.map_id,
                );
                active.map_name = name;
                active.map_id = session_status.map_id;
                // Every reading in the interpolator is about the map that has
                // been left. For all but one entity that does not matter: the
                // object manager emptied itself and `Motion::track` forgets
                // whatever is absent from the next reading. The exception is
                // the transport the character crossed on, which this client
                // keeps because `Map::SendInitTransports` never states it
                // again, and whose two readings are then an ocean apart. The
                // deck is stood on as well as drawn, so one frame of
                // interpolation between them carries the passenger into the
                // ocean. [`Motion`]'s own jump bound also catches this; the map
                // change is a certain signal and does not depend on the two
                // coordinates being far apart.
                motion.reset();
            }
            // There is no directory to stream, so keep the current terrain
            // rather than dropping the world into an empty grid.
            None => warn!(
                "teleported to map {}, which Map.dbc does not name — terrain unchanged",
                session_status.map_id
            ),
        }
    }

    let world = active
        .live
        .read();

    // The elevators' table, held for the loop below. `display_tables` parses
    // on the first call and returns an `Arc` afterwards, so this clones one
    // pointer per step. It is `None` only when the archives did not open, in
    // which case the client has no world.
    let transports = assets.display_tables().ok();

    let mut seen: Vec<(u64, Vec3, f32)> = Vec::new();
    let mut alive: HashMap<u64, ()> = HashMap::default();

    for e in world.iter() {
        let Some(p) = e.position else { continue };
        // The one call that tells a player from a creature, made once: both
        // the drawn weapons and the stat block's three skill lines are about
        // the same three items.
        let weapons = world.weapons_of(e);
        let snapshot = WorldEntity {
            guid: e.guid,
            kind: e.object_type.unwrap_or(ObjectType::Object),
            // The bare name, because this is what `UnitName` returns and what
            // the game's frames put on a name plate. `name_of`'s
            // "Merrick (Human Mage)" and "<Innkeeper>" forms are for a CLI
            // listing. Without the seeded character-list entry, the local
            // player read "Player 450" on the `PlayerFrame`.
            name: world.unit_name_of(e),
            // The three values the template carries, which the name above
            // leaves out: `unit_name_of` is the plate's first line and these
            // are its second.
            sub_name: world
                .creature_of(e)
                .map(|c| c.sub_name.clone())
                .unwrap_or_default(),
            creature_type: world.creature_of(e).map_or(0, |c| c.creature_type),
            pet_family: world.creature_of(e).map_or(0, |c| c.pet_family),
            classification: world.creature_of(e).map_or(0, |c| c.rank),
            entry: e.entry(),
            level: e.level(),
            health_value: e.health().zip(e.max_health()),
            power_value: e
                .power()
                .zip(e.max_power())
                .zip(e.power_type())
                .map(|((now, max), kind)| (now, max, kind)),
            display_id: e.display_id(),
            appearance: e.appearance().map(|[bytes_0, bytes, bytes_2]| {
                vale_assets::look::character::Appearance::from_fields(bytes_0, bytes, bytes_2)
            }),
            race_class: e.race_and_class(),
            gender: e.gender(),
            // Equipment, already passed through the one lookup the archives
            // cannot do: the field holds an item entry and its appearance is in
            // the server's template, so an item not yet queried is absent here
            // and arrives a query interval later.
            equipment: e
                .equipment()
                .map(|entries| {
                    entries
                        .into_iter()
                        .filter(|entry| *entry != 0)
                        .filter_map(|entry| world.items.get(&entry))
                        .filter(|info| info.display_id != 0)
                        // The hide-helm and hide-cloak options. A toggle
                        // changes this list, so the entity's model is rebuilt
                        // without the piece, as any change of gear is.
                        .filter(|info| {
                            vale_assets::look::dress::worn_is_shown(info.inventory_type, e.player_flags())
                        })
                        .map(|info| (info.display_id, info.inventory_type))
                        // The guild's emblem, as one more pair after the
                        // items. A guild tabard is painted with it; see
                        // [`vale_assets::look::emblem`]. Absent for a player
                        // in no guild, for a guild with no emblem, and until
                        // the guild's query answers.
                        .chain(world.guild_of(e).and_then(|guild| {
                            vale_assets::look::emblem::entry(guild.emblem.fields(), false)
                        }))
                        .collect()
                })
                .unwrap_or_default(),
            scale: e.scale(),
            combat_reach: e.combat_reach().unwrap_or(DEFAULT_COMBAT_REACH),
            bounding_radius: e.bounding_radius().unwrap_or(DEFAULT_BOUNDING_RADIUS),
            // The mover is the one entity the object manager does not
            // dead-reckon (the session's `Mover` owns it), so its movement
            // state comes from the status rather than from the world.
            //
            // Every choice below is keyed on the mover, not on `is_self`. The
            // two differ while a possessed unit can be driven: the status then
            // describes the possessed unit, so a character keyed on `is_self`
            // read that unit's `moving: true` at its speed and played Run where
            // it stood (the report was "the player model runs in place during
            // possession"). The character is an ordinary server-stated unit for
            // the duration, and the object manager's values are the correct
            // ones for it.
            moving: if e.guid == session_status.mover {
                session_status.moving
            } else {
                e.is_moving()
            },
            speed: if e.guid == session_status.mover {
                session_status.speed
            } else {
                e.ground_speed()
            },
            // Not split on the mover, unlike the four fields around it. Those
            // come from the mover because the player's movement never comes
            // back from the server. The speeds do come back, in the player's
            // create block and in every `SMSG_FORCE_*_CHANGE` after it, so the
            // entity is the one source for every unit.
            turn_rate: e.speeds.unwrap_or_default().turn_rate(),
            // The test is whether this is the mover, not whether it is the
            // local player. The two are the same guid in every session that
            // never possesses anything. During a possession the session's
            // movement block belongs to the possessed body, so keying on
            // `is_self` would give the character the possessed unit's flags and
            // the possessed unit the server's stale ones.
            move_flags: if e.guid == session_status.mover {
                session_status.movement.flags
            } else {
                e.move_flags()
            },
            // Split for the same reason `move_flags` is: the mover owns the
            // player's transport relationship and the object manager owns
            // everybody else's. See [`WorldEntity::platform`].
            platform: if e.guid == session_status.mover {
                session_status.ferry.map(|ferry| ferry.guid)
            } else {
                e.platform_guid()
            },
            airborne: if e.guid == session_status.mover {
                session_status.airborne
            } else {
                e.is_airborne()
            },
            jumping: if e.guid == session_status.mover {
                session_status.jumping
            } else {
                e.is_jumping()
            },
            is_self: e.is_self,
            // Only the local player's, because it is only asked about the
            // caster, and reading it for every unit in a city would cost a
            // field pair per entity per poll for an answer nothing uses.
            // The local player's alone, for the same reason as the main hand:
            // `PLAYER_FARSIGHT` is not in a field group sent about any other
            // player, so reading it for every unit in view costs a field pair
            // per entity per poll for an answer that is always empty.
            farsight: e.is_self.then(|| e.farsight()).flatten(),
            main_hand_item: e.is_self.then(|| {
                vale_protocol::state::objects::items::equipped_guid(
                    e,
                    vale_protocol::play::items::EQUIPMENT_SLOT_MAINHAND,
                )
            }).flatten(),
            dead: e.is_dead().unwrap_or(false),
            feigning: e.is_feigning(),
            is_ghost: e.is_ghost(),
            timed_release: e.player_field_flags() & vale_protocol::play::death::RELEASE_TIMER != 0,
            experience: e.experience(),
            character_points: e.character_points(),
            rested: e.rested_experience(),
            in_combat: e.in_combat(),
            // Both conditions must hold; see
            // [`vale_protocol::state::objects::Entity::engaged`], which
            // documents the two readings behind them.
            attacking: e.engaged(),
            faction: e.faction(),
            unit_flags: e.unit_flags(),
            player_flags: e.player_flags(),
            guild_id: e.guild_id(),
            guild: world
                .guild_of(e)
                .map(|guild| guild.name.clone())
                .unwrap_or_default(),
            pvp_rank: e.pvp_rank_and_medal().0,
            pvp_medal: e.pvp_rank_and_medal().1,
            owner: owner_of(&world, transports.as_deref(), e),
            duel_arbiter: e.duel_arbiter(),
            duel_team: e.duel_team(),
            npc_flags: e.npc_flags(),
            lootable: e.lootable(),
            target: e.target_guid().filter(|guid| *guid != 0),
            pet: e.pet_guid(),
            summoned_by: e.summoned_by(),
            charmed_by: e.charmed_by(),
            hunters_marked: e.hunters_marked(),
            quest_turn_in: world.quest_status(e.guid)
                == Some(vale_protocol::play::quest::DialogStatus::Reward),
            tracking: e.tracking(),
            pet_number: e.pet_number(),
            pet_stats: e.pet_stats(),
            stand_state: e.stand_state().unwrap_or(0),
            vis_flags: e.vis_flags(),
            // The local player's alone, and from a resource rather than from
            // the wire; see the field, the only one here that no packet states.
            looting: e.is_self && looting,
            object_state: e.game_object_state(),
            // The template's two words, resolved once here; see the two
            // fields. Both are zero until `CMSG_GAMEOBJECT_QUERY` returns, so a
            // door that has just streamed in cannot be clicked for a frame or
            // two, which is better than being wrong.
            object_kind: world.gameobject_of(e).map_or(0, |info| info.object_type),
            object_lock: world
                .gameobject_of(e)
                .and_then(|info| {
                    let kind = vale_assets::look::object::Kind::of(info.object_type);
                    Some(*info.data.get(kind.lock_word()?)?)
                })
                .unwrap_or(0),
            // Live fields rather than template words; see the three fields.
            // All three are read from the entity rather than from
            // `gameobject_of`, because they change during a session and the
            // template does not.
            object_flags: e.game_object_flags().unwrap_or(0),
            object_dyn_flags: e.game_object_dyn_flags().unwrap_or(0),
            object_level: e.game_object_level().unwrap_or(0),
            object_page: world
                .gameobject_of(e)
                .and_then(|info| {
                    let kind = vale_assets::look::object::Kind::of(info.object_type);
                    let (page, material) = kind.page_words()?;
                    Some((
                        info.data.get(page).copied().unwrap_or(0),
                        info.data.get(material).copied().unwrap_or(0),
                    ))
                })
                .unwrap_or((0, 0)),
            object_anims: e.object_anims,
            object_anim: e.last_object_anim,
            object_hover: world
                .gameobject_of(e)
                .map(|info| {
                    vale_assets::look::object::hover_of(
                        vale_assets::look::object::Kind::of(info.object_type),
                        &info.data,
                    )
                })
                .unwrap_or_default(),
            // The mover's value comes from the mover, like the flags above and
            // for the same reason. `is_swimming` reads the object manager,
            // which for the local player holds whatever the server last said,
            // and the server never sends the local player's movement block back
            // to it. Without this split, the one character whose swimming this
            // client decides itself would be drawn walking through the water.
            swimming: if e.guid == session_status.mover {
                session_status.movement.has(vale_protocol::state::movement::move_flags::SWIMMING)
            } else {
                e.is_swimming()
            },
            // The pitch that goes with swimming, split the same way for the
            // same reason: the mover's comes from the mover, and every other
            // unit's comes from the block it broadcast.
            pitch: if e.guid == session_status.mover {
                if session_status
                    .movement
                    .has(vale_protocol::state::movement::move_flags::SWIMMING)
                {
                    session_status.movement.pitch
                } else {
                    0.0
                }
            } else {
                e.pitch()
            },
            swings_thrown: e.swings_thrown,
            last_swing_info: e.last_swing_info,
            last_swing_state: e.last_swing_state,
            last_swing_victim: e.last_swing_victim,
            last_swing_damage: e.last_swing_damage,
            damage_taken: e.damage_taken,
            last_damage: e.last_damage,
            last_damage_info: e.last_damage_info,
            last_damage_state: e.last_damage_state,
            last_damage_spell: e.last_damage_spell,
            healed: e.healed,
            blows_taken: e.blows_taken,
            last_victim_state: e.last_victim_state,
            last_blow_info: e.last_blow_info,
            reactions: e.reactions,
            last_reaction: e.last_reaction,
            emotes: e.emotes,
            last_emote: e.last_emote,
            spell_visuals: e.spell_visuals,
            last_spell_visual: e.last_spell_visual,
            spell_impacts: e.spell_impacts,
            last_spell_impact: e.last_spell_impact,
            emote_state: e.emote_state(),
            shapeshift_form: e.shapeshift_form(),
            aura_state: e.aura_state(),
            combo_points: e.combo_points(),
            action_bar_toggles: e.action_bar_toggles(),
            casts_begun: e.casts_begun,
            casts_released: e.casts_released,
            recent_spells: e.recent_spells,
            casts_channelled: e.casts_channelled,
            casts_landed: e.casts_landed,
            casts_cancelled: e.casts_cancelled,
            cast_time_ms: e.cast_time_ms,
            last_spell: e.last_spell,
            last_spell_target: e.last_spell_target,
            last_spell_targets: e.last_spell_targets.clone(),
            casts_delayed: e.casts_delayed,
            last_cast_delay_ms: e.last_cast_delay_ms,
            // The one call that tells a player from a creature. A creature's
            // weapons are in its update fields, and a player's are three item
            // entries whose templates arrive a round trip later; everything
            // after this point sees one form.
            weapons: {
                let enchantments = e.weapon_enchantments();
                std::array::from_fn(|hand| held(weapons[hand], enchantments[hand]))
            },
            // The three player-only blocks are all gated on `is_self`, for the
            // same reason as `main_hand_item` above, and the gate uses the same
            // source of identity. `UNIT_FIELD_STAT0`, `PLAYER_SKILL_INFO_1_1`
            // and `PLAYER_EXPLORED_ZONES_1` are all `PRIVATE`/`OWNER_ONLY` on
            // the wire, so the server never sends them for a unit other than
            // the local player. Without the gate every one of these `read`s
            // returned `None` for every other entity, after paying for the scan
            // that found nothing: `Explored::read` probes all 64 zone words and
            // `Skills::read` all 128 skill slots. That was 64 hash lookups on
            // every creature and player in view, plus 128 more on every remote
            // player, every poll, to rebuild a `None` the `is_self` flag already
            // gave. The flag is the create block's `UPDATEFLAG_SELF`, the same
            // authority the server uses to decide whether to send the fields,
            // so the gate cannot change any result. See `WorldEntity` for the
            // cost this had with many NPCs in one place.
            // The stat block is decoded here rather than on demand because this
            // is the only place that has both the fields and the wardrobe: the
            // skill line the sheet's "Melee Attack" reads is the weapon's.
            // A pet is the one other unit that gets the probe. Its
            // `UNIT_FIELD_STAT0..` block is `OWNER_ONLY` on the wire, so the
            // server sends it for the local player's pet, and
            // `PetPaperDollFrame` reads it through the same eleven functions
            // the character sheet uses. The probe limits itself:
            // `UnitStats::decode` returns `None` without `STAT0`, which is the
            // case for every pet that belongs to someone else. The two
            // skill-derived values (`UnitDefense`, `UnitAttackBothHands`) read
            // `PLAYER_SKILL_INFO`, which a pet does not have, so they are zero
            // on the pet tab.
            stats: (e.is_self || e.pet_number() != 0)
                .then(|| vale_protocol::play::stats::UnitStats::read(e, &weapons).map(Box::new))
                .flatten(),
            explored: e
                .is_self
                .then(|| vale_protocol::play::explored::Explored::read(e).map(Box::new))
                .flatten(),
            watched_faction: vale_protocol::state::objects::reputation::watched_faction(e),
            skills: e
                .is_self
                .then(|| vale_protocol::play::skills::Skills::read(e).map(Box::new))
                .flatten(),
            sheath_state: e.sheath_state(),
            mounted: e.mounted(),
            mount_display_id: e.mount_display_id(),
            auras: e.aura_slots(),
            area: e.persistent_area(),
        };
        // A moving platform is drawn where its cycle puts it, not where it was
        // spawned. This is the one place to apply that, because both the model
        // and its collision hull read their position from `Motion`. See
        // [`crate::world::entities::transport`] for the full reasoning. `None`
        // for everything that is not a moving platform, which is all but a few
        // dozen objects in the game.
        //
        // The offset is added here rather than written back into the tracked
        // position, which must stay the stationary point the server stated:
        // written there, it would be added again on the next step.
        let travelled = transports
            .as_deref()
            .and_then(|tables| {
                crate::world::entities::transport::offset_of(
                    e,
                    p.orientation,
                    tables.transports(),
                )
            })
            .unwrap_or(Vec3::ZERO);
        // A continent transport has a placement of its own rather than an
        // offset. A boat's position on the wire is zero (vmangos'
        // `GetStationaryX` returns `0.f` for `GAMEOBJECT_TYPE_MO_TRANSPORT`), so
        // there is nothing to add an offset to, and the route says which of two
        // maps the boat is on as well as where. See
        // `world::entities::transport::ship_placement`.
        let sailing = transports.as_deref().and_then(|tables| {
            let taxi = tables.taxi()?;
            let route = ship_routes.of(world.gameobject_of(e)?, taxi)?;
            crate::world::entities::transport::ship_placement(e, &route)
        });
        if let Some(at) = sailing {
            // The boat is on the other continent, so it is not drawn here. The
            // server says so too, a moment later, with an out-of-range block,
            // but a client that waited for it would draw a Kalimdor boat off
            // Menethil for as long as the two clocks disagreed.
            //
            // The exception is the boat the character stands on. It is kept
            // alive and stops being tracked. `Motion` forgets it, so
            // `place_entities` leaves its `Transform` at the last placement on
            // this map and `entities::solid` drops its hull, which hands the
            // passenger to the session thread's platform hold. The deck
            // therefore stays under the character's feet until
            // `SMSG_NEW_WORLD` arrives, rather than vanishing for the round
            // trip `Transport::TeleportTransport` takes to send it.
            if at.map != active.map_id {
                if session_status.ferry.map(|ferry| ferry.guid) != Some(e.guid) {
                    continue;
                }
                alive.insert(e.guid, ());
                continue;
            }
            seen.push((e.guid, Vec3::from_array(at.pos), at.facing));
        } else {
            // A ship without a schedule is not drawn where its create block
            // said, because that is the map origin. vmangos' `GetStationaryX`
            // returns `0.f` for `GAMEOBJECT_TYPE_MO_TRANSPORT`, so the tracked
            // position of any boat is `(0, 0, 0)`. The fallback below would do
            // more than draw it in the wrong place: `entities::solid` builds the
            // placement `Standing::platform_of` answers from this reading, so a
            // passenger would be carried to the origin and the offset stored
            // there would be sent on the wire as the passenger's position. The
            // ship is skipped until the query answers and the route resolves,
            // and kept alive meanwhile if it is the deck being ridden, as in the
            // far-continent branch above.
            let unresolved_ship = e.transport_phase_ms.is_some()
                && match world.gameobject_of(e) {
                    Some(info) => {
                        info.object_type
                            == vale_assets::tables::shiptransport::MoTransport::TYPE
                    }
                    // Not yet queried: a ship is the only object at the exact
                    // origin; no authored placement is there.
                    None => p.x == 0.0 && p.y == 0.0 && p.z == 0.0,
                };
            if unresolved_ship {
                if session_status.ferry.map(|ferry| ferry.guid) == Some(e.guid) {
                    alive.insert(e.guid, ());
                }
                continue;
            }
            seen.push((
                e.guid,
                Vec3::new(p.x, p.y, p.z) + travelled,
                p.orientation,
            ));
        }
        alive.insert(e.guid, ());

        match index.0.get(&e.guid) {
            Some(&entity) => {
                if let Ok(mut component) = existing.get_mut(entity) {
                    // `set_if_neq` rather than a plain write: a `DerefMut` on a
                    // Bevy component marks it changed whether or not the value
                    // differs, and most entities in a busy zone are identical
                    // from one step to the next. See `WorldEntity`.
                    component.set_if_neq(snapshot);
                }
            }
            None => {
                // `Visibility` on the root, which draws nothing itself: its
                // model's batches are children and each carries its own, and a
                // child with inherited visibility whose parent has none triggers
                // Bevy's warning B0004.
                let mut spawned = commands.spawn((
                    snapshot.clone(),
                    Transform::default(),
                    Visibility::default(),
                    // Seeded from the wire and owned by the client from here
                    // on. A creature arrives with its weapons already drawn
                    // (`Creature::Create` sets melee server-side) and a player
                    // arrives with them stowed, and both are correct; later
                    // changes are handled by `entities::sheath`.
                    crate::world::entities::Sheath::seeded(snapshot.sheath_state),
                ));
                if snapshot.is_self {
                    spawned.insert(LocalPlayer);
                }
                index.0.insert(e.guid, spawned.id());
            }
        }
    }

    // Anything the server stopped describing is gone. `SMSG_DESTROY_OBJECT`
    // and out-of-range blocks both appear here as an absence.
    index.0.retain(|guid, entity| {
        if alive.contains_key(guid) {
            true
        } else {
            commands.entity(*entity).despawn();
            false
        }
    });

    let now = time.elapsed_secs();
    status.entity_count = seen.len();
    // Stamped with the simulation's own clock, not with `now`: see `motion`.
    motion.track(now, world_ms, seen.into_iter());

    let pos = session_status.position;
    let (tile_x, tile_y) = tile_for_position(pos.x, pos.y);
    status.character = session_status.character.clone();
    status.map_name = active.map_name.clone();
    status.map_id = active.map_id;
    status.tile = (tile_x, tile_y);
    status.position = Vec3::new(pos.x, pos.y, pos.z);
    status.orientation = pos.orientation;
    status.in_world = session_status.in_world;
    status.mover = session_status.mover;
    status.on_taxi = session_status.restraint.on_taxi;
    status.uncontrolled = session_status.in_world
        && (session_status.mover == 0 || (session_status.ride.is_some() && !status.on_taxi));
    status.weather = session_status.weather;
    status.packets = session_status.packets;
    status.movement_sent = session_status.movement_sent;
    status.flag_changes = session_status.flag_changes;
    status.latency_ms = session_status.latency_ms;
    status.stalls = session_status.stalls;
    status.worst_stall_ms = session_status.worst_stall_ms;
    status.platform_jumps = session_status.platform_jumps;
    status.worst_platform_jump = session_status.worst_platform_jump;
    status.read_slice_overruns = session_status.read_slice_overruns;
    status.ticks = session_status.ticks;
    // An `Arc` swap, not a map copy; see the field's note, and
    // `SessionStatus::traffic`, which is rebuilt on the session thread's own
    // interval so that this line costs almost nothing.
    status.traffic = session_status.traffic.clone();
    status.capture = session_status.capture.clone();
    status.counters = Counters {
        speed_changes: session_status.speed_changes,
        speed_broadcasts: session_status.speed_broadcasts,
        spline_flag_changes: session_status.spline_flag_changes,
        pushed_sounds: session_status.pushed_sounds,
        pushed_visuals: session_status.pushed_visuals,
        bagged_moves: session_status.bagged_moves,
        bagged_packets: session_status.bagged_packets,
        area_triggers: session_status.area_triggers,
        rides: session_status.rides,
        attacks: session_status.attacks,
        emotes: session_status.emotes,
        casts: session_status.casts,
        ai_reactions: session_status.ai_reactions,
        cast_results: session_status.cast_results,
        attack_refusals: session_status.attack_refusals,
        spells_known: session_status.spells_known,
        action_buttons: session_status.action_buttons,
        flag_changes: session_status.flag_changes,
    };

    // A refused position means a packet was read at the wrong offset, and the
    // only other symptom is an entity that vanishes without a message, so the
    // warning goes on the HUD rather than only into the log.
    status.warnings = session_status.warnings.clone();
    if world.rejected_positions > 0 {
        status.warnings.insert(
            0,
            format!(
                "{} impossible positions refused — a packet is being read at the wrong offset",
                world.rejected_positions
            ),
        );
    }
}

/// Write every entity's interpolated position onto its `Transform`.
///
/// The one place entity positions are converted from WoW's axes into Bevy's.
/// Public so the entity pass can order itself after it: a joint is written as a
/// world transform, so it has to be composed against this frame's placement.
///
/// This writes the `Transform`, and until `PostUpdate` that is the only place
/// this frame's placement exists. An entity is spawned at the root of the
/// hierarchy, so its `GlobalTransform` equals its `Transform`, but only one
/// propagation later. Anything in `Update` that needs an entity's current
/// position must read this component. Reading the `GlobalTransform` gets last
/// frame's, which is a whole frame of travel; that separated a running
/// character's helm from their head (see `entities::animate`).
pub fn place_entities(
    time: Res<Time>,
    motion: Res<Motion>,
    predicted: Res<crate::world::predict::Predicted>,
    facing: Res<crate::world::facing::BodyFacing>,
    // Which entity the prediction is about; see [`WorldStatus::mover`].
    status: Res<WorldStatus>,
    mut entities: Query<(
        &WorldEntity,
        Option<&crate::world::entities::EntityModel>,
        &mut Transform,
    )>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Place);
    let now = time.elapsed_secs();
    // The one entity drawn ahead of the simulation rather than behind it,
    // because the keyboard controls it and this client, not the server,
    // decides its position. `None` for every other entity, and for the player
    // while airborne. See `crate::world::predict`.
    //
    // It is the mover, not `is_self`, and that is what makes a possessed body
    // drivable. While Eye of Kilrogg or Mind Control lasts, the session's
    // simulation is the possessed unit, so that is the entity whose position
    // this client decides. The character falls back to the ordinary
    // interpolation, which leaves it standing where the possession began. The
    // two are the same guid in every other session.
    let ahead = predicted.of();
    let mover = status.mover;
    for (entity, model, mut transform) in &mut entities {
        let (position, aim) = match ahead.filter(|_| entity.guid == mover) {
            Some(p) => (Vec3::new(p.x, p.y, p.z), p.orientation),
            None => {
                let Some(position) = motion.position_of(entity.guid, now) else {
                    continue;
                };
                (position, motion.facing_of(entity.guid, now).unwrap_or(0.0))
            }
        };
        // Drawn at the body's heading, not the aim. They are the same for
        // everything facing where it is going. A strafe is the case where they
        // differ, and 1.12 draws a strafe only by turning the model. See
        // `crate::world::facing`.
        let facing = facing.of(entity.guid).map_or(aim, |body| body.yaw);
        // `OBJECT_FIELD_SCALE_X` already equals `modelScale * displayScale`
        // (`Unit::GetScaleForDisplayId`), so the DBC product is a fallback for
        // an entity whose field never arrived and must not be multiplied in.
        let scale = entity
            .scale
            .filter(|s| *s > 0.01)
            .or_else(|| model.map(|m| m.dbc_scale))
            .unwrap_or(1.0);
        // The body is also pitched, which applies only to a swimmer. A diving
        // character points along its direction of travel; without this it was
        // drawn upright doing a breaststroke while sinking, which is what the
        // report described. The pitch is a body rotation about the model's own
        // right axis; see [`crate::render::axes::body`].
        //
        // The lean for walking on a slope is not applied here, contrary to what
        // that function's documentation anticipated. It is a whole basis rather
        // than one angle, it depends on the model's `GlobalModelFlags` rather
        // than on a move flag, and it belongs to the model matrix rather than to
        // the entity. So the blob shadow, the selection ring and the pick box,
        // which read this transform, stay upright, as they do in the 1.12.1
        // client. See `crate::world::entities::conform`.
        let placed = Transform {
            translation: crate::render::axes::to_bevy(position.to_array()),
            rotation: crate::render::axes::body(facing, entity.pitch),
            scale: Vec3::splat(scale),
        };
        // Written only when it changes, the same rule `particles::simulate`
        // applies to an emitter's anchor, for the same reason: a
        // `Mut<Transform>` marks the component changed whether or not the value
        // differs, and a changed `Transform` costs transform propagation, a
        // `GlobalTransform` write, and (through `Changed<GlobalTransform>`) a
        // re-extraction of every mesh under the entity into the render world.
        // A standing guard's interpolated position is bit-identical frame after
        // frame (`interpolate` returns the nearest reading outside the
        // history), so a city's whole idle population paid that cost every
        // frame for a value that had not changed. The comparison is exact, with
        // no epsilon: an entity that moved a millimetre must still be
        // re-extracted, and only a write that changes nothing is skipped.
        if *transform != placed {
            *transform = placed;
        }
    }
}
