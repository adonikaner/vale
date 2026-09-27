//! **The stable window** — what a stable master is holding, and the four verbs
//! that move a pet in or out of it.
//!
//! The client's half of [`vale_protocol::play::stable`]. The split every
//! panel in this directory keeps: the *rule* is
//! [`vale_assets::tables::pet`] — the family's name and icon, the diet, the
//! loyalty label and the slot's price, none of which needs a session — and what
//! is left here is the three things that do.
//!
//! ```text
//! what is stabled   MSG_LIST_STABLED_PETS, which arrives whole and is replaced whole
//! what each one is  CMSG_CREATURE_QUERY, because the packet carries an entry
//!                   and the panel wants a family
//! what a press does the four verbs, and the one byte that answers all of them
//! ```
//!
//! ## The packet says nothing about what a pet *is*
//!
//! A row is a pet number, a creature entry, a level, a name and a loyalty
//! level. The panel wants the **family** ("Wolf"), the **icon**
//! (`Ability_Hunter_Pet_Wolf`) and the **diet** — all three of which hang off
//! `CreatureFamily.dbc`, and the only route from an entry to a family id is
//! `SMSG_CREATURE_QUERY_RESPONSE`. So the list arriving starts a round trip per
//! distinct entry, and [`resolve`] raises `PET_STABLE_UPDATE` again when the
//! last of them lands.
//!
//! **That second raise is the whole of why this is a system rather than a
//! read.** `PetStable_Update` runs once per event and caches nothing; a family
//! that arrives a round trip after the panel drew is a stall labelled with a
//! level and no species, for as long as the window stays open. It is the same
//! shape as [`super::trainer::relayout`], which re-lays the training window
//! when the character's race finally arrives.
//!
//! ## `ClickStablePet` picks between three different packets
//!
//! The interface has one verb for "put this pet in that slot" and the wire has
//! four, so the choice is the client's:
//!
//! ```text
//! slot 0, no pet out          nothing — the current slot is already selected
//! slot 0, a pet out           CMSG_STABLE_PET      put it away
//! slot 1..2, no pet out       CMSG_UNSTABLE_PET    take that one out
//! slot 1..2, a pet out        CMSG_STABLE_SWAP_PET exchange them
//! ```
//!
//! and the fork is `HandleUnstablePet`'s own first test — it refuses outright
//! while a pet is summoned, and `HandleStableSwapPet` is the verb for that
//! case. Sending the wrong one of the two is a `STABLE_ERR_STABLE` and a window
//! that does not change, which is indistinguishable from being out of range.
//!
//! ## The selection is the client's and the server never hears about it
//!
//! `GetSelectedStablePet()` is a panel state: which stall is ticked. It starts
//! at **-1**, which is the value `PetStable_Update` tests on its own first line
//! before choosing one, and it is reset when a fresh list arrives — a swap
//! moves every pet, so a selection kept across one points at whoever is in that
//! stall now.
//!
//! ## Walking away closes it, on the client's own tape
//!
//! There is no stable-close opcode and the server never says the conversation
//! ended, exactly as [`super::gossip`] states for all three of its windows.
//! `ClosePetStables()` is a local clear and a `PET_STABLE_CLOSED`, and
//! [`super::gossip::out_of_range`] takes this window down with the others.

use bevy::prelude::*;
use std::collections::HashSet;

use vale_protocol::play::stable::{StableList, StableResult, StabledPet};
use vale_protocol::socket::session::NpcVerb;

use super::events::{
    PetStableClosed, PetStableShow, PetStableUpdate, PetStableUpdatePaperdoll,
};
use crate::world::session::Session;

/// One of the two stable packets, handed on from [`crate::world::incoming`].
#[derive(Message, Debug, Clone)]
pub enum StableAnswer {
    /// `MSG_LIST_STABLED_PETS`.
    List(Box<StableList>),
    /// `SMSG_STABLE_RESULT` — the answer to all four verbs, which is why the
    /// window re-asks rather than guessing what changed.
    Result(Option<StableResult>),
}

/// A press the interface made.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum StablePress {
    /// `ClickStablePet(id)` — the slot the panel is asking for, 0-based.
    Click(u8),
    /// `PickupStablePet(id)` — the drag off a stall, which the reference sends
    /// the *same* three packets for. See [`act`].
    Pickup(u8),
    /// `BuyStableSlot()`.
    BuySlot,
    /// `ClosePetStables()` — local, like the gossip close.
    Close,
}

/// The open stable, or nothing.
#[derive(Resource, Default)]
pub struct StableWindow {
    open: Option<Open>,
}

/// What is on screen: the packet as it arrived, plus the two things the packet
/// does not carry.
struct Open {
    /// The stable master. Every verb names it again.
    npc: u64,
    list: StableList,
    /// Which stall is ticked, or -1. See the module note.
    selected: i8,
    /// Entries whose `SMSG_CREATURE_QUERY_RESPONSE` has not landed yet — the
    /// set [`resolve`] watches empty out.
    unresolved: HashSet<u32>,
}

impl StableWindow {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn guid(&self) -> Option<u64> {
        Some(self.open.as_ref()?.npc)
    }

    /// `GetNumStableSlots()` — slots **bought**, which is what decides how many
    /// stalls the panel enables.
    pub fn slots(&self) -> u8 {
        self.open.as_ref().map_or(0, |open| open.list.slots)
    }

