//! The packets that must be answered.
//!
//! One `pub(super) fn` per arm of [`super::apply_packet`]'s match. **This is the
//! family the old two-site dispatch existed for**, and the one whose failures do
//! not look like failures: an unanswered forced change stays pending until
//! `OnFailedToAckChange` fires, and an ack sent with the wrong contents is a
//! kick. Nothing here reports an error — the server simply stops believing us.
//!
//! A handler answers by pushing onto [`super::Replies`], never by writing to
//! the socket: these run with the world lock held, and a write under it blocks
//! every reader on network latency.

use super::{read, Incoming};
use crate::state::movement;
use crate::opcodes::Opcode;
use crate::socket::world::Packet;

/// `SMSG_PONG`: the round trip we started, ended.
pub(super) fn pong(ctx: &mut Incoming) {
    if let Some(local) = ctx.local.as_deref_mut() {
        if let Some(sent) = local.ping_sent_at.take() {
            local.latency_ms = sent.elapsed().as_millis().min(u32::MAX as u128) as u32;
        }
    }
}

/// **The server has handed control of a body to this client, or taken it
/// back** — `SMSG_CLIENT_CONTROL_UPDATE`.
///
/// **All it does is set a flag on the unit it names**, and that is the
/// reference's own shape rather than a simplification: the client reads the
/// packed guid and the byte, looks the unit up in the object manager, and sets
/// a control bit on that unit. It changes the mover only
/// when the packet is about the local player itself, and even then it is the
/// derived rule below that decides.
///
/// **Nothing is answered here.** `CMSG_SET_ACTIVE_MOVER` goes out from
/// [`crate::socket::session::SessionLoop::tick_view`], which derives the mover
/// from `PLAYER_FARSIGHT` and this flag every tick the way the client does.
/// That is not tidiness: this packet arrives **three times** around one
/// possess, and one of the three would leave a client that latched on it stuck.
/// `Unit::ModPossess`'s teardown reads
///
/// ```text
/// pCaster->SetMover(nullptr);
/// pCaster->UpdateControl();                    // (the player, 1)
/// pCaster->SetClientControl(pTarget, false);   // (the possessed unit, 0)
/// ```
///
/// — so the **last** packet of a release names the body you no longer control,
/// with the byte clear. A latch that took the guid and ignored the byte held
/// the possessed unit as the mover for the rest of the session, which is the
/// "possess breaks player movement and requires a relog" report exactly.
pub(super) fn client_control(ctx: &mut Incoming, pkt: &Packet) {
    let Some(control) = read(ctx.stats, pkt, movement::parse_client_control(&pkt.body)) else {
        return;
    };
    if let Some(entity) = ctx.world.get_mut(control.guid) {
        entity.client_controlled = control.allow_move;
    }
}

/// **The server has moved us within this map.** A GM summoning the character, a
/// spell, a hearthstone.
///
/// Two things have to happen and neither is optional: the simulation jumps to
/// where the server put us, and the counter goes back.
///
/// Ignoring it is not a cosmetic bug. The mover keeps dead-reckoning from the
/// old position and every packet after that reports a place more than a yard
/// from the server's *with no movement flags set*, which is precisely
/// `MovementAnticheat::CheckTeleport`. The account this was found on was banned
/// for 24 hours, nine detections deep, for standing still in Gadgetzan.
pub(super) fn teleport(ctx: &mut Incoming, pkt: &Packet) {
    let Some(tp) = read(ctx.stats, pkt, movement::parse_teleport(&pkt.body)) else {
        return;
    };
    if ctx.world.player_guid == Some(tp.guid) {
        if let Some(entity) = ctx.world.get_mut(tp.guid) {
            entity.position = Some(tp.info.position);
        }
        if let Some(local) = ctx.local.as_deref_mut() {
            // **Any ride in flight is abandoned rather than acknowledged.**
            // `HandleMoveSplineDoneOpcode` returns early while
            // `IsBeingTeleported()`, so the relocation *is* the hand-back — and
            // riding on would walk the character back off the point the server
            // has just put them on. See [`movement::Mover::abandon_ride`].
            local.mover.abandon_ride();
            // **…and any deck under the feet, for the same reason one axis
            // over.** The carry moves the character by however far the platform
            // has moved since the offset was taken, and a relocation makes the
            // two readings incomparable — a boat crossing to the other
            // continent is a same-map relocation on the Grom'Gol run. See
            // [`movement::Mover::disembark`]; the next tick re-boards from
            // wherever the server has put us, and moves nobody doing it.
            local.mover.disembark();
            local.mover.resync(tp.info.position);
            local.relocations = local.relocations.wrapping_add(1);
        }
    }
    let time = ctx.local.as_deref_mut().map_or(0, |l| l.movement_info().time);
    ctx.replies.push(
        Opcode::MSG_MOVE_TELEPORT_ACK,
        movement::teleport_ack_body(tp.guid, tp.counter, time),
    );
}

