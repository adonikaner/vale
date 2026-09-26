//! What is in the world and what it is doing: object updates, movement, and the
//! packets that drive an animation.
//!
//! One `pub(super) fn` per arm of [`super::apply_packet`]'s match — see that
//! module's comment for why the dispatch stays whole and only the bodies live
//! out here.
//!
//! **`world` here is [`super`]'s child, not `crate::socket::world`.** The latter is the
//! socket; this is the world it describes. They are one directory apart and the
//! paths never mix, but the names are close enough to be worth saying once.

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

/// **`SMSG_COMPRESSED_MOVES`: a bag of movement packets, and the desync.**
///
/// The server does not send this until it has to. `WorldSession::SendMovementPacket`
/// counts the movement packets it has sent this interval, and while that stays
/// under `CONFIG_UINT32_COMPRESSION_MOVEMENT_COUNT` every one of them goes out
/// on its own opcode; past it, they are **batched into this one instead**. So a
/// quiet session never produces it and a busy zone produces nothing else —
/// which is why a client that drops it works perfectly against a localhost
/// server with one player on it and falls apart on a real realm.
///
/// **Everything about movement goes through that door.** `MoveSplineInit::Launch`
/// (`SMSG_MONSTER_MOVE` — every creature in the game), `MovementHandler`'s
/// relays (`MSG_MOVE_*` — every other player), and `MovementPacketSender` (the
/// speed changes, the flag changes, the knockbacks) all call
/// `WorldObject::SendMovementMessageToSet`, which is `SendMovementPacket`. Drop
/// this opcode and you lose all three at once, with no warning and no parse
/// failure: the world simply stops moving and then snaps whenever something
/// else happens to state a position. That is *"mobs appear to be standing far
/// away but are actually attacking you"*, and it was measured before it was
/// fixed — **428 packets and 35.7 KiB in one session**, off the `F4` net tab's
/// unhandled list.
///
/// ## The layout, from `MovementData` in vmangos' `Objects/UpdateData.cpp`
///
/// ```text
/// u32 uncompressedSize
/// <zlib deflate of exactly that many bytes>, and inside it, repeated:
///     u8  size        // the body's length **plus two**, for the opcode
///     u16 opcode
///     ..  body        // size - 2 bytes
/// ```
///
/// The `+2` is `AddPacket`'s own arithmetic and the reason a sub-packet may
/// never exceed 253 bytes of body: `CanAddPacket` refuses anything that would
/// overflow the `u8`, with the comment *"Client crash else"*.
///
/// ## Each one goes back through the *one* dispatch
///
/// Rather than a second switch over the movement opcodes, which is the
/// arrangement `handler`'s own module doc is about: an inner packet is an
/// ordinary packet that happened to arrive in a bag, and every counter, every
/// warning and every `traffic` row it should produce is the one
/// [`crate::socket::handler::apply_packet`] already produces. **A nested bag is
/// refused** — the server never puts one inside another (only movement packets
/// reach the compressor) and unbounded recursion off the wire is not a thing to
/// leave open.
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
/// **Collected rather than streamed**, because the dispatch takes `ctx` mutably
/// and the walk needs `ctx` to report a damaged tail; the buffer is a few
/// kilobytes and the packets inside it are tens of bytes.
///
/// A truncated or nonsensical tail costs the rest of the bag and is *reported*,
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
        // See the doc above: a bag inside a bag is not something the server
        // produces, and recursing on it is how a malformed stream becomes a
        // stack overflow.
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
/// **And it may be *us*.** Warrior Charge moves the caster with the same
/// `MoveSpline` machinery a patrolling creature is walked with, so the packet
/// that arrives is this one with our own guid in it — and it is owed
/// `CMSG_MOVE_SPLINE_DONE`, without which the server discards every movement
/// packet this session sends for the rest of it. See
/// [`crate::state::movement::Ride`](crate::state::movement::Mover::ride), which is where both
/// halves of that live.
///
/// The world's copy is applied either way: it holds the entity the renderer
/// draws, and a snapshot pump with no simulation of its own has nothing else.
pub(super) fn monster_move(ctx: &mut Incoming, pkt: &Packet) {
    monster_move_inner(ctx, pkt, false)
}

