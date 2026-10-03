//! The entity pass: a model for every unit, player and game object the server
//! describes, skinned and animated from the model's own bones.
//!
//! ```text
//! mod.rs        the pass: which entities have models, what they are drawn as, and in what order
//! spawn.rs      building one WorldEntity into an entity and its children
//! pose.rs       posing it: which clip, how far into it, and the joints
//! worn.rs       what it wears: the geosets, the composed skin, the gear
//! sheath.rs     whether its weapons are drawn
//! mount.rs      what it rides: a second model on the same motion
//! conform.rs    tilting both to the ground, for models whose header asks for it
//! solid.rs      the collision hulls of game objects
//! transport.rs  where a moving transport (an elevator) is this frame
//! effects.rs    the spell art and aura art on it
//! tint.rs       the colour and opacity that art gives its own batches
//! procedural.rs what a kit leaves on it for later: a timed colour and a
//!               weapon trail waiting for the next swing
//! fallback.rs   what to draw when the tables resolve to no model
//! ```
//!
//! A `WorldEntity` carries a `DISPLAYID`. The display tables turn that into a
//! model path and skins, and `models.rs` turns those into meshes and materials.
//! These are the same meshes and materials the doodads use, because a creature
//! and a tree are both M2s. This module adds the skeleton.
//!
//! ## Why `M2Skeleton::pose` composes the pose and Bevy only skins
//!
//! Bevy's animation system is not used. The only engine feature used is
//! `SkinnedMesh`, which blends four joint matrices per vertex on the GPU and
//! packs every skinned mesh's joints into shared storage buffers. That removes
//! the WebGL renderer's 38-bone limit: a wolf's 64 bones did not fit in 128
//! vertex uniform vectors, and `Models.initSkinning` fell back to no animation.
//!
//! Everything else stays here, because three properties of a vanilla M2 cannot
//! be expressed in Bevy's hierarchy:
//!
//! * The inverse bindposes are all identity. M2 vertices are already in model
//!   space, and `M2Skeleton::pose` restores each bone's pivot itself
//!   (`local = T(pivot + translation) * R * S * T(-pivot)`). An inverse
//!   bindpose would apply the pivot twice.
//! * The joints are flat, not parented to each other. `pose` resolves the
//!   hierarchy itself, processing a bone once its parent is done rather than
//!   by index, because the format does not guarantee parents come first. It
//!   then does two things transform propagation cannot: spherical billboards
//!   (bone flag 0x8 replaces the bone's rotation with the camera's axes,
//!   expressed in model space) and global-sequence tracks, which ignore the
//!   playing animation and loop on wall-clock time. 357 of the game's 411
//!   creature models drive at least one track that way.
//! * A joint is written as a `GlobalTransform`, not a `Transform`. A composed
//!   bone matrix need not decompose into translation, rotation and scale: a
//!   bone with non-uniform scale and a rotated child shears.
//!   `Transform::from_matrix` would drop the shear and pose the model subtly
//!   wrong. Writing the affine also skips propagating about five thousand
//!   joint entities a frame. The joints are still children of the entity, so
//!   despawning it despawns them. They have no `Transform`, so propagation
//!   skips them; a test checks that Bevy leaves them alone.
//!
//! ## Which animation plays, and when
//!
//! The server never sends an animation id. It says whether an entity is
//! moving, how fast, and under which movement flags: `Entity::is_moving`,
//! `Entity::ground_speed` and `Entity::move_flags`, the same flags or spline
//! the client's dead reckoning advances the entity by. The client picks a gait
//! from the three. The available gaits depend on the art and are not
//! symmetric: on the ground there are Walk, Run and `Walkbackwards` and no
//! sideways gait; in water there are all four. A strafe on land is drawn by
//! turning the body, not by playing a different clip; see
//! [`crate::world::facing`].
//!
//! Movement is read from the stated flags and speed, not from the difference
//! between two interpolated positions. That difference reads zero for the last
//! frames of most snapshot intervals and switched Run to Stand twenty times a
//! second. [`Playback::advance`] implements two further rules: a restart is
//! triggered by a change of sequence, not of intent, and a switch cross-fades
//! over 150 ms.

use crate::assets::GameAssets;
use crate::axes;
use crate::render::models::{Lookup, Materials, ModelAssets, ModelCache, RoomLight, SceneLighting};
use crate::world::session::WorldEntity;
use vale_assets::tables::dbc::DisplayModel;
use vale_assets::look::dress::CharacterLook;
use vale_assets::tables::item::{AttachedModel, Weapon, WeaponAnim};
use vale_assets::world::m2::{
    anim, attach, key_bone, Blend, BodyTwist, M2Attachment, M2Sequence, M2Skeleton, Overlay,
    PoseLayers,
};
use vale_assets::tables::spell::CastAnimation;
use vale_protocol::play::action::victim_state;
use vale_protocol::state::movement::move_flags;
use vale_protocol::state::update::ObjectType;
use bevy::math::Affine3A;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use std::sync::Arc;

/// How long a change of animation takes to cross-fade, in seconds.
///
/// The 1.12.1 client blends over about this long. `M2Sequence::flags` carries
/// a per-sequence hint that vanilla models rarely set, so one value is used
/// for every sequence.
const FADE_SECS: f32 = 0.150;

/// The slowest an entity can be going and still be walking rather than running.
///
/// Vanilla's baseline walk is 2.5 yards a second and its run 7.0, so the exact
/// threshold matters little. It is used for creatures, which the server moves
/// along a patrol spline at walking pace and into a fight at running pace.
const RUN_SPEED: f32 = 4.5;

/// Below this, "moving" is the server turning a creature on the spot.
///
/// A turn on the spot arrives as a spline whose destination the creature is
/// already standing on, at a few hundredths of a yard a second. The value
/// cannot flicker, because a spline's speed is fixed for its whole duration.
const MOVING_FLOOR: f32 = 0.1;

/// How many entities to give models to in one frame. A login burst is ~200 at
/// once and each costs a mesh walk per batch.
const SPAWN_BUDGET: usize = 16;

