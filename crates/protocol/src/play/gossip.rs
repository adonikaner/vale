//! **Talking to an NPC** — the gossip menu, the text behind it, and the vendor
//! window one click deeper.
//!
//! ```text
//! CMSG_GOSSIP_HELLO       guid        right-click anything with the gossip flag
//!   -> SMSG_GOSSIP_MESSAGE            a text *id*, the options, the quests
//! CMSG_NPC_TEXT_QUERY     id, guid    …and the text is a round trip of its own
//!   -> SMSG_NPC_TEXT_UPDATE
//! CMSG_GOSSIP_SELECT_OPTION  guid, n  press a line
//!   -> whatever the line was: another menu, a vendor, a taxi map, the bank
//! SMSG_GOSSIP_COMPLETE                the server closed the window
//! SMSG_GOSSIP_POI         flags,x,y,icon,data,name   …and a flag on the map
//! CMSG_LIST_INVENTORY     guid        …and the vendor half
//!   -> SMSG_LIST_INVENTORY            one row per thing for sale
//! CMSG_BUY_ITEM / CMSG_SELL_ITEM      …and the two verbs over it
//!   -> SMSG_BUY_ITEM / SMSG_BUY_FAILED / SMSG_SELL_ITEM
//! ```
//!
//! ## The menu carries a text *id* and the text is somebody else's packet
//!
//! `SMSG_GOSSIP_MESSAGE` writes `uint32 textId` where every other page in the
//! game writes its words inline. The words live in the server's `npc_text`
//! table, fetched by `CMSG_NPC_TEXT_QUERY` and cached per id for the session —
//! exactly the shape an item template has, and with the same visible state: a
//! gossip window is briefly wordless and then fills, which is the reference's
//! own behaviour. The client's fallback for an id that never answers is its own
//! literal — `"Missing gossip text!"`.
//!
//! ## An option's icon is a *word* to the interface, and the table is the
//! client's own
//!
//! `GossipFrameOptionsUpdate` pastes its second return into
//! `Interface\GossipFrame\<word>GossipIcon`, so the wire's icon byte has to
//! become one of eleven strings — and they are the client's own. As an array
//! that is `gossip = 0, vendor = 1, taxi = 2, trainer = 3, healer = 4, binder = 5,
//! banker = 6, petition = 7, tabard = 8, battlemaster = 9, auctioneer = 10` —
//! which agrees with vmangos' `GOSSIP_ICON_*` enum on every slot the two name.
//! See [`icon_word`].
//!
//! ## A vendor row's `maxcount` is a sentinel, not a number
//!
//! `SendListInventory` writes `0xFFFFFFFF` for an item with unlimited stock and
//! the *remaining count* for a limited one — and `GetMerchantItemInfo`'s
//! `numAvailable` is `-1` for unlimited, which is what `MerchantFrame.lua`
//! compares against. Reading the sentinel as a count shows four billion Brown
//! Linen Shirts.
//!
//! ## …and an empty vendor is a count byte followed by an error byte
//!
//! `count == 0` is followed by one more byte (vmangos writes 0, "vendor has no
//! inventory"), so a parser that stops at the count is one byte short on
//! exactly the vendor that a server misconfiguration produces.

use crate::bytes::{Reader, Writer};
use crate::play::quest::QuestOffer;

/// The interface's word for each icon byte — see the module note, which is
/// where the addresses are. Index 0 is icon 0.
pub const ICON_WORDS: [&str; 11] = [
    "gossip",
    "vendor",
    "taxi",
    "trainer",
    "healer",
    "binder",
    "banker",
    "petition",
    "tabard",
    "battlemaster",
    "auctioneer",
];

/// The word `GossipFrame` builds an icon path from, for one wire byte.
///
/// **An unknown icon is `"gossip"` rather than nothing**: the failure mode of a
/// wrong word is a missing texture on one line, and of a nil is the whole
/// `OnEvent` aborting mid-update.
pub fn icon_word(icon: u8) -> &'static str {
    ICON_WORDS.get(usize::from(icon)).copied().unwrap_or("gossip")
}

/// The client's own fallback for a text id the server never answers — and it
/// is worth being the client's words rather than an empty string: an empty gossip page reads as a bug where this reads as the game.
pub const MISSING_TEXT: &str = "Missing gossip text!";

