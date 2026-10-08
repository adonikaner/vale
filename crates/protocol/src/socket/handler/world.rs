//! What is in the world and what it is doing: object updates, movement, and the
//! packets that drive an animation.
//!
//! One `pub(super) fn` per arm of [`super::apply_packet`]'s match — see that
//! module's comment for why the dispatch stays whole and only the bodies live
//! out here.
//!
//! This `world` module is a child of [`super`], not `crate::socket::world`. That
//! module is the socket; this one applies the world state the socket's packets
//! describe. The two are one directory apart and their paths never mix.

use super::{read, Incoming};
use crate::play::action;
use crate::state::movement;
use crate::opcodes::Opcode;
use crate::play::combatlog::{self, CombatEvent};
use crate::play::sound;
use crate::play::spells::PlayerEvent;
use crate::state::update;
use crate::socket::world::Packet;
use flate2::read::ZlibDecoder;
use std::io::{self, Read};

/// `SMSG_UPDATE_OBJECT`: the block that creates, moves and re-describes
/// everything.
pub(super) fn update(ctx: &mut Incoming, pkt: &Packet) {
    ctx.stats.updates += 1;
    let parsed = update::parse(&pkt.body);
    ctx.stats.note_warning(&parsed);
    ctx.world.apply(&parsed);
}

/// The same block, zlib'd. Counted separately because a decode failure and a
/// parse failure are different faults with the same symptom.
pub(super) fn compressed_update(ctx: &mut Incoming, pkt: &Packet) {
    ctx.stats.updates += 1;
    ctx.stats.compressed += 1;
    match inflate_update(&pkt.body) {
        Ok(raw) => {
            let parsed = update::parse(&raw);
            ctx.stats.note_warning(&parsed);
            ctx.world.apply(&parsed);
        }
        Err(e) => ctx.stats.warn(format!("inflate failed: {e}")),
    }
}

/// `SMSG_COMPRESSED_MOVES`: a bag of movement packets.
///
/// The server sends this only under load. `WorldSession::SendMovementPacket`
/// counts the movement packets it has sent this interval. While that count
/// stays under `CONFIG_UINT32_COMPRESSION_MOVEMENT_COUNT`, each one goes out on
/// its own opcode; past it, they are batched into this one instead. A quiet
/// session never produces it and a busy zone produces little else, so a client
/// that drops it works against a localhost server with one player and fails on
/// a populated realm.
///
/// All server movement passes through `SendMovementPacket`.
/// `MoveSplineInit::Launch` (`SMSG_MONSTER_MOVE`, every creature),
/// `MovementHandler`'s relays (`MSG_MOVE_*`, every other player) and
/// `MovementPacketSender` (speed changes, flag changes, knockbacks) all call
/// `WorldObject::SendMovementMessageToSet`, which is `SendMovementPacket`.
/// Dropping this opcode loses all three at once, with no warning and no parse
/// failure: the world stops moving and units jump whenever another packet
/// states a position. The reported symptom was "mobs appear to be standing far
/// away but are actually attacking you". Before this opcode was handled, one
/// session showed 428 of these packets and 35.7 KiB on the `F4` net tab's
/// unhandled list.
///
/// ## The layout, from `MovementData` in vmangos' `Objects/UpdateData.cpp`
///
/// ```text
/// u32 uncompressedSize
/// <zlib deflate of exactly that many bytes>, and inside it, repeated:
///     u8  size        // the body's length plus two, for the opcode
///     u16 opcode
///     ..  body        // size - 2 bytes
/// ```
///
/// `AddPacket` adds the 2. It is also why a sub-packet body may not exceed 253
/// bytes: `CanAddPacket` refuses anything that would overflow the `u8`, with
/// the comment "Client crash else".
///
/// ## Inner packets go back through `apply_packet`
///
/// Each inner packet is passed to [`crate::socket::handler::apply_packet`]
/// rather than to a second match over the movement opcodes; `handler`'s module
/// comment explains why there is one dispatch. An inner packet is an ordinary
/// packet that arrived in a bag, so it produces the same counters, warnings and
/// `traffic` rows as one that arrived alone. A nested bag is refused: the
/// server never puts one inside another (only movement packets reach the
/// compressor), and received data must not control recursion depth.
pub(super) fn compressed_moves(ctx: &mut Incoming, pkt: &Packet) {
    ctx.stats.bagged_moves = ctx.stats.bagged_moves.saturating_add(1);
    let raw = match inflate_update(&pkt.body) {
        Ok(raw) => raw,
        Err(e) => {
            ctx.stats.warn(format!("SMSG_COMPRESSED_MOVES: inflate failed: {e}"));
            return;
        }
    };
    let inner = unbag(ctx, &raw);
    ctx.stats.bagged_packets = ctx
        .stats
        .bagged_packets
        .saturating_add(inner.len() as u32);
    for packet in inner {
        super::apply_packet(ctx, &packet);
    }
}

