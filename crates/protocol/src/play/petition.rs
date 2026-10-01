//! The guild charter: seven packets in and nine requests out.
//!
//! ```text
//! SMSG_PETITION_SHOWLIST        444  u64 npc, u8 count, then per entry:
//!                                    u32 index, u32 item, u32 display,
//!                                    u32 cost, u32 flags
//! SMSG_PETITION_SHOW_SIGNATURES 447  u64 item, u64 owner, u32 petition,
//!                                    u8 count, count x (u64 signer, u32 0)
//! SMSG_PETITION_SIGN_RESULTS    449  u64 item, u64 player, u32 result
//! MSG_PETITION_DECLINE          450  u64 player
//! SMSG_TURN_IN_PETITION_RESULTS 453  u32 result
//! SMSG_PETITION_QUERY_RESPONSE  455  u32 petition, u64 owner, cstring name,
//!                                    cstring body, u32 flags, u32 min,
//!                                    u32 max, then 31 bytes this client
//!                                    does not read
//! MSG_PETITION_RENAME           705  u64 item, cstring name
//!
//! CMSG_PETITION_SHOWLIST        443  u64 npc
//! CMSG_PETITION_BUY             445  u64 npc, u32 0, u64 0, cstring name,
//!                                    cstring body, 8 x u32 0, u16 0,
//!                                    u32 0, u32 0, u32 choices, u32 index
//! CMSG_PETITION_SHOW_SIGNATURES 446  u64 item
//! CMSG_PETITION_SIGN            448  u64 item, u8 1
//! MSG_PETITION_DECLINE          450  u64 item
//! CMSG_OFFER_PETITION           451  u64 item, u64 player
//! CMSG_TURN_IN_PETITION         452  u64 item
//! CMSG_PETITION_QUERY           454  u32 petition, u64 item
//! MSG_PETITION_RENAME           705  u64 item, cstring name
//! ```
//!
//! The layouts follow vmangos' `Server/Packets/Petition.cpp` and
//! `Handlers/PetitionsHandler.cpp`, and the result values follow the
//! `PetitionSigns` enumeration in its `Guild/Guild.h`.
//!
//! ## A charter is an item, and the petition is a number on it
//!
//! Buying a charter puts item 5863 in the bags. Every request about the
//! charter names that item's guid. The petition's own id is the item's first
//! enchantment field, and the client needs it for one request only,
//! `CMSG_PETITION_QUERY`; the server states it in
//! `SMSG_PETITION_SHOW_SIGNATURES`.
//!
//! ## The signatures carry guids and no names
//!
//! `SMSG_PETITION_SHOW_SIGNATURES` lists each signer as a guid. The names
//! are `CMSG_NAME_QUERY`'s to fetch, and the guild's proposed name and the
//! owner are `SMSG_PETITION_QUERY_RESPONSE`'s.
//!
//! ## The purchase body is a petition record
//!
//! `CMSG_PETITION_BUY` has the layout of `SMSG_PETITION_QUERY_RESPONSE`
//! after its first field: an owner, a name, a body text, the limits and
//! restrictions, and a choice count, with the registrar's guid in front and
//! the offer's index behind. The 1.12.1 client fills in the name and the
//! index and writes every other field as zero. vmangos reads the guid and
//! the name and skips the 51 bytes after it.
//!
//! ## What the server does not answer
//!
//! A purchase that succeeds sends the item and no petition packet. An offer
//! that succeeds sends the signatures to the other player and nothing to the
//! sender. A refusal of either arrives as `SMSG_GUILD_COMMAND_RESULT`; see
//! [`super::guild`].

use crate::bytes::{Reader, Writer};

/// The most signatures a charter takes. The server refuses a tenth, and the
/// petition window has nine lines.
pub const MAX_SIGNATURES: usize = 9;

/// One row of `SMSG_PETITION_SHOWLIST`: a charter the registrar sells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharterOffer {
    /// The index `CMSG_PETITION_BUY` echoes. vmangos sends 1.
    pub index: u32,
    /// The charter's item entry, 5863 on vmangos.
    pub item: u32,
    /// An `ItemDisplayInfo.dbc` id.
    pub display: u32,
    /// The price in copper.
    pub cost: u32,
    pub flags: u32,
}

/// `SMSG_PETITION_SHOWLIST` (444): what a guild registrar sells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowList {
    pub npc: u64,
    pub offers: Vec<CharterOffer>,
}

/// `SMSG_PETITION_SHOWLIST` (444).
pub fn parse_show_list(body: &[u8]) -> Option<ShowList> {
    let mut r = Reader::new(body);
    if !r.has(9) {
        return None;
    }
    let npc = r.u64();
    let count = r.u8() as usize;
    let mut offers = Vec::with_capacity(count);
    for _ in 0..count {
        if !r.has(20) {
            break;
        }
        offers.push(CharterOffer {
            index: r.u32(),
            item: r.u32(),
            display: r.u32(),
            cost: r.u32(),
            flags: r.u32(),
        });
    }
    Some(ShowList { npc, offers })
}