/// How close a character must stand to talk to an NPC, in yards —
/// vmangos' `INTERACTION_DISTANCE` (`ObjectDefines.h`), the gate
/// `Player::CanInteractWithNPC` tests on every hello, list-inventory and
/// questgiver packet.
///
/// **A reading of vmangos, not of the client.** The server's test adds both units' bounding radii to the five;
/// the client-side walk-away check measures through the same
/// combat-reach subtraction its range reads use, which is the closest
/// available analogue. The server itself never announces a walk-away: the
/// reference client closes its windows locally, and so does this one.
pub const INTERACTION_DISTANCE: f32 = 5.0;

/// One line of a gossip menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GossipOption {
    /// **The server's own index**, echoed back in
    /// `CMSG_GOSSIP_SELECT_OPTION` — written explicitly on the wire, so it is
    /// carried rather than recomputed from position.
    pub index: u32,
    pub icon: u8,
    /// Whether pressing it opens a text-entry box (an unlock code). The box is
    /// not implemented; a coded option is drawn and its press sends nothing,
    /// which is stated in `game::gossip`.
    pub coded: bool,
    pub text: String,
}

/// `SMSG_GOSSIP_MESSAGE`, whole.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GossipMenu {
    pub guid: u64,
    /// Into the server's `npc_text` table — see the module note.
    pub text_id: u32,
    pub options: Vec<GossipOption>,
    /// The same shape a questgiver's greeting carries, and the same split: the
    /// icon says which half of the panel a quest draws in.
    pub quests: Vec<QuestOffer>,
}

/// One thing a vendor sells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VendorItem {
    /// One-based on the wire — `SendListInventory` writes `count` after the
    /// increment — and `CMSG_BUY_ITEM` takes the *entry*, so this is carried
    /// for `MERCHANT_UPDATE`'s stock changes rather than for the buy.
    pub slot: u32,
    pub entry: u32,
    pub display_id: u32,
    /// `None` is unlimited — the wire's `0xFFFFFFFF`; see the module note.
    pub available: Option<u32>,
    /// Copper, the reputation discount already applied by the server.
    pub price: u32,
    pub max_durability: u32,
    /// How many one purchase buys — a stack of 5 arrows per click.
    pub buy_count: u32,
}

/// `SMSG_LIST_INVENTORY`, whole.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VendorList {
    pub guid: u64,
    pub items: Vec<VendorItem>,
    /// The byte after a zero count — an empty vendor's "why". Only ever 0 from
    /// vmangos.
    pub empty_reason: Option<u8>,
}

/// Why a buy did not happen — `SMSG_BUY_FAILED`'s byte, vmangos' `BuyResult`.
/// The numbering is sparse; the gaps are gaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuyFailure {
    CantFindItem,
    AlreadySold,
    NotEnoughMoney,
    SellerDoesNotLikeYou,
    TooFar,
    SoldOut,
    CantCarryMore,
    RankRequired,
    ReputationRequired,
}

impl BuyFailure {
    pub fn of(byte: u8) -> Option<BuyFailure> {
        Some(match byte {
            0 => BuyFailure::CantFindItem,
            1 => BuyFailure::AlreadySold,
            2 => BuyFailure::NotEnoughMoney,
            4 => BuyFailure::SellerDoesNotLikeYou,
            5 => BuyFailure::TooFar,
            7 => BuyFailure::SoldOut,
            8 => BuyFailure::CantCarryMore,
            11 => BuyFailure::RankRequired,
            12 => BuyFailure::ReputationRequired,
            _ => return None,
        })
    }
}

/// …and why a sell did not — `SMSG_SELL_ITEM`, which the server only sends as
/// a refusal: a successful sale is money and an item removal in the ordinary
/// update fields, and no packet says anything else about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SellFailure {
    CantFindItem,
    CantSellItem,
    CantFindVendor,
    YouDontOwnThatItem,
    Unknown,
    OnlyEmptyBag,
}

