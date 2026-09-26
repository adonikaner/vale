//! **Dying, and getting up again** — the state behind the game's own `DEATH`,
//! `RECOVER_CORPSE` and `RESURRECT` boxes, and the five C functions they call.
//!
//! ```text
//! (health -> 0)                 -> PLAYER_DEAD          -> StaticPopup "DEATH"
//!   the box's Release   RepopMe()      -> CMSG_REPOP_REQUEST
//!   (PLAYER_FLAGS_GHOST appears)-> PLAYER_ALIVE         -> the box comes down
//!   walk to the body            -> CORPSE_IN_RANGE      -> StaticPopup "RECOVER_CORPSE"
//!   its Accept       RetrieveCorpse() -> CMSG_RECLAIM_CORPSE
//!   (the ghost flag clears)     -> PLAYER_UNGHOST
//! SMSG_RESURRECT_REQUEST        -> RESURRECT_REQUEST    -> StaticPopup "RESURRECT"
//!   Accept / Decline  AcceptResurrect()/DeclineResurrect() -> CMSG_RESURRECT_RESPONSE
//! SMSG_SPIRIT_HEALER_CONFIRM    -> CONFIRM_XP_LOSS      -> StaticPopup "XP_LOSS"
//!   its Accept       AcceptXPLoss()   -> CMSG_SPIRIT_HEALER_ACTIVATE
//! ```
//!
//! The spirit healer's line is a conversation the *gossip* window starts:
//! the ghost right-clicks the healer, `SMSG_GOSSIP_MESSAGE` carries the
//! `healer` line (vmangos' `PrepareGossipMenu` keeps it only for a dead
//! player), selecting it has the healer cast spell 17251 on itself with the
//! player as the original caster, and `Spell::EffectDummy`'s 17251 arm sends
//! `SMSG_SPIRIT_HEALER_CONFIRM` with the healer's guid. `UIParent_OnEvent`
//! then asks `GetResSicknessDuration()` and opens one of two boxes on the
//! answer; the box's own `OnUpdate` closes it when `CheckSpiritHealerDist()`
//! says the ghost has walked off. Both reads answer from [`Dying::healer`] —
//! see [`crate::lua::api`], under *being dead*. Accepting is
//! `CMSG_SPIRIT_HEALER_ACTIVATE`, and `SendSpiritResurrect` answers it with
//! an ordinary values block: health up, the ghost flag cleared, 25% off every
//! item's durability, and the sickness aura for a character over level 10.
//!
//! ## Why this is a module and not two lines in [`super::super::combat::action`]
//!
//! For the reason [`super::super::session::logout`] is one, twice over. **Two of the three
//! clocks a death has are the client's own** — see
//! [`vale_protocol::play::death`] — and the third, the reclaim delay, is stated
//! exactly once and then counted here. And **the transitions are edges the
//! interface has names for and the wire does not**: nothing announces a death, a
//! release or a resurrection; each is a field moving inside an ordinary values
//! block, and the whole of `PLAYER_DEAD`, `PLAYER_ALIVE` and `PLAYER_UNGHOST` is
//! this module noticing.
//!
//! ## Three states, two fields
//!
//! `dead` is health zero and `ghost` is `PLAYER_FLAGS_GHOST`, and the pair is
//! three states rather than four: alive, dead (the body has the spirit in it),
//! and ghost (released; health is **1**, so the health alone cannot tell a ghost
//! from a nearly-dead living player). `dead && ghost` does not occur.
//!
//! ## The corpse's range is the client's to notice
//!
//! `CORPSE_IN_RANGE` and `CORPSE_OUT_OF_RANGE` have no packet at all: the ghost
//! walks and the client measures against
//! [`vale_protocol::play::death::CORPSE_RECLAIM_RADIUS`], which is the same 39 yards
//! `HandleReclaimCorpseOpcode` re-checks. So the body has to be *found* first,
//! which is one `MSG_CORPSE_QUERY` sent on the release — and re-sent on a far
//! teleport, since a corpse in another zone is the ordinary case for a ghost
//! walking back.

