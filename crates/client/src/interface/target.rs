//! Target selection: which unit the player has selected, and everything that
//! changes it.
//!
//! The rest of `interface/` reads this module: the melee swing goes at
//! [`Selection::guid`], a cast binds against it, and the target frame draws it.
//!
//! Four rules, each taken from the game:
//!
//! * The client owns the selection and changes it immediately. Clicking a unit
//!   selects it in that frame with no round trip: the selection ring and the
//!   target frame appear at once. `CMSG_SET_SELECTION` informs the server
//!   afterwards. The server uses it to write our `UNIT_FIELD_TARGET` so other
//!   players can see it, to make a faction visible, to drop a rogue's combo
//!   points and to re-aim an auto-shot. None of these is a permission check.
//! * Escape clears the selection, and so does the target streaming out. A dead
//!   target stays selected; see [`drop_stale_selection`]. A selection that
//!   outlives its unit leaves the target frame drawing a unit that is gone and
//!   sends the swing at a guid the server no longer knows.
//! * Tab picks the best candidate the player can see, not the nearest unit in
//!   the world: on-screen units only, then a weighted score of distance from
//!   the screen centre and distance from the character, with a bonus for a unit
//!   already attacking us, and a short history so a second press moves to the
//!   next candidate. [`tab_target`] explains why this is used instead of the
//!   1.12 cone.
//! * Being attacked selects the attacker when nothing is selected. The 1.12.1
//!   client does this; without it a fight can start with no target frame.
//!
//! ## The two stages of the pick
//!
//! The pointer is on a unit when its ray crosses that unit's drawn triangles,
//! in the pose the unit is drawn in. The 1.12.1 client uses the same test.
//! `vale_assets::look::pick` holds the rule; [`hover`] joins it to the world:
//!
//! 1. The sphere of the animation the unit is playing. This is much smaller
//!    than the box the model declares, which is a union over every clip and
//!    every emitter in the file. Using the declared box made a wisp hoverable
//!    from six yards away.
//! 2. The triangles, nearest candidate first, skinned with the same joints the
//!    GPU skins with, so the silhouette a click tests is the one on the screen.
//!
//! Three differences from the 1.12.1 client remain:
//!
//! * A unit behind a wall is clickable. The 1.12.1 client traces the world
//!   first and lets the object win only if it is nearer; this module does not
//!   trace the world.
//! * Every drawn triangle is tested. The 1.12.1 client skips a texture unit
//!   flagged `0x8` and, for one of its pickable classes, every non-opaque
//!   batch. Both filters would only remove triangles.
//! * The pick has no distance limit. If the server has told us a unit exists
//!   and the pointer is on it, it can be hovered. [`TARGET_RANGE`] bounds Tab
//!   only.
//!
//! All three err towards selecting. The opposite error is a mob that cannot be
//! clicked.

use crate::input::bindings::{Binding, BindingPressed, BindingSet};
use super::events::PlayerTargetChanged;
use crate::world::entities::EntityModel;
use crate::world::session::{ActiveSession, LocalPlayer, Session, WorldEntity};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// The unit this client has selected.
///
/// Both halves are kept because both are asked for constantly and neither
/// derives cheaply from the other: the guid is what goes on the wire and the
/// entity is what a query looks up.
#[derive(Resource, Default)]
pub struct Selection {
    pub guid: Option<u64>,
    pub entity: Option<Entity>,
}

impl Selection {
    pub(crate) fn set(&mut self, guid: u64, entity: Entity) {
        self.guid = Some(guid);
        self.entity = Some(entity);
    }

    fn clear(&mut self) {
        self.guid = None;
        self.entity = None;
    }
}

/// A click that landed on a unit. Raised for every such click, whether or not
/// it changed the selection.
///
/// [`Selection`] is a state and this message is an edge. A second left click on
/// the unit already selected leaves the state unchanged (re-sending it would
/// drop a rogue's combo points) but is still a click. The consumer is the NPC
/// greeting: `sound::npc` plays a display's `hello` on every click, as the
/// 1.12.1 client does when a guard is clicked twice. The message is raised by
/// the two systems that read the mouse buttons, so nothing downstream has to
/// infer a click from a selection that did not change.
///
/// Raised by the left button on any unit it lands on, and by the right button
/// when it opens a conversation on one. A right click that starts a fight or
/// loots a corpse is not a greeting and raises nothing.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitClicked {
    pub guid: u64,
}

/// The unit under the cursor this frame, for the mouseover highlight the UI
/// draws and for the click that follows.
#[derive(Resource, Default)]
pub struct Hovered {
    pub guid: Option<u64>,
    pub entity: Option<Entity>,
    /// Whether a right click here would start a fight. This is the same
    /// [`can_attack`] question [`tab_target`] asks, answered once a frame so
    /// that the pointer and the click use the same answer.
    ///
    /// It is stored on the pick and not in `ui::cursor` because friend or foe
    /// is a game rule that can be decided with no window open; the cursor
    /// module only turns the answer into a bitmap. Written by
    /// [`judge_the_hover`].
    pub attackable: bool,
    /// Whether a right click here would open a loot window:
    /// `UNIT_DYNAMIC_FLAGS` bit 0. The bit is per-viewer, so it already answers
    /// for this player and not for the group.
    ///
    /// Stored beside [`Self::attackable`] for the same reason: the pointer and
    /// the click read one answer, so the cursor shown and the action taken
    /// cannot disagree.
    pub lootable: bool,
    /// The opener a right click here sends to the NPC, or `None` for a unit
    /// that offers no service. Stored here for the same reason as the two
    /// fields above: the cursor and the click read one answer, computed once.
    ///
    /// The route follows the precedence of the NPC flags; see [`Interact`].
    pub interact: Option<Interact>,
    /// Which of the game's pointers is shown over this unit, or `None` for the
    /// ordinary arrow.
    ///
    /// It cannot be derived from [`Self::attackable`]. The client tests
    /// `UNIT_NPC_FLAGS` first and only then asks the attack question, so a
    /// vendor or a quest giver shows its own cursor whether or not it is
    /// attackable. The module note of `vale_assets::look::cursor` describes the
    /// order.
    pub cursor: Option<vale_assets::look::cursor::Cursor>,
}

/// The opcode a right click on a service NPC opens with.
///
/// Eight routes, chosen by `UNIT_NPC_FLAGS` in this order:
///
/// * Gossip (`0x1`): `CMSG_GOSSIP_HELLO`. The server decides what comes back: a
///   menu, a questgiver page or a vendor list. Nearly every service NPC in the
///   game carries this bit beside its trade.
/// * Questgiver only (`0x2` without gossip): `CMSG_QUESTGIVER_HELLO`, which
///   skips the menu for an NPC that has only quests.
/// * Vendor only (`0x4` with neither): `CMSG_LIST_INVENTORY`, which opens the
///   shop directly.
/// * Flight master only (`0x8` with none of the three):
///   `CMSG_TAXIQUERYAVAILABLENODES`, answered with the flight map. It is tested
///   before the trainer because `vale_assets::look::cursor` tests
///   `FLIGHTMASTER` first in its chain. Nearly every flight master in 1.12
///   carries the gossip bit as well; the taxi map is then a gossip option (icon
///   2) and this branch is not taken.
/// * Trainer only (`0x10` with none of the four): `CMSG_TRAINER_LIST`. Rare:
///   nearly every trainer in 1.12 carries the gossip bit as well (a Goldshire
///   trainer is `0x13`). This is the end of the same chain
///   `vale_assets::look::cursor` follows, in the same order, so the book cursor
///   and the action of the click agree.
/// * Banker only (`0x100` with none of the five): `CMSG_BANKER_ACTIVATE`. Rare
///   for the same reason: a banker with the gossip bit opens the bank from its
///   menu, which is `GOSSIP_OPTION_BANKER` on the server and `SendShowBank` in
///   both cases. See `vale_protocol::play::bank`.
/// * Petitioner (`0x200` with none of the six): `CMSG_PETITION_SHOWLIST`,
///   answered with the registrar's charter offer. A guild master in 1.12
///   carries this bit and the tabard designer's and no gossip bit. See
///   `vale_protocol::play::petition`.
/// * Tabard designer only (`0x400` with none of the seven):
///   `MSG_TABARDVENDOR_ACTIVATE`, answered with the same opcode, which opens
///   the designer.
///
/// The precedence is an inference, not a verified rule of the 1.12.1 client. It
/// rests on the server's gossip handler composing the questgiver and vendor
/// pages into its menu: asking the widest question first cannot lose a page,
/// and the opposite order would open a shop over a menu the NPC was meant to
/// show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interact {
    Gossip,
    Questgiver,
    Vendor,
    Taxi,
    Trainer,
    Banker,
    Petitioner,
    TabardDesigner,
}

impl Interact {
    /// The route for one unit's flags, or `None` for a plain mob.
    pub fn of(npc_flags: u32) -> Option<Interact> {
        const GOSSIP: u32 = 0x1;
        const QUESTGIVER: u32 = 0x2;
        const VENDOR: u32 = 0x4;
        const FLIGHTMASTER: u32 = 0x8;
        const TRAINER: u32 = 0x10;
        const BANKER: u32 = 0x100;
        const PETITIONER: u32 = 0x200;
        const TABARDDESIGNER: u32 = 0x400;
        if npc_flags & GOSSIP != 0 {
            Some(Interact::Gossip)
        } else if npc_flags & QUESTGIVER != 0 {
            Some(Interact::Questgiver)
        } else if npc_flags & VENDOR != 0 {
            Some(Interact::Vendor)
        } else if npc_flags & FLIGHTMASTER != 0 {
            Some(Interact::Taxi)
        } else if npc_flags & TRAINER != 0 {
            Some(Interact::Trainer)
        } else if npc_flags & BANKER != 0 {
            Some(Interact::Banker)
        } else if npc_flags & PETITIONER != 0 {
            Some(Interact::Petitioner)
        } else if npc_flags & TABARDDESIGNER != 0 {
            Some(Interact::TabardDesigner)
        } else {
            None
        }
    }
}

/// The reach of Tab targeting, in yards. Tab is its only reader.
///
/// This is the game's `targetNearestDistance` default, 41 yards, still shipped
/// as a CVar in Classic with the description "limited to tab targeting range".
/// Tab picks a candidate from every unit in the world and needs a bound; a
/// click does not, because the pointer is either on the unit or not. [`hover`]
/// once applied this limit and no longer does.
///
/// Measured from the character, not from the camera. `tab_target` once measured
/// from `camera_transform.translation()`, which subtracted the camera rig's
/// distance from the reach and left about a third of the stated value: at the
/// default 25-yard zoom a mob 17 yards away was out of range, and the reach
/// changed with the zoom wheel. The CVar names a distance to the nearest
/// target, which is a property of the character and not of the camera.
const TARGET_RANGE: f32 = 41.0;

/// How long a Tab-cycled guid is skipped by the next press, in seconds.
///
/// The mechanism is the client's `TargetPriorityHighlightHistoryMs`: "time
/// target history should be maintained for repeated activations". The value is
/// a placeholder: the client's default is not known to this project. Five
/// seconds is long enough to cycle through a pull and short enough that the
/// wrap is not noticeable.
const TAB_HISTORY_SECS: f32 = 5.0;

/// Guids Tab has recently handed out, with when, and which pool they came
/// from.
///
/// One history serves the enemy and the friendly pair. A press of the other
/// pair empties it, as the 1.12.1 client rebuilds its candidate list when the
/// kind asked for changes, so a friendly press does not skip units an enemy
/// cycle handed out.
#[derive(Resource, Default)]
struct TabHistory {
    picks: Vec<(u64, f32)>,
    friendly: bool,
}

/// The last unit the character selected that it could attack, for
/// `TargetLastEnemy()`.
///
/// The 1.12.1 client records the new selection whenever it is a unit the
/// character can attack, so after a hostile target is replaced by a friend or
/// cleared, `TARGETLASTHOSTILE` selects it again. A unit whose health has
/// not arrived is not recorded. Written by [`remember_enemy`].
#[derive(Resource, Default)]
struct LastEnemy(Option<u64>);