/// Cut an inflated `MovementData` buffer into the packets it was built from.
///
/// The packets are collected rather than streamed, because the dispatch takes
/// `ctx` mutably and the walk needs `ctx` to report a damaged tail. The buffer
/// is a few kilobytes and the packets inside it are tens of bytes.
///
/// A truncated or invalid tail costs the rest of the bag and is reported,
/// on the same rule every other reader in this crate follows: the packets
/// already read are good and there is no way to resynchronise past a bad length
/// byte.
fn unbag(ctx: &mut Incoming, raw: &[u8]) -> Vec<Packet> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < raw.len() {
        let size = raw[at] as usize;
        // `AddPacket` writes `body + 2`, so anything under two is not a packet
        // and there is nothing to skip forward by.
        if size < 2 {
            ctx.stats
                .warn(format!("SMSG_COMPRESSED_MOVES: a {size}-byte block at {at}"));
            break;
        }
        if at + 1 + size > raw.len() {
            ctx.stats.warn(format!(
                "SMSG_COMPRESSED_MOVES: a block of {size} at {at} runs past the {} bytes inflated",
                raw.len()
            ));
            break;
        }
        let code = u16::from_le_bytes([raw[at + 1], raw[at + 2]]);
        let body = raw[at + 3..at + 1 + size].to_vec();
        at += 1 + size;
        // See the doc above: the server never puts a bag inside a bag, and
        // recursing on one would let a malformed stream overflow the stack.
        if u32::from(code) == Opcode::SMSG_COMPRESSED_MOVES.code() {
            ctx.stats
                .warn("SMSG_COMPRESSED_MOVES: a nested bag, which the server does not send".into());
            continue;
        }
        out.push(Packet { code: u32::from(code), body });
    }
    out
}

/// `SMSG_MONSTER_MOVE`: a server-driven unit along a spline.
///
/// The unit may be the local player. Warrior Charge moves the caster with the
/// same `MoveSpline` code that walks a patrolling creature, so the packet that
/// arrives is this one with the player's own guid in it. It must be answered
/// with `CMSG_MOVE_SPLINE_DONE`; without that, the server discards every
/// movement packet this session sends for the rest of the session. See
/// [`crate::state::movement::Ride`](crate::state::movement::Mover::ride), which
/// follows the spline and sends the answer.
///
/// The world's copy is applied either way: it holds the entity the renderer
/// draws, and a snapshot pump with no simulation of its own has nothing else.
pub(super) fn monster_move(ctx: &mut Incoming, pkt: &Packet) {
    monster_move_inner(ctx, pkt, false)
}