impl SellFailure {
    pub fn of(byte: u8) -> Option<SellFailure> {
        Some(match byte {
            1 => SellFailure::CantFindItem,
            2 => SellFailure::CantSellItem,
            3 => SellFailure::CantFindVendor,
            4 => SellFailure::YouDontOwnThatItem,
            5 => SellFailure::Unknown,
            6 => SellFailure::OnlyEmptyBag,
            _ => return None,
        })
    }
}

// --- parsing ----------------------------------------------------------------

/// `SMSG_GOSSIP_MESSAGE`.
/// `SMSG_GOSSIP_POI` (548) — **a place, put on the world map by name**.
///
/// `PlayerMenu::SendPointOfInterest` (`GossipDef.cpp:239`) writes
/// `u32 flags, f32 x, f32 y, u32 icon, u32 data, cstring name`, and the two
/// senders are a gossip line whose action is a POI id and the scripted
/// "directions" a city guard gives. It is the one thing on this wire that puts
/// a mark on the map, and the client keeps exactly **one** — a second replaces
/// the first, because the reference stores it in a single synthetic `AreaPOI`
/// row. See [`vale_assets::tables::areapoi::GossipPoi`].
///
/// **The packet carries no map.** The reference projects it against whichever
/// map is open, from the character's own, which is what the server means: a
/// guard in Stormwind is naming a place in Stormwind.
pub fn parse_gossip_poi(body: &[u8]) -> Option<(u32, f32, f32, u32, u32, String)> {
    let mut r = crate::bytes::Reader::new(body);
    // **The five words before the name are checked up front**, which is what
    // the `has` guard is for: the `Reader` answers zeroes past the end, and a
    // truncated body would otherwise read as a flag at the origin — the map's
    // own "off this map" test is `0, 0`, so it would draw in the top-left
    // corner of every parchment rather than not at all.
    if !r.has(20) {
        return None;
    }
    let flags = r.u32();
    let x = r.f32();
    let y = r.f32();
    let icon = r.u32();
    let data = r.u32();
    let name = r.cstring();
    Some((flags, x, y, icon, data, name))
}

pub fn parse_gossip_message(body: &[u8]) -> Option<GossipMenu> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4 + 4) {
        return None;
    }
    let guid = r.u64();
    let text_id = r.u32();
    let option_count = r.u32().min(32);
    let mut options = Vec::with_capacity(option_count as usize);
    for _ in 0..option_count {
        // index, icon, coded, then the text.
        if !r.has(4 + 1 + 1) {
            break;
        }
        let index = r.u32();
        let icon = r.u8();
        let coded = r.u8() != 0;
        let Some(text) = cstr(&mut r) else { break };
        options.push(GossipOption {
            index,
            icon,
            coded,
            text,
        });
    }
    if !r.has(4) {
        return None;
    }
    let quest_count = r.u32().min(64);
    let mut quests = Vec::with_capacity(quest_count as usize);
    for _ in 0..quest_count {
        if !r.has(12) {
            break;
        }
        let quest_id = r.u32();
        let icon = r.u32();
        let level = r.u32();
        let Some(title) = cstr(&mut r) else { break };
        quests.push(QuestOffer {
            quest_id,
            icon,
            level,
            title,
        });
    }
    Some(GossipMenu {
        guid,
        text_id,
        options,
        quests,
    })
}

/// `SMSG_NPC_TEXT_UPDATE` — the id, and **the first page's text**.
///
/// The packet is eight pages of `(probability, male text, female text, lang,
/// three emote pairs)`, and the reference rolls between the pages by
/// probability. This takes the first page whose text is non-empty, which is
/// the whole table for all but a handful of flavour NPCs — a **stated
/// approximation**: what is lost is variety between visits, never words.
pub fn parse_npc_text_update(body: &[u8]) -> Option<(u32, String)> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let text_id = r.u32();
    let mut best = String::new();
    for _ in 0..8 {
        if !r.has(4) {
            break;
        }
        let _probability = r.f32();
        let Some(male) = cstr(&mut r) else { break };
        let Some(female) = cstr(&mut r) else { break };
        // language, then three (delay, emote) pairs.
        if !r.has(4 * 7) {
            break;
        }
        for _ in 0..7 {
            r.u32();
        }
        if best.is_empty() {
            best = if male.is_empty() { female } else { male };
        }
    }
    Some((text_id, best))
}