/// How many swings this unit had thrown when [`acquire_attacker`] last looked.
///
/// The first look stores the counter without acting on it. A creature that
/// walks into view mid-fight has already thrown swings, and treating those as
/// new would select a unit that merely came into view. The spell-effect pass
/// uses the same first-look latch for the same reason.
#[derive(Component)]
struct SeenSwings(u32);

/// The system set of this module's chain, so [`super::action`] can order itself
/// after all of it and not after one system in it.
///
/// The ordering is required: a cast binds against the selection, so a key press
/// judged before this frame's click sends the spell at the previous target.
/// Bevy orders two systems that touch `Selection` only while both keep the
/// conflicting access, so the order is declared explicitly.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetSet;

/// Registers this module's resources, messages and ordered systems.
pub struct TargetPlugin;

impl Plugin for TargetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Selection>()
            .init_resource::<Hovered>()
            .init_resource::<TabHistory>()
            .init_resource::<LastEnemy>()
            .add_message::<UnitClicked>()
            .add_systems(
                Update,
                (
                    // The hover is recomputed before the click that reads it,
                    // and the whole chain runs after the world has been placed.
                    // A pick against last frame's transforms misses a running
                    // mob by one frame of its movement.
                    hover,
                    // Judges the hovered unit once here, so the cursor and the
                    // click read the same answer. See [`Hovered::attackable`].
                    judge_the_hover,
                    // Judges the hover for the spell cursor before the click
                    // that acts on it. The valid or invalid spell cursor is the
                    // only warning the player gets, and a frame of delay shows
                    // the wrong cursor over a corpse for that frame.
                    spell_cursor_validity,
                    // The same judgement for a cast that targets a place. It
                    // runs before the click for the same reason: the click
                    // consumes the point this system stores.
                    spell_ground_under_pointer,
                    select_on_click,
                    // The right button, after the left. Both read `Hovered` and
                    // write `Selection`. The order is declared explicitly and
                    // does not rely on the conflicting access: in a frame where
                    // both buttons are released, the last writer decides.
                    attack_on_right_click,
                    tab_target,
                    acquire_attacker,
                    // Last, so a selection that died this frame is dropped before
                    // anything is sent at it.
                    drop_stale_selection,
                    remember_enemy,
                    // After every writer: one `PLAYER_TARGET_CHANGED` per frame
                    // in which the selection ended up different, whichever of
                    // the four systems above changed it. It is written from a
                    // comparison and not by each writer, so a Tab that lands on
                    // the unit already selected raises nothing.
                    announce_target_change,
                    // The same for the pointer. The pointer crosses a unit's
                    // edge in about one frame in a hundred, and a per-frame
                    // event would refill the tooltip on every frame.
                    announce_mouseover_change,
                )
                    .chain()
                    .after(crate::world::session::place_entities)
                    // After the mouse gesture has been judged.
                    // `select_on_click` reads `MouseLook` to tell a click from
                    // a camera look and from a drag that began on the
                    // interface. Unordered, it could read last frame's verdict,
                    // and the release frame is the one whose verdict matters.
                    .after(crate::world::camera::arm_look)
                    .after(BindingSet)
                    .in_set(TargetSet)
                    .in_set(super::GameSet),
            )
            .add_systems(Update, forget.in_set(super::GameSet));
    }
}

/// Clears the selection when the player leaves the world.
///
/// A guid outlives its world and an `Entity` outlives the despawn that frees
/// it, so a target held across a logout is a stale handle.
/// [`drop_stale_selection`] clears it only when a new world exists to fail the
/// lookup against, which is one frame into the next session, and the previous
/// character's target frame would be drawn for that frame. See
/// [`super::events::PlayerLeavingWorld`].
fn forget(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut selection: ResMut<Selection>,
    mut hovered: ResMut<Hovered>,
    mut history: ResMut<TabHistory>,
    mut last_enemy: ResMut<LastEnemy>,
) {
    if leaving.read().next().is_none() {
        return;
    }
    selection.clear();
    *hovered = Hovered::default();
    history.picks.clear();
    last_enemy.0 = None;
}

/// Raises `PLAYER_TARGET_CHANGED` once when the selected guid changes.
fn announce_target_change(
    selection: Res<Selection>,
    mut changed: MessageWriter<PlayerTargetChanged>,
    mut last: Local<Option<Option<u64>>>,
) {
    let now = selection.guid;
    // The first frame stores the value without announcing. There was no
    // previous target to change from, and an event on the first frame would
    // fire before any handler had registered for it.
    if let Some(previous) = last.replace(now) {
        if previous != now {
            changed.write(PlayerTargetChanged);
        }
    }
}

/// Raises `UPDATE_MOUSEOVER_UNIT` once when the hovered guid changes. The
/// counterpart of [`announce_target_change`].
///
/// The 1.12.1 client raises this event when the mouseover unit changes, not per
/// frame. The shipped handler recolours the tooltip, so firing it every frame
/// would repaint the tooltip at frame rate for no effect.
///
/// The guid is compared, not the entity. A unit that streams out and back in
/// under a stationary pointer is the same mouseover; an entity comparison would
/// raise twice and refill the tooltip for a unit that did not change.
///
/// ## The event is not raised when the mouseover token becomes empty
///
/// The event means "the `mouseover` token now names a different unit". The one
/// handler in the shipped interface, in `GameTooltip.xml`, is
/// `TextLeft1:SetTextColor(GameTooltip_UnitColor("mouseover"))`.
/// `GameTooltip_UnitColor` assumes the unit exists: with none, it falls past
/// `UnitPlayerControlled` and past a nil `UnitReaction` to the catch-all, which
/// returns white.
///
/// While the tooltip was hidden on the frame the pointer left, that white was
/// never drawn. The tooltip now holds and fades (see
/// [`crate::lua::widgets::tooltip`]), so raising the event on the empty edge
/// changed the name from its faction colour to white before it faded out.
/// Nothing in Lua could cause or prevent that.
///
/// The 1.12.1 client's tooltip fades in its faction colour, and its handler
/// would paint white if the event were raised on the empty edge, so the client
/// does not raise it there. This system raises the event only when the token
/// comes to name a unit. The last colour stays on a tooltip that is fading out.
fn announce_mouseover_change(
    hovered: Res<Hovered>,
    mut changed: MessageWriter<super::events::MouseoverUnitChanged>,
    mut last: Local<Option<Option<u64>>>,
) {
    let now = hovered.guid;
    if let Some(previous) = last.replace(now) {
        if previous != now && now.is_some() {
            changed.write(super::events::MouseoverUnitChanged);
        }
    }
}

/// Judges the unit under the pointer: whether we could attack it, and which of
/// the game's cursors is shown over it.
///
/// One system produces both answers because their two consumers, the pointer
/// bitmap (`ui::cursor`) and the right click ([`attack_on_right_click`]), must
/// agree. A sword cursor over a unit that the click then declines to attack is
/// misleading.
///
/// The two answers are different questions. Attackability comes from the
/// faction table. The cursor comes from `UNIT_NPC_FLAGS` first and the faction
/// table second, because the client tests the flags first; see
/// `vale_assets::look::cursor`. The base reaction to a friendly NPC is often
/// Neutral (a GM's faction template is 35, which shares no group mask with any
/// other), so a cursor keyed on attackability alone shows a sword over most of
/// a town.
///
/// Runs every frame for the same reason as [`spell_cursor_validity`]: the world
/// changes under a stationary pointer. The unit can die, change faction or be
/// tagged `NON_ATTACKABLE` while the mouse does not move.
fn judge_the_hover(
    assets: Res<crate::assets::GameAssets>,
    units: Query<&WorldEntity>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    // The same two resources the interface's answer is built from.
    // [`crate::interface::api::can_attack_between`] explains why a pass that
    // holds entities must not fall back to the bare faction table.
    group: Res<crate::interface::party::Party>,
    reputation: Res<crate::interface::reputation::PlayerStanding>,
    mut hovered: ResMut<Hovered>,
) {
    let judge = || {
        let tables = assets.display_tables().ok()?;
        let me = player.single().ok()?;
        let unit = units.get(hovered.entity?).ok()?;
        // The local player is not a target here. No faction table says so: a
        // character's template is friendly to itself, and a sword cursor over
        // the player's own body would be wrong in either case. A game object
        // has no services and cannot be attacked, so both answers are empty for
        // it.
        //
        // A dead unit is not attackable but may be lootable: a corpse is the
        // one case where a right click acts because the unit is dead.
        if unit.is_self || !is_unit(unit) {
            return Some((false, false, None, None));
        }
        if unit.dead {
            return Some((
                false,
                unit.lootable,
                None,
                vale_assets::look::cursor::over_unit(0, false, false, unit.lootable),
            ));
        }
        let attackable =
            crate::interface::api::can_attack_between(&tables, &group, &reputation, me, unit);
        // The reaction is ranked in both directions, as the client does. This
        // stops an enemy-faction vendor showing a coin purse cursor. See
        // [`vale_assets::look::cursor::can_interact`].
        let friendship = crate::interface::api::Friendship { party: &group, standing: &reputation };
        let can_interact = vale_assets::look::cursor::can_interact(
            unit.npc_flags,
            unit.unit_flags,
            friendship.rank(&tables, me, unit),
            friendship.rank(&tables, unit, me),
        );
        Some((
            attackable,
            unit.lootable,
            // The panel that opens is gated on the same answer. A right click
            // that opened a shop with an NPC that will not interact with us
            // would disagree with the cursor, and the server refuses it anyway.
            can_interact.then(|| Interact::of(unit.npc_flags)).flatten(),
            vale_assets::look::cursor::over_unit(
                unit.npc_flags,
                can_interact,
                attackable,
                unit.lootable,
            ),
        ))
    };
    let (attackable, lootable, interact, cursor) =
        judge().unwrap_or((false, false, None, None));
    // A `ResMut` written every frame marks the resource changed every frame;
    // the compares keep the change flag meaningful for anything that reads it.
    if hovered.attackable != attackable {
        hovered.attackable = attackable;
    }
    if hovered.lootable != lootable {
        hovered.lootable = lootable;
    }
    if hovered.interact != interact {
        hovered.interact = interact;
    }
    if hovered.cursor != cursor {
        hovered.cursor = cursor;
    }
}

/// Whether the spell waiting for a target accepts the unit under the pointer.
/// The answer selects `Cast.blp` or `UnableCast.blp`.
///
/// Both cursors are in the archives and this system is the only thing that
/// chooses between them, so it is the only feedback the player gets before
/// clicking. The answer is computed every frame and not latched at the key
/// press, because the world changes while the cursor is up: the unit pointed at
/// can die. The rule is the same
/// [`resolve_aim`][vale_assets::tables::spellbook::resolve_aim] the click runs.
///
/// It is separate from [`hover`] because it judges the pick and is not a pick
/// itself. It needs the spell catalog and the faction table, which `hover` does
/// not hold.
fn spell_cursor_validity(
    assets: Res<crate::assets::GameAssets>,
    hovered: Res<Hovered>,
    units: Query<&WorldEntity>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    // The same two the press itself will use, so the cursor and the click
    // cannot disagree — see [`crate::interface::api::Friendship`].
    group: Res<crate::interface::party::Party>,
    reputation: Res<crate::interface::reputation::PlayerStanding>,
    mut targeting: ResMut<super::action::SpellTargeting>,
) {
    // A placed cast is judged by [`spell_ground_under_pointer`], which tests a
    // different ray against the ground.
    if !targeting.is_targeting() || targeting.wants_ground() {
        return;
    }
    let valid = || {
        let tables = assets.display_tables().ok()?;
        let info = tables.spellbook()?.info(targeting.spell()?)?;
        let me = player.single().ok()?;
        let unit = units.get(hovered.entity?).ok()?;
        let who = vale_assets::tables::spellbook::Candidate {
            guid: unit.guid,
            is_self: unit.guid == me.guid,
            reaction: crate::interface::api::Friendship { party: &group, standing: &reputation }
                .reaction(&tables, me, unit),
            unit_flags: unit.unit_flags,
            dead: unit.dead,
        };
        Some(matches!(
            vale_assets::tables::spellbook::resolve_aim(&info, Some(who), None, false, None),
            vale_assets::tables::spellbook::CastAim::Unit(_)
        ))
    };
    targeting.over_valid = valid().unwrap_or(false);
}