use bevy::prelude::*;

use vale_protocol::play::death::{CorpseLocation, ResurrectOffer, CORPSE_RECLAIM_RADIUS};

use super::super::bindings::{Binding, BindingPressed, BindingSet};
use super::super::events::{
    ConfirmXpLoss, CorpseInRange, CorpseOutOfRange, PlayerAlive, PlayerDead, PlayerUnghost,
    ResurrectRequest,
};
use crate::world::session::{LocalPlayer, Session, WorldEntity};

/// **What the server said about dying**, forwarded off the one drain of
/// `LiveSession::take_events` — the same shape, and the same reason, as
/// [`super::super::session::logout::LogoutAnswer`].
#[derive(Message, Debug, Clone)]
pub enum DeathAnswer {
    /// `SMSG_CORPSE_RECLAIM_DELAY`, in milliseconds.
    ReclaimDelay(u32),
    /// The reply to our own `MSG_CORPSE_QUERY`.
    Corpse(Option<CorpseLocation>),
    /// `SMSG_RESURRECT_REQUEST`.
    Offer(ResurrectOffer),
    /// `SMSG_SPIRIT_HEALER_CONFIRM`, whose whole body is the healer's guid.
    SpiritHealer(u64),
}

/// Everything a death has, which is more than the two fields it is derived from.
///
/// **Every clock in here is `Time::elapsed_secs`**, taken when the edge was
/// noticed rather than when the server says it happened, because the server does
/// not say: neither the six-minute auto-release nor the reclaim delay is ever
/// restated. A few hundred milliseconds of round trip is the whole error and the
/// boxes they drive are counting in whole seconds.
#[derive(Resource, Default, Debug)]
pub struct Dying {
    /// Health is zero and the spirit is still in the body.
    pub dead: bool,
    /// `PLAYER_FLAGS_GHOST` — released.
    pub ghost: bool,
    /// When the death was noticed, for `GetReleaseTimeRemaining`.
    died_at: Option<f32>,
    /// Whether the release box counts at all — `PLAYER_FIELD_BYTES`'
    /// `RELEASE_TIMER`, which `Player::KillPlayer` clears inside an instance.
    /// `false` makes `GetReleaseTimeRemaining` answer `-1`, which is what puts
    /// `DEATH_RELEASE_NOTIMER` in the box instead of a countdown.
    timed_release: bool,
    /// `SMSG_CORPSE_RECLAIM_DELAY`, and when it arrived.
    reclaim_delay_secs: f32,
    released_at: Option<f32>,
    /// Where the body is, once `MSG_CORPSE_QUERY` has answered.
    pub corpse: Option<CorpseLocation>,
    /// The map the corpse query was last sent on, so the ghost neither asks once
    /// a frame nor keeps an answer about the wrong world.
    ///
    /// A ghost that runs out of a dungeon is on a different map from its body,
    /// and `MSG_CORPSE_QUERY`'s reply is answered against *that* map — so the
    /// question has to be asked again on the other side of the portal.
    asked_on: Option<u32>,
    /// Whether the ghost is within [`CORPSE_RECLAIM_RADIUS`] — the state behind
    /// the two events, held so the *edge* is what raises them.
    pub in_range: bool,
    /// The offer on the table, if there is one.
    pub offer: Option<ResurrectOffer>,
    /// …and whether its event has been raised, so a standing offer does not
    /// re-open its box once a frame.
    offered: bool,
    /// The spirit healer whose offer is on the table.
    pub healer: Option<u64>,
}

impl Dying {
    /// `GetReleaseTimeRemaining()` — seconds, or **-1 for "no timer"**, which is
    /// the answer the `DEATH` box tests for before it decides which sentence to
    /// show.
    ///
    /// Zero once the six minutes are up, and zero when not dead at all: the box
    /// is only ever shown on `PLAYER_DEAD`, and `UIParent_OnEvent` refuses to
    /// show it for a zero.
    pub fn release_remaining(&self, now: f32) -> i32 {
        if !self.timed_release {
            return -1;
        }
        let Some(died_at) = self.died_at else {
            return 0;
        };
        let left = vale_protocol::play::death::AUTO_RELEASE_SECS as f32 - (now - died_at);
        left.max(0.0).round() as i32
    }