/// **`SMSG_MONSTER_MOVE_TRANSPORT`: the same move, on a moving floor.**
///
/// `MoveSplineInit::Launch` builds `SMSG_MONSTER_MOVE`, then — for a unit it is
/// also boarding — changes the opcode and writes the transport's packed guid
/// before everything else. So the two are one body with one extra guid, and a
/// client that reads the second as the first takes that guid's bytes as the
/// unit's start coordinates — parsing happily while it does, on any path long
/// enough for the extra twelve bytes to be absorbed by the nodes.
///
/// **Every waypoint after it is in the transport's own frame**, which is what
/// makes this worth more than a name: dropped into the world unconverted, a
/// boat's crew walking three yards across the deck are placed three yards from
/// the map's origin. See [`movement::Spline::in_world`], which is where the
/// conversion is and where the "transport not loaded" case is answered.
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
    // can look another unit up — a charge is sent with `SetFacingGUID(target)`
    // and the mover has no way to find it.
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

/// One melee swing.
///
/// The server never says "start attacking" and never names an animation, so
/// this packet is the *only* statement that a blow happened — without it a
/// fight is two creatures standing in their idle loops while the health bars
/// empty.
pub(super) fn attack(ctx: &mut Incoming, pkt: &Packet) {
    let Some(attack) = read(ctx.stats, pkt, action::parse_attack_update(&pkt.body)) else {
        return;
    };
    ctx.stats.attacks += 1;
    ctx.world.note_combat(CombatEvent::Swing(attack));
    ctx.world.apply_attack(&attack);
}

/// **A spell landed on somebody** — or a damage-over-time ticked, which comes
/// down the same opcode with `periodicLog` set.
///
/// The other half of what a fight is made of, and the half this client read
/// nothing of for a long time: `SMSG_ATTACKERSTATEUPDATE` is the *weapon* and
/// nothing else, so a client reading only that draws a number for every
/// auto-attack and nothing at all for a Fireball. That is exactly what "spells
/// are not triggering it" was.
pub(super) fn spell_damage(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, action::parse_spell_damage(&pkt.body)) else {
        return;
    };
    ctx.stats.spell_logs += 1;
    ctx.world.note_combat(CombatEvent::SpellDamage(log));
    ctx.world.apply_spell_damage(&log);
}

/// …and a heal, which is a number over a head like any other.
///
/// `SMSG_SPELLHEALLOG` is sent only for builds over 1.9.4, which 1.12 is — the
/// older clients learned about a heal from the health field moving and nothing
/// else.
pub(super) fn spell_heal(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, action::parse_spell_heal(&pkt.body)) else {
        return;
    };
    ctx.stats.spell_logs += 1;
    ctx.world.note_combat(CombatEvent::SpellHeal(log));
    ctx.world.apply_spell_heal(&log);
}

/// **The seven packets whose only consumer is a line of text.**
///
/// Each one parses, and each one goes on the combat log's own queue for the
/// renderer to compose. There is no state to apply: nothing here moves a unit,
/// changes a field or answers the server. That is exactly why they were unread
/// for so long — a client that drops all seven plays an identical-looking fight
/// with an empty log.
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

/// **One packet, one line per target.**
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

/// **One cast, one line per effect entry**, the same fan-out
/// [`spell_miss_log`] makes and for the same reason.
///
/// `SMSG_SPELLLOGEXECUTE` is the one packet in the family that is a list of
/// lists: an effect id, a count, and that many entries whose *width depends on
/// the id*. Four of its nine effect kinds have a sentence —
/// `SPELLEXTRAATTACKS`, `SPELLINTERRUPT`, `FEEDPET_LOG` and
/// `SPELLDURABILITYDAMAGE` — and the rest are already said by packets of their
/// own: a heal by `SMSG_SPELLHEALLOG`, a drain and an energize by
/// `SMSG_PERIODICAURALOG` and `SMSG_SPELLENERGIZELOG`. Saying them twice would
/// double every line a Life Tap or a Drain Mana produces.
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
                    // **The spell the sentence names is the one that was
                    // stopped**, not the one that stopped it.
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

/// …and one line per aura a dispel took off.
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

