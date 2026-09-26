//! **Flight paths** — the flight master's map, and the ride it buys.
//!
//! ```text
//! CMSG_TAXINODE_STATUS_QUERY  guid   is this master's node one I know?
//!   -> SMSG_TAXINODE_STATUS          guid + one byte
//! CMSG_TAXIQUERYAVAILABLENODES guid  right-click a flight master
//!   -> SMSG_NEW_TAXI_PATH            …a *discovery*, with no body at all
//!   or SMSG_SHOWTAXINODES            the map: 1, guid, current node, 8 mask words
//! CMSG_ACTIVATETAXI        guid, from, to          press a node with a direct path
//! CMSG_ACTIVATETAXIEXPRESS guid, cost, n, nodes[]  …or one that needs hops
//!   -> SMSG_ACTIVATETAXIREPLY        one u32: 0 is yes, 1..12 are why not
//!   and then SMSG_MONSTER_MOVE       which is the whole of the flight
//! ```
//!
//! **There is no close opcode and no arrival packet**, which is the same shape
//! the gossip and trainer windows have at one end and something new at the
//! other. `CloseTaxiMap()` clears the module's guid locally; and
//! the flight itself is not a taxi packet at all.
//!
//! ## The flight is `SMSG_MONSTER_MOVE`, and this client already rides it
//!
//! `FlightPathMovementGenerator::Reset` builds an ordinary `MoveSplineInit`
//! with `SetFly()` over the path's waypoints at `PLAYER_FLIGHT_SPEED` (32 y/s)
//! and `Launch`es it. So the packet that flies you to Ironforge is the same one
//! that walks a wolf across Elwynn, addressed to your own guid — the machinery
//! [`crate::state::movement::Mover::ride`] was written for when Charge turned out to be
//! the same thing.
//!
//! Two consequences worth stating, because both are why nothing new is needed
//! here:
//!
//! * **`SetFly()` is `MoveSplineFlag::Flying`, which is `Mask_CatmullRom`** — so
//!   the path arrives as absolute points rather than packed offsets, and
//!   [`crate::state::movement::parse_monster_move`] already branches on that flag.
//! * **A ride outranks gravity and the terrain**
//!   ([`crate::state::movement::Mover::advance`] returns the moment `ride_step` takes
//!   one), so a character 60 yards up follows the spline instead of falling out
//!   of the sky.
//!
//! What the *server* does around it is state this client only reads:
//! `UNIT_FLAG_TAXI_FLIGHT` and `UNIT_FLAG_REMOVE_CLIENT_CONTROL` on
//! `UNIT_FIELD_FLAGS`, a mount on `UNIT_FIELD_MOUNTDISPLAYID`, and the money
//! coming off `PLAYER_FIELD_COINAGE`. **The client never deducts anything** —
//! `ActivateTaxiPathTo` charges the first leg before it replies and
//! `FlightPathMovementGenerator::Update` charges each further one as the flight
//! crosses into it — so the number in the tooltip is a quote and the field is
//! the truth.
//!
//! ## The reply is a u32 and its twelve refusals each have a sentence
//!
//! `ERR_TAXI*` are keys in `GlobalStrings.lua` and the client's own error table
//! carries all thirteen at indices 170..182.
//!
//! **The code-to-key mapping here is by name, and that is a reading**: the
//! client's own handling of `SMSG_ACTIVATETAXIREPLY` is not established, so the
//! numbering is vmangos' `ActivateTaxiReply` enum — whose names are these
//! strings — rather than something measured. The one pair a rename could cross
//! is 1 and 2, which the client's own table happens to order the other way
//! round.

use crate::bytes::{Reader, Writer};

/// `TaxiMaskSize` — eight `u32`s, 256 bits, one per node, one-based.
pub const TAXI_MASK_WORDS: usize = 8;

/// `SMSG_SHOWTAXINODES` — the whole flight map, as far as the wire is
/// concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaxiMenu {
    /// The flight master, which every verb from this window echoes back.
    pub guid: u64,
    /// **The node the master is standing at**, which the server resolves by
    /// proximity (`GetNearestTaxiNode`) rather than the client. It is the
    /// source of every route and the green icon.
    pub current: u32,
    /// Which nodes this character has visited. See `vale_assets::tables::taxi`'s
    /// `TaxiMask`, which is where the bit arithmetic and everything downstream
    /// of it lives — this crate carries the words and no opinion about them.
    pub mask: [u32; TAXI_MASK_WORDS],
}

