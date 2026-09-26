//! The entity pass: a model for every unit, player and game object the server
//! describes, skinned and animated from the model's own bones.
//!
//! ```text
//! mod.rs       the pass itself: what exists, what it is drawn as, and when
//! spawn.rs     …turning one WorldEntity into an entity and its children
//! pose.rs      …and posing it: which clip, how far into it, and the joints
//! worn.rs      what is on it — the geosets, the composed skin, the gear
//! sheath.rs    …and whether the weapons are out
//! mount.rs     …and what it is riding, which is a second model on one motion
//! conform.rs   …and the ground under both, for the models whose header asks
//! solid.rs     the half that is walked *into* rather than drawn
//! transport.rs …and the few that move it: where an elevator is, this frame
//! effects.rs   what is happening to it: the spell art and the auras' own
//! tint.rs      …and what colour that makes it
//! fallback.rs  …and what to draw when the tables resolve to nothing
//! ```
//!
//! This is the last of the coloured boxes. A `WorldEntity` carries a
//! `DISPLAYID`, the display tables turn that into a model path and the skins to
//! dress it in, and `models.rs` turns *that* into meshes and materials — the
//! same ones the doodads use, because a creature and a tree are both M2s and
//! nothing about the geometry differs. What this adds is the skeleton.
//!
//! ## `M2Skeleton::pose` stays the composer, and Bevy only skins
//!
//! Bevy has a perfectly good animation system and none of it is used here. The
//! engine is asked for exactly one thing: `SkinnedMesh`, which blends four joint
//! matrices per vertex on the GPU and packs every skinned mesh's joints into
//! shared storage buffers — which is what removes the WebGL renderer's 38-bone
//! cliff, where a wolf's 64 bones did not fit in 128 vertex uniform vectors and
//! `Models.initSkinning` fell back to *no animation*.
//!
//! Everything above that stays here, because three properties of a vanilla M2
//! have no expression in Bevy's hierarchy:
//!
//! * **the inverse bindposes are all identity.** M2 vertices are already in
//!   model space and `M2Skeleton::pose` puts each bone's pivot back itself
//!   (`local = T(pivot + translation) * R * S * T(-pivot)`), so there is nothing
//!   for an inverse bindpose to undo. Supplying one would apply the pivot twice.
//! * **the joints are flat, not parented to each other.** `pose` resolves the
//!   hierarchy itself — by "whoever's parent is done" rather than by index,
//!   because nothing in the format promises parents come first — and then does
//!   two things transform propagation cannot: spherical billboards (bone flag
//!   0x8 replaces the bone's rotation with the camera's axes, expressed in the
//!   *model's* space) and global-sequence tracks, which ignore the playing
//!   animation and loop on wall-clock time. 357 of the game's 411 creature
//!   models drive at least one track that way, so it is not an edge case.
//! * **a joint is written as a `GlobalTransform`, not a `Transform`.** A composed
//!   bone matrix is not obliged to be a translation, a rotation and a scale — a
//!   bone with non-uniform scale and a rotated child shears — and
//!   `Transform::from_matrix` would decompose that away and pose the model
//!   *plausibly* wrongly, which is the failure mode this project keeps paying
//!   for. Writing the affine also skips propagating five thousand joint entities
//!   a frame. They are still *children* of the entity, so despawning it takes
//!   them with it; they simply have no `Transform` for propagation to find, and
//!   there is a test pinning that Bevy leaves them alone.
//!
//! ## Which animation, and when
//!
//! The server never sends an animation id. It says whether a thing is moving,
//! how fast, and **which flags it is moving under** — `Entity::is_moving`,
//! `Entity::ground_speed` and `Entity::move_flags`, the very flags or spline the
//! client's own dead reckoning is advancing the entity by — and the client picks
//! a gait from the three. Which gaits exist is the art's business and it is not
//! symmetric: the ground has Walk, Run and `Walkbackwards` and **no sideways
//! gait at all**, where the water has all four. A strafe on land is drawn by
//! turning the *body* rather than by playing a different clip — see
//! [`crate::world::facing`], which is where the whole of it lives.
//! **Stated, not differenced:** the
//! predecessor differenced two interpolated positions, which reads zero for the
//! last frames of most snapshot intervals and flipped Run to Stand twenty times
//! a second. [`Playback::advance`] carries across the two corollaries that cost
//! the most to find — a restart is triggered by a change of *sequence*, not of
//! intent, and a switch cross-fades over 150 ms.

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
/// The real client blends over about this much. `M2Sequence::flags` carries a
/// per-sequence hint that vanilla models barely populate, so one number for
/// everything is the honest version of it.
const FADE_SECS: f32 = 0.150;

/// The slowest an entity can be going and still be walking rather than running.
///
/// Vanilla's baseline walk is 2.5 yards a second and its run 7.0, so the gap is
/// wide and the threshold is not delicate. It earns its keep on creatures, which
/// the server sends along a patrol spline at a walking pace and into a fight at
/// a running one.
const RUN_SPEED: f32 = 4.5;

/// Below this, "moving" is the server turning a creature on the spot.
///
/// That arrives as a spline whose destination the creature is already standing
/// on, so it is moving at a few hundredths of a yard a second. It cannot
/// flicker — a spline's speed is fixed for its whole duration.
const MOVING_FLOOR: f32 = 0.1;

/// How many entities to give models to in one frame. A login burst is ~200 at
/// once and each costs a mesh walk per batch.
const SPAWN_BUDGET: usize = 16;