/// `SMSG_PETITION_SHOW_SIGNATURES` (447): a charter, its owner and who has
/// signed it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signatures {
    /// The charter item's guid.
    pub item: u64,
    pub owner: u64,
    /// The petition's id, for `CMSG_PETITION_QUERY`.
    pub petition: u32,
    /// The signers' guids, in the order they signed.
    pub signers: Vec<u64>,
}

/// `SMSG_PETITION_SHOW_SIGNATURES` (447).
pub fn parse_signatures(body: &[u8]) -> Option<Signatures> {
    let mut r = Reader::new(body);
    if !r.has(21) {
        return None;
    }
    let item = r.u64();
    let owner = r.u64();
    let petition = r.u32();
    let count = r.u8() as usize;
    let mut signers = Vec::with_capacity(count.min(MAX_SIGNATURES));
    for _ in 0..count {
        // A guid and a word the server always writes as zero.
        if !r.has(12) {
            break;
        }
        signers.push(r.u64());
        r.skip(4);
    }
    Some(Signatures {
        item,
        owner,
        petition,
        signers,
    })
}

/// `SMSG_PETITION_QUERY_RESPONSE` (455): what a petition says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PetitionQuery {
    pub petition: u32,
    pub owner: u64,
    /// The proposed guild name.
    pub name: String,
    /// vmangos sends an empty body.
    pub body: String,
    pub flags: u32,
    pub min_signatures: u32,
    pub max_signatures: u32,
}

/// `SMSG_PETITION_QUERY_RESPONSE` (455).
///
/// The fields after the two signature counts are a deadline, a creation
/// time, five restrictions and a list of choices. vmangos writes every one
/// as zero and this client reads none of them.
pub fn parse_query(body: &[u8]) -> Option<PetitionQuery> {
    let mut r = Reader::new(body);
    if !r.has(12) {
        return None;
    }
    let petition = r.u32();
    let owner = r.u64();
    let name = r.cstring();
    let text = r.cstring();
    if !r.has(12) {
        return None;
    }
    Some(PetitionQuery {
        petition,
        owner,
        name,
        body: text,
        flags: r.u32(),
        min_signatures: r.u32(),
        max_signatures: r.u32(),
    })
}

/// The result word of `SMSG_PETITION_SIGN_RESULTS` and
/// `SMSG_TURN_IN_PETITION_RESULTS`. Values follow vmangos' `PetitionSigns`.
pub mod result {
    pub const OK: u32 = 0;
    pub const ALREADY_SIGNED: u32 = 1;
    pub const ALREADY_IN_GUILD: u32 = 2;
    pub const CANT_SIGN_OWN: u32 = 3;
    pub const NEED_MORE: u32 = 4;
    pub const NOT_SERVER: u32 = 5;
}

/// `SMSG_PETITION_SIGN_RESULTS` (449): the answer to a signature. The server
/// sends it to the signer, and to the charter's owner when the signature was
/// taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignResult {
    pub item: u64,
    /// The player who signed.
    pub player: u64,
    /// One of [`result`].
    pub result: u32,
}

/// `SMSG_PETITION_SIGN_RESULTS` (449).
pub fn parse_sign_result(body: &[u8]) -> Option<SignResult> {
    let mut r = Reader::new(body);
    if !r.has(20) {
        return None;
    }
    Some(SignResult {
        item: r.u64(),
        player: r.u64(),
        result: r.u32(),
    })
}

/// `SMSG_TURN_IN_PETITION_RESULTS` (453): one of [`result`].
pub fn parse_turn_in_result(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(4).then(|| r.u32())
}

/// `MSG_PETITION_DECLINE` (450), from the server: the guid of the player who
/// declined to sign.
pub fn parse_decline(body: &[u8]) -> Option<u64> {
    let mut r = Reader::new(body);
    r.has(8).then(|| r.u64())
}

/// `MSG_PETITION_RENAME` (705), from the server: the charter and its new
/// guild name.
pub fn parse_rename(body: &[u8]) -> Option<(u64, String)> {
    let mut r = Reader::new(body);
    if !r.has(9) {
        return None;
    }
    Some((r.u64(), r.cstring()))
}

/// A body that is one guid: the registrar's or the charter's.
pub fn guid_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// `CMSG_PETITION_BUY` (445). `index` is the offer's
/// [`CharterOffer::index`], and it is the last field. The fields between the
/// name and the index are an empty body text and the record's limits and
/// restrictions, all zero.
pub fn buy_body(npc: u64, name: &str, index: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(npc).u32(0).u64(0).cstring(name).cstring("");
    for _ in 0..8 {
        w.u32(0);
    }
    w.u16(0).u32(0).u32(0).u32(0).u32(index);
    w.buf
}

