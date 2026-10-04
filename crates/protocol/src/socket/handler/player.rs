//! Packet handlers for the local character: its spells, action bar, casts,
//! items, pet, quests, NPC windows, group, social lists, mail and timers.
//!
//! There is one `pub(super) fn` per arm of [`super::apply_packet`]'s match, the
//! same layout as [`super::world`]. That module's comment explains why the
//! dispatch match stays in one place and only the bodies live here.
//!
//! The split against `world` is by whose packet it is. Packets in `world` are
//! about any unit in view: a swing, an emote, a cast, a spline. Packets here are
//! about the local character: the spellbook, the action bar, a cast result that
//! answers our `CMSG_CAST_SPELL`, and the five attack-swing refusals that answer
//! our `CMSG_ATTACKSWING`. The two attack-state packets are an exception. They
//! are broadcast about every unit, and
//! [`crate::state::objects::ObjectManager::apply_attack_state`] discards every
//! one that is not about the local character, because the combat flag on each
//! unit already covers the others.
//!
//! A dropped packet here produces no visible error. A dropped cast result is a
//! spell that does nothing; a dropped spellbook is an empty action bar. Both look
//! like missing features rather than bugs. For that reason every handler parses
//! through [`super::read`], which reports a body that will not parse to the HUD
//! as a warning.

use super::{read, Incoming};
use crate::opcodes::Opcode;
use crate::play::spells::{self, PlayerEvent};
use crate::play::timers;
use crate::play::wdb::Kind;
use crate::socket::world::Packet;

/// `SMSG_INITIAL_SPELLS`: every spell this character knows.
///
/// Sent once, in the login burst, and never sent again. The action bar's
/// contents depend on this packet.
pub(super) fn initial_spells(ctx: &mut Incoming, pkt: &Packet) {
    let Some(book) = read(ctx.stats, pkt, spells::parse_initial_spells(&pkt.body)) else {
        return;
    };
    ctx.stats.spells_known = book.known.len() as u32;
    ctx.world.apply_spellbook(book);
}

/// `SMSG_ACTION_BUTTONS`: where the last session left the bar.
///
/// The body is 120 `u32` words with no count before them. The body length is
/// the only framing, so a short body produces a partly filled bar rather than a
/// parse failure.
pub(super) fn action_buttons(ctx: &mut Incoming, pkt: &Packet) {
    let buttons = spells::parse_action_buttons(&pkt.body);
    ctx.stats.action_buttons = buttons.len() as u32;
    ctx.world.apply_action_buttons(buttons);
}

/// `SMSG_LEARNED_SPELL` (`u32` spell id) and `SMSG_REMOVED_SPELL` (`u16` spell
/// id).
///
/// The two packets are adjacent in the opcode table but their id widths differ.
/// Reading both with the same width produces wrong spell ids, such as spell 0.
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
/// The client performs the replacement itself, in the spellbook and in every
/// action bar slot that holds the old id. The server sends this packet once. If
/// it is missed, the action bar keeps the old rank: the button draws correctly
/// but casts nothing, because the server refuses a superseded rank without
/// replying. See [`spells::parse_superceded_spell`].
pub(super) fn superceded_spell(ctx: &mut Incoming, pkt: &Packet) {
    let Some((old, new)) = read(ctx.stats, pkt, spells::parse_superceded_spell(&pkt.body)) else {
        return;
    };
    ctx.world.apply_superceded_spell(old, new);
}

/// `SMSG_CAST_RESULT`: the answer to our own `CMSG_CAST_SPELL`.
///
/// Sent on success as well as on failure. A success tells the cast bar to keep
/// running; a failure, with no success, stops it. See [`spells::CastResult`].
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
    // On a failure, also cancel the cast animation. This client starts its own
    // cast animation when the key is pressed, so a refusal arrives while the
    // wind-up pose is held and, for an instant cast, the release is already
    // playing. No other packet ends either animation; see
    // [`crate::state::objects::Entity::casts_cancelled`].
    if result.failure.is_some() {
        if let Some(guid) = ctx.world.player_guid {
            ctx.world.apply_cast_cancelled(guid, result.spell_id);
        }
    }
}

/// `SMSG_PET_SPELLS`: the whole pet action bar, or an eight-byte body that
/// removes it.
///
/// The bar is stored as state rather than raised as events, for two reasons. The
/// pet panel is drawn from it every frame. The dismissal is the same packet with
/// a zero guid, which a queue of events could not express.
///
/// The pet's name is not queried here. On a summon this packet arrives before
/// the pet's create block, so the pet number the name query is keyed by is not
/// known yet. `ObjectManager::unresolved_pet_names` builds the query from the
/// bar and the entity together instead.
pub(super) fn pet_spells(ctx: &mut Incoming, pkt: &Packet) {
    let Some(spells) = read(ctx.stats, pkt, crate::play::pet::parse_pet_spells(&pkt.body)) else {
        return;
    };
    ctx.world.apply_pet_spells(spells);
    // Tell the query pass to run on the next tick instead of waiting for its
    // two-second interval, so the pet name query goes out promptly. See
    // `ObjectManager::take_query_hint`.
    ctx.world.hint_queries();
}

/// `SMSG_PET_MODE`: the pet's four state bytes on their own, sent when only the
/// pet's mode changed.
pub(super) fn pet_mode(ctx: &mut Incoming, pkt: &Packet) {
    let Some(mode) = read(ctx.stats, pkt, crate::play::pet::parse_pet_mode(&pkt.body)) else {
        return;
    };
    ctx.world.apply_pet_mode(mode);
}

/// `SMSG_PET_NAME_QUERY_RESPONSE`: the name the player gave the pet, keyed by
/// pet number.
pub(super) fn pet_name(ctx: &mut Incoming, pkt: &Packet) {
    let Some(name) = read(ctx.stats, pkt, crate::play::pet::parse_pet_name(&pkt.body)) else {
        return;
    };
    // A rename is answered under the same pet number, so this cached answer
    // replaces an existing entry instead of being skipped. See
    // [`crate::play::wdb`].
    ctx.world
        .remember_again(Kind::PetName, u64::from(name.pet_number), &pkt.body);
    ctx.world.apply_pet_name(name);
}

/// Pet feedback: the four pet packets that each carry one message line, plus
/// the unlearn confirmation.
///
/// Each is an event rather than state; see [`PlayerEvent::PetFeedback`]. Two of
/// the five have no body, like the five attack-swing refusals: the opcode is the
/// message.
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

/// `SMSG_PET_ACTION_SOUND`: the pet plays a voice sound. The value selects one
/// of two `CreatureSoundData` columns rather than naming a sound. It is an event,
/// like the pet feedback above: two orders produce two sounds.
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

/// `SMSG_PET_DISMISS_SOUND`: the sound played when a pet is dismissed. It is the
/// only pet packet about a unit that no longer exists, so it carries a model id
/// and a position.
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

/// `SMSG_INVENTORY_CHANGE_FAILURE`: an item action was refused.
///
/// This is the only packet that answers an item right-click the server
/// discarded. A potion whose level requirement is not met never reaches
/// vmangos's `Spell::prepare`, so no `SMSG_CAST_RESULT` is sent. Without this
/// packet the click looks the same as a click on empty ground, which is the
/// reported problem this handler fixes.
///
/// `EQUIP_ERR_OK` also arrives here, as a one-byte body: several item actions
/// end with one, and it is not a failure. It is passed on rather than dropped.
/// The release code that `HandleUseItemOpcode` sends before the real reason
/// (`EQUIP_ERR_NONE`) is a different value with no string, and the event's
/// reader tells the two apart.
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