/// `SMSG_MONSTER_MOVE_TRANSPORT`: the same move, for a unit on a transport.
///
/// `MoveSplineInit::Launch` builds `SMSG_MONSTER_MOVE` and, for a unit that is
/// on a transport, changes the opcode and writes the transport's packed guid
/// before everything else. The two are one body with one extra guid. A client
/// that reads the second as the first takes that guid's bytes as the unit's
/// start coordinates, and the parse still succeeds on any path long enough for
/// the extra twelve bytes to be absorbed by the nodes.
///
/// Every waypoint after the guid is in the transport's own frame. Placed in the
/// world unconverted, a boat's crew walking three yards across the deck are
/// placed three yards from the map's origin. See [`movement::Spline::in_world`],
/// which does the conversion and handles the case where the transport is not
/// loaded.
pub(super) fn monster_move_transport(ctx: &mut Incoming, pkt: &Packet) {
    monster_move_inner(ctx, pkt, true)
}

fn monster_move_inner(ctx: &mut Incoming, pkt: &Packet, on_transport: bool) {
    let parsed = match on_transport {
        true => movement::parse_monster_move_transport(&pkt.body),
        false => movement::parse_monster_move(&pkt.body),
    };
    let Some(mm) = read(ctx.stats, pkt, parsed) else {
        return;
    };
    ctx.stats.monster_moves += 1;
    let is_us = ctx.world.player_guid == Some(mm.guid);
    // Resolved before the world is handed over, because only the object manager
    // can look up another unit. A charge is sent with `SetFacingGUID(target)`,
    // and the mover has no way to find that target.
    let facing_at = match mm.facing {
        movement::SplineFacing::Target(guid) => ctx
            .world
            .get(guid)
            .and_then(|e| e.position)
            .map(|p| [p.x, p.y, p.z]),
        _ => None,
    };
    ctx.world.apply_monster_move(&mm);
    if !is_us {
        return;
    }
    if let Some(local) = ctx.local.as_deref_mut() {
        local.mover.ride(&mm, facing_at);
        local.relocations = local.relocations.wrapping_add(1);
    }
}

/// `SMSG_ATTACKERSTATEUPDATE`: one melee swing.
///
/// The server never says "start attacking" and never names an animation, so
/// this packet is the only statement that a blow happened. Without it, a fight
/// shows two creatures in their idle animations while the health bars empty.
pub(super) fn attack(ctx: &mut Incoming, pkt: &Packet) {
    let Some(attack) = read(ctx.stats, pkt, action::parse_attack_update(&pkt.body)) else {
        return;
    };
    ctx.stats.attacks += 1;
    ctx.world.note_combat(CombatEvent::Swing(attack));
    ctx.world.apply_attack(&attack);
}

/// Spell damage on a unit, or a damage-over-time tick, which arrives on the
/// same opcode with `periodicLog` set.
///
/// `SMSG_ATTACKERSTATEUPDATE` covers weapon swings only. A client that reads
/// only that packet draws a number for every auto-attack and nothing for a
/// Fireball. This client did that before this handler existed, and it was
/// reported as "spells are not triggering it".
pub(super) fn spell_damage(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, action::parse_spell_damage(&pkt.body)) else {
        return;
    };
    ctx.stats.spell_logs += 1;
    ctx.world.note_combat(CombatEvent::SpellDamage(log));
    ctx.world.apply_spell_damage(&log);
}

/// `SMSG_SPELLHEALLOG`: a heal, drawn as a number over the target like spell
/// damage.
///
/// The server sends this packet only to builds after 1.9.4, which includes
/// 1.12. Older clients learned about a heal only from the health field
/// changing.
pub(super) fn spell_heal(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, action::parse_spell_heal(&pkt.body)) else {
        return;
    };
    ctx.stats.spell_logs += 1;
    ctx.world.note_combat(CombatEvent::SpellHeal(log));
    ctx.world.apply_spell_heal(&log);
}

