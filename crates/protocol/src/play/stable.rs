//! **The stable** — where a hunter leaves a pet, and the five packets that
//! move one in and out of it.
//!
//! ```text
//! CMSG_GOSSIP_SELECT_OPTION  the stable master's own menu line
//!   -> MSG_LIST_STABLED_PETS      the whole window in one packet
//! MSG_LIST_STABLED_PETS      …and the same opcode is how the client re-asks
//! CMSG_STABLE_PET            put the pet that is out into the first free slot
//! CMSG_UNSTABLE_PET          …take one out, by pet number
//! CMSG_STABLE_SWAP_PET       …or exchange it for the one that is out
//! CMSG_BUY_STABLE_SLOT       buy the next slot
//!   -> SMSG_STABLE_RESULT         one byte, for all four of them
//! ```
//!
//! The client's half is `client/src/game/npc/stable.rs` and the panel is
//! `Interface\FrameXML\PetStable.lua`, whose whole content is eight reads over
//! what lands here.
//!
//! ## The slot byte is one-based on the wire and zero-based in the panel
//!
//! `SendStablePet` writes `uint8(0x01)` for the pet that is currently out and
//! `it->slot + 1` for a stabled one, and vmangos' own comment on the first says
//! *"client slot 1 == current pet (0)"*. `PetStable.lua` then indexes
//! `GetStablePetInfo(0)` for the current pet and `1..NUM_PET_STABLE_SLOTS` for
//! the stabled ones. So [`StabledPet::slot`] is the wire's byte and
//! [`StabledPet::panel_slot`] is the number the interface asks by; a reader
//! that conflates them loses the current pet's row and shifts every other one.
//!
//! ## The count is a placeholder the server backfills, and it is not the slots
//!
//! Two bytes follow the guid and they are easy to swap: the first is **how many
//! pet records follow** (`data.put<uint8>(wpos, num)` at the end of
//! `SendStablePet`) and the second is **how many stable slots the character has
//! bought** (`GetPlayer()->m_stableSlots`). Transposed, a hunter with two pets
//! and no slots reads as one with two slots — and the panel's own
//! `i <= GetNumStableSlots()` test then enables two rows nobody paid for.
//!
//! ## The prices are a DBC and never cross the wire
//!
//! `GetNextStableSlotCost()` is `StableSlotPrices.dbc` row `m_stableSlots + 1`,
//! which is the same table `HandleBuyStableSlot` charges from. Two rows in
//! 1.12: 500 copper for the first bought slot and 50,000 for the second. See
//! `vale_assets::tables::pet`, which owns it, and `vale pet`, which
//! checks it.
//!
//! Source: vmangos `Handlers/NPCHandler.cpp` (`SendStablePet`,
//! `SendStableResult`, `HandleStablePet`, `HandleUnstablePet`,
//! `HandleStableSwapPet`, `HandleBuyStableSlot`) and
//! `Server/Packets/Npc.h`, which is where the two-field client packets are
//! declared.

use crate::bytes::{Reader, Writer};

/// **How many stable slots 1.12 has**, and it is both ends of the same number:
/// `MAX_PET_STABLES` on the server and `NUM_PET_STABLE_SLOTS` at the top of
/// `PetStable.lua`. `StableSlotPrices.dbc` has exactly this many rows.
pub const MAX_STABLE_SLOTS: u8 = 2;

/// One row of `MSG_LIST_STABLED_PETS`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StabledPet {
    /// `UNIT_FIELD_PETNUMBER` — what `CMSG_UNSTABLE_PET` and
    /// `CMSG_STABLE_SWAP_PET` name a pet by, and what survives it being
    /// dismissed. **Not a guid**: a stabled pet has no object in the world.
    pub pet_number: u32,
    /// The creature template entry, which is the only route to the pet's
    /// family, its icon and its diet — all three come back from
    /// `CMSG_CREATURE_QUERY` rather than from this packet.
    pub entry: u32,
    pub level: u32,
    /// What the player called it, already in this packet — so unlike the pet
    /// that is out, a stabled one costs no `CMSG_PET_NAME_QUERY`.
    pub name: String,
    /// `PetLoyalty.dbc` row, 1..6.
    pub loyalty: u32,
    /// The wire's byte: 1 for the pet that is out, 2.. for a stabled one. See
    /// the module note, and [`Self::panel_slot`].
    pub slot: u8,
}