/// **The server has moved us to another map.**
///
/// The same shape as [`teleport`] with a worse consequence: `TeleportTo` raises
/// `SetSemaphoreTeleportFar` *before* sending this and only the ack lowers it,
/// so until the client answers the player belongs to no map at all — nothing it
/// sends is processed and nothing comes back. Ignoring this does not mis-report
/// a position, it ends the session in place.
///
/// Three things happen and the order matters. The old map's objects go first (no
/// destroy block will ever arrive for them — the player was removed from the map
/// rather than the objects destroyed one by one), the simulation adopts the
/// position so the first heartbeat after arrival is not a teleport hack, and
/// only then does the ack go out, because the server starts the new map's burst
/// the moment it reads it.
pub(super) fn new_world(ctx: &mut Incoming, pkt: &Packet) {
    let Some(nw) = read(ctx.stats, pkt, movement::parse_new_world(&pkt.body)) else {
        return;
    };
    // **Read before the world is emptied, because it decides what survives
    // it.** `Map::SendInitTransports` skips the transport the player is
    // standing on, so a deck dropped here is a deck nothing on the far side
    // ever states again. The flag is read rather than taken; the take is below,
    // where the offset is dealt with.
    let riding = ctx
        .local
        .as_deref()
        .filter(|local| local.transfer_on_transport)
        .and_then(|local| local.mover.ferry())
        .map(|ferry| ferry.guid);
    ctx.world.leave_map(riding);
    // **The deck is either taken with us or left behind, and only
    // `SMSG_TRANSFER_PENDING` says which.** Its two extra dwords are written by
    // `ExecuteTeleportFar` for a character standing on a transport and by
    // nothing else; this packet carries no such mark. And the two cases do not
    // even agree about what the four floats in this packet *are*:
    // `Player::SendNewWorld` writes the **transport offset** for a passenger
    // (`m_movementInfo.GetTransportPos()`) and the world destination only for
    // everyone else. So the passenger case must not adopt them as a position —
    // that is the map origin — and must not let anything derive an offset from
    // a position until the kept deck is placed on the new map. See
    // `Mover::transfer_aboard`, which is the whole of that rule.
    //
    // Taken rather than read: a transfer is one event and a stale flag would
    // rebase the next ordinary teleport onto a boat.
    let aboard = ctx
        .local
        .as_deref_mut()
        .is_some_and(|local| std::mem::take(&mut local.transfer_on_transport));
    if !aboard {
        if let Some(guid) = ctx.world.player_guid {
            if let Some(entity) = ctx.world.get_mut(guid) {
                entity.position = Some(nw.position);
            }
        }
    }
    if let Some(local) = ctx.local.as_deref_mut() {
        local.map_id = nw.map_id;
        // …and the same for a ride, one map further: see [`teleport`].
        local.mover.abandon_ride();
        if aboard {
            local.mover.transfer_aboard(nw.position);
        } else {
            local.mover.resync(nw.position);
            local.mover.disembark();
        }
        local.relocations = local.relocations.wrapping_add(1);
    }
    ctx.replies.push(Opcode::MSG_MOVE_WORLDPORT_ACK, Vec::new());
}