    /// `GetNumStablePets()`.
    pub fn pet_count(&self) -> usize {
        self.open.as_ref().map_or(0, |open| open.list.count())
    }

    /// `GetSelectedStablePet()` — **-1 when nothing is picked**, which is the
    /// value the panel's own first branch tests for.
    pub fn selected(&self) -> i8 {
        self.open.as_ref().map_or(-1, |open| open.selected)
    }

    /// One stall's row, or nothing for an empty one.
    pub fn pet(&self, panel_slot: u8) -> Option<&StabledPet> {
        self.open.as_ref()?.list.pet(panel_slot)
    }

    /// Take the window down, answering whether one was up — the door
    /// [`super::gossip::out_of_range`] calls, like the trainer's and the
    /// vendor's.
    pub(crate) fn close(&mut self) -> bool {
        self.open.take().is_some()
    }
}

pub struct StablePlugin;

impl Plugin for StablePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<StableAnswer>()
            .add_message::<StablePress>()
            .init_resource::<StableWindow>()
            .add_systems(
                Update,
                (announce, resolve, presses, act)
                    .chain()
                    .in_set(super::GameSet),
            );
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
                // **A fresh list resets the selection**, which is the module
                // note's reason: a swap moves every pet, and a slot index kept
                // across one names whoever is in that stall now.
                let first = !window.is_open();
                let unresolved = list.pets.iter().map(|pet| pet.entry).collect();
                window.open = Some(Open {
                    npc: list.npc,
                    list: (**list).clone(),
                    selected: -1,
                    unresolved,
                });
                ask_for_templates(&session, &window);
                if first {
                    shown.write(PetStableShow);
                }
                updated.write(PetStableUpdate);
                paperdoll.write(PetStableUpdatePaperdoll);
            }
            // **Any success re-asks**, because the byte does not say what
            // changed — see the module note. A refusal says its sentence, if it
            // has one, and leaves the window alone.
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

/// **Ask for every creature template the list named**, so the stalls can be
/// labelled with a species rather than a level.
fn ask_for_templates(session: &Session, window: &StableWindow) {
    let (Some(open), Some(active)) = (window.open.as_ref(), session.active.as_ref()) else {
        return;
    };
    let Ok(mut world) = active.live.world().lock() else {
        return;
    };
    for entry in &open.unresolved {
        world.want_creature(*entry);
    }
}

/// **Raise `PET_STABLE_UPDATE` again when the families land.**
///
/// See the module note: `PetStable_Update` runs once per event and caches
/// nothing, so a family that arrives after the panel drew would never be shown.
/// Costs one lock and a small set walk per frame *only while a query is
/// outstanding* — the set is emptied by the answers and the system returns on
/// its first line thereafter.
fn resolve(
    mut window: ResMut<StableWindow>,
    session: Res<Session>,
    mut updated: MessageWriter<PetStableUpdate>,
) {
    let Some(open) = window.open.as_mut() else {
        return;
    };
    if open.unresolved.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let Ok(world) = active.live.world().lock() else {
        return;
    };
    let before = open.unresolved.len();
    open.unresolved.retain(|entry| !world.creatures.contains_key(entry));
    if open.unresolved.len() != before {
        updated.write(PetStableUpdate);
    }
}

/// Drain what the interface pressed.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<StablePress>,
) {
    let Some(mut host) = host else { return };
    for press in host.take_stable_presses() {
        out.write(press);
    }
}

/// …and act on it: three verbs that leave, and one close that does not.
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
            // **A drag off a stall is the same three packets as a click**, and
            // `PetStableSlotTemplate`'s own `OnReceiveDrag` says so: it calls
            // `ClickStablePet(this:GetID())` and nothing else. The reference
            // has no cursor state for a pet, so `PickupStablePet` is the click
            // by another name.
            StablePress::Click(slot) | StablePress::Pickup(slot) => {
                let Some(open) = window.open.as_mut() else {
                    continue;
                };
                let has_pet = units.exists(super::api::UnitId::Pet);
                let verb = verb_for(open, *slot, has_pet);
                // The tick moves whatever the wire does — the reference's own
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

/// **Which of the three packets a click on `slot` is**, or none.
///
/// Pulled out of [`act`] because it is the one decision in this file that is a
/// rule rather than plumbing, and it is the one a live session is most likely
/// to disagree with. The table is in the module note.
fn verb_for(open: &Open, slot: u8, has_pet: bool) -> Option<NpcVerb> {
    if slot == 0 {
        // The current stall. With a pet out, the click puts it away; with none
        // out there is nothing to do — the pet is already "in" slot 0.
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

    /// **The whole fork, both ways.** `HandleUnstablePet` refuses while a pet
    /// is summoned and `HandleStableSwapPet` is the verb for that case, so
    /// getting this wrong is a window that silently does not change.
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

    /// An empty stall sends nothing at all, whichever way round the pet is —
    /// there is no pet number to name.
    #[test]
    fn an_empty_stall_sends_nothing() {
        let open = window(2, &[(7, 1)]);
        assert_eq!(verb_for(&open, 1, false), None);
        assert_eq!(verb_for(&open, 1, true), None);
        assert_eq!(verb_for(&open, 2, true), None);
    }

    /// The window's own defaults are the ones `PetStable_Update` branches on
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