/// The model an entity has been given, and the parts it was drawn as.
///
/// Requires a [`conform::Stance`] rather than having one inserted beside it.
/// Every modelled entity has a ground stance whether or not its own model
/// leans: a rider whose model is level may mount a leaning horse a second
/// later. A required component ensures the tests cannot omit it.
#[derive(Component)]
#[require(conform::Stance, pose::Posed, procedural::Procedurals)]
pub struct EntityModel {
    /// The display id this model was built for. A `DISPLAYID` can change on an
    /// entity (a shapeshift, a mount), and that requires a rebuild rather than
    /// a redress.
    pub display_id: u32,
    /// `modelScale * displayScale` from the DBCs, used when the entity's own
    /// `OBJECT_FIELD_SCALE_X` never arrived. The field and this value are not
    /// multiplied together: `Unit::GetScaleForDisplayId` returns the product,
    /// and the field holds that product.
    pub dbc_scale: f32,
    /// The joint entities, in bone order, with the identity joint last.
    pub joints: Vec<Entity>,
    /// The appearance and wardrobe this was dressed from, for a player.
    ///
    /// Recorded because of equipment. A player's gear is a server query answer
    /// and arrives a query interval after the entity does, so a model built
    /// when the entity appears is built without gear. Without a record to
    /// compare against, it would stay without gear for the rest of the session.
    look: Option<CharacterLook>,
    /// The models the wearer's equipment attaches to it, still to be spawned.
    ///
    /// Drained as each model's M2 arrives. An attached model loads after the
    /// character's own, so a fully geared player appears first and the
    /// pauldrons appear a moment later.
    wanted: Vec<AttachedModel>,
    /// The attached models already placed on their bones.
    attached: Vec<AttachedPart>,
    /// The spell effect models this unit is wearing, split into sets by
    /// lifetime.
    ///
    /// Held apart from `attached` even though they ride the same bones and are
    /// placed by the same code, because their lifetime differs. A pauldron
    /// belongs to the model and is rebuilt with it. A spell effect has to be
    /// removed again, and each set is removed for a different reason. See
    /// [`spell_effects`].
    ///
    /// `cast` holds what this unit is doing: the wind-up held while the cast
    /// bar runs, or the release. It ends on a clock.
    cast: EffectSet,
    /// What was done to the unit: the `impactKit`, hung when a spell lands. A
    /// separate set because a unit can be casting and be hit at the same time,
    /// and with one list the hit would cancel the caster's own glow.
    impact: EffectSet,
    /// What is true of the unit: the `stateKit`s of the auras it carries. It
    /// ends when the aura ends, which is a condition, not a clock.
    state: EffectSet,
    /// An event on the unit that no spell describes. Currently this is only the
    /// level-up glow. It is a separate set because its lifetime matches none
    /// of the three above: it is not a cast, nothing did it to the unit, and
    /// it is not a state of the unit. In `cast`, the next spell cast would
    /// cancel it. See [`effects::level_up`].
    milestone: EffectSet,
    /// What is available on the unit: the sparkle over a body that still has
    /// loot. A separate set because its lifetime matches none of the four
    /// above: it is not a cast, nothing did it to the unit, no aura states it,
    /// and unlike `milestone` it ends on a condition rather than a clock
    /// (`UNIT_DYNFLAG_LOOTABLE` clearing). See [`effects::loot_art`].
    loot: EffectSet,
    /// What the server sent directly: `SMSG_PLAY_SPELL_VISUAL` and
    /// `SMSG_PLAY_SPELL_IMPACT`, whose body is a `SpellVisualKit` id with no
    /// spell.
    ///
    /// A separate set because its lifetime matches none of the five above. It
    /// is not a cast, no aura states it, and nothing else in the world implies
    /// it; the packet is the event. In `cast`, the next spell cast would cancel
    /// a character's eating visual.
    ///
    /// Both opcodes share one set. On the wire they differ in whether the guid
    /// names the unit that acted or the unit acted upon. By the time either
    /// reaches this entity, the guid has already selected it, so on this unit
    /// the two mean the same thing. See [`effects::pushed_kits`].
    pushed: EffectSet,
    /// Models a host asked for directly: a model and a point with no spell, kit
    /// or packet behind it, kept until the host removes it. See [`Self::hang`].
    /// Empty in every session, because nothing on the wire fills it. It is
    /// separate from [`Self::pushed`] because that set is restarted by the next
    /// packet and this one must not be.
    hung: EffectSet,
    // The sets above are carried across a re-dressing rather than rebuilt with
    // it; see [`CarriedEffects`].
    /// The aura ids [`Self::state`] was built from: the ones that state a
    /// visual, in slot order. The next frame's `UNIT_FIELD_AURA` is filtered
    /// and compared against this list. A buff that expires and a buff that is
    /// applied both make the list disagree with the wire. A debuff ticking on
    /// and off does not, because it is never in the list.
    state_auras: Vec<u32>,
    /// The colour this model's own batches are currently tinted with, or
    /// `None` (the usual case). See [`tint`].
    ///
    /// Stored on the model rather than beside `state_auras` in
    /// [`CarriedEffects`], because it records what the materials currently
    /// carry, not what the auras request. A re-dressing builds fresh materials
    /// with no tint and drops this field with the old model, so the next
    /// frame's comparison finds `None` against, for example, a stone-formed
    /// dwarf and reapplies the colour.
    painted: Option<[u8; 3]>,
    /// The opacity this model's own batches are currently drawn at, or `None`
    /// (the usual case). See [`tint::fade_models`].
    ///
    /// Stored on the model for the same reason as [`Self::painted`], by the
    /// same mechanism: it records what the materials carry, not what the wire
    /// says. A re-dressing builds fresh opaque materials and drops this field
    /// with the old model, so the next frame's comparison reapplies the fade.
    faded: Option<u8>,
    /// The entity's own drawn parts whose colour is animated, and the model's
    /// colour tracks. Almost always empty (a wolf does not fade), but the
    /// mechanism is the one the effects use, so it uses the same pair of
    /// fields. See [`Tinted`].
    tinted: Vec<Tinted>,
    tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
    /// `None` until [`spell_effects`] has seen this entity once. On the first
    /// look it records the counters without acting on them, so a creature that
    /// walks into view mid-cast does not get the glow of a cast that began
    /// before it was modelled.
    ///
    /// Holds `(begun, released, cancelled, delayed)` and the channel count. A
    /// cancel and a pushback are edges too, so a first look must not act on
    /// them either.
    effects_for: Option<effects::CastCounters>,
    /// The wire's release counter as last seen, which the impact is hung from;
    /// see [`WorldEntity::casts_landed`]. `None` until the first look, for the
    /// same reason: a creature that walks into view just after casting must not
    /// show the impact on its target.
    landed_for: Option<u32>,
    /// The two pushed-kit counters as last seen, on the same terms: a unit that
    /// walks into view after being sent kit 406 must not show another
    /// character's eating visual. See [`WorldEntity::spell_visuals`].
    pushed_for: Option<(u32, u32)>,
    /// The wearer's own attachment points, from its M2. One `Arc` clone per
    /// entity, shared with every other entity using the same model.
    points: Arc<Vec<M2Attachment>>,
    /// The model's sound cues, per sequence. See `sound::cues`.
    pub cues: Arc<vale_assets::world::m2::SoundCues>,
    /// Height of the entity's head above its origin, in model yards; a caller
    /// multiplies by the entity's scale. See [`head_height`].
    ///
    /// Used only by missiles, which aim at the middle of a body. It is not
    /// [`Self::anchor`], which is the camera's target and on a quadruped is
    /// the muzzle.
    pub head: f32,
    /// Height of the entity's name above its origin, in model yards:
    /// attachment 18, `PlayerName`, which is used for nothing else. See
    /// [`vale_assets::look::unitname::ANCHOR_ATTACHMENT`], and [`name_height`]
    /// for the fallback when a model lacks the point.
    ///
    /// Stored beside [`Self::head`] rather than derived from it because the two
    /// are different points: the helm point is the top of the head, and the
    /// client places the name almost a foot higher on a human.
    pub name_anchor: f32,
    /// Height of the camera anchor above the entity's origin, in model yards;
    /// a caller multiplies by the entity's scale. See
    /// [`vale_assets::look::anchor`], whose rule matches the 1.12.1 client.
    ///
    /// Attachment point 17, the neck, not the helm point above it. The
    /// difference is 0.13 yards on a human, and it caused the report that the
    /// camera's focal point was slightly too high.
    pub anchor: f32,
    /// The camera anchor's height once the `Mount` clip has placed the body in
    /// a saddle, or `None` for a model with no such clip (every model that is
    /// not a character).
    ///
    /// Static. Read from the live pose, it would rise and fall with a
    /// galloping horse's spine and move the camera with it.
    pub mounted_anchor: Option<f32>,
    /// The model's own header sphere, in model yards. Every path in
    /// [`Self::pick_sphere`] falls back to it, and it is the whole answer for a
    /// model with no skeleton.
    ///
    /// Stored here, like the two radii below, because the `ModelAssets` it came
    /// from is not kept after the build.
    model_sphere: vale_assets::look::pick::Sphere,
    /// The drawn triangles the pointer is tested against: pick stage two, in
    /// model space, shared per model path.
    ///
    /// See [`vale_assets::look::pick::hit_mesh`], which builds it, and
    /// [`crate::interface::target`], which skins it with [`Self::joints`] so
    /// that a click tests the silhouette on screen rather than a separately
    /// computed pose.
    pub pick: Arc<vale_assets::look::pick::PickMesh>,
    /// The model's particle emitters and ribbon trails (a wisp's dust, an
    /// imp's flames, an elemental's streamers). They are root entities because
    /// both kinds build their mesh in world space, so a rebuild despawns them
    /// from this list. When an entity streams out, the emitter and ribbon
    /// passes' own retirement sweeps remove them.
    roots: Vec<Entity>,
    /// Radius of the shadow blob, in model yards: the model's own declared
    /// half-extent. Stored because the `ModelAssets` it came from is not kept.
    ///
    /// Public because the blob is not a child of this entity: every blob in the
    /// world is one mesh built by [`crate::render::shadows`], which reads the
    /// footprint from here. That module explains why.
    pub shadow_radius: f32,
    /// A sphere in model yards that covers the model's own declared box from
    /// the entity's origin (centre offset plus half-diagonal), so it encloses
    /// the same volume the part meshes are frustum-culled by. [`animate`]'s
    /// visibility gate tests it. The two must agree, because a part the camera
    /// can draw must never use a pose the gate skipped.
    cull_radius: f32,
    /// Whether this model tilts with the ground under it, from the M2 header's
    /// `GlobalModelFlags`; see [`vale_assets::look::conform`]. Stored here, like
    /// the two radii above, because the `ModelAssets` it came from is not kept
    /// after the build.
    ///
    /// `Level` for every character model in the game, so a player on foot
    /// stays upright on a hillside. The 1.12.1 client does the same.
    pub conform: vale_assets::look::conform::Conform,
    /// The room this model was dressed for. Always `None`; kept so the retag
    /// path stays one comparison.
    ///
    /// Units were once re-dressed with room lighting when crossing a door and
    /// retagged with the room's colour between rooms. The 1.12.1 client does
    /// neither: every model inside an inn is lit by the same sun in view space
    /// at the 0.5 scale, the zone's own fill, and the room's point lights. The
    /// room's baked colour applies to its walls and furniture, not to units. So
    /// a unit keeps its sun-lit dressing everywhere, and what changes at a door
    /// is [`SunScale`], which blends.
    room: Option<RoomLight>,
    /// The sun scale its batches were tagged with; see [`SunScale`]. Stored so
    /// that a change is a retag rather than a rebuild, as the room's colour is.
    sun: f32,
    /// What was in the entity's hands when this was built, and whether it was
    /// drawn.
    ///
    /// Needed because weapons change a creature's attached models: a guard who
    /// draws his mace on entering combat changes the models attached to him
    /// and nothing else.
    ///
    /// Stores the whole [`Weapon`] rather than its display id, because the
    /// sheath type decides which point a sheathed weapon hangs from. A player's
    /// sheath type arrives a query round trip after the display id, so a
    /// comparison on the id alone would keep the first model, built from an
    /// incomplete item, and never rebuild it.
    hands: ([Weapon; 3], u8),
}