/// `SMSG_LIST_INVENTORY`.
pub fn parse_list_inventory(body: &[u8]) -> Option<VendorList> {
    let mut r = Reader::new(body);
    if !r.has(8 + 1) {
        return None;
    }
    let guid = r.u64();
    let count = r.u8();
    // **Zero is followed by a reason byte** — see the module note.
    if count == 0 {
        return Some(VendorList {
            guid,
            items: Vec::new(),
            empty_reason: r.has(1).then(|| r.u8()),
        });
    }
    let mut items = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        if !r.has(4 * 7) {
            break;
        }
        let slot = r.u32();
        let entry = r.u32();
        let display_id = r.u32();
        let available = match r.u32() {
            0xFFFF_FFFF => None,
            n => Some(n),
        };
        items.push(VendorItem {
            slot,
            entry,
            display_id,
            available,
            price: r.u32(),
            max_durability: r.u32(),
            buy_count: r.u32(),
        });
    }
    Some(VendorList {
        guid,
        items,
        empty_reason: None,
    })
}

/// `SMSG_BUY_ITEM` — a purchase went through: the vendor, the **one-based**
/// slot, the stock left (`None` unlimited), and how many were bought.
pub fn parse_buy_item(body: &[u8]) -> Option<(u64, u32, Option<u32>, u32)> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4 + 4 + 4) {
        return None;
    }
    let guid = r.u64();
    let slot = r.u32();
    let left = match r.u32() {
        0xFFFF_FFFF => None,
        n => Some(n),
    };
    Some((guid, slot, left, r.u32()))
}

/// `SMSG_BUY_FAILED` — guid, item entry, reason.
pub fn parse_buy_failed(body: &[u8]) -> Option<(u64, u32, Option<BuyFailure>)> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4 + 1) {
        return None;
    }
    let guid = r.u64();
    let entry = r.u32();
    Some((guid, entry, BuyFailure::of(r.u8())))
}

/// `SMSG_SELL_ITEM` — the refusal; see [`SellFailure`].
pub fn parse_sell_failed(body: &[u8]) -> Option<(u64, u64, Option<SellFailure>)> {
    let mut r = Reader::new(body);
    if !r.has(8 + 8 + 1) {
        return None;
    }
    let vendor = r.u64();
    let item = r.u64();
    Some((vendor, item, SellFailure::of(r.u8())))
}

// --- what we send -----------------------------------------------------------

/// `CMSG_GOSSIP_HELLO` and `CMSG_LIST_INVENTORY` — a guid apiece.
pub fn guid_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// `CMSG_GOSSIP_SELECT_OPTION` — the guid and the **server's** option index.
///
/// A coded option would append its answer string here; coded options are not
/// pressed by this client, so the field never exists.
pub fn select_option_body(guid: u64, option: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.u32(option);
    w.buf
}

/// `CMSG_NPC_TEXT_QUERY` — **the id first and then the guid**, which is
/// backwards from every other query in the game and is the server's own read
/// order (`recv_data >> textID; recv_data >> guid`).
pub fn npc_text_query_body(text_id: u32, guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(text_id);
    w.u64(guid);
    w.buf
}

/// `CMSG_BUY_ITEM` — vendor, item **entry**, how many, and a trailing byte the
/// server reads and ignores.
///
/// **`count` is a number of purchases, not of items**, and the two differ by
/// the row's own [`VendorItem::buy_count`] every time it is above one.
/// `Player::BuyItemFromVendor` is explicit about it in two lines that have to be
/// read together — `uint32 totalCount = pProto->BuyCount * count;` and `uint32
/// price = pProto->BuyPrice * count;` — so a client that sends the item count it
/// wants receives `BuyCount` times that many and pays `BuyCount` times over.
/// Nothing refuses it and no error comes back: the goods and the charge agree
/// with each other, and only with the wrong quantity.
pub fn buy_item_body(vendor: u64, entry: u32, count: u8) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(vendor);
    w.u32(entry);
    w.u8(count);
    w.u8(0);
    w.buf
}

/// `CMSG_SELL_ITEM` — vendor, the item object's **guid**, how many (0 for the
/// whole stack, which is what a right-click sells).
pub fn sell_item_body(vendor: u64, item: u64, count: u8) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(vendor);
    w.u64(item);
    w.u8(count);
    w.buf
}