/// `CMSG_PETITION_SIGN` (448). The 1.12.1 client sends 1 in the trailing
/// byte; the server reads the byte and ignores it.
pub fn sign_body(item: u64, byte: u8) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(item).u8(byte);
    w.buf
}

/// `CMSG_OFFER_PETITION` (451): ask `player` to sign the charter.
pub fn offer_body(item: u64, player: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(item).u64(player);
    w.buf
}

/// `CMSG_PETITION_QUERY` (454).
pub fn query_body(petition: u32, item: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(petition).u64(item);
    w.buf
}

/// `MSG_PETITION_RENAME` (705), to the server.
pub fn rename_body(item: u64, name: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(item).cstring(name);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each signature is twelve bytes: a guid and a word of zero. A reader
    /// that takes eight reads the second signer from the middle of the first.
    #[test]
    fn a_signature_is_a_guid_and_four_bytes() {
        let mut w = Writer::new();
        w.u64(0x4000_0000_0000_0007).u64(0x10).u32(42).u8(2);
        w.u64(0x11).u32(0).u64(0x12).u32(0);
        let shown = parse_signatures(&w.buf).expect("parses");
        assert_eq!(shown.item, 0x4000_0000_0000_0007);
        assert_eq!((shown.owner, shown.petition), (0x10, 42));
        assert_eq!(shown.signers, [0x11, 0x12]);
    }

    /// A charter nobody has signed is the 21 fixed bytes and no rows.
    #[test]
    fn an_unsigned_charter_has_no_signers() {
        let mut w = Writer::new();
        w.u64(7).u64(0x10).u32(42).u8(0);
        assert_eq!(parse_signatures(&w.buf).expect("parses").signers, [] as [u64; 0]);
        assert!(parse_signatures(&w.buf[..20]).is_none());
    }

    /// The query answer is read up to the signature counts, and the 31 bytes
    /// after them are not needed.
    #[test]
    fn the_query_answer_names_the_guild_and_the_counts() {
        let mut w = Writer::new();
        w.u32(42).u64(0x10).cstring("The Watch").cstring("");
        w.u32(1).u32(9).u32(9);
        // Deadline, creation, guild, classes, races, gender, two levels, the
        // choice count and the default choice.
        w.u32(0).u32(0).u32(0).u32(0).u32(0).u16(0).u32(0).u32(0).u32(0).u32(0);
        let query = parse_query(&w.buf).expect("parses");
        assert_eq!((query.petition, query.owner), (42, 0x10));
        assert_eq!(query.name, "The Watch");
        assert_eq!((query.min_signatures, query.max_signatures), (9, 9));
    }

    #[test]
    fn the_show_list_reads_each_offer() {
        let mut w = Writer::new();
        w.u64(0x99).u8(1).u32(1).u32(5863).u32(16161).u32(1000).u32(1);
        let list = parse_show_list(&w.buf).expect("parses");
        assert_eq!(list.npc, 0x99);
        assert_eq!(
            list.offers,
            [CharterOffer {
                index: 1,
                item: 5863,
                display: 16161,
                cost: 1000,
                flags: 1
            }]
        );
    }

    /// The name sits after twenty bytes, the server skips 51 bytes after it,
    /// and the index is the last word.
    #[test]
    fn the_buy_body_puts_the_name_after_twenty_bytes_and_the_index_last() {
        let body = buy_body(0x99, "The Watch", 1);
        assert_eq!(body.len(), 8 + 4 + 8 + 10 + 51);
        let mut r = Reader::new(&body);
        assert_eq!(r.u64(), 0x99);
        r.skip(12);
        assert_eq!(r.cstring(), "The Watch");
        r.skip(47);
        assert_eq!(r.u32(), 1);
    }

    #[test]
    fn the_small_bodies_are_their_fields_in_order() {
        assert_eq!(sign_body(7, 1), [7, 0, 0, 0, 0, 0, 0, 0, 1]);
        let offer = offer_body(7, 0x11);
        let mut r = Reader::new(&offer);
        assert_eq!((r.u64(), r.u64()), (7, 0x11));
        let query = query_body(42, 7);
        let mut r = Reader::new(&query);
        assert_eq!((r.u32(), r.u64()), (42, 7));
        let mut w = Writer::new();
        w.u64(7).cstring("New Name");
        assert_eq!(parse_rename(&w.buf), Some((7, "New Name".to_string())));
        assert_eq!(rename_body(7, "New Name"), w.buf);
        let mut w = Writer::new();
        w.u64(7).u64(0x11).u32(result::ALREADY_SIGNED);
        assert_eq!(
            parse_sign_result(&w.buf),
            Some(SignResult {
                item: 7,
                player: 0x11,
                result: 1
            })
        );
    }
}