/// How far along the pointer's ray the ground is searched for, in yards.
///
/// This is not a spell's range. The ray starts at the camera, which sits up to
/// `CameraRig::distance` behind and above the character, so a bound equal to
/// the spell's range would find no ground at full zoom-out. [`TARGET_RANGE`]
/// documents the same error of measuring from the camera instead of the
/// character. The range check is applied separately to the point once it is
/// found; this bound only limits how far the search goes before it gives up.
///
/// The bound is the distance to which terrain is drawn. It was once a flat 150
/// yards, which is ample for Blizzard (range 30) but not for Eagle Eye:
/// `SpellRange.dbc` gives it 50,000 yards and the 1.12.1 client lets it land on
/// any terrain on the screen, so a pointer aimed at a hillside 300 yards away
/// found no ground and the click cancelled the cast. The terrain reach is the
/// far corner of the streamed block and the camera's far plane; beyond it
/// nothing is drawn to aim at and nothing is loaded to answer the query.
///
/// The cost does not grow in proportion. The step of `ground_under_ray` is half
/// a yard out to 150 yards and proportional to the distance after that, so the
/// cost inside the old bound is unchanged and ten times the reach costs about
/// three times the samples. The search runs only while a placed cast is waiting
/// for its click. The value is read from
/// [`crate::render::terrain::TerrainReach`] and not written as a constant, so a
/// host that streams a wider block can aim across all of it.
fn ground_pick_range(reach: &crate::render::terrain::TerrainReach) -> f32 {
    reach.yards()
}

/// Finds the point on the ground under the pointer while a placed cast is
/// waiting for one. This is the game's ground-targeting circle without the
/// circle drawn.
///
/// The pick and the judgement are one system here, where the unit half uses
/// two. [`hover`] casts its ray on every frame of every session because the
/// tooltip and the highlight need it, so judging it separately costs nothing
/// extra. This ray costs a search along the terrain, so it runs only while the
/// cursor is up, and the system that runs it already has what the validity test
/// needs.
///
/// It finds no valid point in three cases. Each shows `UnableCast.blp` and none
/// is an error:
///
/// * the pointer is on the sky, or past the edge of the loaded world;
/// * the point is further than the spell's range. This is the
///   `SPELL_FAILED_OUT_OF_RANGE` the click would receive, shown before the
///   click;
/// * the pointer is on the interface, where 1.12 has no world pick.
///
/// The ground is the one the character walks on. `ActiveSession::standing` is
/// the same join the movement code uses, so a spell placed on a bridge lands on
/// the bridge and not in the river under it.
#[allow(clippy::too_many_arguments)]
fn spell_ground_under_pointer(
    assets: Res<crate::assets::GameAssets>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::world::camera::WorldCamera>>,
    interface: Res<crate::lua::api::mouse::MouseFocus>,
    session: Res<Session>,
    solids: Res<crate::world::session::Solids>,
    // The same planted pointer the unit pick reads, so a scripted run that aims
    // a placed cast gets the place it aimed at. See
    // [`crate::HoverProbe::instead_of`].
    probe: Res<crate::HoverProbe>,
    reach: Res<crate::render::terrain::TerrainReach>,
    player: Query<&Transform, With<LocalPlayer>>,
    mut targeting: ResMut<super::action::SpellTargeting>,
) {
    if !targeting.wants_ground() {
        return;
    }
    targeting.over_ground = None;
    targeting.over_valid = false;

    let (Ok(window), Ok((camera, camera_transform))) = (windows.single(), camera.single()) else {
        return;
    };
    let Some(cursor) = probe
        .instead_of(window.cursor_position())
        .filter(|_| !interface.over_interface)
    else {
        return;
    };
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let Ok(here) = player.single() else {
        return;
    };
    // Physical pixels, for the reason [`hover`] gives: the world camera draws
    // into `render::present`'s frame image, which is sized in them.
    let Ok(ray) = camera.viewport_to_world(camera_transform, cursor * window.scale_factor()) else {
        return;
    };

    let standing = active.standing(&solids);
    let map_id = active.map_id;
    let hit = vale_assets::world::collision::ground_under_ray(
        crate::render::axes::to_wow(ray.origin),
        crate::render::axes::to_wow(*ray.direction),
        ground_pick_range(&reach),
        |x, y, z| {
            use vale_protocol::socket::session::World;
            standing.floor(map_id, x, y, z)
        },
    );
    let Some(at) = hit else { return };
    targeting.over_ground = Some(at);

    // The range decides between the valid and the invalid pointer. This is the
    // same `check_cast` the click runs, measured the same way, but with only
    // the distance filled in. The pointer reports where the cast may land;
    // marking it invalid because mana is short would be feedback the 1.12.1
    // client does not give. The click still runs the full check.
    let in_range = || {
        let info = assets
            .display_tables()
            .ok()?
            .spellbook()?
            .info(targeting.spell()?)?;
        let conditions = vale_assets::tables::spellbook::CastConditions {
            distance: Some(here.translation.distance(crate::render::axes::to_bevy(at))),
            ..vale_assets::tables::spellbook::CastConditions::default()
        };
        Some(vale_assets::tables::spellbook::check_cast(&info, &conditions).is_none())
    };
    targeting.over_valid = in_range().unwrap_or(true);
}

/// Everything [`hover`] needs about one entity to decide whether the pointer is
/// on it: what it is, where it stands, what model it was given and what that
/// model is playing.
///
/// The model and the clip are optional for the same reason: an entity exists
/// from the first update block and acquires its model a frame or several later,
/// and its `Playback` only says what it is playing once `animate` has run.
type Pickable = (
    Entity,
    &'static WorldEntity,
    &'static Transform,
    Option<&'static EntityModel>,
    Option<&'static crate::world::entities::Playback>,
    // The mount the unit is riding. It is a second rig on the same entity, not
    // an entity of its own, so nothing else in this query sees it, and without
    // it a click on the mount's flank would pass through to whatever is behind.
    // See `Mount::pick_target`.
    Option<&'static crate::world::entities::Mount>,
);

/// The unit under the cursor: the sphere of the clip it is playing, then its
/// drawn triangles.
///
/// ## No range limit
///
/// The pointer has no range limit. It once applied [`TARGET_RANGE`]'s 41 yards,
/// measured from the camera instead of the character, so the real reach was `41
/// - zoom`. The limit does not belong here in any case: `targetNearestDistance`
/// is Tab's value (its description says "limited to tab targeting range") and a
/// pointer is not Tab. A click requires that the unit is in the world and under
/// the cursor. The streaming radius decides the first: an entity that is
/// visible is one the server has told us about.
///
/// One asymmetry follows and is intended: a unit can be hovered and selected
/// from further away than any action against it is allowed. The 1.12.1 client
/// behaves the same way. The range check is on the cast and the swing
/// (`assets::spellbook::check_cast`, surface to surface using
/// `UNIT_FIELD_COMBATREACH`), not on the selection.
///
/// ## A pointer over the interface hovers nothing
///
/// The tooltip, the mouseover highlight and the click depend on this.
/// `select_on_click` used to decline such a click separately while the pick
/// stayed active underneath, so a unit behind the action bar was highlighted
/// and showed its tooltip while the pointer was on a button. 1.12 has no such
/// state, because `WorldFrame` is a frame and the pointer lands on exactly one
/// frame.
///
/// ## Why the two stages are one system
///
/// The narrow phase is cheap only because the broad phase is selective. It
/// walks a few thousand triangles per candidate and runs against the candidates
/// in distance order with the first hit winning, as the 1.12.1 client does. On
/// an ordinary frame the pointer is over nothing and no triangles are walked;
/// over a crowd the walk happens once. Splitting the stages would mean sorting
/// twice or passing a list between systems, for no gain.
///
/// ## The joints are read, not recomputed
///
/// `run_after` puts this system after [`crate::world::entities::animate`],
/// because the triangles are skinned with the joint entities the GPU skins
/// from. The outline a click tests is therefore the one on the screen. Posing
/// the skeleton again here would produce a second pose, and the drawn one
/// composes a cross-fade, a counter-twist and a masked second track.
pub(crate) fn hover(
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::world::camera::WorldCamera>>,
    interface: Res<crate::lua::api::mouse::MouseFocus>,
    units: Query<Pickable>,
    joints: Query<&GlobalTransform>,
    // `--hover` plants a pointer position for a scripted run. See
    // [`crate::HoverProbe::instead_of`], which is the identity without it. The
    // position is read here and the real mouse is not moved, because a window
    // without focus cannot move its cursor and a scripted run has no focus.
    probe: Res<crate::HoverProbe>,
    mut candidates: Local<Vec<(f32, Entity, u64)>>,
    mut hovered: ResMut<Hovered>,
    // The game-object result of the same pick. [`super::object::HoveredObject`]
    // explains why the answer is split into two resources and the pick is not
    // split into two rays.
    mut object: ResMut<super::object::HoveredObject>,
) {
    let (Ok(window), Ok((camera, camera_transform))) = (windows.single(), camera.single()) else {
        return;
    };
    let Some(cursor) = probe
        .instead_of(window.cursor_position())
        .filter(|_| !interface.over_interface)
    else {
        hovered.guid = None;
        hovered.entity = None;
        object.guid = None;
        object.entity = None;
        return;
    };
    // Physical pixels, not logical. `cursor_position` is logical. The world
    // camera's target is `render::present`'s frame image, which is created at
    // the window's physical size with `scale_factor: 1.0`, so its viewport is
    // measured in physical pixels. On a 125% display the two differ by a
    // quarter of the screen, and a pick in logical pixels lands up and left of
    // the cursor.
    let viewport = cursor * window.scale_factor();
    let Ok(ray) = camera.viewport_to_world(camera_transform, viewport) else {
        return;
    };

    // Stage one. The list is reused across frames and not allocated per frame:
    // this runs sixty times a second and the list is usually empty.
    candidates.clear();
    // The pointer has no range limit; see this system's doc comment. `f32::MAX`
    // is passed as the range because the returned distance is used only to
    // order the triangle tests.
    let crosses = |centre: Vec3, radius: f32| {
        vale_assets::look::pick::ray_sphere(
            ray.origin.to_array(),
            ray.direction.to_array(),
            f32::MAX,
            centre.to_array(),
            radius,
        )
    };
    for (entity, unit, transform, model, playback, mount) in &units {
        // Only units and game objects are candidates. An item is an entry in a
        // bag and not an object in the world, and nothing here draws a corpse's
        // `Corpse` object. The local player is excluded: the pointer is usually
        // over the player's own character.
        //
        // The game object arm is required for usable objects. The test was once
        // `!is_unit(unit)`, which excluded every game object from the pick, so
        // the Deadmines doors could not be used.
        if !(is_unit(unit) || is_pickable_object(unit)) || unit.is_self {
            continue;
        }
        let Some(model) = model else { continue };
        let sphere = model.pick_sphere(playback.and_then(|p| p.clip()));
        let (centre, radius) = place_sphere(transform, sphere);
        // A hit on either rig counts. The two are tested separately and not
        // merged into one sphere, because a sphere round a rider and its kodo
        // covers most of the ground the animal stands on, and the broad phase
        // is only useful while its spheres are small.
        let seat = mount.map(|mount| {
            let (sphere, placed, ..) = mount.pick_target();
            (
                placed.transform_point3(crate::render::axes::to_bevy(sphere.centre)),
                sphere.radius * placed.to_scale_rotation_translation().0.max_element(),
            )
        });
        let nearest = crosses(centre, radius)
            .into_iter()
            .chain(seat.and_then(|(centre, radius)| crosses(centre, radius)))
            .reduce(f32::min);
        let Some(along_ray) = nearest else { continue };
        candidates.push((along_ray, entity, unit.guid));
    }
    // Nearest sphere first, which is what makes "the first mesh hit wins"
    // correct — a far unit's triangles can be nearer than a near unit's sphere
    // only if the spheres overlap, and then the loop below keeps looking anyway.
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));

    // Stage two, nearest first. The answer is the nearest triangle hit, not the
    // first candidate whose sphere was nearest.
    let mut best: Option<(f32, Entity, u64)> = None;
    for &(sphere_at, entity, guid) in candidates.iter() {
        // Everything left to test starts further away than a confirmed hit.
        if best.is_some_and(|(hit, ..)| sphere_at > hit) {
            break;
        }
        let Ok((_, _, transform, Some(model), playback, mount)) = units.get(entity) else {
            continue;
        };
        // Whether the joints hold a pose. A unit spawned this frame has not
        // been through `animate`, so its joint transforms are whatever the
        // spawn left. Skinning to those would put the silhouette at the world
        // origin and make the unit unclickable. `Playback::clip` is `None` for
        // exactly that period.
        let posed = playback.is_some_and(|p| p.clip().is_some());
        let rider = mesh_hit(&ray, transform, &model.pick, &model.joints, &joints, posed, None);
        // The mount under the rider, in its own frame. See
        // `Mount::pick_target`.
        let animal = mount.and_then(|mount| {
            let (_, placed, mesh, bones) = mount.pick_target();
            mesh_hit(&ray, transform, mesh, bones, &joints, posed, Some(placed))
        });
        let Some(at) = rider.into_iter().chain(animal).reduce(f32::min) else {
            continue;
        };
        if best.is_none_or(|(nearest, ..)| at < nearest) {
            best = Some((at, entity, guid));
        }
    }
    // One result, stored by what was hit. Exactly one of the two resources is
    // filled. Which one is decided by the depth test above and not by a
    // precedence rule, so a mob in front of a mailbox takes the click, and a
    // mailbox in front of a mob takes it.
    let is_object = best
        .and_then(|(_, entity, _)| units.get(entity).ok())
        .is_some_and(|(_, unit, ..)| is_game_object(unit));
    hovered.entity = best.map(|(_, entity, _)| entity).filter(|_| !is_object);
    hovered.guid = best.map(|(.., guid)| guid).filter(|_| !is_object);
    object.entity = best.map(|(_, entity, _)| entity).filter(|_| is_object);
    object.guid = best.map(|(.., guid)| guid).filter(|_| is_object);
}