    /// `GetCorpseRecoveryDelay()` — seconds until `CMSG_RECLAIM_CORPSE` would be
    /// accepted, zero once it would.
    ///
    /// Both popups that use it call it as a `StartDelay`, so a non-zero here is
    /// a Retrieve button that is drawn disabled and counts down rather than one
    /// that is pressed and silently ignored.
    pub fn recovery_delay(&self, now: f32) -> i32 {
        let Some(released_at) = self.released_at else {
            return 0;
        };
        (self.reclaim_delay_secs - (now - released_at)).max(0.0).round() as i32
    }

    /// Forget everything. A logout, or a death the character has come back from.
    fn reset(&mut self) {
        *self = Dying::default();
    }
}

/// **Stand the `RECOVER_CORPSE` box down**, by lowering the state its two events
/// are the edges of.
///
/// One function because two systems have to be able to do it and the *edge* is
/// the whole content: [`corpse_range`] lowers it when the ghost walks away, and
/// [`watch`] lowers it when the ghost stops being one — and the second is the
/// case that had no route at all.
///
/// **`PLAYER_UNGHOST` does not close that box** — read `UIParent_OnEvent` and
/// count: it hides `RESURRECT`, `RESURRECT_NO_SICKNESS`, `RESURRECT_NO_TIMER`,
/// `SKINNED` and `SKINNED_REPOP`, and `RECOVER_CORPSE` is in none of the five.
/// The only line in the shipped directory that hides it is
/// `CORPSE_OUT_OF_RANGE`'s, and the reason is that the box does not close itself
/// either: `StaticPopupDialogs["RECOVER_CORPSE"].OnAccept` **returns 1**, which
/// is `StaticPopup_OnClick`'s `dontHide`. So the real client leaves it up on the
/// press, the reclaim succeeds, the corpse ceases to exist, and the *range*
/// measurement — which is the client's own, with no packet behind it — falls to
/// nothing and takes the box with it.
fn leave_corpse_range(dying: &mut Dying, left: &mut MessageWriter<CorpseOutOfRange>) {
    if dying.in_range {
        dying.in_range = false;
        left.write(CorpseOutOfRange);
    }
}

pub struct DeathPlugin;

impl Plugin for DeathPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DeathAnswer>()
            .init_resource::<Dying>()
            .add_systems(
                Update,
                // **The answers before the watch, and the watch before the
                // range.** A `SMSG_CORPSE_RECLAIM_DELAY` arrives in the same
                // breath as the ghost flag it belongs to, and the range test
                // needs both this frame's ghost state and this frame's corpse.
                (answers, watch, corpse_range, verbs)
                    .chain()
                    .in_set(super::super::GameSet)
                    .after(BindingSet),
            );
    }
}

/// What the server said, folded into the state — and the one of the four that
/// is also an event the interface waits on: the healer's offer is
/// `CONFIRM_XP_LOSS`, which is what opens the `XP_LOSS` box at all. It was
/// stored and never announced for three rounds, which is the whole of "the
/// spirit healer does not work": the gossip line sent its packet, the confirm
/// came back, and nothing on screen changed.
fn answers(
    mut incoming: MessageReader<DeathAnswer>,
    time: Res<Time>,
    mut dying: ResMut<Dying>,
    mut confirm: MessageWriter<ConfirmXpLoss>,
) {
    for answer in incoming.read() {
        match answer {
            DeathAnswer::ReclaimDelay(ms) => {
                dying.reclaim_delay_secs = *ms as f32 / 1000.0;
                // **Stamped on arrival, not on the release**, because this is
                // the packet that follows the release — the server sends it from
                // inside `BuildPlayerRepop` after `corpse->ResetGhostTime()`, so
                // its own zero is now.
                dying.released_at = Some(time.elapsed_secs());
            }
            DeathAnswer::Corpse(place) => dying.corpse = *place,
            DeathAnswer::Offer(offer) => dying.offer = Some(offer.clone()),
            DeathAnswer::SpiritHealer(healer) => {
                dying.healer = Some(*healer);
                confirm.write(ConfirmXpLoss);
            }
        }
    }
}

