//! **Making an inn your home, and where the hearthstone points.**
//!
//! Two halves that arrive from opposite directions and meet in one resource.
//!
//! ## The bind, which is a conversation the client has to finish
//!
//! Selecting an innkeeper's *"Make this inn your home"* sends
//! `CMSG_GOSSIP_SELECT_OPTION` like any other gossip option, and the server
//! answers by **closing the window and asking**: `PlayerTalkClass->CloseGossip()`
//! then `SMSG_BINDER_CONFIRM`. Nothing is bound. The bind happens only when the
//! client answers `CMSG_BINDER_ACTIVATE`, which is what a person pressing
//! Accept on `UIParent.lua`'s `CONFIRM_BINDER` popup means.
//!
//! So a client that reads none of it sees the *whole* visible effect of the
//! click — the gossip window shuts — and never binds. That is how it was
//! reported: "clicking it does nothing".
//!
//! ## …and where the stone currently points, which is read from the login burst
//!
//! `SMSG_BINDPOINTUPDATE` states the home once in the login burst
//! (`SendInitialPacketsBeforeAddToMap` sends it from `m_homebind`, so a
//! character who has never spoken to an innkeeper still has one) and again on
//! every change. Nothing restates it, so it is kept rather than watched for.
//!
//! It is what `GetBindLocation()` answers and what the hearthstone's own `$z`
//! is substituted with — the two are the same lookup in the reference, both
//! reading the bind area id and both falling back to `HOME_INN`. See
//! [`vale_assets::tables::spelltext`], which takes the resolved name.

use bevy::prelude::*;

use vale_protocol::play::bindpoint::BindPoint;
use vale_protocol::socket::session::NpcVerb;

use crate::game::events::ConfirmBinder;
use crate::world::session::Session;

/// **How far an innkeeper may be and still be bound to** — `CheckBinderDist()`,
/// which the popup's `OnUpdate` hides the dialog on.
///
/// The server's own interaction distance, not a reading of the client's: every
/// gossip verb goes through `GetNPCIfCanInteractWith`, which is
/// `IsWithinDistInMap(pl, INTERACTION_DISTANCE)` at 5.0 yards plus the two
/// units' bounding radii. **The client's own number was not measured**, so this
/// is the distance at which the *request* would be refused rather than the one
/// at which the reference takes the box away; they are the same rule in
/// practice, because a popup that stayed up past it would send a packet the
/// server drops.
const BINDER_RANGE: f32 = 10.0;

/// **What the server said about the home**, and the innkeeper waiting on an
/// answer.
#[derive(Resource, Default)]
pub struct HomeBind {
    /// Where the hearthstone returns the character to. `None` until the login
    /// burst has landed.
    point: Option<BindPoint>,
    /// The innkeeper who asked, held between `SMSG_BINDER_CONFIRM` and the
    /// person pressing Accept.
    ///
    /// A guid rather than a bool, because `CMSG_BINDER_ACTIVATE` names it and
    /// `HandleBinderActivateOpcode` drops the packet without a word if it is
    /// not an innkeeper in range — so the wrong guid here is
    /// indistinguishable, from the screen, from the bug this module fixes.
    pending: Option<u64>,
}

impl HomeBind {
    /// The `AreaTable` id of the home, or `None` before the burst.
    pub fn area(&self) -> Option<u32> {
        self.point.map(|point| point.area_id)
    }

    /// Which map the home is on — the one thing an area id does not settle.
    pub fn map(&self) -> Option<u32> {
        self.point.map(|point| point.map_id)
    }

    pub fn position(&self) -> Option<[f32; 3]> {
        self.point.map(|point| point.position)
    }

    /// Whoever is waiting on an answer.
    pub fn pending(&self) -> Option<u64> {
        self.pending
    }

    /// **Take the pending innkeeper** — `ConfirmBinder()`'s own read, and it
    /// takes rather than borrows so that a second press cannot send a second
    /// `CMSG_BINDER_ACTIVATE`. The popup's Accept is one answer to one
    /// question.
    pub fn take_pending(&mut self) -> Option<u64> {
        self.pending.take()
    }

    /// Give up on the question — the popup was dismissed, or the innkeeper
    /// walked out of range.
    pub fn forget(&mut self) {
        self.pending = None;
    }
}

/// What the session thread said about the home. Written by
/// [`crate::game::incoming`], read by [`answer`].
#[derive(Message, Debug, Clone, Copy)]
pub enum BinderAnswer {
    /// `SMSG_BINDER_CONFIRM` — an innkeeper is asking.
    Confirm { guid: u64 },
    /// `SMSG_PLAYERBOUND` — it went through.
    Bound { guid: u64, area_id: u32 },
}

pub struct BinderPlugin;

impl Plugin for BinderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HomeBind>()
            .add_message::<BinderAnswer>()
            .add_systems(
                Update,
                (follow, answer, press)
                    .chain()
                    // **After the binding dispatch**, so a press made this
                    // frame is sent this frame — the same edge every other
                    // popup button in this client is ordered by. Stated rather
                    // than inherited.
                    .after(crate::game::bindings::BindingSet)
                    .in_set(crate::game::GameSet),
            );
    }
}