/// `CMSG_BUYBACK_ITEM` — vendor and the **wire's** buyback slot, which starts
/// at 69 (`BUYBACK_SLOT_START`): the interface's slot 1 is 69 on the wire.
pub fn buyback_body(vendor: u64, wire_slot: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(vendor);
    w.u32(wire_slot);
    w.buf
}

/// `CMSG_REPAIR_ITEM` — the armourer, and **which** item, where `0` means all
/// of them.
///
/// Two guids and nothing else. The zero is not a sentinel invented here:
/// `HandleRepairItemOpcode` reads `npcGuid >> itemGuid` and branches on
/// `if (itemGuid)` — a guid repairs that one item at its own cost, and a zero
/// takes `DurabilityRepairAll`, which walks the worn slots, the backpack and
/// every bag. So `RepairAllItems()` and the repair cursor are the same opcode
/// with one field different.
///
/// **There is no answer packet.** The server charges the money, sets
/// `ITEM_FIELD_DURABILITY` to the maximum and sends nothing — the client learns
/// what happened from the update blocks for the money and the items, which is
/// why nothing here waits for a reply. A repair that is refused (out of money,
/// out of range, an NPC without `UNIT_NPC_FLAG_REPAIR`) is refused in **silence**
/// too: the handler returns, and the item simply does not change.
pub fn repair_item_body(vendor: u64, item: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(vendor);
    w.u64(item);
    w.buf
}