/// `SMSG_ACTIVATETAXIREPLY`'s `u32` — vmangos' `ActivateTaxiReply`.
///
/// **`Ok` is a value and not an absence.** The server sends it immediately
/// before `SendDoFlight`, so it is the "the flight is happening" edge and the
/// only one there is: nothing else announces a departure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxiReply {
    Ok,
    UnspecifiedServerError,
    NoSuchPath,
    NotEnoughMoney,
    TooFarAway,
    NoVendorNearby,
    NotVisited,
    PlayerBusy,
    PlayerAlreadyMounted,
    PlayerShapeshifted,
    PlayerMoving,
    SameNode,
    NotStanding,
}

impl TaxiReply {
    pub fn of(code: u32) -> Option<TaxiReply> {
        Some(match code {
            0 => TaxiReply::Ok,
            1 => TaxiReply::UnspecifiedServerError,
            2 => TaxiReply::NoSuchPath,
            3 => TaxiReply::NotEnoughMoney,
            4 => TaxiReply::TooFarAway,
            5 => TaxiReply::NoVendorNearby,
            6 => TaxiReply::NotVisited,
            7 => TaxiReply::PlayerBusy,
            8 => TaxiReply::PlayerAlreadyMounted,
            9 => TaxiReply::PlayerShapeshifted,
            10 => TaxiReply::PlayerMoving,
            11 => TaxiReply::SameNode,
            12 => TaxiReply::NotStanding,
            _ => return None,
        })
    }

    /// The `GlobalStrings.lua` key for what to show, or `None` for the one
    /// answer that is not a refusal.
    pub fn key(self) -> Option<&'static str> {
        Some(match self {
            TaxiReply::Ok => return None,
            TaxiReply::UnspecifiedServerError => "ERR_TAXIUNSPECIFIEDSERVERERROR",
            TaxiReply::NoSuchPath => "ERR_TAXINOSUCHPATH",
            TaxiReply::NotEnoughMoney => "ERR_TAXINOTENOUGHMONEY",
            TaxiReply::TooFarAway => "ERR_TAXITOOFARAWAY",
            TaxiReply::NoVendorNearby => "ERR_TAXINOVENDORNEARBY",
            TaxiReply::NotVisited => "ERR_TAXINOTVISITED",
            TaxiReply::PlayerBusy => "ERR_TAXIPLAYERBUSY",
            TaxiReply::PlayerAlreadyMounted => "ERR_TAXIPLAYERALREADYMOUNTED",
            TaxiReply::PlayerShapeshifted => "ERR_TAXIPLAYERSHAPESHIFTED",
            TaxiReply::PlayerMoving => "ERR_TAXIPLAYERMOVING",
            TaxiReply::SameNode => "ERR_TAXISAMENODE",
            TaxiReply::NotStanding => "ERR_TAXINOTSTANDING",
        })
    }
}

// --- parsing ----------------------------------------------------------------

/// `SMSG_SHOWTAXINODES`.
///
/// **The leading word is a constant 1** and not a count — `SendTaxiMenu` writes
/// `uint32(1)` before the guid and nothing reads it back. It is skipped here
/// rather than named, because naming it would invite somebody to loop on it.
pub fn parse_show_taxi_nodes(body: &[u8]) -> Option<TaxiMenu> {
    let mut r = Reader::new(body);
    if !r.has(4 + 8 + 4 + 4 * TAXI_MASK_WORDS) {
        return None;
    }
    let _unread = r.u32();
    let guid = r.u64();
    let current = r.u32();
    let mut mask = [0u32; TAXI_MASK_WORDS];
    for word in &mut mask {
        *word = r.u32();
    }
    Some(TaxiMenu {
        guid,
        current,
        mask,
    })
}

/// `SMSG_TAXINODE_STATUS` — `(guid, is this master's node known?)`.
///
/// Sent both in answer to the query and unprompted, as the second half of a
/// discovery: `SendLearnNewTaxiNode` writes `SMSG_NEW_TAXI_PATH` and then this,
/// with the byte already 1.
pub fn parse_taxi_node_status(body: &[u8]) -> Option<(u64, bool)> {
    let mut r = Reader::new(body);
    if !r.has(8 + 1) {
        return None;
    }
    Some((r.u64(), r.u8() != 0))
}

