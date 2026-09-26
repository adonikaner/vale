//! **Trading with another player** — twelve opcodes, two of them inbound, and
//! a window that both sides see the same way.
//!
//! ```text
//! CMSG_INITIATE_TRADE     u64 guid            ask somebody
//!   -> SMSG_TRADE_STATUS  BEGIN_TRADE, u64      …which they see as a popup
//! CMSG_BEGIN_TRADE        (no body)           the popup's Yes
//!   -> SMSG_TRADE_STATUS  OPEN_WINDOW           …to both, and the window opens
//! CMSG_SET_TRADE_ITEM     u8 slot, u8 bag, u8 slot   put a bag square in a slot
//! CMSG_CLEAR_TRADE_ITEM   u8 slot             …take it back out
//! CMSG_SET_TRADE_GOLD     u32 copper
//!   -> SMSG_TRADE_STATUS_EXTENDED               …each answered with the whole
//!                                               offer, to both sides
//!   -> SMSG_TRADE_STATUS  BACK_TO_TRADE         …and both accepts cleared
//! CMSG_ACCEPT_TRADE       u32 1
//!   -> SMSG_TRADE_STATUS  TRADE_ACCEPT          to the partner
//!   -> SMSG_TRADE_STATUS  TRADE_COMPLETE        to both, when both have
//! CMSG_UNACCEPT_TRADE     (no body)
//! CMSG_CANCEL_TRADE       (no body)           …the popup's No, and the window's
//!   -> SMSG_TRADE_STATUS  TRADE_CANCELED        to both
//! CMSG_BUSY_TRADE, CMSG_IGNORE_TRADE           (no body) — the two other refusals
//! ```
//!
//! The client's half is `client/src/game/session/trade.rs` and the panel is
//! `Interface\FrameXML\TradeFrame.lua`, whose whole content is two reads per
//! slot over what lands here.
//!
//! ## Both offers come from the server, including your own
//!
//! `SendUpdateTrade(trader_state)` is sent twice on every change: once to the
//! partner with `trader_state = 1` ("this is what the other side offers") and
//! once back to the one who changed it with `0` ("this is what you offer"). So
//! the client keeps no model of what it put in the window — the first byte of
//! [`TradeOffer`] says whose the seven slots are, and `GetTradePlayerItemInfo`
//! reads the echo. A client that drew its own offer locally would draw a slot
//! the server refused (a soulbound item, a bank slot) as if it were in.
//!
//! ## The seventh slot is not traded
//!
//! `TRADE_SLOT_NONTRADED` (6 on the wire, id 7 in the panel) is the "Will not
//! be traded" square: an item shown to the partner for an enchant or a lockpick
//! to be cast on, and returned untouched. The panel labels it from the offer's
//! `spell` field, which is the spell the partner has aimed at it.
//!
//! ## `BACK_TO_TRADE` is "both accepts are off"
//!
//! `TradeData::SetItem`, `SetMoney` and `SetSpell` each call `SetAccepted(false)`
//! on **both** sides and each side is told; `HandleUnacceptTrade` tells only the
//! partner. Read as "your accept is off" the second case would show a partner
//! un-accepted whom the server still holds accepted; read as "both are off" a
//! stale accept costs one more press of Trade, which the reference asks for
//! too. So the client clears both highlights on it.
//!
//! ## The status codes' words are this client's reading
//!
//! [`TradeStatus::key`] maps each refusal to a `GlobalStrings.lua` key. The
//! sentences are the game's; which code gets which sentence was not taken from
//! the client's own handler, and is labelled as a reading for that reason.
//! The two that are certain are the pair with their own keys —
//! `ERR_TRADE_CANCELLED` and `ERR_TRADE_COMPLETE`.
//!
//! Source: vmangos `Handlers/TradeHandler.cpp`, `Objects/TradeData.cpp`,
//! `Server/Packets/Trade.cpp` (the layouts) and `SharedDefines.h` (the codes).

use crate::bytes::{Reader, Writer};