/// The model an entity has been given, and the parts it was drawn as.
///
/// **Requires a [`conform::Stance`]**, rather than inserting one beside it:
/// every modelled entity has a ground stance whether or not its own model leans
/// — a rider whose model is level acquires a leaning horse a second later — and
/// a required component is the one way to say that which the tests cannot
/// forget.
#[derive(Component)]
#[require(conform::Stance, pose::Posed)]
pub struct EntityModel {
    /// What it was built for. A `DISPLAYID` can change under an entity — a
    /// shapeshift, a mount — and that has to rebuild rather than redress.
    pub display_id: u32,
    /// `modelScale * displayScale` from the DBCs, used when the entity's own
    /// `OBJECT_FIELD_SCALE_X` never arrived. The two are **not** multiplied:
    /// `Unit::GetScaleForDisplayId` returns the product and the field is it.
    pub dbc_scale: f32,
    /// The joint entities, in bone order, with the identity joint last.
    pub joints: Vec<Entity>,
    /// The appearance and wardrobe this was dressed from, for a player.
    ///
    /// **Equipment is why this has to be recorded.** A player's gear is a
    /// server answer and arrives a query interval *after* the entity does, so a
    /// model built the moment the entity appears is built undressed — and
    /// without something to compare against, it stays that way for the rest of
    /// the session. Nothing about that reads as a bug: the character is simply
    /// wearing their underwear.
    look: Option<CharacterLook>,
    /// The models the wearer's equipment hangs off them, still to be spawned.
    ///
    /// Drained as each one's M2 arrives — an attached model is a second load
    /// behind the character's, so a fully geared player appears and *then*
    /// grows pauldrons, exactly as they appear and then get dressed.
    wanted: Vec<AttachedModel>,
    /// The ones now standing on their bones.
    attached: Vec<AttachedPart>,
    /// **The spell effect models this unit is wearing right now**, in the three
    /// sets that answer three different questions.
    ///
    /// Held apart from `attached` even though they ride the same bones and are
    /// placed by the same code, because their *lifetime* is nothing like a
    /// pauldron's: a pauldron belongs to the model and is rebuilt with it, where
    /// a spell effect has to be taken off again — and each of these three is
    /// taken off for a different reason. See [`spell_effects`].
    ///
    /// What this unit is **doing**: the wind-up held while the cast bar runs, or
    /// the release. Ends on a clock.
    cast: EffectSet,
    /// What was **done to** it: the `impactKit`, hung when a spell lands. A
    /// separate set because a unit can be casting and be hit in the same
    /// moment, and one list would have the blow cancel the caster's own glow.
    impact: EffectSet,
    /// What is **true of** it: the `stateKit`s of the auras it is carrying.
    /// Ends when the aura does, which is a condition and not a clock.
    state: EffectSet,
    /// What just **happened to** it that no spell describes — today exactly one
    /// thing, the level-up glow. A fourth set rather than a corner of one of the
    /// three above because its lifetime is unlike all of them: it is not a cast,
    /// nothing did it to the unit, and it is not true of the unit. Folding it
    /// into `cast` would mean the next spell pressed cancelled it. See
    /// [`effects::level_up`].
    milestone: EffectSet,
    /// …and what is **available on** it: the sparkle over a body with loot
    /// still on it. A fifth set for the same reason the fourth exists — its
    /// lifetime is unlike all four. It is not a cast, nothing did it to the
    /// unit, no aura states it, and unlike the milestone it ends on a
    /// **condition** rather than a clock: `UNIT_DYNFLAG_LOOTABLE` going out.
    /// See [`effects::loot_art`].
    loot: EffectSet,
    /// …and what the **server said outright** — `SMSG_PLAY_SPELL_VISUAL` and
    /// `SMSG_PLAY_SPELL_IMPACT`, whose body is a `SpellVisualKit` id and no
    /// spell at all.
    ///
    /// A sixth set, on the same terms as the fourth and the fifth: its lifetime
    /// is unlike any of the five. It is not a cast, no aura states it, and
    /// nothing in the world implies it — the packet *is* the event. Folding it
    /// into `cast` would let the next spell pressed cancel a character's dinner.
    ///
    /// **One set for both opcodes**, which they are not on the wire: they differ
    /// in whether the guid names the unit that acted or the unit acted upon, and
    /// by the time either reaches this entity that question has already been
    /// answered — the guid is what selected it. On *this* unit they are the same
    /// statement. See [`effects::pushed_kits`].
    pushed: EffectSet,
    /// …and what a **host asked for outright**: a model and a point and no
    /// spell, kit or packet behind it, on for as long as the host leaves it
    /// there. See [`Self::hang`]. Empty in every session, because nothing on
    /// the wire fills it; a seventh set rather than a corner of the sixth
    /// because [`Self::pushed`] is restarted by the next packet and this must
    /// not be.
    hung: EffectSet,
    // The sets above are **carried across a re-dressing** rather than rebuilt
    // with it — see [`CarriedEffects`], which is where that is done and why.
    /// The aura ids [`Self::state`] was built from — the ones that **state a
    /// visual**, in slot order, which is what the next frame's
    /// `UNIT_FIELD_AURA` is filtered and compared against. A buff that expires
    /// and a buff that is applied both read as this list disagreeing with the
    /// wire; a debuff ticking on and off does not, because it never entered it.
    state_auras: Vec<u32>,
    /// **The colour this model's own batches are currently painted through**,
    /// or `None` for the whole world most of the time — see [`tint`].
    ///
    /// Deliberately on the *model* rather than beside `state_auras` in
    /// [`CarriedEffects`], and the difference is the whole of why it works: this
    /// records what the **materials** are wearing, not what the auras want. A
    /// re-dressing builds fresh materials with no paint on them and takes this
    /// field with the model it belonged to, so the next frame's compare finds
    /// `None` against a stone-formed dwarf and puts the colour back on.
    painted: Option<[u8; 3]>,
    /// **How solid this model's own batches are currently drawn**, or `None` for
    /// the whole world nearly all of the time — see [`tint::fade_models`].
    ///
    /// On the model for exactly [`Self::painted`]'s reason and it is the same
    /// mechanism: this records what the *materials* are wearing rather than what
    /// the wire says, so a re-dressing — which builds fresh, solid materials —
    /// carries this field away with the model it belonged to and the next
    /// frame's compare puts the fade back on.
    faded: Option<u8>,
    /// The entity's **own** drawn parts whose colour is animated, and the
    /// model's tracks. Almost always empty — a wolf does not fade — but the
    /// mechanism is the same one the effects use, so it is the same pair of
    /// fields rather than a second one. See [`Tinted`].
    tinted: Vec<Tinted>,
    tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
    /// `None` until this entity has been *looked at* once — see
    /// [`spell_effects`], where the first look adopts the counters without
    /// acting on them. Anything the caster did before it was modelled is
    /// history, and a creature that walks into view mid-cast must not be handed
    /// the glow of a cast that began before it existed.
    ///
    /// `(begun, released, cancelled, delayed)` — the last two for the same
    /// reason the first two are here, since a cast being taken off and a cast
    /// being pushed back are both edges and a first look must not act on either.
    effects_for: Option<effects::CastCounters>,
    /// …and the same for the **wire's** release counter, which is what the
    /// impact is hung off — see [`WorldEntity::casts_landed`]. `None` until the
    /// first look, for the reason above: a creature that walks into view having
    /// just cast something must not burst it on whatever it hit.
    landed_for: Option<u32>,
    /// …and the two pushed-kit counters, on the same terms as both: a unit that
    /// walks into view having been sent kit 406 must not be handed somebody
    /// else's dinner. See [`WorldEntity::spell_visuals`].
    pushed_for: Option<(u32, u32)>,
    /// The wearer's own attachment points, from its M2 — one `Arc` clone per
    /// entity, shared with every other wearer of the same model.
    points: Arc<Vec<M2Attachment>>,
    /// The model's sound cues, per sequence — see `sound::cues`.
    pub cues: Arc<vale_assets::world::m2::SoundCues>,
    /// How far above the entity's own origin its **head** is, in **model**
    /// yards — so a caller multiplies by the entity's scale. See
    /// [`head_height`].
    ///
    /// One caller: the missile, which throws at a point in the middle of a
    /// body. **Not [`Self::anchor`]**, which is where a *camera* looks and is
    /// a quadruped's muzzle.
    pub head: f32,
    /// How far above the entity's own origin its **name** hangs, in **model**
    /// yards — attachment **18**, `PlayerName`, which is the one point in the
    /// enum that exists for nothing else. See
    /// [`vale_assets::look::unitname::ANCHOR_ATTACHMENT`], and
    /// [`name_height`] for what a model without the point falls back to.
    ///
    /// Beside [`Self::head`] rather than derived from it because the two are
    /// different points on the same skull: the helm is the top of it and this
    /// is where the client hangs a name, which on a human is most of a foot
    /// higher.
    pub name_anchor: f32,
    /// How far above the entity's own origin the **camera anchor** is, in
    /// **model** yards — so a caller multiplies by the entity's scale. See
    /// [`vale_assets::look::anchor`], whose rule is the client's own.
    ///
    /// Attachment point **17**, the neck, not the helm point above it: the
    /// difference is 0.13 yards on a human and it is the whole of the "the
    /// focal point is a little too high" report.
    pub anchor: f32,
    /// …and where that anchor sits once the `Mount` clip has folded the body
    /// onto a saddle, or `None` for a model with no such clip — which is
    /// everything in the world that is not a character.
    ///
    /// **Static, and that is the point.** Read off the live pose it rises and
    /// falls with a galloping horse's spine and takes the whole view with it.
    pub mounted_anchor: Option<f32>,
    /// **The model's own header sphere**, in model yards — the fallback every
    /// branch of [`Self::pick_sphere`] ends in, and the whole answer for
    /// anything with no skeleton to ask.
    ///
    /// Kept here for the reason the two radii below it are: the `ModelAssets`
    /// it came off is not held past the build.
    model_sphere: vale_assets::look::pick::Sphere,
    /// **The drawn triangles the pointer is actually tested against** — stage
    /// two, in model space, shared per model path.
    ///
    /// See [`vale_assets::look::pick::hit_mesh`], which is the walk, and
    /// [`crate::game::combat::target`], which skins it with [`Self::joints`] so that the
    /// silhouette a click tests is the one on the screen rather than a second
    /// pose computed beside it.
    pub pick: Arc<vale_assets::look::pick::PickMesh>,
    /// The model's particle emitters and ribbon trails — a wisp's dust, an
    /// imp's flames, an elemental's streamers. **Root** entities, because both
    /// kinds build their mesh in world space, so a rebuild has to despawn them
    /// by this list; an entity streaming out leaves them to the two passes'
    /// own retirement sweeps.
    roots: Vec<Entity>,
    /// How wide to draw that blob, in model yards — the model's own declared
    /// half-extent. Kept because the `ModelAssets` it came from is not.
    ///
    /// **Public**, because the blob is no longer a child of this entity: every
    /// blob in the world is one mesh built by [`crate::render::shadows`], which
    /// reads the footprint from here. See that module for why.
    pub shadow_radius: f32,
    /// A sphere in model yards that covers the model's own declared box from
    /// the entity's origin — centre offset plus half-diagonal, so it encloses
    /// the same volume the part meshes are frustum-culled by. This is what
    /// [`animate`]'s gate tests: the two must agree, because a part the camera
    /// can draw must never ride a pose the gate declined to compute.
    cull_radius: f32,
    /// **Whether this model leans with the ground under it**, off the M2
    /// header's `GlobalModelFlags` — see [`vale_assets::look::conform`]. Kept here
    /// for the same reason the two radii above it are: the `ModelAssets` it came
    /// off is not held past the build.
    ///
    /// `Level` for every character model in the game, which is why a player on
    /// foot stays upright on a hillside — that is the reference's own behaviour
    /// and not a gap.
    pub conform: vale_assets::look::conform::Conform,
    /// The room this was dressed for — **always `None` now**, and kept so the
    /// retag path stays one comparison.
    ///
    /// A unit used to be re-dressed room-lit on crossing a door and retagged
    /// with the room's colour between rooms. The Direct3D trace of the
    /// reference (rendering facts, *The reference client traced*) shows it
    /// does neither: every model inside the inn is lit by the same sun in view
    /// space at the 0.5 scale, the zone's own fill, and the room's point
    /// lights — the room's baked colour is for its walls and its furniture,
    /// not for who walks in. So a unit keeps its sun-lit dressing everywhere
    /// and what changes at the door is [`SunScale`], which blends.
    room: Option<RoomLight>,
    /// The sun scale its batches were tagged with — see [`SunScale`]. Kept
    /// so that a change of it is a retag rather than a rebuild, exactly as
    /// the room's colour is.
    sun: f32,
    /// What was in the entity's hands when this was built, and whether it was
    /// drawn.
    ///
    /// A creature used to be immutable once modelled — "nothing but the display
    /// id can change what it is" — and that stopped being true the moment
    /// weapons arrived: a guard who draws his mace on entering combat changes
    /// the models hanging off him without changing anything else about himself.
    ///
    /// **The whole [`Weapon`] rather than its display id**, because the sheath
    /// type is what decides which point a put-away weapon hangs from — and a
    /// player's arrives a query round trip after the display id does, so a
    /// comparison on the id alone would leave the first model built on a
    /// half-answered item and never rebuild it.
    hands: ([Weapon; 3], u8),
}