impl EntityModel {
    /// Stage one of the mouse pick: the sphere the pointer's ray must cross
    /// before this unit's triangles are tested, in model yards.
    ///
    /// `playing` is the sequence the unit is in this frame
    /// ([`Playback::clip`]), because the 1.12.1 client picks against the
    /// playing sequence's bounds. The model's header box is a union over every
    /// animation and emitter, so a wisp's is a 12.8-yard cube around a creature
    /// a yard across. See [`vale_assets::look::pick`] for the rule, and
    /// `vale pick`, which measures it: the standing sphere averages 57% of the
    /// header sphere across the bestiary.
    pub fn pick_sphere(&self, playing: Option<&M2Sequence>) -> vale_assets::look::pick::Sphere {
        vale_assets::look::pick::sequence_sphere(self.model_sphere, playing)
    }

    /// Hang models on this unit with no spell behind them, until
    /// [`Self::unhang`]. Each is filtered against the points the unit's own M2
    /// carries, loaded and hung as a kit's models are, and posed on the bone
    /// it names, so a model hung here is drawn as the game would draw it in the
    /// same slot.
    ///
    /// Models already hung are removed first: the argument replaces the set,
    /// it does not add to it.
    pub fn hang(&mut self, commands: &mut Commands, models: Vec<vale_assets::tables::spell::KitEffect>) {
        self.hung.clear(commands);
        let points = Arc::clone(&self.points);
        self.hung.want(models, f32::INFINITY, &points);
    }

    /// Take down whatever [`Self::hang`] put up.
    pub fn unhang(&mut self, commands: &mut Commands) {
        self.hung.clear(commands);
    }

    /// The root entity of each model [`Self::hang`] has landed so far, in the
    /// order they were asked for. A root's `GlobalTransform` is the attachment
    /// point's frame in the world, which is what a host placing something
    /// relative to that point needs.
    pub fn hung_roots(&self) -> Vec<Entity> {
        self.hung.parts.iter().map(|part| part.root).collect()
    }

    /// Whether every model [`Self::hang`] asked for is up, or has failed.
    pub fn hung_settled(&self) -> bool {
        self.hung.wanted.is_empty()
    }
}

/// One set of spell-effect models on a unit, from asking for them to taking
/// them off.
///
/// An [`EntityModel`] holds several of these (the cast, the impact, the aura
/// state and others). They share one type because the mechanism is the same
/// and only the policy differs. Every set loads asynchronously, filters
/// against the wearer's own attachment points, hangs through [`hang_model`]
/// and is despawned by its root. Only what sets `until` differs, and the
/// caller decides that. One type keeps a fix to the mechanism from reaching
/// only some of the sets.
#[derive(Default)]
struct EffectSet {
    /// Requested and still loading. Drained as each model's M2 arrives, which
    /// is a load after the wearer's, as with an item attachment.
    wanted: Vec<vale_assets::tables::spell::KitEffect>,
    /// Built and hanging on their bones.
    parts: Vec<AttachedPart>,
    /// When this set is removed, in `Time::elapsed_secs`.
    ///
    /// `f32::INFINITY` for the aura state, which ends on a condition rather
    /// than a clock. That is why this is an `f32` rather than an
    /// `Option<f32>`: the comparison is the same either way, and a set that
    /// never expires needs no special case in the code.
    until: f32,
    /// Whether the models' own clip length may extend [`Self::until`].
    ///
    /// A release and an impact end when their models finish playing, and only
    /// the models state when that is: `IceArmor_Low_Head`'s one-shot runs
    /// 3,000 ms and `ArcaneIntellect_Impact_Base`'s 1,900 ms. A flat 1,500 ms
    /// deadline cut both off part-way through their fade, so the effect
    /// appeared and then vanished abruptly. The deadline is raised in
    /// [`Self::spawn`] as each model loads, because the length is unknown until
    /// the M2 is read.
    ///
    /// False for a wind-up, which ends when the cast ends regardless of how
    /// long its glow was authored to run (an interrupted Fireball must not keep
    /// its hands lit). False for the aura state and the loot sparkle, which end
    /// on a condition.
    clip_bound: bool,
}

/// The effect sets, held on the entity across a re-dressing.
///
/// A spell effect is not part of the dressing and must not be rebuilt with it.
/// `SpellCastOmni` carries `AnimationData`'s `STOW_HANDS_BUSY`, so every cast
/// stows the weapon. That changes `Sheath`, which once failed
/// [`EntityModel::matches`]' `hands` test and tore the whole model down three
/// frames after the effect was hung, while its glow was still growing. The
/// teardown removed every other effect on the unit as well: an aura's state
/// glow, an impact landing at the same moment, and the ground decals owned by
/// all of them.
///
/// That teardown was not the only cause of spell effects being cut short; the
/// other was the dropped hit list (see `Entity::casts_landed`). Any other
/// rebuild of the model, such as walking through a door, still tears it down.
///
/// So [`super::spawn::rebuild_changed_models`] lifts the sets off the old
/// model and leaves their roots in place, and [`super::spawn::spawn_models`]
/// moves them onto the new one.
///
/// This happens only when the display id is unchanged, which is the first
/// test `matches` makes. An `AttachedPart::bone` indexes the wearer's skeleton
/// and the roots are posed against it, so on a different model a glow would
/// hang from whichever bone had the same index. A re-dressing keeps the
/// skeleton. A shapeshift does not, and then the sets are removed.
#[derive(Component, Default)]
pub(super) struct CarriedEffects {
    cast: EffectSet,
    impact: EffectSet,
    state: EffectSet,
    /// The level-up glow, the longest-lived one-shot of these sets and so the
    /// most likely to be up when a rebuild happens.
    milestone: EffectSet,
    /// The loot sparkle, which lasts longest of all: it stays on while the body
    /// has loot, so a rebuild that dropped it would leave a lootable corpse
    /// without it for good. Nothing re-arms it, because [`effects::loot_art`]
    /// acts on the edge and the edge has passed.
    loot: EffectSet,
    /// The server's directly sent kits, which nothing re-arms either: the
    /// packet that requested them has already been handled.
    pushed: EffectSet,
    /// The host's hung models, for the same reason: a re-dressing is not the
    /// host's action and must not remove what the host hung.
    hung: EffectSet,
    /// Carried with [`Self::state`] to keep the two in step. Otherwise the new
    /// model would start with an empty aura list, disagree with the wire on the
    /// first frame, and clear and re-hang every buff glow on the unit,
    /// restarting each one's clock. `spell_effects` filters the aura set to
    /// avoid exactly that.
    state_auras: Vec<u32>,
    /// The cast counters already handled, so the rebuild is not treated as a
    /// first look. `None` here would drop a cast that began during the two
    /// frames the model was gone.
    effects_for: Option<effects::CastCounters>,
    /// The wire's release counter, for the same reason: a rebuild between the
    /// key press and `SMSG_SPELL_GO` would otherwise drop the impact, and a
    /// cast's weapon stow triggers a rebuild in exactly that window.
    landed_for: Option<u32>,
    /// The two pushed-kit counters, on the same terms: without them a rebuild
    /// would be treated as a first look, and a first look acts on nothing.
    pushed_for: Option<(u32, u32)>,
}