/// Seven slots on each side: six traded and the one that is not.
pub const TRADE_SLOT_COUNT: usize = 7;
/// …of which the first six change hands.
pub const TRADE_SLOT_TRADED_COUNT: usize = 6;
/// The "Will not be traded" square, zero-based on the wire.
pub const TRADE_SLOT_NONTRADED: u8 = 6;
/// vmangos' `TRADE_DISTANCE` (`Player.h`): how far apart two players may stand
/// at the initiate and at the accept, centre to centre.
pub const TRADE_DISTANCE: f32 = 11.11;

/// `SMSG_TRADE_STATUS`'s first field — vmangos' `TradeStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeStatus {
    Busy = 0,
    BeginTrade = 1,
    OpenWindow = 2,
    TradeCanceled = 3,
    TradeAccept = 4,
    Busy2 = 5,
    NoTarget = 6,
    BackToTrade = 7,
    TradeComplete = 8,
    TradeRejected = 9,
    TargetTooFar = 10,
    WrongFaction = 11,
    CloseWindow = 12,
    Unknown13 = 13,
    IgnoreYou = 14,
    YouStunned = 15,
    TargetStunned = 16,
    YouDead = 17,
    TargetDead = 18,
    YouLogout = 19,
    TargetLogout = 20,
    TrialAccount = 21,
    OnlyConjured = 22,
}

impl TradeStatus {
    pub fn from_code(code: u32) -> Option<TradeStatus> {
        use TradeStatus::*;
        Some(match code {
            0 => Busy,
            1 => BeginTrade,
            2 => OpenWindow,
            3 => TradeCanceled,
            4 => TradeAccept,
            5 => Busy2,
            6 => NoTarget,
            7 => BackToTrade,
            8 => TradeComplete,
            9 => TradeRejected,
            10 => TargetTooFar,
            11 => WrongFaction,
            12 => CloseWindow,
            13 => Unknown13,
            14 => IgnoreYou,
            15 => YouStunned,
            16 => TargetStunned,
            17 => YouDead,
            18 => TargetDead,
            19 => YouLogout,
            20 => TargetLogout,
            21 => TrialAccount,
            22 => OnlyConjured,
            _ => return None,
        })
    }

    /// **Does this code end the trade?** Everything that closes the window
    /// or refuses to open one; the four that do not are the request, the
    /// open, the accept and the un-accept.
    pub fn ends_trade(self) -> bool {
        !matches!(
            self,
            TradeStatus::BeginTrade
                | TradeStatus::OpenWindow
                | TradeStatus::TradeAccept
                | TradeStatus::BackToTrade
        )
    }

    /// The `GlobalStrings.lua` key this code is said with, or `None` for the
    /// ones that are not said at all. A key ending `_S` wants the partner's
    /// name. See the module note: a reading, except for the two `ERR_TRADE_`
    /// keys that name their own code.
    pub fn key(self) -> Option<&'static str> {
        use TradeStatus::*;
        Some(match self {
            Busy | Busy2 => "ERR_PLAYER_BUSY_S",
            TradeCanceled | CloseWindow | Unknown13 | TradeRejected => "ERR_TRADE_CANCELLED",
            TradeComplete => "ERR_TRADE_COMPLETE",
            TargetTooFar => "ERR_TRADE_TOO_FAR",
            WrongFaction => "ERR_PLAYER_WRONG_FACTION",
            IgnoreYou => "ERR_IGNORING_YOU_S",
            YouStunned => "ERR_GENERIC_STUNNED",
            TargetStunned => "ERR_TARGET_STUNNED",
            YouDead => "ERR_PLAYER_DEAD",
            TargetDead => "ERR_TRADE_TARGET_DEAD",
            YouLogout => "ERR_LOGGING_OUT",
            TargetLogout => "ERR_TARGET_LOGGING_OUT",
            NoTarget => "ERR_GENERIC_NO_TARGET",
            OnlyConjured => "ERR_TRADE_WRONG_REALM",
            BeginTrade | OpenWindow | TradeAccept | BackToTrade | TrialAccount => return None,
        })
    }
}

