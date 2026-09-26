//! **What this character can do, and what the server said about doing it.**
//!
//! One `pub(super) fn` per arm of [`super::apply_packet`]'s match, exactly as
//! [`super::world`] is — see that module's comment for why the dispatch stays
//! whole and only the bodies live out here.
//!
//! The split against `world` is *whose* packet it is. Everything there is about
//! the world and arrives about anybody: a swing, an emote, a cast, a spline.
//! Everything here is about **us** — the spellbook is ours, the action bar is
//! ours, a cast result answers a `CMSG_CAST_SPELL` we sent, and the five
//! attack-swing refusals answer a `CMSG_ATTACKSWING` we sent. The two attack
//! *state* packets are the exception that proves it: they are broadcast about
//! everyone and [`crate::state::objects::ObjectManager::apply_attack_state`] throws
//! away every one that is not ours, because the rest is already covered by the
//! combat flag on the units themselves.
//!
//! **None of these can fail loudly.** A cast result that is dropped is a spell
//! that silently does nothing; a spellbook that is dropped is an empty action
//! bar. Both look like "the client does not implement casting yet" rather than
//! like a bug, which is why every one of them goes through [`super::read`] and
//! reaches the HUD as a warning when the body will not parse.

use super::{read, Incoming};
use crate::opcodes::Opcode;
use crate::play::spells::{self, PlayerEvent};
use crate::play::timers;
use crate::play::wdb::Kind;
use crate::socket::world::Packet;

/// `SMSG_INITIAL_SPELLS`: every spell this character knows.
///
/// Said **once**, in the login burst, and never restated. Everything the action
/// bar can do is downstream of this one packet.
pub(super) fn initial_spells(ctx: &mut Incoming, pkt: &Packet) {
    let Some(book) = read(ctx.stats, pkt, spells::parse_initial_spells(&pkt.body)) else {
        return;
    };
    ctx.stats.spells_known = book.known.len() as u32;
    ctx.world.apply_spellbook(book);
}

/// `SMSG_ACTION_BUTTONS`: where the last session left the bar.
///
/// 120 bare words with no count in front of them, so the *length* is the
/// framing and a short read is a half-filled bar rather than a failure.
pub(super) fn action_buttons(ctx: &mut Incoming, pkt: &Packet) {
    let buttons = spells::parse_action_buttons(&pkt.body);
    ctx.stats.action_buttons = buttons.len() as u32;
    ctx.world.apply_action_buttons(buttons);
}

/// `SMSG_LEARNED_SPELL` (`u32`) and `SMSG_REMOVED_SPELL` (**`u16`**).
///
/// The widths differ and the packets are adjacent in the opcode table, which is
/// the sort of asymmetry that reads as "learned spell 0" when it is guessed at.
pub(super) fn spell_change(ctx: &mut Incoming, pkt: &Packet, learned: bool) {
    let parsed = if learned {
        spells::parse_learned_spell(&pkt.body)
    } else {
        spells::parse_removed_spell(&pkt.body)
    };
    let Some(spell_id) = read(ctx.stats, pkt, parsed) else {
        return;
    };
    ctx.world.apply_spell_change(spell_id, learned);
}

/// `SMSG_SUPERCEDED_SPELL`: a higher rank replaced a lower one.
///
/// **The root cause of a stale action bar, and it had never been read.** The
/// swap is the client's to perform — in the book and in every slot holding the
/// old id — and the server says so once and never again. What it costs to miss
/// is a button that draws correctly and casts nothing, because the server
/// refuses a superseded rank without replying; see
/// [`spells::parse_superceded_spell`].
pub(super) fn superceded_spell(ctx: &mut Incoming, pkt: &Packet) {
    let Some((old, new)) = read(ctx.stats, pkt, spells::parse_superceded_spell(&pkt.body)) else {
        return;
    };
    ctx.world.apply_superceded_spell(old, new);
}

/// `SMSG_CAST_RESULT`: the answer to our own `CMSG_CAST_SPELL`.
///
/// **Sent on success as well as on failure.** The success is not noise: it is
/// what says a cast bar may keep running, and its *absence* after a failure is
/// what stops one. See [`spells::CastResult`].
pub(super) fn cast_result(ctx: &mut Incoming, pkt: &Packet) {
    let Some(result) = read(ctx.stats, pkt, spells::parse_cast_result(&pkt.body)) else {
        return;
    };
    ctx.stats.cast_results += 1;
    ctx.world.note_event(match result.failure {
        Some(reason) => PlayerEvent::CastFailed {
            spell_id: result.spell_id,
            reason,
            requirement: result.requirement,
        },
        None => PlayerEvent::CastAccepted {
            spell_id: result.spell_id,
        },
    });
    // **And take the art back off.** This client draws its own cast at the
    // press, so a refusal arrives with the wind-up already held and — for an
    // instant — the release already playing. Nothing else would end either; see
    // [`crate::state::objects::Entity::casts_cancelled`].
    if result.failure.is_some() {
        if let Some(guid) = ctx.world.player_guid {
            ctx.world.apply_cast_cancelled(guid, result.spell_id);
        }
    }
}

/// `SMSG_INVENTORY_CHANGE_FAILURE`: an item verb was refused.
///
/// **The only thing on the wire that answers a right-click the server threw
/// away.** A potion whose level requirement is not met never reaches
/// `Spell::prepare`, so no `SMSG_CAST_RESULT` is sent and the click is otherwise
/// indistinguishable from a click on empty ground — which is the report this arm
/// exists for.
///
/// `EQUIP_ERR_OK` arrives here too, as a one-byte body: several verbs end with
/// one and it is not a failure. It is passed on rather than dropped, because the
/// *release* code `HandleUseItemOpcode` sends before the real reason
/// (`EQUIP_ERR_NONE`) is a different value with no string, and telling the two
/// apart is the reader's business rather than this one's.
/// **`SMSG_PET_SPELLS`: the whole pet panel, or the eight bytes that take it
/// down.**
///
/// The bar is stored rather than fanned out into events because it is *state* —
/// a panel is drawn from it every frame — and because the dismissal is the same
/// packet with a zero guid, which a queue of edges could not express.
///
/// **The pet's *name* is not asked for here**, and that is deliberate: this
/// packet arrives before the pet's own create block on a summon, so the number
/// it is keyed by does not exist yet. `ObjectManager::unresolved_pet_names`
/// derives the ask from the bar and the entity together instead.
pub(super) fn pet_spells(ctx: &mut Incoming, pkt: &Packet) {
    let Some(spells) = read(ctx.stats, pkt, crate::play::pet::parse_pet_spells(&pkt.body)) else {
        return;
    };
    ctx.world.apply_pet_spells(spells);
    // …and the query pass is told to look, which is what turns the name's ask
    // from a two-second beat into the next tick. See
    // `ObjectManager::take_query_hint`.
    ctx.world.hint_queries();
}

/// `SMSG_PET_MODE` — the four state bytes on their own, for when only the mood
/// changed.
pub(super) fn pet_mode(ctx: &mut Incoming, pkt: &Packet) {
    let Some(mode) = read(ctx.stats, pkt, crate::play::pet::parse_pet_mode(&pkt.body)) else {
        return;
    };
    ctx.world.apply_pet_mode(mode);
}

/// `SMSG_PET_NAME_QUERY_RESPONSE` — what the player called it, keyed by number.
pub(super) fn pet_name(ctx: &mut Incoming, pkt: &Packet) {
    let Some(name) = read(ctx.stats, pkt, crate::play::pet::parse_pet_name(&pkt.body)) else {
        return;
    };
    // A rename answers under the same pet number, so this is the one cached
    // answer that replaces rather than skips — see [`crate::play::wdb`].
    ctx.world
        .remember_again(Kind::PetName, u64::from(name.pet_number), &pkt.body);
    ctx.world.apply_pet_name(name);
}