/// The smallest broad-phase sphere a unit may have, in yards.
///
/// A few model files state no radius and no usable box, and a unit with a
/// zero-radius sphere cannot be clicked. Half a yard is the minimum the earlier
/// pick box used, for the same reason.
const MIN_PICK_RADIUS: f32 = 0.5;

/// Places a model-space broad-phase sphere at the unit's position in the world.
///
/// The centre is given in the file's axes and must go through the whole
/// transform, not only the translation. The centre is not on the model's origin
/// (`HumanMale`'s standing box is centred 0.12 yards behind it), so translating
/// alone leaves the sphere of a turned unit offset to one side. The radius is a
/// scalar and needs only the scale, which `place_entities` has already put on
/// the transform from `OBJECT_FIELD_SCALE_X`.
fn place_sphere(transform: &Transform, sphere: vale_assets::look::pick::Sphere) -> (Vec3, f32) {
    (
        transform.transform_point(crate::render::axes::to_bevy(sphere.centre)),
        (sphere.radius * transform.scale.max_element()).max(MIN_PICK_RADIUS),
    )
}

/// The narrow phase: a ray against a unit's drawn triangles, in the pose the
/// unit is drawn in.
///
/// Each vertex goes through the joint the GPU would use: `world = joint_affine
/// * to_bevy(model_position)`. This is the skinning identity, because a joint's
/// `GlobalTransform` is already `world_from_model · B · P · B⁻¹` (see
/// `crate::render::axes::pose_to_bevy`). No second pose is computed here, so
/// the tested pose cannot differ from the drawn one.
///
/// A model with no joints (scenery, and any model whose skeleton did not
/// validate) falls back to the entity's own transform, which is the bind pose
/// it is drawn in. `posed` says the same about a unit that has bones but has
/// not been through `animate` yet.
///
/// The joints are one frame old. The entity chain runs `.after(GameSet)` and
/// this system is inside `GameSet`, so it reads the pose `animate` composed
/// last frame against last frame's placement. At 100 fps that is 0.07 yards on
/// a running mob, less than the spacing between the vertices tested. Reversing
/// the order would break a chain whose ordering the weapons and the ground
/// stance depend on.
///
/// `placed` is the rig's own frame when it is not the entity's (a mount's
/// `world_from_mount`), and `None` for the entity's own model.
#[allow(clippy::too_many_arguments)]
fn mesh_hit(
    ray: &Ray3d,
    transform: &Transform,
    mesh: &vale_assets::look::pick::PickMesh,
    rig: &[Entity],
    joints: &Query<&GlobalTransform>,
    posed: bool,
    placed: Option<Mat4>,
) -> Option<f32> {
    if mesh.is_empty() {
        return None;
    }
    let fallback = placed.unwrap_or_else(|| transform.to_matrix());
    // Resolved once per unit rather than per vertex: this is a random-access
    // component lookup and there are up to a hundred bones.
    let bones: Vec<Mat4> = rig
        .iter()
        .filter(|_| posed)
        .map(|&joint| joints.get(joint).map_or(fallback, |g| g.to_matrix()))
        .collect();
    hit_posed_mesh(ray, mesh, &bones, fallback)
}

/// The arithmetic of [`mesh_hit`], with no ECS access.
///
/// It is separate so that a test can check the join of a model-space vertex,
/// the change of basis and a world matrix composed elsewhere, which can be
/// wrong without being obviously wrong. `crate::render::axes` already tests
/// that `pose_to_bevy(bone) * to_bevy(v)` equals `to_bevy(bone * v)`. A test of
/// this function asserts that the pick applies that identity in the same
/// direction the GPU does, against a stated world position and not against a
/// second value derived from the same assumption. `solid.rs` documents why that
/// distinction matters.
fn hit_posed_mesh(
    ray: &Ray3d,
    mesh: &vale_assets::look::pick::PickMesh,
    bones: &[Mat4],
    fallback: Mat4,
) -> Option<f32> {
    vale_assets::look::pick::hit_mesh(
        mesh,
        |vertex| {
            let local = crate::render::axes::to_bevy(mesh.positions[vertex]);
            let (Some(weights), Some(indices)) = (mesh.weights.get(vertex), mesh.bones.get(vertex))
            else {
                return fallback.transform_point3(local).to_array();
            };
            let mut out = Vec3::ZERO;
            let mut total = 0.0f32;
            for slot in 0..4 {
                let weight = f32::from(weights[slot]) / 255.0;
                if weight == 0.0 {
                    continue;
                }
                let Some(bone) = bones.get(usize::from(indices[slot])) else {
                    continue;
                };
                total += weight;
                out += weight * bone.transform_point3(local);
            }
            // A vertex with no usable weights follows the entity's own
            // transform, which is the identity joint the skinning shader gives
            // it.
            if total == 0.0 {
                fallback.transform_point3(local).to_array()
            } else {
                out.to_array()
            }
        },
        ray.origin.to_array(),
        ray.direction.to_array(),
    )
}

/// The CVar that decides whether a click on nothing clears the target:
/// `deselectOnClick`, registered `"1"`. It is the `GAMEFIELD_DESELECT_TEXT`
/// checkbox of `UIOptionsFrame.lua`, under General, so a player can reach it
/// without the console.
const DESELECT_ON_CLICK: &str = "deselectOnClick";

/// A left click selects the hovered unit; on empty ground it clears the
/// selection.
///
/// ## A click, not a drag
///
/// The left button also orbits the camera, so a release after the mouse moved
/// is a camera look and must not retarget. Otherwise every camera swing that
/// ends over a mob selects it.
///
/// [`crate::world::camera::MouseLook`] decides which of the two a gesture was.
/// This system once kept its own `Local<bool>` set by `motion.delta !=
/// Vec2::ZERO`, so any movement was a drag. A mouse moves a pixel or two during
/// an ordinary click, so clicking a mob sometimes failed. The shared state also
/// decides a second case: a gesture that began on the interface is not a click
/// on the world, wherever it ends.
///
/// ## While the targeting cursor is up, a click casts
///
/// The spell cursor is what the 1.12.1 client shows when a friendly spell is
/// pressed with no suitable unit selected (see
/// [`super::action::SpellTargeting`]). While it is up, a click names the unit
/// the waiting spell hits and does not change the selection, as in the 1.12.1
/// client: healing a unit does not make it the target.
///
/// This is handled here and not in a separate system because both decisions use
/// the drag test above, and one click can be only one of them.
/// [`super::action::run_bindings`] does the casting; this system does the pick.
#[allow(clippy::too_many_arguments)]
fn select_on_click(
    buttons: Res<ButtonInput<MouseButton>>,
    look: Res<crate::world::camera::MouseLook>,
    hovered: Res<Hovered>,
    session: Res<Session>,
    // Read only, for the one test in the empty-ground arm, which keeps the
    // selection through a click on a game object.
    object: Res<super::object::HoveredObject>,
    mut targeting: ResMut<super::action::SpellTargeting>,
    mut picked: MessageWriter<super::action::SpellTargetPicked>,
    mut placed: MessageWriter<super::action::SpellGroundPicked>,
    mut selection: ResMut<Selection>,
    // `deselectOnClick`. It decides only the empty-ground arm at the bottom of
    // this system.
    cvars: Res<crate::settings::cvars::CVars>,
    mut clicked: MessageWriter<UnitClicked>,
) {
    targeting.took_click = false;

    // Escape is not read here. Two `just_pressed(KeyCode::Escape)` tests were
    // once at the top of this function, one dismissing the spell cursor and one
    // clearing the selection. Escape is `TOGGLEGAMEMENU` in the game's shipped
    // defaults, so the key did three things at once and none of them could be
    // rebound.
    //
    // `ToggleGameMenu`'s seven-branch chain runs them now, in the game's order:
    // `SpellStopCasting()`, then `SpellStopTargeting()`, then
    // `CloseAllWindows()`, then `ClearTarget()`, and the menu opens only if
    // none of those acted. Each returns whether it acted, which stops the
    // chain. See [`crate::lua::api`], where the three are registered as reads
    // with a write attached.
    //
    // The right button does not cancel the cast, although it does in the 1.12.1
    // client. This client steers with the right button, so a right click over a
    // unit is as often a hundred-millisecond camera turn as a refusal, and
    // cancelling the cast on it would dismiss the mode unintentionally. Escape
    // and a click on empty ground are the two exits.

    // Three conditions, read from the button state and
    // [`crate::world::camera::MouseLook`]: the button was released, the press
    // landed on the world, and the pointer did not travel far enough to make
    // the gesture a look. The second replaces an `over_interface` test that was
    // further down. A click is decided by where it started: a press on the
    // world that ends over the action bar is a click on the world, and a press
    // on a button that ends over a mob is not.
    if !buttons.just_released(MouseButton::Left) || !look.on_world || look.dragged {
        return;
    }
    // The spell cursor's click: it names a unit or a place, or it cancels the
    // mode. In every case the selection is unchanged.
    if targeting.is_targeting() {
        targeting.took_click = true;
        // A placed cast reads the ground point this frame's ray already found
        // and does not cast a second ray, so the click lands where the cursor
        // showed.
        //
        // An out-of-range point is sent, not dropped, as in the unit case:
        // `cast_known_spell` runs the same `check_cast` and answers with the
        // game's "Out of range". Cancelling the mode silently would be a click
        // with no effect and no message.
        if targeting.wants_ground() {
            match targeting.over_ground {
                Some(at) => {
                    placed.write(super::action::SpellGroundPicked { at });
                }
                // The sky, or off the edge of the loaded world: cancel the
                // mode.
                None => targeting.stop(),
            }
            return;
        }
        match hovered.entity {
            Some(unit) => {
                picked.write(super::action::SpellTargetPicked { unit });
            }
            // Empty ground with the cursor up cancels the mode. The same
            // gesture clears a selection, so this arm returns before that can
            // happen.
            None => targeting.stop(),
        }
        return;
    }
    match (hovered.guid, hovered.entity) {
        // Already selected: a second click on the same unit is not a change,
        // and re-sending the selection would drop a rogue's combo points. It is
        // still a click, and the greeting is played on every click; see
        // [`UnitClicked`].
        (Some(guid), _) if selection.guid == Some(guid) => {
            clicked.write(UnitClicked { guid });
        }
        (Some(guid), Some(entity)) => {
            selection.set(guid, entity);
            tell_server(&session, Some(guid));
            clicked.write(UnitClicked { guid });
        }
        // Empty ground clears the selection, as Escape does. A game object is
        // not empty ground. This match sees a game object as no unit at all, so
        // without the `object.guid` test a click that opens a door would also
        // drop the target the player was fighting in front of it.
        //
        // `deselectOnClick` decides whether empty ground clears at all. It is
        // the game's checkbox, registered `"1"`, so clearing is on unless the
        // player turns it off. Escape is not gated on it: the CVar is about an
        // accidental click on the ground, and a key press is deliberate.
        (None, _)
            if selection.guid.is_some()
                && object.guid.is_none()
                && cvars.flag(DESELECT_ON_CLICK) =>
        {
            selection.clear();
            tell_server(&session, None);
        }
        _ => {}
    }
}

