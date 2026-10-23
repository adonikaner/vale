//! The stable window: the pets a stable master holds, and the four verbs that
//! move a pet in or out of it.
//!
//! The client side of [`vale_protocol::play::stable`]. As in every panel in
//! this directory, the rules are in [`vale_assets::tables::pet`] (the family's
//! name and icon, the diet, the loyalty label and the slot's price, none of
//! which needs a session), and this file holds the three parts that need one.
//!
//! ```text
//! what is stabled   MSG_LIST_STABLED_PETS, which arrives whole and is replaced whole
//! what each one is  CMSG_CREATURE_QUERY, because the packet carries an entry
//!                   and the panel wants a family
//! what a press does the four verbs, and the one byte that answers all of them
//! ```
//!
//! ## The list packet does not carry a pet's family
//!
//! A row is a pet number, a creature entry, a level, a name and a loyalty
//! level. The panel shows the family ("Wolf"), the icon
//! (`Ability_Hunter_Pet_Wolf`) and the diet, all three from
//! `CreatureFamily.dbc`, and the only route from an entry to a family id is
//! `SMSG_CREATURE_QUERY_RESPONSE`. The list therefore starts one query per
//! distinct entry, and [`resolve`] raises `PET_STABLE_UPDATE` again when the
//! last answer arrives.
//!
//! That second raise is why the resolution is a system and not a read.
//! `PetStable_Update` runs once per event and caches nothing, so a family that
//! arrives a round trip after the panel drew would leave the stall labelled
//! with a level and no species for as long as the window stays open.
//! [`super::trainer::relayout`] does the same for the training window when the
//! character's race arrives.
//!
//! ## `ClickStablePet` chooses between three packets
//!
//! The interface has one verb for "put this pet in that slot" and the protocol
//! has four, so the client chooses:
//!
//! ```text
//! slot 0, no pet out          nothing — the current slot is already selected
//! slot 0, a pet out           CMSG_STABLE_PET      put it away
//! slot 1..2, no pet out       CMSG_UNSTABLE_PET    take that one out
//! slot 1..2, a pet out        CMSG_STABLE_SWAP_PET exchange them
//! ```
//!
//! The choice follows vmangos' `HandleUnstablePet`, whose first test refuses
//! while a pet is summoned; `HandleStableSwapPet` is the verb for that case.
//! The wrong one of the two gets `STABLE_ERR_STABLE` and the window does not
//! change, which looks the same as being out of range.
//!
//! ## The selection is local
//!
//! `GetSelectedStablePet()` is panel state: which stall is ticked. The server
//! is never told. It starts at -1, the value `PetStable_Update` tests on its
//! first line before choosing one, and it is reset when a new list arrives: a
//! swap moves every pet, so a selection kept across one would point at
//! whichever pet is in that stall now.
//!
//! ## Closing the window
//!
//! There is no stable-close opcode, and the server does not say when the
//! conversation ended, as [`super::gossip`] states for its three windows.
//! `ClosePetStables()` is a local clear and a `PET_STABLE_CLOSED`, and
//! [`super::gossip::close_when_npc_gone`] closes this window when the
//! character walks away.

use bevy::prelude::*;
use std::collections::HashSet;

use vale_protocol::play::stable::{StableList, StableResult, StabledPet};
use vale_protocol::play::gossip::NpcVerb;

use super::events::{
    PetStableClosed, PetStableShow, PetStableUpdate, PetStableUpdatePaperdoll,
};
use crate::world::session::Session;
use crate::world::templates::TemplatesArrived;

/// One of the two stable packets, handed on from [`crate::world::incoming`].
#[derive(Message, Debug, Clone)]
pub(crate) enum StableAnswer {
    /// `MSG_LIST_STABLED_PETS`.
    List(Box<StableList>),
    /// `SMSG_STABLE_RESULT`: the answer to all four verbs. It does not say
    /// what changed, so the window asks for the list again.
    Result(Option<StableResult>),
}