impl StabledPet {
    /// The number `GetStablePetInfo` is asked by — the wire's slot minus one,
    /// so the pet that is out is 0.
    ///
    /// Saturating rather than wrapping: a slot byte of 0 is a server this
    /// client has never seen, and answering 0 files it as the current pet,
    /// which is the least wrong of the available wrong answers.
    pub fn panel_slot(&self) -> u8 {
        self.slot.saturating_sub(1)
    }

    /// Whether this is the pet the character has out (or last had out), which
    /// is the row `PetStableCurrentPet` draws.
    pub fn is_current(&self) -> bool {
        self.panel_slot() == 0
    }
}

/// `MSG_LIST_STABLED_PETS`, whole — the stable window in one packet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StableList {
    /// The stable master. Every one of the four verbs names it again, and the
    /// server refuses one that does not (`CheckStableMaster`).
    pub npc: u64,
    /// Slots **bought**, 0..=[`MAX_STABLE_SLOTS`]. Not the number of pets.
    pub slots: u8,
    pub pets: Vec<StabledPet>,
}

impl StableList {
    /// The row for one panel slot, or nothing — which for slot 0 is a hunter
    /// who has never tamed anything, and for 1 or 2 an empty stall.
    pub fn pet(&self, panel_slot: u8) -> Option<&StabledPet> {
        self.pets.iter().find(|pet| pet.panel_slot() == panel_slot)
    }

    /// `GetNumStablePets()` — **every row the packet carried**, the current pet
    /// included, which is what the reference counts.
    pub fn count(&self) -> usize {
        self.pets.len()
    }
}

/// Parse `MSG_LIST_STABLED_PETS`.
///
/// ```text
/// u64 npcGuid
/// u8  petCount
/// u8  stableSlots
/// petCount x:
///   u32     petNumber
///   u32     creatureEntry
///   u32     level
///   cstring name
///   u32     loyalty
///   u8      slot          1 = the pet that is out, 2.. = stabled
/// ```
///
/// **A truncated tail keeps the rows that did parse.** The count is the
/// server's own and the strings are variable-length, so a body cut short mid-row
/// would otherwise discard a window that is nine tenths there — the same
/// tolerance every list packet in this crate takes.
pub fn parse_stabled_pets(body: &[u8]) -> Option<StableList> {
    let mut r = Reader::new(body);
    if !r.has(10) {
        return None;
    }
    let npc = r.u64();
    let count = r.u8();
    let slots = r.u8();
    let mut pets = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        if !r.has(12) {
            break;
        }
        let pet_number = r.u32();
        let entry = r.u32();
        let level = r.u32();
        let name = r.cstring();
        if !r.has(5) {
            break;
        }
        let loyalty = r.u32();
        let slot = r.u8();
        pets.push(StabledPet {
            pet_number,
            entry,
            level,
            name,
            loyalty,
            slot,
        });
    }
    Some(StableList { npc, slots, pets })
}

/// **`SMSG_STABLE_RESULT`'s one byte** — `StableResultCode` at the top of
/// `NPCHandler.cpp`, and the values are sparse rather than sequential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StableResult {
    /// `STABLE_ERR_MONEY` — the only refusal with a reason of its own.
    NotEnoughMoney,
    /// `STABLE_ERR_STABLE` — every other failure, and there are a dozen ways
    /// to reach it: dead, too far, not a hunter pet, no free slot.
    Failed,
    /// `STABLE_SUCCESS_STABLE` — a pet went in.
    Stabled,
    /// `STABLE_SUCCESS_UNSTABLE` — one came out, or two swapped.
    Unstabled,
    /// `STABLE_SUCCESS_BUY_SLOT`.
    SlotBought,
}

impl StableResult {
    pub fn of(byte: u8) -> Option<StableResult> {
        match byte {
            0x01 => Some(StableResult::NotEnoughMoney),
            0x06 => Some(StableResult::Failed),
            0x08 => Some(StableResult::Stabled),
            0x09 => Some(StableResult::Unstabled),
            0x0a => Some(StableResult::SlotBought),
            _ => None,
        }
    }