/// `TargetNearestEnemy([reverse])` and `TargetUnit("player")`: the game's
/// targeting functions, run from binding names and not from keys.
///
/// The archives' `Bindings.xml` shows that the two enemy bindings are one
/// function:
///
/// ```xml
/// <Binding name="TARGETNEARESTENEMY">  TargetNearestEnemy();  </Binding>
/// <Binding name="TARGETPREVIOUSENEMY"> TargetNearestEnemy(1); -- 1 (or "true") means reverse! </Binding>
/// ```
///
/// ## Classic's priority scheme, not the 1.12 cone
///
/// The 1.12 rule is a ±30° cone about the character's facing. It is authentic
/// and does not work behind a third-person camera: at two yards the cone is
/// about a yard wide, the camera sits nine yards back, and the mob filling the
/// screen is often forty degrees off the character's axis. Blizzard rewrote
/// targeting in 7.2 for the same reason. Classic ships screen-space selection
/// first and then a weighted score, and this system does the same.
///
/// The weights here are placeholders. The real defaults are in CVars
/// (`TargetPriorityFrustumPullIn*`, `TargetPriorityValueBank`) whose values are
/// not known to this project.
#[allow(clippy::too_many_arguments)]
fn tab_target(
    mut pressed: MessageReader<BindingPressed>,
    time: Res<Time>,
    session: Res<Session>,
    assets: Res<crate::assets::GameAssets>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::world::camera::WorldCamera>>,
    player: Query<(&WorldEntity, &Transform), With<LocalPlayer>>,
    units: Query<(Entity, &WorldEntity, &Transform)>,
    // The roster, for `TargetUnit("party1")`: the guid behind the token, which
    // is what a party frame's click needs and the only part of
    // [`super::api::Units`] this system can hold.
    //
    // `Units` itself cannot be used: it carries `Res<Selection>` for the
    // `"target"` token and this system writes `ResMut<Selection>`, which is
    // Bevy's B0002. That is a panic at startup and not a compile error, since a
    // `SystemParam` conflict is only checked when the schedule is built.
    party: Res<super::party::Party>,
    // The character's reputation. The candidate pool must give the same answer
    // as the pointer and the interface; see
    // [`crate::interface::api::can_attack_between`].
    reputation: Res<super::reputation::PlayerStanding>,
    mut selection: ResMut<Selection>,
    mut history: ResMut<TabHistory>,
    last_enemy: Res<LastEnemy>,
    // `assistAttack`, read by `AssistUnit`.
    cvars: Res<crate::settings::cvars::CVars>,
    mut errors: super::messages::UiErrors,
    mut sheathing: MessageWriter<crate::world::entities::SheathRequest>,
) {
    // Read the stream; do not `clear()` it. `action::run_bindings` reads the
    // same stream and takes the action-bar bindings. A targeting system that
    // drained the queue would consume every button press in the same frame.
    // Each reader has its own cursor, which is why these are messages; see
    // `super::events`.
    // `(reverse, friendly)` of the last nearest-unit press this frame.
    let mut wanted: Option<(bool, bool)> = None;
    for BindingPressed(binding) in pressed.read() {
        match binding {
            Binding::TargetNearestEnemy => wanted = Some((false, false)),
            Binding::TargetPreviousEnemy => wanted = Some((true, false)),
            Binding::TargetNearestFriend => wanted = Some((false, true)),
            Binding::TargetPreviousFriend => wanted = Some((true, true)),
            // `TargetLastEnemy()`: select the recorded unit again if it is
            // still in the world. Nothing recorded, or gone, does nothing.
            Binding::TargetLastEnemy => {
                if let Some(guid) = last_enemy.0.filter(|guid| selection.guid != Some(*guid)) {
                    if let Some((entity, _, _)) = units.iter().find(|(_, unit, _)| unit.guid == guid) {
                        selection.set(guid, entity);
                        tell_server(&session, Some(guid));
                    }
                }
            }
            // `AssistUnit(unit)`: select what the unit has selected. A token
            // that names no unit in view says `ERR_GENERIC_NO_TARGET`; a unit
            // with nothing selected changes nothing. With `assistAttack` on,
            // the attack starts on the new selection when it can be attacked.
            Binding::AssistUnit(token) => {
                let Ok((me, _)) = player.single() else { continue };
                let guid = match token {
                    super::api::UnitId::Player => Some(me.guid),
                    super::api::UnitId::Target => selection.guid,
                    super::api::UnitId::Party(index) => party.member(*index).map(|member| member.guid),
                    _ => None,
                };
                let Some((_, assisted, _)) =
                    guid.and_then(|guid| units.iter().find(|(_, unit, _)| unit.guid == guid))
                else {
                    errors.key("ERR_GENERIC_NO_TARGET");
                    continue;
                };
                let Some(chosen) = assisted.target.filter(|guid| *guid != 0) else {
                    continue;
                };
                let Some((entity, unit, _)) = units.iter().find(|(_, unit, _)| unit.guid == chosen) else {
                    continue;
                };
                if selection.guid != Some(chosen) {
                    selection.set(chosen, entity);
                    tell_server(&session, Some(chosen));
                }
                let attacks = cvars.flag(ASSIST_ATTACK)
                    && assets.display_tables().is_ok_and(|tables| {
                        crate::interface::api::can_attack_between(&tables, &party, &reputation, me, unit)
                    });
                if let (true, Some(active)) = (attacks, session.active.as_ref()) {
                    if let Some((mine, _, _)) = units.iter().find(|(_, unit, _)| unit.guid == me.guid) {
                        active.live.attack(Some(chosen));
                        sheathing.write(crate::world::entities::SheathRequest {
                            entity: mine,
                            state: vale_assets::look::sheath::MELEE,
                        });
                    }
                }
            }
            // The game's `TARGETSELF` binding targets the pet if the player is
            // already selected; this client has no pet. See
            // `bindings::Binding`.
            Binding::TargetSelf => {
                if let Ok((me, _)) = player.single() {
                    if let Some((entity, _, _)) =
                        units.iter().find(|(_, unit, _)| unit.guid == me.guid)
                    {
                        selection.set(me.guid, entity);
                        tell_server(&session, Some(me.guid));
                    }
                }
            }
            // A party frame's click. The unit must be in the world for this to
            // act: `CMSG_SET_SELECTION` names a guid the server checks against
            // what we can see, and the 1.12.1 client cannot select a member
            // across the zone either. So this arm does nothing in that case and
            // does not send a guid with no entity.
            Binding::TargetToken(token) => {
                let super::api::UnitId::Party(index) = token else {
                    continue;
                };
                // The roster gives the guid; the world says whether the member
                // is present. A member with no entity selects nothing, as in
                // the 1.12.1 client; see the comment on this arm.
                if let Some(guid) = party.member(*index).map(|member| member.guid) {
                    if let Some((entity, _, _)) = units.iter().find(|(_, u, _)| u.guid == guid) {
                        selection.set(guid, entity);
                        tell_server(&session, Some(guid));
                    }
                }
            }
            // `ClearTarget()`: Escape's last branch before the game menu opens.
            // It replaces a `just_pressed(KeyCode::Escape)` test that was in
            // `select_on_click`. The interface has already read whether there
            // was a target to clear (that read is what stopped the chain); this
            // arm performs the clear.
            Binding::ClearTarget => {
                if selection.guid.is_some() {
                    selection.clear();
                    tell_server(&session, None);
                }
            }
            _ => {}
        }
    }
    let Some((reverse, friendly)) = wanted else { return };
    let (Ok((camera, camera_transform)), Ok((me, standing))) = (camera.single(), player.single())
    else {
        return;
    };
    let Ok(tables) = assets.display_tables() else {
        return;
    };

    let now = time.elapsed_secs();
    if history.friendly != friendly {
        history.picks.clear();
        history.friendly = friendly;
    }
    history.picks.retain(|(_, at)| now - at < TAB_HISTORY_SECS);
    // From the character, not from the camera; see [`TARGET_RANGE`]. This was
    // once `camera_transform.translation()`, which subtracted the rig's
    // distance from the reach and made it change with the zoom wheel.
    let standing = standing.translation;

    // The reverse binding walks back to the most recent history entry that is
    // still in the world. It undoes a press that went one target too far.
    if reverse {
        if let Some((guid, entity)) = history
            .picks
            .iter()
            .rev()
            .filter(|(guid, _)| Some(*guid) != selection.guid)
            .find_map(|(guid, _)| {
                units
                    .iter()
                    .find(|(_, unit, _)| unit.guid == *guid)
                    .map(|(entity, unit, _)| (unit.guid, entity))
            })
        {
            selection.set(guid, entity);
            tell_server(&session, Some(guid));
        }
        return;
    }

    let mut best: Option<(f32, Entity, u64)> = None;
    for (entity, unit, transform) in &units {
        if unit.is_self || !is_unit(unit) || unit.dead {
            continue;
        }
        // The pool: the units the character may attack, or for the friendly
        // pair the ones it may assist. The 1.12.1 client's friendly test is
        // `UnitCanAssist` and health above zero, which `unit.dead` covers.
        let eligible = if friendly {
            crate::interface::api::can_assist_between(&tables, &party, &reputation, me, unit)
        } else {
            crate::interface::api::can_attack_between(&tables, &party, &reputation, me, unit)
        };
        if !eligible {
            continue;
        }
        let distance = standing.distance(transform.translation);
        if distance > TARGET_RANGE {
            continue;
        }
        // Skip what this cycle has already handed out; exhausting the pool
        // clears the history, which is the wrap.
        if history.picks.iter().any(|(guid, _)| *guid == unit.guid) {
            continue;
        }
        let Some(score) = screen_score(camera, camera_transform, transform, distance) else {
            continue;
        };
        // A unit already fighting us scores better than a peaceful one, but
        // only by a bonus, so Tab can still move off it. A hard combat lock
        // would prevent that.
        let score = score - if !friendly && unit.target == me.target && unit.in_combat { 0.5 } else { 0.0 };
        if best.is_none_or(|(lowest, ..)| score < lowest) {
            best = Some((score, entity, unit.guid));
        }
    }

    match best {
        Some((_, entity, guid)) => {
            if let Some(previous) = selection.guid {
                history.picks.push((previous, now));
            }
            history.picks.push((guid, now));
            selection.set(guid, entity);
            tell_server(&session, Some(guid));
        }
        // Nothing left to hand out: clear the history so the next press starts
        // the cycle again rather than doing nothing forever.
        None => history.picks.clear(),
    }
}

