//! **The bank** — the window a banker opens, the slot it sells, and the two
//! packets that move an item across the counter.
//!
//! ```text
//! CMSG_BANKER_ACTIVATE        u64 guid          right-click on a banker with no gossip bit
//! CMSG_GOSSIP_SELECT_OPTION   …or the banker's own menu line (GOSSIP_OPTION_BANKER)
//!   -> SMSG_SHOW_BANK         u64 guid          the window opens
//! CMSG_BUY_BANK_SLOT          u64 guid          the next bag slot, at BankBagSlotPrices.dbc's price
//!   -> SMSG_BUY_BANK_SLOT_RESULT  u32 result    only ever a refusal, or 3
//! CMSG_AUTOBANK_ITEM          u8 bag, u8 slot   put this in the bank, wherever it fits
//! CMSG_AUTOSTORE_BANK_ITEM    u8 bag, u8 slot   …and the same packet the other way:
//!                                               HandleAutoStoreBankItemOpcode forks on
//!                                               whether the source is a bank position
//! ```
//!
//! Every layout is `Server/Packets/Npc.cpp` and `Item.cpp` in vmangos, read
//! rather than inferred. The client's half is `client/src/game/npc/bank.rs`
//! and the panel is `Interface\FrameXML\BankFrame.lua`.
//!
//! ## The bank is not a packet, it is twenty-four more fields
//!
//! Nothing in this family carries an item. `SMSG_SHOW_BANK` is a guid and
//! nothing else, because what is *in* the bank arrived at login in the same
//! `SMSG_UPDATE_OBJECT` as the backpack: `PLAYER_FIELD_BANK_SLOT_1` is 24
//! guids and `PLAYER_FIELD_BANKBAG_SLOT_1` six more, each a container like a
//! worn bag. [`crate::play::items::Inventory`] reads them beside the rest,
//! and a deposit is an ordinary field change the update block reports. So
//! the window can be *drawn* with no banker in sight — which is what the
//! reference does too — and only the verbs are gated.
//!
//! ## The result is only ever a refusal
//!
//! `HandleBuyBankSlotOpcode` sends `SMSG_BUY_BANK_SLOT_RESULT` for three
//! failures and sends *nothing* on success: it writes the new count into
//! `PLAYER_BYTES_2`'s third byte and takes the money, and both arrive as
//! update fields. `ERR_BANKSLOT_OK` (3) exists in the enum and is never sent
//! by this server. A client that waited for it would wait for ever; the
//! panel is redrawn off the byte instead — `PLAYERBANKBAGSLOTS_CHANGED`.
//!
//! ## The slot count is a byte of `PLAYER_BYTES_2`
//!
//! `GetBankBagSlotCount` is `GetByteValue(PLAYER_BYTES_2, 2)`: the third byte
//! of field 194 of the player's block. It is the number of bag slots
//! **bought**, 0..6, and `GetNumBankSlots()` is it and nothing else.

use crate::bytes::{Reader, Writer};

/// `BuyBankSlotResult` in `Player.h`, byte for byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankSlotResult {
    /// `ERR_BANKSLOT_FAILED_TOO_MANY` (0): no `BankBagSlotPrices.dbc` row
    /// for the next slot.
    TooMany,
    /// `ERR_BANKSLOT_INSUFFICIENT_FUNDS` (1).
    InsufficientFunds,
    /// `ERR_BANKSLOT_NOTBANKER` (2): the guid is not a banker in reach.
    NotBanker,
    /// `ERR_BANKSLOT_OK` (3) — declared, and never sent by vmangos. See the
    /// module note.
    Ok,
}

impl BankSlotResult {
    pub fn of(code: u32) -> Option<BankSlotResult> {
        Some(match code {
            0 => BankSlotResult::TooMany,
            1 => BankSlotResult::InsufficientFunds,
            2 => BankSlotResult::NotBanker,
            3 => BankSlotResult::Ok,
            _ => return None,
        })
    }

    /// The `GlobalStrings.lua` key the reference shows for a refusal — each
    /// is a record in `vale_assets::interface::messages` (ids 256..258),
    /// so the error frame and its chime come with it. `None` for the success
    /// nobody sends.
    pub fn key(self) -> Option<&'static str> {
        Some(match self {
            BankSlotResult::TooMany => "ERR_BANKSLOT_FAILED_TOO_MANY",
            BankSlotResult::InsufficientFunds => "ERR_BANKSLOT_INSUFFICIENT_FUNDS",
            BankSlotResult::NotBanker => "ERR_BANKSLOT_NOTBANKER",
            BankSlotResult::Ok => return None,
        })
    }
}

/// `SMSG_SHOW_BANK`: **`u64 guid`** — the banker, and nothing else. See the
/// module note on why the contents are not here.
pub fn parse_show_bank(body: &[u8]) -> Option<u64> {
    let mut r = Reader::new(body);
    r.has(8).then(|| r.u64())
}

/// `SMSG_BUY_BANK_SLOT_RESULT`: **`u32 result`**, kept raw beside its reading
/// so a code this client does not know is data rather than a dropped packet.
pub fn parse_buy_bank_slot_result(body: &[u8]) -> Option<(u32, Option<BankSlotResult>)> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let code = r.u32();
    Some((code, BankSlotResult::of(code)))
}

/// Body for `CMSG_BANKER_ACTIVATE` — the bare guid, the same shape as
/// `CMSG_LIST_INVENTORY` and `CMSG_TRAINER_LIST`.
pub fn banker_activate_body(banker: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(banker);
    w.buf
}

/// Body for `CMSG_BUY_BANK_SLOT` — the banker again. The server prices the
/// slot itself (`GetBankBagSlotCount() + 1` into the table) and the client
/// names none.
pub fn buy_bank_slot_body(banker: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(banker);
    w.buf
}

/// Body for `CMSG_AUTOBANK_ITEM` and `CMSG_AUTOSTORE_BANK_ITEM` alike —
/// **`u8 srcBag, u8 srcSlot`** in the server's own numbering
/// ([`crate::play::items::server_container_slot`]). No destination: both
/// let `CanBankItem` / `CanStoreItem` choose, which is what a right-click
/// across the counter is.
pub fn bank_move_body(src_bag: u8, src_slot: u8) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(src_bag).u8(src_slot);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_show_packet_is_the_banker_and_nothing_else() {
        assert_eq!(parse_show_bank(&0x1234u64.to_le_bytes()), Some(0x1234));
        assert_eq!(parse_show_bank(&[1, 2, 3]), None);
    }

    #[test]
    fn every_result_code_reads_and_the_success_has_no_sentence() {
        assert_eq!(
            parse_buy_bank_slot_result(&1u32.to_le_bytes()),
            Some((1, Some(BankSlotResult::InsufficientFunds)))
        );
        assert_eq!(parse_buy_bank_slot_result(&9u32.to_le_bytes()), Some((9, None)));
        assert_eq!(parse_buy_bank_slot_result(&[0, 0]), None);
        assert_eq!(BankSlotResult::TooMany.key(), Some("ERR_BANKSLOT_FAILED_TOO_MANY"));
        assert_eq!(BankSlotResult::NotBanker.key(), Some("ERR_BANKSLOT_NOTBANKER"));
        assert_eq!(BankSlotResult::Ok.key(), None);
    }

    #[test]
    fn the_bodies_are_the_guid_and_the_pair() {
        assert_eq!(banker_activate_body(7), 7u64.to_le_bytes());
        assert_eq!(buy_bank_slot_body(7), 7u64.to_le_bytes());
        assert_eq!(bank_move_body(255, 23), vec![255, 23]);
    }
}