/// A null-terminated string, or `None` when it never terminated — the same
/// guard [`crate::play::quest`] keeps, for the same packets-with-fields-after reason.
fn cstr(r: &mut Reader) -> Option<String> {
    let before = r.remaining();
    let s = r.cstring();
    (r.remaining() + s.len() < before).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(options: &[(u32, u8, &str)], quests: &[(u32, u32, &str)]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u64(0x77);
        w.u32(1234);
        w.u32(options.len() as u32);
        for (index, icon, text) in options {
            w.u32(*index);
            w.u8(*icon);
            w.u8(0);
            w.bytes(text.as_bytes());
            w.u8(0);
        }
        w.u32(quests.len() as u32);
        for (id, icon, title) in quests {
            w.u32(*id);
            w.u32(*icon);
            w.u32(5);
            w.bytes(title.as_bytes());
            w.u8(0);
        }
        w.buf
    }

    /// The menu, whole: the text id, both lists, and the server's own indices
    /// carried rather than recomputed.
    #[test]
    fn a_gossip_menu_carries_a_text_id_and_both_lists() {
        let parsed = parse_gossip_message(&menu(
            &[(0, 1, "Let me browse your goods."), (1, 3, "Train me.")],
            &[(47, 5, "Kill Six Kobolds")],
        ))
        .expect("a menu");
        assert_eq!(parsed.guid, 0x77);
        assert_eq!(parsed.text_id, 1234, "an id, not words");
        assert_eq!(parsed.options.len(), 2);
        assert_eq!(parsed.options[0].index, 0);
        assert_eq!(parsed.options[0].icon, 1);
        assert_eq!(parsed.options[1].text, "Train me.");
        assert_eq!(parsed.quests.len(), 1);
        assert_eq!(parsed.quests[0].quest_id, 47);
    }

    /// **The icon table is the client's own**, in the client's own order — and
    /// an unknown byte degrades to `"gossip"` rather than to a nil that would
    /// abort the panel's whole update.
    #[test]
    fn every_icon_byte_becomes_one_of_the_clients_own_words() {
        assert_eq!(icon_word(0), "gossip");
        assert_eq!(icon_word(1), "vendor");
        assert_eq!(icon_word(2), "taxi");
        assert_eq!(icon_word(3), "trainer");
        assert_eq!(icon_word(5), "binder");
        assert_eq!(icon_word(9), "battlemaster");
        assert_eq!(icon_word(10), "auctioneer");
        assert_eq!(icon_word(99), "gossip");
    }

    /// The text update: eight pages, first non-empty wins, female text stands
    /// in when the male one is blank.
    #[test]
    fn the_npc_text_takes_the_first_page_with_words_on_it() {
        let mut w = Writer::new();
        w.u32(1234);
        // Page 1: blank male, female carries it.
        w.f32(0.5);
        w.bytes(b"\0");
        w.bytes(b"Well met, $N.\0");
        for _ in 0..7 {
            w.u32(0);
        }
        // Page 2: words that must not displace page 1's.
        w.f32(0.5);
        w.bytes(b"Second page.\0");
        w.bytes(b"\0");
        for _ in 0..7 {
            w.u32(0);
        }
        let (id, text) = parse_npc_text_update(&w.buf).expect("a text");
        assert_eq!(id, 1234);
        assert_eq!(text, "Well met, $N.");
    }

    fn vendor_row(w: &mut Writer, slot: u32, entry: u32, available: u32, price: u32) {
        w.u32(slot);
        w.u32(entry);
        w.u32(entry + 7);
        w.u32(available);
        w.u32(price);
        w.u32(0);
        w.u32(1);
    }

    /// **`0xFFFFFFFF` is "unlimited", not a count** — the difference between a
    /// shop and four billion shirts.
    #[test]
    fn a_vendors_stock_sentinel_reads_as_unlimited() {
        let mut w = Writer::new();
        w.u64(0x77);
        w.u8(2);
        vendor_row(&mut w, 1, 2589, 0xFFFF_FFFF, 15);
        vendor_row(&mut w, 2, 858, 3, 400);
        let vendor = parse_list_inventory(&w.buf).expect("a list");
        assert_eq!(vendor.items.len(), 2);
        assert_eq!(vendor.items[0].available, None, "unlimited");
        assert_eq!(vendor.items[1].available, Some(3));
        assert_eq!(vendor.items[1].price, 400);
        assert_eq!(vendor.empty_reason, None);
    }

    /// An empty vendor is a zero count **and one more byte**; stopping at the
    /// count is one byte short on exactly the misconfigured-vendor case.
    #[test]
    fn an_empty_vendor_carries_a_reason_byte() {
        let mut w = Writer::new();
        w.u64(0x77);
        w.u8(0);
        w.u8(0);
        let vendor = parse_list_inventory(&w.buf).expect("a list");
        assert!(vendor.items.is_empty());
        assert_eq!(vendor.empty_reason, Some(0));
    }

    /// The two refusals and the success, plus the six bodies.
    #[test]
    fn the_verbs_and_their_answers_round_trip() {
        let mut w = Writer::new();
        w.u64(0x77);
        w.u32(2589);
        w.u8(2);
        assert_eq!(
            parse_buy_failed(&w.buf),
            Some((0x77, 2589, Some(BuyFailure::NotEnoughMoney)))
        );
        let mut w = Writer::new();
        w.u64(0x77);
        w.u64(0x99);
        w.u8(2);
        assert_eq!(
            parse_sell_failed(&w.buf),
            Some((0x77, 0x99, Some(SellFailure::CantSellItem)))
        );
        let mut w = Writer::new();
        w.u64(0x77);
        w.u32(3);
        w.u32(0xFFFF_FFFF);
        w.u32(5);
        assert_eq!(parse_buy_item(&w.buf), Some((0x77, 3, None, 5)));

        assert_eq!(guid_body(9).len(), 8);
        assert_eq!(select_option_body(9, 2).len(), 12);
        // **Id first, then guid** — backwards from every other query.
        let q = npc_text_query_body(1234, 9);
        assert_eq!(&q[0..4], &1234u32.to_le_bytes());
        assert_eq!(&q[4..12], &9u64.to_le_bytes());
        assert_eq!(buy_item_body(9, 2589, 5).len(), 14);
        assert_eq!(sell_item_body(9, 0x99, 0).len(), 17);
        assert_eq!(buyback_body(9, 69).len(), 12);
    }

    /// The gaps in both refusal tables are gaps, not silently something.
    #[test]
    fn the_refusal_numberings_are_sparse_and_stay_sparse() {
        assert_eq!(BuyFailure::of(3), None);
        assert_eq!(BuyFailure::of(6), None);
        assert_eq!(BuyFailure::of(9), None);
        assert_eq!(SellFailure::of(0), None);
        assert_eq!(SellFailure::of(7), None);
    }
}
