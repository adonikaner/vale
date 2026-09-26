//! What things are called: the four `*_QUERY_RESPONSE` packets.
//!
//! Each answer's body is also handed to `ObjectManager::remember` for the
//! on-disk cache — see [`crate::play::wdb`].
//!
//! One `pub(super) fn` per arm of [`super::apply_packet`]'s match. These are the
//! answers to the questions `crate::state::query` asks — same subject, one directory
//! apart: `crate::state::query` builds the outbound bodies, this folds in the replies.

use super::{read, Incoming};
use crate::play::wdb::Kind;
use crate::state::query;
use crate::socket::world::Packet;

/// `SMSG_CREATURE_QUERY_RESPONSE`: a creature template, keyed by entry.
pub(super) fn creature(ctx: &mut Incoming, pkt: &Packet) {
    let Some(info) = read(ctx.stats, pkt, query::parse_creature_response(&pkt.body)) else {
        return;
    };
    ctx.stats.creatures_resolved += 1;
    let entry = info.entry;
    ctx.world.remember(Kind::Creature, u64::from(entry), &pkt.body);
    ctx.world.creatures.insert(entry, info);
    // …and the edge, which is what a quest log holding "  : 3/8" waits on.
    ctx.world.note_creature_arrival(entry);
}

/// `SMSG_NAME_QUERY_RESPONSE`: a player's name, keyed by guid rather than entry
/// — players have no template.
pub(super) fn name(ctx: &mut Incoming, pkt: &Packet) {
    let Some(info) = read(ctx.stats, pkt, query::parse_name_response(&pkt.body)) else {
        return;
    };
    ctx.stats.players_resolved += 1;
    ctx.world.remember(Kind::Name, info.guid, &pkt.body);
    ctx.world.players.insert(info.guid, info);
}

/// `SMSG_GAMEOBJECT_QUERY_RESPONSE`: a chest, a door, a campfire.
pub(super) fn gameobject(ctx: &mut Incoming, pkt: &Packet) {
    let Some(info) = read(ctx.stats, pkt, query::parse_gameobject_response(&pkt.body)) else {
        return;
    };
    ctx.stats.gameobjects_resolved += 1;
    let entry = info.entry;
    ctx.world.remember(Kind::GameObject, u64::from(entry), &pkt.body);
    ctx.world.gameobjects.insert(entry, info);
    ctx.world.note_gameobject_arrival(entry);
}

/// What a piece of a player's equipment looks like.
///
/// **A refusal is not a parse failure** — it is the entry with the high bit set
/// and nothing after it — and it has to be *recorded*, or vmangos' `Discovered`
/// gate means the same entry is asked for again every query interval for the
/// rest of the session.
pub(super) fn item(ctx: &mut Incoming, pkt: &Packet) {
    match query::parse_item_response(&pkt.body) {
        Some(info) => {
            ctx.stats.items_resolved += 1;
            let entry = info.entry;
            // The body is kept as well as the record, for the on-disk cache —
            // see [`crate::play::wdb`]. `remember` skips a template already
            // known, so a seeded entry the server restates is not written
            // again.
            ctx.world.remember(Kind::Item, u64::from(entry), &pkt.body);
            ctx.world.items.insert(entry, info);
            // **…and the edge**, which is the only thing that can tell a panel
            // holding a blank name to read it again. See
            // `ObjectManager::note_item_arrival`.
            ctx.world.note_item_arrival(entry);
        }
        None if pkt.body.len() >= 4 => {
            let entry = crate::bytes::Reader::new(&pkt.body).u32() & !0x8000_0000;
            ctx.world.unknown_items.insert(entry);
            // A refusal is an answer: without it a row waits for ever.
            ctx.world.note_item_arrival(entry);
        }
        None => ctx.stats.unreadable(pkt),
    }
}