impl EntityModel {
    /// **Stage one of the mouse pick**: the sphere the pointer's ray has to
    /// cross before this unit's triangles are worth walking, in **model** yards.
    ///
    /// `playing` is the sequence the unit is in *this frame* — [`Playback::clip`]
    /// — because that is what the reference reads and it is the whole of the
    /// difference this makes: the model's own header box is a union over every
    /// animation and every emitter it has, so a wisp's is a 12.8-yard cube round
    /// a creature a yard across. See [`vale_assets::look::pick`], which is the rule
    /// and the addresses, and `vale pick`, which measures it: the standing
    /// sphere averages **57%** of the header sphere across the bestiary.
    pub fn pick_sphere(&self, playing: Option<&M2Sequence>) -> vale_assets::look::pick::Sphere {
        vale_assets::look::pick::sequence_sphere(self.model_sphere, playing)
    }

    /// **Hang models on this unit with no spell behind them**, until
    /// [`Self::unhang`]. Each is filtered against the points the unit's own M2
    /// carries, loaded and hung exactly as a kit's models are, and posed on
    /// the bone it names — so what a host hangs here is drawn as the game
    /// would draw the same model in the same slot.
    ///
    /// What is already hanging comes down first: the set is a statement of
    /// what should be there, not an addition to it.
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
/// Three of these hang off an [`EntityModel`] — the cast, the impact and the
/// aura state — and the reason they are one type is that the *mechanism* is
/// identical and the *policy* is not. Every set loads asynchronously, filters
/// against the wearer's own attachment points, hangs through [`hang_model`] and
/// is despawned by the root; what differs is only what sets `until`, and that
/// is the caller's business. Before this existed there was one set, and adding
/// the second and third by copying its four fields and its three code paths is
/// exactly how a fix lands on one and not the others.
#[derive(Default)]
struct EffectSet {
    /// Asked for and still loading — drained as each model's M2 lands, which is
    /// a second load behind the wearer's exactly as an attachment is.
    wanted: Vec<vale_assets::tables::spell::KitEffect>,
    /// Built and hanging on their bones.
    parts: Vec<AttachedPart>,
    /// When this set comes off, in `Time::elapsed_secs`.
    ///
    /// `f32::INFINITY` for the aura state, which ends on a condition rather
    /// than on a clock — and which is why this is a float rather than an
    /// `Option<f32>`: the comparison is the same either way and a never-expiring
    /// set is not a special case in the code, only in the value.
    until: f32,
    /// **Whether the art itself may push [`Self::until`] out.**
    ///
    /// A release and an impact are over when their models have finished
    /// playing, and only the models know when that is: `IceArmor_Low_Head`'s
    /// one-shot runs **3,000 ms** and `ArcaneIntellect_Impact_Base`'s 1,900,
    /// where the flat fallback every set used to take is 1,500 — so both were
    /// cut off part-way down their own fade, which is the "it appears and then
    /// abruptly disappears" report exactly. The clip is raised in
    /// [`Self::spawn`], as each model lands, because nothing knows the length
    /// until the M2 is read.
    ///
    /// **False for a wind-up**, which ends when the *cast* does whatever its
    /// glow was authored to run for — an interrupted Fireball must not keep its
    /// hands lit — and false for the aura state and the loot sparkle, which end
    /// on a condition.
    clip_bound: bool,
}

/// **The three effect sets, held on the entity across a re-dressing.**
///
/// A spell effect is not part of the dressing and must not be rebuilt with it,
/// and getting that wrong is not a corner case: `SpellCastOmni` carries
/// `AnimationData`'s `STOW_HANDS_BUSY`, so **every cast stows the weapon**, which
/// moves `Sheath`, which fails [`EntityModel::matches`]' `hands` test, which
/// tears the whole model down — three frames after the effect was hung and with
/// the glow still growing. It takes every other effect on the unit with it: an
/// aura's state glow, a burst landing at the same moment, the ground decals
/// owned by all three.
///
/// **It was not, on its own, the "spell effects get cut off" report**, and that
/// attribution is retracted — the effects went on failing after this was fixed,
/// on the swallowed hit list (see `Entity::casts_landed`). What is left here is
/// a real teardown that would still cost an effect its life the moment anything
/// else rebuilds the model, which is a door away at all times.
///
/// So [`super::spawn::rebuild_changed_models`] lifts the sets off the dying
/// model, leaves their roots standing, and [`super::spawn::spawn_models`] puts
/// them into the new one.
///
/// **Only when the display id is unchanged**, which is the same first test
/// `matches` makes. An `AttachedPart::bone` indexes the *wearer's* skeleton and
/// the roots are posed against it, so carrying a glow onto a different model
/// would hang it off whatever bone happened to share the index. A re-dressing
/// keeps the skeleton; a shapeshift does not, and there the sets go as before.
#[derive(Component, Default)]
pub(super) struct CarriedEffects {
    cast: EffectSet,
    impact: EffectSet,
    state: EffectSet,
    /// …and the level-up glow, which is the longest-lived one-shot of the four
    /// and therefore the likeliest to be standing when a rebuild happens.
    milestone: EffectSet,
    /// …and the loot sparkle, which outlives all of them: it is on for as long
    /// as the body has anything on it, so a rebuild that dropped it would leave
    /// a lootable corpse dark for the rest of its life. Nothing re-arms it —
    /// [`effects::loot_art`] is edge-driven and the edge has already passed.
    loot: EffectSet,
    /// …and the server's own, which nothing re-arms either: the packet that
    /// asked for it has been and gone.
    pushed: EffectSet,
    /// …and the host's own, for the same reason: a re-dressing is not the
    /// host's doing and must not take down what it hung.
    hung: EffectSet,
    /// Carried with [`Self::state`] or the two desync: the new model would find
    /// its aura list empty, disagree with the wire on the first frame, and clear
    /// and re-hang every buff glow on the unit — restarting the clock of each,
    /// which is exactly what `spell_effects` filters the aura set to avoid.
    state_auras: Vec<u32>,
    /// …and the cast counters already accounted for, so the rebuild is not read
    /// as a first look. `None` here would swallow a cast that began during the
    /// two frames the model was gone.
    effects_for: Option<effects::CastCounters>,
    /// …and the wire's own release counter beside them, for the same reason:
    /// a rebuild between the press and `SMSG_SPELL_GO` would otherwise swallow
    /// the impact, which is exactly the window a cast's stow opens.
    landed_for: Option<u32>,
    /// …and the two pushed-kit counters, on the same terms: a rebuild that
    /// forgot them would be read as a first look, and a first look deliberately
    /// acts on nothing.
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
    /// **Filtered against the points the wearer's own M2 carries**, the same
    /// filter an item attachment gets: a model with no head point cannot wear a
    /// crown of frost, and loading the effect anyway would put it nowhere.
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