/// Notice the two fields moving, and raise the game's own three names off them.
///
/// **Nothing on the wire announces any of this** — see the module comment — so
/// this is the whole of how the interface hears about a death.
///
/// **No socket here.** The one packet a death sends of its own accord — the
/// corpse query — belongs to [`corpse_range`], which is the system that needs
/// the answer; keeping this one to the world state is what lets a test drive it
/// with an entity and no session.
fn watch(
    time: Res<Time>,
    mut dying: ResMut<Dying>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut died: MessageWriter<PlayerDead>,
    mut alive: MessageWriter<PlayerAlive>,
    mut unghost: MessageWriter<PlayerUnghost>,
    mut offered: MessageWriter<ResurrectRequest>,
    mut left_corpse: MessageWriter<CorpseOutOfRange>,
) {
    let Ok(player) = player.single() else {
        // No player entity: a session that has gone, or one that has not
        // arrived. Either way nothing is dead any more.
        if dying.dead || dying.ghost {
            leave_corpse_range(&mut dying, &mut left_corpse);
            dying.reset();
        }
        return;
    };

    let was_dead = dying.dead;
    let was_ghost = dying.ghost;
    // A ghost's health is 1, so "dead" has to exclude one explicitly — see the
    // module comment on the three states.
    let ghost = player.is_ghost;
    let dead = player.dead && !ghost;

    if dead && !was_dead {
        dying.died_at = Some(time.elapsed_secs());
        dying.timed_release = player.timed_release;
        dying.corpse = None;
        dying.asked_on = None;
        dying.in_range = false;
        died.write(PlayerDead);
    }
    if ghost && !was_ghost {
        // **`PLAYER_ALIVE` is what the release raises**, which reads oddly and
        // is the game's own meaning: `UIParent_OnEvent` answers it by hiding the
        // `DEATH` box, and a ghost is alive to every other rule in the game.
        alive.write(PlayerAlive);
        // The body is somewhere else now and nothing will say where unless we
        // ask. `corpse_range` is what asks, off this field.
        dying.asked_on = None;
    }
    if !ghost && !dead && (was_ghost || was_dead) {
        // Back among the living, by whichever of the four routes. `PLAYER_ALIVE`
        // goes with it because the same handler hides the resurrect box on it.
        alive.write(PlayerAlive);
        unghost.write(PlayerUnghost);
        // …and the corpse is gone, which no packet says and no event above
        // covers — see [`leave_corpse_range`], which is what takes the
        // `RECOVER_CORPSE` box off the screen. Before [`Dying::reset`], because
        // the reset clears the very field the edge is measured on.
        leave_corpse_range(&mut dying, &mut left_corpse);
        dying.reset();
    }
    dying.dead = dead;
    dying.ghost = ghost;

    // **The offer's own event, raised where the state is** rather than in
    // `answers`: the popup it opens is refused outright by `StaticPopup_Show`
    // unless the player is dead (`whileDead`), so it belongs after this frame's
    // death state rather than before it.
    if !dying.offered {
        // **Empty for a player caster**, which is `Spell::SendResurrectRequest`'s
        // doing rather than a gap here: it writes `""` when a player cast it and
        // the creature's own name otherwise. The box then formats a sentence
        // with a blank in it, which is the honest picture of what arrived — the
        // real client fills it from its own name cache, which is a lookup this
        // system has no other reason to take the world lock for.
        if let Some(name) = dying.offer.as_ref().map(|offer| offer.name.clone()) {
            dying.offered = true;
            offered.write(ResurrectRequest(name));
        }
    }
}