/// **The server has thrown us.**
///
/// Not answering costs the same as not answering a root — the change stays
/// pending, `OnFailedToAckChange` fires at four seconds, and the client
/// meanwhile keeps reporting a position the server is no longer expecting.
pub(super) fn knock_back(ctx: &mut Incoming, pkt: &Packet) {
    let Some(kb) = read(ctx.stats, pkt, movement::parse_knock_back(&pkt.body)) else {
        return;
    };
    ctx.stats.flag_changes += 1;
    let is_us = ctx.world.player_guid == Some(kb.guid);
    let Some(local) = ctx.local.as_deref_mut() else {
        return;
    };
    // **Unlike the flag changes, this ack is checked against its own data**, so
    // it can only be answered for a unit whose arc we are actually simulating.
    // Answering for one we are not would be `OnWrongAckData` — three of which is
    // a kick — where staying quiet is merely the pending-ack delay we would have
    // had anyway.
    if !is_us {
        return;
    }
    local
        .mover
        .knock_back(kb.cos_angle, kb.sin_angle, kb.xy_speed, kb.z_speed);
    // The four floats ride back inside the movement block's jump section, which
    // `knock_back` has just filled in — they are compared to within 0.01 and a
    // mismatch is as good as silence.
    let info = local.movement_info();
    ctx.replies.push(
        Opcode::CMSG_MOVE_KNOCK_BACK_ACK,
        movement::knock_back_ack_body(kb.guid, kb.counter, &info),
    );
}

/// `SMSG_FORCE_*_SPEED_CHANGE`: record it, then acknowledge it.
///
/// **The record is about whoever the packet names, not only about us**, and that
/// is the half the old two-site dispatch dropped. `Entity::speeds` is otherwise
/// written only by an update block's movement section, so a hasted or slowed
/// player nearby kept their old speed and was dead-reckoned at it — visible as
/// them drifting away and being snapped back, with nothing logged anywhere.
pub(super) fn speed_change(ctx: &mut Incoming, pkt: &Packet, op: Opcode) {
    let Some((slot, ack)) = movement::speed_change_slot(op) else {
        return;
    };
    let Some(change) = read(ctx.stats, pkt, movement::parse_speed_change(&pkt.body)) else {
        return;
    };
    ctx.stats.speed_changes += 1;

    if let Some(entity) = ctx.world.get_mut(change.guid) {
        let mut speeds = entity.speeds.unwrap_or_default();
        speeds.0[slot] = change.speed;
        entity.speeds = Some(speeds);
    }

    // The local simulation, and the acknowledgement, both of which need a
    // session to belong to. A snapshot pump has neither and has still recorded
    // the change above, which is all it could have used it for.
    let is_us = ctx.world.player_guid == Some(change.guid);
    if let Some(local) = ctx.local.as_deref_mut() {
        if is_us {
            local.mover.speeds.0[slot] = change.speed;
        }
        let info = local.movement_info();
        ctx.replies.push(
            ack,
            movement::speed_change_ack_body(change.guid, change.counter, &info, change.speed),
        );
    }
}

/// One of the six forced flag changes: apply it, then acknowledge it.
///
/// **The acknowledgement is the whole point, and it has one hard requirement.**
/// `HandleMoveRootAck` kicks outright if the movement block that comes back with
/// a root *apply* does not carry `MOVEFLAG_ROOT` — "Client sent root apply ack,
/// but movement info does not have rooted movement flag!" — so the flag is set
/// on the mover before the block is stamped, not after. The other three are
/// laxer, and are set the same way for one shape.
///
/// A change about **another** unit is recorded on that unit and not answered:
/// the ack is the controller's business, and we control one character.
pub(super) fn flag_change(ctx: &mut Incoming, pkt: &Packet, op: Opcode) {
    let Some(change) = movement::FlagChange::of(op) else {
        return;
    };
    let Some((guid, counter)) = read(ctx.stats, pkt, movement::parse_flag_change(&pkt.body))
    else {
        return;
    };
    ctx.stats.flag_changes += 1;

    if let Some(entity) = ctx.world.get_mut(guid) {
        let mut info = entity.movement.unwrap_or_default();
        if change.apply {
            info.flags |= change.flag;
        } else {
            info.flags &= !change.flag;
        }
        entity.movement = Some(info);
    }

    let is_us = ctx.world.player_guid == Some(guid);
    let Some(local) = ctx.local.as_deref_mut() else {
        return;
    };
    if is_us {
        if change.flag == movement::move_flags::ROOT {
            local.mover.set_rooted(change.apply);
        } else if change.apply {
            local.mover.info.flags |= change.flag;
        } else {
            local.mover.info.flags &= !change.flag;
        }
    }
    let info = local.movement_info();
    let apply = change.apply_field().then_some(change.apply);
    ctx.replies.push(
        change.ack,
        movement::flag_change_ack_body(guid, counter, &info, apply),
    );
}