/// `SMSG_TRADE_STATUS`, whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeStatusPacket {
    /// The raw code, kept beside its reading so an unknown one is data.
    pub code: u32,
    pub status: Option<TradeStatus>,
    /// `BEGIN_TRADE` only: who is asking.
    pub partner: Option<u64>,
}

/// `SMSG_TRADE_STATUS` — `{u32 status}`, then for `BEGIN_TRADE` `{u64 guid}`,
/// for `CLOSE_WINDOW` `{u32 result, u8, u32 limit category}` (dropped: the
/// three say why a trade could not be stored and the panel reads none of
/// them), for `ONLY_CONJURED` `{u8 slot}` (dropped likewise).
pub fn parse_trade_status(body: &[u8]) -> Option<TradeStatusPacket> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let code = r.u32();
    let status = TradeStatus::from_code(code);
    let partner = match status {
        Some(TradeStatus::BeginTrade) => r.has(8).then(|| r.u64()),
        _ => None,
    };
    Some(TradeStatusPacket {
        code,
        status,
        partner,
    })
}

/// One filled slot of an offer. Fifteen `u32`s' worth on the wire; the six
/// this client reads are named and the rest are carried for a plate that
/// wants them later.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TradeItem {
    pub entry: u32,
    pub display_id: u32,
    pub count: u32,
    pub wrapped: bool,
    pub gift_creator: u64,
    pub enchant: u32,
    pub creator: u64,
    pub charges: u32,
    pub suffix_factor: u32,
    pub random_property: u32,
    pub lock_id: u32,
    pub max_durability: u32,
    pub durability: u32,
}

/// `SMSG_TRADE_STATUS_EXTENDED`, whole: one side's seven slots and its money.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TradeOffer {
    /// The first byte: **1 is the partner's offer, 0 is the echo of your
    /// own.** See the module note.
    pub theirs: bool,
    pub money: u32,
    /// The spell aimed at the non-traded slot, or 0.
    pub spell: u32,
    pub items: [Option<TradeItem>; TRADE_SLOT_COUNT],
}

/// `SMSG_TRADE_STATUS_EXTENDED` — `{u8 theirs, u32 count, u32 count, u32 money,
/// u32 spell}` then seven records of `{u8 slot}` + 60 bytes. An empty slot is
/// the same 60 bytes as zeros, so the records are fixed-size and the slot byte
/// is read rather than trusted: a slot outside 0..7 ends the walk.
pub fn parse_trade_status_extended(body: &[u8]) -> Option<TradeOffer> {
    let mut r = Reader::new(body);
    if !r.has(1 + 4 * 4) {
        return None;
    }
    let theirs = r.u8() != 0;
    let _slots = r.u32();
    let _slots_again = r.u32();
    let money = r.u32();
    let spell = r.u32();
    let mut items: [Option<TradeItem>; TRADE_SLOT_COUNT] = Default::default();
    while r.has(1 + 60) {
        let slot = usize::from(r.u8());
        if slot >= TRADE_SLOT_COUNT {
            break;
        }
        let item = TradeItem {
            entry: r.u32(),
            display_id: r.u32(),
            count: r.u32(),
            wrapped: r.u32() != 0,
            gift_creator: r.u64(),
            enchant: r.u32(),
            creator: r.u64(),
            charges: r.u32(),
            suffix_factor: r.u32(),
            random_property: r.u32(),
            lock_id: r.u32(),
            max_durability: r.u32(),
            durability: r.u32(),
        };
        items[slot] = (item.entry != 0).then_some(item);
    }
    Some(TradeOffer {
        theirs,
        money,
        spell,
        items,
    })
}

/// `CMSG_INITIATE_TRADE` — `{u64 guid}`.
pub fn initiate_trade_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// `CMSG_ACCEPT_TRADE` — one `u32` the server skips ("set to 1 when the player
/// got `OPEN_WINDOW` at least once this session").
pub fn accept_trade_body() -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(1);
    w.buf
}