    /// Ask for `models`, to come off when **the art has finished playing** — or
    /// at `floor`, whichever is later.
    ///
    /// The deadline only ever moves *out*, never in. A short clip keeps the
    /// floor rather than shortening to itself: the reference frees the effect
    /// on the clip's end and this client's emitters go on spraying past it, so
    /// taking the clip as an upper bound as well would trade the bug this fixes
    /// for a burst that vanishes early. See [`Self::clip_bound`].
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

    /// **Fold one landed model's own end into the deadline**, now that its file
    /// has been read.
    ///
    /// The set was armed on a flat fallback because nothing knew the length
    /// until this moment, and the models of one set land on different frames —
    /// so this only ever moves the deadline *out*, and the set comes off when
    /// the longest-running of its models has finished. A set armed with
    /// [`Self::want`] rather than [`Self::want_until_played`] is untouched: a
    /// wind-up ends when the cast does.
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
            // **No room**, unlike an item attachment: a spell effect is an
            // `unlit` additive glow, so the interior branch would change
            // nothing about it and a second dressing per building would be
            // paid for every effect model in the game.
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
                        // **The base point is the floor**, and it is the only
                        // one of the fourteen an effect can hang from that is:
                        // `attach::BASE` (19) is the caster's feet, which is
                        // where every painted-on-the-ground spell effect in the
                        // game is anchored. 8,071 spells put a model there
                        // (`vale spell`).
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
    /// Index into the *wearer's* skeleton. Bounds-checked when it is written,
    /// because a model may name a bone it does not have and the pose would then
    /// read some other bone's matrix rather than failing.
    bone: usize,
    /// The attachment point in the bone's own frame, in the model's axes.
    offset: [f32; 3],
    /// **Whether this part lies on the floor**, which is `attach::BASE` and
    /// only that — the roots, the ice, the net, the ground rings.
    ///
    /// Two things read it and they are the same claim seen twice: the ground
    /// **decal** path in [`super::effects::hang_model`], which draws a flat
    /// quad in world space rather than as a child; and [`super::pose`], which
    /// takes the wearer's *rotation* back out of the part's frame. A thing on
    /// the ground does not turn when the thing standing on it turns — a rooted
    /// player spinning on the spot dragged its roots round with it, which is
    /// what made the effect read as painted on the character rather than on
    /// the floor.
    pub grounded: bool,
    /// A multiplier on the attached model *only*.
    ///
    /// One for a pauldron — a helm is authored to fit the head it hangs on — and
    /// `SpellVisualEffectName`'s own scale for a spell effect, where the same
    /// glow model is reused at a range of sizes and the table is the only thing
    /// that says which. **Public** because a missile hangs off no bone and
    /// composes its own frame from it — see `missiles.rs`.
    pub scale: f32,
    /// The one child of the wearer that carries the attachment's frame.
    /// [`animate`] writes its `Transform`; the drawn parts and the model's own
    /// joints hang under it, so despawning it takes the whole attachment —
    /// and its emitters follow through `retire_emitters`, which despawns an
    /// emitter whose owner is gone.
    root: Entity,
    /// The attached model's **own** joints (bone order, identity last), when
    /// it has a skeleton. An attached model is not rigid: a torch's glow plane
    /// rides a billboarded bone of the torch's own skeleton, and a spell
    /// effect's growth is its own bone animation.
    joints: Vec<Entity>,
    skeleton: Option<Arc<M2Skeleton>>,
    /// The resolved index of the model's own Stand (id 0), or its first
    /// non-empty sequence — the one an attached model plays.
    sequence: usize,
    /// Whether that sequence loops (bit 0 of its flags clear — a read of the
    /// 5875 loader). A non-looping effect holds its **last** frame,
    /// which for a one-shot like Arcane Explosion's dome is the collapsed one.
    loops: bool,
    /// When it went on, `Time::elapsed_secs` — the sequence clock's zero.
    since: f32,
    /// The drawn parts whose colour is animated, and which tracks animate it.
    /// Empty for a pauldron and for most of the world; this is the mechanism
    /// that makes a spell effect **end**. See [`Tinted`].
    tinted: Vec<Tinted>,
    /// The model's own colour and transparency tracks, shared with every other
    /// wearer of it. `None` when nothing on it fades.
    tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
}

/// One drawn part whose `MeshTag` is its animated colour rather than its room.
///
/// **The tag is the only per-instance channel there is**, so a fade cannot be a
/// material — see [`crate::render::models::tint_tag`]. What this pairs is the part
/// entity with the batch's own two track indices, because two batches of one
/// model fade on different curves: a fireball's core and its halo are separate
/// `M2Color` rows.
struct Tinted {
    part: Entity,
    tint: vale_assets::world::m2::BatchTint,
}

impl AttachedPart {
    /// The entity carrying this attachment's frame, so a caller can write it.
    pub(crate) fn root(&self) -> Entity {
        self.root
    }