/// `SMSG_ACTIVATETAXIREPLY` — one `u32`.
pub fn parse_activate_taxi_reply(body: &[u8]) -> Option<TaxiReply> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    TaxiReply::of(r.u32())
}

// --- what we send -----------------------------------------------------------

/// `CMSG_ACTIVATETAXI` — the master and **two node ids**, the source first.
///
/// The source is the map's current node rather than anything the player picked:
/// the client sends the map's own `curloc` beside the pressed button's node.
pub fn activate_taxi_body(guid: u64, from: u32, to: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.u32(from);
    w.u32(to);
    w.buf
}

/// `CMSG_ACTIVATETAXIEXPRESS` — the master, **the total cost**, a count and the
/// whole route including its source.
///
/// The cost is read and thrown away by vmangos (`_totalcost`), which is worth
/// knowing and not worth acting on: it is what the reference sends
/// (the accumulated cost) and a server that checked it would
/// refuse a body that lied.
pub fn activate_taxi_express_body(guid: u64, cost: u32, nodes: &[u32]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.u32(cost);
    w.u32(nodes.len() as u32);
    for node in nodes {
        w.u32(*node);
    }
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_carries_a_master_a_node_and_eight_mask_words() {
        let mut w = Writer::new();
        w.u32(1);
        w.u64(0xdead_beef);
        w.u32(6);
        for word in 0..TAXI_MASK_WORDS as u32 {
            w.u32(word + 1);
        }
        let menu = parse_show_taxi_nodes(&w.buf).expect("a menu");
        assert_eq!(menu.guid, 0xdead_beef);
        assert_eq!(menu.current, 6);
        assert_eq!(menu.mask, [1, 2, 3, 4, 5, 6, 7, 8]);
        // **A short mask is not a menu**: reading four words and zeroing the
        // rest would hide every node past 128 with no sign of it.
        assert_eq!(parse_show_taxi_nodes(&w.buf[..w.buf.len() - 1]), None);
    }

    #[test]
    fn the_status_byte_is_a_flag_and_the_reply_is_a_word() {
        let mut w = Writer::new();
        w.u64(9);
        w.u8(1);
        assert_eq!(parse_taxi_node_status(&w.buf), Some((9, true)));
        let mut w = Writer::new();
        w.u32(3);
        assert_eq!(
            parse_activate_taxi_reply(&w.buf),
            Some(TaxiReply::NotEnoughMoney)
        );
        let mut w = Writer::new();
        w.u32(0);
        assert_eq!(parse_activate_taxi_reply(&w.buf), Some(TaxiReply::Ok));
        assert_eq!(TaxiReply::Ok.key(), None, "success says nothing");
        assert_eq!(TaxiReply::of(13), None, "the numbering stops at twelve");
    }

    /// **Every refusal has a sentence**, which is the check that matters here:
    /// a code with no key displays as nothing at all and reads as the client
    /// ignoring the press.
    #[test]
    fn every_refusal_names_a_globalstrings_key() {
        for code in 1..=12 {
            let reply = TaxiReply::of(code).expect("a reply");
            let key = reply.key().expect("a key");
            assert!(key.starts_with("ERR_TAXI"), "{code} -> {key}");
        }
    }

    #[test]
    fn the_two_verbs_write_the_route_the_server_reads() {
        let body = activate_taxi_body(0x77, 2, 6);
        assert_eq!(&body[0..8], &0x77u64.to_le_bytes());
        assert_eq!(&body[8..12], &2u32.to_le_bytes());
        assert_eq!(&body[12..16], &6u32.to_le_bytes());
        let body = activate_taxi_express_body(0x77, 500, &[2, 4, 6]);
        assert_eq!(&body[8..12], &500u32.to_le_bytes());
        assert_eq!(&body[12..16], &3u32.to_le_bytes(), "the count");
        assert_eq!(&body[16..20], &2u32.to_le_bytes(), "the source is in it");
        assert_eq!(body.len(), 8 + 4 + 4 + 12);
    }
}