/// Is the ghost standing on its own body?
///
/// The measurement nothing on the wire makes — see the module comment — and the
/// two events it raises are what put the `RECOVER_CORPSE` box up and take it
/// away again.
fn corpse_range(
    session: Res<Session>,
    mut dying: ResMut<Dying>,
    player: Query<&Transform, With<LocalPlayer>>,
    mut entered: MessageWriter<CorpseInRange>,
    mut left: MessageWriter<CorpseOutOfRange>,
) {
    let Some(active) = session.active.as_ref() else {
        leave_corpse_range(&mut dying, &mut left);
        return;
    };
    if !dying.ghost {
        // Not a ghost any more — resurrected, or never released. Either way
        // there is no corpse to be near, and the box that says there is has to
        // come down: see [`leave_corpse_range`]. `watch` has usually done it a
        // system earlier; this is the same lowering for the paths that do not
        // go through a reset.
        leave_corpse_range(&mut dying, &mut left);
        return;
    }
    // Asked once per release and once per map the ghost stands on — see
    // [`Dying::asked_on`], which is why it is a map id rather than a flag.
    if dying.asked_on != Some(active.map_id) {
        active.live.corpse_query();
        dying.asked_on = Some(active.map_id);
    }
    let inside = match (dying.corpse, player.single().ok()) {
        (Some(corpse), Some(at)) if corpse.corpse_map_id == active.map_id => {
            // The transform is in Bevy's axes and the corpse is in the wire's,
            // so the comparison is made in the wire's — `axes::to_wow` is one
            // negate and a swap, against a `Vec3` conversion of five constants.
            let here = crate::render::axes::to_wow(at.translation);
            let d = [here[0] - corpse.x, here[1] - corpse.y, here[2] - corpse.z];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= CORPSE_RECLAIM_RADIUS
        }
        _ => false,
    };
    if inside != dying.in_range {
        dying.in_range = inside;
        if inside {
            entered.write(CorpseInRange);
        } else {
            left.write(CorpseOutOfRange);
        }
    }
}