/// A press the interface made.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum StablePress {
    /// `ClickStablePet(id)`: the slot the panel asks for, 0-based.
    Click(u8),
    /// `PickupStablePet(id)`: a drag off a stall, which sends the same three
    /// packets as a click. See [`act`].
    Pickup(u8),
    /// `BuyStableSlot()`.
    BuySlot,
    /// `ClosePetStables()`: local, like the gossip close.
    Close,
}

/// The open stable, or nothing.
#[derive(Resource, Default)]
pub(crate) struct StableWindow {
    open: Option<Open>,
}

/// The open window: the list packet as it arrived, and the two values the
/// packet does not carry.
struct Open {
    /// The stable master. Every verb names it again.
    npc: u64,
    list: StableList,
    /// Which stall is ticked, or -1. See the module note.
    selected: i8,
    /// Entries whose `SMSG_CREATURE_QUERY_RESPONSE` has not arrived yet. The
    /// set [`resolve`] empties.
    unresolved: HashSet<u32>,
}

impl StableWindow {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn guid(&self) -> Option<u64> {
        Some(self.open.as_ref()?.npc)
    }

    /// `GetNumStableSlots()`: the slots bought, which decide how many stalls
    /// the panel enables.
    pub fn slots(&self) -> u8 {
        self.open.as_ref().map_or(0, |open| open.list.slots)
    }

    /// `GetNumStablePets()`.
    pub(crate) fn pet_count(&self) -> usize {
        self.open.as_ref().map_or(0, |open| open.list.count())
    }

    /// `GetSelectedStablePet()`: -1 when nothing is picked, the value the
    /// panel's first branch tests for.
    pub fn selected(&self) -> i8 {
        self.open.as_ref().map_or(-1, |open| open.selected)
    }

    /// One stall's row, or nothing for an empty one.
    pub fn pet(&self, panel_slot: u8) -> Option<&StabledPet> {
        self.open.as_ref()?.list.pet(panel_slot)
    }

    /// Close the window and return whether one was open. Called by
    /// [`super::gossip::close_when_npc_gone`], as the trainer's and the vendor's
    /// are.
    pub(crate) fn close(&mut self) -> bool {
        self.open.take().is_some()
    }
}

pub(crate) struct StablePlugin;

impl Plugin for StablePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<StableAnswer>()
            .add_message::<StablePress>()
            .add_message::<TemplatesArrived>()
            .init_resource::<StableWindow>()
            .add_systems(
                Update,
                (announce, resolve, presses, act)
                    .chain()
                    .after(crate::world::incoming::drain_queues)
                    .in_set(super::GameSet),
            );
        super::gossip::claims_the_token::<StableWindow>(app);
        super::gossip::closes_on_walk_away::<StableWindow>(app);
    }
}

impl super::gossip::NpcWindow for StableWindow {
    type Closed = super::events::PetStableClosed;
    const CLOSED: super::events::PetStableClosed = super::events::PetStableClosed;

    fn guid(&self) -> Option<u64> {
        StableWindow::guid(self)
    }

    fn close(&mut self) -> bool {
        StableWindow::close(self)
    }
}

/// The first line of `PetStable_Update` is
/// `SetPortraitTexture(PetStableFramePortrait, "npc")`, so a window that does
/// not name the token draws the plate with an empty face.
impl super::gossip::NpcToken for StableWindow {
    const ORDER: u8 = 5;

    fn npc(&self) -> Option<u64> {
        StableWindow::guid(self)
    }
}