/// `CMSG_SET_TRADE_ITEM` — `{u8 trade slot, u8 bag, u8 slot}`, the bag and the
/// slot in the **server's** numbering (see [`super::items::server_container_slot`]).
pub fn set_trade_item_body(trade_slot: u8, bag: u8, slot: u8) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(trade_slot).u8(bag).u8(slot);
    w.buf
}

/// `CMSG_CLEAR_TRADE_ITEM` — `{u8 trade slot}`.
pub fn clear_trade_item_body(trade_slot: u8) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(trade_slot);
    w.buf
}

/// `CMSG_SET_TRADE_GOLD` — `{u32 copper}`.
pub fn set_trade_gold_body(copper: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(copper);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_reads_its_code_and_only_begin_trade_carries_a_guid() {
        let mut w = Writer::new();
        w.u32(1).u64(0x0000_0000_0000_002a);
        let begin = parse_trade_status(&w.buf).expect("reads");
        assert_eq!(begin.status, Some(TradeStatus::BeginTrade));
        assert_eq!(begin.partner, Some(42));

        let mut w = Writer::new();
        w.u32(8);
        let done = parse_trade_status(&w.buf).expect("reads");
        assert_eq!(done.status, Some(TradeStatus::TradeComplete));
        assert_eq!(done.partner, None);
        assert!(done.status.unwrap().ends_trade());
        assert!(!TradeStatus::TradeAccept.ends_trade());

        let mut w = Writer::new();
        w.u32(99);
        let odd = parse_trade_status(&w.buf).expect("an unknown code is data");
        assert_eq!(odd.code, 99);
        assert_eq!(odd.status, None);
        assert_eq!(parse_trade_status(&[1, 0]), None, "short is nothing");
    }

    fn offer(theirs: u8, money: u32, filled: &[(u8, u32, u32)]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(theirs).u32(7).u32(7).u32(money).u32(0);
        for slot in 0..7u8 {
            w.u8(slot);
            match filled.iter().find(|(s, _, _)| *s == slot) {
                Some((_, entry, count)) => {
                    w.u32(*entry).u32(1234).u32(*count).u32(0).u64(0).u32(0).u64(0);
                    w.u32(0).u32(0).u32(0).u32(0).u32(40).u32(35);
                }
                None => {
                    for _ in 0..15 {
                        w.u32(0);
                    }
                }
            }
        }
        w.buf
    }

    #[test]
    fn an_extended_status_is_seven_fixed_records_and_an_empty_one_is_none() {
        let body = offer(1, 12_345, &[(0, 2589, 20), (6, 25, 1)]);
        assert_eq!(body.len(), 17 + 7 * 61);
        let got = parse_trade_status_extended(&body).expect("reads");
        assert!(got.theirs);
        assert_eq!(got.money, 12_345);
        let first = got.items[0].as_ref().expect("slot 0");
        assert_eq!((first.entry, first.count, first.durability), (2589, 20, 35));
        assert!(got.items[1].is_none());
        assert_eq!(got.items[6].as_ref().map(|i| i.entry), Some(25));

        let mine = parse_trade_status_extended(&offer(0, 0, &[])).expect("reads");
        assert!(!mine.theirs);
        assert!(mine.items.iter().all(Option::is_none));
    }

    #[test]
    fn the_bodies_are_the_layouts_the_server_reads() {
        assert_eq!(initiate_trade_body(0x1122).len(), 8);
        assert_eq!(accept_trade_body(), vec![1, 0, 0, 0]);
        assert_eq!(set_trade_item_body(2, 255, 23), vec![2, 255, 23]);
        assert_eq!(clear_trade_item_body(6), vec![6]);
        assert_eq!(set_trade_gold_body(0x0102), vec![2, 1, 0, 0]);
    }

    #[test]
    fn every_code_has_a_reading() {
        for code in 0..=22 {
            let status = TradeStatus::from_code(code).expect("all 23 are named");
            assert_eq!(status as u32, code);
        }
        assert_eq!(TradeStatus::from_code(23), None);
        assert_eq!(TradeStatus::TradeComplete.key(), Some("ERR_TRADE_COMPLETE"));
        assert_eq!(TradeStatus::OpenWindow.key(), None);
    }
}