    /// **Where this attachment sits in the wearer's own space**, given the
    /// wearer's pose: the carrying bone's matrix, the point in that bone's
    /// frame, and the attached model's own scale, in that order.
    ///
    /// `None` when the wearer's skeleton has no such bone — which a model may
    /// name and not have, and reading past the pose would give some other bone's
    /// matrix rather than failing.
    ///
    /// A method rather than three fields read at the call site because there are
    /// now two callers composing this product — the entity pose and the
    /// character standing on the character-select plinth — and they must not
    /// drift.
    pub(crate) fn local(&self, pose: &[[f32; 12]]) -> Option<Mat4> {
        let bone = pose.get(self.bone)?;
        Some(
            crate::render::axes::pose_to_bevy(bone)
                * Mat4::from_translation(crate::render::axes::to_bevy(self.offset))
                * Mat4::from_scale(Vec3::splat(self.scale)),
        )
    }

    /// **How long this part's clip runs**, in seconds — `None` for one that
    /// loops or states no sequence at all.
    ///
    /// This is what says when a one-shot spell effect is *over*: the reference
    /// frees a shard on its model's own animation-finished callback, so the
    /// clip **is** the lifetime and every flat constant
    /// around it is a fallback for the models that state none. A looping clip
    /// deliberately answers `None` rather than its own length — `ChargeTrail`'s
    /// take is 334 ms and it is meant to run for as long as the charge does, so
    /// reading a loop as a deadline would end an effect after one turn of it.
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

    /// An attachment with no geometry, no skeleton and nothing to fade — the
    /// shape [`animate_attachment`] does nothing with.
    ///
    /// Exists for the missile flight test, which is about a projectile's
    /// *arithmetic* and would otherwise need an archive open to build one:
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
        }
    }
}