impl CarriedEffects {
    /// The attachment roots the teardown must leave alone.
    fn roots(&self) -> impl Iterator<Item = Entity> + '_ {
        [
            &self.cast,
            &self.impact,
            &self.state,
            &self.milestone,
            &self.loot,
            &self.pushed,
            &self.hung,
        ]
            .into_iter()
            .flat_map(|set| set.parts.iter().map(|part| part.root))
    }
}

impl EffectSet {
    /// Take everything off, loaded or not.
    fn clear(&mut self, commands: &mut Commands) {
        self.wanted.clear();
        for part in std::mem::take(&mut self.parts) {
            commands.entity(part.root).despawn();
        }
    }

    /// Ask for `models`, to come off at `until`.
    ///
    /// Filtered against the points the wearer's own M2 carries, as an item
    /// attachment is: a model with no head point cannot wear a crown of frost,
    /// and loading the effect anyway would place it nowhere.
    fn want(
        &mut self,
        models: Vec<vale_assets::tables::spell::KitEffect>,
        until: f32,
        points: &[M2Attachment],
    ) {
        self.wanted.extend(
            models
                .into_iter()
                .filter(|e| points.iter().any(|p| p.id == e.point)),
        );
        self.until = until;
        self.clip_bound = false;
    }

    /// Ask for `models`, to be removed when their clip has finished playing or
    /// at `floor`, whichever is later.
    ///
    /// The deadline only moves later, never earlier. A short clip keeps the
    /// floor rather than shortening it. The 1.12.1 client removes the effect
    /// when the clip ends, but this client's emitters keep spraying past that
    /// point, so using the clip as an upper bound too would make a burst vanish
    /// early. See [`Self::clip_bound`].
    fn want_until_played(
        &mut self,
        models: Vec<vale_assets::tables::spell::KitEffect>,
        floor: f32,
        points: &[M2Attachment],
    ) {
        self.want(models, floor, points);
        self.clip_bound = true;
    }

    /// Whether anything is on or on its way.
    fn is_empty(&self) -> bool {
        self.parts.is_empty() && self.wanted.is_empty()
    }

    /// Extend the deadline to cover one loaded model's clip, now that its file
    /// has been read.
    ///
    /// The set was armed with a flat fallback because the length was unknown
    /// until now, and the models of one set load on different frames. So this
    /// only moves the deadline later, and the set is removed when its
    /// longest-running model has finished. A set armed with [`Self::want`]
    /// rather than [`Self::want_until_played`] is unchanged: a wind-up ends
    /// when the cast does.
    fn stated(&mut self, part: &AttachedPart, now: f32) {
        if !self.clip_bound {
            return;
        }
        if let Some(clip) = part.clip_secs() {
            self.until = self.until.max(now + clip);
        }
    }

    /// Hang whatever has finished loading. Called every frame while
    /// [`Self::wanted`] is non-empty, which for a spell effect is a frame or
    /// two after the cast.
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &mut self,
        commands: &mut Commands,
        cache: &mut ModelCache,
        materials: &mut Materials,
        meshes: &mut Assets<Mesh>,
        wearer: Entity,
        points: &[M2Attachment],
        now: f32,
    ) {
        if self.wanted.is_empty() {
            return;
        }
        let mut still_wanted = Vec::new();
        for effect in std::mem::take(&mut self.wanted) {
            // No room, unlike an item attachment. A spell effect is an `unlit`
            // additive glow, so interior lighting would not change it, and a
            // second dressing per building would be built for every effect
            // model in the game.
            match cache.attached(&effect.path, None, None, SceneLighting::NONE, meshes, materials) {
                Lookup::Loading => still_wanted.push(effect),
                Lookup::Failed => warn!("spell effect {} will not read", effect.path),
                Lookup::Ready(attached) => {
                    let Some((bone, offset)) = points
                        .iter()
                        .find(|p| p.id == effect.point)
                        .map(|p| (p.bone as usize, p.position))
                    else {
                        continue;
                    };
                    let part = hang_model(
                        commands,
                        meshes,
                        wearer,
                        &attached,
                        bone,
                        offset,
                        effect.scale,
                        // The base point is on the floor, and it is the only
                        // one of the fourteen effect points that is.
                        // `attach::BASE` (19) is the caster's feet, where every
                        // ground-painted spell effect in the game is anchored.
                        // 8,071 spells put a model there (`vale spell`).
                        effect.point == vale_assets::world::m2::attach::BASE,
                        None,
                        crate::render::models::sun_scale::NEUTRAL,
                        now,
                    );
                    self.stated(&part, now);
                    self.parts.push(part);
                }
            }
        }
        self.wanted = still_wanted;
    }
}

/// One attached model in place: the bone it rides, where on that bone, and the
/// child entities drawing it.
pub struct AttachedPart {
    /// Index into the wearer's skeleton. Bounds-checked when it is written,
    /// because a model may name a bone it does not have, and the pose would
    /// then return another bone's matrix rather than failing.
    bone: usize,
    /// The attachment point in the bone's own frame, in the model's axes.
    offset: [f32; 3],
    /// Whether this part lies on the floor, which is true for `attach::BASE`
    /// only: roots, ice, nets, ground rings.
    ///
    /// Two places read it. The ground decal path in
    /// [`super::effects::hang_model`] draws a flat quad in world space rather
    /// than as a child. [`super::pose`] removes the wearer's rotation from the
    /// part's frame, because something on the ground does not turn when the
    /// unit standing on it turns. Without that, a rooted player turning on the
    /// spot turned its roots with it, and the effect looked painted on the
    /// character rather than on the floor.
    pub grounded: bool,
    /// A scale applied to the attached model only.
    ///
    /// 1 for a pauldron, since a helm is authored to fit its head.
    /// `SpellVisualEffectName`'s scale for a spell effect, where the same glow
    /// model is reused at several sizes and only the table says which. Public
    /// because a missile hangs from no bone and builds its own frame from it;
    /// see `missiles.rs`.
    pub scale: f32,
    /// The one child of the wearer that carries the attachment's frame.
    /// [`animate`] writes its `Transform`. The drawn parts and the model's own
    /// joints hang under it, so despawning it removes the whole attachment.
    /// Its emitters are removed by `retire_emitters`, which despawns an emitter
    /// whose owner is gone.
    root: Entity,
    /// The attached model's own joints (bone order, identity last), when it
    /// has a skeleton. An attached model is not rigid: a torch's glow plane
    /// rides a billboarded bone of the torch's own skeleton, and a spell
    /// effect grows through its own bone animation.
    joints: Vec<Entity>,
    skeleton: Option<Arc<M2Skeleton>>,
    /// The resolved index of the model's own Stand (id 0), or its first
    /// non-empty sequence. This is the sequence an attached model plays.
    sequence: usize,
    /// Whether that sequence loops (bit 0 of its flags clear, as in the 5875
    /// client). A non-looping effect holds its last frame, which for a
    /// one-shot like Arcane Explosion's dome is the collapsed one.
    loops: bool,
    /// When it was attached, in `Time::elapsed_secs`: the sequence clock's
    /// zero.
    since: f32,
    /// The drawn parts whose colour is animated, and which tracks animate it.
    /// Empty for a pauldron and for most models. This is how a spell effect
    /// fades out at its end. See [`Tinted`].
    tinted: Vec<Tinted>,
    /// The model's own colour and transparency tracks, shared with every other
    /// wearer of it. `None` when nothing on it fades.
    tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
    /// This model's own attachment points, which [`Self::nested`] hang from.
    points: Arc<Vec<M2Attachment>>,
    /// Models hung on this model's points rather than on the wearer's: an
    /// item visual's glows on a weapon. [`animate_attachment`] poses them from
    /// this model's pose. Their roots are children of [`Self::root`], so
    /// despawning it removes them.
    nested: Vec<AttachedPart>,
    /// Models asked for on this model's points whose files are still loading.
    /// Drained by `worn::hang_nested`.
    pending: Vec<vale_assets::tables::itemvisual::ItemEffect>,
}