    /// The byte, for the tests that pin the pairing.
    pub fn byte(self) -> u8 {
        match self {
            StableResult::NotEnoughMoney => 0x01,
            StableResult::Failed => 0x06,
            StableResult::Stabled => 0x08,
            StableResult::Unstabled => 0x09,
            StableResult::SlotBought => 0x0a,
        }
    }

    /// Whether the stable changed, which is what decides a re-ask.
    pub fn succeeded(self) -> bool {
        matches!(
            self,
            StableResult::Stabled | StableResult::Unstabled | StableResult::SlotBought
        )
    }

    /// **The `GlobalStrings.lua` key the client says for a refusal**, or
    /// nothing for a success.
    ///
    /// `ERR_NOT_ENOUGH_MONEY` is the one the money refusal shares with every
    /// other purchase in the game. The generic failure says nothing at all:
    /// there is no `ERR_STABLE_*` key in `GlobalStrings.lua` and the reference
    /// has no sentence for it either — the window simply re-draws unchanged,
    /// which is the client's own behaviour rather than a gap.
    pub fn key(self) -> Option<&'static str> {
        match self {
            StableResult::NotEnoughMoney => Some("ERR_NOT_ENOUGH_MONEY"),
            _ => None,
        }
    }
}

/// Parse `SMSG_STABLE_RESULT` — one byte, and an unknown one is data.
pub fn parse_stable_result(body: &[u8]) -> Option<(u8, Option<StableResult>)> {
    let mut r = Reader::new(body);
    if !r.has(1) {
        return None;
    }
    let byte = r.u8();
    Some((byte, StableResult::of(byte)))
}

/// Body for `MSG_LIST_STABLED_PETS` **outbound** — the same opcode both ways,
/// which is what `MSG_` means.
///
/// The client sends this to re-ask; the stable master's gossip option makes the
/// server send the first one unprompted.
pub fn list_stabled_pets_body(npc: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(npc);
    w.buf
}

/// Body for `CMSG_STABLE_PET` — put the pet that is out into the first free
/// slot. **The client does not choose the slot**; the server takes the lowest
/// free one.
pub fn stable_pet_body(npc: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(npc);
    w.buf
}

/// Body for `CMSG_UNSTABLE_PET` — take one out, by pet number.
///
/// Refused outright while a pet is summoned; the swap below is the verb for
/// that case, and `ClickStablePet` picks between them.
pub fn unstable_pet_body(npc: u64, pet_number: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(npc);
    w.u32(pet_number);
    w.buf
}

/// Body for `CMSG_STABLE_SWAP_PET` — exchange the pet that is out for a stabled
/// one, named by pet number.
pub fn swap_stable_pet_body(npc: u64, pet_number: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(npc);
    w.u32(pet_number);
    w.buf
}