/// A creature has noticed you, or has decided to fight you.
///
/// **The only packet in the protocol whose whole purpose is a sound.** The
/// client's handler branches on two of the five reactions, plays a
/// voice for each and does nothing else — no animation, no message, no
/// interface. So this arm records an event nobody but `sound::combat` reads,
/// which is unusual enough to be worth saying out loud: dropping it is a world
/// where nothing ever growls at you.
pub(super) fn ai_reaction(ctx: &mut Incoming, pkt: &Packet) {
    let Some(reaction) = read(ctx.stats, pkt, action::parse_ai_reaction(&pkt.body)) else {
        return;
    };
    ctx.stats.ai_reactions += 1;
    ctx.world.apply_ai_reaction(&reaction);
}

/// A one-shot emote.
///
/// **The closest the protocol comes to naming an animation** — the id is an
/// `Emotes.dbc` row whose third column is the `AnimationData.dbc` id — but the
/// join is a game-data lookup, so what is recorded here is the row and the
/// renderer does the hop.
pub(super) fn emote(ctx: &mut Incoming, pkt: &Packet) {
    let Some(emote) = read(ctx.stats, pkt, action::parse_emote(&pkt.body)) else {
        return;
    };
    ctx.stats.emotes += 1;
    ctx.world.apply_emote(&emote);
}

/// **The three packets whose whole content is a noise** — `SMSG_PLAY_SOUND`,
/// `SMSG_PLAY_MUSIC`, `SMSG_PLAY_OBJECT_SOUND`.
///
/// Pushed onto the player queue rather than folded into the world, because
/// there is no state to fold: two arrivals are two noises. See
/// [`crate::play::sound`] for the bodies and for the fact that the object one
/// is laid out the opposite way round from the spell visuals below.
///
/// **Nothing else in the protocol reaches these sounds.** Every scripted event
/// in the game — a boss line, a gate, `Map::PlayDirectSoundToMap` over a whole
/// zone, the outdoor PvP banners — arrives here or is silent.
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

/// **A `SpellVisualKit` the server wants played on a unit** —
/// `SMSG_PLAY_SPELL_VISUAL` about whoever is doing it,
/// `SMSG_PLAY_SPELL_IMPACT` about whoever it happened to.
///
/// Recorded on the entity rather than pushed on the queue, which is the
/// opposite of [`play_sound`] one function up and is the difference between a
/// noise and a picture: a noise is played at this client and is over, and a
/// visual becomes true about a unit and stays true for as long as its models
/// are up. It is also about *anybody*, where the queue is only ever about us.
///
/// **This is the only thing on the wire that says a character is eating.**
/// vmangos sends kit 406 or 438 on every regeneration tick while one sits with
/// food or drink, and both kits are `animID 61` (`EmoteEat`) plus a model.
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
/// `SMSG_SPELL_GO` is the release, and a **next-swing** ability sends only the
/// second. Both are recorded against the **caster**, which is the second guid in
/// the body and not the first.
///
/// **And both are news for our own casting state, which is the whole of why the
/// interface half exists.** This client used to draw its own wind-up, release
/// and cast bar at the *press* and take them back off when the server said no;
/// 5875 does neither. See [`crate::play::spells::PlayerEvent::CastStarted`], which
/// carries the two addresses and vmangos' own comment.
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
/// The server echoes their own `MSG_MOVE_*` packet to everyone nearby, and it
/// is the **only** thing that moves a player — `SMSG_MONSTER_MOVE` covers
/// server-driven units and a values update carries no position at all.
pub(super) fn moved(ctx: &mut Incoming, pkt: &Packet) {
    let Some((guid, info)) = read(ctx.stats, pkt, movement::parse_movement_broadcast(&pkt.body))
    else {
        return;
    };
    ctx.stats.player_moves += 1;
    ctx.world.apply_movement(guid, &info);
}

/// **Somebody else's speed changed**, on one of the two observer families —
/// see [`movement::SpeedBroadcast`], which says why there are two.
///
/// This is not acknowledged and must not be: the ack belongs to whoever
/// *controls* the unit, and both of these are sent to everybody except them
/// (`SendMovementMessageToSet(..., mover)`). Answering one would be
/// `OnWrongAckData`, three of which is a kick — the same reason
/// [`super::acks::flag_change`] records a change about another unit in silence.
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

    // The block first, because it creates the entity a speed can be recorded
    // on — and because `apply_movement` is the one place a broadcast position
    // is vetted.
    if let Some(info) = info {
        ctx.world.apply_movement(guid, &info);
    }
    if let Some(entity) = ctx.world.get_mut(guid) {
        let mut speeds = entity.speeds.unwrap_or_default();
        speeds.0[slot] = speed;
        entity.speeds = Some(speeds);
    }
}