/// **Keep the stored home in step with the world's.**
///
/// Polled rather than delivered as an event, because the packet that states it
/// arrives inside the login burst: the session thread's event queue is bounded
/// and drops its oldest, so the one statement of the home in a session could be
/// evicted by the hundreds of packets behind it and never come back.
///
/// The lock is taken only while there is nothing stored or something has just
/// been bound, which is once per session and once per inn.
fn follow(session: Res<Session>, mut home: ResMut<HomeBind>, mut bound: MessageReader<BinderAnswer>) {
    let changed = bound
        .read()
        .any(|answer| matches!(answer, BinderAnswer::Bound { .. }));
    if home.point.is_some() && !changed {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        // Left the world: the home belongs to the character, not to the client.
        home.point = None;
        home.pending = None;
        return;
    };
    if let Some(point) = active.live.bind_point() {
        home.point = Some(point);
    }
}

/// **Put the question in front of the player.**
///
/// One `CONFIRM_BINDER` per `SMSG_BINDER_CONFIRM`, whose `arg1` is the place —
/// `UIParent.lua:547` is `StaticPopup_Show("CONFIRM_BINDER", arg1)` and the
/// sentence is `"Do you want to make %s your new home?"`.
///
/// The place is **where the character is standing**, not where they are bound:
/// the packet carries only the innkeeper's guid, and the sentence is about the
/// home being offered. The sub-zone is used where there is one, so it reads
/// "Lion's Pride Inn" rather than "Elwynn Forest".
fn answer(
    mut answers: MessageReader<BinderAnswer>,
    mut home: ResMut<HomeBind>,
    place: Res<crate::game::place::worldmap::WorldMapState>,
    assets: Res<crate::assets::GameAssets>,
    mut confirm: MessageWriter<ConfirmBinder>,
) {
    for answer in answers.read() {
        match *answer {
            BinderAnswer::Confirm { guid } => {
                home.pending = Some(guid);
                confirm.write(ConfirmBinder {
                    place: standing_in(&place, &assets),
                    guid,
                });
            }
            // The area is folded in by the session thread as well, so this is
            // only the edge that says the question is answered.
            BinderAnswer::Bound { .. } => home.forget(),
        }
    }
}

/// The name of the place the character is standing in — the sub-zone where
/// there is one, the zone otherwise, and empty with no archives.
///
/// The same two-step [`crate::lua::panels::worldmap`]'s `GetSubZoneText` takes,
/// including the building's own name: "Lion's Pride Inn" is a `WMOAreaTable`
/// row and appears in no other table, so there is no area id that reaches it.
fn standing_in(
    place: &crate::game::place::worldmap::WorldMapState,
    assets: &crate::assets::GameAssets,
) -> String {
    if !place.sub_name.is_empty() {
        return place.sub_name.clone();
    }
    let Ok(tables) = assets.display_tables() else {
        return String::new();
    };
    let Some(areas) = tables.areas() else {
        return String::new();
    };
    let sub = areas.sub_zone_name(place.area);
    if sub.is_empty() {
        areas.zone_name(place.area)
    } else {
        sub
    }
}

/// **The popup's Accept, on the wire.**
///
/// `ConfirmBinder()` is a `Binding` like the death boxes' four buttons, so it
/// arrives here as a press rather than through a queue of its own — see
/// [`crate::lua::api::verbs`], where the rule that every write is a recorded
/// verb is written down.
fn press(
    mut pressed: MessageReader<crate::game::bindings::BindingPressed>,
    mut home: ResMut<HomeBind>,
    session: Res<Session>,
) {
    for crate::game::bindings::BindingPressed(binding) in pressed.read() {
        if matches!(binding, crate::game::bindings::Binding::ConfirmBinder) {
            confirm_binder(&mut home, &session);
        }
    }
}

/// **`ConfirmBinder()` — yes, bind me here.**
///
/// The popup's `OnAccept`, and the only caller of `CMSG_BINDER_ACTIVATE`.
/// Answers whether anything was sent, so a press with nothing pending is a
/// no-op rather than a packet naming guid zero.
pub fn confirm_binder(home: &mut HomeBind, session: &Session) -> bool {
    let Some(guid) = home.take_pending() else {
        return false;
    };
    let Some(active) = session.active.as_ref() else {
        return false;
    };
    active.live.npc(NpcVerb::BinderActivate(guid));
    true
}

/// **`CheckBinderDist()` — is the innkeeper still close enough?**
///
/// The popup's `OnUpdate` hides the dialog when this answers false, which is
/// what takes the box away when the player walks off mid-question.
///
/// **True with nothing pending**, which is the answer that keeps a popup up
/// rather than taking one down: the dialog is only ever on screen while there
/// *is* a question, and answering false for an absent one would race the frame
/// between the event and the popup's first update.
pub fn binder_in_range(home: &HomeBind, units: &crate::game::api::Units) -> bool {
    let Some(guid) = home.pending() else {
        return true;
    };
    let Some((_, _, Some(here))) = units
        .all
        .iter()
        .find(|(_, unit, _)| unit.is_self)
    else {
        return true;
    };
    let Some((_, _, Some(there))) = units.all.iter().find(|(_, unit, _)| unit.guid == guid) else {
        // **An innkeeper who is not in the world is not out of range.** The
        // guid is scanned rather than reached through a token because the
        // gossip window has already closed by the time the question arrives, so
        // `"npc"` names nobody — and an entity streamed out from under the
        // popup would otherwise read as "walked away" on the frame it left.
        return true;
    };
    here.translation.distance(there.translation) <= BINDER_RANGE
}