/// Lower is better: how far from the centre of the screen a unit is, plus how
/// far away it is. `None` for a unit the camera cannot see.
///
/// The frustum edges are pulled in towards the centre, which is the mechanism
/// the client's `TargetPriorityFrustumPullIn{Sides,Top,Bot}` CVars name. A
/// candidate at the very edge of the screen is unlikely to be the one the
/// player wants.
fn screen_score(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    unit: &Transform,
    distance: f32,
) -> Option<f32> {
    const PULL_IN: f32 = 0.1;
    let Ok(point) = camera.world_to_viewport(camera_transform, unit.translation + Vec3::Y) else {
        return None;
    };
    let size = camera.logical_viewport_size()?;
    let centred = (point / size - Vec2::splat(0.5)) * 2.0;
    if centred.x.abs() > 1.0 - PULL_IN || centred.y.abs() > 1.0 - PULL_IN {
        return None;
    }
    Some(centred.length() + distance / TARGET_RANGE)
}

/// Selects a unit that swung at us when nothing is selected.
///
/// This is the client's auto-acquire on `SMSG_ATTACKERSTATEUPDATE`, which
/// arrives here as [`WorldEntity::swings_thrown`] changing on a unit whose
/// target is us. Without it a fight that starts behind the player has no target
/// frame.
fn acquire_attacker(
    session: Res<Session>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    units: Query<(Entity, &WorldEntity, Option<&SeenSwings>), Changed<WorldEntity>>,
    mut commands: Commands,
    mut selection: ResMut<Selection>,
) {
    let Ok(me) = player.single() else { return };
    for (entity, unit, seen) in &units {
        let previous = seen.map(|seen| seen.0);
        // The latch is a component and not a map keyed by guid, as in the
        // missile pass and for the same reason. A `Local<HashMap>` outlives
        // every entity in it, so a session that crosses three zones keeps a
        // counter for every creature it has seen. A component is dropped with
        // its entity.
        commands.entity(entity).insert(SeenSwings(unit.swings_thrown));
        if selection.guid.is_some() || unit.is_self {
            continue;
        }
        // A first look adopts the counter without acting on it: a creature that
        // walks into view mid-fight must not be adopted as an attacker on the
        // strength of swings thrown before it existed.
        let Some(previous) = previous else { continue };
        if unit.swings_thrown != previous && unit.target == Some(me.guid) {
            selection.set(unit.guid, entity);
            tell_server(&session, Some(unit.guid));
        }
    }
}

/// Drops a selection whose unit has streamed out. A dead unit stays selected.
///
/// Nothing is shown to the player. This system once put "target gone" on
/// screen, a sentence this client composed. The game has no string for losing a
/// target: the 1.12.1 client says nothing and empties the frame.
/// [`super::messages`] explains why that module forbids composed lines. The
/// `PLAYER_TARGET_CHANGED` that `announce_target_change` writes is the only
/// notification, and the target frame waits on it.
fn drop_stale_selection(
    session: Res<Session>,
    units: Query<&WorldEntity>,
    mut selection: ResMut<Selection>,
) {
    let Some(entity) = selection.entity else { return };
    let gone = match units.get(entity) {
        // A dead target stays selected in the 1.12.1 client, because it is
        // looted. Only a unit that no longer exists clears the selection. The
        // arm is explicit so the difference is visibly intended.
        Ok(_) => false,
        Err(_) => true,
    };
    if gone {
        selection.clear();
        tell_server(&session, None);
    }
}

/// A right click on a unit selects it and then acts on it: it loots a lootable
/// corpse, opens a conversation with a service NPC, or starts the melee swing
/// at a unit we may attack.
///
/// The right button is a different action from [`select_on_click`]: the left
/// button picks, the right button acts on what it picks. In the 1.12.1 client a
/// right click on a unit is one gesture with two effects: the selection
/// changes, and then the unit is interacted with, which for an attackable unit
/// is the swing and for a friendly NPC is the interact (gossip, vendor, flight
/// master, loot).
///
/// A friendly unit with no service and no loot is selected and nothing more.
///
/// The select-then-attack behaviour is not measured against the 1.12.1 client
/// or vmangos, unlike the other targeting rules in this module. It is the
/// game's commonly known behaviour (right click a mob and the character walks
/// up and hits it) implemented with systems that already existed.
///
/// Three things it does not do:
///
/// * Empty ground does not clear the selection. Only the left button does that.
///   A player who right-clicks the ground to turn the camera would otherwise
///   drop the target whenever the gesture was short.
/// * A second right click does not stop the attack. [`super::action`]'s
///   `AttackTarget` is a toggle because the button is one; the mouse is not,
///   and right-clicking the unit already being fought must not stop the attack.
/// * It does nothing while the spell cursor is up. The right button steers the
///   camera in this client, so the 1.12.1 client's "right click cancels the
///   cast" is not available (see [`select_on_click`]), and selecting a unit
///   while the client waits for a spell target would be worse than doing
///   nothing.
#[allow(clippy::too_many_arguments)]
fn attack_on_right_click(
    buttons: Res<ButtonInput<MouseButton>>,
    look: Res<crate::world::camera::MouseLook>,
    targeting: Res<super::action::SpellTargeting>,
    hovered: Res<Hovered>,
    session: Res<Session>,
    player: Query<Entity, With<LocalPlayer>>,
    mut selection: ResMut<Selection>,
    mut sheathing: MessageWriter<crate::world::entities::SheathRequest>,
    mut clicked: MessageWriter<UnitClicked>,
) {
    // The same three conditions [`select_on_click`] tests for the left button,
    // plus a fourth: with the left button also down the gesture is autorun, not
    // a click. `MouseLook` arms once per gesture and not once per button, so
    // releasing the right button during a both-button run would otherwise read
    // as a right click that did not travel.
    if !buttons.just_released(MouseButton::Right)
        || buttons.pressed(MouseButton::Left)
        || !look.on_world
        || look.dragged
        || targeting.is_targeting()
    {
        return;
    }
    let (Some(guid), Some(entity)) = (hovered.guid, hovered.entity) else {
        return;
    };
    // Re-selecting what is already selected is a dropped combo point — the same
    // reason the left button checks.
    if selection.guid != Some(guid) {
        selection.set(guid, entity);
        tell_server(&session, Some(guid));
    }

    // The answers [`judge_the_hover`] computed once, so the cursor shown and
    // the action taken here agree.
    let Ok(me_entity) = player.single() else { return };
    // A lootable corpse is looted. A wrong guess costs nothing: `CMSG_LOOT` on
    // a corpse this player may not loot is answered with `SMSG_LOOT_RESPONSE`
    // with a type of zero and an `ERR_LOOT_*` line, which is what the 1.12.1
    // client shows. The server decides permission. This client asks only when
    // the pointer shows something to loot, which is `UNIT_DYNAMIC_FLAGS` and is
    // per-viewer.
    //
    // Loot is tested before the attack, the order the cursor rule uses. A
    // lootable corpse cannot be attacked, so the two never compete, and using
    // one order in both places keeps them in agreement.
    if hovered.lootable {
        if let Some(ActiveSession { live, .. }) = session.active.as_ref() {
            live.loot(guid);
        }
        return;
    }
    // A unit that offers a service is interacted with. The opener is chosen by
    // the precedence of the NPC flags; see [`Interact`]. The reply drives
    // `interface::gossip`, `interface::quest` or `interface::merchant`; nothing
    // here waits for it.
    if let Some(interact) = hovered.interact {
        // Opening a conversation is a click on a unit, and the greeting belongs
        // to the click and not to the window; see [`UnitClicked`]. It is raised
        // whether or not there is a session to carry the packet, so the
        // headless probes see the same edge a player does.
        clicked.write(UnitClicked { guid });
        if let Some(ActiveSession { live, .. }) = session.active.as_ref() {
            match interact {
                Interact::Gossip => {
                    live.npc(vale_protocol::socket::session::NpcVerb::GossipHello(guid));
                }
                Interact::Questgiver => {
                    live.quest(vale_protocol::socket::session::QuestVerb::Hello(guid));
                }
                Interact::Vendor => {
                    live.npc(vale_protocol::socket::session::NpcVerb::ListInventory(guid));
                }
                Interact::Taxi => {
                    live.taxi(vale_protocol::socket::session::TaxiVerb::QueryNodes(guid));
                }
                Interact::Trainer => {
                    live.npc(vale_protocol::socket::session::NpcVerb::TrainerList(guid));
                }
                Interact::Banker => {
                    live.npc(vale_protocol::socket::session::NpcVerb::BankerActivate(guid));
                }
                Interact::Petitioner => {
                    live.petition(vale_protocol::socket::session::PetitionVerb::ShowList(guid));
                }
                Interact::TabardDesigner => {
                    live.guild(vale_protocol::socket::session::GuildVerb::TabardVendor(guid));
                }
            }
        }
        return;
    }
    if !hovered.attackable {
        return;
    }
    if let Some(ActiveSession { live, .. }) = session.active.as_ref() {
        // Already swinging at this unit: the packet would be a no-op and the
        // weapon is already drawn. Do nothing; do not stop the attack.
        if live.attacking() == Some(guid) {
            return;
        }
        live.attack(Some(guid));
    }
    // Starting an attack draws the weapon, for the reason
    // `super::action::attack_target` gives: the server never changes the sheath
    // state, so without this the character swings a sheathed weapon until the
    // reconcile corrects it a round trip later.
    sheathing.write(crate::world::entities::SheathRequest {
        entity: me_entity,
        state: vale_assets::look::sheath::MELEE,
    });
}

/// `assistAttack`: whether `AssistUnit` also starts the attack. Registered
/// `"0"`.
const ASSIST_ATTACK: &str = "assistAttack";

