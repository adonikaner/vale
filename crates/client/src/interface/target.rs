//! **Who the player is pointing at**, and everything that changes it.
//!
//! Nothing in the world can be acted on until it can be *named*, so this is the
//! first thing in `interface/` and everything else here reads it: the swing goes at
//! [`Selection::guid`], a cast binds against it, and the target frame draws it.
//!
//! Four rules, each the game's own:
//!
//! * **The selection is the client's, and it is immediate.** Clicking a unit
//!   selects it in that frame with no round trip — the ring appears at once, the
//!   frame fills at once — and `CMSG_SET_SELECTION` only *tells* the server
//!   afterwards. The server does several things with knowing (writes our
//!   `UNIT_FIELD_TARGET` so other people can see it, makes a faction visible,
//!   drops a rogue's combo points, re-aims an auto-shot) and none of them is
//!   permission.
//! * **Escape clears it, and so does the target dying or streaming out.** A
//!   selection that outlives its unit is a frame drawing a corpse's health and a
//!   swing sent at a guid the server has forgotten.
//! * **Tab picks the best candidate the player can see**, not the nearest thing
//!   in the world: on-screen first, then a weighted score of how far off-centre
//!   and how far away it is, with a bonus for whatever is already attacking us,
//!   and a short history so a second press moves on. See [`tab_target`] for why
//!   that is the shape rather than 1.12's own cone.
//! * **Being attacked acquires the attacker**, if nothing is selected. That is
//!   the client's own behaviour and it is what stops a fight starting with no
//!   target frame.
//!
//! ## Picking is two stages, and both of them are the reference's
//!
//! The pointer is on a unit when its ray crosses **that unit's drawn triangles,
//! at the pose it is drawn in** — which is the real client's answer and not an
//! approximation of it. `vale_assets::look::pick` is the rule;
//! [`hover`] is the join:
//!
//! 1. **the sphere of the animation it is playing**, which is a
//!    much smaller thing than the model's own declared box — that box is a union
//!    over every clip *and* every emitter the file has, and using it is what made
//!    a wisp hoverable from six yards away;
//! 2. **the triangles**, nearest candidate first, skinned with the
//!    same joints the GPU skins with, so the silhouette a click tests is the one
//!    on the screen.
//!
//! Three things about it are still not the reference's, each stated rather than
//! discovered later:
//!
//! * **a unit behind a wall is clickable.** The real pick traces the world
//!   first and lets the object win only if it is nearer;
//!   nothing here does.
//! * **every drawn triangle is tested**, where the reference skips a texture
//!   unit flagged `0x8` and, for one of its pickable classes, every non-opaque
//!   batch. Both filters would only remove triangles.
//! * **there is no distance limit on the pick at all** — if the server has told
//!   us a unit exists and the pointer is on it, it can be hovered.
//!   [`TARGET_RANGE`] bounds Tab and only Tab.
//!
//! All three err towards selecting, which is the direction that leaves the
//! player able to play; the opposite is a mob that cannot be clicked at all.

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

/// **A click landed on a unit** — each one, whether or not it changed the
/// selection.
///
/// [`Selection`] is a state and this is an edge, and the two are not the same
/// thing: a second left click on the unit already selected leaves the state
/// alone (re-sending it would drop a rogue's combo points) and is still a
/// click. What wants the edge is the greeting — `sound::npc` plays a display's
/// `hello` on *every* click, the way the reference does when a guard is
/// clicked twice — and it is raised here, by the two systems that read the
/// button, so that nothing downstream has to reconstruct a click from a
/// selection that did not move.
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
    /// **Whether a right click here would start a fight** — the same
    /// [`can_attack`] question [`tab_target`] asks, answered once a frame so
    /// that the pointer and the click cannot disagree about it.
    ///
    /// It lives on the pick rather than in `ui::cursor` because friend or foe
    /// is a *game rule*, decidable with no window open, and
    /// the cursor's job is to turn an answer into a bitmap. Written by
    /// [`judge_the_hover`].
    pub attackable: bool,
    /// **Whether a right click here would open a loot window** —
    /// `UNIT_DYNAMIC_FLAGS` bit 0, which is per-viewer and so is already the
    /// honest answer for *this* player rather than for the group.
    ///
    /// Beside [`Self::attackable`] and for the same reason: the pointer and the
    /// click read one answer, so a hand the player is looking at and what the
    /// button does cannot disagree.
    pub lootable: bool,
    /// **What a right click here says to the NPC**, or `None` for a unit with
    /// nothing to say. Beside the two above and for the same reason: what the
    /// pointer says and what the button does are one answer, computed once.
    ///
    /// The routing is the flags' own precedence — see [`Interact`].
    pub interact: Option<Interact>,
    /// **Which of the game's pointers belongs over this unit**, or `None` for
    /// the ordinary arrow.
    ///
    /// Not derivable from [`Self::attackable`], and the difference is the whole
    /// of `vale_assets::look::cursor`'s module note: the client asks
    /// `UNIT_NPC_FLAGS` *first* and falls out of that chain into the attack
    /// question, so a vendor or a quest giver shows its own cursor however
    /// attackable it is.
    pub cursor: Option<vale_assets::look::cursor::Cursor>,
}

/// **Which opcode a right click on a service NPC opens with.**
///
/// Three routes, chosen by `UNIT_NPC_FLAGS` in this order:
///
/// * **gossip** (`0x1`) — `CMSG_GOSSIP_HELLO`, and the server decides what
///   comes back: a menu, a questgiver page, or a vendor list. Nearly every
///   service NPC in the game carries this bit beside its trade.
/// * **questgiver only** (`0x2` without gossip) — `CMSG_QUESTGIVER_HELLO`,
///   which skips the menu for an NPC that has nothing but quests.
/// * **vendor only** (`0x4` with neither) — `CMSG_LIST_INVENTORY`, straight to
///   the shop.
/// * **flight master only** (`0x8` with none of the three) —
///   `CMSG_TAXIQUERYAVAILABLENODES`, which comes back as the flight map. It
///   sits *before* the trainer for the same reason the cursor does: see
///   `vale_assets::look::cursor`, whose chain tests `FLIGHTMASTER` first, and note
///   that nearly every flight master in 1.12 carries the gossip bit as well —
///   the taxi map is then a gossip *option* (icon 2) and this branch is not
///   taken at all.
/// * **trainer only** (`0x10` with none of the three) — `CMSG_TRAINER_LIST`.
///   Rare: nearly every trainer in 1.12 carries the gossip bit as well (a
///   Goldshire trainer is `0x13`), so this is the tail of the same chain
///   `vale_assets::look::cursor` walks, in the same order — which is what keeps
///   the book on the pointer and what the click does from disagreeing.
/// * **banker only** (`0x100` with none of the five) — `CMSG_BANKER_ACTIVATE`.
///   Rare on the same terms: a banker with the gossip bit opens the bank
///   from its menu, which is `GOSSIP_OPTION_BANKER` on the server's side and
///   `SendShowBank` either way. See `vale_protocol::play::bank`.
///
/// The precedence is a **reading**, stated as one: the reference's own dispatch
/// site was not followed, and what pins it is that the server's gossip handler
/// composes the other two pages into its menu — so asking the widest question
/// first can never lose a page, where the other order opens a shop over a
/// menu the NPC wanted to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interact {
    Gossip,
    Questgiver,
    Vendor,
    Taxi,
    Trainer,
    Banker,
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
        } else {
            None
        }
    }
}

/// How far **Tab** reaches, in yards — and nothing else.
///
/// The game's own `targetNearestDistance` default, 41 yards, still shipped as a
/// CVar in Classic with the description "limited to tab targeting range". That
/// description is the whole of why this constant has one reader: Tab picks a
/// candidate out of everything in the world and needs a bound to pick from,
/// where a click already has one — the pointer is on the thing or it is not. See
/// [`hover`], which used to carry this and does not.
///
/// **Measured from the character, not from the eye.** Getting that wrong is what
/// made the reach a third of what it says: `tab_target` measured from
/// `camera_transform.translation()`, so the whole of the rig's own distance came
/// off the top — at the default 25-yard zoom a mob 17 yards away was out of
/// range, and the number moved with the wheel. The CVar's name is the argument:
/// a distance to the *nearest target* is a thing the character has and the
/// camera does not.
const TARGET_RANGE: f32 = 41.0;