/// **A movement flag stated about a unit no player is moving** — one of the
/// twelve `SMSG_SPLINE_MOVE_*`.
///
/// The body is a packed guid and nothing else; the flag and its direction come
/// from the opcode, through [`movement::SplineFlagChange`], which has the table
/// and the reason there are three families rather than one.
///
/// **Not acknowledged, and it must not be.** `SendMovementFlagChangeToAll` is
/// the branch `Unit::SetRooted` takes when the unit is *not* moved by a player,
/// so there is no pending change on the server to answer and nothing waiting
/// for a counter. Replying would be `OnWrongAckData` — three of which is a
/// kick — for exactly the reason [`speed_broadcast`] gives about its own
/// family.
///
/// What dropping it costs: a creature's movement flags arrive once, in its
/// create block, and nothing else on the wire ever restates them. So a rooted
/// creature goes on being reckoned and drawn as one that can move, a patrol
/// told to walk is dead-reckoned at its run speed, and a hovering or
/// water-walking unit is one this client never learns about at all.
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

/// **The intro cinematic, ended the frame it is offered.**
///
/// The body is one `u32`, the `ChrRaces.dbc` `CinematicSequence` of the
/// character's race. This client has no cinematic player, so it answers
/// `CMSG_COMPLETE_CINEMATIC` — an empty body — at once.
///
/// **Not answering empties the world.** The packet is sent on exactly one login
/// per character, the first (`HandlePlayerLogin`'s gate is total played time
/// zero), and `Player::CinematicStart` then runs `UpdateCinematic` from
/// `Player::Update` for as long as `m_currentCinematicEntry` is set. Once a
/// second that summons a waypoint creature and calls `GetCamera().SetView` on
/// it, which recomputes the whole client-visible set from the waypoint:
/// `VisibleNotifier` sends an out-of-range destroy for every unit, player and
/// game object that was visible from the character, and create blocks for
/// whatever is near the waypoint instead. The character's own object is not in
/// that set and is never destroyed, which is why the symptom is an empty world
/// with the player and the ground still in it.
///
/// `CinematicEnd` is the only thing that clears the entry and calls
/// `Camera::ResetView`, and `HandleCompleteCinematic` is its only caller. So
/// the reply is not a courtesy: it is what gives the camera back. Logging out
/// and back in cleared it because the second login sets played time above zero
/// and the packet is never sent again.
///
/// `CinematicEnd` also calls `CheckAreaExploreAndOutdoor`, so the reply is
/// additionally what marks a new character's starting zone explored.
pub(super) fn cinematic(ctx: &mut Incoming, pkt: &Packet) {
    ctx.stats.cinematics += 1;
    ctx.replies.push(Opcode::CMSG_COMPLETE_CINEMATIC, Vec::new());
    let _ = pkt;
}

/// The world's clock, stated once and never again — see [`crate::play::time`].
///
/// Nothing is acknowledged and nothing else moves; the light tables are indexed
/// by it and the renderer runs it forward itself.
pub(super) fn game_time(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(time) = read(
        ctx.stats,
        pkt,
        crate::play::time::parse_login_set_time_speed(&pkt.body),
    ) {
        ctx.world.game_time = Some(time);
    }
}

/// `u32 inflatedSize` + a zlib stream — the header
/// `SMSG_COMPRESSED_UPDATE_OBJECT` and `SMSG_COMPRESSED_MOVES` share.
/// **`SMSG_WEATHER`** — kept as state, like the clock, and counted.
///
/// A body that will not parse — short, or a type past 3 — is reported on the
/// warnings channel by `read` and the sky is left as it was, which is what
/// the reference does with a type it refuses.
/// See [`crate::play::weather`], where the ramp and the density rule are.
pub(super) fn weather(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(weather) = read(ctx.stats, pkt, crate::play::weather::parse_weather(&pkt.body)) {
        ctx.stats.weather += 1;
        ctx.world.weather = Some(weather);
    }
}

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