impl AttachedPart {
    /// Whether a model asked for on this model's points is still loading, so
    /// a caller can skip [`hang_nested`] when nothing is outstanding.
    pub(crate) fn wants_nested(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// One drawn part whose `MeshTag` is its animated colour rather than its room.
///
/// The tag is the only per-instance channel, so a fade cannot be a material;
/// see [`crate::render::models::tint_tag`]. This pairs the part entity with the
/// batch's own two track indices, because two batches of one model fade on
/// different curves: a fireball's core and its halo are separate `M2Color`
/// rows.
struct Tinted {
    part: Entity,
    tint: vale_assets::world::m2::BatchTint,
}

impl AttachedPart {
    /// The entity carrying this attachment's frame, so a caller can write it.
    pub(crate) fn root(&self) -> Entity {
        self.root
    }

    /// Where this attachment sits in the wearer's own space, given the
    /// wearer's pose: the carrying bone's matrix, the point in that bone's
    /// frame, and the attached model's own scale, in that order.
    ///
    /// `None` when the wearer's skeleton has no such bone. A model may name a
    /// bone it does not have, and reading past the pose would return another
    /// bone's matrix rather than failing.
    ///
    /// A method rather than three fields read by the caller, because two
    /// callers compose this product (the entity pose and the character on the
    /// character-select plinth) and they must stay identical.
    pub(crate) fn local(&self, pose: &[[f32; 12]]) -> Option<Mat4> {
        let bone = pose.get(self.bone)?;
        Some(
            crate::render::axes::pose_to_bevy(bone)
                * Mat4::from_translation(crate::render::axes::to_bevy(self.offset))
                * Mat4::from_scale(Vec3::splat(self.scale)),
        )
    }

    /// How long this part's clip runs, in seconds. `None` for a clip that
    /// loops or a model with no sequence.
    ///
    /// This decides when a one-shot spell effect ends. The 1.12.1 client
    /// removes a shard when its model's animation finishes, so the clip is the
    /// lifetime, and every flat constant around it is a fallback for models
    /// that state no clip. A looping clip returns `None` rather than its
    /// length: `ChargeTrail`'s take is 334 ms and it runs for as long as the
    /// charge does, so treating a loop as a deadline would end the effect
    /// after one cycle.
    pub(crate) fn clip_secs(&self) -> Option<f32> {
        if self.loops {
            return None;
        }
        self.skeleton
            .as_ref()
            .and_then(|s| s.sequences.get(self.sequence))
            .map(|s| s.end.saturating_sub(s.start) as f32 / 1000.0)
            .filter(|s| *s > 0.0)
    }

    /// An attachment with no geometry, no skeleton and nothing to fade, which
    /// [`animate_attachment`] leaves alone.
    ///
    /// Used by the missile flight test, which checks a projectile's arithmetic
    /// and would otherwise need an open archive to build one, since
    /// [`hang_model`] takes a loaded [`ModelAssets`].
    #[cfg(test)]
    pub(crate) fn rigid(root: Entity, scale: f32) -> AttachedPart {
        AttachedPart {
            bone: 0,
            offset: [0.0; 3],
            grounded: false,
            scale,
            root,
            joints: Vec::new(),
            skeleton: None,
            sequence: 0,
            loops: true,
            since: 0.0,
            tinted: Vec::new(),
            tints: None,
            points: Arc::new(Vec::new()),
            nested: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Where this attachment sits in the wearer's space when the carrying
    /// model has no skeleton: the point and the scale, with the bone in its
    /// bind pose. A rigid weapon's points are in its model space already.
    pub(crate) fn unposed(&self) -> Mat4 {
        Mat4::from_translation(crate::render::axes::to_bevy(self.offset))
            * Mat4::from_scale(Vec3::splat(self.scale))
    }

    /// Ask for `effects` to be hung on this model's own points as each loads.
    /// An effect on a point this model does not carry is dropped, as a wearer
    /// drops an item for a point it lacks. Replaces anything still pending.
    pub(crate) fn want_nested(&mut self, effects: Vec<vale_assets::tables::itemvisual::ItemEffect>) {
        let points = &self.points;
        self.pending = effects
            .into_iter()
            .filter(|effect| points.iter().any(|p| p.id == effect.point))
            .collect();
    }
}

impl EntityModel {
    /// Whether this model still describes the entity, or is a frame behind a
    /// change that has to be rebuilt rather than nudged.
    fn matches(&self, world: &WorldEntity, room: Option<RoomLight>) -> bool {
        if world.display_id != Some(self.display_id) {
            return false;
        }
        // Walking through a door changes what lights the entity, not the
        // entity. Only the indoor/outdoor choice is in the materials; a change
        // of room colour is a retag, not a rebuild.
        if self.room.is_some() != room.is_some() {
            return false;
        }
        // What is in the hands is not tested here. Drawing a weapon changes
        // only the attached models: `vale_assets::look::dress` reads `weapons`
        // and `sheath_state` only to build the attachment list, because a
        // weapon has no geoset and does not affect the composed skin. The
        // body's batches, joints and dressing are unchanged, so rebuilding
        // them produces nothing new.
        //
        // Testing the hands here also broke effects. `SpellCastOmni` carries
        // `AnimationData`'s `STOW_HANDS_BUSY`, so every cast stows the weapon.
        // That failed this test a frame or two after the cast's glow was hung,
        // and the teardown removed the glow. A caster who was also swinging
        // was rebuilt again on the next Ready stance: twice per cast, per
        // caster, during a fight.
        //
        // [`super::spawn::rebuild_changed_models`] re-hangs the wardrobe in
        // place instead, in the same way as the room retag above.
        match (&self.look, world.appearance) {
            (Some(built), Some(appearance)) => {
                built.appearance == appearance && built.equipment == world.equipment
            }
            // A creature: nothing else about it can change what it is.
            (None, _) => true,
            // Dressed as a character and no longer one, which a shapeshift does.
            (Some(_), None) => false,
        }
    }
}

/// The room the entity is standing in, or `None` outdoors.
///
/// Written by [`light_entities`] once a frame and read by the dressing; this is
/// the only link between entities and interior lighting. A component rather
/// than a field on `WorldEntity` because the server does not supply it: it is
/// a geometric fact about where the entity stands, and the object manager has
/// no knowledge of buildings.
#[derive(Component, Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Indoors(pub Option<RoomLight>);

/// The per-instance multiplier on the sun term this entity's batches carry:
/// [`crate::render::models::sun_scale`]'s 2.5 on lit ground and 0.5 on ground
/// that carries a baked `MCSH` shadow, indoors or out.
///
/// A doodad takes this from the shadow bit under its origin at tile load. A
/// unit moves, so it is sampled here once a frame from the same terrain cache
/// the mover stands on. Units take it as doodads do: the 1.12.1 client draws
/// the character's own batches at `Diffuse = 2.5 × band 0` outdoors and the
/// models around it at 0.5 inside an inn, all with the zone's own fill and
/// the room's lamps and none lit by the room's colour. Which condition makes
/// the 1.12.1 client use 0.5 for a model is not established; this client uses
/// the ground's shadow bit, and an inn's floor lies in the inn's own baked
/// shadow.
///
/// The scale is blended, not switched. The bit is per 0.5-yard texel, so a
/// unit crossing a shadow's edge would otherwise jump between 2.5 and 0.5 in
/// one step, which was reported as the lighting flipping on. The 1.12.1 client
/// has been seen with one model mid-way, at 0.81, so it eases the value. Its
/// rate is not measured; [`SUN_SCALE_RATE`] is this client's value.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub struct SunScale {
    /// What the batches are tagged with this frame.
    now: f32,
    /// Where it is heading: the bit under the feet.
    target: f32,
}

/// How fast the sun scale moves toward its target, per second: the full
/// 2.5 to 0.5 change takes one second. See [`SunScale`].
const SUN_SCALE_RATE: f32 = 2.0;

impl SunScale {
    /// A scale that is already where it is going.
    pub fn at(scale: f32) -> SunScale {
        SunScale { now: scale, target: scale }
    }

    /// The multiplier to tag the batches with.
    pub fn now(&self) -> f32 {
        self.now
    }

    /// One frame's step toward `target`, at [`SUN_SCALE_RATE`].
    fn toward(self, target: f32, dt: f32) -> SunScale {
        let step = SUN_SCALE_RATE * dt;
        let now = if (target - self.now).abs() <= step {
            target
        } else {
            self.now + step.copysign(target - self.now)
        };
        SunScale { now, target }
    }
}

/// How far above an entity's feet the interiority test is taken, in yards.
///
/// Not at the feet, where the position is. A room's box is its own geometry,
/// so its top face is the outside of the ceiling, and a character standing on
/// a flat roof stands exactly on that face, which counts as inside. A yard up
/// is still well inside any room a character can enter and above every roof
/// they can stand on.
const INDOOR_PROBE: f32 = 1.0;

/// Decide which entities are standing in a room, and in whose.
///
/// Runs after [`crate::world::session::place_entities`] with the rest of this
/// pass, so the point tested is this frame's. The `Interior` components are the
/// placed buildings of the loaded 3x3 tiles, a few dozen. Each rejects on its
/// own box before testing its rooms, so a city's three hundred rooms are only
/// tested for an entity standing in the city.
///
/// Written only when it changes, so `Changed<Indoors>` stays a useful filter
/// and an entity standing still is not re-dressed sixty times a second.
fn light_entities(
    mut commands: Commands,
    entities: Query<(Entity, &Transform, Option<&Indoors>, Option<&SunScale>), With<WorldEntity>>,
    interiors: Query<&crate::render::wmos::Interior>,
    // The terrain under the entity, for the shadow bit; see [`SunScale`].
    // Optional because the indoor test needs no session and is tested without
    // one. With no terrain, the ground counts as lit.
    session: Option<Res<crate::world::session::Session>>,
    // For the blend; an app with no clock steps the whole way at once.
    time: Option<Res<Time>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::LightEntities);
    let active = session.as_ref().and_then(|s| s.active.as_ref());
    let dt = time.as_ref().map_or(f32::MAX, |t| t.delta_secs());
    for (entity, transform, current, scale) in &entities {
        let feet = crate::render::axes::to_wow(transform.translation);
        let mut point = feet;
        point[2] += INDOOR_PROBE;
        let now = Indoors(
            interiors
                .iter()
                .find(|interior| interior.holds(point))
                .map(|interior| interior.light),
        );
        if current != Some(&now) {
            commands.entity(entity).insert(now);
        }
        let target = if active.is_some_and(|a| a.terrain_shadowed(a.map_id, feet[0], feet[1])) {
            crate::render::models::sun_scale::SHADOWED_GROUND
        } else {
            crate::render::models::sun_scale::LIT_GROUND
        };
        let sun = match scale {
            Some(scale) => scale.toward(target, dt),
            None => SunScale::at(target),
        };
        if scale != Some(&sun) {
            commands.entity(entity).insert(sun);
        }
    }
}

/// An entity whose display id resolves to nothing, or whose model will not read.
///
/// Marked so the lookup is not retried every frame, and so the fallback shape
/// knows to draw it.
#[derive(Component)]
pub struct NoModel;

/// One bone of one entity. Carries a `GlobalTransform` and no `Transform`; see
/// the module comment.
///
/// A joint of the entity's own skeleton also carries a [`pose::Bone`], which
/// lets `animate` write it in parallel. The joints of what the entity carries
/// (its mount, its attachments, its effects) do not, and are written from the
/// pass's serial lists. See [`pose::Posed`].
#[derive(Component)]
pub struct Joint;

/// One batch of an entity's model.
#[derive(Component)]
pub struct EntityPart;

/// One batch of a model attached to an entity, such as a pauldron or a helm.
///
/// It carries an ordinary `Transform` rather than the composed world matrix a
/// [`Joint`] gets. An attached model is drawn, so Bevy requires a `Transform`
/// on it, and transform propagation composes the wearer's placement into it.
/// What is written here is therefore the bone's pose in the model's own frame,
/// the part a joint keeps separate for a different reason.
#[derive(Component)]
pub struct AttachedTo;

/// What an entity is playing, and since when.
#[derive(Component)]
pub struct Playback {
    /// The key hints [`vale_assets::world::m2::M2Skeleton::pose_into`] keeps
    /// between frames. Per entity, because two units of one model run on
    /// different clocks. Empty until the first pose; see
    /// [`vale_assets::world::m2::M2Track::sample_hinted`].
    pub(super) hints: Vec<u32>,
    /// The span of the base sequence this frame advanced through: the
    /// sequence, the clock before and after, and whether the sequence is new
    /// this frame. `sound::cues` fires the model's cues from it. Recorded by
    /// `animate` for every rig, on or off screen.
    window: Option<(usize, u32, u32, bool)>,
    /// The `AnimationData.dbc` id last asked for.
    wanted: u16,
    /// Which of the model's sequences that resolved to.
    sequence: usize,
    /// When it started, in `Time::elapsed_secs`.
    since: f32,
    fade: Option<Fade>,
    /// A one-shot in progress (a swing, a flinch) and when it must end.
    ///
    /// Kept apart from `wanted` because it is not a state: the entity is still
    /// running or standing underneath it. When the one-shot finishes, the
    /// animation returns to what the state says then, not what it said when
    /// the one-shot began.
    oneshot: Option<OneShot>,
    /// Every event counter as this entity last reported it.
    ///
    /// The trigger is the change, not the value: the server states each event
    /// exactly once, so a renderer that polls must notice the counter move.
    /// `None` until the first poll, because an entity that comes into view
    /// mid-fight with a swing count of forty should play no swings.
    seen: Option<Counters>,
    /// A cast in progress: which pose it holds, and until when.
    ///
    /// A state, not a one-shot, so it is not a [`OneShot`]: the wind-up is
    /// held while the cast bar runs, whose length the server states in
    /// `SMSG_SPELL_START`. It normally ends early when `SMSG_SPELL_GO`
    /// releases it. The timer exists so that an interrupted cast does not
    /// leave a character in the wind-up pose for the rest of the session.
    ///
    /// The pose is stored with the cast because it belongs to the spell, not
    /// to casting in general: a fireball is thrown from the shoulder and a heal
    /// is raised overhead, and only the spell id, through `SpellVisual`, tells
    /// them apart. See [`vale_assets::tables::spell`].
    casting: Option<Casting>,
    /// The pose an aura holds this unit in: `SpellVisualKit`'s state-kit
    /// `animID`, resolved from the unit's own `UNIT_FIELD_AURA` slots.
    ///
    /// It uses the same track slot as a cast's wind-up and the same held-pose
    /// rules, but it is a separate field rather than another `Casting`. A cast
    /// has a length the server states; an aura is a condition with no clock,
    /// ending when the server stops listing the aura. A `Casting::until` would
    /// require inventing a duration for a stun.
    ///
    /// 308 spells state an aura pose and 257 of them are `Stun`
    /// (`vale spell`); without this field, stuns played no animation.
    /// Re-read every frame from the snapshot rather than latched on a counter,
    /// for the same reason as [`Playback::speed`]: it is a fact about the unit,
    /// not an event.
    aura: Option<u16>,
    /// `UNIT_FLAG_STUNNED` with no pose to play: the unit's animation clock is
    /// held.
    ///
    /// A stun almost always names its own pose, and 257 of the 308 that name
    /// one name `Stun`. Some stuns name none; Ice Block is the clearest case.
    /// Spell 11958 is three `SPELL_AURA_MOD_STUN` effects (`Spell.dbc` columns
    /// 91..93 read `12, 39, 39`) whose state kit 3709 reads `animID = -1`. The
    /// 1.12.1 client shows the mage standing still inside the ice, not idling
    /// and not breathing, and this field draws that.
    ///
    /// Part of this is established and part comes from a report. Established:
    /// the 1.12.1 client uses this flag, and nothing in the `SpellVisual` chain
    /// names a pose for this spell. Not established: that the client does it
    /// by stopping the animation clock. None of `SpellVisualKit`'s 35 columns
    /// says "freeze", so this reproduces the reported picture in the simplest
    /// way. It applies only when the flag is set and no pose is stated, so no
    /// spell that states its own pose reaches it.
    frozen: Option<u32>,
    /// Whether this frame wants the clock held; see [`Self::frozen`].
    freeze: bool,
    /// The second track: what the upper body plays over the legs.
    ///
    /// A swing at a run, an emote while walking and a moving caster's wind-up
    /// all go here rather than into [`Self::oneshot`], which keeps a character
    /// from stopping mid-stride to raise its hands. [`route_oneshot`] chooses
    /// the slot for each play from the unit's state at that moment, never from
    /// the animation id alone.
    overlay: Option<Masked>,
    /// How fast the unit is travelling, as of the last poll.
    ///
    /// A locomotion clip plays at `speed / move_speed`, not at 1×; see
    /// [`M2Skeleton::playback_rate`] for the rule and
    /// [`Playback::note_actions`], which keeps this current. A field rather
    /// than an argument because every clock here is derived from a start time
    /// and `now`, and passing the speed to each would put the same value in
    /// five signatures.
    speed: f32,
    /// The base track's own playback multiplier. Only the combat fast path
    /// sets it to anything but 1; see [`Playback::fast_path`]. Separate from
    /// the locomotion scaling in [`M2Skeleton::playback_rate`], which depends
    /// on the clip and the unit's speed. This belongs to the current play and
    /// is reset by the next one.
    base_rate: f32,
    /// A deferred combat request. The 1.12.1 client keeps one such pending
    /// request.
    ///
    /// A combat clip requested while another combat clip is playing is not
    /// started: the running one speeds up, and the request waits here until
    /// nothing is playing. One slot, because the 1.12.1 client keeps one, and
    /// because a fast weapon that outran two clips has already lost the first
    /// swing either way.
    deferred: Option<u16>,
    /// The model's bones. Held here rather than looked up per frame; a model is
    /// shared, so this is one `Arc` clone per entity.
    skeleton: Arc<M2Skeleton>,
    /// The animation id a play was started with this frame, on either track,
    /// or `None` if nothing new started.
    ///
    /// The only input to the sheath reconcile (see [`sheath::reconcile`]). It
    /// is a latch rather than "whatever is playing" because the 1.12.1 client
    /// reconciles the weapons only when an animation starts. A frame with no
    /// new play must leave the weapons alone, or a held cast's stow would be
    /// undone on the next frame. Taken, not read; see [`Playback::take_played`].
    ///
    /// The id requested, not the one the model resolved to. A weaponless
    /// creature asked for `Attack2H` reconciles on that row's flags even though
    /// its skeleton fell back to the unarmed swing, because the 1.12.1 client
    /// reconciles on the requested animation.
    played: Option<u16>,
}

/// A play on the masked upper-body track — see [`Playback::overlay`].
///
/// Like [`OneShot`], with one addition: it carries the sequence as well as the
/// id, because unlike the base track nothing else records what the torso
/// resolved to.
struct Masked {
    /// The `AnimationData.dbc` id, so a held loop can tell whether it is
    /// already the thing playing.
    wanted: u16,
    sequence: usize,
    /// This play's own multiplier: 1, or 2 once the combat fast path has
    /// re-timed it. [`Playback::base_rate`] is the same value for the other
    /// track.
    rate: f32,
    /// When it started, in `Time::elapsed_secs`.
    since: f32,
    /// When it ends. As with [`OneShot::until`], taken from the resolved
    /// sequence's own length. `f32::INFINITY` for a held loop: a moving
    /// caster's wind-up runs until the cast ends, not until the clip ends.
    until: f32,
    /// Whether the clock wraps. A one-shot holds its last frame the way the
    /// base track does; a held wind-up loops.
    looping: bool,
}

/// The values an entity last reported whose change triggers an animation:
/// event counters and two states, as one value.
///
/// Grouped rather than stored as separate fields on [`Playback`] because they
/// are read and replaced as a set. The comparison is "what has happened since
/// the last look", and a half-updated snapshot would answer it wrongly.
///
/// [`Self::airborne`] is a condition, not a counter, and belongs here anyway.
/// The renderer wants its edges: the take-off and the landing are one-shots at
/// the two ends of a jump whose middle is an ordinary state. An edge needs
/// what a counter needs: the previous reading, and the first-look guard that
/// stops an entity streaming into view mid-air from playing a jump it did not
/// make. Storing it in this struct provides both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Counters {
    swings: u32,
    blows: u32,
    emotes: u32,
    /// The two `SpellVisualKit` counters the server sends at a unit directly;
    /// see [`WorldEntity::spell_visuals`]. Counters like the emote beside them:
    /// the kit's `animID` is a pose, the packet is an instant, and the poll
    /// wants the edge.
    spell_visuals: u32,
    spell_impacts: u32,
    casts_begun: u32,
    casts_released: u32,
    /// Casts cancelled; see [`WorldEntity::casts_cancelled`]. A counter for
    /// the same reason as the other two: the poll wants the edge, and a
    /// cancel is stated exactly once.
    casts_cancelled: u32,
    /// Pushbacks; see [`WorldEntity::casts_delayed`]. A counter because the
    /// poll wants the edge, and a cast pushed back twice is two statements
    /// whose delays add.
    casts_delayed: u32,
    /// How many of the begins were a channel starting, the one begin that
    /// arrives after the release it belongs to; see
    /// [`WorldEntity::casts_channelled`]. Without it, the release that arrived
    /// just before cancelled the channel's own wind-up, and Evocation stood
    /// still for eight seconds.
    casts_channelled: u32,
    airborne: bool,
    /// `GAMEOBJECT_STATE`, an edge on the same terms as [`Self::airborne`]: a
    /// chest going from shut to open plays `Open` once between two held poses.
    /// 255 means "not a game object", so a unit never looks like an object that
    /// changed state.
    object_state: u8,
    /// One-shot animations the server asked a game object to play; see
    /// [`WorldEntity::object_anims`].
    object_anims: u32,
}

impl Counters {
    fn of(world: &WorldEntity) -> Counters {
        Counters {
            swings: world.swings_thrown,
            blows: world.blows_taken,
            emotes: world.emotes,
            spell_visuals: world.spell_visuals,
            spell_impacts: world.spell_impacts,
            casts_begun: world.casts_begun,
            casts_released: world.casts_released,
            casts_channelled: world.casts_channelled,
            casts_cancelled: world.casts_cancelled,
            casts_delayed: world.casts_delayed,
            airborne: world.airborne,
            object_state: world.object_state.unwrap_or(u8::MAX),
            object_anims: world.object_anims,
        }
    }
}

/// The wind-up half of a cast: what is being held, and until when.
struct Casting {
    /// The `AnimationData.dbc` id to hold: the spell's own, from
    /// `SpellVisual`'s precast (or channel) kit, and nothing else.
    ///
    /// There is no generic fallback. A spell whose chain names no wind-up gets
    /// no `Casting` record, because `SpellVisualKit`'s `animID` is the only
    /// data that says what a cast looks like, and 9,467 of the game's 22,360
    /// spells name none. See [`super::pose::Playback::note_actions`].
    hold: u16,
    /// When the cast bar runs out, in `Time::elapsed_secs`.
    until: f32,
}

/// A one-shot animation: play it through once, then go back to the state.
struct OneShot {
    /// The `AnimationData.dbc` id, so a repeat of the same event can restart
    /// it: two swings in a row are two swings, not one held pose.
    wanted: u16,
    /// When it ends, in `Time::elapsed_secs`. Taken from the sequence's own
    /// length rather than a constant: a wolf's bite and a giant's overhead
    /// swing differ in duration, and a fixed value would either cut the long
    /// one off or hold the short one on its last frame.
    until: f32,
}

/// The animation being faded out of.
struct Fade {
    sequence: usize,
    /// When the faded-out sequence started, so it keeps its own clock while it
    /// fades: a wolf's stride finishes on the way out instead of freezing in
    /// the air for 150 ms.
    since: f32,
    /// When the fade began.
    from: f32,
    /// The time a one-shot ran out, when a one-shot is what is fading. The
    /// fade samples that frame, not the wrapped clock.
    ///
    /// The clock keeps running for a loop (see `since`), and
    /// `M2Skeleton::phase` wraps it, which for a one-shot that has just
    /// reached its end gives frame zero. The test `jump_pose_continuity`
    /// showed both effects: `JumpStart` ended mid-air and the fade sampled its
    /// crouch on the ground, a 2-yard bone move in one frame; `JumpLandRun`
    /// ended on the ground and the fade sampled its airborne first frame. This
    /// was reported as the model resetting mid-air and then replaying the
    /// second half of the jump after landing. `None` for a loop keeps the
    /// wolf's stride finishing.
    held: Option<f32>,
}

/// Display ids already resolved, and the tables they were resolved from.
///
/// The tables are four DBCs, parsed once on the first entity that needs them.
/// The per-id results are cached because forty identical wolves share one
/// display id and each would otherwise search the table.
#[derive(Resource, Default)]
pub struct DisplayCache {
    resolved: HashMap<(bool, u32), Option<Arc<DisplayModel>>>,
    /// Set once the tables have been requested, whether or not they parsed, so
    /// an archive chain that cannot supply them is not reopened per entity.
    tried: bool,
    tables: Option<Arc<vale_assets::tables::dbc::DisplayTables>>,
}

impl DisplayCache {
    /// Forget the tables and everything resolved from them.
    ///
    /// `GameAssets::forget_tables` does the same one layer down, but it does
    /// not reach this cache: this holds an `Arc` taken on the first entity of
    /// the session, so dropping the asset bank's copy leaves this pass reading
    /// the old parse. `forget_tables`' own doc warns about this case ("anything
    /// holding an `Arc` from before keeps what it holds, which is every pass
    /// that cached one"), and this is that pass.
    ///
    /// It is for a host that has made `DBFilesClient\` return different bytes:
    /// an edited `SpellVisualKit` naming a different model, an edited
    /// `CreatureDisplayInfo`. Without both calls the file is new on disk and
    /// in the archives' namespace, but the world keeps drawing what it read at
    /// startup.
    ///
    /// The per-id resolutions are cleared too. They are derived from the
    /// tables (display id to model path and skin), so keeping them across a
    /// change would leave them stale.
    ///
    /// It takes effect on the next request and is not free: the next entity
    /// to need a display re-reads and re-parses every table
    /// `DisplayTables::load` names, which is 73 reads and `Spell.dbc` twice.
    /// Call it only when something has changed.
    pub fn forget(&mut self) {
        self.tables = None;
        self.tried = false;
        self.resolved.clear();
    }
}

/// Everything this module does to an entity's model, in one name.
///
/// A pass that reads a dressing orders itself after this set rather than after
/// whichever system it currently depends on, so the ordering still holds when
/// a system is added to the middle of the chain.
/// [`crate::render::selection`]'s highlight uses it: it rewrites a part's
/// material, and a rebuild in the same frame would discard the rewrite.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntitySet;

pub struct EntityPlugin;

impl Plugin for EntityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DisplayCache>()
            .init_resource::<PendingImpacts>()
            // The hull budget's requested work against the work done, which
            // the scene tab's hull count cannot show. See [`solid::HullWork`].
            .init_resource::<solid::HullWork>()
            .add_message::<SheathRequest>()
            .add_systems(
                Update,
                (
                    light_entities,
                    // First, because both the rebuild below and the pose read
                    // the committed sheath state: a weapon drawn this frame must
                    // be in the hand this frame, on the key press that drew it.
                    sheath::commit,
                    rebuild_changed_models,
                    spawn_models,
                    spawn_attachments,
                    // Before `animate`, which poses the mount and reads the seat
                    // it gives the rider. A mount spawned here is ridden in the
                    // same frame, so a rider is never drawn standing inside
                    // its horse.
                    spawn_mounts,
                    // Before `spell_effects`, which spawns and expires the effect
                    // sets. This only requests one, so a glow requested here is
                    // hung in the same frame rather than the next.
                    effects::level_up,
                    // Beside it for the same reason: it acts on an edge and only
                    // requests, so a body that becomes lootable sparkles in that
                    // frame rather than the next.
                    effects::loot_art,
                    spell_effects,
                    persistent_areas,
                    // After `persistent_areas`, which inserts the `AreaRain`
                    // this reads, and before `animate_areas`, which poses what
                    // it spawns. An impact unposed on its first frame is a
                    // shard drawn in bind pose, thirty yards up, for one frame.
                    effects::retire_impacts,
                    effects::rain_impacts,
                    fallback_shapes,
                    // After `spawn_models`, which resolves display ids. This
                    // reads that result and does not resolve its own, so a game
                    // object becomes solid the frame after it is first drawn and
                    // the two passes cannot disagree about the table. See
                    // `solid`. The ordering it needs is against `DisplayCache`,
                    // not any transform: it takes the position from `Motion`,
                    // as `place_entities` does, and a game object does not move.
                    solid::solidify_objects,
                    animate,
                    // Between the pose, which starts an animation, and the
                    // reconcile, which takes the record that it did.
                    procedural::start_trails,
                    // Last, because its only input is the animation `animate`
                    // chose. The weapon moves one frame after the play starts.
                    // The 1.12.1 client has the same one-frame lag, because it
                    // moves the weapons after the pose is already chosen.
                    sheath::reconcile,
                    animate_areas,
                    // Last of all, because it rewrites materials that the rebuild
                    // above would otherwise discard. `render::selection`'s
                    // highlight is ordered after this whole set for the same
                    // reason. It reads only the aura list and the model's current
                    // tint, so nothing after it depends on it.
                    tint::paint_models,
                    // The fade, which changes the same models' materials in the
                    // same way and is last for the same reason. It runs after the
                    // tint, not in parallel, because they share the material
                    // pool: `with_opacity` and `with_model_tint` each copy the
                    // material as it currently is. A fixed order makes a
                    // stone-formed rogue in stealth one material carrying both,
                    // rather than two systems overwriting each other.
                    tint::fade_models,
                )
                    .chain()
                    // After the session has placed the entities: a joint is
                    // written as a world transform, so it must be composed
                    // against this frame's placement, not last frame's.
                    .after(crate::world::session::place_entities)
                    // After the player's input this frame: `AttackTarget()` and
                    // `ToggleSheath()` write a `SheathRequest`, and
                    // `sheath::commit` executes it. Without this ordering a drawn
                    // weapon appears on whichever frame Bevy schedules, the key
                    // press's own frame or the next.
                    //
                    // One system in `interface/` reads what this chain writes,
                    // and reads it one frame late by design. The mouse pick
                    // tests a unit's posed triangles (`interface::target::mesh_hit`)
                    // through the joints `animate` writes here, so it tests last
                    // frame's pose against this frame's ray. That is 0.07 yards
                    // on a running mob at 100 fps, less than the vertex spacing
                    // it tests against. The alternative is a cycle: the pick is
                    // chained after `arm_look` and the click, and this chain is
                    // after the input those read.
                    .after(crate::interface::GameSet)
                    .in_set(EntitySet),
            )
            // The impacts' own cleanup, which is not in that chain: an impact is
            // a root entity that the world teardown does not reach, so it is
            // dropped on the leaving-world message rather than with its owner.
            .add_systems(Update, effects::forget_impacts);
        // Two debug switches for what the world's population draws; see
        // [`crate::render::tuning`]. Hiding the model root hides its batches,
        // its attachments and its shadow blob. The blob has its own switch as
        // well, to tell whether a shadow under a character is the blob or a
        // decal in the art. Entities stream in and out with the session, so
        // both switches rely on the catch-up pass.
        app.add_systems(
            Update,
            (crate::render::tuning::switch::<EntityModel>(|tuning| {
                tuning.entities
            }),),
        );
    }
}