/// How long a Tab-cycled guid is skipped by the next press, in seconds.
///
/// The mechanism is the client's `TargetPriorityHighlightHistoryMs` — "time
/// target history should be maintained for repeated activations". The *number*
/// is a stand-in: Blizzard's default lives in a code section this project has
/// not read, and five seconds is long enough to walk a pull and short enough
/// that nobody notices the wrap.
const TAB_HISTORY_SECS: f32 = 5.0;

/// Guids Tab has recently handed out, with when.
#[derive(Resource, Default)]
struct TabHistory(Vec<(u64, f32)>);

/// How many swings this unit had thrown when [`acquire_attacker`] last looked.
///
/// The **first look adopts the counter without acting on it**: a creature that
/// walks into view mid-fight has already thrown swings, and adopting it as our
/// attacker on the strength of them would target whatever wandered past. Same
/// shape and same reason as the spell-effect pass's own first-look latch.
#[derive(Component)]
struct SeenSwings(u32);

/// This module's chain, so [`super::action`] can order itself after the whole of
/// it rather than after whichever system it happens to read from.
///
/// **The ordering is not optional**: a cast binds against the selection, so a
/// press judged before this frame's click is a spell sent at the *previous*
/// target. Two systems that both touch `Selection` are sequenced by Bevy only
/// while they both keep the conflicting access — an inheritance worth writing
/// down instead.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetSet;

/// Everything in this module, ordered.
pub struct TargetPlugin;

impl Plugin for TargetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Selection>()
            .init_resource::<Hovered>()
            .init_resource::<TabHistory>()
            .add_message::<UnitClicked>()
            .add_systems(
                Update,
                (
                    // The hover is recomputed before the click that reads it, and
                    // the whole chain runs after the world has been placed — a
                    // pick against last frame's transforms misses a running mob
                    // by its own stride.
                    hover,
                    // …and what the *pointer* has to say about it, which is
                    // asked once here rather than by whoever draws it — see
                    // [`Hovered::attackable`].
                    judge_the_hover,
                    // …and the judgement about the hover, before the click that
                    // acts on it: the spell cursor's red-or-not is the only
                    // warning the player gets, and one frame late is one frame
                    // of the wrong cursor over a corpse.
                    spell_cursor_validity,
                    // …and the other half of the same judgement, for the cast
                    // that wants a *place*. Before the click for the same
                    // reason: the click spends what this leaves behind.
                    spell_ground_under_pointer,
                    select_on_click,
                    // …and the other button, after it: they read the same
                    // `Hovered` and write the same `Selection`, and the ordering
                    // is stated rather than inherited from the conflicting
                    // access — a frame in which both buttons come up is a frame
                    // in which the last writer decides.
                    attack_on_right_click,
                    tab_target,
                    acquire_attacker,
                    // Last, so a selection that died this frame is dropped before
                    // anything is sent at it.
                    drop_stale_selection,
                    // Last of all: one `PLAYER_TARGET_CHANGED` per frame in which
                    // the selection ended up different, whichever of the four
                    // above did it. Written from a comparison rather than by each
                    // writer, so a Tab that lands on what was already selected is
                    // correctly silent.
                    announce_target_change,
                    // …and the same for the pointer, on the same terms: the
                    // pointer crosses a unit's edge for one frame in a hundred
                    // and a per-frame event would refill the plate every one of
                    // them.
                    announce_mouseover_change,
                )
                    .chain()
                    .after(crate::world::session::place_entities)
                    // **After the mouse gesture has been judged.** `select_on_
                    // click` reads `MouseLook` to tell a click from a look and
                    // from a drag that began on the interface; unordered, it
                    // would as often as not read last frame's verdict, which on
                    // the release frame is the one that matters.
                    .after(crate::world::camera::arm_look)
                    .after(BindingSet)
                    .in_set(TargetSet)
                    .in_set(super::GameSet),
            )
            .add_systems(Update, forget.in_set(super::GameSet));
    }
}

/// **Let go of the selection when the world does.**
///
/// A guid outlives its world and an `Entity` outlives the despawn that frees it,
/// so a target held across a logout is a stale handle that
/// [`drop_stale_selection`] only clears once there is a *new* world to fail the
/// lookup against — which is one frame into the next session, with the last
/// character's target frame drawn in between. See
/// [`super::events::PlayerLeavingWorld`].
fn forget(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut selection: ResMut<Selection>,
    mut hovered: ResMut<Hovered>,
    mut history: ResMut<TabHistory>,
) {
    if leaving.read().next().is_none() {
        return;
    }
    selection.clear();
    *hovered = Hovered::default();
    history.0.clear();
}

/// `PLAYER_TARGET_CHANGED`, once, on a real change.
fn announce_target_change(
    selection: Res<Selection>,
    mut changed: MessageWriter<PlayerTargetChanged>,
    mut last: Local<Option<Option<u64>>>,
) {
    let now = selection.guid;
    // The first frame adopts without announcing: there was no previous target to
    // have changed *from*, and an event on frame one would fire before anything
    // had registered for it.
    if let Some(previous) = last.replace(now) {
        if previous != now {
            changed.write(PlayerTargetChanged);
        }
    }
}

/// `UPDATE_MOUSEOVER_UNIT`, once, on a real change — [`announce_target_change`]'s
/// twin, and it exists for the same reason.
///
/// The reference raises this from the C side's own mouseover *publisher* rather
/// than per frame (the hovered unit's highlight is set and the publisher
/// beside it names the token), and the shipped handler is a recolour:
/// firing it every frame would repaint every plate in the game at frame rate for
/// nothing.
///
/// **The guid is what is compared, not the entity.** A unit that streams out and
/// straight back in under a still pointer is the same mouseover, and an entity
/// comparison would raise twice and refill the plate for a unit that never
/// changed.
///
/// ## …and the token going *empty* is not a change this raises
///
/// The event says "the `mouseover` token now names somebody else", and the one
/// handler in the shipped directory reads it that way — `GameTooltip.xml`'s
/// whole body is
/// `TextLeft1:SetTextColor(GameTooltip_UnitColor("mouseover"))`, and
/// `GameTooltip_UnitColor` is written against a unit that exists: with none, it
/// falls past `UnitPlayerControlled` and past a nil `UnitReaction` to the
/// catch-all that answers **white**.
///
/// That white was invisible for as long as the plate went down on the same
/// frame the pointer left. It stopped being invisible the moment the plate
/// learned to **hold and fade** (see [`crate::lua::widgets::tooltip`]): the
/// name flicked from its faction colour to white and then dissolved, which is
/// the bug this paragraph exists for and which nothing in Lua could have caused.
///
/// So the reference cannot be raising it on the empty edge either — its own
/// tooltip fades in faction colour, and its handler would have painted the same
/// white. Raising only when the token comes to name somebody is what the event's
/// name says and what its one consumer is written for; the last colour then
/// simply stays on a plate that is on its way out, which is what a plate on its
/// way out should look like.
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