/// Record the selection as [`LastEnemy`] when it changes to a unit the
/// character can attack.
#[allow(clippy::too_many_arguments)]
fn remember_enemy(
    selection: Res<Selection>,
    assets: Res<crate::assets::GameAssets>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    units: Query<&WorldEntity>,
    party: Res<super::party::Party>,
    reputation: Res<super::reputation::PlayerStanding>,
    mut last: ResMut<LastEnemy>,
    mut seen: Local<Option<u64>>,
) {
    if *seen == selection.guid {
        return;
    }
    *seen = selection.guid;
    let (Some(guid), Ok(me)) = (selection.guid, player.single()) else {
        return;
    };
    let Some(unit) = units.iter().find(|unit| unit.guid == guid) else {
        return;
    };
    if unit.health_value.is_none() {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    if crate::interface::api::can_attack_between(&tables, &party, &reputation, me, unit) {
        last.0 = Some(guid);
    }
}

/// Is this thing selectable at all? Units and players are; game objects,
/// dynamic objects and items are not.
fn is_unit(unit: &WorldEntity) -> bool {
    use vale_protocol::state::update::ObjectType;
    matches!(unit.kind, ObjectType::Unit | ObjectType::Player)
}

/// Whether the entity is a game object. Game objects are hovered and never
/// selected; see [`super::object`].
fn is_game_object(entity: &WorldEntity) -> bool {
    entity.kind == vale_protocol::state::update::ObjectType::GameObject
}

/// Whether the pick admits this game object: only objects the pointer has
/// something to show for. See [`vale_assets::look::object::worth_pointing_at`]
/// for the two conditions.
///
/// The filter exists to protect the units behind game objects. A zone holds
/// many game objects that have geometry here and that a click cannot act on:
/// the spell focuses (anvils and forges), the transports, and 468 of the 1,869
/// inert `GAMEOBJECT_TYPE_GENERIC` rows. Each is a `WorldEntity` with a
/// `Transform` and a pick mesh. Without the filter they enter the broad phase
/// and can win the depth test, and because a game object's result goes to
/// `HoveredObject`, the unit behind is cleared from [`Hovered`]. Measured: a
/// pointer swept across Northshire hovered four of them, and on each such frame
/// no unit could be targeted.
///
/// The other 1,197 generic rows are the street signs and are admitted. A
/// signpost arm is a game object whose only purpose is to be hovered.
/// `worth_pointing_at` separates it from the scenery, using the template.
/// Excluding the whole type, as an earlier version of this filter did, excludes
/// the signs.
///
/// An unresolved template reads as usable. `Kind::of(0)` is `Door`, and zero is
/// also what `object_kind` holds during the round trip `CMSG_GAMEOBJECT_QUERY`
/// takes. This makes a newly streamed door clickable a frame early; the
/// opposite choice would make a real door unclickable permanently.
fn is_pickable_object(entity: &WorldEntity) -> bool {
    is_game_object(entity)
        && (vale_assets::look::object::Kind::of(entity.object_kind).usable()
            || entity.object_hover.floating)
}

/// Tells the server the new selection, and stops the melee swing if its target
/// is no longer the unit selected.
///
/// The stop is required; without it the melee auto-attack cannot be stopped.
/// `attack_target` toggles by comparing `live.attacking()` against the current
/// selection, so once the two differ a press can only start a new attack and no
/// reachable state stops the old one. With the target cleared the button has
/// nothing to compare. The reported symptom was: "it cannot be cleared until
/// you re-select a target and manually click off".
///
/// The server does not stop the swing. `HandleSetSelectionOpcode`
/// (`MiscHandler.cpp:399`) updates the selection, drops a rogue's or druid's
/// combo points, and cancels the auto-shot when the new target is not
/// attackable. It does nothing about the melee swing, which continues against
/// the old unit. A client that does not send `CMSG_ATTACKSTOP` here leaves a
/// swing the player cannot see and cannot end.
///
/// A switch of target stops the attack and does not move it to the new target.
/// Stopping is one packet that the player can undo with one press; following
/// would attack a unit the player had only clicked to inspect.
fn tell_server(session: &Session, guid: Option<u64>) {
    let Some(ActiveSession { live, .. }) = session.active.as_ref() else {
        return;
    };
    // The stop is sent before the selection change, so the two packets reach
    // the socket in the order the events happened. `cancel_cast` orders a
    // cancel behind its cast for the same reason.
    if breaks_off(live.attacking(), guid) {
        live.attack(None);
    }
    live.target(guid);
}

/// Whether selecting `selecting` ends the swing at `attacking`. This is the
/// rule [`tell_server`] applies, as a function so a test can check it.
///
/// True for a switch and for a clear; false for re-selecting the same unit. A
/// right click that selects and then attacks tells the server the same guid
/// twice, and without this guard the rule would stop the attack it had just
/// started.
fn breaks_off(attacking: Option<u64>, selecting: Option<u64>) -> bool {
    attacking.is_some_and(|at| Some(at) != selecting)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A swing ends when the selection moves off its target, and not otherwise.
    ///
    /// `attack_target` toggles by comparing the unit being attacked against the
    /// unit selected, so once those differ a press can only start a new attack:
    /// no reachable state stops the old one, and the server does not stop it
    /// either (`HandleSetSelectionOpcode` leaves the melee swing alone).
    #[test]
    fn moving_the_selection_off_a_unit_ends_the_swing_at_it() {
        assert!(breaks_off(Some(7), None), "un-targeting");
        assert!(breaks_off(Some(7), Some(9)), "switching");
        assert!(
            !breaks_off(Some(7), Some(7)),
            "re-picking the same unit must not stop the attack it just started"
        );
        assert!(!breaks_off(None, Some(9)), "nothing to stop");
        assert!(!breaks_off(None, None));
    }

    fn transform(at: Vec3) -> Transform {
        Transform::from_translation(at)
    }

    /// `UPDATE_MOUSEOVER_UNIT` is raised only when the token names a unit. The
    /// event's one shipped handler recolours the tooltip's first line with
    /// `GameTooltip_UnitColor("mouseover")`, and that function returns white
    /// for a token naming no unit. Raising the event when the pointer leaves
    /// therefore repaints the name white. That was not visible while the
    /// tooltip was hidden on the same frame, and is visible now that the
    /// tooltip holds and fades.
    #[test]
    fn the_mouseover_event_is_raised_for_a_unit_and_never_for_the_absence_of_one() {
        let mut app = App::new();
        app.init_resource::<Hovered>()
            .add_message::<super::super::events::MouseoverUnitChanged>()
            .add_systems(Update, announce_mouseover_change);

        let raised = |app: &mut App| {
            app.update();
            app.world_mut()
                .resource_mut::<Messages<super::super::events::MouseoverUnitChanged>>()
                .drain()
                .count()
        };

        // The first frame adopts without announcing — nothing had registered yet.
        assert_eq!(raised(&mut app), 0);

        // The pointer arrives on a unit: that is the event.
        app.world_mut().resource_mut::<Hovered>().guid = Some(7);
        assert_eq!(raised(&mut app), 1);
        // Holding still is not an event.
        assert_eq!(raised(&mut app), 0);

        // The pointer leaves: no event, or the tooltip turns white as it fades.
        app.world_mut().resource_mut::<Hovered>().guid = None;
        assert_eq!(raised(&mut app), 0, "the empty edge repainted the plate");

        // Coming back is a change again, even to the same unit, because the
        // colour on the tooltip was set for the previous mouseover.
        app.world_mut().resource_mut::<Hovered>().guid = Some(7);
        assert_eq!(raised(&mut app), 1);

        // Moving straight from one unit to another, with no gap, is also a
        // change.
        app.world_mut().resource_mut::<Hovered>().guid = Some(8);
        assert_eq!(raised(&mut app), 1);
    }

    /// An app with [`attack_on_right_click`] in it, a unit under the pointer
    /// and a local player to draw the weapon on.
    ///
    /// `Hovered::attackable` starts false. [`judge_the_hover`] sets it, not
    /// this system, so these tests drive the gesture, the selection and the use
    /// the click makes of that flag. Whether a given unit is attackable is
    /// [`can_attack`]'s rule, tested where that lives.
    fn clicking_app() -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<Selection>()
            .init_resource::<Hovered>()
            .init_resource::<Session>()
            .init_resource::<super::super::action::SpellTargeting>()
            .init_resource::<crate::world::camera::MouseLook>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<crate::world::entities::SheathRequest>()
            .add_message::<UnitClicked>()
            .add_systems(Update, attack_on_right_click);
        let unit = app
            .world_mut()
            .spawn(WorldEntity { guid: 42, ..Default::default() })
            .id();
        app.world_mut()
            .spawn((WorldEntity { guid: 1, is_self: true, ..Default::default() }, LocalPlayer));
        app.world_mut().resource_mut::<Hovered>().guid = Some(42);
        app.world_mut().resource_mut::<Hovered>().entity = Some(unit);
        (app, unit)
    }

    /// How many weapon draws the frame asked for: the observable result of the
    /// attack branch when there is no session to send a packet on.
    fn draws(app: &App) -> usize {
        app.world()
            .resource::<bevy::ecs::message::Messages<crate::world::entities::SheathRequest>>()
            .len()
    }

    /// Releases the right button and runs a frame.
    ///
    /// The button is pressed and then released, because `ButtonInput::release`
    /// records nothing for a button that was never down. A helper that only
    /// released would make every test here pass without exercising the system.
    fn release_right(app: &mut App, dragged: bool) {
        {
            let mut look = app.world_mut().resource_mut::<crate::world::camera::MouseLook>();
            look.on_world = true;
            look.dragged = dragged;
        }
        let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        buttons.press(MouseButton::Right);
        buttons.release(MouseButton::Right);
        app.update();
    }

    /// A right *click*: on the world, having travelled nothing.
    fn right_click(app: &mut App) {
        release_right(app, false);
    }

    /// The right button selects as well as the left. Before this system existed
    /// only the left button selected, so a player who plays with the right
    /// button had no way to pick a target.
    #[test]
    fn a_right_click_on_a_unit_selects_it() {
        let (mut app, unit) = clicking_app();
        right_click(&mut app);
        let selection = app.world().resource::<Selection>();
        assert_eq!(selection.guid, Some(42));
        assert_eq!(selection.entity, Some(unit));
    }

    /// An app with [`select_on_click`] in it and a unit under the pointer.
    fn left_clicking_app() -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<Selection>()
            .init_resource::<Hovered>()
            .init_resource::<Session>()
            .init_resource::<super::super::object::HoveredObject>()
            .init_resource::<super::super::action::SpellTargeting>()
            .init_resource::<crate::world::camera::MouseLook>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<crate::settings::cvars::CVars>()
            .add_message::<super::super::action::SpellTargetPicked>()
            .add_message::<super::super::action::SpellGroundPicked>()
            .add_message::<UnitClicked>()
            .add_systems(Update, select_on_click);
        let unit = app
            .world_mut()
            .spawn(WorldEntity { guid: 42, ..Default::default() })
            .id();
        app.world_mut().resource_mut::<Hovered>().guid = Some(42);
        app.world_mut().resource_mut::<Hovered>().entity = Some(unit);
        (app, unit)
    }

    /// Let the left button go on the world, having travelled nothing, and run
    /// a frame.
    fn left_click(app: &mut App) {
        {
            let mut look = app.world_mut().resource_mut::<crate::world::camera::MouseLook>();
            look.on_world = true;
            look.dragged = false;
        }
        let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        buttons.press(MouseButton::Left);
        buttons.release(MouseButton::Left);
        app.update();
    }

    /// The clicks on a unit the frame announced.
    fn clicks(app: &mut App) -> Vec<UnitClicked> {
        app.world_mut()
            .resource_mut::<bevy::ecs::message::Messages<UnitClicked>>()
            .drain()
            .collect()
    }

    /// Every click on a unit is announced, including a second click on the same
    /// unit. The selection does not change on the second click (re-sending it
    /// would drop a rogue's combo points), but the greeting belongs to the
    /// click, and a guard clicked twice plays `hello` twice.
    #[test]
    fn a_click_on_the_unit_already_selected_is_still_a_click() {
        let (mut app, unit) = left_clicking_app();
        left_click(&mut app);
        assert_eq!(app.world().resource::<Selection>().guid, Some(42));
        assert_eq!(app.world().resource::<Selection>().entity, Some(unit));
        assert_eq!(clicks(&mut app), vec![UnitClicked { guid: 42 }]);

        left_click(&mut app);
        assert_eq!(app.world().resource::<Selection>().guid, Some(42), "unchanged");
        assert_eq!(clicks(&mut app), vec![UnitClicked { guid: 42 }], "and announced again");

        // Empty ground clears the selection and is not a click on anybody.
        app.world_mut().resource_mut::<Hovered>().guid = None;
        app.world_mut().resource_mut::<Hovered>().entity = None;
        left_click(&mut app);
        assert_eq!(app.world().resource::<Selection>().guid, None);
        assert!(clicks(&mut app).is_empty());
    }

    /// A right drag is a camera look and selects nothing. It is the commonest
    /// gesture in the game, because the right button is the one players steer
    /// with, so a failure here would retarget on most camera turns.
    #[test]
    fn a_right_drag_is_a_look_and_never_a_pick() {
        let (mut app, _) = clicking_app();
        release_right(&mut app, true);
        assert_eq!(app.world().resource::<Selection>().guid, None);
    }

    /// Both buttons down is autorun, not a click. `MouseLook` arms once per
    /// gesture and not once per button, so releasing the right button during a
    /// both-button run reads as a right release that did not travel. Without
    /// the guard the character would select whatever unit it was running
    /// towards.
    #[test]
    fn letting_go_of_one_half_of_autorun_is_not_a_click() {
        let (mut app, _) = clicking_app();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        right_click(&mut app);
        assert_eq!(app.world().resource::<Selection>().guid, None);
    }

    /// A right click on empty ground does not clear the selection; a left click
    /// does. A right click on the ground is how the camera is turned, and a
    /// player would otherwise drop the target whenever such a turn was short
    /// enough to count as a click.
    #[test]
    fn a_right_click_on_nothing_keeps_the_target() {
        let (mut app, unit) = clicking_app();
        {
            let mut selection = app.world_mut().resource_mut::<Selection>();
            selection.set(42, unit);
        }
        {
            let mut hovered = app.world_mut().resource_mut::<Hovered>();
            hovered.guid = None;
            hovered.entity = None;
        }
        right_click(&mut app);
        assert_eq!(app.world().resource::<Selection>().guid, Some(42));
    }

    /// A friendly NPC with no service is selected and not attacked. The click
    /// reads the same answer the sword cursor is drawn from, so a pointer that
    /// shows no sword cannot start a fight and one that shows a sword always
    /// does.
    #[test]
    fn only_an_attackable_hover_draws_the_weapon() {
        let (mut app, _) = clicking_app();
        right_click(&mut app);
        assert_eq!(app.world().resource::<Selection>().guid, Some(42), "still selected");
        assert_eq!(draws(&app), 0, "a friendly hover started a fight");

        let (mut app, _) = clicking_app();
        app.world_mut().resource_mut::<Hovered>().attackable = true;
        right_click(&mut app);
        assert_eq!(draws(&app), 1, "the weapon comes out on the press");
    }

    /// The spell cursor takes precedence. The right button steers the camera in
    /// this client, so it cannot be the cancel it is in the 1.12.1 client. It
    /// must not select a unit while the client is waiting for a spell target.
    #[test]
    fn the_spell_cursor_declines_the_right_button() {
        let (mut app, _) = clicking_app();
        app.world_mut()
            .resource_mut::<super::super::action::SpellTargeting>()
            .begin(133, crate::interface::action::Asking::Unit);
        right_click(&mut app);
        assert_eq!(app.world().resource::<Selection>().guid, None);
    }

    /// A model-space sphere for the tests below: a standing humanoid's, as
    /// `vale pick` reads it off `HumanMale` — centred a little behind the
    /// origin and a yard up, radius 1.12.
    fn standing() -> vale_assets::look::pick::Sphere {
        vale_assets::look::pick::Sphere {
            centre: [-0.12, 0.0, 1.0],
            radius: 1.12,
        }
    }

    /// The broad-phase sphere stands *on* the entity's position and covers its
    /// body — one centred on the feet would be half underground and a click on
    /// a character's chest would miss.
    #[test]
    fn the_pick_sphere_covers_the_model_and_not_the_ground_under_it() {
        let (centre, radius) = place_sphere(&transform(Vec3::ZERO), standing());
        let hit = |y: f32| {
            vale_assets::look::pick::ray_sphere(
                [0.0, y, 10.0],
                [0.0, 0.0, -1.0],
                f32::MAX,
                centre.to_array(),
                radius,
            )
        };
        assert!(hit(1.0).is_some(), "chest height");
        // Under the feet is a miss, and so is well over the head.
        assert_eq!(hit(-1.0), None);
        assert_eq!(hit(3.0), None);
    }

    /// The centre goes through the whole transform, not only the translation.
    ///
    /// A standing box is centred a little behind the model's origin, so a
    /// sphere only translated onto a unit facing east sits half a yard to one
    /// side of it. Half a turn puts the offset on the opposite side, which is
    /// what this test measures.
    #[test]
    fn the_spheres_centre_turns_with_the_unit() {
        let mut turned = transform(Vec3::ZERO);
        turned.rotation = Quat::from_rotation_y(std::f32::consts::PI);
        let (facing_north, _) = place_sphere(&transform(Vec3::ZERO), standing());
        let (facing_south, _) = place_sphere(&turned, standing());
        assert!(
            (facing_north.z + facing_south.z).abs() < 1e-5,
            "the offset did not turn: {facing_north:?} against {facing_south:?}"
        );
        assert!(facing_north.z.abs() > 0.1, "there was no offset to turn");
    }

    /// The scale is the entity's, not the model's. A tauren and a gnome share
    /// the pick code but not the sphere, and `place_entities` has already put
    /// `OBJECT_FIELD_SCALE_X` on the transform.
    #[test]
    fn the_sphere_follows_the_entitys_own_scale() {
        let mut big = transform(Vec3::ZERO);
        big.scale = Vec3::splat(3.0);
        let (_, small) = place_sphere(&transform(Vec3::ZERO), standing());
        let (centre, large) = place_sphere(&big, standing());
        assert!((large - small * 3.0).abs() < 1e-4, "{small} -> {large}");
        // The centre is lifted by the same scale; otherwise a giant's sphere
        // sits round its knees.
        assert!((centre.y - 3.0).abs() < 1e-4, "{centre:?}");
    }

    /// A model that declares no extent still has to be clickable, or it cannot
    /// be played against at all.
    #[test]
    fn a_model_with_no_declared_extent_still_has_a_sphere() {
        let (_, radius) = place_sphere(&transform(Vec3::ZERO), Default::default());
        assert_eq!(radius, MIN_PICK_RADIUS);
    }

    /// The sphere is the playing clip's. Falling back to the model's header
    /// sphere caused the wisp bug.
    ///
    /// `Creature\Wisp\Wisp.m2` declares a 12.8-yard cube covering the dust it
    /// emits; the wisp inside it is about a yard across. With the header sphere
    /// the wisp is hovered from six yards away. The numbers are from `vale
    /// pick`.
    #[test]
    fn the_sphere_comes_from_the_clip_and_not_from_the_file() {
        let header = vale_assets::look::pick::Sphere {
            centre: [0.02, 0.0, 0.02],
            radius: 6.42,
        };
        let stand = vale_assets::world::m2::M2Sequence {
            id: 0,
            variation: 0,
            start: 0,
            end: 1000,
            move_speed: 0.0,
            flags: 0,
            probability: 0x7fff,
            bounds: [[-1.62, -1.76, -1.84], [2.04, 1.90, 2.90]],
            radius: 2.79,
        };
        let playing = vale_assets::look::pick::sequence_sphere(header, Some(&stand));
        assert!((playing.radius - 2.79).abs() < 1e-5);

        // Four yards out, level with the wisp: inside the header sphere, outside
        // the one the animation states.
        let at = |sphere: vale_assets::look::pick::Sphere| {
            let (centre, radius) = place_sphere(&transform(Vec3::ZERO), sphere);
            vale_assets::look::pick::ray_sphere(
                [0.0, 4.0, 10.0],
                [0.0, 0.0, -1.0],
                f32::MAX,
                centre.to_array(),
                radius,
            )
        };
        assert!(at(header).is_some(), "the file's own box reaches four yards up");
        assert_eq!(at(playing), None, "the animation's does not");
    }

    /// The narrow phase tests the triangle where the model is drawn, in world
    /// coordinates. This is the absolute check.
    ///
    /// `solid.rs` argues for this form of test: the relative test below can
    /// pass while the result is wrong by an axis swap, because both of its
    /// sides are built from the same assumption. This test states a number. It
    /// uses a one-triangle model with one bone and a unit standing at a stated
    /// WoW position and facing west. The ray is aimed at where that triangle
    /// must be in Bevy's frame, computed here from the axis rule and not from
    /// anything the pick does.
    #[test]
    fn a_posed_triangle_is_hit_where_the_model_is_drawn() {
        use crate::render::axes;

        // A yard-wide plate lying flat a yard and a half up, offset onto the
        // model's +X (north) side. The three model-space coordinates are
        // distinct and none is zero on two axes, so a transposed axis cannot
        // pass by coincidence. The plate is flat so that a ray from above
        // crosses it and does not lie in its plane.
        let mut mesh = vale_assets::look::pick::PickMesh::default();
        mesh.positions
            .extend([[1.5, -0.5, 1.5], [1.5, 0.5, 1.5], [0.5, 0.0, 1.5]]);
        mesh.weights.extend([[255, 0, 0, 0]; 3]);
        mesh.bones.extend([[0, 0, 0, 0]; 3]);
        mesh.indices.extend([0u16, 1, 2]);

        // Where the unit stands, in the server's frame, and its facing: a
        // quarter turn counter-clockwise from north, which is west.
        let tracked = [-9470.0, 60.0, 56.0];
        let facing = std::f32::consts::FRAC_PI_2;
        let placement = Transform {
            translation: axes::to_bevy(tracked),
            rotation: axes::body(facing, 0.0),
            scale: Vec3::ONE,
        };
        // The joint the GPU would skin with: `world_from_model * B * P * B⁻¹`
        // with an identity bone, which is the placement itself.
        let bone = placement.to_matrix();

        // The triangle's centre, computed in WoW space and converted once: the
        // model's +X is west after the turn, so the centre is 1.17 yards west
        // of the unit and a yard and a half up.
        let centre_model = [7.0 / 6.0, 0.0, 1.5];
        let centre_world = [
            tracked[0] + centre_model[0] * facing.cos() - centre_model[1] * facing.sin(),
            tracked[1] + centre_model[0] * facing.sin() + centre_model[1] * facing.cos(),
            tracked[2] + centre_model[2],
        ];
        let aim = axes::to_bevy(centre_world);

        // Straight down at it from twenty yards up, which crosses the plate's
        // plane and does not lie in it.
        let ray = Ray3d::new(aim + Vec3::Y * 20.0, Dir3::NEG_Y);
        let hit = hit_posed_mesh(&ray, &mesh, &[bone], placement.to_matrix());
        let hit = hit.expect("the triangle is under the ray");
        assert!((hit - 20.0).abs() < 0.01, "hit {hit} yards down, wanted 20");

        // A ray two yards to the side is a miss, so the assertion above is not
        // passing on a hull that covers the surrounding area.
        let beside = Ray3d::new(aim + Vec3::X * 2.0 + Vec3::Y * 20.0, Dir3::NEG_Y);
        assert_eq!(hit_posed_mesh(&beside, &mesh, &[bone], placement.to_matrix()), None);
    }

    /// The relative check: the pick follows the pose, which is the purpose of
    /// stage two.
    ///
    /// The same ray and the same model with two different bone matrices: with
    /// the arm down the ray misses, with the arm up it hits. A pick built on a
    /// box, or on the bind pose, gives the same answer for both.
    #[test]
    fn the_narrow_phase_follows_the_bone_that_moves_it() {
        let mut mesh = vale_assets::look::pick::PickMesh::default();
        mesh.positions
            .extend([[0.0, -0.5, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 1.0]]);
        mesh.weights.extend([[255, 0, 0, 0]; 3]);
        mesh.bones.extend([[0, 0, 0, 0]; 3]);
        mesh.indices.extend([0u16, 1, 2]);

        // The triangle stands in Bevy's `z = 0` plane, a yard tall from the
        // ground; the ray crosses that plane head-on at four yards up, which is
        // where the raised bone puts it and the lowered one does not.
        let ray = Ray3d::new(Vec3::new(0.0, 4.5, -10.0), Dir3::Z);
        let down = Mat4::IDENTITY;
        let up = Mat4::from_translation(Vec3::Y * 4.0);
        assert_eq!(hit_posed_mesh(&ray, &mesh, &[down], down), None);
        assert!(hit_posed_mesh(&ray, &mesh, &[up], up).is_some());
    }

    /// The ray parameter is not the range from the character.
    ///
    /// The bug this test guards against used one value for two questions. The
    /// value `ray_sphere` returns is how far the hit is from the camera, and it
    /// was compared against [`TARGET_RANGE`], which is a distance from the
    /// character. A third-person camera sits `CameraRig::distance` behind the
    /// character, so the zoom was subtracted from the reach: at the default 25
    /// the reach was 16 yards, and it shrank with each step of the zoom wheel.
    ///
    /// The test is kept although [`hover`] no longer bounds the pick, because
    /// `tab_target` still applies the bound and must measure it from the
    /// character.
    #[test]
    fn the_ray_distance_is_not_the_range_from_the_character() {
        // A mob 20 yards in front of a character, seen from 25 yards behind: the
        // camera is 45 yards from it and the character is 20.
        let mob = transform(Vec3::new(0.0, 0.0, -20.0));
        let character = Vec3::ZERO;
        let (centre, radius) = place_sphere(&mob, standing());

        let along_ray = vale_assets::look::pick::ray_sphere(
            [0.0, 1.0, 25.0],
            [0.0, 0.0, -1.0],
            f32::MAX,
            centre.to_array(),
            radius,
        )
        .expect("a hit");
        assert!(along_ray > 43.0, "along the ray: {along_ray}");
        assert!(along_ray > TARGET_RANGE, "which is what used to refuse it");

        let reach = character.distance(mob.translation);
        assert!(reach < TARGET_RANGE, "and the character is well inside: {reach}");
    }
}