/// Fold what the server said into the window, and tell the interface.
fn announce(
    mut answers: MessageReader<StableAnswer>,
    mut window: ResMut<StableWindow>,
    session: Res<Session>,
    mut say: super::messages::Announce,
    mut shown: MessageWriter<PetStableShow>,
    mut updated: MessageWriter<PetStableUpdate>,
    mut paperdoll: MessageWriter<PetStableUpdatePaperdoll>,
) {
    for answer in answers.read() {
        match answer {
            StableAnswer::List(list) => {
                // A new list resets the selection: a swap moves every pet, and
                // a slot index kept across one names whichever pet is in that
                // stall now. See the module note.
                let first = !window.is_open();
                let unresolved = list.pets.iter().map(|pet| pet.entry).collect();
                window.open = Some(Open {
                    npc: list.npc,
                    list: (**list).clone(),
                    selected: -1,
                    unresolved,
                });
                ask_for_templates(&session, &mut window);
                if first {
                    shown.write(PetStableShow);
                }
                updated.write(PetStableUpdate);
                paperdoll.write(PetStableUpdatePaperdoll);
            }
            // Any success asks for the list again, because the result byte does
            // not say what changed. A refusal shows its message, if it has one,
            // and leaves the window unchanged.
            StableAnswer::Result(result) => {
                let Some(result) = result else { continue };
                if let Some(key) = result.key() {
                    say.key(key);
                }
                if !result.succeeded() {
                    continue;
                }
                let (Some(npc), Some(active)) = (window.guid(), session.active.as_ref()) else {
                    continue;
                };
                active.live.npc(NpcVerb::StableList(npc));
            }
        }
    }
}

/// Queue a creature template query for every entry the list named that the
/// session does not know yet, so the stalls can be labelled with a species.
/// The known entries leave `unresolved` here, because no arrival will name
/// them; the rest leave it in [`resolve`].
fn ask_for_templates(session: &Session, window: &mut StableWindow) {
    let (Some(open), Some(active)) = (window.open.as_mut(), session.active.as_ref()) else {
        return;
    };
    {
        let world = active.live.read();
        open.unresolved.retain(|entry| !world.creatures.contains_key(entry));
    }
    for entry in &open.unresolved {
        active.live.want_creature(*entry);
    }
}

/// Raise `PET_STABLE_UPDATE` again when the families arrive.
///
/// `PetStable_Update` runs once per event and caches nothing, so a family that
/// arrives after the panel drew would not be shown otherwise (see the module
/// note). Each [`TemplatesArrived`] naming an outstanding entry removes it
/// from the set and raises the event once.
fn resolve(
    mut window: ResMut<StableWindow>,
    mut arrived: MessageReader<TemplatesArrived>,
    mut updated: MessageWriter<PetStableUpdate>,
) {
    for batch in arrived.read() {
        let Some(open) = window.open.as_mut() else {
            continue;
        };
        if open.unresolved.is_empty() {
            continue;
        }
        let before = open.unresolved.len();
        open.unresolved
            .retain(|entry| !batch.0.creatures.contains(entry));
        if open.unresolved.len() != before {
            updated.write(PetStableUpdate);
        }
    }
}

/// Drain what the interface pressed.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<StablePress>,
) {
    let Some(host) = host else { return };
    for press in crate::lua::panels::stable::take(&host) {
        out.write(press);
    }
}

/// Act on each press: three verbs that send a packet, and a close that does
/// not.
fn act(
    mut presses: MessageReader<StablePress>,
    mut window: ResMut<StableWindow>,
    session: Res<Session>,
    units: super::api::Units,
    mut closed: MessageWriter<PetStableClosed>,
    mut updated: MessageWriter<PetStableUpdate>,
    mut paperdoll: MessageWriter<PetStableUpdatePaperdoll>,
) {
    for press in presses.read() {
        match press {
            // A drag off a stall sends the same three packets as a click:
            // `PetStableSlotTemplate`'s `OnReceiveDrag` calls
            // `ClickStablePet(this:GetID())` and nothing else. The 1.12.1
            // client has no cursor state for a pet, so `PickupStablePet` acts
            // as a click.
            StablePress::Click(slot) | StablePress::Pickup(slot) => {
                let Some(open) = window.open.as_mut() else {
                    continue;
                };
                let has_pet = units.exists(super::api::UnitId::Pet);
                let verb = verb_for(open, *slot, has_pet);
                // The tick moves whatever is sent: the 1.12.1 client's
                // `ClickStablePet` returns true and the panel redraws.
                open.selected = i8::try_from(*slot).unwrap_or(-1);
                if let (Some(verb), Some(active)) = (verb, session.active.as_ref()) {
                    active.live.npc(verb);
                }
                updated.write(PetStableUpdate);
                paperdoll.write(PetStableUpdatePaperdoll);
            }
            StablePress::BuySlot => {
                let (Some(npc), Some(active)) = (window.guid(), session.active.as_ref()) else {
                    continue;
                };
                active.live.npc(NpcVerb::BuyStableSlot(npc));
            }
            StablePress::Close => {
                if window.close() {
                    closed.write(PetStableClosed);
                }
            }
        }
    }
}