/// `SMSG_ITEM_PUSH_RESULT`: an item was added to a bag. It is the only packet
/// that reports this, whatever the source of the item.
///
/// The rest of the inventory arrives as update fields, and a field states the
/// current value, not what happened: a stack that grew by three does not say
/// whether it was looted or bought. vmangos's `Player::SendNewItem` has twenty
/// callers (loot, a vendor, a quest reward, mail, a trade, a craft, a
/// battleground mark, a GM command), and every one of them sends this packet.
///
/// For loot it is broadcast to the group, so the guid in the body is not always
/// the local character's and the reader has to check it. See
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

/// `SMSG_ITEM_ENCHANT_TIME_UPDATE`: how long a temporary enchantment has left.
///
/// The 1.12.1 client counts the time down itself from the moment the packet
/// arrives; the time the interface prints is not the item's own duration
/// field. See [`crate::play::items::ItemTimers`]. The inventory version is
/// incremented so the snapshot the interface reads picks up the new expiry.
pub(super) fn enchant_time(ctx: &mut Incoming, pkt: &Packet) {
    let Some(time) = read(
        ctx.stats,
        pkt,
        crate::play::items::parse_item_enchant_time_update(&pkt.body),
    ) else {
        return;
    };
    ctx.world
        .item_timers
        .note_enchantment(time, std::time::Instant::now());
    ctx.world.inventory_version = ctx.world.inventory_version.wrapping_add(1);
}

/// `SMSG_ITEM_TIME_UPDATE`: how long an expiring item has left, on the same
/// terms as [`enchant_time`].
pub(super) fn item_time(ctx: &mut Incoming, pkt: &Packet) {
    let Some(time) = read(
        ctx.stats,
        pkt,
        crate::play::items::parse_item_time_update(&pkt.body),
    ) else {
        return;
    };
    ctx.world
        .item_timers
        .note_item(time, std::time::Instant::now());
    ctx.world.inventory_version = ctx.world.inventory_version.wrapping_add(1);
}

/// `SMSG_TRANSFER_ABORTED`: the requested map change will not happen.
///
/// The body is one byte. Apart from the teleport itself, this is the only reply
/// an instance portal can produce: vmangos's `HandleAreaTriggerOpcode` sends
/// nothing for a trigger it declines to act on, so the two outcomes are this
/// packet and `SMSG_NEW_WORLD`. The reason byte is passed on unchanged; see
/// [`crate::play::areatrigger::TransferAbort`]. For three of the six codes the
/// 1.12.1 client shows nothing.
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

/// `SMSG_AREA_TRIGGER_MESSAGE`: a teleport trigger refused the character, with
/// the row's text. The other refusal a portal can produce besides
/// `SMSG_TRANSFER_ABORTED`; see
/// [`crate::play::areatrigger::parse_area_trigger_message`].
pub(super) fn area_trigger_message(ctx: &mut Incoming, pkt: &Packet) {
    let Some(text) = read(
        ctx.stats,
        pkt,
        crate::play::areatrigger::parse_area_trigger_message(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::AreaTriggerMessage { text });
}

/// `SMSG_TRANSFER_PENDING`: the character is about to change map.
///
/// This packet exists so the client can show the loading screen. Nothing is
/// acknowledged, no state changes and no position arrives. vmangos sends it from
/// `Player::ExecuteTeleportFar` shortly before it removes the character from the
/// old map, so the client can show the loading screen before the old world is
/// unloaded.
///
/// The loading screen is `crates/client/src/game/loading.rs`.
pub(super) fn transfer_pending(ctx: &mut Incoming, pkt: &Packet) {
    let Some(pending) = read(
        ctx.stats,
        pkt,
        crate::state::movement::parse_transfer_pending(&pkt.body),
    ) else {
        return;
    };
    // The packet also says whether the transfer began on a transport. The two
    // extra dwords are written only for a character standing on a transport,
    // and `SMSG_NEW_WORLD` has no equivalent field. This is therefore the only
    // packet that distinguishes a transport carrying the character to the new
    // map from a teleport that takes the character off the transport. See
    // [`crate::state::movement::Mover::transfer_aboard`].
    if let Some(local) = ctx.local.as_deref_mut() {
        local.transfer_on_transport = pending.transport.is_some();
    }
    ctx.world.note_event(PlayerEvent::TransferPending {
        map_id: pending.map_id,
    });
}

/// `SMSG_LOGIN_VERIFY_WORLD`: the map the login actually placed the character
/// on.
///
/// Sets [`LocalState::map_id`] and nothing else. The reasoning is in
/// [`crate::state::movement::LoginVerifyWorld`]: the map named in the character
/// list can be stale by the time the login finishes, vmangos corrects it inside
/// `Player::LoadFromDB` with a bare `Relocate`, and this is the only packet that
/// carries the correction.
///
/// Not acknowledged, unlike `SMSG_NEW_WORLD`. No transfer semaphore is set: the
/// server is not holding the session until the client confirms arrival; it is
/// stating where the character
/// already is. A `MSG_MOVE_WORLDPORT_ACK` sent with no transfer pending is a
/// packet the server has no state for.
///
/// The position in the packet is not used. It arrives before the character's
/// create block, which carries the same position together with the speeds and
/// movement flags that must agree with it; `SessionLoop::enter_world` resyncs
/// the mover from that block. Using one source avoids two positions that could
/// disagree, and the create block is the source with more data.
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
/// Broadcast, so it arrives for any caster in view; the reader decides whether
/// the guid is the local character's. vmangos never sends `SMSG_SPELL_FAILURE`.
pub(super) fn cast_interrupted(ctx: &mut Incoming, pkt: &Packet) {
    let Some((guid, spell_id)) = read(ctx.stats, pkt, spells::parse_spell_failed_other(&pkt.body))
    else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::CastInterrupted { guid, spell_id });
    // The packet is broadcast, so this cancels the wind-up animation of any
    // caster, not only the local character. Without this call an interrupted
    // caster held the pose until `HOLD_GRACE_SECS` ran out: a silenced mage
    // kept the casting pose for a second after the cast had stopped. The
    // `PlayerEvent` above ends the cast bar, which only the local character has;
    // this call ends the animation, which every caster has.
    ctx.world.apply_cast_cancelled(guid, spell_id);
}

/// `SMSG_SPELL_DELAYED`: a cast was pushed back by damage taken.
///
/// This packet is the only source of cast pushback, and the client does none
/// of the arithmetic: the server has already moved its own `m_timer`, and the
/// packet says by how much. Two readers in the client need it. The world's copy
/// extends the wind-up pose and the effects attached to the caster
/// ([`ObjectManager::apply_cast_delayed`]). The event queue's copy moves the
/// cast bar, which FrameXML's `CastingBarFrame_OnEvent` does on
/// `SPELLCAST_DELAYED`.
pub(super) fn cast_delayed(ctx: &mut Incoming, pkt: &Packet) {
    let Some((guid, delay_ms)) = read(ctx.stats, pkt, spells::parse_spell_delayed(&pkt.body)) else {
        return;
    };
    ctx.world.apply_cast_delayed(guid, delay_ms);
    // The packet is sent only to the caster, but the guid in it, not the
    // opcode, decides whether the local cast bar moves. A pushback on a pet or
    // any other unit has no
    // cast bar here; applying it to the player's bar would move the bar for a
    // cast the player is not making.
    if ctx.world.player_guid == Some(guid) {
        ctx.world.note_event(PlayerEvent::CastDelayed { delay_ms });
    }
}