impl EntityModel {
    /// Whether this model still describes the entity, or is a frame behind a
    /// change that has to be rebuilt rather than nudged.
    fn matches(&self, world: &WorldEntity, room: Option<RoomLight>) -> bool {
        if world.display_id != Some(self.display_id) {
            return false;
        }
        // Walking through a door changes nothing about the entity and
        // everything about what lights it — but only the *branch* is in the
        // materials. A change of room colour is a retag, not a rebuild.
        if self.room.is_some() != room.is_some() {
            return false;
        }
        // **What is in the hands is deliberately *not* here**, and it used to
        // be. Drawing a weapon is a change of models — but only of *attached*
        // ones: `vale_assets::look::dress` reads `weapons` and `sheath_state` to
        // build the attachment list and nothing else, because a weapon has no
        // geoset and no bearing on the composed skin. So the body's batches, its
        // joints and its dressing are all unaffected, and rebuilding them is
        // work with no output.
        //
        // It was not merely wasteful. `SpellCastOmni` carries
        // `AnimationData`'s `STOW_HANDS_BUSY`, so **every cast stows the
        // weapon** — which failed this test a frame or two after the cast's own
        // glow was hung, and the teardown took the glow with it. The same
        // rebuild fired again on the next Ready stance for anyone also swinging:
        // twice per cast, per caster, in the middle of a fight.
        //
        // [`super::spawn::rebuild_changed_models`] re-hangs the wardrobe in
        // place instead — the same shape as the room retag above.
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

/// **What room the entity is standing in**, or `None` for out of doors.
///
/// Written by [`light_entities`] once a frame and read by the dressing, which is
/// the whole of the interior-lighting join on this side. A component rather than
/// a field on `WorldEntity` because it is not the server's answer to anything —
/// it is a geometric fact about where the entity is standing, and the object
/// manager has never heard of a building.
#[derive(Component, Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Indoors(pub Option<RoomLight>);

/// **The per-instance multiplier on the sun term** this entity's batches
/// carry — [`crate::render::models::sun_scale`]'s 2.5 on lit ground and 0.5 on
/// ground that carries a baked `MCSH` shadow, indoors or out.
///
/// A doodad takes this by the bit under its origin at tile load; a unit moves,
/// so it is sampled here, once a frame, off the same terrain cache the mover
/// stands on. **Units take it as the doodads do**: a Direct3D trace of the
/// reference measured the character's own batches at `Diffuse = 2.5 × band 0`
/// outdoors and the models around it at 0.5 inside an inn, all of them with
/// the zone's own fill and the room's lamps and none of them lit by the room
/// (rendering facts, *The reference client traced*). Which condition puts the
/// reference's model at 0.5 is not separated by that trace; the ground's
/// shadow is this file's reading, and the inn's floor is in the inn's own
/// baked shadow.
///
/// **And it is blended, not switched.** The bit is per 0.5-yard texel, so a
/// unit crossing a shadow's edge would otherwise flip between 2.5 and 0.5 on
/// one step — the reported "it flips on". The trace holds one model mid-way,
/// at 0.81, which says the reference eases it; how fast is not measured, and
/// [`SUN_SCALE_RATE`] is this client's number.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub struct SunScale {
    /// What the batches are tagged with this frame.
    now: f32,
    /// Where it is heading: the bit under the feet.
    target: f32,
}

/// How fast the sun scale moves toward its target, per second — the whole
/// 2.5..0.5 swing in a second. See [`SunScale`].
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
/// **Not at the feet**, which is where the position is: a room's box is its own
/// geometry, so its top face *is* the outside of the ceiling — and a character
/// standing on a flat roof stands exactly on that face, which counts as inside.
/// A yard up is still well within any room a character can enter and clear of
/// every roof they can stand on.
const INDOOR_PROBE: f32 = 1.0;

/// Decide which entities are standing in a room, and in whose.
///
/// Runs after [`crate::world::session::place_entities`] with the rest of this pass, so
/// the point tested is this frame's. The `Interior` components are the placed
/// buildings of the loaded 3x3 — a few dozen — and each rejects on its own box
/// before walking its rooms, so a city's three hundred is only paid for by an
/// entity standing in the city.
///
/// **Written only when it changes**, which is what keeps `Changed<Indoors>` a
/// meaningful filter: an entity standing still is not re-dressed sixty times a
/// second.
fn light_entities(
    mut commands: Commands,
    entities: Query<(Entity, &Transform, Option<&Indoors>, Option<&SunScale>), With<WorldEntity>>,
    interiors: Query<&crate::render::wmos::Interior>,
    // The terrain under the entity, for the shadow bit — see [`SunScale`].
    // Optional because the interiority half needs no session and is tested
    // without one; with no terrain to ask, the ground is lit.
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

/// One bone of one entity. Carries a `GlobalTransform` and deliberately no
/// `Transform` — see the module comment.
///
/// A joint of the entity's **own** skeleton also carries a [`pose::Bone`],
/// which is how `animate` writes it in parallel; the joints of what the
/// entity carries — its mount, its attachments, its effects — do not, and are
/// written from the pass's serial lists. See [`pose::Posed`].
#[derive(Component)]
pub struct Joint;

/// One batch of an entity's model.
#[derive(Component)]
pub struct EntityPart;

/// One batch of a model *attached* to an entity — a pauldron, a helm.
///
/// It carries an ordinary `Transform` rather than the composed world matrix a
/// [`Joint`] gets, and the difference is not an inconsistency: an attached model
/// is drawn, so Bevy requires a `Transform` on it and transform propagation will
/// compose the wearer's placement into it. What this writes is therefore the
/// bone's pose in the *model's own* frame, which is exactly the half a joint
/// keeps separate for a different reason.
#[derive(Component)]
pub struct AttachedTo;

/// What an entity is playing, and since when.
#[derive(Component)]
pub struct Playback {
    /// The key hints [`vale_assets::world::m2::M2Skeleton::pose_into`]
    /// keeps between frames — per entity, because two units of one model are
    /// at two clocks. Empty until the first pose; see
    /// [`vale_assets::world::m2::M2Track::sample_hinted`].
    pub(super) hints: Vec<u32>,
    /// **The window of the base sequence this frame advanced through**:
    /// the sequence, the clock before and after, and whether the sequence
    /// is new this frame — what `sound::cues` fires the model's cues off.
    /// Recorded by `animate` for every rig, on or off screen.
    window: Option<(usize, u32, u32, bool)>,
    /// The `AnimationData.dbc` id last asked for.
    wanted: u16,
    /// Which of the model's sequences that resolved to.
    sequence: usize,
    /// When it started, in `Time::elapsed_secs`.
    since: f32,
    fade: Option<Fade>,
    /// A one-shot in progress — a swing, a flinch — and when it must end.
    ///
    /// Kept separately from `wanted` because it is not a *state*: the entity is
    /// still running or standing underneath it, and when the shot finishes the
    /// answer is whatever the state says now rather than whatever it said when
    /// the shot began.
    oneshot: Option<OneShot>,
    /// Every event counter as this entity last reported it.
    ///
    /// The trigger is the *difference*, not the value: each of these is an event
    /// the server states exactly once, so a renderer that polls has to notice the
    /// counter move. `None` until the first poll, because an entity that comes
    /// into view mid-fight with a swing count of forty should not play forty
    /// swings — or even one.
    seen: Option<Counters>,
    /// A cast in progress: which pose it holds, and when that stops being the
    /// answer.
    ///
    /// **A state, not a one-shot**, and that is why it is not an [`OneShot`]:
    /// the wind-up is *held* for as long as the cast bar runs, which the server
    /// states in `SMSG_SPELL_START`. It ends early when `SMSG_SPELL_GO`
    /// releases it, which is the normal case — the timer is only there so that
    /// an interrupted cast does not leave a character frozen in the wind-up for
    /// the rest of the session.
    ///
    /// **The pose travels with it because it belongs to the spell**, not to
    /// casting: a fireball is thrown from the shoulder and a heal is raised
    /// overhead, and the only thing that tells them apart is the spell id
    /// through `SpellVisual`. See [`vale_assets::tables::spell`].
    casting: Option<Casting>,
    /// **The pose an aura holds this unit in** — `SpellVisualKit`'s state-kit
    /// `animID`, resolved from the unit's own `UNIT_FIELD_AURA` slots.
    ///
    /// The same slot a cast's wind-up uses and the same held-pose rules, but a
    /// different *kind* of claim, which is why it is a second field rather than
    /// another `Casting`: a cast is a moment with a length the server states,
    /// and this is a **condition with no clock at all** — it ends when the
    /// server stops saying the aura is there. Giving it a `Casting::until`
    /// would mean inventing a duration for a stun.
    ///
    /// 308 spells state one and **257 of them are `Stun`** (`vale spell`);
    /// it is what "stuns do not play an animation" was. Re-asserted every frame
    /// from the snapshot rather than latched on a counter, for the same reason
    /// [`Playback::speed`] is: it is a fact about the unit, not an event.
    aura: Option<u16>,
    /// **`UNIT_FLAG_STUNNED` with nothing to play** — the unit's clock is held.
    ///
    /// A stun almost always names its own pose, and 257 of the 308 that name one
    /// name `Stun`. The exception is the family that does not, of which Ice
    /// Block is the plain case: spell 11958 is three `SPELL_AURA_MOD_STUN`
    /// effects (`Spell.dbc` columns 91..93 read `12, 39, 39`) whose state kit
    /// 3709 reads `animID = -1`. The reference client leaves the mage standing
    /// **still** inside the ice — not idling, not breathing — and this is what
    /// draws that.
    ///
    /// **Half measured, half taken from a report, and the halves are different
    /// claims.** Measured: the flag, where the client reads it, and that
    /// nothing in the `SpellVisual` chain names a pose for this spell. Not
    /// measured: that the mechanism is the animation clock stopping. Nothing in
    /// `SpellVisualKit`'s 35 columns says "freeze", so this is the picture the
    /// report describes reproduced by the simplest thing that produces it. It
    /// is gated as tightly as the evidence allows — the flag *and* no stated
    /// pose — so nothing that states its own pose can reach it.
    frozen: Option<u32>,
    /// Whether this frame wants the clock held; see [`Self::frozen`].
    freeze: bool,
    /// **The second track**: what the upper body is playing over the legs.
    ///
    /// A swing thrown at a run, an emote while walking, a moving caster's
    /// wind-up — all of them go here rather than onto [`Self::oneshot`], and
    /// the difference is what stops a character halting mid-stride to raise its
    /// hands. Which of the two slots a play lands in is decided per play by
    /// [`route_oneshot`], off the state the unit is in at that moment, and
    /// never by the animation id on its own.
    overlay: Option<Masked>,
    /// How fast the unit is travelling, as of the last poll.
    ///
    /// **A locomotion clip is played at `speed / move_speed`**, not at 1× — see
    /// [`M2Skeleton::playback_rate`], which is the rule, and
    /// [`Playback::note_actions`], which is what keeps this current. It is a
    /// field rather than an argument because every clock in here is derived
    /// from a start time and a `now`, and threading a speed through each of
    /// them would put the same value in five signatures.
    speed: f32,
    /// **The base track's own playback multiplier**, and the only thing that
    /// ever sets it to anything but 1 is the combat fast-path — see
    /// [`Playback::fast_path`]. Separate from the locomotion rate scaling in
    /// [`M2Skeleton::playback_rate`], which is a property of the *clip* and the
    /// unit's speed; this is a property of *this play* and is reset by the next
    /// one.
    base_rate: f32,
    /// **The parked combat request** — the client's own one-slot cache.
    ///
    /// A combat clip asked for while another combat clip is playing is not
    /// armed: the one running speeds up and the request waits here until nothing
    /// is playing. One slot, because that is what the
    /// client has, and because a fast weapon that outran two clips has already
    /// lost the first swing whatever is done with it.
    deferred: Option<u16>,
    /// The model's bones. Held here rather than looked up per frame; a model is
    /// shared, so this is one `Arc` clone per entity.
    skeleton: Arc<M2Skeleton>,
    /// **The animation id a play was started with this frame**, on either track,
    /// or `None` for a frame in which nothing new started.
    ///
    /// The one input to the sheath reconcile (see [`sheath::reconcile`]), and
    /// the reason it is a latch rather than "whatever is playing" is that the
    /// client's reconcile runs *inside* `PlayAnimation`: a frame with no play
    /// must leave the weapons alone, or a held cast's stow is undone by the very
    /// next frame's re-assert. Taken, not read — see [`Playback::take_played`].
    ///
    /// **The id asked for, not the one the model resolved to.** A weaponless
    /// creature asked for `Attack2H` reconciles on that row's flags even though
    /// its skeleton fell back to the unarmed swing, because the client's arm
    /// descriptor carries the request.
    played: Option<u16>,
}

/// A play on the masked upper-body track — see [`Playback::overlay`].
///
/// The mirror of [`OneShot`] with one addition: it carries the sequence as well
/// as the id, because unlike the base track there is nothing else holding what
/// the torso resolved to.
struct Masked {
    /// The `AnimationData.dbc` id, so a held loop can tell whether it is
    /// already the thing playing.
    wanted: u16,
    sequence: usize,
    /// This play's own multiplier — 1, or 2 once the combat fast-path has
    /// re-timed it. See [`Playback::base_rate`], which is the same thing for the
    /// other track.
    rate: f32,
    /// When it started, in `Time::elapsed_secs`.
    since: f32,
    /// When it stops being the answer — as [`OneShot::until`], from the
    /// resolved sequence's own length. `f32::INFINITY` for a **held** loop: a
    /// moving caster's wind-up runs until the cast does, not until the clip
    /// does.
    until: f32,
    /// Whether the clock wraps. A one-shot holds its last frame the way the
    /// base track does; a held wind-up loops.
    looping: bool,
}

/// What an entity last reported that a change in is worth an animation: five
/// event counters and one **state**, as one value.
///
/// Together rather than as six fields on [`Playback`] because they are read and
/// replaced as a set: the comparison is "what has happened since I last
/// looked", and a half-updated snapshot would answer it wrongly.
///
/// **[`Self::airborne`] is the odd one and belongs here anyway.** The other
/// five are counters the server states once each; this is a condition that is
/// simply true or false. What makes it the same kind of thing is that the
/// renderer wants its *edges* — the take-off and the landing are one-shots at
/// the two ends of an arc whose middle is an ordinary state — and an edge needs
/// exactly what a counter needs: the previous reading, and the first-look guard
/// that stops an entity streaming into view mid-air from being handed a jump it
/// did not make. Keeping it in this struct is what gets it both for free.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Counters {
    swings: u32,
    blows: u32,
    emotes: u32,
    /// …and the two `SpellVisualKit`s the server pushes at a unit outright —
    /// see [`WorldEntity::spell_visuals`]. Counters on the same terms as the
    /// emote beside them: the kit's `animID` is a pose, the packet is an
    /// instant, and the poll wants the edge.
    spell_visuals: u32,
    spell_impacts: u32,
    casts_begun: u32,
    casts_released: u32,
    /// …and the casts taken off — see [`WorldEntity::casts_cancelled`]. It is a
    /// counter here for the same reason the other two are: the poll wants the
    /// *edge*, and "a cast stopped" is stated exactly once.
    casts_cancelled: u32,
    /// …and the pushbacks — see [`WorldEntity::casts_delayed`]. A counter for
    /// the third time and for the third variation of the same reason: the poll
    /// wants the edge, and a cast knocked back twice is two statements whose
    /// payloads add.
    casts_delayed: u32,
    /// …and how many of the begins were a **channel** starting, which is the
    /// one begin that arrives *after* the release it belongs to — see
    /// [`WorldEntity::casts_channelled`]. It is what stops a channel's own
    /// wind-up being cancelled by the release that came a microsecond before
    /// it, which is an Evocation standing still for eight seconds.
    casts_channelled: u32,
    airborne: bool,
    /// `GAMEOBJECT_STATE`, and an edge on the same terms [`Self::airborne`] is
    /// one: a chest going from shut to open wants `Open` played once on the way
    /// between two held poses. 255 stands for "not a game object", so a unit
    /// can never look like one that changed state.
    object_state: u8,
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
        }
    }
}