/// `SMSG_LOG_XPGAIN`, the first of the thirteen combat-log packets whose only
/// consumer is a line of text. The functions from here to
/// [`periodic_aura_log`] handle the rest.
///
/// Each one is parsed and put on the combat log's queue for the renderer to
/// compose. There is no state to apply: nothing here moves a unit, changes a
/// field or answers the server. A client that drops all of them shows the same
/// fight with an empty combat log, which is why they went unread for a long
/// time.
pub(super) fn xp_gain(ctx: &mut Incoming, pkt: &Packet) {
    let Some(gain) = read(ctx.stats, pkt, combatlog::parse_xp_gain(&pkt.body)) else {
        return;
    };
    ctx.world.note_combat(CombatEvent::XpGain(gain));
}

pub(super) fn party_kill(ctx: &mut Incoming, pkt: &Packet) {
    let Some(kill) = read(ctx.stats, pkt, combatlog::parse_party_kill(&pkt.body)) else {
        return;
    };
    ctx.world.note_combat(CombatEvent::PartyKill(kill));
}

pub(super) fn environmental_damage(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(
        ctx.stats,
        pkt,
        combatlog::parse_environmental_damage(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_combat(CombatEvent::Environmental(log));
}

pub(super) fn damage_shield(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, combatlog::parse_damage_shield(&pkt.body)) else {
        return;
    };
    ctx.world.note_combat(CombatEvent::DamageShield(log));
}

/// `SMSG_SPELLLOGMISS`: one line per target.
///
/// An area spell resisted by four of six is one `SMSG_SPELLLOGMISS` and four
/// lines, so the fan-out happens here rather than at the composer: the queue
/// carries what the log holds, one entry per sentence.
pub(super) fn spell_miss_log(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, combatlog::parse_spell_miss_log(&pkt.body)) else {
        return;
    };
    for (target, miss) in log.targets {
        ctx.world.note_combat(CombatEvent::SpellMissed {
            spell_id: log.spell_id,
            caster: log.caster,
            target,
            miss,
        });
    }
}

/// `SMSG_SPELLLOGEXECUTE`: one line per effect entry, the same fan-out
/// [`spell_miss_log`] makes and for the same reason.
///
/// This is the one packet in the family that is a list of lists: an effect id,
/// a count, and that many entries whose width depends on the id. Four of its
/// nine effect types have a sentence: `SPELLEXTRAATTACKS`, `SPELLINTERRUPT`,
/// `FEEDPET_LOG` and `SPELLDURABILITYDAMAGE`. The others are already reported
/// by packets of their own: a heal by `SMSG_SPELLHEALLOG`, a drain and an
/// energize by `SMSG_PERIODICAURALOG` and `SMSG_SPELLENERGIZELOG`. Reporting
/// them here as well would double every line a Life Tap or a Drain Mana
/// produces.
pub(super) fn spell_execute_log(ctx: &mut Incoming, pkt: &Packet) {
    use crate::play::combatlog::ExecuteEntry;
    let Some(log) = read(ctx.stats, pkt, combatlog::parse_spell_execute_log(&pkt.body)) else {
        return;
    };
    for entry in log.entries {
        match entry {
            ExecuteEntry::ExtraAttacks { target, count } => {
                ctx.world.note_combat(CombatEvent::ExtraAttacks {
                    target,
                    spell_id: log.spell_id,
                    count,
                });
            }
            ExecuteEntry::InterruptCast { target, spell_id } => {
                ctx.world.note_combat(CombatEvent::Interrupt {
                    caster: log.caster,
                    target,
                    // The sentence names the spell that was stopped, not the
                    // one that stopped it.
                    spell_id,
                });
            }
            ExecuteEntry::FeedPet { item } => {
                ctx.world.note_combat(CombatEvent::FeedPet {
                    caster: log.caster,
                    item,
                });
            }
            ExecuteEntry::DurabilityDamage { target, item, .. } => {
                ctx.world.note_combat(CombatEvent::DurabilityDamage {
                    caster: log.caster,
                    target,
                    spell_id: log.spell_id,
                    item,
                });
            }
            // The five that already have a packet of their own, and the bare
            // guid every summon and resurrection reduces to.
            _ => {}
        }
    }
}

