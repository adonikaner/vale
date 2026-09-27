//! **Whether a unit's weapons are out** — the client's own state, and the two
//! systems that are allowed to change it.
//!
//! The policy is `vale_assets::look::sheath`, where it can be unit-tested with no
//! window and where the data behind it (`AnimationData.dbc`'s `WeaponFlags`
//! column) is read. What is *here* is the three things that need a running
//! world: the per-unit cache, the one setter, and the reconcile's hook into
//! whatever animation was played this frame.
//!
//! ## Why this is not just `WorldEntity::sheath_state`
//!
//! Because for our own character that byte is an echo. `CMSG_SETSHEATHED` is the
//! only thing in vmangos that writes a `Player`'s, so reading the field to
//! decide what to draw is a client waiting for its own message to come back —
//! and, at login, waiting for a message it has never sent. It sits at 0 for
//! ever, which is why this client fought the whole game bare-handed while every
//! NPC in it had a weapon out (a creature's byte *is* the server's:
//! `Creature::Create` sets melee).
//!
//! So [`Sheath`] is the client-side committed cache — the real client's
//! `[CGUnit+0xd40]` — and `WorldEntity::sheath_state` is demoted to what it
//! actually is: a seed, and a change-notification for *other people's*
//! decisions.
//!
//! ## The three writes, and nothing else may write
//!
//! ```text
//! adopt      the descriptor byte CHANGED    a remote unit's own client said so
//! request    a verb asked                   the Z toggle, the attack-start draw
//! reconcile  an animation was played        AnimationData.dbc's own column
//! ```
//!
//! [`commit`] does the first two and [`reconcile`] the third, and they are the
//! whole set — which is the property worth keeping, because a second author
//! would race the first and the symptom is a weapon that flickers in and out of
//! the hand at frame rate. The real client funnels all of it through one
//! setter, and the reason is the same.
//!
//! ## Snapping, and the ceremony that is not here
//!
//! Every transition here is a **snap**: the weapon is simply somewhere else on
//! the next frame. That is right for all but one path — in the client only the
//! manual `ToggleSheath` plays the draw/stow clip. So `TOGGLESHEATH` is this
//! client's one **stated deviation** in this area, and what it would take is
//! named rather than hand-waved: the `Sheath`(89)/`HipSheath`(90) clips per
//! *arm* (this client's overlay masks one subtree, at `SpineLow`, where the
//! ceremony wants the right and left arms independently), the clip's own
//! `$SHL`/`$SHR` event to swap the models at, and a pin holding the old
//! placement until it arrives. Everything reactive — the attack draw, the swim
//! stow, the gossip stow — is *supposed* to snap and does.

use super::*;
use vale_assets::look::sheath;

/// A unit's **committed** sheath state: what it is drawn with, right now.
///
/// Not the wire's byte — see the module comment. Inserted with every
/// `WorldEntity` (`session::poll_world`), so every unit in the world has one
/// from the frame it appears and no reader needs a fallback.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sheath {
    /// 0 stowed, 1 melee drawn, 2 ranged drawn.
    state: u8,
    /// The descriptor byte as last seen.
    ///
    /// **Held so that a *change* can be told from a standing disagreement.** Our
    /// own echo arrives already committed and adopts as a no-op; a remote unit's
    /// client volunteering a change arrives as this field moving. Comparing the
    /// byte against `state` instead would drag the local player back to the
    /// server's stale value on every poll, which is the same bug the field being
    /// an echo causes in the first place.
    seen: u8,
}

impl Sheath {
    /// Seeded from the first descriptor byte the unit was seen with.
    pub fn seeded(byte: u8) -> Sheath {
        Sheath {
            state: byte,
            seen: byte,
        }
    }

    pub fn state(&self) -> u8 {
        self.state
    }
}

/// **Ask** a unit to draw or stow — the client's one setter.
///
/// A message rather than a direct write for the reason the whole of
/// [`crate::interface::events`] is: more than one thing asks (a binding, the attack
/// verb) and exactly one thing executes, so the idempotency refusal, the mounted
/// block and the `CMSG_SETSHEATHED` volunteer are written once. A second request
/// for the same unit in one frame replaces the first, which is the client's
/// last-call-wins.
#[derive(Message, Debug, Clone, Copy)]
pub struct SheathRequest {
    pub entity: Entity,
    /// 0 stow, 1 melee, 2 ranged.
    pub state: u8,
}