/// **The four pet packets whose whole content is a sentence**, plus the unlearn
/// confirmation.
///
/// Each is an edge rather than state — see [`PlayerEvent::PetFeedback`] — and
/// two of the five have no body at all, which is the same shape the five
/// attack-swing refusals have: the opcode *is* the message.
pub(super) fn pet_feedback(ctx: &mut Incoming, pkt: &Packet) {
    let Some(message) = read(ctx.stats, pkt, crate::play::pet::parse_pet_feedback(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::PetFeedback(message));
}

pub(super) fn pet_cast_failed(ctx: &mut Incoming, pkt: &Packet) {
    let Some(fail) = read(
        ctx.stats,
        pkt,
        crate::play::pet::parse_pet_cast_failed(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::PetCastFailed {
        spell_id: fail.spell_id,
        reason: fail.reason,
    });
}

pub(super) fn pet_tame_failure(ctx: &mut Incoming, pkt: &Packet) {
    let Some(reason) = read(
        ctx.stats,
        pkt,
        crate::play::pet::parse_pet_tame_failure(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::PetTameFailure(reason));
}

pub(super) fn pet_broken(ctx: &mut Incoming, _pkt: &Packet) {
    ctx.world.note_event(PlayerEvent::PetBroken);
}

pub(super) fn pet_name_invalid(ctx: &mut Incoming, _pkt: &Packet) {
    ctx.world.note_event(PlayerEvent::PetNameInvalid);
}

pub(super) fn pet_unlearn_confirm(ctx: &mut Incoming, pkt: &Packet) {
    let Some((pet, cost)) = read(
        ctx.stats,
        pkt,
        crate::play::pet::parse_pet_unlearn_confirm(&pkt.body),
    ) else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::PetUnlearnConfirm { pet, cost });
}

/// **The pet said something** — `SMSG_PET_ACTION_SOUND`, whose value selects
/// between two `CreatureSoundData` columns rather than naming a sound. An edge
/// like the feedback above it: two orders are two barks.
pub(super) fn pet_action_sound(ctx: &mut Incoming, pkt: &Packet) {
    let Some((pet, talk)) = read(
        ctx.stats,
        pkt,
        crate::play::pet::parse_pet_action_sound(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::PetTalk { pet, talk });
}

/// …and its goodbye — `SMSG_PET_DISMISS_SOUND`, the one pet packet about a
/// unit that no longer exists, which is why it carries a model id and a place.
pub(super) fn pet_dismiss_sound(ctx: &mut Incoming, pkt: &Packet) {
    let Some(sound) = read(
        ctx.stats,
        pkt,
        crate::play::pet::parse_pet_dismiss_sound(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::PetDismissSound(sound));
}

pub(super) fn inventory_failed(ctx: &mut Incoming, pkt: &Packet) {
    let Some(failure) = read(
        ctx.stats,
        pkt,
        crate::play::items::parse_inventory_change_failure(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::InventoryFailed(failure));
}

/// **`SMSG_ITEM_PUSH_RESULT`: something went into a bag** — and the only packet
/// that says so whatever brought it.
///
/// Everything else about the inventory is update fields, and a field says what
/// is true now rather than what happened: a stack that grew by three carries no
/// statement that it was looted rather than bought. `Player::SendNewItem` has
/// twenty call sites — loot, a vendor, a quest reward, mail, a trade, a craft,
/// a battleground mark, a GM command — and this is all of them.
///
/// **Broadcast to the group for a loot**, so the guid in the body is not
/// necessarily ours and the reader has to check. See
/// [`crate::play::items::ItemPush`].
pub(super) fn item_received(ctx: &mut Incoming, pkt: &Packet) {
    let Some(push) = read(
        ctx.stats,
        pkt,
        crate::play::items::parse_item_push_result(&pkt.body),
    ) else {
        return;
    };
    ctx.stats.items_received += 1;
    ctx.world.note_event(PlayerEvent::ItemReceived(push));
}

/// **`SMSG_TRANSFER_ABORTED`: the map change we asked for is not happening.**
///
/// One byte, and it is the only reply an instance portal ever gets that is not
/// the teleport itself — `HandleAreaTriggerOpcode` answers a trigger it declines
/// to act on with silence, so this arm and `SMSG_NEW_WORLD` are the two
/// outcomes. The reason is passed on raw; see
/// [`crate::play::areatrigger::TransferAbort`], where three of the six codes show
/// nothing at all and that is the client's own behaviour.
pub(super) fn transfer_aborted(ctx: &mut Incoming, pkt: &Packet) {
    let Some(reason) = read(
        ctx.stats,
        pkt,
        crate::play::areatrigger::parse_transfer_aborted(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::TransferAborted { reason });
}

/// **`SMSG_TRANSFER_PENDING`: the map change we are about to make.**
///
/// The one packet in this dispatch whose entire purpose is a *picture*. Nothing
/// is acknowledged, no state changes and no position arrives — the server sends
/// it from `Player::ExecuteTeleportFar` a few lines before it removes the
/// character from the old map, so that the client can put the loading screen up
/// before the world underneath it goes away.
///
/// This arm was deliberately absent for the life of the project, because this
/// client had no loading screen to raise. It has one now: `crates/client/src/game/loading.rs`.
pub(super) fn transfer_pending(ctx: &mut Incoming, pkt: &Packet) {
    let Some(pending) = read(
        ctx.stats,
        pkt,
        crate::state::movement::parse_transfer_pending(&pkt.body),
    ) else {
        return;
    };
    // **…and one thing that is not a picture**: whether this transfer began on
    // a boat. The two extra dwords are written only for a character standing on
    // a transport, and `SMSG_NEW_WORLD` carries no such mark — so this is the
    // only place the difference between *the boat is taking me with it* and
    // *something has taken me off the boat* crosses the wire. See
    // [`crate::state::movement::Mover::transfer_aboard`].
    if let Some(local) = ctx.local.as_deref_mut() {
        local.transfer_on_transport = pending.transport.is_some();
    }
    ctx.world.note_event(PlayerEvent::TransferPending {
        map_id: pending.map_id,
    });
}

/// **`SMSG_LOGIN_VERIFY_WORLD`: which map this login actually landed on.**
///
/// Sets [`LocalState::map_id`] and nothing else. The whole argument for the
/// arm is in [`crate::state::movement::LoginVerifyWorld`]: the map the
/// character list named can be stale by the time the login finishes, the server
/// corrects it inside `Player::LoadFromDB` with a bare `Relocate`, and this is
/// the only packet that carries the correction.
///
/// **Not acknowledged**, unlike `SMSG_NEW_WORLD`. There is no semaphore up: the
/// server is telling us where we already are rather than holding the session
/// until we say we have arrived, and a `MSG_MOVE_WORLDPORT_ACK` with nothing
/// pending is a packet the server has no state for.
///
/// **The position is deliberately not adopted.** It arrives before the
/// character's own create block, which states the same position along with the
/// speeds and the movement flags that have to agree with it — see
/// `SessionLoop::enter_world`, which resyncs the mover from that block. Two
/// sources for one position is a place for them to disagree, and this is the
/// one with less in it.
///
/// [`LocalState::map_id`]: crate::socket::handler::LocalState::map_id
pub(super) fn login_verify_world(ctx: &mut Incoming, pkt: &Packet) {
    let Some(verified) = read(
        ctx.stats,
        pkt,
        crate::state::movement::parse_login_verify_world(&pkt.body),
    ) else {
        return;
    };
    ctx.stats.landed_on_map = Some(verified.map_id);
    if let Some(local) = ctx.local.as_deref_mut() {
        local.map_id = verified.map_id;
    }
}

/// `SMSG_SPELL_FAILED_OTHER`: a cast in progress was interrupted.
///
/// Broadcast, so it arrives about anyone in sight; the reader decides whether
/// the guid is ours. vmangos never sends `SMSG_SPELL_FAILURE` at all.
pub(super) fn cast_interrupted(ctx: &mut Incoming, pkt: &Packet) {
    let Some((guid, spell_id)) = read(ctx.stats, pkt, spells::parse_spell_failed_other(&pkt.body))
    else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::CastInterrupted { guid, spell_id });
    // **Broadcast, so this one ends anybody's wind-up.** Before it, an
    // interrupted caster held the pose until `HOLD_GRACE_SECS` ran out — a
    // silenced mage stood with his hands up for a second after the cast had
    // stopped. The bar's own end is the `PlayerEvent` above, and it is ours
    // only; this is the *art*, and it is everyone's.
    ctx.world.apply_cast_cancelled(guid, spell_id);
}

/// `SMSG_SPELL_DELAYED`: our cast was pushed back by damage taken.
///
/// **The whole of "an interrupt adds time to the cast"**, and the client owns
/// none of the arithmetic: the server has already moved its own `m_timer` and
/// this says by how much. Both halves of the client have to be told, and they
/// are two different readers — the world's copy extends the wind-up pose and the
/// art hanging off the caster ([`ObjectManager::apply_cast_delayed`]), and the
/// queue's copy is what slides the cast bar, which is
/// `CastingBarFrame_OnEvent`'s own `SPELLCAST_DELAYED` arm.
pub(super) fn cast_delayed(ctx: &mut Incoming, pkt: &Packet) {
    let Some((guid, delay_ms)) = read(ctx.stats, pkt, spells::parse_spell_delayed(&pkt.body)) else {
        return;
    };
    ctx.world.apply_cast_delayed(guid, delay_ms);
    // Caster-only on the wire, but the guid is what says so rather than the
    // opcode: a pushback against a pet or any other unit has no bar of ours to
    // slide, and putting one on the player's would be a bar that moves for a
    // cast they are not making.
    if ctx.world.player_guid == Some(guid) {
        ctx.world.note_event(PlayerEvent::CastDelayed { delay_ms });
    }
}

/// **The quest family, one arm apiece** — see [`crate::play::quest`], which owns the
/// wire and the two things about it that are not guessable: the log's packed
/// six-bit counters and the `| 0x80000000` that makes an objective's target a
/// game object rather than a creature.
///
/// A macro because eleven of these are the same three lines and the alternative
/// is eleven copies of a `let Some(x) = read(…) else { return }` — the shape
/// this file already repeats twenty times, at the point where repeating it
/// again stops being clearer than naming it.
macro_rules! quest_arm {
    ($name:ident, $parse:path, $event:expr) => {
        pub(super) fn $name(ctx: &mut Incoming, pkt: &Packet) {
            let Some(parsed) = read(ctx.stats, pkt, $parse(&pkt.body)) else {
                return;
            };
            #[allow(clippy::redundant_closure_call)]
            ctx.world.note_event(($event)(parsed));
        }
    };
}

quest_arm!(quest_greeting, crate::play::quest::parse_quest_list, |page| {
    PlayerEvent::QuestGreeting(Box::new(page))
});
quest_arm!(quest_details, crate::play::quest::parse_quest_details, |page| {
    PlayerEvent::QuestDetails(Box::new(page))
});
quest_arm!(quest_reward, crate::play::quest::parse_offer_reward, |page| {
    PlayerEvent::QuestReward(Box::new(page))
});
quest_arm!(quest_progress, crate::play::quest::parse_request_items, |page| {
    PlayerEvent::QuestProgress(Box::new(page))
});
quest_arm!(
    quest_complete,
    crate::play::quest::parse_quest_complete,
    PlayerEvent::QuestComplete
);
/// `SMSG_QUEST_QUERY_RESPONSE`: one quest's template. The body is kept for the
/// on-disk cache, keyed by the quest id it begins with — see
/// [`crate::play::wdb`].
pub(super) fn quest_template(ctx: &mut Incoming, pkt: &Packet) {
    let Some(template) = read(
        ctx.stats,
        pkt,
        crate::play::quest::parse_quest_template(&pkt.body),
    ) else {
        return;
    };
    if let Some(key) = Kind::Quest.key_of(&pkt.body) {
        ctx.world.remember(Kind::Quest, key, &pkt.body);
    }
    ctx.world.note_event(PlayerEvent::QuestTemplate(Box::new(template)));
}
quest_arm!(
    quest_kill,
    crate::play::quest::parse_quest_kill,
    PlayerEvent::QuestKill
);
quest_arm!(quest_objectives_done, crate::play::quest::parse_quest_id, |id| {
    PlayerEvent::QuestObjectivesDone { quest_id: id }
});
quest_arm!(quest_refused, crate::play::quest::parse_quest_id, |reason| {
    PlayerEvent::QuestRefused { reason }
});

/// `SMSG_QUESTGIVER_STATUS`: **what is over this giver's head.**
///
/// Kept in the world as well as raised, because it is the one member of the
/// family that outlives its own arrival: the `!` stays there until the server
/// says otherwise, and the pass that draws it reads the world rather than
/// listening for an edge it may have been spawned after.
pub(super) fn quest_status(ctx: &mut Incoming, pkt: &Packet) {
    let Some((guid, status)) = read(
        ctx.stats,
        pkt,
        crate::play::quest::parse_questgiver_status(&pkt.body),
    ) else {
        return;
    };
    ctx.world.set_quest_status(guid, status);
    ctx.world.note_event(PlayerEvent::QuestStatus { guid, status });
}

/// `SMSG_QUESTUPDATE_FAILED` and `SMSG_QUESTUPDATE_FAILEDTIMER`.
///
/// Two opcodes, one body, and the difference is what the interface says about
/// it — which is why the flag is carried rather than the two being folded.
pub(super) fn quest_failed(ctx: &mut Incoming, pkt: &Packet, timed_out: bool) {
    let Some(quest_id) = read(ctx.stats, pkt, crate::play::quest::parse_quest_id(&pkt.body)) else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::QuestFailed { quest_id, timed_out });
}

/// **Talking to an NPC** — the gossip menu, its text, and the vendor window.
/// See [`crate::play::gossip`], which owns the wire and the icon-word table.
pub(super) fn gossip_show(ctx: &mut Incoming, pkt: &Packet) {
    let Some(menu) = read(ctx.stats, pkt, crate::play::gossip::parse_gossip_message(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GossipShow(Box::new(menu)));
}

/// `SMSG_GOSSIP_COMPLETE`: the server closed the window. The opcode is the
/// whole message.
pub(super) fn gossip_closed(ctx: &mut Incoming) {
    ctx.world.note_event(PlayerEvent::GossipClosed);
}

/// `SMSG_BINDPOINTUPDATE`: where the hearthstone returns the character to.
///
/// Arrives once in the login burst and again on every change, and nothing else
/// ever restates it — hence [`ObjectManager::bind_point`] rather than an event.
/// See [`crate::play::bindpoint`] for the whole subject.
pub(super) fn bind_point(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(point) = read(
        ctx.stats,
        pkt,
        crate::play::bindpoint::parse_bind_point(&pkt.body),
    ) {
        ctx.world.bind_point = Some(point);
    }
}

/// `SMSG_PLAYERBOUND`: who bound us, and to which area.
///
/// Raised as an event *and* folded into the stored bind point. The two packets
/// arrive together out of `Spell::EffectBind`, and there is no ordering rule
/// between them — so the area is taken from whichever lands first rather than
/// leaving a window in which the tooltip names the old home.
pub(super) fn player_bound(ctx: &mut Incoming, pkt: &Packet) {
    let Some((guid, area_id)) = read(
        ctx.stats,
        pkt,
        crate::play::bindpoint::parse_player_bound(&pkt.body),
    ) else {
        return;
    };
    if let Some(point) = ctx.world.bind_point.as_mut() {
        point.area_id = area_id;
    }
    ctx.world.note_event(PlayerEvent::PlayerBound { guid, area_id });
}

/// `SMSG_BINDER_CONFIRM`: an innkeeper asking to be made home.
///
/// **Not answered here.** The reply is a person pressing Accept on the
/// interface's own `CONFIRM_BINDER` popup, so the guid goes to the client and
/// comes back as `CMSG_BINDER_ACTIVATE` if it does. A handler that answered it
/// outright would bind the character to every inn they asked a question in.
pub(super) fn binder_confirm(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(guid) = read(
        ctx.stats,
        pkt,
        crate::play::bindpoint::parse_binder_confirm(&pkt.body),
    ) {
        ctx.world.note_event(PlayerEvent::BinderConfirm { guid });
    }
}

/// `SMSG_DUEL_REQUESTED`: a duel asked for, by us or at us.
///
/// Passed on whole: what the reference does with it depends on whether the
/// initiator is us, whether they are on the ignore list and whether they are
/// in view, and all three are the client's to answer. See
/// [`crate::play::duel`].
pub(super) fn duel_requested(ctx: &mut Incoming, pkt: &Packet) {
    if let Some((arbiter, initiator)) =
        read(ctx.stats, pkt, crate::play::duel::parse_duel_requested(&pkt.body))
    {
        ctx.world.note_event(PlayerEvent::DuelRequested { arbiter, initiator });
    }
}

/// `SMSG_DUEL_COUNTDOWN`: accepted, and starting in this many milliseconds.
pub(super) fn duel_countdown(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(ms) = read(ctx.stats, pkt, crate::play::duel::parse_duel_countdown(&pkt.body)) {
        ctx.world.note_event(PlayerEvent::DuelCountdown { ms });
    }
}

/// `SMSG_DUEL_OUTOFBOUNDS` and `SMSG_DUEL_INBOUNDS`: no body; the opcode is the
/// message. The server forfeits the duel ten seconds after the first unless
/// the second follows.
pub(super) fn duel_bounds(ctx: &mut Incoming, out: bool) {
    ctx.world.note_event(PlayerEvent::DuelBounds { out });
}

/// `SMSG_DUEL_COMPLETE`: over, one way or another.
pub(super) fn duel_complete(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(started) = read(ctx.stats, pkt, crate::play::duel::parse_duel_complete(&pkt.body)) {
        ctx.world.note_event(PlayerEvent::DuelComplete { started });
    }
}

/// `SMSG_DUEL_WINNER`: who won. Broadcast to everyone near the flag, so it
/// arrives for other people's duels too, and the reference prints it for them.
pub(super) fn duel_winner(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(winner) = read(ctx.stats, pkt, crate::play::duel::parse_duel_winner(&pkt.body)) {
        ctx.world.note_event(PlayerEvent::DuelWinner(winner));
    }
}

/// `SMSG_SUMMON_REQUEST`: somebody wants to bring us to them.
///
/// **The summoner's name is asked for here**, because it is the one thing the
/// popup says and the summoner is almost never in view — they are at a stone
/// or a ritual on the far side of the world. The reference reads its own name
/// cache and fills the gap when the query lands, via a callback, which is what
/// [`ObjectManager::want_social_guid`]'s query
/// pass does here.
pub(super) fn summon_request(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(request) = read(ctx.stats, pkt, crate::play::summon::parse_summon_request(&pkt.body)) {
        ctx.world.want_social_guid(request.summoner);
        ctx.world.note_event(PlayerEvent::SummonRequest(request));
    }
}

/// `SMSG_PLAYED_TIME`: `/played` answered.
pub(super) fn played_time(ctx: &mut Incoming, pkt: &Packet) {
    if let Some((total, level)) = read(ctx.stats, pkt, crate::play::played::parse_played_time(&pkt.body)) {
        ctx.world.note_event(PlayerEvent::PlayedTime { total, level });
    }
}

/// `SMSG_FISH_NOT_HOOKED` and `SMSG_FISH_ESCAPED`: no body.
pub(super) fn fish(ctx: &mut Incoming, escaped: bool) {
    ctx.world.note_event(PlayerEvent::Fish { escaped });
}

/// `SMSG_NPC_TEXT_UPDATE`: the words a gossip page's text id stood for.
pub(super) fn npc_text(ctx: &mut Incoming, pkt: &Packet) {
    let Some((text_id, text)) = read(ctx.stats, pkt, crate::play::gossip::parse_npc_text_update(&pkt.body))
    else {
        return;
    };
    ctx.world.remember(Kind::NpcText, u64::from(text_id), &pkt.body);
    ctx.world.note_event(PlayerEvent::NpcText { text_id, text });
}

/// `SMSG_LIST_INVENTORY`: the vendor window.
pub(super) fn vendor_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(vendor) = read(ctx.stats, pkt, crate::play::gossip::parse_list_inventory(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::VendorShow(Box::new(vendor)));
}

/// `SMSG_BUY_ITEM`: a purchase went through and a slot's stock moved.
pub(super) fn vendor_sold(ctx: &mut Incoming, pkt: &Packet) {
    let Some((guid, slot, left, _count)) =
        read(ctx.stats, pkt, crate::play::gossip::parse_buy_item(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::VendorSold { guid, slot, left });
}

/// `SMSG_BUY_FAILED`: …or did not, and why.
pub(super) fn buy_failed(ctx: &mut Incoming, pkt: &Packet) {
    let Some((_guid, entry, reason)) =
        read(ctx.stats, pkt, crate::play::gossip::parse_buy_failed(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::BuyFailed { entry, reason });
}

/// `SMSG_SELL_ITEM`: a sale refused — the success has no packet at all.
pub(super) fn sell_failed(ctx: &mut Incoming, pkt: &Packet) {
    let Some((_vendor, item, reason)) =
        read(ctx.stats, pkt, crate::play::gossip::parse_sell_failed(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::SellFailed { item, reason });
}

/// `SMSG_TRAINER_LIST`: **what this NPC will teach.** See [`crate::play::trainer`],
/// which owns the row layout and the three states.
pub(super) fn trainer_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(ctx.stats, pkt, crate::play::trainer::parse_trainer_list(&pkt.body)) else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::TrainerShow(Box::new(list)));
}

/// `SMSG_TRAINER_BUY_SUCCEEDED`: a service was learned.
///
/// **This is not how the spell reaches the spellbook** — that is
/// `SMSG_LEARNED_SPELL`, sent by the cast the server runs on our behalf. All
/// this says is which row of the open window is now grey, and the server
/// re-sends the whole list anyway; both are read, because the row has to
/// re-colour on the press rather than a round trip later.
pub(super) fn trainer_bought(ctx: &mut Incoming, pkt: &Packet) {
    let Some((_guid, spell)) = read(ctx.stats, pkt, crate::play::trainer::parse_trainer_bought(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::TrainerBought { spell });
}

/// `SMSG_TRAINER_BUY_FAILED`: …or was not, and why — a **`u32`** reason where
/// every other refusal in the game is a byte.
pub(super) fn trainer_buy_failed(ctx: &mut Incoming, pkt: &Packet) {
    let Some((_guid, spell, reason)) = read(
        ctx.stats,
        pkt,
        crate::play::trainer::parse_trainer_buy_failed(&pkt.body),
    ) else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::TrainerBuyFailed { spell, reason });
}

/// `MSG_LIST_STABLED_PETS`: **the stable window, whole.** See
/// [`crate::play::stable`], which owns the row layout and the slot convention.
///
/// The same opcode the client re-asks with, which is what `MSG_` means — the
/// direction is decided by who sent it and not by the name.
pub(super) fn stable_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(ctx.stats, pkt, crate::play::stable::parse_stabled_pets(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::StableList(Box::new(list)));
}

/// `SMSG_STABLE_RESULT`: **one byte, and it is the answer to all four verbs.**
///
/// Which verb it answers is not in the packet — a stable, an unstable, a swap
/// and a slot purchase all come back here, and only the success codes say which
/// of them happened. The window re-asks on any success rather than guessing;
/// see `client/src/game/npc/stable.rs`.
pub(super) fn stable_result(ctx: &mut Incoming, pkt: &Packet) {
    let Some((byte, result)) = read(ctx.stats, pkt, crate::play::stable::parse_stable_result(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::StableResult { byte, result });
}

/// `SMSG_SHOW_BANK`: **the bank window, which is a guid.** What is in the bank
/// is twenty-four guid fields that arrived with the backpack — see
/// [`crate::play::bank`] — so this packet opens the frame and names the
/// banker every verb will name again, and carries nothing else.
pub(super) fn bank_show(ctx: &mut Incoming, pkt: &Packet) {
    let Some(banker) = read(ctx.stats, pkt, crate::play::bank::parse_show_bank(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::BankShow(banker));
}

/// `SMSG_BUY_BANK_SLOT_RESULT`: **a refusal, and only ever a refusal.** The
/// success writes a byte of `PLAYER_BYTES_2` and sends nothing; see
/// [`crate::play::bank`].
pub(super) fn bank_slot_result(ctx: &mut Incoming, pkt: &Packet) {
    let Some((code, result)) = read(
        ctx.stats,
        pkt,
        crate::play::bank::parse_buy_bank_slot_result(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::BankSlotResult { code, result });
}

/// `SMSG_SHOWTAXINODES`: **the flight map.** See [`crate::play::taxi`], which owns
/// the wire, and `vale_assets::tables::taxi`, which owns everything the window then
/// makes of it.
pub(super) fn taxi_show(ctx: &mut Incoming, pkt: &Packet) {
    let Some(menu) = read(ctx.stats, pkt, crate::play::taxi::parse_show_taxi_nodes(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::TaxiShow(menu));
}

/// `SMSG_TAXINODE_STATUS`: is this master's own node one we know?
///
/// **Both an edge and a state**, which is the shape `SMSG_QUESTGIVER_STATUS`
/// already has and for the same reason: the green `!` over an undiscovered
/// flight master has to be drawable by a pass that was not listening when the
/// byte arrived, so the answer is kept on the world as well as announced.
pub(super) fn taxi_node_status(ctx: &mut Incoming, pkt: &Packet) {
    let Some((guid, known)) = read(
        ctx.stats,
        pkt,
        crate::play::taxi::parse_taxi_node_status(&pkt.body),
    ) else {
        return;
    };
    ctx.world.set_taxi_status(guid, known);
    ctx.world
        .note_event(PlayerEvent::TaxiNodeStatus { guid, known });
}

/// `SMSG_NEW_TAXI_PATH`: **a flight point discovered.** No body at all — the
/// packet is the whole message, and the mask that changed with it arrives on
/// the next `SMSG_SHOWTAXINODES` rather than here.
///
/// **Nothing is invalidated here either**, which is worth stating because it
/// looks like an omission: the green `!` this discovery takes down is keyed on
/// the master's guid, and `SendLearnNewTaxiNode` sends a fresh
/// `SMSG_TAXINODE_STATUS` carrying 1 for that very guid immediately after this
/// packet. The mark comes down because the server said so, not because the
/// client guessed which unit was meant.
pub(super) fn new_taxi_path(ctx: &mut Incoming) {
    ctx.world.note_event(PlayerEvent::NewTaxiPath);
}

/// `SMSG_ACTIVATETAXIREPLY`: the flight is happening, or one of twelve reasons
/// it is not.
pub(super) fn taxi_reply(ctx: &mut Incoming, pkt: &Packet) {
    let Some(reply) = read(
        ctx.stats,
        pkt,
        crate::play::taxi::parse_activate_taxi_reply(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::TaxiReply(reply));
}

/// `SMSG_LOOT_RESPONSE`: **the window on a body — or the refusal to open one.**
///
/// One opcode, two meanings, told apart by a loot type of zero. See
/// [`crate::play::loot`], which is where that reading is argued and where the two
/// numberings — the server's sparse index and the interface's dense row — are
/// crossed.
pub(super) fn loot_response(ctx: &mut Incoming, pkt: &Packet) {
    let Some(loot) = read(ctx.stats, pkt, crate::play::loot::parse_loot_response(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LootOpened(loot));
}

/// `SMSG_LOOT_RELEASE_RESPONSE`: **the server agrees the body is shut.**
///
/// What actually closes the window. `HandleLootReleaseOpcode` discards the guid
/// the client sends and releases whatever it last recorded, so closing on the
/// send is a window the server still thinks is open — and a body that will not
/// reopen.
pub(super) fn loot_released(ctx: &mut Incoming, pkt: &Packet) {
    let Some(guid) = read(
        ctx.stats,
        pkt,
        crate::play::loot::parse_loot_release_response(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LootClosed { guid });
}

/// `SMSG_LOOT_REMOVED`: one row is gone, by the **server's** index.
///
/// Sent to everyone with the window open, so it arrives for a row a group
/// member took as well as for one we took — which is why the window is amended
/// from this rather than from our own click.
pub(super) fn loot_removed(ctx: &mut Incoming, pkt: &Packet) {
    let Some(index) = read(ctx.stats, pkt, crate::play::loot::parse_loot_removed(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LootRemoved { index });
}

/// `SMSG_LOOT_START_ROLL`: **a group roll has opened on one row.**
///
/// The window on the body already holds that row as `RollOngoing`; this is what
/// puts a `GroupLootFrame` in front of the player. See
/// [`crate::play::lootroll`], and note the id the interface uses is the
/// client's own counter rather than anything in this packet.
pub(super) fn loot_roll_started(ctx: &mut Incoming, pkt: &Packet) {
    let Some(roll) = read(
        ctx.stats,
        pkt,
        crate::play::lootroll::parse_loot_start_roll(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LootRollStarted(roll));
}

/// `SMSG_LOOT_ROLL`: somebody chose, or somebody's dice landed.
///
/// **Both, on one packet.** The discriminator is the roll *number* and not the
/// type byte beside it — see [`crate::play::lootroll::RollLine::of`].
pub(super) fn loot_roll_cast(ctx: &mut Incoming, pkt: &Packet) {
    let Some(cast) = read(
        ctx.stats,
        pkt,
        crate::play::lootroll::parse_loot_roll(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LootRollCast(cast));
}

/// `SMSG_LOOT_ROLL_WON`: the item is in somebody's bags already.
///
/// It can name a roll that never started here — the result goes to everyone who
/// was eligible, and a member who joined after the roll opened was sent no
/// start. The reference has that branch too; see [`crate::play::lootroll`].
pub(super) fn loot_roll_won(ctx: &mut Incoming, pkt: &Packet) {
    let Some(won) = read(
        ctx.stats,
        pkt,
        crate::play::lootroll::parse_loot_roll_won(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LootRollWon(won));
}

/// `SMSG_LOOT_ALL_PASSED`: **nobody wanted it, and the row is clickable again.**
///
/// The second half of that sentence is the client's own: no packet ever says a
/// blocked row has been unblocked. See [`crate::play::loot::Loot::unblock`].
pub(super) fn loot_roll_all_passed(ctx: &mut Incoming, pkt: &Packet) {
    let Some(passed) = read(
        ctx.stats,
        pkt,
        crate::play::lootroll::parse_loot_all_passed(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LootRollAllPassed(passed));
}

/// `SMSG_LOOT_CLEAR_MONEY`: the coins are gone from the window. No body at all.
pub(super) fn loot_money_cleared(ctx: &mut Incoming) {
    ctx.world.note_event(PlayerEvent::LootMoneyCleared);
}

/// `SMSG_LOOT_MONEY_NOTIFY`: **your share**, which in a group is not what was on
/// the body. A different statement from the clear above; see [`crate::play::loot`].
pub(super) fn loot_money_gained(ctx: &mut Incoming, pkt: &Packet) {
    let Some(copper) = read(
        ctx.stats,
        pkt,
        crate::play::loot::parse_loot_money_notify(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LootMoneyGained { copper });
}

/// `SMSG_START_MIRROR_TIMER`: **a bar the server is counting for us.**
///
/// The breath meter, and the two beside it. Nothing here decides anything: the
/// packet carries a remaining, a length and a *rate*, and the interface
/// interpolates between statements — see [`crate::play::timers`], which is the whole
/// of the family, including why the third of these packets is one the shipped
/// handler cannot act on.
///
/// A start and not an update: the server resends this to say *paused* as well,
/// because its own pause event is broken, so two arrivals with identical fields
/// are two statements about the bar.
pub(super) fn mirror_timer_start(ctx: &mut Incoming, pkt: &Packet) {
    let Some(start) = read(ctx.stats, pkt, timers::parse_start_mirror_timer(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::MirrorTimerStarted(start));
}

/// `SMSG_STOP_MIRROR_TIMER`: that bar goes away. One `u32` and no more.
pub(super) fn mirror_timer_stop(ctx: &mut Incoming, pkt: &Packet) {
    let Some(timer) = read(ctx.stats, pkt, timers::parse_stop_mirror_timer(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::MirrorTimerStopped { timer });
}

/// `SMSG_PAUSE_MIRROR_TIMER`: it freezes where it stands.
///
/// **The reference server never sends this** and the reference interface could
/// not use it if it did — `MirrorTimerFrame_OnEvent` reads `arg1` as both the
/// timer's name and the paused flag. Read and raised anyway, faithfully, so
/// that a server which does send it is not silently ignored.
pub(super) fn mirror_timer_pause(ctx: &mut Incoming, pkt: &Packet) {
    let Some((timer, paused)) = read(ctx.stats, pkt, timers::parse_pause_mirror_timer(&pkt.body))
    else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::MirrorTimerPaused { timer, paused });
}

/// `SMSG_SPELL_COOLDOWN`: the server's **override** path, not the ordinary one.
///
/// A plain cast's own recovery is computed by the client from `Spell.dbc` and
/// started when its `SMSG_SPELL_GO` returns — vmangos sends no packet for it.
/// This is a school lockout, a pet's list, or a GM reset, and it may name
/// several spells at once.
pub(super) fn spell_cooldown(ctx: &mut Incoming, pkt: &Packet) {
    let Some(cooldowns) = read(ctx.stats, pkt, spells::parse_spell_cooldowns(&pkt.body)) else {
        return;
    };
    // About a pet or another unit: this client has one cooldown store and it is
    // the player's, so anything else is dropped rather than applied to it.
    if ctx.world.player_guid != Some(cooldowns.guid) {
        return;
    }
    for (spell_id, ms) in cooldowns.entries {
        ctx.world
            .note_event(PlayerEvent::CooldownStarted { spell_id, ms });
    }
}

/// `SMSG_CLEAR_COOLDOWN`: **the cooldown is over now.**
///
/// The counterpart to [`spell_cooldown`] above, and the one this client read
/// nothing for: a school lockout lifted early, any cooldown removal, the
/// warlock ritual fix-up. Without it the swirl runs to the length the *start*
/// packet stated, which is right whenever the lockout runs its course and wrong
/// whenever it does not.
///
/// **The guid test is doing real work here**, not just filtering pets: it is
/// what makes the layout reading in [`spells::parse_clear_cooldown`] check
/// itself. See there.
pub(super) fn clear_cooldown(ctx: &mut Incoming, pkt: &Packet) {
    let Some((spell_id, guid)) = read(ctx.stats, pkt, spells::parse_clear_cooldown(&pkt.body))
    else {
        return;
    };
    if ctx.world.player_guid != Some(guid) {
        return;
    }
    ctx.world
        .note_event(PlayerEvent::CooldownCleared { spell_id });
}

/// `SMSG_COOLDOWN_EVENT`: release a cooldown that was parked.
///
/// `SPELL_ATTR_COOLDOWN_ON_EVENT` — Stealth and Feign Death take their recovery
/// when they *break* rather than when they are cast, so the client inserts the
/// record on hold and this starts its clock.
pub(super) fn cooldown_event(ctx: &mut Incoming, pkt: &Packet) {
    let Some((spell_id, guid)) = read(ctx.stats, pkt, spells::parse_cooldown_event(&pkt.body))
    else {
        return;
    };
    if ctx.world.player_guid != Some(guid) {
        return;
    }
    ctx.world
        .note_event(PlayerEvent::CooldownReleased { spell_id });
}

/// `SMSG_ATTACKSTART` and `SMSG_ATTACKSTOP`: a unit has begun or stopped
/// swinging at another.
///
/// **The two disagree about guid packing** — start streams two plain ones and
/// stop two packed ones, one function apart in `Unit.cpp`. Broadcast about
/// everybody; only our own reaches the state, see the module comment.
pub(super) fn attack_state(ctx: &mut Incoming, pkt: &Packet, starting: bool) {
    let parsed = if starting {
        spells::parse_attack_start(&pkt.body)
    } else {
        spells::parse_attack_stop(&pkt.body)
    };
    let Some((attacker, victim)) = read(ctx.stats, pkt, parsed) else {
        return;
    };
    ctx.world
        .apply_attack_state(attacker, starting.then_some(victim));
}

/// `SMSG_CANCEL_AUTO_REPEAT`: the ranged loop has stopped.
///
/// **No body and no guid** — it is sent to the one player it is about
/// (`Player::SendAutoRepeatCancel`), so there is nothing to check and nothing
/// to read. See [`PlayerEvent::AutoRepeatCancelled`] for why it has more
/// senders than the player's own press, and why that is the reason it has to be
/// read at all: every server-side end of the loop (the target dies, walks out
/// of range, a wand-user moves) comes through here and nowhere else.
pub(super) fn auto_repeat_cancelled(ctx: &mut Incoming) {
    ctx.world.note_event(PlayerEvent::AutoRepeatCancelled);
}

/// `MSG_CHANNEL_START`: a channel has begun, and for how long.
///
/// **The one packet that says how long to hold a channel's pose.** Everything
/// else about a channelled spell looks like an instant one from the wire —
/// `SMSG_SPELL_START` is not sent and `SMSG_SPELL_GO` fires immediately — so an
/// Evocation was drawn as a release with nothing after it. See
/// [`ObjectManager::apply_channel_start`], which folds it into the cast
/// counters because from the animation's side that is what a channel is.
pub(super) fn channel_start(ctx: &mut Incoming, pkt: &Packet) {
    let Some((spell_id, duration_ms)) = read(ctx.stats, pkt, spells::parse_channel_start(&pkt.body))
    else {
        return;
    };
    ctx.world.apply_channel_start(spell_id, duration_ms);
    // **…and the same news on the interface's own queue.** The world's copy
    // above drives the held pose; this is what a cast bar has to hear, and there
    // is no other packet that would tell it — see [`PlayerEvent::ChannelStart`].
    ctx.world
        .note_event(spells::PlayerEvent::ChannelStart {
            spell_id,
            duration_ms,
        });
}

/// `MSG_CHANNEL_UPDATE`: how much of it is left, and zero for "it is over".
pub(super) fn channel_update(ctx: &mut Incoming, pkt: &Packet) {
    let Some(remaining_ms) = read(ctx.stats, pkt, spells::parse_channel_update(&pkt.body)) else {
        return;
    };
    ctx.world.apply_channel_update(remaining_ms);
    ctx.world
        .note_event(spells::PlayerEvent::ChannelUpdate { remaining_ms });
}

/// `SMSG_UPDATE_AURA_DURATION`: how long the buff in this slot has left.
///
/// **Ours by construction** — the server sends it only to the aura's own target
/// and only when that target is a player — so there is no guid to check and
/// nothing to filter. What a mis-slotted reading costs is a timer under the
/// wrong icon, which is why the join is the reader's and is gated there; see
/// [`ObjectManager::note_aura_duration`].
pub(super) fn aura_duration(ctx: &mut Incoming, pkt: &Packet) {
    let Some((slot, remaining_ms)) = read(ctx.stats, pkt, spells::parse_aura_duration(&pkt.body))
    else {
        return;
    };
    ctx.world.note_aura_duration(slot, remaining_ms);
}

/// `SMSG_LEVELUP_INFO`: we gained a level.
///
/// The one packet that says a level-up *happened*, and the eleven deltas it
/// carries beside the level — all of which the interface's own handler reads.
/// See [`spells::parse_levelup`].
pub(super) fn levelup(ctx: &mut Incoming, pkt: &Packet) {
    let Some(gained) = read(ctx.stats, pkt, spells::parse_levelup(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LevelUp(gained));
}

/// `SMSG_CORPSE_RECLAIM_DELAY`: how long the body must lie there.
///
/// **Said once, at the moment of release, and never restated** — so a client
/// that drops it has no way to ask again, and offers a Retrieve button the
/// server will silently ignore for the next thirty seconds.
pub(super) fn corpse_reclaim_delay(ctx: &mut Incoming, pkt: &Packet) {
    let Some(ms) = read(ctx.stats, pkt, crate::play::death::parse_corpse_reclaim_delay(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::CorpseReclaimDelay { ms });
}

/// `MSG_CORPSE_QUERY`, the **reply** — the same opcode we asked with.
///
/// A found corpse and no corpse are both valid answers and only the first has a
/// body past its flag byte; see [`crate::play::death::parse_corpse_query`], which is
/// why this is `Option<Option<_>>` all the way through rather than collapsing
/// "no corpse" into "would not parse".
pub(super) fn corpse_located(ctx: &mut Incoming, pkt: &Packet) {
    let Some(place) = read(ctx.stats, pkt, crate::play::death::parse_corpse_query(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::CorpseLocated(place));
}

/// `SMSG_RESURRECT_REQUEST`: somebody has offered to bring us back.
pub(super) fn resurrect_offered(ctx: &mut Incoming, pkt: &Packet) {
    let Some(offer) = read(ctx.stats, pkt, crate::play::death::parse_resurrect_request(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::ResurrectOffered(offer));
}

/// `SMSG_SPIRIT_HEALER_CONFIRM`: the healer's offer, whose body is its guid.
pub(super) fn spirit_healer_offered(ctx: &mut Incoming, pkt: &Packet) {
    let Some(healer) = read(
        ctx.stats,
        pkt,
        crate::play::death::parse_spirit_healer_confirm(&pkt.body),
    ) else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::SpiritHealerOffered { healer });
}

/// `SMSG_LOGOUT_RESPONSE`: what the server made of `CMSG_LOGOUT_REQUEST`.
///
/// **The only one of the four logout packets with a body**, and the only one
/// that can say no — see [`crate::play::logout`], where the three `reason` values and
/// the instant flag come from. A body that will not parse is reported rather
/// than taken as an acceptance: five bytes read as a zero reason would put the
/// character screen up over a character still standing in the world.
pub(super) fn logout_response(ctx: &mut Incoming, pkt: &Packet) {
    let Some(logout) = read(ctx.stats, pkt, crate::play::logout::parse_logout_response(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::Logout(logout));
}

/// `SMSG_LOGOUT_COMPLETE` and `SMSG_LOGOUT_CANCEL_ACK`: **no body on either**,
/// so the opcode is the whole message — the same shape as the five attack
/// refusals below.
pub(super) fn logout_state(ctx: &mut Incoming, state: crate::play::logout::Logout) {
    ctx.world.note_event(PlayerEvent::Logout(state));
}

/// The five `SMSG_ATTACKSWING_*` refusals.
///
/// **The opcode is the whole message**; there is no body on any of them. Each
/// names a `GlobalStrings.lua` key rather than a string this client made up —
/// "You are too far away!" and "You are facing the wrong way!" are the game's
/// own words for the two that a player meets constantly.
pub(super) fn attack_refused(ctx: &mut Incoming, refusal: spells::AttackRefusal) {
    ctx.stats.attack_refusals += 1;
    ctx.world.note_event(PlayerEvent::AttackRefused(refusal));
}

/// Which of the two spell-change opcodes this is, for the dispatch's guard.
pub(super) fn is_spell_change(op: Opcode) -> Option<bool> {
    match op {
        Opcode::SMSG_LEARNED_SPELL => Some(true),
        Opcode::SMSG_REMOVED_SPELL => Some(false),
        _ => None,
    }
}

// --- the party ------------------------------------------------------------
//
// See [`crate::play::group`]. Six edges and one piece of state; the state is
// `SMSG_GROUP_LIST` and it arrives whole every time.

/// `SMSG_GROUP_INVITE`: **somebody wants us in their party.** The body is their
/// name and nothing else, and what it costs when it is dropped is the whole of
/// joining a group: there is no second announcement and no way to ask.
pub(super) fn group_invite(ctx: &mut Incoming, pkt: &Packet) {
    let Some(name) = read(ctx.stats, pkt, crate::play::group::parse_name(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GroupInvite { name });
}

/// `SMSG_GROUP_DECLINE`: somebody we asked said no. **Only the inviter is
/// told**, so this reaches exactly one client and leaves no other trace.
pub(super) fn group_decline(ctx: &mut Incoming, pkt: &Packet) {
    let Some(name) = read(ctx.stats, pkt, crate::play::group::parse_name(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GroupDecline { name });
}

/// `SMSG_GROUP_LIST`: **the roster, and there is no incremental form of it.**
///
/// Every join, leave, promotion and loot-rule change re-sends the whole thing,
/// and the copy each member gets leaves *them* out of it — so this list is
/// `party1..N` directly.
pub(super) fn group_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(ctx.stats, pkt, crate::play::group::parse_group_list(&pkt.body)) else {
        return;
    };
    // **The name query is asked from here rather than from the entity walk**,
    // because a member out of range has no entity — and their *class* is what a
    // raid button is coloured by. See `ObjectManager::group_guids`.
    ctx.world
        .note_group(list.members.iter().map(|member| member.guid).collect());
    ctx.world.note_event(PlayerEvent::GroupList(Box::new(list)));
}

/// `SMSG_GROUP_DESTROYED`: the party is over. **No body**, and it is the only
/// thing that says so — the server never sends a `SMSG_GROUP_LIST` with an empty
/// roster, so a client reading only that one leaves a party frame standing.
pub(super) fn group_destroyed(ctx: &mut Incoming) {
    ctx.world.note_group(Vec::new());
    ctx.world.note_event(PlayerEvent::GroupDestroyed);
}

/// `SMSG_GROUP_SET_LEADER`: who leads now — **by name**, where
/// `CMSG_GROUP_SET_LEADER` asked by guid.
pub(super) fn group_new_leader(ctx: &mut Incoming, pkt: &Packet) {
    let Some(name) = read(ctx.stats, pkt, crate::play::group::parse_name(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GroupNewLeader { name });
}

/// `SMSG_PARTY_COMMAND_RESULT`: what came of an invite or a leave.
///
/// **Sent on success as well as failure**, which is what makes it the only
/// acknowledgement `/invite` has: the invitee's own client gets the invitation
/// and ours gets this.
pub(super) fn party_result(ctx: &mut Incoming, pkt: &Packet) {
    let Some(result) = read(
        ctx.stats,
        pkt,
        crate::play::group::parse_party_command_result(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::PartyResult(result));
}

/// `SMSG_PARTY_MEMBER_STATS` and `SMSG_PARTY_MEMBER_STATS_FULL`: **one body,
/// two opcodes**, told apart only by whether the server sent everything or only
/// what changed — which the mask already says.
///
/// This is the only thing that knows a party member's health when they are out
/// of the object manager's range, which for a party spread across a zone is most
/// of the time.
pub(super) fn party_member_stats(ctx: &mut Incoming, pkt: &Packet) {
    let Some(stats) = read(
        ctx.stats,
        pkt,
        crate::play::group::parse_party_member_stats(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::PartyMemberStats(stats));
}

/// `MSG_RAID_READY_CHECK`: **the same opcode in three directions**, and the
/// length is the only thing that tells them apart.
///
/// An empty body is the leader starting one and is by far the commonest form —
/// a reader that treats a zero-length packet as damaged never shows the box.
/// See [`crate::play::group::ReadyCheck`].
pub(super) fn raid_ready_check(ctx: &mut Incoming, pkt: &Packet) {
    let Some(check) = read(
        ctx.stats,
        pkt,
        crate::play::group::parse_ready_check(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::RaidReadyCheck(check));
}

// --- who you know ---------------------------------------------------------
//
// See [`crate::play::social`]. Two lists that arrive once and are patched by a
// third packet, and one search reply — and **not one of the four carries a
// name for anybody on the two lists**, which is why the two list arms mark
// their guids wanted here rather than leaving it to the entity walk.

/// `SMSG_FRIEND_LIST`: **the whole friends list, and it arrives once.**
///
/// Every later change is an `SMSG_FRIEND_STATUS` about one player, so a client
/// that reads this and not that shows the list it logged in with for the rest of
/// the session.
pub(super) fn friend_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(
        ctx.stats,
        pkt,
        crate::play::social::parse_friend_list(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_social_friends(list.iter().map(|f| f.guid).collect());
    ctx.world.note_event(PlayerEvent::FriendList(list));
}

/// `SMSG_IGNORE_LIST`: the same, for the ignore list — guids and nothing else.
pub(super) fn ignore_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(
        ctx.stats,
        pkt,
        crate::play::social::parse_ignore_list(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_social_ignores(list.clone());
    ctx.world.note_event(PlayerEvent::IgnoreList(list));
}

/// `SMSG_FRIEND_STATUS`: one answer about one player.
///
/// **Both a refusal and a change arrive here**, which is why the event carries
/// the result byte rather than a patched list: "already your friend" and "your
/// friend just logged in" are the same packet with a different first byte, and
/// only one of them touches the list.
pub(super) fn friend_status(ctx: &mut Incoming, pkt: &Packet) {
    let Some(status) = read(
        ctx.stats,
        pkt,
        crate::play::social::parse_friend_status(&pkt.body),
    ) else {
        return;
    };
    // A new friend or a new ignore is a guid nothing has ever named, and the
    // panel draws the name — so ask on the tick it arrives rather than on
    // whichever later one something else happens to want it.
    if !status.result.removes() {
        ctx.world.want_social_guid(status.guid);
    }
    ctx.world.note_event(PlayerEvent::FriendStatus(status));
}

/// `SMSG_WHO`: the search's answer. Names, not guids — see
/// [`crate::play::social`].
pub(super) fn who_results(ctx: &mut Incoming, pkt: &Packet) {
    let Some(results) = read(ctx.stats, pkt, crate::play::social::parse_who(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::WhoResults(results));
}

/// `SMSG_INITIALIZE_FACTIONS`: the whole standing table, at login.
///
/// Sixty-four slots whether or not the character has met them, so this replaces
/// rather than merges — the same statement `SMSG_INITIAL_SPELLS` makes about the
/// spellbook, and for the same reason: the server sends it again after anything
/// that could have changed the set.
pub(super) fn initialize_factions(ctx: &mut Incoming, pkt: &Packet) {
    let Some(states) = read(
        ctx.stats,
        pkt,
        crate::play::reputation::parse_initialize_factions(&pkt.body),
    ) else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::FactionsInitialized(Box::new(states)));
}

/// `SMSG_SET_FACTION_STANDING`: one or more slots moved.
pub(super) fn faction_standing(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(
        ctx.stats,
        pkt,
        crate::play::reputation::parse_set_faction_standing(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::FactionStandings(list));
}

/// `SMSG_SET_FACTION_VISIBLE`: a faction met for the first time.
///
/// vmangos suppresses this while the character is loading, so it is always the
/// mid-session case and always a *new row* rather than a changed one.
pub(super) fn faction_visible(ctx: &mut Incoming, pkt: &Packet) {
    let Some(reputation_list_id) = read(
        ctx.stats,
        pkt,
        crate::play::reputation::parse_set_faction_visible(&pkt.body),
    ) else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::FactionVisible { reputation_list_id });
}

/// `SMSG_SET_FACTION_ATWAR`: the crossed-swords box, from the server's side.
pub(super) fn faction_at_war(ctx: &mut Incoming, pkt: &Packet) {
    let Some((reputation_list_id, flags)) = read(
        ctx.stats,
        pkt,
        crate::play::reputation::parse_set_faction_at_war(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::FactionAtWar {
        reputation_list_id,
        flags,
    });
}

/// `SMSG_GAMEOBJECT_PAGETEXT`: **this thing has something written on it.**
///
/// One guid and nothing else — the page id is in the template the client
/// queried when the object came into view, which is what makes this packet a
/// *nudge* rather than a statement. See [`crate::play::pagetext`], which says
/// why a sign gets no packet at all and reaches the same window anyway.
pub(super) fn gameobject_pagetext(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(guid) = read(
        ctx.stats,
        pkt,
        crate::play::pagetext::parse_gameobject_pagetext(&pkt.body),
    ) {
        ctx.world.note_event(PlayerEvent::GameObjectPageText { guid });
    }
}

/// `SMSG_PAGE_TEXT_QUERY_RESPONSE`: **one page of it**, and the id of the next.
///
/// The server answers the whole chain to one query — `HandlePageTextQueryOpcode`
/// loops until `next_page` is zero — so these arrive in a burst and the reader
/// assembles them by id rather than by arrival.
pub(super) fn page_text(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(page) = read(ctx.stats, pkt, crate::play::pagetext::parse_page_text(&pkt.body)) {
        ctx.world.remember(Kind::PageText, u64::from(page.id), &pkt.body);
        ctx.world.note_event(PlayerEvent::PageText(page));
    }
}

/// `SMSG_GOSSIP_POI`: **the flag a guard's directions put on the map**.
///
/// The only packet in the game that marks a place, and the client keeps one —
/// see [`crate::play::gossip::parse_gossip_poi`], which says why a second
/// replaces the first rather than joining it.
pub(super) fn gossip_poi(ctx: &mut Incoming, pkt: &Packet) {
    let Some((flags, x, y, icon, data, name)) = read(
        ctx.stats,
        pkt,
        crate::play::gossip::parse_gossip_poi(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GossipPoi {
        flags,
        position: (x, y),
        icon,
        data,
        name,
    });
}

/// `SMSG_SET_FORCED_REACTIONS`: **the server overriding friend-or-foe outright**,
/// and the one thing that can do it without changing a field on any unit.
///
/// Unhandled, a `SPELL_AURA_FORCE_REACTION` is invisible to this client: the
/// faction table still says friendly, the name plate stays green, the unit
/// cannot be clicked into combat — and the server, which is answering off its
/// own copy of the map, damages it and is attacked back by it. That
/// disagreement is what the aura exists to state.
pub(super) fn forced_reactions(ctx: &mut Incoming, pkt: &Packet) {
    let Some(reactions) = read(
        ctx.stats,
        pkt,
        crate::play::reputation::parse_forced_reactions(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::ForcedReactions(reactions));
}

/// `SMSG_EXPLORATION_EXPERIENCE`: **a place was discovered, and this is the only
/// announcement it gets.**
///
/// `PLAYER_EXPLORED_ZONES` is a `PRIVATE` update field with no event of its own,
/// so without this the map fills in silently — and, because the interface only
/// re-reads the overlays on `WORLD_MAP_UPDATE`, a zone map opened before the
/// bit arrived keeps showing bare paper until something else moves the view.
/// That is the "does not update until relog" half of the report.
pub(super) fn discovered(ctx: &mut Incoming, pkt: &Packet) {
    let parsed = {
        let mut r = crate::bytes::Reader::new(&pkt.body);
        (r.remaining() >= 8).then(|| (r.u32(), r.u32()))
    };
    let Some((area, experience)) = read(ctx.stats, pkt, parsed) else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::Discovered { area, experience });
}

// --- the box on the corner ---------------------------------------------------

/// `SMSG_MAIL_LIST_RESULT`: **the inbox, whole.** See [`crate::play::mail`],
/// which owns the header layout and the union in its sender field.
///
/// It replaces whatever was held rather than patching it, because that is what
/// the packet is: there is no per-letter update on the wire, and every one of
/// the six verbs is answered by a bare result code that names an id and nothing
/// else. A client that patched its own copy from those codes would drift from
/// the server the first time anything else touched the mailbox.
/// `SMSG_TRADE_STATUS`: **the trade's state machine**, one code at a time.
/// See [`crate::play::trade`]. The window is `client/src/game/session/trade.rs`.
pub(super) fn trade_status(ctx: &mut Incoming, pkt: &Packet) {
    let Some(status) = read(ctx.stats, pkt, crate::play::trade::parse_trade_status(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::TradeStatus(status));
}

/// `SMSG_TRADE_STATUS_EXTENDED`: **one side's offer, whole.** The items are
/// entries and the panel wants names, so every entry is asked for here the way
/// a vendor's are — `want_item` puts it on the query list and the template
/// lands a round trip later, which is what the window's own refresh waits on.
pub(super) fn trade_offer(ctx: &mut Incoming, pkt: &Packet) {
    let Some(offer) = read(
        ctx.stats,
        pkt,
        crate::play::trade::parse_trade_status_extended(&pkt.body),
    ) else {
        return;
    };
    for item in offer.items.iter().flatten() {
        ctx.world.want_item(item.entry);
    }
    ctx.world.note_event(PlayerEvent::TradeOffer(Box::new(offer)));
}

/// **One item class's whole proficiency mask.** Without it every item in the
/// bags draws as usable, which is a character told they may equip a weapon they
/// cannot and finding out from a refusal a round trip later.
pub(super) fn proficiency(ctx: &mut Incoming, pkt: &Packet) {
    let Some(said) = read(ctx.stats, pkt, crate::play::skills::parse_proficiency(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::Proficiency(said));
}

/// **One modifier bit's running total.** A talent that shortens a cast reaches
/// the client here and nowhere else; dropped, the cast bar and the tooltip print
/// the untalented number for the whole of a session.
pub(super) fn spell_modifier(ctx: &mut Incoming, pkt: &Packet, percent: bool) {
    let Some(modifier) = read(
        ctx.stats,
        pkt,
        crate::play::spells::parse_spell_modifier(&pkt.body, percent),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::SpellModifier(modifier));
}

/// **An enchant landed, or one faded.** The item's own fields move in an update
/// block with no announcement, so this is the only thing that can put a line in
/// the log.
pub(super) fn enchantment_log(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, crate::play::items::parse_enchantment_log(&pkt.body))
    else {
        return;
    };
    // The line names the item, and the name is a template this session may not
    // have asked for yet.
    ctx.world.want_item(log.item_entry);
    ctx.world.note_combat(crate::play::combatlog::CombatEvent::Enchantment(log));
}

pub(super) fn mail_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(ctx.stats, pkt, crate::play::mail::parse_mail_list(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::MailList(Box::new(list)));
}

/// `SMSG_SEND_MAIL_RESULT`: **the one answer seven verbs share**, and its tail
/// is conditional on both of its words — see
/// [`crate::play::mail::parse_mail_result`].
pub(super) fn mail_result(ctx: &mut Incoming, pkt: &Packet) {
    let Some(response) = read(ctx.stats, pkt, crate::play::mail::parse_mail_result(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::MailResult(response));
}

/// `SMSG_RECEIVED_MAIL`: a letter has been delivered.
///
/// The body is one word and vmangos always writes zero, so the arrival is the
/// whole message. What it is *for* is the envelope on the minimap, which the
/// client turns on by asking `MSG_QUERY_NEXT_MAIL_TIME` again.
pub(super) fn mail_received(ctx: &mut Incoming) {
    ctx.world.note_event(PlayerEvent::MailReceived);
}

/// `MSG_QUERY_NEXT_MAIL_TIME`: **seconds until the next letter, and 0 means one
/// is already waiting.**
///
/// The sign is the answer and it reads backwards — see
/// [`crate::play::mail::parse_next_mail_time`], where the client's own
/// `abs(x) <= epsilon` test is.
pub(super) fn mail_next_time(ctx: &mut Incoming, pkt: &Packet) {
    let Some(seconds) = read(
        ctx.stats,
        pkt,
        crate::play::mail::parse_next_mail_time(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::MailNextTime(seconds));
}

/// `SMSG_ITEM_TEXT_QUERY_RESPONSE`: **the words of one letter.**
///
/// Named for items rather than for mail because the same query answers for a
/// book or a signpost's page; in 1.12 the only sender of it here is an open
/// letter, and the id it echoes is `MailHeader::item_text_id`.
pub(super) fn item_text(ctx: &mut Incoming, pkt: &Packet) {
    let Some((id, text)) = read(ctx.stats, pkt, crate::play::mail::parse_item_text(&pkt.body))
    else {
        return;
    };
    ctx.world.remember(Kind::ItemText, u64::from(id), &pkt.body);
    ctx.world.note_event(PlayerEvent::ItemText { id, text });
}

/// `SMSG_QUESTUPDATE_ADD_ITEM`: **an item objective moved.**
///
/// Two words, an entry and an increment, and **no quest id** — see
/// [`crate::play::quest::parse_quest_item`]. It was named and undispatched for
/// several rounds, which is the whole of "the tracker updates silently": an
/// item objective's counter comes off the bags, so the row moved and the line
/// that says so never fired.
pub(super) fn quest_item(ctx: &mut Incoming, pkt: &Packet) {
    let Some((entry, added)) = read(ctx.stats, pkt, crate::play::quest::parse_quest_item(&pkt.body))
    else {
        return;
    };
    // **The bag count is read here and carried, not looked up later.**
    //
    // The line the reference shows is `min(bags + added, required)`, and the
    // addition is the whole point: the packet arrives
    // *before* the item lands in the bags, so the count on its own is one
    // behind on every pickup. That arithmetic is only right if `bags` is the
    // count **at the instant this packet was handled** — which is here, on the
    // session thread, ahead of whatever object update the same server tick
    // sends. Read a frame later off the renderer's copy of the inventory it is
    // a coin toss whether the item has landed yet, and the two outcomes are
    // "6/8" twice running and then "8/8" for the seventh — which is exactly how
    // it was reported.
    let have = crate::play::items::Inventory::read(ctx.world).count_of(entry);
    ctx.world
        .note_event(PlayerEvent::QuestItem { entry, added, have });
}