/// The wind-up half of a cast: what is being held, and until when.
struct Casting {
    /// The `AnimationData.dbc` id to hold — **the spell's own and nothing
    /// else**, out of `SpellVisual`'s precast (or channel) kit.
    ///
    /// There is no generic fallback any more: a spell whose chain names no
    /// wind-up gets no `Casting` record at all, because `SpellVisualKit`'s
    /// `animID` is the only thing in the game that says what a cast looks like
    /// and 9,467 of the game's 22,360 spells deliberately say nothing. See
    /// [`super::pose::Playback::note_actions`].
    hold: u16,
    /// When the cast bar runs out, in `Time::elapsed_secs`.
    until: f32,
}

/// A one-shot animation: play it through once, then go back to the state.
struct OneShot {
    /// The `AnimationData.dbc` id, so a repeat of the same event can restart it
    /// — two swings in a row are two swings, not one held pose.
    wanted: u16,
    /// When it stops being the answer, in `Time::elapsed_secs`. Taken from the
    /// sequence's own length rather than from a constant: a wolf's bite and a
    /// giant's overhead swing are not the same duration, and a fixed guess
    /// either cuts the long one off or leaves the short one frozen at its last
    /// frame.
    until: f32,
}

/// The animation being faded out of.
struct Fade {
    sequence: usize,
    /// When *it* started, so it keeps its own clock while it fades: a wolf's
    /// stride finishes on the way out instead of hanging in the air for 150 ms.
    since: f32,
    /// When the fade began.
    from: f32,
    /// **The instant a one-shot ran out**, when that is what is fading — and
    /// the frame it is sampled at is *that* one, not the wrapped clock.
    ///
    /// The clock keeps running for a loop on purpose (see `since`), and
    /// `M2Skeleton::phase` wraps it — which for a one-shot that has just
    /// reached its end is frame **zero**. Measured with
    /// `jump_pose_continuity`: `JumpStart` ended mid-air and the fade sampled
    /// its crouch on the ground, a 2-yard bone move in one frame; `JumpLandRun`
    /// ended on the ground and the fade sampled its airborne first frame. That
    /// is "the model resets mid-air, then replays the second half of the jump
    /// after landing", both halves, and `None` here for a loop is what keeps
    /// the wolf's stride finishing.
    held: Option<f32>,
}

/// Display ids already resolved, and the tables they were resolved from.
///
/// The tables are four DBCs and are parsed once, on the first entity that needs
/// them; the per-id results are cached because forty identical wolves share one
/// display id and every one of them would otherwise walk the table.
#[derive(Resource, Default)]
pub struct DisplayCache {
    resolved: HashMap<(bool, u32), Option<Arc<DisplayModel>>>,
    /// Set once the tables have been asked for, whether or not they parsed —
    /// a chain that cannot supply them should not be reopened per entity.
    tried: bool,
    tables: Option<Arc<vale_assets::tables::dbc::DisplayTables>>,
}

impl DisplayCache {
    /// **Forget the tables and everything resolved from them.**
    ///
    /// `GameAssets::forget_tables` is the same seam one layer down, and on its
    /// own it does not reach here: this holds an `Arc` it took on the first
    /// entity of the session, so dropping the *bank's* copy leaves every pass
    /// reading the parse from before. That is the case its own doc warns
    /// about — "anything holding an `Arc` from before keeps what it holds,
    /// which is every pass that cached one" — and this is that pass.
    ///
    /// What it is for is a host that has made `DBFilesClient\` answer with
    /// different bytes: an edited `SpellVisualKit` naming a different model,
    /// an edited `CreatureDisplayInfo`. Without both halves the file is new
    /// on disk, new in the archives' namespace, and the world goes on drawing
    /// what it read at startup.
    ///
    /// The per-id resolutions go too. They are answers *derived* from the
    /// tables — display id to model path and skin — so keeping them across a
    /// change would be the same staleness one step further on.
    ///
    /// **It takes effect on the next ask, and it is not free**: the next
    /// entity to want a display re-reads and re-parses every table
    /// `DisplayTables::load` names, which is 73 reads and `Spell.dbc` twice.
    /// Call it when something has actually changed.
    pub fn forget(&mut self) {
        self.tables = None;
        self.tried = false;
        self.resolved.clear();
    }
}