/// Quest packet handlers, one function per opcode. The wire format is in
/// [`crate::play::quest`], including two details that cannot be inferred: the
/// quest log's packed six-bit counters, and the `| 0x80000000` flag that makes
/// an objective's target a game object rather than a creature.
///
/// This is a macro because eleven of the handlers are the same three lines. The
/// alternative is eleven more copies of `let Some(x) = read(…) else { return }`,
/// a pattern this file already repeats twenty times.
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
/// `SMSG_QUEST_QUERY_RESPONSE`: one quest's template. The body is stored in the
/// on-disk cache, keyed by the quest id it begins with. See
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

/// `SMSG_QUESTGIVER_STATUS`: the quest marker over a quest giver's head.
///
/// Stored in the world as well as raised as an event, because unlike the other
/// quest packets its value persists: the `!` stays until the server sends a new
/// status. The pass that draws the marker reads the stored value, because it may
/// have started after the event was raised.
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
/// Two opcodes with the same body. They differ only in the message the
/// interface shows, so the event carries a `timed_out` flag instead of merging
/// the two.
pub(super) fn quest_failed(ctx: &mut Incoming, pkt: &Packet, timed_out: bool) {
    let Some(quest_id) = read(ctx.stats, pkt, crate::play::quest::parse_quest_id(&pkt.body)) else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::QuestFailed { quest_id, timed_out });
}