mod conform;
mod effects;
pub mod fallback;
mod mount;
/// Drop a model that no longer matches its entity, so the next frame builds
/// the right one.
///
/// A player's gear arrives a query round trip after the player appears.
/// `PLAYER_VISIBLE_ITEM` carries an item entry, and its appearance is in the
/// server's item template, so the first model is built from whatever had
/// arrived by then, usually nothing. Without a rebuild, the character keeps
/// that first model and appears without equipment for the whole session.
///
/// It rebuilds rather than redresses because the geosets change too, so the
/// batch list differs, not just its materials.
///
/// Filtered on `Changed<WorldEntity>`, which works only because
/// [`crate::world::session::poll_world`] writes the component only when it
/// changes. Only a change to the entity can make a model stop matching it, so
/// an unchanged entity never needs a rebuild. Without the filter, every
/// modelled entity was compared every frame at 100+ fps against a snapshot
/// that changes 40 times a second; with it, a city of standing guards costs
/// nothing here.
mod pose;
mod procedural;
mod sheath;
pub(crate) mod solid;
mod spawn;
pub(crate) mod transport;
mod tint;
mod worn;

// Re-exported flat, so every `world::entities::Foo` path elsewhere still
// resolves: the submodules are internal organisation, not a new API.
pub(crate) use effects::*;
use fallback::*;
use mount::spawn_mounts;
pub use mount::Mount;
pub(crate) use pose::*;
// `Sheath` is public and `SheathRequest` is not. `Sheath` is part of every
// unit: `spawn::spawn_models` requires it in its query rather than as an
// option, so anything that builds a unit (a test, the crowd, a host with no
// server) must add one, or the unit is never drawn and no error is reported.
// `SheathRequest` is a message this crate sends to itself.
pub use sheath::Sheath;
pub(crate) use sheath::SheathRequest;
use spawn::*;
pub(crate) use worn::hang_nested;
use worn::*;
#[cfg(test)]
mod tests;