/// Everything this module does to an entity's model, in one name.
///
/// So that a pass which *reads* a dressing can order itself after the whole
/// chain rather than after whichever system it happens to depend on today —
/// which is the difference between an ordering that survives a system being
/// added to the middle of the chain and one that silently stops meaning
/// anything. [`crate::render::selection`]'s highlight is the first caller: it
/// rewrites a part's material, and a rebuild in the same frame would throw the
/// rewrite away.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntitySet;

pub struct EntityPlugin;

impl Plugin for EntityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DisplayCache>()
            .init_resource::<PendingImpacts>()
            // …and what the hull budget was asked for against what it paid, which
            // is the one thing the scene tab's hull *count* structurally
            // cannot say. See [`solid::HullWork`].
            .init_resource::<solid::HullWork>()
            .add_message::<SheathRequest>()
            .add_systems(
                Update,
                (
                    light_entities,
                    // **First**, because both the rebuild below and the pose read
                    // the committed sheath state: a weapon drawn this frame has to
                    // be in the hand this frame, on the very press that drew it.
                    sheath::commit,
                    rebuild_changed_models,
                    spawn_models,
                    spawn_attachments,
                    // **Before `animate`**, which poses it and reads the seat it
                    // puts the rider in — a mount given out here is ridden the same
                    // frame rather than the next, so nothing is ever drawn standing
                    // inside its own horse.
                    spawn_mounts,
                    // **Before `spell_effects`**, which is what spawns and expires
                    // all four sets — this only asks for one, so a glow asked for
                    // here is hung in the same frame rather than the next.
                    effects::level_up,
                    // …and beside it for the same reason: an edge that only
                    // *asks*, so a body that becomes lootable sparkles in the
                    // frame it does rather than the next.
                    effects::loot_art,
                    spell_effects,
                    persistent_areas,
                    // **After `persistent_areas`**, which is what installs the
                    // `AreaRain` this reads, and **before `animate_areas`**,
                    // which poses what it spawns: an impact whose first frame
                    // is unposed is a shard drawn in bind pose, thirty yards
                    // up, for one frame at the exact moment the eye is on it.
                    effects::retire_impacts,
                    effects::rain_impacts,
                    fallback_shapes,
                    // **After `spawn_models`**, which is the pass that resolves
                    // a display id — this reads the answer and deliberately
                    // does not resolve one of its own, so a game object is
                    // solid on the frame after it is first drawn rather than
                    // two passes disagreeing about what a table says. See
                    // `solid`, and note that the ordering it needs is against
                    // `DisplayCache` rather than against any transform: it
                    // takes the position from `Motion`, as `place_entities`
                    // does, and a game object does not move.
                    solid::solidify_objects,
                    animate,
                    // …and **last**, because its one input is the animation
                    // `animate` decided to play. One frame of lag between the play
                    // and the weapon moving, which is the same frame the real
                    // client's reconcile costs — it runs inside `PlayAnimation`,
                    // after the pose is already chosen.
                    sheath::reconcile,
                    animate_areas,
                    // **Last of all, because it rewrites materials the rebuild
                    // above would have thrown away** — the same ordering
                    // `render::selection`'s highlight takes against this whole
                    // set, and for the same reason. It reads only the aura list
                    // and what the model is already painted, so nothing after
                    // it depends on it.
                    tint::paint_models,
                    // …and its sibling, which changes the same models' materials
                    // in the same walk-shaped way and for the same reason it is
                    // last. Ordered after the paint rather than beside it only
                    // because they share the material pool: `with_opacity` and
                    // `with_model_tint` each copy the material *as it stands*,
                    // so running them in a fixed order is what makes a
                    // stone-formed rogue in stealth one material carrying both
                    // rather than two systems overwriting each other.
                    tint::fade_models,
                )
                    .chain()
                    // After the session has placed the entities: a joint is written
                    // as a world transform, so it has to be composed against this
                    // frame's placement and not last frame's.
                    .after(crate::world::session::place_entities)
                    // …and after what the *player* did this frame, which is one
                    // specific thing: `AttackTarget()` and `ToggleSheath()` write a
                    // `SheathRequest`, and `sheath::commit` is what executes it.
                    // Without this the two sets are unordered and a drawn weapon
                    // appears on whichever frame Bevy happened to schedule — the
                    // press's own or the one after it.
                    //
                    // **One thing in `game/` does read what this chain writes,
                    // and it reads it a frame late on purpose.** The mouse pick
                    // tests a unit's *posed* triangles (`game::target::mesh_hit`)
                    // through the joints `animate` writes here, so under this
                    // ordering it sees last frame's pose against this frame's
                    // ray. That is 0.07 yards on a running mob at 100 fps —
                    // inside the vertex spacing it is testing against — and the
                    // alternative is a cycle: the pick is chained behind
                    // `arm_look` and the click, and this chain is behind the
                    // press those read.
                    .after(crate::game::GameSet)
                    .in_set(EntitySet),
            )
            // …and the impacts' own sweep, which is deliberately *not* in that
            // chain: an impact is a root entity that the world teardown does
            // not reach, so it has to be dropped on the message rather than on
            // its owner going.
            .add_systems(Update, effects::forget_impacts);
        // **What the world's own population draws**, two switches — see
        // [`crate::render::tuning`]. The model root takes its batches, its
        // attachments and its blob with it; the blob has its own switch as
        // well, because "is that shadow under the character's feet or a
        // decal in the art" is a question no count answers. Entities stream in
        // and out with the session, so both rely on the catch-up pass.
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
/// Drop a model whose entity no longer looks like it, so the next frame builds
/// the right one.
///
/// **A player is dressed a round trip after they appear.** `PLAYER_VISIBLE_ITEM`
/// carries an item *entry*; what it looks like is in the server's template, so
/// the first model an entity gets is built from whatever had arrived by then —
/// usually nothing. Without this the character keeps the body it was born with
/// and stays in its underwear for the session, which reads as "equipment is not
/// implemented" rather than as a race.
///
/// Rebuilding rather than redressing is the honest option: the geosets change
/// too, so the *batch list* is different, not just its materials.
///
/// **Filtered on `Changed<WorldEntity>`, which is only meaningful because
/// [`crate::world::session::poll_world`] writes the component conditionally.** Nothing
/// but a change to the entity can make a model stop describing it, so an
/// unchanged entity cannot need rebuilding — and this used to compare every
/// modelled entity in the world on every frame at 100+ fps against a snapshot
/// that changes at 40. The saving is the whole zone: a city of standing guards
/// now costs nothing here.
mod pose;
mod sheath;
pub(crate) mod solid;
mod spawn;
pub(crate) mod transport;
mod tint;
mod worn;

// Flat, so every `world::entities::Foo` elsewhere keeps resolving: this is
// internal organisation, not a new API.
pub(crate) use effects::*;
use fallback::*;
use mount::spawn_mounts;
pub use mount::Mount;
pub(crate) use pose::*;
// **`Sheath` is public and `SheathRequest` is not.** The first is part of what
// an entity *is*: `spawn::spawn_models` takes it in its query rather than as an
// option, so anything that builds a unit — a test, the crowd, a host with no
// server — has to give it one or the unit is never drawn and nothing says why.
// The second is a message this crate sends to itself.
pub use sheath::Sheath;
pub(crate) use sheath::SheathRequest;
use spawn::*;
use worn::*;
#[cfg(test)]
mod tests;