/// **What the pointer is over, judged**: could we attack it, and which of the
/// game's cursors belongs on it.
///
/// One system rather than two answers, because the things that want them — the
/// pointer's bitmap (`ui::cursor`) and the right click
/// ([`attack_on_right_click`]) — must never disagree: a sword drawn over a unit
/// the click then declines is worse than no sword at all.
///
/// **The two answers are not the same question**, and conflating them is what
/// this round's report was. Attackability is the faction table's; the cursor is
/// `UNIT_NPC_FLAGS`' *and then* the faction table's, in that order, because the
/// client asks the flags first — see `vale_assets::look::cursor`. The base reaction
/// to a perfectly friendly NPC is often **Neutral** (a GM's faction template is
/// 35, which shares no group mask with anybody), so a pointer keyed on
/// attackability alone puts a sword over most of a town.
///
/// Asked every frame for the same reason [`spell_cursor_validity`] is: the world
/// moves under a stationary pointer. The unit can die, change faction or be
/// tagged `NON_ATTACKABLE` while the mouse sits still.
fn judge_the_hover(
    assets: Res<crate::assets::GameAssets>,
    units: Query<&WorldEntity>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    // **The same two the interface's own answer is built from** — see
    // [`crate::interface::api::can_attack_between`], which is where the reason a
    // pass holding entities must not fall back to the bare table is.
    group: Res<crate::interface::party::Party>,
    reputation: Res<crate::interface::reputation::PlayerStanding>,
    mut hovered: ResMut<Hovered>,
) {
    let judge = || {
        let tables = assets.display_tables().ok()?;
        let me = player.single().ok()?;
        let unit = units.get(hovered.entity?).ok()?;
        // **Ourselves are not a target for this**, which no faction table says:
        // a character's own template is friendly to itself, but the sword over
        // your own body would be nonsense either way. A game object or a corpse
        // has no services and no fight in it, so both answers are empty.
        // **A dead unit is not attackable and may still be lootable**, which is
        // the whole of what this branch had to grow: a corpse is the one thing
        // in the world that a right click does something to *because* it is
        // dead. A game object still answers nothing.
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
        // **Both directions**, which is what the client asks and what stops an
        // enemy-faction vendor drawing a coin purse — see
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
            // **The panel that opens is gated on the same answer.** A
            // right-click that would open a shop with somebody who will not
            // talk to us is the same disagreement one layer down, and the
            // server refuses it either way.
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

/// **Would the waiting spell take what the pointer is over?** — which is the
/// whole of `Cast.blp` against `UnableCast.blp`.
///
/// Both cursors are in the archives and this is the only thing that chooses
/// between them, so it is also the only feedback the player gets before
/// committing a click. The answer is re-asked every frame rather than latched at
/// the press, because the world moves under a cursor that is up: the priest you
/// are pointing at can die, and the rule that says so is the same
/// [`resolve_aim`][vale_assets::tables::spellbook::resolve_aim] the click will run.
///
/// It is separate from [`hover`] because it is not a pick — it is a *judgement*
/// about the pick, needing the spell catalog and the faction table that `hover`
/// has no business holding.
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
    // A *placed* cast is judged by [`spell_ground_under_pointer`] instead, which
    // is asking a different question of a different ray.
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

/// How far down the pointer's ray the floor is looked for, in yards.
///
/// **Not a spell's range**, deliberately: the ray starts at the *camera*, which
/// sits up to `CameraRig::distance` behind the character and above it, so a
/// bound cut to the spell's own reach would refuse to find the floor at all at
/// full zoom-out — the same "measured from the eye rather than from the
/// character" mistake [`TARGET_RANGE`] is a note about. The range check is a
/// separate question asked of the point once it is found; this is only how far
/// the walk is allowed to go before giving up on the sky.
///
/// **So the honest bound is how far the ground is drawn**, and it used to be a
/// flat 150 yards. That is generous for a Blizzard, whose own range is 30, and
/// it is the whole of the second Eagle Eye report: `SpellRange.dbc` gives it
/// 50,000 yards and the reference lets it land on any terrain that is on the
/// screen, so a pointer aimed at a hillside three hundred yards off found no
/// floor at all and the click was a "never mind". The streamed block's reach is the far
/// corner of the streamed block and the camera's own far plane, so beyond it
/// there is nothing drawn to aim at and nothing loaded to answer with.
///
/// The cost does not grow with it: `ground_under_ray`'s step is half a yard out
/// to 150 and proportional to the distance after that, which is unchanged
/// inside the old bound and about three times the samples for ten times the
/// reach. And the walk only runs while a placed cast is waiting for its click.
/// It is read off [`crate::render::terrain::TerrainReach`] rather than written
/// down, so a host that streams a wider block can aim across the whole of it.
fn ground_pick_range(reach: &crate::render::terrain::TerrainReach) -> f32 {
    reach.yards()
}

/// **Where the pointer is on the floor**, while a *placed* cast is waiting for
/// one — Blizzard's circle, without the circle.
///
/// The pick and the judgement are one system here where the unit half is two,
/// and that is not an inconsistency: [`hover`] runs its ray for every frame of
/// every session because the plate and the highlight want it, so judging it
/// separately costs nothing. This ray costs a walk down the terrain and is
/// therefore run **only while the cursor is up**, which means the thing that
/// would ask "is it valid" is already the thing that has the answer.
///
/// Three ways it comes back with nothing, all of which read as
/// `UnableCast.blp` and none of which is an error:
///
/// * the pointer is on the sky, or past the edge of the loaded world;
/// * the point is further than the spell's own range — the same
///   `SPELL_FAILED_OUT_OF_RANGE` the click would earn, shown before it is
///   spent rather than after;
/// * the pointer is on the interface, where 1.12 has no world pick at all.
///
/// **The floor is the mover's own floor.** `ActiveSession::standing` is the same
/// join the character walks on, so a spell placed on a bridge lands on the
/// bridge and not in the river under it.
#[allow(clippy::too_many_arguments)]
fn spell_ground_under_pointer(
    assets: Res<crate::assets::GameAssets>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::world::camera::WorldCamera>>,
    interface: Res<crate::lua::api::mouse::MouseFocus>,
    session: Res<Session>,
    solids: Res<crate::world::session::Solids>,
    // …and the same planted pointer the unit pick reads, so a scripted run
    // aiming a placed cast sees the place it was aimed at. See
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

    // …and the range, which is the whole of the red pointer against the green
    // one. The same `check_cast` the click will run, measured the same way —
    // but with **only the distance filled in**, deliberately: the pointer is
    // about *where*, and greying it out because the mana is short would be this
    // client inventing feedback the reference does not give. The click still
    // asks the full question.
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
    // **What it is riding**, which is a second rig on the same entity rather
    // than an entity of its own — so nothing else in this query would see the
    // horse, and a click on its flank would pass through to whatever is behind
    // it. See `Mount::pick_target`.
    Option<&'static crate::world::entities::Mount>,
);

/// The unit under the cursor: the sphere of the clip it is playing, then its
/// drawn triangles.
///
/// **The pointer has no range limit at all, and that is deliberate.** It used to
/// carry [`TARGET_RANGE`]'s 41 yards, which was wrong twice over: it was
/// measured from the *eye* rather than from the character, so the real reach was
/// `41 - zoom`; and once that was corrected the limit turned out to have no
/// business being here in the first place. `targetNearestDistance` is Tab's
/// number — its own description says "limited to tab targeting range" — and a
/// pointer is not Tab. What bounds a click is that the thing has to be **in the
/// world and under the cursor**, and the streaming radius already decides the
/// first: an entity you can see is an entity the server has told us about, so
/// hovering it is the only answer that does not need a second rule to explain.
///
/// It leaves one asymmetry that is real and is not a bug: a unit can be hovered
/// and selected from further away than any *action* against it will be allowed
/// from. That is the reference's shape too — the range check lives on the cast
/// and the swing (`assets::spellbook::check_cast`, surface to surface off
/// `UNIT_FIELD_COMBATREACH`), not on the selection.
///
/// **A pointer the interface has taken hovers nothing**, which is a rule this
/// system did not use to have and three things now depend on: the plate, the
/// mouseover highlight and the click. `select_on_click` was already declining
/// separately — the pick stayed live underneath and only the *click* was
/// refused — so a unit standing behind the action bar lit up and put its tooltip
/// on screen while the pointer was on a button. 1.12 has no such state at all,
/// because `WorldFrame` is a frame and the pointer lands on exactly one thing.
///
/// ## Why the two stages are one system and not two
///
/// The narrow phase is only cheap because the broad phase is *selective*: it
/// walks a few thousand triangles per candidate, and it is run against the
/// candidates in **distance order** with the first hit winning, exactly as
/// the client's own loop does. On an ordinary frame the pointer is over nothing and
/// the walk never happens at all; over a crowd it happens once. Splitting the
/// two would mean either doing the sort twice or handing a list between systems
/// for no gain.
///
/// **The joints, not a re-pose.** `run_after` puts this behind
/// [`crate::world::entities::animate`], because the triangles are skinned with
/// the joint entities the GPU skins from — so the outline a click tests is the
/// one on the screen by construction. Posing the skeleton again here would be a
/// second answer to "what pose is this unit in", and the drawn one composes a
/// cross-fade, a counter-twist and a masked second track.
pub(crate) fn hover(
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::world::camera::WorldCamera>>,
    interface: Res<crate::lua::api::mouse::MouseFocus>,
    units: Query<Pickable>,
    joints: Query<&GlobalTransform>,
    // **`--hover` plants a pointer position for a scripted run** — see
    // [`crate::HoverProbe::instead_of`], which is the identity without it. It is
    // read here rather than by moving the real mouse because a window without
    // focus refuses to have its cursor moved, and no scripted run has focus.
    probe: Res<crate::HoverProbe>,
    mut candidates: Local<Vec<(f32, Entity, u64)>>,
    mut hovered: ResMut<Hovered>,
    // **The other half of this one walk** — see
    // [`super::object::HoveredObject`], which says why the answer is
    // split into two resources rather than the pick into two rays.
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
    // **Physical pixels, not logical.** `cursor_position` is logical and the
    // world camera's target is `render::present`'s frame image, which is created
    // at the window's *physical* size with `scale_factor: 1.0` — so its viewport
    // is measured in physical pixels. On a 125% display the two differ by a
    // quarter of the screen and every pick lands up and left of the cursor.
    let viewport = cursor * window.scale_factor();
    let Ok(ray) = camera.viewport_to_world(camera_transform, viewport) else {
        return;
    };

    // **Stage one.** Reused across frames rather than allocated per frame: this
    // runs sixty times a second and is usually empty.
    candidates.clear();
    // **The pointer has no reach of its own** — see this system's own note.
    // `f32::MAX` rather than a range: what comes back is used for exactly one
    // thing, which is the order the triangles are walked in.
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
        // **Units and game objects, and nothing else** — an item is a number in
        // a bag rather than a thing in the world, and a corpse's `Corpse` object
        // is drawn by nothing here. Ourselves are excluded outright: the pointer
        // spends most of a session inside our own back.
        //
        // The game object arm is what the Deadmines doors were missing. It used
        // to read `!is_unit(unit)`, with the comment "a game object is usable
        // and never selectable" — which was true and was also the whole reason
        // nothing was ever usable.
        if !(is_unit(unit) || is_pickable_object(unit)) || unit.is_self {
            continue;
        }
        let Some(model) = model else { continue };
        let sphere = model.pick_sphere(playback.and_then(|p| p.clip()));
        let (centre, radius) = place_sphere(transform, sphere);
        // **Either rig will do.** The two are tested separately rather than
        // merged into one sphere covering both, because a sphere round a rider
        // *and* its kodo is most of the ground the animal is standing on — the
        // whole point of the broad phase is that it is small.
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

    // **Stage two**, nearest first, and the answer is the first *triangle* hit
    // rather than the first candidate whose sphere was nearest.
    let mut best: Option<(f32, Entity, u64)> = None;
    for &(sphere_at, entity, guid) in candidates.iter() {
        // Everything left to test starts further away than a confirmed hit.
        if best.is_some_and(|(hit, ..)| sphere_at > hit) {
            break;
        }
        let Ok((_, _, transform, Some(model), playback, mount)) = units.get(entity) else {
            continue;
        };
        // **Whether the joints hold a pose at all.** A unit spawned this frame
        // has never been through `animate`, so its joint transforms are whatever
        // the spawn left there — and skinning to those would put the silhouette
        // at the world origin and make the unit unclickable rather than merely
        // slightly wrong. `Playback::clip` is `None` for exactly that window.
        let posed = playback.is_some_and(|p| p.clip().is_some());
        let rider = mesh_hit(&ray, transform, &model.pick, &model.joints, &joints, posed, None);
        // …and the animal under it, in its own frame — see `Mount::pick_target`.
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
    // **The one answer, split by what was hit.** Exactly one of the two is
    // filled, and which it is comes out of the depth test above rather than out
    // of a precedence rule — so a mob standing in front of a mailbox takes the
    // click, and a mailbox in front of a mob does.
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
/// A handful of files state no radius and no box worth the name, and a unit with
/// a zero-radius sphere is one nothing can ever click. Half a yard is the same
/// floor the pick box it replaces carried, and for the same reason.
const MIN_PICK_RADIUS: f32 = 0.5;

/// A model-space broad-phase sphere put where the unit actually stands.
///
/// The centre is stated in the **file's** axes and has to be carried through the
/// *whole* transform rather than merely translated: a sphere is symmetric but its
/// centre is not on the model's origin — `HumanMale`'s standing box is centred
/// 0.12 yards behind it — so translating alone leaves a turned unit's sphere off
/// its shoulder. The radius is a scalar and only wants the scale, which
/// `place_entities` has already put on the transform out of
/// `OBJECT_FIELD_SCALE_X`.
fn place_sphere(transform: &Transform, sphere: vale_assets::look::pick::Sphere) -> (Vec3, f32) {
    (
        transform.transform_point(crate::render::axes::to_bevy(sphere.centre)),
        (sphere.radius * transform.scale.max_element()).max(MIN_PICK_RADIUS),
    )
}

/// Ray against a unit's drawn triangles, posed the way it is drawn — the narrow
/// phase.
///
/// Each vertex goes through the **joint** the GPU would have used:
/// `world = joint_affine * to_bevy(model_position)`, which is exactly the
/// skinning identity, because a joint's `GlobalTransform` is already
/// `world_from_model · B · P · B⁻¹` (see `crate::render::axes::pose_to_bevy`).
/// So there is no second pose here and nothing that can drift from the drawn
/// one.
///
/// A model with **no joints** — scenery, and anything whose skeleton did not
/// validate — falls back to the entity's own transform, which is the bind pose
/// and is what it is drawn in. `posed` says the same thing about a unit that has
/// bones but has not been through `animate` yet.
///
/// **The joints are one frame old**, and that is stated rather than fixed: the
/// entity chain runs `.after(GameSet)` and this is inside it, so what is read
/// here is the pose `animate` composed last frame against last frame's
/// placement. At 100 fps that is 0.07 yards on a running mob — less than the
/// spacing between the vertices being tested — and reversing the two would mean
/// breaking a chain whose own ordering note is load-bearing for the weapons and
/// the ground stance.
/// `placed` is the rig's own frame when it is not the entity's — a mount's
/// `world_from_mount` — and `None` for the entity's own model.
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

/// …and the arithmetic of it, with the ECS taken out.
///
/// Separate from [`mesh_hit`] because the join it makes is the one this feature
/// can get *plausibly* wrong: a model-space vertex, the change of basis, and a
/// world matrix composed by somebody else. `crate::render::axes` already pins
/// that `pose_to_bevy(bone) * to_bevy(v)` equals `to_bevy(bone * v)`; what this
/// lets a test assert is that the pick spends that identity in the same
/// direction the GPU does, against a *stated* world position rather than
/// against a second copy of the same premise. `solid.rs` records what that
/// distinction is worth.
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
            // A vertex with no usable weights rides the entity itself, which is
            // the identity joint the skinning shader gives it.
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

/// **Whether a click on nothing drops the target** — `deselectOnClick`,
/// registered `"1"` and `UIOptionsFrame.lua`'s
/// `GAMEFIELD_DESELECT_TEXT` box, under General — so it is a setting a player
/// reaches rather than a console-only one.
const DESELECT_ON_CLICK: &str = "deselectOnClick";

/// A left click selects whatever is hovered; on empty ground it clears.
///
/// **A click, not a drag.** The left button also orbits the camera, so a release
/// that moved the mouse is a look-around and must not retarget — otherwise every
/// camera swing that happens to end over a mob selects it.
///
/// Which of the two it was is [`crate::world::camera::MouseLook`]'s answer now
/// and not this system's. It used to keep its own `Local<bool>` set by
/// `motion.delta != Vec2::ZERO`, so **any** movement at all was a drag — and a
/// mouse moves a pixel or two under the finger during an ordinary click, which
/// is why clicking a mob worked most of the time and not all of it. The shared
/// state also settles the other half of the same question: a gesture that began
/// on the interface is not a click on the world however it ends.
///
/// ## …and while the targeting cursor is up, a click casts instead
///
/// The spell cursor is the reference's answer to a friendly spell pressed with
/// nothing suitable selected (see [`super::action::SpellTargeting`]), and while
/// it is up the click means something else entirely: it names what the waiting
/// spell hits and **does not change the selection**, which is the real client's
/// behaviour and the whole point of the mode — healing someone does not make
/// them your target.
///
/// It is handled here rather than in a system of its own because the two
/// decisions share the *drag* test above, and one click can only be one of
/// them. [`super::action::run_bindings`] does the casting; what happens here is
/// the pick.
#[allow(clippy::too_many_arguments)]
fn select_on_click(
    buttons: Res<ButtonInput<MouseButton>>,
    look: Res<crate::world::camera::MouseLook>,
    hovered: Res<Hovered>,
    session: Res<Session>,
    // **Read only, and only for the one test at the bottom** — see the arm that
    // holds a selection through a click on a chest.
    object: Res<super::object::HoveredObject>,
    mut targeting: ResMut<super::action::SpellTargeting>,
    mut picked: MessageWriter<super::action::SpellTargetPicked>,
    mut placed: MessageWriter<super::action::SpellGroundPicked>,
    mut selection: ResMut<Selection>,
    // **`deselectOnClick`**, and only that — see the empty-ground arm at the
    // bottom, which is the one thing in this system it decides.
    cvars: Res<crate::settings::cvars::CVars>,
    mut clicked: MessageWriter<UnitClicked>,
) {
    targeting.took_click = false;

    // **Escape is not read here any more, and that is the fix rather than a
    // simplification.** Two `just_pressed(KeyCode::Escape)` tests used to sit
    // at the top of this function — one putting the spell cursor away, one
    // clearing the selection — and Escape is `TOGGLEGAMEMENU` in the game's own
    // shipped defaults, so the key did three things at once and could be
    // rebound away from none of them.
    //
    // What runs them now is `ToggleGameMenu`'s own seven-branch chain, in the
    // reference's own order of urgency: `SpellStopCasting()`, then
    // `SpellStopTargeting()`, then `CloseAllWindows()`, then `ClearTarget()`,
    // and only if none of those did anything does the menu open. Each answers
    // whether it acted, which is what stops the chain — see
    // [`crate::lua::api`], where the three are registered as reads with a write
    // attached.
    //
    // **The right button is deliberately not a cancel**, where the reference
    // makes it one: this client steers with it, so a right click over a unit is
    // a hundred-millisecond camera turn as often as it is a refusal, and taking
    // the cast away on it would be a mode that closes itself. Escape and a
    // click on empty ground are the two exits.

    // **Three questions and one answer each**, all of them
    // [`crate::world::camera::MouseLook`]'s: did the finger come up, did the
    // press land on the world, and did the pointer travel far enough to make
    // this a look instead. The middle one replaces the `over_interface` test
    // that used to sit further down — a click is decided by where it *started*,
    // so a press on the world that ends over the action bar is still a click on
    // the world, and a press on a button that ends over a mob is not.
    if !buttons.just_released(MouseButton::Left) || !look.on_world || look.dragged {
        return;
    }
    // The spell cursor's own click: it names a victim or a place, or it stands
    // down — and in every case the selection is left exactly as it was.
    if targeting.is_targeting() {
        targeting.took_click = true;
        // **A placed cast reads the floor point this frame's ray already
        // found** rather than running a second one, which is also what makes the
        // click land exactly where the cursor said it would.
        //
        // Out of range is *sent* rather than swallowed, which is a judgement and
        // matches the unit half: `cast_known_spell` runs the same `check_cast`
        // and answers with the game's own "Out of range", where standing the
        // mode down silently would be a click that did nothing and said nothing.
        if targeting.wants_ground() {
            match targeting.over_ground {
                Some(at) => {
                    placed.write(super::action::SpellGroundPicked { at });
                }
                // The sky, or off the edge of the loaded world: "never mind".
                None => targeting.stop(),
            }
            return;
        }
        match hovered.entity {
            Some(unit) => {
                picked.write(super::action::SpellTargetPicked { unit });
            }
            // Empty ground with the cursor up is "never mind" — the same
            // gesture that clears a selection, which is why it must not also do
            // that on the way past.
            None => targeting.stop(),
        }
        return;
    }
    match (hovered.guid, hovered.entity) {
        // Already selected: a second click on the same unit is not a change, and
        // re-sending the selection would drop a rogue's combo points. **It is
        // still a click**, and the greeting wants every one — see
        // [`UnitClicked`].
        (Some(guid), _) if selection.guid == Some(guid) => {
            clicked.write(UnitClicked { guid });
        }
        (Some(guid), Some(entity)) => {
            selection.set(guid, entity);
            tell_server(&session, Some(guid));
            clicked.write(UnitClicked { guid });
        }
        // Empty ground clears it — the same as Escape. **A game object is not
        // empty ground**, which is the one thing the object pick had to change
        // here: as far as this match is concerned a chest is nothing at all, so
        // without the test a click that opens a door would also drop the target
        // the player was fighting in front of it.
        //
        // **…and `deselectOnClick` is what says whether it clears at all**, the
        // game's own checkbox, registered `"1"` so this is on unless somebody
        // turns it off. Escape
        // is deliberately *not* gated on it: the CVar is about a stray click on
        // the floor, and a key pressed on purpose is not one.
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

/// `TargetNearestEnemy([reverse])` and `TargetUnit("player")` — the game's own
/// targeting verbs, run off binding names rather than keys.
///
/// The archives' `Bindings.xml` is what says these are one verb and not two:
///
/// ```xml
/// <Binding name="TARGETNEARESTENEMY">  TargetNearestEnemy();  </Binding>
/// <Binding name="TARGETPREVIOUSENEMY"> TargetNearestEnemy(1); -- 1 (or "true") means reverse! </Binding>
/// ```
///
/// **This is Classic's priority scheme rather than 1.12's cone, deliberately.**
/// The 1.12 rule is a ±30° cone about the *character's* facing, and it is
/// authentic and unusable behind a third-person camera: at two yards the cone is
/// about a yard wide, the eye sits nine yards back, and the mob filling the
/// screen is routinely forty degrees off the character's axis. Blizzard hit the
/// same wall and rewrote targeting in 7.2; what Classic ships — and what this
/// does — is screen-space first, then a weighted score.
///
/// The weights here are stand-ins, named as such: the real defaults live in
/// CVars (`TargetPriorityFrustumPullIn*`, `TargetPriorityValueBank`) whose values
/// this project has not read.
#[allow(clippy::too_many_arguments)]
fn tab_target(
    mut pressed: MessageReader<BindingPressed>,
    time: Res<Time>,
    session: Res<Session>,
    assets: Res<crate::assets::GameAssets>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::world::camera::WorldCamera>>,
    player: Query<(&WorldEntity, &Transform), With<LocalPlayer>>,
    units: Query<(Entity, &WorldEntity, &Transform)>,
    // **The roster, for `TargetUnit("party1")`** — the guid behind the token,
    // which is the one thing a party frame's click needs and the only part of
    // [`super::api::Units`] this system can hold.
    //
    // Not `Units` itself: it carries `Res<Selection>` for the `"target"` token
    // and this system writes `ResMut<Selection>`, which is Bevy's B0002 — a
    // panic at startup rather than a compile error, since a `SystemParam`
    // conflict is only checked when the schedule is built.
    party: Res<super::party::Party>,
    // …and what the character's own reputation says, which the pool has to
    // agree with the pointer and the interface about — see
    // [`crate::interface::api::can_attack_between`].
    reputation: Res<super::reputation::PlayerStanding>,
    mut selection: ResMut<Selection>,
    mut history: ResMut<TabHistory>,
) {
    // **Read, do not `clear()`.** `action::run_bindings` reads the same stream
    // and takes the action-bar half of it; a targeting system that drained the
    // queue would eat every button press in the same frame. Each reader has its
    // own cursor, which is the whole reason these are messages — see
    // `super::events`.
    let mut wanted: Option<bool> = None;
    for BindingPressed(binding) in pressed.read() {
        match binding {
            Binding::TargetNearestEnemy => wanted = Some(false),
            Binding::TargetPreviousEnemy => wanted = Some(true),
            // `TARGETSELF`'s real body targets the pet if you are already on
            // yourself; this client has no pet — see `bindings::Binding`.
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
            // **A party frame's click.** The unit has to be *in the world* for
            // this to do anything: `CMSG_SET_SELECTION` names a guid the server
            // will check against what we can see, and a member across the zone
            // is not selectable in the reference either — which is why this
            // answers nothing rather than sending a guid with no entity.
            Binding::TargetToken(token) => {
                let super::api::UnitId::Party(index) = token else {
                    continue;
                };
                // The roster says who; the world says whether they are here.
                // A member with no entity selects nothing, which is the
                // reference's behaviour and not a shortcut — see the note on
                // this arm's own comment above.
                if let Some(guid) = party.member(*index).map(|member| member.guid) {
                    if let Some((entity, _, _)) = units.iter().find(|(_, u, _)| u.guid == guid) {
                        selection.set(guid, entity);
                        tell_server(&session, Some(guid));
                    }
                }
            }
            // **`ClearTarget()`** — Escape's last branch before the game menu
            // opens, and the whole of what a `just_pressed(KeyCode::Escape)`
            // test in `select_on_click` used to be. The *reading* of whether
            // there was a target to clear was made in the interface (it is what
            // stopped the chain); what is left here is the doing.
            Binding::ClearTarget => {
                if selection.guid.is_some() {
                    selection.clear();
                    tell_server(&session, None);
                }
            }
            _ => {}
        }
    }
    let Some(reverse) = wanted else { return };
    let (Ok((camera, camera_transform)), Ok((me, standing))) = (camera.single(), player.single())
    else {
        return;
    };
    let Ok(tables) = assets.display_tables() else {
        return;
    };

    let now = time.elapsed_secs();
    history.0.retain(|(_, at)| now - at < TAB_HISTORY_SECS);
    // **From the character**, not from the eye — see [`TARGET_RANGE`]. It used
    // to be `camera_transform.translation()`, which took the rig's whole
    // distance off the reach and made the number move with the zoom wheel.
    let standing = standing.translation;

    // The reverse walks back: the most recent history entry that is still in the
    // world, which is the "I went one too far" key.
    if reverse {
        if let Some((guid, entity)) = history
            .0
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
        if !crate::interface::api::can_attack_between(&tables, &party, &reputation, me, unit) {
            continue;
        }
        let distance = standing.distance(transform.translation);
        if distance > TARGET_RANGE {
            continue;
        }
        // Skip what this cycle has already handed out; exhausting the pool
        // clears the history, which is the wrap.
        if history.0.iter().any(|(guid, _)| *guid == unit.guid) {
            continue;
        }
        let Some(score) = screen_score(camera, camera_transform, transform, distance) else {
            continue;
        };
        // Whatever is already fighting us outranks anything peaceful, but only
        // as a bonus — Tab can always walk off it, which is what a hard combat
        // lock got wrong.
        let score = score - if unit.target == me.target && unit.in_combat { 0.5 } else { 0.0 };
        if best.is_none_or(|(lowest, ..)| score < lowest) {
            best = Some((score, entity, unit.guid));
        }
    }

    match best {
        Some((_, entity, guid)) => {
            if let Some(previous) = selection.guid {
                history.0.push((previous, now));
            }
            history.0.push((guid, now));
            selection.set(guid, entity);
            tell_server(&session, Some(guid));
        }
        // Nothing left to hand out: clear the history so the next press starts
        // the cycle again rather than doing nothing forever.
        None => history.0.clear(),
    }
}

/// Lower is better: how far off the centre of the screen a unit is, plus how far
/// away, and `None` for anything the camera cannot see.
///
/// The frustum edges are pulled in towards the centre, which is the mechanism
/// the client's `TargetPriorityFrustumPullIn{Sides,Top,Bot}` names — a candidate
/// at the very edge of the screen is not what the player means.
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

/// Somebody swung at us and nothing is selected: select them.
///
/// The client's own auto-acquire, off `SMSG_ATTACKERSTATEUPDATE` — which reaches
/// here as [`WorldEntity::swings_thrown`] moving on a unit whose target is us.
/// Without it a fight that starts behind the player has no target frame at all.
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
        // **The latch is a component and not a map keyed by guid**, which is the
        // shape the missile pass already uses and the reason is the same: a
        // `Local<HashMap>` outlives every entity in it, so a session that walks
        // three zones carries a counter for every creature it has ever seen. A
        // component is dropped with its entity.
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

/// Drop a selection whose unit has died or streamed out.
///
/// **Silently.** This used to put "target gone" on screen, which is a sentence
/// this client composed — the game has no string for losing a target because the
/// real client says nothing when it happens, it simply empties the frame. See
/// [`super::messages`] on why a composed line is the one thing that module
/// forbids. The `PLAYER_TARGET_CHANGED` that `announce_target_change` writes is
/// the whole of the news, and it is what the target frame is waiting on anyway.
fn drop_stale_selection(
    session: Res<Session>,
    units: Query<&WorldEntity>,
    mut selection: ResMut<Selection>,
) {
    let Some(entity) = selection.entity else { return };
    let gone = match units.get(entity) {
        // A dead target stays selected in the real client — you loot it — so
        // only *disappearing* clears. Kept as an explicit arm so the difference
        // is on purpose rather than by omission.
        Ok(_) => false,
        Err(_) => true,
    };
    if gone {
        selection.clear();
        tell_server(&session, None);
    }
}

/// **A right click on a unit selects it, and on one we may attack it also
/// starts swinging.**
///
/// The other button, and it is a different verb rather than a second copy of
/// [`select_on_click`]: the left button *picks*, the right one **acts on what it
/// picks**. In the reference a right click on a unit is one gesture with two
/// effects — the selection changes and then the unit's own right-click handler
/// runs, which for anything attackable is the swing and for a friendly NPC is
/// the interact (gossip, vendor, flight master, loot).
///
/// **The interact half is deliberately absent and stated.** It needs a gossip
/// packet family, a loot window and a merchant panel, none of which this client
/// has; a friendly NPC right-clicked is therefore *selected and nothing else*,
/// which is the harmless half of the pair. The report this closes asked for the
/// attack and named the interact as later work.
///
/// **Which half of this is measured**: none of it, and that is worth writing
/// down rather than leaving to be assumed. Every other targeting rule in this
/// module is the 1.12.1 client's or vmangos'; this one is the game's well-known
/// behaviour — right click a mob, your character walks up and hits it — expressed
/// over machinery that already existed.
///
/// Three things it does **not** do, each of which would be a visible fault:
///
/// * **empty ground does not clear the selection.** That is the left button's
///   job and only the left button's — a player who right-clicks the floor to
///   turn the camera would otherwise drop their target every time the gesture
///   happened to be short.
/// * **a second right click does not stop the attack.** [`super::action`]'s
///   `AttackTarget` is a toggle because the *button* is one; the mouse is not,
///   and right-clicking what you are already fighting must never break you off.
/// * **it declines while the spell cursor is up.** The right button steers here,
///   so the reference's "right click cancels the cast" is not available (see
///   [`select_on_click`]); acquiring a target mid-question would be worse than
///   doing nothing.
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
    // The same three questions [`select_on_click`] asks of the left button, plus
    // a fourth: **with the other button also down this is autorun**, not a
    // click. `MouseLook` arms once per *gesture* rather than once per button, so
    // letting go of the right half of a both-button run would otherwise read as
    // a right click that never travelled.
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

    // **The pointer's own answer**, computed once by [`judge_the_hover`] — so
    // the sword the player is looking at and what this does cannot disagree.
    let Ok(me_entity) = player.single() else { return };
    // **A body with something on it is the first half of the interact**, and it
    // is the half that costs nothing to be wrong about: `CMSG_LOOT` on a corpse
    // this player may not touch comes back as `SMSG_LOOT_RESPONSE` with a type
    // of zero and an `ERR_LOOT_*` line, which is exactly what the reference
    // shows. The server is the authority on permission and this client does not
    // try to be — it asks only when the pointer says there is something to ask
    // about, which is `UNIT_DYNAMIC_FLAGS` and is already per-viewer.
    //
    // Before the attack test rather than after it, in the same order the cursor
    // rule puts them: a lootable body cannot be attacked anyway, so the two
    // never compete, and reading them in one order in two places is what keeps
    // them from ever disagreeing.
    if hovered.lootable {
        if let Some(ActiveSession { live, .. }) = session.active.as_ref() {
            live.loot(guid);
        }
        return;
    }
    // **…and the second half: a unit with something to say.** Three openers,
    // routed by the flags' own precedence — see [`Interact`], which is where
    // the reading is stated. What comes back drives `interface::gossip`,
    // `interface::quest` or `interface::merchant`; nothing here waits for it.
    if let Some(interact) = hovered.interact {
        // A conversation opened is a click on somebody, and the greeting is
        // the click's rather than the window's — see [`UnitClicked`]. Raised
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
            }
        }
        return;
    }
    if !hovered.attackable {
        return;
    }
    if let Some(ActiveSession { live, .. }) = session.active.as_ref() {
        // Already swinging at this one: the packet would be a no-op and the
        // draw below is already done. Silence is the right answer, not a stop.
        if live.attacking() == Some(guid) {
            return;
        }
        live.attack(Some(guid));
    }
    // **And starting an attack draws the weapon**, for the reason
    // `super::action::attack_target` states: the server never touches the sheath
    // state, so without this the character swings a sword it is not holding
    // until the reconcile catches up a round trip later.
    sheathing.write(crate::world::entities::SheathRequest {
        entity: me_entity,
        state: vale_assets::look::sheath::MELEE,
    });
}

/// Is this thing selectable at all? Units and players are; game objects,
/// dynamic objects and items are not.
fn is_unit(unit: &WorldEntity) -> bool {
    use vale_protocol::state::update::ObjectType;
    matches!(unit.kind, ObjectType::Unit | ObjectType::Player)
}

/// …and its twin, for the population that is hovered and never selected — see
/// [`super::object`].
fn is_game_object(entity: &WorldEntity) -> bool {
    entity.kind == vale_protocol::state::update::ObjectType::GameObject
}

/// **…and only the ones the pointer has something to say about**, which is what
/// the pick admits — see
/// [`vale_assets::look::object::worth_pointing_at`], where the two halves of
/// that are.
///
/// The filter is not tidiness and it is not about the cursor — it is about the
/// *units behind them*. A zone is carpeted with game objects that have real
/// geometry here and that a click can never act on: the spell focuses that are
/// the anvils and forges, the transports, and 468 of the 1,869 inert
/// `GAMEOBJECT_TYPE_GENERIC` rows. Every one is a `WorldEntity` with a
/// `Transform` and a pick mesh, so without this they enter the broad phase, win
/// the depth test on their own merits — and because a game object's answer goes
/// to `HoveredObject`, **the unit behind them is cleared**. Measured: a pointer
/// swept across Northshire hovered four of them, and each frame it did, nothing
/// was targetable.
///
/// **The other 1,197 generics are the street signs and they belong in**, which
/// is the correction this filter needed after its first draft threw the whole
/// type out. A signpost arm is a game object whose only purpose is to be
/// hovered; `worth_pointing_at` is what tells it from the scenery, and the
/// template is what tells *it*.
///
/// **An unresolved template reads as usable**, which is deliberate: `Kind::of(0)`
/// is `Door`, and zero is also what `object_kind` holds for the round trip
/// `CMSG_GAMEOBJECT_QUERY` costs. Erring that way makes a freshly streamed door
/// clickable a frame early rather than a real one unclickable for ever.
fn is_pickable_object(entity: &WorldEntity) -> bool {
    is_game_object(entity)
        && (vale_assets::look::object::Kind::of(entity.object_kind).usable()
            || entity.object_hover.floating)
}

/// Tell the server what we picked — **and stop swinging at whatever we were
/// swinging at**, if it is no longer the thing selected.
///
/// The second half is not an extra: without it the melee auto-attack is
/// *unstoppable*, and that is arithmetic rather than a judgement.
/// `attack_target` toggles by comparing `live.attacking()` against the current
/// **selection**, so once the two differ the press can only ever start a new
/// attack — there is no reachable state in which it stops the old one. Clear
/// the target entirely and the button has nothing to compare at all. That is
/// exactly the report: *"it cannot be cleared until you re-select a target and
/// manually click off"*.
///
/// **The server will not do it for us.** `HandleSetSelectionOpcode`
/// (`MiscHandler.cpp:399`) updates the selection, drops a rogue's or druid's
/// combo points, and cancels the **auto-shot** when the new target is not
/// attackable — and says nothing at all about the melee swing, which keeps
/// running against the old unit. So a client that does not send
/// `CMSG_ATTACKSTOP` here leaves a swing the player cannot see and cannot end.
///
/// **A switch stops rather than follows**, which is the behaviour asked for and
/// is the safe half of the choice either way: stopping is one packet the player
/// can undo with one press, where following would send the character at
/// whatever they had merely clicked to inspect.
fn tell_server(session: &Session, guid: Option<u64>) {
    let Some(ActiveSession { live, .. }) = session.active.as_ref() else {
        return;
    };
    // **Before the selection goes**, so the two statements reach the socket in
    // the order they happened — the same argument `cancel_cast` makes about
    // ordering a cancel behind its cast.
    if breaks_off(live.attacking(), guid) {
        live.attack(None);
    }
    live.target(guid);
}

/// **Does picking `selecting` end the swing at `attacking`?** — the rule
/// [`tell_server`] applies, as a function so it can be pinned.
///
/// Yes for a switch and for a clear; **no for re-picking the same unit**, which
/// is the case that matters: a right click that selects and then attacks tells
/// the server the same guid twice, and a rule without this guard would stop the
/// attack it had just started.
fn breaks_off(attacking: Option<u64>, selecting: Option<u64>) -> bool {
    attacking.is_some_and(|at| Some(at) != selecting)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A swing ends when the selection moves off it, and not otherwise.**
    ///
    /// The stuck state this fixes is arithmetic rather than taste:
    /// `attack_target` toggles by comparing what we are attacking against what
    /// is *selected*, so once those differ the press can only start a new
    /// attack — there is no reachable state in which it stops the old one, and
    /// the server does not stop it either (`HandleSetSelectionOpcode` leaves
    /// the melee swing alone).
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

    /// **`UPDATE_MOUSEOVER_UNIT` names somebody or it is not raised**, which is
    /// the whole of the white-name bug: the event's one shipped handler recolours
    /// the plate's first line with `GameTooltip_UnitColor("mouseover")`, and that
    /// function answers **white** for a token naming nobody. Firing it on the
    /// pointer *leaving* therefore repaints the plate white — invisible while the
    /// plate went down on the same frame, and plainly visible the moment it
    /// learned to hold and fade.
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
        // …and holding still is not.
        assert_eq!(raised(&mut app), 0);

        // The pointer leaves. **Nothing**, or the plate goes white on its way out.
        app.world_mut().resource_mut::<Hovered>().guid = None;
        assert_eq!(raised(&mut app), 0, "the empty edge repainted the plate");

        // …and coming back is a change again, even to the same unit, because the
        // colour on the plate is a unit ago.
        app.world_mut().resource_mut::<Hovered>().guid = Some(7);
        assert_eq!(raised(&mut app), 1);

        // …and moving straight from one unit to another, with no gap.
        app.world_mut().resource_mut::<Hovered>().guid = Some(8);
        assert_eq!(raised(&mut app), 1);
    }

    /// An app with [`attack_on_right_click`] in it, a unit under the pointer and
    /// a local player to draw the weapon on.
    ///
    /// **`Hovered::attackable` starts false**, which is [`judge_the_hover`]'s
    /// answer and not this system's — so what these tests drive is the gesture,
    /// the selection, and the *use* the click makes of that flag. Whether a
    /// given unit is attackable is [`can_attack`]'s, tested where that lives.
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

    /// How many weapon draws the frame asked for — the observable half of the
    /// attack branch when there is no session to send a packet down.
    fn draws(app: &App) -> usize {
        app.world()
            .resource::<bevy::ecs::message::Messages<crate::world::entities::SheathRequest>>()
            .len()
    }

    /// Let the right button go, and run a frame.
    ///
    /// **Pressed and released**, because `ButtonInput::release` records nothing
    /// for a button that was never down — a helper that only released would make
    /// every test here pass by doing nothing at all.
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

    /// **The right button selects too**, which is the first half of the report:
    /// until this existed only the left one did, so a player who plays with the
    /// right hand had no way to pick a target at all.
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

    /// **Every click on a unit is announced, the second as much as the
    /// first.** The selection does not move on the second — re-sending it
    /// would drop a rogue's combo points — but the greeting is the click's,
    /// and a guard clicked twice says hello twice.
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

    /// **A right *drag* is a camera swing and selects nothing**, and it is the
    /// commonest gesture in the game — this is the button people steer with, so
    /// getting it wrong would retarget on almost every turn of the view.
    #[test]
    fn a_right_drag_is_a_look_and_never_a_pick() {
        let (mut app, _) = clicking_app();
        release_right(&mut app, true);
        assert_eq!(app.world().resource::<Selection>().guid, None);
    }

    /// **Both buttons down is autorun, not a click.** `MouseLook` arms once per
    /// gesture rather than once per button, so letting go of the right half of a
    /// both-button run reads as a right release that never travelled — and
    /// without this test the character would acquire whatever they happened to
    /// be running at.
    #[test]
    fn letting_go_of_one_half_of_autorun_is_not_a_click() {
        let (mut app, _) = clicking_app();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        right_click(&mut app);
        assert_eq!(app.world().resource::<Selection>().guid, None);
    }

    /// **Empty ground does not clear the selection**, where the left button's
    /// click does. A right click on the floor is how the camera is turned; a
    /// player would otherwise drop their target every time one of those turns
    /// happened to be short enough to count as a click.
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

    /// **A friendly NPC is selected and not swung at**, which is the interact
    /// half deliberately absent: the click reads the *same* answer the sword
    /// cursor is drawn from, so a pointer that shows no sword cannot start a
    /// fight and one that does cannot fail to.
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

    /// **The spell cursor outranks it.** The right button steers here, so this
    /// client cannot make it the reference's cancel; what it must not do is
    /// quietly acquire a target while the client is still asking the player a
    /// question.
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

    /// **The centre goes through the whole transform, not just the translation.**
    ///
    /// A standing box is centred a little behind the model's own origin, so a
    /// sphere merely *translated* onto a unit facing east sits beside it —
    /// half a yard off, in the one direction nothing else in the frame would
    /// show. Half a turn puts the offset on the other side, which is what this
    /// measures.
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

    /// **Scale is the entity's, not the model's.** A tauren and a gnome share the
    /// pick code and not the sphere, and `place_entities` has already put
    /// `OBJECT_FIELD_SCALE_X` on the transform.
    #[test]
    fn the_sphere_follows_the_entitys_own_scale() {
        let mut big = transform(Vec3::ZERO);
        big.scale = Vec3::splat(3.0);
        let (_, small) = place_sphere(&transform(Vec3::ZERO), standing());
        let (centre, large) = place_sphere(&big, standing());
        assert!((large - small * 3.0).abs() < 1e-4, "{small} -> {large}");
        // …and the centre is lifted by the same scale, or a giant's sphere sits
        // round its knees.
        assert!((centre.y - 3.0).abs() < 1e-4, "{centre:?}");
    }

    /// A model that declares no extent still has to be clickable, or it cannot
    /// be played against at all.
    #[test]
    fn a_model_with_no_declared_extent_still_has_a_sphere() {
        let (_, radius) = place_sphere(&transform(Vec3::ZERO), Default::default());
        assert_eq!(radius, MIN_PICK_RADIUS);
    }

    /// **The sphere is the clip's, and falling back to the model's is what the
    /// bug was.**
    ///
    /// `Creature\Wisp\Wisp.m2` declares a 12.8-yard cube covering the dust it
    /// sprays; the wisp inside it is about a yard across. Reading the header
    /// sphere hovers it from six yards away, which is the report this round is
    /// about. The numbers are `vale pick`'s own.
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

    /// **The narrow phase tests the triangle where the model is actually
    /// drawn**, in the world's own coordinates — the absolute check.
    ///
    /// This is the shape `solid.rs` argues for and pays for: the relative half
    /// below can pass while the whole thing is off by an axis swap, because both
    /// of its sides are built from the same premise. So this one names a number.
    /// A one-triangle model, one bone, a unit standing at a stated **WoW**
    /// position and facing west; the ray is aimed at where that triangle must be
    /// in Bevy's frame, computed here from the axis rule rather than from
    /// anything the pick does.
    #[test]
    fn a_posed_triangle_is_hit_where_the_model_is_drawn() {
        use crate::render::axes;

        // A yard-wide plate lying flat a yard and a half up, offset onto the
        // model's own +X (north) side. Three distinct model-space coordinates
        // and none of them zero on two axes, so a transposed axis cannot pass
        // by coincidence — and *flat*, so a ray from above crosses it rather
        // than lying in its plane.
        let mut mesh = vale_assets::look::pick::PickMesh::default();
        mesh.positions
            .extend([[1.5, -0.5, 1.5], [1.5, 0.5, 1.5], [0.5, 0.0, 1.5]]);
        mesh.weights.extend([[255, 0, 0, 0]; 3]);
        mesh.bones.extend([[0, 0, 0, 0]; 3]);
        mesh.indices.extend([0u16, 1, 2]);

        // Where the unit stands, in the server's own frame, and which way it
        // looks — a quarter turn counter-clockwise from north, which is west.
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

        // The triangle's own centre, worked out in **WoW** space and converted
        // once: the model's +X is west after the turn, so it lands 1.17 yards
        // west of the unit and a yard and a half up.
        let centre_model = [7.0 / 6.0, 0.0, 1.5];
        let centre_world = [
            tracked[0] + centre_model[0] * facing.cos() - centre_model[1] * facing.sin(),
            tracked[1] + centre_model[0] * facing.sin() + centre_model[1] * facing.cos(),
            tracked[2] + centre_model[2],
        ];
        let aim = axes::to_bevy(centre_world);

        // Straight down at it from twenty yards up, which crosses the flag's
        // plane rather than lying in it.
        let ray = Ray3d::new(aim + Vec3::Y * 20.0, Dir3::NEG_Y);
        let hit = hit_posed_mesh(&ray, &mesh, &[bone], placement.to_matrix());
        let hit = hit.expect("the triangle is under the ray");
        assert!((hit - 20.0).abs() < 0.01, "hit {hit} yards down, wanted 20");

        // …and a ray two yards to the side of it is a miss, so the assertion
        // above is not passing on a hull that covers the whole neighbourhood.
        let beside = Ray3d::new(aim + Vec3::X * 2.0 + Vec3::Y * 20.0, Dir3::NEG_Y);
        assert_eq!(hit_posed_mesh(&beside, &mesh, &[bone], placement.to_matrix()), None);
    }

    /// …and the relative half: **the pick follows the pose**, which is the whole
    /// reason for stage two.
    ///
    /// The same ray, the same model, two different bone matrices: the arm is
    /// down and the ray misses, the arm is up and it hits. A pick built on a
    /// box — or on the bind pose — answers the same thing to both.
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

    /// **The ray parameter is not the range from the character.**
    ///
    /// The bug this pins was one value used for two questions: the number this
    /// returns is how far the *camera* travelled to the hit, and it was compared
    /// against [`TARGET_RANGE`], which is a distance from the character. A
    /// third-person eye sits `CameraRig::distance` behind, so the whole of the
    /// zoom came off the top — at the default 25 the reach was 16 yards and it
    /// shrank further every click of the wheel.
    ///
    /// Kept after [`hover`] stopped bounding the pick at all, because the two
    /// quantities are still measured a screen apart and the same confusion is
    /// still available to `tab_target`, which does bound and has to bound from
    /// the character.
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