/// Which of the three packets a click on `slot` sends, or none.
///
/// Separate from [`act`] because it is the one rule in this file, and the
/// part a live session is most likely to disagree with. The table is in the
/// module note.
fn verb_for(open: &Open, slot: u8, has_pet: bool) -> Option<NpcVerb> {
    if slot == 0 {
        // The current stall. With a pet out, the click puts it away; with none
        // out there is nothing to do, because the pet is already in slot 0.
        return has_pet.then_some(NpcVerb::StablePet(open.npc));
    }
    let pet_number = open.list.pet(slot)?.pet_number;
    Some(match has_pet {
        true => NpcVerb::StableSwap {
            stable: open.npc,
            pet_number,
        },
        false => NpcVerb::UnstablePet {
            stable: open.npc,
            pet_number,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(slots: u8, pets: &[(u32, u8)]) -> Open {
        Open {
            npc: 0x99,
            list: StableList {
                npc: 0x99,
                slots,
                pets: pets
                    .iter()
                    .map(|(number, slot)| StabledPet {
                        pet_number: *number,
                        entry: 299,
                        level: 30,
                        name: "Bruiser".into(),
                        loyalty: 4,
                        slot: *slot,
                    })
                    .collect(),
            },
            selected: -1,
            unresolved: HashSet::new(),
        }
    }

    /// Every branch of the choice. `HandleUnstablePet` refuses while a pet is
    /// summoned and `HandleStableSwapPet` is the verb for that case; the wrong
    /// verb leaves the window unchanged with no message.
    #[test]
    fn a_click_picks_its_packet_from_the_slot_and_whether_a_pet_is_out() {
        // wire slot 2 is panel slot 1, wire 3 is panel 2.
        let open = window(2, &[(7, 1), (9, 2), (11, 3)]);
        assert_eq!(verb_for(&open, 0, false), None, "nothing to put away");
        assert_eq!(verb_for(&open, 0, true), Some(NpcVerb::StablePet(0x99)));
        assert_eq!(
            verb_for(&open, 1, false),
            Some(NpcVerb::UnstablePet { stable: 0x99, pet_number: 9 }),
        );
        assert_eq!(
            verb_for(&open, 1, true),
            Some(NpcVerb::StableSwap { stable: 0x99, pet_number: 9 }),
        );
        assert_eq!(
            verb_for(&open, 2, false),
            Some(NpcVerb::UnstablePet { stable: 0x99, pet_number: 11 }),
        );
    }

    /// An empty stall sends nothing, with or without a pet out: there is no pet
    /// number to name.
    #[test]
    fn an_empty_stall_sends_nothing() {
        let open = window(2, &[(7, 1)]);
        assert_eq!(verb_for(&open, 1, false), None);
        assert_eq!(verb_for(&open, 1, true), None);
        assert_eq!(verb_for(&open, 2, true), None);
    }

    /// The closed window answers the values `PetStable_Update` branches on
    /// before anything has arrived: no slots, no pets, and a selection of -1.
    #[test]
    fn a_shut_window_answers_the_values_the_panel_tests_for() {
        let window = StableWindow::default();
        assert!(!window.is_open());
        assert_eq!(window.selected(), -1);
        assert_eq!(window.slots(), 0);
        assert_eq!(window.pet_count(), 0);
        assert_eq!(window.guid(), None);
        assert!(window.pet(0).is_none());
    }
}
