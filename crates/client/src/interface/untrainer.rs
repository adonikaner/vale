//! **Resetting a pet's skills, which is a conversation the client has to
//! finish** — the pet trainer's *"I would like to untrain my pet"*.
//!
//! The same shape as [`super::binder`] one trainer over. Selecting the gossip
//! option sends `CMSG_GOSSIP_SELECT_OPTION` like any other, and the server
//! answers by closing the window and asking: `PlayerTalkClass->CloseGossip()`
//! then `SMSG_PET_UNLEARN_CONFIRM`, whose body is the pet's guid and the cost
//! in copper. Nothing is unlearned. The unlearn happens only when the client
//! answers `CMSG_PET_UNLEARN`, which is what a person pressing Accept on
//! `UIParent.lua`'s `CONFIRM_PET_UNLEARN` popup means.
//!
//! So a client that reads none of it sees the whole visible effect of the
//! click — the gossip window shuts — and never resets the pet.
//!
//! ## The cost, and the guid, and where each goes
//!
//! `UIParent.lua:540` is `StaticPopup_Show("CONFIRM_PET_UNLEARN")` followed by
//! `MoneyFrame_Update(dialog:GetName().."MoneyFrame", arg1)`, so **`arg1` is
//! the cost** and the popup's money frame is the whole of what it draws. The
//! box's `OnAccept` calls `ConfirmPetUnlearn()`, which takes no arguments — the
//! reference keeps the pet's guid in C, exactly as `ConfirmBinder` keeps the
//! innkeeper's.
//!
//! ## Who the trainer is
//!
//! `SMSG_PET_UNLEARN_CONFIRM` names the *pet*, not the trainer, so — unlike the
//! innkeeper, whose guid the confirm carries — there is nothing on the wire to
//! measure `CheckPetUntrainerDist()` against. The trainer is whoever the
//! character was just talking to, so [`remember`] latches the open gossip
//! window's guid while it is up, and that is the guid the range check uses. It
//! is the same fact the reference keeps (the last-interacted NPC), read from
//! the one place this client already holds it.

use bevy::prelude::*;

use vale_protocol::socket::session::PetVerb;

use crate::interface::events::ConfirmPetUnlearn;
use crate::world::session::Session;

/// **How far the trainer may be and still be untrained at** — the server's own
/// interaction distance, the same number and for the same reason as
/// [`super::binder::binder_in_range`]'s: a popup that stayed up past it would
/// send a packet `HandlePetUnlearnConfirmOpcode`'s `GetNPCIfCanInteractWith`
/// gate drops.
const UNTRAINER_RANGE: f32 = 10.0;

/// **The pet trainer's pending question**, and the trainer to measure it
/// against.
#[derive(Resource, Default)]
pub struct Untrainer {
    /// The pet whose skills would reset, and the cost, held between
    /// `SMSG_PET_UNLEARN_CONFIRM` and the person pressing Accept.
    pending: Option<(u64, u32)>,
    /// **Whoever we were last talking to** — latched from the open gossip
    /// window, because the confirm names the pet and not the trainer. `None`
    /// until a gossip window has been open this session.
    trainer: Option<u64>,
}

impl Untrainer {
    /// Whoever is waiting on an answer — `(pet, cost)`.
    pub fn pending(&self) -> Option<(u64, u32)> {
        self.pending
    }

    /// **Take the pending pet** — `ConfirmPetUnlearn()`'s own read, and it takes
    /// rather than borrows so a second press cannot send a second
    /// `CMSG_PET_UNLEARN`.
    pub fn take_pending(&mut self) -> Option<u64> {
        self.pending.take().map(|(pet, _)| pet)
    }
}

/// What the session thread said. Written by [`crate::world::incoming`], read by
/// [`answer`].
#[derive(Message, Debug, Clone, Copy)]
pub struct UntrainerAnswer {
    pub pet: u64,
    pub cost: u32,
}

pub struct UntrainerPlugin;

impl Plugin for UntrainerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Untrainer>()
            .add_message::<UntrainerAnswer>()
            .add_systems(
                Update,
                (remember, answer, press)
                    .chain()
                    // **After the binding dispatch**, so a press made this frame
                    // is sent this frame — the same edge [`super::binder`] and
                    // every other popup button is ordered by.
                    .after(crate::input::bindings::BindingSet)
                    .in_set(crate::interface::GameSet),
            );
    }
}

/// **Latch the trainer's guid off the open gossip window**, for the range
/// check — see the module comment on why the confirm cannot supply it.
fn remember(gossip: Res<crate::interface::gossip::GossipWindow>, mut untrainer: ResMut<Untrainer>) {
    if let Some(guid) = gossip.guid() {
        untrainer.trainer = Some(guid);
    }
}

/// **Put the question in front of the player.**
///
/// One `CONFIRM_PET_UNLEARN` per `SMSG_PET_UNLEARN_CONFIRM`, whose `arg1` is
/// the cost.
fn answer(
    mut answers: MessageReader<UntrainerAnswer>,
    mut untrainer: ResMut<Untrainer>,
    mut confirm: MessageWriter<ConfirmPetUnlearn>,
) {
    for answer in answers.read() {
        untrainer.pending = Some((answer.pet, answer.cost));
        confirm.write(ConfirmPetUnlearn { cost: answer.cost });
    }
}

/// **The popup's Accept, on the wire.**
///
/// `ConfirmPetUnlearn()` is a `Binding` like the death boxes' buttons, so it
/// arrives here as a press — see [`crate::lua::api::verbs`].
fn press(
    mut pressed: MessageReader<crate::input::bindings::BindingPressed>,
    mut untrainer: ResMut<Untrainer>,
    session: Res<Session>,
) {
    for crate::input::bindings::BindingPressed(binding) in pressed.read() {
        if matches!(binding, crate::input::bindings::Binding::ConfirmPetUnlearn) {
            confirm_pet_unlearn(&mut untrainer, &session);
        }
    }
}

/// **`ConfirmPetUnlearn()` — yes, reset the pet.**
///
/// The popup's `OnAccept`, and the only caller of `CMSG_PET_UNLEARN`. Answers
/// whether anything was sent, so a press with nothing pending is a no-op rather
/// than a packet naming guid zero.
pub fn confirm_pet_unlearn(untrainer: &mut Untrainer, session: &Session) -> bool {
    let Some(pet) = untrainer.take_pending() else {
        return false;
    };
    let Some(active) = session.active.as_ref() else {
        return false;
    };
    active.live.pet(PetVerb::Unlearn(pet));
    true
}

/// **`CheckPetUntrainerDist()` — is the trainer still close enough?**
///
/// The popup's `OnUpdate` hides the dialog when this answers false. **True with
/// nothing pending** and true with no trainer latched, for the reason
/// [`super::binder::binder_in_range`] gives: the box is only ever up while
/// there is a question, and answering false for an absent one would race the
/// frame between the event and the popup's first update.
pub fn untrainer_in_range(untrainer: &Untrainer, units: &crate::interface::api::Units) -> bool {
    if untrainer.pending.is_none() {
        return true;
    }
    let Some(guid) = untrainer.trainer else {
        return true;
    };
    let Some((_, _, Some(here))) = units.all.iter().find(|(_, unit, _)| unit.is_self) else {
        return true;
    };
    let Some((_, _, Some(there))) = units.all.iter().find(|(_, unit, _)| unit.guid == guid) else {
        // A trainer who is not in the world is not out of range — the same
        // reading the innkeeper gets, and for the same reason: the gossip
        // window has closed by the time the question arrives.
        return true;
    };
    here.translation.distance(there.translation) <= UNTRAINER_RANGE
}