/// `SMSG_SPELLDISPELLOG`: one line per aura a dispel removed.
pub(super) fn spell_dispel_log(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, combatlog::parse_spell_dispel_log(&pkt.body)) else {
        return;
    };
    for spell_id in log.spells {
        ctx.world.note_combat(CombatEvent::Dispel {
            victim: log.victim,
            spell_id,
        });
    }
}

/// `SMSG_SPELLORDAMAGE_IMMUNE`: one line. See
/// [`combatlog::parse_spell_not_taken`].
pub(super) fn immune_log(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, combatlog::parse_spell_not_taken(&pkt.body)) else {
        return;
    };
    ctx.world.note_combat(CombatEvent::Immune {
        caster: log.caster,
        target: log.target,
        spell_id: log.spell_id,
    });
}

/// `SMSG_PROCRESIST`: one line, on the same layout as the immunity.
pub(super) fn proc_resist_log(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, combatlog::parse_spell_not_taken(&pkt.body)) else {
        return;
    };
    ctx.world.note_combat(CombatEvent::ProcResist {
        caster: log.caster,
        target: log.target,
        spell_id: log.spell_id,
    });
}

/// `SMSG_DISPEL_FAILED`: one line per aura that stayed.
pub(super) fn dispel_failed_log(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, combatlog::parse_dispel_failed(&pkt.body)) else {
        return;
    };
    for spell_id in log.spells {
        ctx.world.note_combat(CombatEvent::DispelFailed {
            caster: log.caster,
            victim: log.victim,
            spell_id,
        });
    }
}

/// `SMSG_SPELLINSTAKILLLOG`: one line.
pub(super) fn instakill_log(ctx: &mut Incoming, pkt: &Packet) {
    let Some((victim, spell_id)) = read(ctx.stats, pkt, combatlog::parse_instakill_log(&pkt.body)) else {
        return;
    };
    ctx.world.note_combat(CombatEvent::InstaKill { victim, spell_id });
}

pub(super) fn energize_log(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, combatlog::parse_energize_log(&pkt.body)) else {
        return;
    };
    ctx.world.note_combat(CombatEvent::Energize(log));
}