/// `SMSG_GOSSIP_MESSAGE`: an NPC's gossip menu. This handler and the ones after
/// it cover NPC interaction: the gossip menu, its text, and the vendor window.
/// The wire format and the icon table are in [`crate::play::gossip`].
pub(super) fn gossip_show(ctx: &mut Incoming, pkt: &Packet) {
    let Some(menu) = read(ctx.stats, pkt, crate::play::gossip::parse_gossip_message(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GossipShow(Box::new(menu)));
}

/// `SMSG_GOSSIP_COMPLETE`: the server closed the gossip window. The packet has
/// no body.
pub(super) fn gossip_closed(ctx: &mut Incoming) {
    ctx.world.note_event(PlayerEvent::GossipClosed);
}

/// `SMSG_BINDPOINTUPDATE`: where the hearthstone returns the character to.
///
/// Arrives once in the login burst and again on every change, and no other
/// packet carries the bind point. It is therefore stored in
/// [`ObjectManager::bind_point`] rather than raised as an event. See
/// [`crate::play::bindpoint`].
pub(super) fn bind_point(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(point) = read(
        ctx.stats,
        pkt,
        crate::play::bindpoint::parse_bind_point(&pkt.body),
    ) {
        ctx.world.bind_point = Some(point);
    }
}

/// `SMSG_PLAYERBOUND`: the unit that set the local character's bind point, and
/// the area it was set to.
///
/// Raised as an event and also written into the stored bind point. vmangos's
/// `Spell::EffectBind` sends this packet and `SMSG_BINDPOINTUPDATE` together,
/// with no fixed order between them. Taking the area from whichever arrives
/// first means the tooltip never names the old home in between.
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
/// Not answered here. The reply comes from the player pressing Accept on the
/// interface's `CONFIRM_BINDER` popup: the guid goes to the client, which sends
/// `CMSG_BINDER_ACTIVATE` if the player accepts. Answering automatically would
/// bind the character to every inn whose innkeeper they spoke to.
pub(super) fn binder_confirm(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(guid) = read(
        ctx.stats,
        pkt,
        crate::play::bindpoint::parse_binder_confirm(&pkt.body),
    ) {
        ctx.world.note_event(PlayerEvent::BinderConfirm { guid });
    }
}

/// `SMSG_DUEL_REQUESTED`: a duel request, sent by the local character or to it.
///
/// Passed on unchanged. What the 1.12.1 client does with it depends on whether
/// the initiator is the local character, whether they are on the ignore list,
/// and whether they are in view; the client decides all three. See
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

/// `SMSG_DUEL_COMPLETE`: the duel has ended, whether or not it started.
pub(super) fn duel_complete(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(started) = read(ctx.stats, pkt, crate::play::duel::parse_duel_complete(&pkt.body)) {
        ctx.world.note_event(PlayerEvent::DuelComplete { started });
    }
}

/// `SMSG_DUEL_WINNER`: who won. Broadcast to every player near the duel flag, so
/// it also arrives for other players' duels, and the 1.12.1 client prints the
/// message for those too.
pub(super) fn duel_winner(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(winner) = read(ctx.stats, pkt, crate::play::duel::parse_duel_winner(&pkt.body)) {
        ctx.world.note_event(PlayerEvent::DuelWinner(winner));
    }
}

/// `SMSG_SUMMON_REQUEST`: another player is summoning the local character.
///
/// The summoner's name is queried here. The popup shows the name, and the
/// summoner is almost never in view: they are at a meeting stone or a ritual
/// elsewhere in the world. The 1.12.1 client takes the name from its name cache
/// and fills it in when the query answer arrives; here the query pass behind
/// [`ObjectManager::want_social_guid`] does the same.
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

/// `SMSG_INSPECT`: the server accepted an inspect request. It carries the guid
/// alone and nothing the inspect window draws, which comes from the visible
/// item fields; it is read so that a malformed body is reported.
pub(super) fn inspect(ctx: &mut Incoming, pkt: &Packet) {
    let _ = read(ctx.stats, pkt, crate::play::inspect::parse_inspect(&pkt.body));
}

/// `MSG_INSPECT_HONOR_STATS`: the honor tab of the player being inspected.
pub(super) fn inspect_honor(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(honor) = read(ctx.stats, pkt, crate::play::inspect::parse_honor_stats(&pkt.body)) {
        ctx.world.note_event(PlayerEvent::InspectHonor(honor));
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

/// `SMSG_BUY_FAILED`: a purchase was refused, and the reason.
pub(super) fn buy_failed(ctx: &mut Incoming, pkt: &Packet) {
    let Some((_guid, entry, reason)) =
        read(ctx.stats, pkt, crate::play::gossip::parse_buy_failed(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::BuyFailed { entry, reason });
}

/// `SMSG_SELL_ITEM`: a sale was refused. A successful sale sends no packet.
pub(super) fn sell_failed(ctx: &mut Incoming, pkt: &Packet) {
    let Some((_vendor, item, reason)) =
        read(ctx.stats, pkt, crate::play::gossip::parse_sell_failed(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::SellFailed { item, reason });
}

/// `SMSG_TRAINER_LIST`: the services this trainer offers. The row layout and
/// the three row states are in [`crate::play::trainer`].
pub(super) fn trainer_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(ctx.stats, pkt, crate::play::trainer::parse_trainer_list(&pkt.body)) else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::TrainerShow(Box::new(list)));
}

/// `SMSG_TRAINER_BUY_SUCCEEDED`: a service was learned.
///
/// This packet does not add the spell to the spellbook; `SMSG_LEARNED_SPELL`
/// does that, sent by the cast the server runs for the player. This packet only
/// says which row of the open window is now grey. The server also re-sends the
/// whole list. Both are read, because the row has to change colour on the
/// purchase rather than a round trip later.
pub(super) fn trainer_bought(ctx: &mut Incoming, pkt: &Packet) {
    let Some((_guid, spell)) = read(ctx.stats, pkt, crate::play::trainer::parse_trainer_bought(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::TrainerBought { spell });
}

/// `SMSG_TRAINER_BUY_FAILED`: a trainer purchase was refused. The reason is a
/// `u32`, where every other refusal code in the game is a byte.
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

/// `MSG_LIST_STABLED_PETS`: the full contents of the stable window. The row
/// layout and the slot numbering are in [`crate::play::stable`].
///
/// The client sends the same opcode to request the list again. The `MSG_`
/// prefix means the opcode is used in both directions; the sender decides the
/// direction, not the name.
pub(super) fn stable_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(ctx.stats, pkt, crate::play::stable::parse_stabled_pets(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::StableList(Box::new(list)));
}

/// `SMSG_STABLE_RESULT`: a one-byte result shared by all four stable actions.
///
/// The packet does not say which action it answers. Stabling, unstabling,
/// swapping and buying a slot all reply here, and only the success codes
/// distinguish them. The window requests the list again after any success
/// instead of inferring the change; see `client/src/game/npc/stable.rs`.
pub(super) fn stable_result(ctx: &mut Incoming, pkt: &Packet) {
    let Some((byte, result)) = read(ctx.stats, pkt, crate::play::stable::parse_stable_result(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::StableResult { byte, result });
}

/// `SMSG_SHOW_BANK`: opens the bank window. The body is the banker's guid and
/// nothing else. The bank's contents are twenty-four guid fields that arrive
/// with the backpack (see [`crate::play::bank`]), so this packet only opens the
/// frame and names the banker that every later bank action names again.
pub(super) fn bank_show(ctx: &mut Incoming, pkt: &Packet) {
    let Some(banker) = read(ctx.stats, pkt, crate::play::bank::parse_show_bank(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::BankShow(banker));
}

/// `SMSG_BUY_BANK_SLOT_RESULT`: a refused bank slot purchase. This packet is
/// only sent on failure; a success changes a byte of `PLAYER_BYTES_2` and sends
/// nothing. See [`crate::play::bank`].
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

/// `SMSG_SHOWTAXINODES`: opens the flight map. The wire format is in
/// [`crate::play::taxi`]; the data the window draws from is in
/// `vale_assets::tables::taxi`.
pub(super) fn taxi_show(ctx: &mut Incoming, pkt: &Packet) {
    let Some(menu) = read(ctx.stats, pkt, crate::play::taxi::parse_show_taxi_nodes(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::TaxiShow(menu));
}

/// `SMSG_TAXINODE_STATUS`: whether the local character knows this flight
/// master's node.
///
/// Stored as state and also raised as an event, like `SMSG_QUESTGIVER_STATUS`
/// and for the same reason. The green `!` over an undiscovered flight master is
/// drawn by a pass that may have started after the byte arrived, so the value
/// is kept in the world as well as announced.
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

/// `SMSG_NEW_TAXI_PATH`: a flight point was discovered. The packet has no body.
/// The updated known-nodes mask arrives with the next `SMSG_SHOWTAXINODES`, not
/// here.
///
/// No stored status is cleared here. The green `!` that the discovery removes is
/// keyed by the flight master's guid, and vmangos's `SendLearnNewTaxiNode` sends
/// a new `SMSG_TAXINODE_STATUS` with value 1 for that guid immediately after
/// this packet. The marker is removed by that status packet, so the client does
/// not have to work out which unit the discovery refers to.
pub(super) fn new_taxi_path(ctx: &mut Incoming) {
    ctx.world.note_event(PlayerEvent::NewTaxiPath);
}

/// `SMSG_ACTIVATETAXIREPLY`: the flight has started, or one of twelve reasons it
/// has not.
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

/// `SMSG_LOOT_RESPONSE`: the loot window for a corpse or object, or a refusal to
/// open one.
///
/// A loot type of zero marks the refusal. [`crate::play::loot`] explains that
/// reading and maps between the two numberings: the server's sparse slot index
/// and the interface's dense row number.
pub(super) fn loot_response(ctx: &mut Incoming, pkt: &Packet) {
    let Some(loot) = read(ctx.stats, pkt, crate::play::loot::parse_loot_response(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LootOpened(loot));
}

/// `SMSG_LOOT_RELEASE_RESPONSE`: the server has closed the loot session.
///
/// This packet, not the client's release request, closes the window.
/// vmangos's `HandleLootReleaseOpcode` ignores the guid the client sends and
/// releases whatever loot it last recorded. Closing the window when the request
/// is sent leaves the server believing the window is still open, and the corpse
/// cannot be looted again.
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

/// `SMSG_LOOT_REMOVED`: one row was taken, identified by the server's slot
/// index.
///
/// Sent to every player with the window open, so it arrives for a row a group
/// member took as well as one the local character took. The window is updated
/// from this packet rather than from the local click for that reason.
pub(super) fn loot_removed(ctx: &mut Incoming, pkt: &Packet) {
    let Some(index) = read(ctx.stats, pkt, crate::play::loot::parse_loot_removed(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LootRemoved { index });
}

/// `SMSG_LOOT_START_ROLL`: a group roll has started on one loot row.
///
/// The loot window already holds that row as `RollOngoing`; this packet opens a
/// `GroupLootFrame` for the player. See [`crate::play::lootroll`]. The roll id
/// the interface uses is a counter kept by the client, not a value from this
/// packet.
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

/// `SMSG_LOOT_ROLL`: a player chose need, greed or pass, or a player's roll
/// result.
///
/// Both use this one packet. The roll number tells them apart, not the roll
/// type byte next to it; see [`crate::play::lootroll::RollLine::of`].
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

/// `SMSG_LOOT_ROLL_WON`: the roll is over and the item is in the winner's bags.
///
/// It can name a roll that never started on this client. The result goes to
/// every eligible player, and a member who joined after the roll opened was not
/// sent the start. The 1.12.1 client handles that case as well; see
/// [`crate::play::lootroll`].
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

/// `SMSG_LOOT_ALL_PASSED`: every player passed on the roll.
///
/// The client also makes the row clickable again on this packet. No packet
/// states that a blocked row has been unblocked. See
/// [`crate::play::loot::Loot::unblock`].
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

/// `SMSG_LOOT_CLEAR_MONEY`: the money has been removed from the loot window. The
/// packet has no body.
pub(super) fn loot_money_cleared(ctx: &mut Incoming) {
    ctx.world.note_event(PlayerEvent::LootMoneyCleared);
}

/// `SMSG_LOOT_MONEY_NOTIFY`: the local character's share of the money, which in
/// a group differs from the amount on the corpse. It is a separate event from
/// the clear above; see [`crate::play::loot`].
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

/// `SMSG_START_MIRROR_TIMER`: a timer bar that the server counts down.
///
/// These are the breath meter and the two bars next to it. This handler makes
/// no decisions: the packet carries a remaining value, a maximum and a rate, and
/// the interface interpolates between packets. See [`crate::play::timers`],
/// which covers all three mirror-timer packets, including why the shipped
/// FrameXML handler cannot act on the third (pause).
///
/// Every arrival is a start, not an update. The server also resends this packet
/// to signal a pause, because its own pause event is broken, so two arrivals
/// with identical fields are still two separate statements about the bar.
pub(super) fn mirror_timer_start(ctx: &mut Incoming, pkt: &Packet) {
    let Some(start) = read(ctx.stats, pkt, timers::parse_start_mirror_timer(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::MirrorTimerStarted(start));
}

/// `SMSG_STOP_MIRROR_TIMER`: removes a timer bar. The body is one `u32`.
pub(super) fn mirror_timer_stop(ctx: &mut Incoming, pkt: &Packet) {
    let Some(timer) = read(ctx.stats, pkt, timers::parse_stop_mirror_timer(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::MirrorTimerStopped { timer });
}

/// `SMSG_PAUSE_MIRROR_TIMER`: freezes a timer bar at its current value.
///
/// vmangos never sends this, and the FrameXML interface could not use it if it
/// did: `MirrorTimerFrame_OnEvent` reads `arg1` as both the timer's name and
/// the paused flag. It is still parsed and raised, so a server that does send it
/// is not ignored.
pub(super) fn mirror_timer_pause(ctx: &mut Incoming, pkt: &Packet) {
    let Some((timer, paused)) = read(ctx.stats, pkt, timers::parse_pause_mirror_timer(&pkt.body))
    else {
        return;
    };
    ctx.world
        .note_event(PlayerEvent::MirrorTimerPaused { timer, paused });
}

/// `SMSG_SPELL_COOLDOWN`: a cooldown the server imposes directly, not the
/// ordinary per-cast cooldown.
///
/// The client computes an ordinary cast's cooldown from `Spell.dbc` and starts
/// it when the cast's `SMSG_SPELL_GO` arrives; vmangos sends no packet for it.
/// This packet carries a school lockout, a pet's cooldown list, or a GM reset,
/// and it can name several spells at once.
pub(super) fn spell_cooldown(ctx: &mut Incoming, pkt: &Packet) {
    let Some(cooldowns) = read(ctx.stats, pkt, spells::parse_spell_cooldowns(&pkt.body)) else {
        return;
    };
    // A packet about a pet or another unit is dropped: this client has one
    // cooldown store, and it belongs to the player.
    if ctx.world.player_guid != Some(cooldowns.guid) {
        return;
    }
    for (spell_id, ms) in cooldowns.entries {
        ctx.world
            .note_event(PlayerEvent::CooldownStarted { spell_id, ms });
    }
}

/// `SMSG_CLEAR_COOLDOWN`: a cooldown ends now.
///
/// The counterpart to [`spell_cooldown`] above. The server sends it when a
/// school lockout is lifted early, when any cooldown is removed, and for the
/// warlock ritual correction. Without it the cooldown animation runs for the
/// length the start packet gave, which is correct only when the lockout runs its
/// full course.
///
/// The guid comparison does more than filter out pets: it validates the field
/// layout assumed by [`spells::parse_clear_cooldown`]. See that function.
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

/// `SMSG_COOLDOWN_EVENT`: start a cooldown that was held.
///
/// Spells with `SPELL_ATTR_COOLDOWN_ON_EVENT`, such as Stealth and Feign Death,
/// start their cooldown when the effect ends rather than when they are cast. The
/// client records the cooldown as held at cast time, and this packet starts it.
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
/// The two packets encode guids differently: start writes two plain guids and
/// stop writes two packed guids, in adjacent functions of vmangos's `Unit.cpp`.
/// Both are broadcast about every unit; only the local character's reach the
/// state. See the module comment.
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

/// `SMSG_CANCEL_AUTO_REPEAT`: the auto-repeat ranged attack has stopped.
///
/// No body and no guid. vmangos sends it only to the player it concerns
/// (`Player::SendAutoRepeatCancel`), so there is nothing to check or read. See
/// [`PlayerEvent::AutoRepeatCancelled`] for the senders other than the player's
/// own key press. Those senders are why the packet must be handled: every
/// server-side end of the loop (the target dies, the target moves out of range,
/// a wand user moves) is reported only by this packet.
pub(super) fn auto_repeat_cancelled(ctx: &mut Incoming) {
    ctx.world.note_event(PlayerEvent::AutoRepeatCancelled);
}

/// `MSG_CHANNEL_START`: a channel has begun, and for how long.
///
/// This is the only packet that says how long to hold a channel's pose. On the
/// wire every other packet of a channelled spell looks like an instant cast:
/// `SMSG_SPELL_START` is not sent and `SMSG_SPELL_GO` arrives immediately.
/// Without this packet Evocation was drawn as a release animation with nothing
/// after it. See [`ObjectManager::apply_channel_start`], which records the
/// channel in the cast counters, because for the animation a channel is a cast.
pub(super) fn channel_start(ctx: &mut Incoming, pkt: &Packet) {
    let Some((spell_id, duration_ms)) = read(ctx.stats, pkt, spells::parse_channel_start(&pkt.body))
    else {
        return;
    };
    ctx.world.apply_channel_start(spell_id, duration_ms);
    // Also raise the event for the interface. The world's copy above drives the
    // held pose; this event drives the cast bar, and no other packet starts a
    // channel's bar. See [`PlayerEvent::ChannelStart`].
    ctx.world
        .note_event(spells::PlayerEvent::ChannelStart {
            spell_id,
            duration_ms,
        });
}

/// `MSG_CHANNEL_UPDATE`: the time left on the channel; zero means it has ended.
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
/// Always about the local character: the server sends it only to the aura's
/// target, and only when that target is a player. There is no guid to check. A
/// wrong slot reading would put a timer under the wrong icon, so matching the
/// slot to an aura is done and checked by the reader; see
/// [`ObjectManager::note_aura_duration`].
pub(super) fn aura_duration(ctx: &mut Incoming, pkt: &Packet) {
    let Some((slot, remaining_ms)) = read(ctx.stats, pkt, spells::parse_aura_duration(&pkt.body))
    else {
        return;
    };
    ctx.world.note_aura_duration(slot, remaining_ms);
}

/// `SMSG_LEVELUP_INFO`: the local character gained a level.
///
/// The only packet that reports a level-up as an event. It carries the new
/// level and eleven stat deltas, all of which the FrameXML handler reads. See
/// [`spells::parse_levelup`].
pub(super) fn levelup(ctx: &mut Incoming, pkt: &Packet) {
    let Some(gained) = read(ctx.stats, pkt, spells::parse_levelup(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::LevelUp(gained));
}

/// `SMSG_CORPSE_RECLAIM_DELAY`: how long until the corpse can be reclaimed.
///
/// Sent once, when the spirit is released, and never again. A client that drops
/// it cannot ask again, and shows a Retrieve button that the server ignores for
/// the next thirty seconds.
pub(super) fn corpse_reclaim_delay(ctx: &mut Incoming, pkt: &Packet) {
    let Some(ms) = read(ctx.stats, pkt, crate::play::death::parse_corpse_reclaim_delay(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::CorpseReclaimDelay { ms });
}

/// `MSG_CORPSE_QUERY`, the server's reply, which uses the same opcode as the
/// request.
///
/// A found corpse and no corpse are both valid answers, and only the first has
/// data after its flag byte; see [`crate::play::death::parse_corpse_query`]. For
/// that reason the value is `Option<Option<_>>` throughout, so "no corpse" is
/// not confused with "would not parse".
pub(super) fn corpse_located(ctx: &mut Incoming, pkt: &Packet) {
    let Some(place) = read(ctx.stats, pkt, crate::play::death::parse_corpse_query(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::CorpseLocated(place));
}

/// `SMSG_RESURRECT_REQUEST`: a unit has offered to resurrect the local
/// character.
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
/// The only one of the four logout packets with a body, and the only one that
/// can refuse. The three `reason` values and the instant flag are documented in
/// [`crate::play::logout`]. A body that will not parse is reported, not treated
/// as an acceptance: reading five bytes as a zero reason would show the
/// character screen while the character is still in the world.
pub(super) fn logout_response(ctx: &mut Incoming, pkt: &Packet) {
    let Some(logout) = read(ctx.stats, pkt, crate::play::logout::parse_logout_response(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::Logout(logout));
}

/// `SMSG_LOGOUT_COMPLETE` and `SMSG_LOGOUT_CANCEL_ACK`: neither has a body, so
/// the opcode is the message, as with the five attack refusals below.
pub(super) fn logout_state(ctx: &mut Incoming, state: crate::play::logout::Logout) {
    ctx.world.note_event(PlayerEvent::Logout(state));
}

/// The five `SMSG_ATTACKSWING_*` refusals.
///
/// None of them has a body; the opcode is the message. Each maps to a
/// `GlobalStrings.lua` key rather than a string written for this client. "You
/// are too far away!" and "You are facing the wrong way!" are the game's own
/// text for the two most common ones.
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

// --- Group packets --------------------------------------------------------
//
// See [`crate::play::group`]. Six packets are events and one is state; the
// state is `SMSG_GROUP_LIST`, which always arrives complete.

/// `SMSG_GROUP_INVITE`: another player invited the local character to a group.
/// The body is the inviter's name and nothing else. If it is dropped the player
/// cannot join the group: the invitation is not repeated and cannot be
/// requested.
pub(super) fn group_invite(ctx: &mut Incoming, pkt: &Packet) {
    let Some(name) = read(ctx.stats, pkt, crate::play::group::parse_name(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GroupInvite { name });
}

/// `SMSG_GROUP_DECLINE`: a player the local character invited declined. Only
/// the inviter receives it, and nothing else records the decline.
pub(super) fn group_decline(ctx: &mut Incoming, pkt: &Packet) {
    let Some(name) = read(ctx.stats, pkt, crate::play::group::parse_name(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GroupDecline { name });
}

/// `SMSG_GROUP_LIST`: the full group roster. There is no incremental form.
///
/// Every join, leave, promotion and loot-rule change re-sends the whole list,
/// and each member's copy leaves out that member, so the list maps directly to
/// `party1..N`.
pub(super) fn group_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(ctx.stats, pkt, crate::play::group::parse_group_list(&pkt.body)) else {
        return;
    };
    // Member names are queried from here rather than from the entity walk,
    // because a member out of range has no entity, and the name query also
    // supplies the class that colours a raid button. See
    // `ObjectManager::group_guids`.
    ctx.world
        .note_group(list.members.iter().map(|member| member.guid).collect());
    ctx.world.note_event(PlayerEvent::GroupList(Box::new(list)));
}

/// `SMSG_GROUP_DESTROYED`: the group was disbanded. The packet has no body and
/// is the only notice of the disband: the server never sends a
/// `SMSG_GROUP_LIST` with an empty roster, so a client that reads only that
/// packet keeps showing the party frames.
pub(super) fn group_destroyed(ctx: &mut Incoming) {
    ctx.world.note_group(Vec::new());
    ctx.world.note_event(PlayerEvent::GroupDestroyed);
}

/// `SMSG_GROUP_SET_LEADER`: the new group leader, identified by name, whereas
/// `CMSG_GROUP_SET_LEADER` identifies the leader by guid.
pub(super) fn group_new_leader(ctx: &mut Incoming, pkt: &Packet) {
    let Some(name) = read(ctx.stats, pkt, crate::play::group::parse_name(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GroupNewLeader { name });
}

/// `SMSG_PARTY_COMMAND_RESULT`: the result of an invite or a leave.
///
/// Sent on success as well as on failure, so it is the only acknowledgement
/// `/invite` gets: the invitee's client receives the invitation, and the
/// inviter's client receives this.
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

/// `SMSG_PARTY_MEMBER_STATS` and `SMSG_PARTY_MEMBER_STATS_FULL`: two opcodes
/// with one body layout. They differ only in whether the server sent every
/// field or only the changed ones, and the field mask in the body already says
/// which.
///
/// This is the only source of a party member's health when the member is out
/// of the object manager's range, which for a party spread across a zone is
/// most of the time.
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

/// `MSG_RAID_READY_CHECK`: one opcode used for three different messages, told
/// apart only by body length.
///
/// An empty body means the leader started a ready check, and it is the most
/// common form. A reader that treats a zero-length body as malformed never
/// shows the ready-check dialog. See [`crate::play::group::ReadyCheck`].
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

// --- Friends, ignore list and /who ----------------------------------------
//
// See [`crate::play::social`]. Two lists arrive once and are updated by a third
// packet, and one packet is a search reply. None of the four carries a name
// for anyone on the two lists, so the two list handlers mark their guids for a
// name query here instead of leaving it to the entity walk.

/// `SMSG_FRIEND_LIST`: the whole friends list, sent once.
///
/// Every later change is an `SMSG_FRIEND_STATUS` about one player. A client that
/// reads this packet but not that one shows the login-time list for the rest
/// of the session.
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

/// `SMSG_IGNORE_LIST`: the whole ignore list, sent once. The body is guids and
/// nothing else.
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
/// Both refusals and list changes arrive in this packet, so the event carries
/// the result byte rather than an updated list. "Already your friend" and "your
/// friend just logged in" are the same packet with a different first byte, and
/// only the second changes the list.
pub(super) fn friend_status(ctx: &mut Incoming, pkt: &Packet) {
    let Some(status) = read(
        ctx.stats,
        pkt,
        crate::play::social::parse_friend_status(&pkt.body),
    ) else {
        return;
    };
    // A new friend or ignore entry is a guid with no known name, and the panel
    // shows the name. Queue the name query on the tick the packet arrives
    // rather than waiting for some later request to ask for it.
    if !status.result.removes() {
        ctx.world.want_social_guid(status.guid);
    }
    ctx.world.note_event(PlayerEvent::FriendStatus(status));
}

/// `SMSG_WHO`: the reply to a `/who` search. It carries names, not guids; see
/// [`crate::play::social`].
pub(super) fn who_results(ctx: &mut Incoming, pkt: &Packet) {
    let Some(results) = read(ctx.stats, pkt, crate::play::social::parse_who(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::WhoResults(results));
}

/// `SMSG_GUILD_QUERY_RESPONSE`: a guild's name, rank names and emblem.
pub(super) fn guild_query(ctx: &mut Incoming, pkt: &Packet) {
    let Some(query) = read(ctx.stats, pkt, crate::play::guild::parse_query(&pkt.body)) else {
        return;
    };
    // Kept in the world as well as forwarded: the name under a player's name
    // is read from here, and the guild tab is filled from the event.
    ctx.world.guilds.insert(query.id, query.clone());
    ctx.world.note_event(PlayerEvent::GuildQuery(query));
}

/// `SMSG_GUILD_ROSTER`: the whole member list.
pub(super) fn guild_roster(ctx: &mut Incoming, pkt: &Packet) {
    let Some(roster) = read(ctx.stats, pkt, crate::play::guild::parse_roster(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GuildRoster(roster));
}

/// `SMSG_GUILD_EVENT`: one thing that happened in the guild.
pub(super) fn guild_event(ctx: &mut Incoming, pkt: &Packet) {
    let Some(event) = read(ctx.stats, pkt, crate::play::guild::parse_event(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GuildEvent(event));
}

/// `SMSG_GUILD_COMMAND_RESULT`: the answer to a guild request.
pub(super) fn guild_command_result(ctx: &mut Incoming, pkt: &Packet) {
    let Some(result) = read(
        ctx.stats,
        pkt,
        crate::play::guild::parse_command_result(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GuildCommandResult(result));
}

/// `SMSG_GUILD_INVITE`: an invitation to join a guild.
pub(super) fn guild_invite(ctx: &mut Incoming, pkt: &Packet) {
    let Some(invite) = read(ctx.stats, pkt, crate::play::guild::parse_invite(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GuildInvite(invite));
}

/// `SMSG_GUILD_DECLINE`: the invited player declined.
pub(super) fn guild_decline(ctx: &mut Incoming, pkt: &Packet) {
    let Some(name) = read(ctx.stats, pkt, crate::play::guild::parse_decline(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GuildDecline(name));
}

/// `SMSG_GUILD_INFO`: the answer to `/ginfo`.
pub(super) fn guild_info(ctx: &mut Incoming, pkt: &Packet) {
    let Some(info) = read(ctx.stats, pkt, crate::play::guild::parse_info(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GuildInfo(info));
}

/// `MSG_TABARDVENDOR_ACTIVATE`: a tabard designer opened its window.
pub(super) fn tabard_vendor(ctx: &mut Incoming, pkt: &Packet) {
    let Some(npc) = read(ctx.stats, pkt, crate::play::guild::parse_tabard_vendor(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::TabardVendor(npc));
}

/// `MSG_SAVE_GUILD_EMBLEM`: the answer to saving an emblem.
pub(super) fn guild_emblem_result(ctx: &mut Incoming, pkt: &Packet) {
    let Some(result) = read(ctx.stats, pkt, crate::play::guild::parse_emblem_result(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::GuildEmblemResult(result));
}

/// `SMSG_PETITION_SHOWLIST`: what a guild registrar sells.
pub(super) fn petition_show_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(ctx.stats, pkt, crate::play::petition::parse_show_list(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::PetitionShowList(list));
}

/// `SMSG_PETITION_SHOW_SIGNATURES`: a charter and its signers. The packet
/// names the owner and each signer by guid, so their names are asked for
/// here.
pub(super) fn petition_signatures(ctx: &mut Incoming, pkt: &Packet) {
    let Some(shown) = read(ctx.stats, pkt, crate::play::petition::parse_signatures(&pkt.body))
    else {
        return;
    };
    ctx.world.want_social_guid(shown.owner);
    for signer in &shown.signers {
        ctx.world.want_social_guid(*signer);
    }
    ctx.world.note_event(PlayerEvent::PetitionSignatures(shown));
}

/// `SMSG_PETITION_QUERY_RESPONSE`: a petition's guild name and owner.
pub(super) fn petition_query(ctx: &mut Incoming, pkt: &Packet) {
    let Some(query) = read(ctx.stats, pkt, crate::play::petition::parse_query(&pkt.body)) else {
        return;
    };
    ctx.world.want_social_guid(query.owner);
    ctx.world.note_event(PlayerEvent::PetitionQuery(query));
}

/// `SMSG_PETITION_SIGN_RESULTS`: the answer to a signature. The signer's name
/// is needed for the sentence the charter's owner reads.
pub(super) fn petition_sign_result(ctx: &mut Incoming, pkt: &Packet) {
    let Some(result) = read(ctx.stats, pkt, crate::play::petition::parse_sign_result(&pkt.body))
    else {
        return;
    };
    ctx.world.want_social_guid(result.player);
    ctx.world.note_event(PlayerEvent::PetitionSignResult(result));
}

/// `SMSG_TURN_IN_PETITION_RESULTS`: the answer to handing a charter in.
pub(super) fn petition_turn_in(ctx: &mut Incoming, pkt: &Packet) {
    let Some(result) = read(
        ctx.stats,
        pkt,
        crate::play::petition::parse_turn_in_result(&pkt.body),
    ) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::PetitionTurnInResult(result));
}

/// `MSG_PETITION_DECLINE`: a player declined to sign.
pub(super) fn petition_declined(ctx: &mut Incoming, pkt: &Packet) {
    let Some(player) = read(ctx.stats, pkt, crate::play::petition::parse_decline(&pkt.body)) else {
        return;
    };
    ctx.world.want_social_guid(player);
    ctx.world.note_event(PlayerEvent::PetitionDeclined(player));
}

/// `MSG_PETITION_RENAME`: a charter's guild name changed.
pub(super) fn petition_renamed(ctx: &mut Incoming, pkt: &Packet) {
    let Some((item, name)) = read(ctx.stats, pkt, crate::play::petition::parse_rename(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::PetitionRenamed { item, name });
}

/// `SMSG_INITIALIZE_FACTIONS`: the whole standing table, at login.
///
/// Sixty-four slots, whether or not the character has met each faction, so the
/// table replaces the stored one rather than merging into it. `SMSG_INITIAL_SPELLS`
/// replaces the spellbook for the same reason: the server sends the packet again
/// after anything that could have changed the set.
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

/// `SMSG_SET_FACTION_STANDING`: the standing in one or more slots changed.
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
/// vmangos does not send this while the character is loading, so it always
/// arrives mid-session and always adds a new row rather than changing one.
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

/// `SMSG_SET_FACTION_ATWAR`: the server's update of a faction's at-war flag,
/// shown as the crossed-swords checkbox in the reputation panel.
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

/// `SMSG_GAMEOBJECT_PAGETEXT`: open the text of a readable game object.
///
/// The body is one guid and nothing else. The page id is in the object's
/// template, which the client queried when the object came into view, so this
/// packet only tells the client to open it. See [`crate::play::pagetext`],
/// which explains why a sign gets no packet and still opens the same window.
pub(super) fn gameobject_pagetext(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(guid) = read(
        ctx.stats,
        pkt,
        crate::play::pagetext::parse_gameobject_pagetext(&pkt.body),
    ) {
        ctx.world.note_event(PlayerEvent::GameObjectPageText { guid });
    }
}

/// `SMSG_GAMEOBJECT_CUSTOM_ANIM`: play `Custom0`..`Custom3` on a game object.
/// An `anim` of 4 or more is read and dropped, as the 1.12.1 client drops it.
pub(super) fn gameobject_custom_anim(ctx: &mut Incoming, pkt: &Packet) {
    if let Some((guid, anim)) = read(ctx.stats, pkt, crate::play::object::parse_custom_anim(&pkt.body)) {
        if let Some(anim) = anim {
            ctx.world.apply_object_anim(guid, anim);
        }
    }
}

/// `SMSG_GAMEOBJECT_DESPAWN_ANIM`: play `Despawn` on a game object. The
/// server removes the object afterwards with `SMSG_DESTROY_OBJECT`.
pub(super) fn gameobject_despawn_anim(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(guid) = read(ctx.stats, pkt, crate::play::object::parse_despawn_anim(&pkt.body)) {
        ctx.world.apply_object_anim(guid, crate::play::object::ObjectAnim::Despawn);
    }
}

/// `SMSG_PAGE_TEXT_QUERY_RESPONSE`: one page of text and the id of the next
/// page.
///
/// The server answers one query with the whole chain: vmangos's
/// `HandlePageTextQueryOpcode` loops until `next_page` is zero. The pages
/// arrive in a burst, and the reader assembles them by id rather than by
/// arrival order.
pub(super) fn page_text(ctx: &mut Incoming, pkt: &Packet) {
    if let Some(page) = read(ctx.stats, pkt, crate::play::pagetext::parse_page_text(&pkt.body)) {
        ctx.world.remember(Kind::PageText, u64::from(page.id), &pkt.body);
        ctx.world.note_event(PlayerEvent::PageText(page));
    }
}

/// `SMSG_GOSSIP_POI`: the map marker placed by a guard's directions.
///
/// The only packet in the game that marks a place, and the client keeps only
/// one marker. See [`crate::play::gossip::parse_gossip_poi`], which explains why
/// a second marker replaces the first instead of being added.
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

/// `SMSG_SET_FORCED_REACTIONS`: the server overrides the friendly or hostile
/// reaction toward factions directly. It is the only mechanism that does this
/// without changing a field on any unit.
///
/// If this packet is not handled, a `SPELL_AURA_FORCE_REACTION` has no effect in
/// this client: the faction table still says friendly, the name plate stays
/// green, and the unit cannot be attacked with a click. The server uses its own
/// reaction table, so it lets the character damage the unit and the unit
/// attacks back. The aura exists to state that override.
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

/// `SMSG_EXPLORATION_EXPERIENCE`: an area was discovered. This is the only
/// packet that announces a discovery.
///
/// `PLAYER_EXPLORED_ZONES` is a `PRIVATE` update field with no event of its own,
/// so without this packet the map changes with no notification. The interface
/// re-reads the map overlays only on `WORLD_MAP_UPDATE`, so a zone map opened
/// before the bit arrived keeps showing the unexplored map until something
/// else changes the view. That is one of the two causes of the reported
/// problem that the map did not update until relog.
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

// --- Mail, trade, proficiency, spell modifiers, enchantments ----------------

/// `SMSG_TRADE_STATUS`: one state change of the trade, as a status code. See
/// [`crate::play::trade`]. The window is `client/src/game/session/trade.rs`.
pub(super) fn trade_status(ctx: &mut Incoming, pkt: &Packet) {
    let Some(status) = read(ctx.stats, pkt, crate::play::trade::parse_trade_status(&pkt.body))
    else {
        return;
    };
    ctx.world.note_event(PlayerEvent::TradeStatus(status));
}

/// `SMSG_TRADE_STATUS_EXTENDED`: one side's complete trade offer. The items are
/// given as entries and the panel shows names, so every entry is queried here,
/// as a vendor's items are. `want_item` adds it to the query list, the template
/// arrives a round trip later, and the window refreshes when it does.
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

/// `SMSG_SET_PROFICIENCY`: the complete proficiency mask for one item class.
/// Without it every item in the bags is drawn as usable, so the player sees a
/// weapon they cannot equip as equippable and learns otherwise only from the
/// server's refusal a round trip later.
pub(super) fn proficiency(ctx: &mut Incoming, pkt: &Packet) {
    let Some(said) = read(ctx.stats, pkt, crate::play::skills::parse_proficiency(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::Proficiency(said));
}

/// `SMSG_SET_FLAT_SPELL_MODIFIER` and `SMSG_SET_PCT_SPELL_MODIFIER`: the
/// running total for one spell modifier bit. A talent that shortens a cast
/// reaches the client only through this packet. If it is dropped, the cast bar
/// and the tooltip show the value without the talent for the whole session.
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

/// `SMSG_ENCHANTMENTLOG`: an enchantment was applied or expired. The item's own
/// fields change in an update block with no announcement, so this packet is the
/// only source for the combat log line.
pub(super) fn enchantment_log(ctx: &mut Incoming, pkt: &Packet) {
    let Some(log) = read(ctx.stats, pkt, crate::play::items::parse_enchantment_log(&pkt.body))
    else {
        return;
    };
    // The log line names the item, and the name comes from an item template
    // this session may not have queried yet.
    ctx.world.want_item(log.item_entry);
    ctx.world.note_combat(crate::play::combatlog::CombatEvent::Enchantment(log));
}

/// `SMSG_MAIL_LIST_RESULT`: the whole inbox. See [`crate::play::mail`], which
/// has the header layout and the union in its sender field.
///
/// It replaces the stored inbox rather than updating it, because the wire has
/// no per-letter update: each of the six mail actions is answered by a bare
/// result code that names a mail id and nothing else. A client that updated its
/// own copy from those codes would diverge from the server as soon as anything
/// else changed the mailbox.
pub(super) fn mail_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(ctx.stats, pkt, crate::play::mail::parse_mail_list(&pkt.body)) else {
        return;
    };
    ctx.world.note_event(PlayerEvent::MailList(Box::new(list)));
}

/// `SMSG_SEND_MAIL_RESULT`: the result shared by seven mail actions. Whether
/// the trailing fields are present depends on both of its leading words; see
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
/// The body is one word and vmangos always writes zero, so only the arrival
/// matters. It drives the envelope icon on the minimap: the client responds by
/// sending `MSG_QUERY_NEXT_MAIL_TIME` again, and the answer turns the icon on.
pub(super) fn mail_received(ctx: &mut Incoming) {
    ctx.world.note_event(PlayerEvent::MailReceived);
}

/// `MSG_QUERY_NEXT_MAIL_TIME`: seconds until the next letter; 0 means a letter
/// is already waiting.
///
/// The sign of the value carries the answer, and it reads the opposite way from
/// what the name suggests; see
/// [`crate::play::mail::parse_next_mail_time`], which describes how the 1.12.1
/// client treats a value at or near zero.
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

/// `SMSG_ITEM_TEXT_QUERY_RESPONSE`: the text of one letter.
///
/// It is named for items rather than mail because the same query also answers
/// for a book or a signpost's page. In 1.12 the only thing here that queries it
/// is an open letter, and the id it echoes is `MailHeader::item_text_id`.
pub(super) fn item_text(ctx: &mut Incoming, pkt: &Packet) {
    let Some((id, text)) = read(ctx.stats, pkt, crate::play::mail::parse_item_text(&pkt.body))
    else {
        return;
    };
    ctx.world.remember(Kind::ItemText, u64::from(id), &pkt.body);
    ctx.world.note_event(PlayerEvent::ItemText { id, text });
}

/// `SMSG_QUESTUPDATE_ADD_ITEM`: progress on an item objective.
///
/// Two words, an item entry and an increment, and no quest id; see
/// [`crate::play::quest::parse_quest_item`]. An item objective's counter is
/// computed from the bags, so without this handler the tracker row updated but
/// the progress message was never shown.
pub(super) fn quest_item(ctx: &mut Incoming, pkt: &Packet) {
    let Some((entry, added)) = read(ctx.stats, pkt, crate::play::quest::parse_quest_item(&pkt.body))
    else {
        return;
    };
    // The bag count is read here and carried in the event, not looked up
    // later.
    //
    // The 1.12.1 client shows `min(bags + added, required)`. The addition is
    // needed because the packet arrives before the item is in the bags, so the
    // bag count alone is one behind on every pickup. The formula is correct
    // only if `bags` is the count at the moment this packet is handled: here,
    // on the session thread, before any object update the same server tick
    // sends. Read a frame later from the renderer's copy of the inventory, the
    // item may or may not have arrived, which produced "6/8" twice and then
    // "8/8" for the seventh item.
    let have = crate::play::items::Inventory::read(ctx.world).count_of(entry);
    ctx.world
        .note_event(PlayerEvent::QuestItem { entry, added, have });
}