/// Adopt what the server said, then run whatever asked.
///
/// **Before the model rebuild and the pose**, because both read [`Sheath`] and a
/// draw that landed after them would show a frame late — visible on the very
/// press that causes it, which is the one frame a player is looking at.
pub(super) fn commit(
    session: Res<crate::world::session::Session>,
    mut requests: MessageReader<SheathRequest>,
    mut units: Query<(Entity, &WorldEntity, &mut Sheath)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Sheath);
    // The descriptor adopt. Only on a *change*: see [`Sheath::seen`].
    for (_, world, mut sheath) in &mut units {
        if world.sheath_state != sheath.seen {
            sheath.seen = world.sheath_state;
            sheath.state = world.sheath_state;
        }
    }
    // …then this frame's requests, last per unit winning.
    let mut wanted: HashMap<Entity, u8> = HashMap::default();
    for request in requests.read() {
        wanted.insert(request.entity, request.state);
    }
    for (entity, world, mut sheath) in &mut units {
        let Some(state) = wanted.get(&entity).copied() else {
            continue;
        };
        // The mounted draw-block is the one refusal the setter itself makes:
        // the reconcile would undo the draw on the next play anyway, but not
        // before it had been drawn for a frame.
        if state == sheath.state || (world.mounted && state != sheath::UNARMED) {
            continue;
        }
        sheath.state = state;
        volunteer(&session, world, state);
    }
}

/// Run the per-animation reconcile against whatever was played this frame.
///
/// **After [`animate`]**, because the input is the animation the pose pass
/// decided on — and *only* on a play, which is the half of the rule that is easy
/// to lose. The client's reconcile lives inside `PlayAnimation`, so a frame in
/// which nothing new started leaves the committed state alone; re-running it
/// every frame against whatever is currently playing makes a held cast stow the
/// weapon and the next frame's engaged draw pull it straight back out, sixty
/// times a second.
pub(super) fn reconcile(
    session: Res<crate::world::session::Session>,
    displays: Res<DisplayCache>,
    mut units: Query<(&WorldEntity, &mut Playback, &mut Sheath)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Sheath);
    for (world, mut play, mut sheath) in &mut units {
        let Some(anim) = play.take_played() else {
            continue;
        };
        // No table in the chain is zero flags everywhere, which degrades to
        // "only the engaged draw and the server's byte move it" — see
        // `DisplayTables::weapon_flags`.
        let flags = displays.tables().map_or(0, |t| t.weapon_flags(anim));
        // **`UNIT_FLAG_IN_COMBAT`, where the client reads its own auto-attack
        // target** (`[unit+0xc48]`). A stated widening, and it costs nothing
        // because this branch only ever *draws* — there is no `!engaged` stow
        // anywhere in the rule.
        //
        // It also buys less than it looks like it does, which is worth knowing
        // before someone "fixes" the attack-start request out of `interface::action`
        // as redundant. `ReadyUnarmed` (25) carries the **empty-hands** flag,
        // and a stowed character in combat is playing exactly that — so the
        // stow at priority 3 answers before the engaged draw at 4 ever runs,
        // and entering combat on its own never draws. That is not a hole: the
        // real client's `engaged` is the auto-attack target, which is set by
        // `Player::Attack`, which is the same press that sends the explicit
        // draw. The explicit request is the mechanism; this is a backstop.
        //
        // **And it is the auto-attack target now, which is what that paragraph
        // already said it should be** — `SMSG_ATTACKSTART` until
        // `SMSG_ATTACKSTOP`, the same gate the ready stance takes (see
        // `pose::wanted_animation`). `UNIT_FLAG_IN_COMBAT` drew a weapon on
        // anything a fight was happening near.
        let engaged = world.attacking;
        let forced = sheath::reconcile(
            sheath.state,
            anim,
            flags,
            engaged,
            world.is_self,
            world.sheath_state,
            world.mounted,
        );
        let Some(forced) = forced.filter(|state| *state != sheath.state) else {
            continue;
        };
        sheath.state = forced;
        volunteer(&session, world, forced);
    }
}

/// Tell the server, for our own character and nobody else.
///
/// A remote unit's state is its own client's business and ours to *mirror*;
/// sending `CMSG_SETSHEATHED` because a wolf played an animation would rewrite
/// our own byte from somebody else's pose.
fn volunteer(session: &crate::world::session::Session, world: &WorldEntity, state: u8) {
    if !world.is_self {
        return;
    }
    if let Some(active) = session.active.as_ref() {
        active.live.set_sheathed(state);
    }
}