/// Body for `CMSG_BUY_STABLE_SLOT`.
pub fn buy_stable_slot_body(npc: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(npc);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(number: u32, entry: u32, level: u32, name: &str, loyalty: u32, slot: u8) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(number);
        w.u32(entry);
        w.u32(level);
        w.cstring(name);
        w.u32(loyalty);
        w.u8(slot);
        w.buf
    }

    fn packet(npc: u64, slots: u8, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u64(npc);
        w.u8(rows.len() as u8);
        w.u8(slots);
        for r in rows {
            w.bytes(r);
        }
        w.buf
    }

    #[test]
    fn the_list_reads_the_guid_the_count_and_the_slots_in_that_order() {
        let body = packet(
            0xF130_0000_0001_1111,
            1,
            &[
                row(7, 299, 32, "Bruiser", 4, 1),
                row(9, 1126, 28, "Snarl", 3, 2),
            ],
        );
        let list = parse_stabled_pets(&body).expect("parses");
        assert_eq!(list.npc, 0xF130_0000_0001_1111);
        assert_eq!(list.slots, 1, "one slot bought, two pets");
        assert_eq!(list.count(), 2);
        assert_eq!(list.pets[0].name, "Bruiser");
        assert_eq!(list.pets[1].entry, 1126);
    }

    /// **The wire's slot is one-based and the panel's is zero-based**, which is
    /// the transposition that loses the current pet. See the module note.
    #[test]
    fn the_current_pet_is_wire_slot_one_and_panel_slot_zero() {
        let body = packet(1, 2, &[row(7, 299, 32, "Bruiser", 4, 1), row(9, 1126, 28, "Snarl", 3, 3)]);
        let list = parse_stabled_pets(&body).unwrap();
        assert_eq!(list.pets[0].panel_slot(), 0);
        assert!(list.pets[0].is_current());
        assert_eq!(list.pets[1].panel_slot(), 2, "wire 3 is the second stall");
        assert!(!list.pets[1].is_current());
        assert_eq!(list.pet(0).map(|p| p.name.as_str()), Some("Bruiser"));
        assert_eq!(list.pet(2).map(|p| p.name.as_str()), Some("Snarl"));
        assert_eq!(list.pet(1), None, "the first stall is empty");
    }

    /// A hunter with nothing tamed: the header parses and the list is empty,
    /// which is a real answer rather than a failure.
    #[test]
    fn an_empty_stable_is_a_list_of_nothing() {
        let list = parse_stabled_pets(&packet(42, 0, &[])).expect("parses");
        assert_eq!(list.count(), 0);
        assert_eq!(list.slots, 0);
        assert_eq!(list.pet(0), None);
    }

    /// A body cut short mid-row keeps the rows that arrived whole — the
    /// tolerance stated in the module note.
    #[test]
    fn a_truncated_tail_keeps_the_rows_that_parsed() {
        let mut body = packet(1, 2, &[row(7, 299, 32, "Bruiser", 4, 1), row(9, 1126, 28, "Snarl", 3, 2)]);
        body.truncate(body.len() - 6);
        let list = parse_stabled_pets(&body).expect("parses");
        assert_eq!(list.count(), 1, "the whole first row survived");
        assert_eq!(list.pets[0].name, "Bruiser");
    }

    /// A body too short even for the header is nothing, not a zero-pet list.
    #[test]
    fn a_header_that_did_not_arrive_is_none() {
        assert!(parse_stabled_pets(&[]).is_none());
        assert!(parse_stabled_pets(&[0u8; 9]).is_none());
    }

    /// The five result codes are sparse; each maps back to its own byte and
    /// nothing else does.
    #[test]
    fn the_result_codes_round_trip_and_nothing_else_is_one() {
        for result in [
            StableResult::NotEnoughMoney,
            StableResult::Failed,
            StableResult::Stabled,
            StableResult::Unstabled,
            StableResult::SlotBought,
        ] {
            assert_eq!(StableResult::of(result.byte()), Some(result));
        }
        for byte in [0u8, 2, 3, 4, 5, 7, 11, 255] {
            assert_eq!(StableResult::of(byte), None, "{byte}");
        }
        assert_eq!(parse_stable_result(&[0x08]), Some((0x08, Some(StableResult::Stabled))));
        assert_eq!(parse_stable_result(&[0x05]), Some((0x05, None)));
        assert_eq!(parse_stable_result(&[]), None);
    }

    /// Three of the five change the stable and two do not, which is what
    /// decides whether the window re-asks.
    #[test]
    fn only_the_three_successes_re_ask() {
        assert!(StableResult::Stabled.succeeded());
        assert!(StableResult::Unstabled.succeeded());
        assert!(StableResult::SlotBought.succeeded());
        assert!(!StableResult::Failed.succeeded());
        assert!(!StableResult::NotEnoughMoney.succeeded());
        assert_eq!(StableResult::NotEnoughMoney.key(), Some("ERR_NOT_ENOUGH_MONEY"));
        assert_eq!(StableResult::Failed.key(), None);
    }

    /// The four outbound bodies: the guid always, and the pet number only on
    /// the two that name one.
    #[test]
    fn the_bodies_are_the_guid_and_at_most_one_number() {
        let npc = 0x0102_0304_0506_0708u64;
        assert_eq!(list_stabled_pets_body(npc), npc.to_le_bytes());
        assert_eq!(stable_pet_body(npc), npc.to_le_bytes());
        assert_eq!(buy_stable_slot_body(npc), npc.to_le_bytes());
        let mut expected = npc.to_le_bytes().to_vec();
        expected.extend_from_slice(&77u32.to_le_bytes());
        assert_eq!(unstable_pet_body(npc, 77), expected);
        assert_eq!(swap_stable_pet_body(npc, 77), expected);
    }
}