pub(super) fn periodic_aura_log(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(
        ctx.stats,
        pkt,
        combatlog::parse_periodic_aura_log(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_combat(CombatEvent::PeriodicAura(log));
}

/// `SMSG_AI_REACTION`: a creature has noticed the player, or has decided to
/// fight.
///
/// This is the only packet in the protocol whose whole purpose is a sound. The
/// 1.12.1 client plays a voice for two of the five reactions and does nothing
/// else: no animation, no message, no interface change. This arm therefore
/// records an event that only `sound::combat` reads. Dropping it leaves
/// creatures silent when they aggro.
pub(super) fn ai_reaction(ctx: &mut Incoming, pkt: &Packet) {
    let Some(reaction) = read(ctx.stats, pkt, action::parse_ai_reaction(&pkt.body)) else {
        return;
    };
    ctx.stats.ai_reactions += 1;
    ctx.world.apply_ai_reaction(&reaction);
}

/// `SMSG_EMOTE`: a one-shot emote.
///
/// This is the closest the protocol comes to naming an animation. The id is an
/// `Emotes.dbc` row whose third column is the `AnimationData.dbc` id. That join
/// is a game-data lookup, so this arm records the row and the renderer looks up
/// the animation.
pub(super) fn emote(ctx: &mut Incoming, pkt: &Packet) {
    let Some(emote) = read(ctx.stats, pkt, action::parse_emote(&pkt.body)) else {
        return;
    };
    ctx.stats.emotes += 1;
    ctx.world.apply_emote(&emote);
}

/// The three packets whose whole content is a sound: `SMSG_PLAY_SOUND`,
/// `SMSG_PLAY_MUSIC` and `SMSG_PLAY_OBJECT_SOUND`.
///
/// Pushed onto the player queue rather than applied to the world, because
/// there is no state to apply: two arrivals are two sounds. See
/// [`crate::play::sound`] for the bodies, and for the field order of the
/// object sound, which is the reverse of the spell visuals below.
///
/// No other packet plays these sounds. Every scripted sound in the game (a
/// boss line, a gate, `Map::PlayDirectSoundToMap` over a whole zone, the
/// outdoor PvP banners) arrives here or is not heard.
pub(super) fn play_sound(ctx: &mut Incoming, pkt: &Packet, op: Opcode) {
    let cue = match op {
        Opcode::SMSG_PLAY_SOUND => {
            read(ctx.stats, pkt, sound::parse_play_sound(&pkt.body)).map(sound::Cue::Direct)
        }
        Opcode::SMSG_PLAY_MUSIC => {
            read(ctx.stats, pkt, sound::parse_play_sound(&pkt.body)).map(sound::Cue::Music)
        }
        Opcode::SMSG_PLAY_OBJECT_SOUND => read(
            ctx.stats,
            pkt,
            sound::parse_play_object_sound(&pkt.body),
        )
        .map(|(sound_id, guid)| sound::Cue::Object { sound_id, guid }),
        _ => return,
    };
    let Some(cue) = cue else {
        return;
    };
    ctx.stats.pushed_sounds += 1;
    ctx.world.note_event(PlayerEvent::PlaySound(cue));
}

/// A `SpellVisualKit` the server wants played on a unit:
/// `SMSG_PLAY_SPELL_VISUAL` on the unit doing something,
/// `SMSG_PLAY_SPELL_IMPACT` on the unit it was done to.
///
/// Recorded on the entity rather than pushed on the queue, the opposite of
/// [`play_sound`] above. A sound is played at this client and is then over. A
/// visual is a property of a unit and lasts as long as the unit's models are
/// loaded. A visual can also be about any unit, where the queue is only about
/// the player.
///
/// This is the only packet that says a character is eating. vmangos sends kit
/// 406 or 438 on every regeneration tick while a character sits with food or
/// drink, and both kits are `animID 61` (`EmoteEat`) plus a model.
pub(super) fn play_spell_visual(ctx: &mut Incoming, pkt: &Packet, impact: bool) {
    let Some((guid, kit)) = read(ctx.stats, pkt, sound::parse_play_spell_visual(&pkt.body)) else {
        return;
    };
    ctx.stats.pushed_visuals += 1;
    ctx.world.apply_spell_visual(guid, kit, impact);
}

/// The two halves of a cast.
///
/// `SMSG_SPELL_START` is the wind-up and carries how long the bar runs;
/// `SMSG_SPELL_GO` is the release, and a next-swing ability sends only the
/// second. Both are recorded against the caster, which is the second guid in
/// the body, not the first.
///
/// When the caster is the player, each packet is also queued as a player event,
/// because the interface's cast bar is driven by these packets. This client
/// once drew its own wind-up, release and cast bar at the key press and removed
/// them when the server refused; build 5875 does neither. See
/// [`crate::play::spells::PlayerEvent::CastStarted`], which quotes vmangos'
/// comment and states the 1.12.1 client's behaviour.
pub(super) fn cast(ctx: &mut Incoming, pkt: &Packet, start: bool) {
    let Some(cast) = read(ctx.stats, pkt, action::parse_spell_cast(&pkt.body, start)) else {
        return;
    };
    ctx.stats.casts += 1;
    if ctx.world.player_guid == Some(cast.caster) {
        ctx.world.note_event(match start {
            true => PlayerEvent::CastStarted {
                spell_id: cast.spell_id,
                cast_time_ms: cast.cast_time_ms,
            },
            false => PlayerEvent::CastReleased {
                spell_id: cast.spell_id,
            },
        });
    }
    ctx.world.apply_cast(&cast, start);
}

/// `SMSG_DESTROY_OBJECT` — a plain u64 guid (`Object::DestroyForPlayer`).
///
/// Distinct from the OUT_OF_RANGE block inside an update: this one means the
/// object is gone, not merely too far away. Ignoring it leaves the corpses of
/// despawned creatures standing around for the rest of the session.
pub(super) fn destroy(ctx: &mut Incoming, pkt: &Packet) {
    if pkt.body.len() < 8 {
        ctx.stats.unreadable(pkt);
        return;
    }
    let guid = crate::bytes::Reader::new(&pkt.body).u64();
    ctx.world.remove(guid);
}

/// Another player moved.
///
/// The server relays that player's own `MSG_MOVE_*` packet to everyone nearby.
/// It is the only packet that moves another player: `SMSG_MONSTER_MOVE` covers
/// server-driven units, and a values update carries no position.
pub(super) fn moved(ctx: &mut Incoming, pkt: &Packet) {
    let Some((guid, info)) = read(ctx.stats, pkt, movement::parse_movement_broadcast(&pkt.body))
    else {
        return;
    };
    ctx.stats.player_moves += 1;
    ctx.world.apply_movement(guid, &info);
}

/// Another unit's speed changed, on one of the two observer families. See
/// [`movement::SpeedBroadcast`], which says why there are two.
///
/// This is not acknowledged and must not be. The acknowledgement belongs to
/// the client that controls the unit, and both families are sent to everybody
/// except that client (`SendMovementMessageToSet(..., mover)`). Answering one
/// triggers `OnWrongAckData`, and three of those kick the player. For the same
/// reason [`super::acks::flag_change`] records a change about another unit
/// without answering.
///
/// The speed is applied even when the block's position is refused, because they
/// are separate claims and the speed is the one that lasts: a rejected
/// coordinate leaves the entity where it was, while a stale speed goes on
/// mis-reckoning it for the rest of the session.
pub(super) fn speed_broadcast(ctx: &mut Incoming, pkt: &Packet, op: Opcode) {
    let Some((slot, kind)) = movement::broadcast_speed_slot(op) else {
        return;
    };
    let parsed = match kind {
        movement::SpeedBroadcast::Observed => {
            read(ctx.stats, pkt, movement::parse_observed_speed(&pkt.body))
                .map(|(guid, info, speed)| (guid, Some(info), speed))
        }
        movement::SpeedBroadcast::Spline => {
            read(ctx.stats, pkt, movement::parse_spline_speed(&pkt.body))
                .map(|(guid, speed)| (guid, None, speed))
        }
    };
    let Some((guid, info, speed)) = parsed else {
        return;
    };
    ctx.stats.speed_broadcasts += 1;

    // The block first, because it creates the entity the speed is recorded
    // on, and because `apply_movement` is the one place a broadcast position
    // is checked.
    if let Some(info) = info {
        ctx.world.apply_movement(guid, &info);
    }
    if let Some(entity) = ctx.world.get_mut(guid) {
        let mut speeds = entity.speeds.unwrap_or_default();
        speeds.0[slot] = speed;
        entity.speeds = Some(speeds);
    }
}

/// A movement flag change about a unit no player is moving: one of the twelve
/// `SMSG_SPLINE_MOVE_*` opcodes.
///
/// The body is a packed guid and nothing else. The flag and its direction come
/// from the opcode, through [`movement::SplineFlagChange`], which has the table
/// and the reason there are three families rather than one.
///
/// This is not acknowledged and must not be. `SendMovementFlagChangeToAll` is
/// the branch vmangos' `Unit::SetRooted` takes when no player moves the unit,
/// so the server has no pending change to answer and is not waiting for a
/// counter. Replying triggers `OnWrongAckData`, and three of those kick the
/// player, for the reason [`speed_broadcast`] gives about its own family.
///
/// A creature's movement flags arrive once, in its create block, and no other
/// packet restates them. If this family is dropped, a rooted creature is still
/// reckoned and drawn as one that can move, a patrol told to walk is
/// dead-reckoned at its run speed, and this client never learns that a unit is
/// hovering or water walking.
pub(super) fn spline_flag(ctx: &mut Incoming, pkt: &Packet, op: Opcode) {
    let Some(change) = movement::SplineFlagChange::of(op) else {
        return;
    };
    let Some(guid) = read(ctx.stats, pkt, movement::parse_spline_flag(&pkt.body)) else {
        return;
    };
    ctx.stats.spline_flag_changes += 1;
    ctx.world.apply_spline_flag(guid, change);
}

/// `SMSG_TRIGGER_CINEMATIC`: the intro cinematic, ended as soon as it is
/// offered.
///
/// The body is one `u32`, the `ChrRaces.dbc` `CinematicSequence` of the
/// character's race. This client has no cinematic player, so it answers at
/// once with `CMSG_COMPLETE_CINEMATIC`, which has an empty body.
///
/// Not answering empties the world. The packet is sent on one login per
/// character, the first (`HandlePlayerLogin` sends it when total played time
/// is zero). `Player::CinematicStart` then runs `UpdateCinematic` from
/// `Player::Update` for as long as `m_currentCinematicEntry` is set. Once a
/// second, that summons a waypoint creature and calls `GetCamera().SetView` on
/// it, which recomputes the whole client-visible set from the waypoint:
/// `VisibleNotifier` sends an out-of-range destroy for every unit, player and
/// game object that was visible from the character, and create blocks for
/// whatever is near the waypoint instead. The character's own object is not in
/// that set and is never destroyed, which is why the symptom is an empty world
/// with the player and the ground still in it.
///
/// `CinematicEnd` is the only thing that clears the entry and calls
/// `Camera::ResetView`, and `HandleCompleteCinematic` is its only caller, so
/// the reply is what restores the camera. Logging out and back in also cleared
/// the state, because at the second login played time is above zero and the
/// packet is not sent again.
///
/// `CinematicEnd` also calls `CheckAreaExploreAndOutdoor`, so the reply is
/// additionally what marks a new character's starting zone explored.
pub(super) fn cinematic(ctx: &mut Incoming, pkt: &Packet) {
    ctx.stats.cinematics += 1;
    ctx.replies.push(Opcode::CMSG_COMPLETE_CINEMATIC, Vec::new());
    let _ = pkt;
}

/// `SMSG_LOGIN_SETTIMESPEED`: the world's clock, stated once at login and not
/// restated. See [`crate::play::time`].
///
/// Nothing is acknowledged and nothing else changes. The light tables are
/// indexed by the clock, and the renderer advances it itself.
pub(super) fn game_time(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(time) = read(
        ctx.stats,
        pkt,
        crate::play::time::parse_login_set_time_speed(&pkt.body),
    ) {
        ctx.world.game_time = Some(time);
    }
}

/// `SMSG_WEATHER`: kept as state, like the clock, and counted.
///
/// A body that will not parse (short, or a type past 3) is reported on the
/// warnings channel by `read`, and the sky is left as it was. The 1.12.1 client
/// also leaves the weather unchanged for a type it refuses. See
/// [`crate::play::weather`] for the ramp and the density rule.
pub(super) fn weather(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(weather) = read(ctx.stats, pkt, crate::play::weather::parse_weather(&pkt.body)) {
        ctx.stats.weather += 1;
        ctx.world.weather = Some(weather);
    }
}

/// Inflate `u32 inflatedSize` + a zlib stream, the header
/// `SMSG_COMPRESSED_UPDATE_OBJECT` and `SMSG_COMPRESSED_MOVES` share.
fn inflate_update(body: &[u8]) -> io::Result<Vec<u8>> {
    if body.len() < 4 {
        return Err(io::Error::other("compressed update too short"));
    }
    let expected = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
    let mut out = Vec::with_capacity(expected);
    ZlibDecoder::new(&body[4..]).read_to_end(&mut out)?;
    if out.len() != expected {
        return Err(io::Error::other(format!(
            "inflated {} bytes, header promised {expected}",
            out.len()
        )));
    }
    Ok(out)
}