/// The five verbs, off the same queue every other one arrives on.
fn verbs(
    mut pressed: MessageReader<BindingPressed>,
    session: Res<Session>,
    mut dying: ResMut<Dying>,
) {
    for BindingPressed(binding) in pressed.read() {
        let Some(active) = session.active.as_ref() else {
            continue;
        };
        match binding {
            // **Nothing is predicted and nothing is gated here.** The server
            // refuses a repop from a living player and a reclaim from a ghost
            // that is too far or too early, both silently — and this client
            // holds the same two facts, so a guard here would be a second
            // opinion that can only be wrong when the two disagree.
            Binding::RepopMe => active.live.repop(),
            Binding::RetrieveCorpse => active.live.reclaim_corpse(),
            Binding::AcceptResurrect | Binding::DeclineResurrect => {
                let accept = matches!(binding, Binding::AcceptResurrect);
                // The guid is **theirs**, echoed from the offer: the server
                // compares it, and a zero is logged as a hack rather than
                // ignored. So an answer with no offer to answer is not sent.
                if let Some(offer) = dying.offer.take() {
                    dying.offered = false;
                    active.live.resurrect_response(offer.caster, accept);
                }
            }
            Binding::AcceptXPLoss => {
                if let Some(healer) = dying.healer.take() {
                    active.live.spirit_healer_activate(healer);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harness() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);
        crate::game::events::register(&mut app);
        app.init_resource::<Session>()
            .add_message::<BindingPressed>()
            .add_plugins(DeathPlugin);
        // The one entity `watch` reads. Alive to begin with, like a login.
        app.world_mut()
            .spawn((LocalPlayer, player(false, false), Transform::default()));
        app.update();
        app
    }

    /// A snapshot with the two fields this module reads and the ordinary values
    /// for the rest.
    fn player(dead: bool, ghost: bool) -> WorldEntity {
        WorldEntity {
            guid: 1,
            kind: vale_protocol::state::update::ObjectType::Player,
            dead,
            is_ghost: ghost,
            // Outdoors: the release box counts. `Player::KillPlayer` clears this
            // inside an instance, which is the `-1` case.
            timed_release: true,
            ..Default::default()
        }
    }

    /// Move the player's state on and run a frame.
    fn becomes(app: &mut App, dead: bool, ghost: bool) {
        let mut q = app.world_mut().query_filtered::<&mut WorldEntity, With<LocalPlayer>>();
        *q.single_mut(app.world_mut()).unwrap() = player(dead, ghost);
        app.update();
    }

    /// How many of a message type have been written **since the last ask**.
    ///
    /// Draining rather than peeking, because these tests walk a state machine
    /// over several frames and the question at each step is "did this edge fire
    /// *now*" — `iter_current_update_messages` answers over a double buffer that
    /// still holds the previous frame's, which reads as an edge firing twice.
    fn raised<M: Message>(app: &mut App) -> usize {
        app.world_mut()
            .get_resource_mut::<bevy::ecs::message::Messages<M>>()
            .map_or(0, |mut queue| queue.drain().count())
    }

    /// **The three edges, in the order a death goes through them** — and the
    /// one that reads backwards: `PLAYER_ALIVE` is what the *release* raises.
    ///
    /// Nothing on the wire announces any of it; two descriptor fields move and
    /// this module notices. Getting the second edge wrong leaves the game's own
    /// `DEATH` box on screen for the whole corpse run, because
    /// `UIParent_OnEvent` hides it on `PLAYER_ALIVE` and on nothing else.
    #[test]
    fn dying_releasing_and_standing_up_are_three_edges_off_two_fields() {
        let mut app = harness();
        assert_eq!(raised::<PlayerDead>(&mut app), 0, "a living character is not news");

        becomes(&mut app, true, false);
        assert_eq!(raised::<PlayerDead>(&mut app), 1);
        assert_eq!(raised::<PlayerAlive>(&mut app), 0);
        assert!(app.world().resource::<Dying>().dead);

        // Held, not repeated: the state is a condition and the event is an edge.
        app.update();
        assert_eq!(raised::<PlayerDead>(&mut app), 0, "PLAYER_DEAD fired twice");

        // Released. Health is 1 and the ghost flag is on, so `dead` goes false
        // — which is why the two fields cannot be collapsed into one.
        becomes(&mut app, false, true);
        assert_eq!(raised::<PlayerAlive>(&mut app), 1, "the release is PLAYER_ALIVE");
        assert_eq!(raised::<PlayerUnghost>(&mut app), 0, "…and not PLAYER_UNGHOST");
        assert!(app.world().resource::<Dying>().ghost);

        // Stood up on the corpse.
        becomes(&mut app, false, false);
        assert_eq!(raised::<PlayerUnghost>(&mut app), 1);
        assert_eq!(raised::<PlayerAlive>(&mut app), 1, "both, because both boxes come down");
        let dying = app.world().resource::<Dying>();
        assert!(!dying.dead && !dying.ghost && dying.corpse.is_none());
    }

    /// **A resurrection accepted from the corpse skips the ghost entirely**, and
    /// still has to end in `PLAYER_UNGHOST` — that is the event the three
    /// resurrect boxes come down on.
    #[test]
    fn a_resurrection_without_a_release_still_stands_the_boxes_down() {
        let mut app = harness();
        becomes(&mut app, true, false);
        becomes(&mut app, false, false);
        assert_eq!(raised::<PlayerUnghost>(&mut app), 1);
        assert_eq!(raised::<PlayerAlive>(&mut app), 1);
    }

    /// The offer raises its name **once** and carries the caster's, which the
    /// box formats into its own sentence.
    #[test]
    fn an_offer_is_announced_once_and_carries_its_casters_name() {
        let mut app = harness();
        becomes(&mut app, true, false);
        app.world_mut().write_message(DeathAnswer::Offer(ResurrectOffer {
            caster: 42,
            name: "Bram".to_string(),
            sickness: true,
            timer: false,
        }));
        app.update();
        assert_eq!(raised::<ResurrectRequest>(&mut app), 1);
        app.update();
        assert_eq!(raised::<ResurrectRequest>(&mut app), 0, "the box reopened itself");
        let dying = app.world().resource::<Dying>();
        assert_eq!(dying.offer.as_ref().map(|o| o.caster), Some(42));
    }

    /// **Standing up takes the corpse box down**, and the only thing in the
    /// shipped directory that can do it is `CORPSE_OUT_OF_RANGE`.
    ///
    /// Two facts stack into one bug here, both read out of `StaticPopup.lua` and
    /// `UIParent.lua`: `StaticPopupDialogs["RECOVER_CORPSE"].OnAccept` **returns
    /// 1**, so `StaticPopup_OnClick` does *not* hide it on the press, and
    /// `UIParent_OnEvent`'s `PLAYER_UNGHOST` arm hides the three resurrect boxes
    /// and the two skinned ones and **not** this one. So a client that stops
    /// measuring the range the moment the ghost stands up leaves "Resurrect
    /// now?" on the screen until the session ends, which is the report.
    #[test]
    fn standing_up_on_the_corpse_says_the_corpse_is_out_of_range() {
        let mut app = harness();
        becomes(&mut app, true, false);
        becomes(&mut app, false, true);
        // The measurement is `corpse_range`'s and needs a session to make; the
        // state is what the box is drawn off, so set it and take the edge.
        app.world_mut().resource_mut::<Dying>().in_range = true;
        let _ = raised::<CorpseOutOfRange>(&mut app);

        becomes(&mut app, false, false);
        assert_eq!(raised::<PlayerUnghost>(&mut app), 1);
        assert_eq!(
            raised::<CorpseOutOfRange>(&mut app),
            1,
            "the box that PLAYER_UNGHOST does not close"
        );
        assert!(!app.world().resource::<Dying>().in_range);

        // Once, not once a frame: it is an edge like the other three.
        app.update();
        assert_eq!(raised::<CorpseOutOfRange>(&mut app), 0);
    }

    /// The three states, and the one that is not simply "health is zero".
    #[test]
    fn a_ghost_is_not_dead_and_a_corpse_is() {
        let corpse = Dying { dead: true, ..Default::default() };
        assert!(corpse.dead && !corpse.ghost, "the body, spirit still in it");
        let ghost = Dying { ghost: true, ..Default::default() };
        assert!(!ghost.dead && ghost.ghost, "released");
    }

    /// **`-1`, not zero, when there is no timer** — the `DEATH` box tests for
    /// exactly `-1` before it swaps its own text for `DEATH_RELEASE_NOTIMER`,
    /// so a zero here shows a countdown that reads "0" for ever.
    #[test]
    fn an_instance_death_answers_minus_one_and_a_field_death_counts_down() {
        let mut dying = Dying {
            died_at: Some(100.0),
            timed_release: false,
            ..Default::default()
        };
        assert_eq!(dying.release_remaining(105.0), -1);

        dying.timed_release = true;
        assert_eq!(dying.release_remaining(100.0), 360, "six minutes");
        assert_eq!(dying.release_remaining(160.0), 300);
        // Clamped rather than negative: the server has already released the
        // spirit by then and the box is gone, but a negative would draw.
        assert_eq!(dying.release_remaining(1000.0), 0);
    }

    /// The reclaim delay counts from the packet's arrival and stops at zero.
    #[test]
    fn the_recovery_delay_runs_out_and_then_stays_out() {
        let mut dying = Dying::default();
        assert_eq!(dying.recovery_delay(0.0), 0, "no death, nothing to wait for");
        dying.reclaim_delay_secs = 30.0;
        dying.released_at = Some(10.0);
        // (assigned rather than built, because the point is that the *absence*
        // of a release is the zero above and the presence of one is the count.)
        assert_eq!(dying.recovery_delay(10.0), 30);
        assert_eq!(dying.recovery_delay(25.0), 15);
        assert_eq!(dying.recovery_delay(40.0), 0);
        assert_eq!(dying.recovery_delay(400.0), 0);
    }
}
