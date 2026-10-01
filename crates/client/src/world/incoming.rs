//! The system that turns server packets about the player into game state.
//!
//! `LiveSession` hands the renderer a queue of [`PlayerEvent`]s: everything
//! the server said about this character that is an edge rather than a field.
//! One system, [`drain_events`], drains the queue and forwards each event to
//! the module that owns its subject. It has about sixty match arms and
//! forwards to thirteen sibling modules.
//!
//! ## Why the drain is a module of its own
//!
//! The drain used to be in [`crate::interface::action`]. Every subject added
//! to this directory after it (the flight map, the trainer, the party, the
//! mirror timers) was then an edit to a file named after the action bar. The
//! project applies one rule to this: when adding a subject requires editing a
//! file about every other subject, that file is restructured. The same rule
//! already split `lib.rs`'s plugin list and `hud.rs`'s numbers.
//!
//! Adding a subject still adds an arm here. This module is the client-side
//! counterpart of `vale_protocol::socket::handler`, whose note gives the
//! reason for keeping a single dispatch: there is exactly one place an opcode
//! becomes an action. The difference is that this file is about the fan-out,
//! so the edit is on-subject. Moving the drain out made the action bar 450
//! lines shorter.
//!
//! ## What stays in `action`
//!
//! The system's place in the frame. `drain_events` is registered inside
//! `ActionPlugin`'s chain, with the events before the input, so a cooldown the
//! server has just stated is in place before this frame's key press is
//! checked. That ordering belongs to the action chain. Registering the system
//! here would split one ordering across two modules, and the chain is written
//! as one call to prevent that.

use crate::interface::action::{
    disarm_auto_repeat, disarm_next_swing, ActionEvents, AutoRepeat, Casting, Cooldowns,
};
use crate::interface::events::{
    ActionbarSlotChanged, PlayerLevelUp, SpellcastChannelStart, SpellcastChannelStop,
    SpellcastChannelUpdate, SpellcastDelayed, SpellcastFailed, SpellcastInterrupted,
    SpellcastStart, SpellcastStop,
};
use crate::interface::messages::UiErrors;
use crate::assets::GameAssets;
use crate::world::session::Session;
use vale_protocol::play::spells::{self, action_kind, PlayerEvent};
use bevy::prelude::*;
use std::time::Duration;

/// Message writers grouped only to stay under Bevy's sixteen-parameter limit.
///
/// [`NpcAnswers`] groups writers that share a subject. These do not: they are
/// the loot window, the quest log, the party and the others below. They are
/// bundled because `drain_events` reached the sixteen-parameter limit of a
/// system when the party writer was added. The grouping makes no claim that
/// they belong together.
#[derive(bevy::ecs::system::SystemParam)]
pub struct SubjectAnswers<'w> {
    pub loot: MessageWriter<'w, crate::interface::loot::LootAnswer>,
    pub loot_roll: MessageWriter<'w, crate::interface::lootroll::RollAnswer>,
    pub quest: MessageWriter<'w, crate::interface::quest::QuestAnswer>,
    pub party: MessageWriter<'w, crate::interface::party::PartyAnswer>,
    /// The reputation panel's four events, bundled for the same reason. See
    /// [`crate::interface::reputation`].
    pub reputation: MessageWriter<'w, crate::interface::reputation::ReputationAnswer>,
    /// The social panel's four events, bundled for the same reason. See
    /// [`crate::interface::social`].
    pub social: MessageWriter<'w, crate::interface::social::SocialAnswer>,
    /// The chat channels' two events. See [`crate::interface::channels`].
    pub channels: MessageWriter<'w, crate::interface::channels::ChannelAnswer>,
    /// A text emote. See [`crate::interface::emotetext`].
    pub emotes: MessageWriter<'w, crate::interface::emotetext::EmoteAnswer>,
    /// A measurement rather than a subject, bundled here for the same
    /// sixteen-parameter reason. See [`super::desync`]: the measurement is taken
    /// on the frame the refusal arrives, so the refusal is forwarded from here.
    pub range: MessageWriter<'w, super::desync::RangeRefused>,
    /// The profession windows, bundled for the same reason. See
    /// [`crate::interface::tradeskill`]: the two SHOW events and the repeat
    /// counter both depend on which of the player's casts released or failed.
    pub profession: MessageWriter<'w, crate::interface::tradeskill::ProfessionNews>,
    /// A sound the server asked the client to play, bundled for the same reason.
    /// See [`crate::interface::events::SoundPushed`]: the three `SMSG_PLAY_*`
    /// sound opcodes have no subject in this directory. They change nothing about
    /// the character, open no panel, and their only reader is `sound::pushed`.
    pub sound: MessageWriter<'w, crate::interface::events::SoundPushed>,
    /// An item arriving in a bag, bundled for the same reason. See
    /// [`crate::interface::received`].
    pub received: MessageWriter<'w, crate::interface::received::ItemReceived>,
    /// The pet's two sounds, bundled for the same reason as `sound`: they change
    /// nothing about the character and their only reader is `sound::combat`.
    pub pet_talk: MessageWriter<'w, crate::interface::events::PetTalkHeard>,
    pub pet_dismiss: MessageWriter<'w, crate::interface::events::PetDismissHeard>,
    /// A duel, a summon and `/played`. See [`crate::interface::duel`],
    /// [`crate::interface::summon`] and [`crate::interface::played`]. `/played`
    /// is written as the event itself, because the answer changes no state the
    /// client keeps.
    pub duel: MessageWriter<'w, crate::interface::duel::DuelAnswer>,
    pub summon: MessageWriter<'w, crate::interface::summon::SummonAnswer>,
    pub played: MessageWriter<'w, crate::interface::events::TimePlayedMsg>,
    /// The inspected player's honor tab. See [`crate::interface::inspect`].
    pub inspect: MessageWriter<'w, crate::interface::inspect::InspectAnswer>,
    /// A message-table line named by a packet with no body. See
    /// [`crate::interface::messages::TableLine`], which explains why this is not
    /// `Announce`.
    pub table_line: MessageWriter<'w, crate::interface::messages::TableLine>,
    /// The two statements the server makes about what the character can do. See
    /// [`CapabilityAnswers`].
    pub caps: CapabilityAnswers<'w>,
}

/// What the character may equip, and what their talents do to their spells.
///
/// Bundled because [`drain_events`] is at Bevy's sixteen-parameter limit. The
/// limit applies to a system's arguments, so a new writer has to go inside an
/// existing group rather than beside it. `SystemParam` structs have no such
/// limit.
///
/// The two share a subject: both are the server stating a capability rather
/// than an event, both replace the previous value rather than accumulate, and
/// neither comes from a shipped file. Each keeps its own message and its own
/// module.
#[derive(bevy::ecs::system::SystemParam)]
pub struct CapabilityAnswers<'w> {
    /// `SMSG_SET_PROFICIENCY` — see [`super::proficiency`].
    pub proficiency: MessageWriter<'w, super::proficiency::ProficiencyAnswer>,
    /// `SMSG_SET_*_SPELL_MODIFIER` — see [`super::spellmods`].
    pub spell_mods: MessageWriter<'w, super::spellmods::SpellModAnswer>,
    /// The spell modifiers as they currently stand. One arm below reads them: a
    /// cooldown started locally when the player's own `SMSG_SPELL_GO` returns
    /// must include talent modifiers, or it is wrong for its whole duration.
    ///
    /// This is a `Res` in a bundle of writers because [`drain_events`] is at
    /// Bevy's sixteen-parameter limit and this bundle already has the subject. It
    /// reads the previous frame's value. The systems that fold the modifier
    /// messages into the resource run in the same `Update`, and a modifier
    /// arriving on the same frame as a cast release has no defined order relative
    /// to it, so the previous frame's value is correct.
    pub held: Res<'w, super::spellmods::SpellMods>,
}

/// Bundled for the same sixteen-parameter reason as [`SubjectAnswers`]. These
/// writers do share a subject: `SMSG_TRANSFER_PENDING` says a transfer is
/// happening and `SMSG_TRANSFER_ABORTED` says it is not. The two are mutually
/// exclusive and their arms in the drain are three lines apart.
///
/// The two world-map writers were added here when the seventeenth parameter
/// was needed. They are on the same subject: each is the server saying where
/// the character is or is going. That covers the transfer, a place discovered
/// on the way, and the point of interest a guard marks on the map.
#[derive(bevy::ecs::system::SystemParam)]
pub struct TransferAnswers<'w> {
    pub aborted: MessageWriter<'w, super::areatrigger::TransferAborted>,
    pub pending: MessageWriter<'w, crate::glue::loading::TransferPending>,
    pub discovered: MessageWriter<'w, crate::interface::worldmap::Discovered>,
    pub poi: MessageWriter<'w, crate::interface::worldmap::PoiAnswer>,
}

/// The three windows a right-click on an NPC opens, and the flight master's
/// window, which opens the same way.
///
/// Bundled for the same reason as [`SubjectAnswers`]: [`drain_events`] had
/// reached Bevy's sixteen-parameter limit when the seventeenth parameter was
/// added. These writers also share a subject.
#[derive(bevy::ecs::system::SystemParam)]
pub struct NpcAnswers<'w> {
    pub gossip: MessageWriter<'w, crate::interface::gossip::GossipAnswer>,
    pub merchant: MessageWriter<'w, crate::interface::merchant::MerchantAnswer>,
    pub trainer: MessageWriter<'w, crate::interface::trainer::TrainerAnswer>,
    pub taxi: MessageWriter<'w, crate::interface::taxi::TaxiAnswer>,
    /// The fifth: the mailbox, opened the same way on an object rather than a
    /// person. See [`crate::interface::mail`].
    pub mail: MessageWriter<'w, crate::interface::mail::MailAnswer>,
    /// The sixth: the stable, one gossip option deeper than the others. See
    /// [`crate::interface::stable`].
    pub stable: MessageWriter<'w, crate::interface::stable::StableAnswer>,
    /// The seventh: the innkeeper's bind, a popup rather than a window. Its
    /// answer arrives after the gossip window has closed. See
    /// [`crate::interface::binder`].
    pub binder: MessageWriter<'w, crate::interface::binder::BinderAnswer>,
    /// The eighth: the pet untrainer's question, which also arrives after its
    /// gossip window has closed. See [`crate::interface::untrainer`].
    pub untrainer: MessageWriter<'w, crate::interface::untrainer::UntrainerAnswer>,
    /// The ninth: the trade window, which belongs to another player rather than
    /// an NPC but uses the same `"npc"` unit token. See
    /// [`crate::interface::trade`].
    pub trade: MessageWriter<'w, crate::interface::trade::TradeAnswer>,
    /// The tenth: the bank, whose window is identified by a guid and whose
    /// contents are update fields. See [`crate::interface::bank`].
    pub bank: MessageWriter<'w, crate::interface::bank::BankAnswer>,
    /// The eleventh: the text on a sign, a plaque or a book. See
    /// [`crate::interface::pagetext`].
    pub pagetext: MessageWriter<'w, crate::interface::pagetext::PageTextAnswer>,
}

/// Drains the session's event queue and forwards each event to the module
/// that owns its subject.
///
/// The system has sixteen parameters, which is Bevy's limit, and three of
/// them are bundles that exist because of the limit. The module comment
/// explains why the drain is a module rather than a function in another one.
///
/// The queue has exactly one reader. An event drained here and not forwarded
/// is seen by nothing else, so every arm ends in a write rather than in a
/// decision.
///
/// Clippy's `too_many_arguments` warning is deliberately left enabled. It
/// predates this module and is the one automatic signal that this function
/// is a hub, so silencing it would hide the property the module keeps.
pub(crate) fn drain_events(
    session: Res<Session>,
    assets: Res<GameAssets>,
    mut cooldowns: ResMut<Cooldowns>,
    mut casting: ResMut<Casting>,
    mut auto_repeat: ResMut<AutoRepeat>,
    // `UiErrors`, not [`crate::interface::messages::Announce`]. Both groups of
    // keys this system shows go to the red error frame either way. The cast
    // failures are `SPELL_FAILED_*` keys and have no row in the message table.
    // All nine attack refusals have rows, and each says red frame with no sound
    // (checked with `vale messages ERR_BADATTACKPOS`). Using the table would
    // change nothing here, and this system is already at Bevy's
    // sixteen-parameter limit.
    mut errors: UiErrors,
    mut events: ActionEvents,
    mut level_up: MessageWriter<PlayerLevelUp>,
    mut logout: MessageWriter<crate::interface::logout::LogoutAnswer>,
    mut death: MessageWriter<crate::interface::death::DeathAnswer>,
    mut refused: MessageWriter<crate::interface::items::ItemRefused>,
    mut mirror: MessageWriter<crate::interface::timers::MirrorTimerAnswer>,
    mut transfer: TransferAnswers,
    mut subjects: SubjectAnswers,
    mut npc: NpcAnswers,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // `incoming` is the session thread's queue of what the server said;
    // `events` is what this module announces to the interface. Both were once
    // named `events`, and the shadowing looked like a bug in a match arm.
    let incoming = active.live.take_events();
    if incoming.is_empty() {
        return;
    }
    let tables = assets.display_tables().ok();
    let catalog = tables.as_deref().and_then(|t| t.spellbook());
    let player = active
        .live
        .world()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .player_guid;

    for event in incoming {
        match event {
            // Accepted means the server has answered the request, so the pending record
            // ends here. It does not start anything: `Spell::cast` sends this when the
            // cast time finishes, so for a 2.5-second spell it arrives 2.5 seconds after
            // the `SMSG_SPELL_START` that showed the bar. The bar is not touched here.
            PlayerEvent::CastAccepted { .. } => {
                casting.pending = None;
            }
            PlayerEvent::CastFailed { spell_id, reason, requirement } => {
                if casting.spell_id == spell_id {
                    casting.end();
                }
                // Disarm the queued swing as well. It is a different slot refused by the
                // same packet: not enough rage, the target moved away, or the server dropped
                // it. See [`Casting::next_swing`].
                disarm_next_swing(&mut casting, spell_id, &mut events);
                // Disarm the auto-repeat as well. A refused press is the one end of an
                // auto-repeat that no packet announces; see
                // [`crate::interface::action::disarm_auto_repeat`] for the full reasoning.
                // The server refuses the press without starting the repeat, so it never
                // sends the `SMSG_CANCEL_AUTO_REPEAT` that ends it in every other case.
                // Without this, Auto Shot pressed with no target stayed lit for the rest of
                // the session.
                disarm_auto_repeat(&mut auto_repeat, spell_id, &mut events);
                // Clear the pending record whatever spell it is for. The 1.12.1 client
                // clears it here too. The clear must not depend on the id: vmangos'
                // `SendCastResult` sends the original spell of a triggered chain, so a
                // pending cast kept because the id did not match would leave the button
                // unusable until another answer arrived.
                casting.pending = None;
                events.cast_failed.write(SpellcastFailed);
                // Tell the repeat counter, so that it stops pressing a button the server has
                // refused. See [`crate::interface::tradeskill`].
                subjects.profession.write(
                    crate::interface::tradeskill::ProfessionNews::Failed { spell_id },
                );
                // The cast never reached its `SPELL_GO`, so the only state to undo is the
                // global cooldown it started locally.
                cooldowns.clear_gcd(spell_id);
                events.cooldown_moved();
                // Record an out-of-range refusal on the frame it arrives.
                // `SPELL_FAILED_OUT_OF_RANGE` means the server places the two units further
                // apart than this client does. [`super::desync`] records the measurement.
                // It is written before the error text is shown; the text is for the player,
                // and the measurement is for later analysis.
                if reason == spells::SPELL_FAILED_OUT_OF_RANGE {
                    subjects.range.write(super::desync::RangeRefused {
                        spell_id: Some(spell_id),
                        // The range limit the server applied. Only this system has it: the spell
                        // catalogue is already loaded here and `desync` holds no tables.
                        range_yards: catalog
                            .and_then(|c| c.info(spell_id))
                            .map(|info| info.range_yards),
                    });
                }
                if let Some(key) = spells::cast_failure_key(reason) {
                    // Two layers: the reason's own key, unless it is one of the dozen keys the
                    // client replaces by power type.
                    let power = catalog
                        .and_then(|c| c.info(spell_id))
                        .map_or(0, |info| info.power_type);
                    let key = vale_assets::tables::spellbook::failure_override(key, power).unwrap_or(key);
                    // A third layer: three of those keys contain a `%s`, and the server sends
                    // the argument. Shown through `key` alone they read "Must have a %s equipped
                    // in the main hand", which was reported for warriors. A requirement that
                    // cannot be named falls back to the plain key rather than to an empty
                    // substitution, since the sentence still names the hand.
                    let named = requirement.and_then(|req| {
                        tables
                            .as_deref()?
                            .item_tables()
                            .requirement_name(req.class, req.subclass_mask)
                    });
                    match named {
                        Some(what) => errors.formatted(key, what),
                        None => errors.key(key),
                    }
                }
            }
            // The server accepted the cast, so the cast bar is shown here.
            //
            // The bar is not shown at the key press; see [`Casting`]. Its length is the
            // server's `m_timer`, not `Spell.dbc`'s base cast time, so haste and every
            // talent this client does not model are already included.
            //
            // An instant cast raises no `SPELLCAST_START`, although the packet arrives
            // for it. The game's `CastingBarFrame.lua` shows why: its handler sets
            // `maxValue = GetTime() + arg2/1000` and `this.casting = 1`, so with `arg2`
            // zero the bar shows at min == max and `OnUpdate` never clears `casting`.
            // `Casting::begin` applies the same zero-length check to its own clock.
            PlayerEvent::CastStarted { spell_id, cast_time_ms } => {
                let name = catalog
                    .and_then(|c| c.info(spell_id))
                    .map_or_else(|| format!("spell {spell_id}"), |info| info.label());
                casting.begin(spell_id, name.clone(), cast_time_ms);
                if cast_time_ms > 0 {
                    // `SPELLCAST_START`'s argument order: the name, then the milliseconds.
                    events.cast_start.write(SpellcastStart {
                        name,
                        duration_ms: cast_time_ms,
                    });
                }
            }
            // The release of one of the player's spells. It ends the cast bar, starts
            // the spell's own cooldown, and is the only thing that empties a queued
            // swing. See [`vale_protocol::play::spells::PlayerEvent::CastReleased`].
            //
            // It also clears the pending record, which an instant cast's press waits
            // on: an instant gets a `SPELL_START` of zero length and then this packet,
            // and neither puts anything on the bar.
            //
            // The bar and the cooldown are ended here, once per packet. They used to be
            // ended by a system that polled the player entity's release counter and then
            // read `last_spell` to find which spell it was. `last_spell` is one field, so
            // a second release in the same poll overwrote it. `Entity::recent_spells`
            // exists because this happens: Charge produces two releases inside one 25 ms
            // tick, measured against a live server, and every effect that triggers a
            // second spell does the same (a paladin's seal proc on every swing, a
            // warrior's, an item's). The animation code was moved to the ring of recent
            // spells; the cast bar still read the single field.
            //
            // When the cast bar read the wrong spell, nothing else cleared `started`, so
            // `in_progress` stayed true for the rest of the session. Every later press
            // was refused with `SPELL_FAILED_SPELL_IN_PROGRESS` ("another action is in
            // progress"). A per-packet event carries its own spell id, so it cannot read
            // the wrong spell.
            PlayerEvent::CastReleased { spell_id } => {
                // The profession windows first: an opening spell's release shows one, and a
                // recipe's release is what the repeat counter counts. See
                // [`crate::interface::tradeskill`].
                subjects.profession.write(
                    crate::interface::tradeskill::ProfessionNews::Released { spell_id },
                );
                disarm_next_swing(&mut casting, spell_id, &mut events);
                if casting.pending == Some(spell_id) {
                    casting.pending = None;
                }
                // The 1.12.1 client starts a spell's cooldown on this packet, for this
                // packet's spell rather than for whichever spell was released last.
                if let Some(info) = catalog.and_then(|c| c.info(spell_id)) {
                    cooldowns.start_recovery(&info, &subjects.caps.held);
                    events.cooldown_moved();
                }
                // A channelled spell's release is its start, not its end. `SMSG_SPELL_GO`
                // arrives as soon as an Evocation is pressed and `MSG_CHANNEL_START` follows
                // it. Ending the bar here would remove the channel bar that is about to
                // appear, or, when the two packets arrive in the other order, the one
                // already shown. A channel ends with `MSG_CHANNEL_UPDATE` carrying zero.
                if casting.channelling.is_some() {
                    continue;
                }
                // Only a bar that was running raises a stop. An instant cast raises no
                // `SPELLCAST_START` and must raise no `SPELLCAST_STOP` either:
                // `CastingBarFrame_OnEvent`'s stop branch acts on whatever bar is shown, so
                // a stop for a cast that never had a bar colours and fades another one.
                if casting.spell_id == spell_id && casting.end() {
                    events.cast_stop.write(SpellcastStop);
                }
            }
            PlayerEvent::CastInterrupted { guid, spell_id } => {
                if player == Some(guid) {
                    disarm_next_swing(&mut casting, spell_id, &mut events);
                }
                if player == Some(guid) && casting.spell_id == spell_id {
                    let channelled = casting.channelling == Some(spell_id);
                    casting.end();
                    // An interrupted channel ends through its own event: the interrupt branch of
                    // `CastingBarFrame_OnEvent` checks `not this.channeling` and would do
                    // nothing.
                    if channelled {
                        events.channel_stop.write(SpellcastChannelStop);
                    } else {
                        events.cast_interrupted.write(SpellcastInterrupted);
                    }
                }
            }
            // A pushback lengthens the cast rather than ending it, so it has its own
            // arm. `SMSG_SPELL_DELAYED` means the server's `m_timer` has moved. The
            // client's bar was started at the key press from `Spell.dbc`'s base cast
            // time, and no other packet restates the time. Without this arm, a Fireball
            // pushed back twice showed a full bar for a second while the server was
            // still casting, which was reported as "stuck casting a spell that was
            // interrupted".
            //
            // A channel is not pushed back this way. vmangos' `DelayedChannel` shortens
            // `m_timer` and sends `MSG_CHANNEL_UPDATE`, which is handled below.
            // `CastingBarFrame_OnEvent`'s `SPELLCAST_DELAYED` branch moves a casting bar
            // and would compute the wrong value for a channelling one.
            PlayerEvent::CastDelayed { delay_ms } => {
                if casting.delay(delay_ms) {
                    events.cast_delayed.write(SpellcastDelayed { delay_ms });
                }
            }
            // A channel is the one cast whose length arrives after it has started. See
            // [`PlayerEvent::ChannelStart`]: `SMSG_SPELL_GO` arrives at once and carries
            // no duration, so until this packet arrives there is nothing to show on the
            // bar. Without it, an Evocation ran for eight seconds with the cast bar
            // empty.
            PlayerEvent::ChannelStart { spell_id, duration_ms } => {
                casting.spell_id = spell_id;
                casting.name = catalog
                    .and_then(|c| c.info(spell_id))
                    .map_or_else(|| format!("spell {spell_id}"), |info| info.label());
                casting.duration = Duration::from_millis(u64::from(duration_ms));
                // `started` stays empty: a channel has no cast time, and `progress` must
                // not read one. See [`Casting::channelling`].
                casting.started = None;
                casting.channelling = Some(spell_id);
                events.channel_start.write(SpellcastChannelStart {
                    duration_ms,
                    name: casting.name.clone(),
                });
            }
            // Zero means the channel ended, interrupted or completed. Any other value
            // restates the time left, which moves the bar rather than restarting it.
            PlayerEvent::ChannelUpdate { remaining_ms } => {
                if casting.channelling.is_none() {
                    // An update for a channel this client never saw start. The bar has nothing
                    // to move, and `CastingBarFrame` would turn whatever is on it green.
                    continue;
                }
                if remaining_ms == 0 {
                    casting.end();
                    events.channel_stop.write(SpellcastChannelStop);
                } else {
                    casting.duration = Duration::from_millis(u64::from(remaining_ms));
                    events
                        .channel_update
                        .write(SpellcastChannelUpdate { remaining_ms });
                }
            }
            // The melee form of the same refusal, which is a whole opcode rather than a
            // reason byte. See [`super::desync`].
            PlayerEvent::AttackRefused(refusal) => {
                if refusal == spells::AttackRefusal::NotInRange {
                    // No `range_yards`: a melee swing's limit is not a table value but depends
                    // on both units' size. See `desync::melee_reach`.
                    subjects.range.write(super::desync::RangeRefused {
                        spell_id: None,
                        range_yards: None,
                    });
                }
                errors.key(refusal.key());
            }
            // The one packet that ends an auto-repeat, whatever ended it. See
            // [`AutoRepeat`]: the player's own press, the target dying, moving out of
            // range and a wand user moving all arrive here, so the client's own cancel
            // is handled on the same line as the four it cannot predict.
            PlayerEvent::AutoRepeatCancelled => auto_repeat.stop(&mut events),
            // The pet's four refusals. Two of them are not pet keys. The client maps the
            // byte to a message id, and two of the four map to the ordinary
            // attack-refusal rows. See [`vale_protocol::play::pet::feedback::key`],
            // which holds the mapping. A byte outside 1..4 shows nothing.
            //
            // `UiErrors` rather than `Announce`, for the reason given on this system's
            // `errors` parameter: all six pet keys here are `Surface::Error` with no
            // sound, so the message table would change nothing.
            PlayerEvent::PetFeedback(message) => {
                if let Some(key) = vale_protocol::play::pet::feedback::key(message) {
                    errors.key(key);
                }
            }
            // The pet's cast failure, which is the pet's own `SMSG_CAST_RESULT` and
            // indexes the same 146-entry table.
            PlayerEvent::PetCastFailed { spell_id, reason } => {
                // The same 146-entry table that `SMSG_CAST_RESULT` indexes, and the same
                // two-layer lookup: the reason's own key, unless the power type replaces it.
                // Three of the keys contain a `%s` that the pet's packet has no argument
                // for, so the plain key is shown. The 1.12.1 client also shows no argument.
                if let Some(key) = spells::cast_failure_key(reason) {
                    let power = catalog
                        .and_then(|c| c.info(spell_id))
                        .map_or(0, |info| info.power_type);
                    let key = vale_assets::tables::spellbook::failure_override(key, power)
                        .unwrap_or(key);
                    errors.key(key);
                }
            }
            // A tame failure always shows a message: a reason out of range falls back to
            // `PETTAME_UNKNOWNERROR`. The sentence is substituted into
            // `ERR_TAME_FAILED`, whose text is `"%s."`.
            PlayerEvent::PetTameFailure(reason) => {
                let key = vale_protocol::play::pet::tame_failure::key(reason);
                errors.substituted("ERR_TAME_FAILED", key);
            }
            // The 1.12.1 client shows one message for this opcode.
            PlayerEvent::PetBroken => errors.key("ERR_PET_BROKEN"),
            // The server rejected the pet's name. The packet has no body, so the opcode
            // is the message, as with the five attack-swing refusals.
            // `ERR_PET_NOT_RENAMEABLE` is the message-table row for it, which is how the
            // rename box's failure reads. The 1.12.1 client also re-opens the rename
            // box, which needs a `StaticPopup` this client does not raise yet.
            PlayerEvent::PetNameInvalid => errors.key("ERR_PET_NOT_RENAMEABLE"),
            // The pet trainer's question. Its gossip window has already closed when it
            // arrives, as with the innkeeper's bind. See
            // [`crate::interface::untrainer`], which holds the pet's guid until the
            // player presses Accept.
            PlayerEvent::PetUnlearnConfirm { pet, cost } => {
                npc.untrainer
                    .write(crate::interface::untrainer::UntrainerAnswer { pet, cost });
            }
            // The pet's two sounds. They have no subject in this directory; as with
            // `PlaySound` below, the only reader is `sound::combat`.
            PlayerEvent::PetTalk { pet, talk } => {
                subjects.pet_talk.write(crate::interface::events::PetTalkHeard { pet, talk });
            }
            PlayerEvent::PetDismissSound(sound) => {
                subjects.pet_dismiss.write(crate::interface::events::PetDismissHeard(sound));
            }
            // This arm changes no state. A pushed sound is not a fact about the
            // character, a panel or the bar; it is a sound the server chose, and it is
            // forwarded to `sound::pushed`. It passes through this system rather than
            // being read from the session queue by the sound module because the queue
            // has exactly one reader, which is this system.
            PlayerEvent::PlaySound(cue) => {
                subjects.sound.write(crate::interface::events::SoundPushed(cue));
            }
            // The only packet that says an item arrived in a bag. It is forwarded whole
            // rather than filtered here: a loot message is broadcast to the group, so
            // whose bag received the item is part of the event. See
            // [`crate::interface::received`].
            PlayerEvent::ItemReceived(push) => {
                subjects.received.write(crate::interface::received::ItemReceived(push));
            }
            PlayerEvent::CooldownStarted { spell_id, ms } => {
                // Pass the spell's own cooldown categories when the catalogue has them. See
                // `Cooldowns::set`, which describes the bug caused by recording the
                // cooldown without them.
                cooldowns.set(spell_id, ms, catalog.and_then(|c| c.info(spell_id)).as_ref());
                events.cooldown_moved();
            }
            PlayerEvent::CooldownReleased { spell_id } => {
                cooldowns.release(spell_id);
                events.cooldown_moved();
            }
            // The opposite packet: a school lockout lifted early, or any other removal.
            // Without this arm, the cooldown sweep ran for the length the start packet
            // stated. That was correct when a lockout ran its full length and wrong when
            // it did not, which was reported as the cooldown displaying inconsistently.
            PlayerEvent::CooldownCleared { spell_id } => {
                cooldowns.clear(spell_id);
                events.cooldown_moved();
            }
            PlayerEvent::SpellLearned(_) | PlayerEvent::SpellRemoved(_) => {}
            // A spell rank was replaced, and the server has to be told about the action
            // bar change. The spellbook side is already done in the object manager: it
            // incremented `spellbook_version`, which rebuilds the book and raises
            // `SPELLS_CHANGED`. The action bar is client state that the server only
            // stores, so a replacement that is not sent back is undone at the next
            // login and leaves the button dead. See [`PlayerEvent::SpellSuperceded`].
            PlayerEvent::SpellSuperceded { new, slots, .. } => {
                for slot in slots {
                    // The world's copy already holds `new`. This goes through the same method
                    // as every other action bar change, so there is one place a slot is sent to
                    // the socket. Repeating the local update is harmless.
                    active.live.set_action_button(slot, new, Some(action_kind::SPELL));
                    // Zero-based on the wire, one-based in the interface; the same conversion
                    // [`crate::interface::cursor`]'s `commit` makes.
                    events.slot_changed.write(ActionbarSlotChanged(slot + 1));
                }
            }
            // Forwarded, not handled here. A level-up is not an answer to a key press
            // and has nothing to do with the action bar. It arrives through this system
            // only because `take_events` is a queue with one reader. Everything that
            // uses it (the chat line, the glow) reads the message.
            PlayerEvent::LevelUp(gained) => {
                level_up.write(PlayerLevelUp(gained));
            }
            // Forwarded for the same reason: the logout answer belongs to
            // [`crate::interface::logout`], and this system is the only reader of the
            // queue it arrives on.
            PlayerEvent::Logout(answer) => {
                logout.write(crate::interface::logout::LogoutAnswer(answer));
            }
            // The four death events, forwarded the same way to
            // [`crate::interface::death`], which owns the two timers and the three
            // edges.
            PlayerEvent::CorpseReclaimDelay { ms } => {
                death.write(crate::interface::death::DeathAnswer::ReclaimDelay(ms));
            }
            PlayerEvent::CorpseLocated(place) => {
                death.write(crate::interface::death::DeathAnswer::Corpse(place));
            }
            PlayerEvent::ResurrectOffered(offer) => {
                death.write(crate::interface::death::DeathAnswer::Offer(offer));
            }
            PlayerEvent::SpiritHealerOffered { healer } => {
                death.write(crate::interface::death::DeathAnswer::SpiritHealer(healer));
            }
            // The refusal of an item action, forwarded the same way to
            // [`crate::interface::items`]. The swing refusal above is handled in this
            // system because swings belong to the action module; items do not.
            PlayerEvent::InventoryFailed(failure) => {
                refused.write(crate::interface::items::ItemRefused(failure));
            }
            // The loot window's five events, forwarded the same way to
            // [`crate::interface::loot`]. Loot is not this module's subject, and one of
            // the five is a refusal that uses the same opcode as an answer; that
            // distinction is handled in the loot module.
            PlayerEvent::LootOpened(window) => {
                subjects.loot.write(crate::interface::loot::LootAnswer::Opened(window));
            }
            PlayerEvent::LootRemoved { index } => {
                subjects.loot.write(crate::interface::loot::LootAnswer::Removed(index));
            }
            PlayerEvent::LootMoneyCleared => {
                subjects.loot.write(crate::interface::loot::LootAnswer::MoneyCleared);
            }
            PlayerEvent::LootMoneyGained { copper } => {
                subjects.loot.write(crate::interface::loot::LootAnswer::MoneyGained(copper));
            }
            PlayerEvent::LootRollStarted(start) => {
                subjects
                    .loot_roll
                    .write(crate::interface::lootroll::RollAnswer::Started(start));
            }
            PlayerEvent::LootRollCast(cast) => {
                subjects
                    .loot_roll
                    .write(crate::interface::lootroll::RollAnswer::Cast(cast));
            }
            PlayerEvent::LootRollWon(won) => {
                subjects
                    .loot_roll
                    .write(crate::interface::lootroll::RollAnswer::Won(won));
            }
            PlayerEvent::LootRollAllPassed(passed) => {
                subjects
                    .loot_roll
                    .write(crate::interface::lootroll::RollAnswer::AllPassed(passed));
            }
            PlayerEvent::LootClosed { guid } => {
                subjects.loot.write(crate::interface::loot::LootAnswer::Closed(guid));
            }
            // The quest family's eleven events, forwarded the same way. See
            // [`crate::interface::quest`]. The quest status is about another unit and
            // the world already stores it, so it is not forwarded;
            // `render::questmarks` reads the world instead.
            PlayerEvent::QuestStatus { .. } => {}
            PlayerEvent::QuestGreeting(page) => {
                subjects.quest.write(crate::interface::quest::QuestAnswer::Greeting(page));
            }
            PlayerEvent::QuestDetails(page) => {
                subjects.quest.write(crate::interface::quest::QuestAnswer::Details(page));
            }
            PlayerEvent::QuestProgress(page) => {
                subjects.quest.write(crate::interface::quest::QuestAnswer::Progress(page));
            }
            PlayerEvent::QuestReward(page) => {
                subjects.quest.write(crate::interface::quest::QuestAnswer::Reward(page));
            }
            PlayerEvent::QuestComplete(done) => {
                subjects.quest.write(crate::interface::quest::QuestAnswer::Complete(done));
            }
            PlayerEvent::QuestTemplate(template) => {
                subjects.quest.write(crate::interface::quest::QuestAnswer::Template(template));
            }
            PlayerEvent::QuestKill(kill) => {
                subjects.quest.write(crate::interface::quest::QuestAnswer::Kill(kill));
            }
            // The item objective's own packet, which had a name but no dispatch before
            // this arm. See [`crate::interface::quest::item_progress`].
            PlayerEvent::QuestItem { entry, added, have } => {
                subjects
                    .quest
                    .write(crate::interface::quest::QuestAnswer::Item { entry, added, have });
            }
            PlayerEvent::QuestObjectivesDone { quest_id } => {
                subjects.quest.write(crate::interface::quest::QuestAnswer::ObjectivesDone(quest_id));
            }
            PlayerEvent::QuestFailed { quest_id, timed_out } => {
                subjects.quest.write(crate::interface::quest::QuestAnswer::Failed { quest_id, timed_out });
            }
            PlayerEvent::QuestRefused { reason } => {
                subjects.quest.write(crate::interface::quest::QuestAnswer::Refused(reason));
            }
            // The NPC windows' seven events, split between their two modules.
            PlayerEvent::GossipShow(menu) => {
                npc.gossip.write(crate::interface::gossip::GossipAnswer::Show(menu));
            }
            PlayerEvent::GossipClosed => {
                npc.gossip.write(crate::interface::gossip::GossipAnswer::Closed);
            }
            // A gossip point of interest outlives the window: the flag a guard's
            // directions put on the map stays after the conversation ends, so it goes to
            // the map rather than to the gossip panel. See
            // [`crate::interface::worldmap::MapLandmarks`].
            PlayerEvent::GossipPoi { flags, position, icon, data, name } => {
                transfer.poi.write(crate::interface::worldmap::PoiAnswer {
                    poi: vale_assets::tables::areapoi::GossipPoi {
                        flags,
                        position,
                        icon,
                        data,
                        name: name.clone(),
                    },
                });
            }
            PlayerEvent::NpcText { text_id, text } => {
                npc.gossip.write(crate::interface::gossip::GossipAnswer::Text { text_id, text });
            }
            // The one gossip option whose answer arrives after the window has closed.
            // See [`crate::interface::binder`].
            PlayerEvent::BinderConfirm { guid } => {
                npc.binder.write(crate::interface::binder::BinderAnswer::Confirm { guid });
            }
            PlayerEvent::PlayerBound { guid, area_id } => {
                npc.binder.write(crate::interface::binder::BinderAnswer::Bound { guid, area_id });
            }
            // A duel, in six packets. See [`crate::interface::duel`], which decides
            // what each one means.
            PlayerEvent::DuelRequested { arbiter, initiator } => {
                subjects
                    .duel
                    .write(crate::interface::duel::DuelAnswer::Requested { arbiter, initiator });
            }
            PlayerEvent::DuelCountdown { ms } => {
                subjects.duel.write(crate::interface::duel::DuelAnswer::Countdown { ms });
            }
            PlayerEvent::DuelBounds { out } => {
                subjects.duel.write(crate::interface::duel::DuelAnswer::Bounds { out });
            }
            PlayerEvent::DuelComplete { started } => {
                subjects.duel.write(crate::interface::duel::DuelAnswer::Complete { started });
            }
            PlayerEvent::DuelWinner(winner) => {
                subjects.duel.write(crate::interface::duel::DuelAnswer::Winner(winner));
            }
            PlayerEvent::SummonRequest(request) => {
                subjects.summon.write(crate::interface::summon::SummonAnswer(request));
            }
            PlayerEvent::PlayedTime { total, level } => {
                subjects.played.write(crate::interface::events::TimePlayedMsg { total, level });
            }
            PlayerEvent::InspectHonor(honor) => {
                subjects.inspect.write(crate::interface::inspect::InspectAnswer(honor));
            }
            // Each of these shows one message-table line and does nothing else. The
            // 1.12.1 client shows opcode 456 as message 318 and opcode 457 as message
            // 319, both on the yellow frame.
            PlayerEvent::Fish { escaped } => {
                subjects.table_line.write(crate::interface::messages::TableLine(if escaped {
                    "ERR_FISH_ESCAPED"
                } else {
                    "ERR_FISH_NOT_HOOKED"
                }));
            }
            PlayerEvent::VendorShow(vendor) => {
                npc.merchant.write(crate::interface::merchant::MerchantAnswer::Show(vendor));
            }
            PlayerEvent::VendorSold { slot, left, .. } => {
                npc.merchant.write(crate::interface::merchant::MerchantAnswer::Sold { slot, left });
            }
            PlayerEvent::BuyFailed { entry, reason } => {
                npc.merchant.write(crate::interface::merchant::MerchantAnswer::BuyFailed { entry, reason });
            }
            PlayerEvent::SellFailed { item, reason } => {
                npc.merchant.write(crate::interface::merchant::MerchantAnswer::SellFailed { item, reason });
            }
            // The trainer's three events, with the same structure.
            PlayerEvent::TrainerShow(list) => {
                npc.trainer.write(crate::interface::trainer::TrainerAnswer::Show(list));
            }
            PlayerEvent::TrainerBought { spell } => {
                npc.trainer.write(crate::interface::trainer::TrainerAnswer::Bought { spell });
            }
            PlayerEvent::TrainerBuyFailed { spell, reason } => {
                npc.trainer.write(crate::interface::trainer::TrainerAnswer::BuyFailed { spell, reason });
            }
            // The stable master's two events. The result byte is not routed by the
            // request it answers, because it does not say which. See
            // [`crate::interface::stable`], which requests the list again on any
            // success.
            PlayerEvent::StableList(list) => {
                npc.stable.write(crate::interface::stable::StableAnswer::List(list));
            }
            PlayerEvent::StableResult { result, .. } => {
                npc.stable.write(crate::interface::stable::StableAnswer::Result(result));
            }
            // The banker's two events. The bank's contents are not here: they are update
            // fields, and the inventory diff raises their events. See
            // [`crate::interface::bank`].
            PlayerEvent::BankShow(banker) => {
                npc.bank.write(crate::interface::bank::BankAnswer::Show(banker));
            }
            PlayerEvent::BankSlotResult { result, .. } => {
                npc.bank.write(crate::interface::bank::BankAnswer::SlotResult(result));
            }
            // The flight master's four events. The flight itself is not here: it
            // arrives as `SMSG_MONSTER_MOVE` and goes to the movement code, so these
            // events cover only the window. See [`crate::interface::taxi`].
            PlayerEvent::TaxiShow(menu) => {
                npc.taxi.write(crate::interface::taxi::TaxiAnswer::Show(menu));
            }
            PlayerEvent::TaxiNodeStatus { guid, known } => {
                npc.taxi.write(crate::interface::taxi::TaxiAnswer::NodeStatus { guid, known });
            }
            PlayerEvent::NewTaxiPath => {
                npc.taxi.write(crate::interface::taxi::TaxiAnswer::Discovered);
            }
            PlayerEvent::TaxiReply(reply) => {
                npc.taxi.write(crate::interface::taxi::TaxiAnswer::Reply(reply));
            }
            // The three mirror timers the server counts for the player (handled after
            // the two transfer arms below). They belong to no action. See
            // [`crate::interface::timers`], which resolves the caption and holds the
            // argument lists.
            // The only packet that says an instance portal was refused deliberately,
            // forwarded the same way to [`super::areatrigger`].
            PlayerEvent::TransferAborted { reason } => {
                transfer
                    .aborted
                    .write(super::areatrigger::TransferAborted(reason));
            }
            // The packet that says a transfer is happening. It is forwarded because it
            // arrives on the last frame before the current world is unloaded. See
            // [`crate::glue::loading`].
            PlayerEvent::TransferPending { map_id } => {
                transfer
                    .pending
                    .write(crate::glue::loading::TransferPending(map_id));
            }
            PlayerEvent::MirrorTimerStarted(started) => {
                mirror.write(crate::interface::timers::MirrorTimerAnswer::Started(started));
            }
            PlayerEvent::MirrorTimerStopped { timer } => {
                mirror.write(crate::interface::timers::MirrorTimerAnswer::Stopped(timer));
            }
            PlayerEvent::MirrorTimerPaused { timer, paused } => {
                mirror.write(crate::interface::timers::MirrorTimerAnswer::Paused { timer, paused });
            }
            // The group's eight events, all forwarded to the one module that holds a
            // roster. See [`crate::interface::party`], and [`crate::interface::raid`]
            // for the ready check.
            PlayerEvent::GroupInvite { name } => {
                subjects.party.write(crate::interface::party::PartyAnswer::Invited(name));
            }
            PlayerEvent::GroupDecline { name } => {
                subjects.party.write(crate::interface::party::PartyAnswer::Declined(name));
            }
            PlayerEvent::GroupList(list) => {
                subjects.party.write(crate::interface::party::PartyAnswer::List(list));
            }
            PlayerEvent::GroupDestroyed => {
                subjects.party.write(crate::interface::party::PartyAnswer::Destroyed);
            }
            PlayerEvent::GroupNewLeader { name } => {
                subjects.party.write(crate::interface::party::PartyAnswer::NewLeader(name));
            }
            PlayerEvent::PartyResult(result) => {
                subjects.party.write(crate::interface::party::PartyAnswer::Result(result));
            }
            PlayerEvent::PartyMemberStats(stats) => {
                subjects.party.write(crate::interface::party::PartyAnswer::Stats(stats));
            }
            PlayerEvent::RaidReadyCheck(check) => {
                subjects.party.write(crate::interface::party::PartyAnswer::ReadyCheck(check));
            }
            // The reputation panel's events. One arm rather than one per event, because
            // the per-event `match` is in the reputation module and this system does not
            // need to know which event it is. See
            // [`crate::interface::reputation::answer_of`].
            ref event @ (PlayerEvent::FactionsInitialized(_)
            | PlayerEvent::FactionStandings(_)
            | PlayerEvent::FactionVisible { .. }
            | PlayerEvent::FactionAtWar { .. }
            | PlayerEvent::ForcedReactions(_)) => {
                if let Some(answer) = crate::interface::reputation::answer_of(event) {
                    subjects.reputation.write(answer);
                }
            }
            // The social panel's four events are folded the same way, for the same
            // reason; see [`crate::interface::social::answer_of`].
            // The mailbox's five events are folded the same way; see
            // [`crate::interface::mail::answer_of`].
            // The trade window's two events; see [`crate::interface::trade`].
            PlayerEvent::TradeStatus(status) => {
                npc.trade.write(crate::interface::trade::TradeAnswer::Status(status));
            }
            // Both replace the previous value rather than accumulate, and both modules'
            // notes explain why: a proficiency packet carries that item class's whole
            // mask, and a modifier packet carries that bit's running total. See
            // [`super::proficiency`] and [`super::spellmods`].
            PlayerEvent::Proficiency(said) => {
                subjects
                    .caps
                    .proficiency
                    .write(super::proficiency::ProficiencyAnswer(said));
            }
            PlayerEvent::SpellModifier(modifier) => {
                subjects
                    .caps
                    .spell_mods
                    .write(super::spellmods::SpellModAnswer(modifier));
            }
            PlayerEvent::TradeOffer(offer) => {
                npc.trade.write(crate::interface::trade::TradeAnswer::Offer(offer));
            }
            ref event @ (PlayerEvent::GameObjectPageText { .. } | PlayerEvent::PageText(_)) => {
                if let Some(answer) = crate::interface::pagetext::answer_of(event) {
                    npc.pagetext.write(answer);
                }
            }
            ref event @ (PlayerEvent::MailList(_)
            | PlayerEvent::MailResult(_)
            | PlayerEvent::MailReceived
            | PlayerEvent::MailNextTime(_)
            | PlayerEvent::ItemText { .. }) => {
                if let Some(answer) = crate::interface::mail::answer_of(event) {
                    npc.mail.write(answer);
                }
            }
            ref event @ (PlayerEvent::FriendList(_)
            | PlayerEvent::IgnoreList(_)
            | PlayerEvent::FriendStatus(_)
            | PlayerEvent::WhoResults(_)) => {
                if let Some(answer) = crate::interface::social::answer_of(event) {
                    subjects.social.write(answer);
                }
            }
            ref event @ (PlayerEvent::ChannelNotify(_) | PlayerEvent::ChannelList(_)) => {
                if let Some(answer) = crate::interface::channels::answer_of(event) {
                    subjects.channels.write(answer);
                }
            }
            ref event @ PlayerEvent::TextEmote(_) => {
                if let Some(answer) = crate::interface::emotetext::answer_of(event) {
                    subjects.emotes.write(answer);
                }
            }
            // Shows "Discovered: Stranglethorn Vale" in the game's own strings.
            //
            // Two keys, chosen by whether the area gave experience:
            // `ERR_ZONE_EXPLORED_XP` is "Discovered %s: %d experience gained" and
            // `ERR_ZONE_EXPLORED` is "Discovered: %s". vmangos sends the packet in both
            // cases, so a max-level character still gets the line.
            //
            // The two keys go to different places: `ERR_ZONE_EXPLORED` is a
            // `UI_INFO_MESSAGE` and `ERR_ZONE_EXPLORED_XP` is a system chat line. This
            // is the message table's `type` column. See
            // [`crate::interface::worldmap::announce_discovery`].
            PlayerEvent::Discovered { area, experience } => {
                // The area's own name, not its zone's. The server sends the id of the area
                // flag, which is usually a sub-area, and the 1.12.1 client shows
                // "Discovered: Northshire Valley". The zone name is the fallback for an id
                // `AreaTable` does not have.
                let name = tables
                    .as_deref()
                    .and_then(|t| t.areas())
                    .map(|areas| {
                        areas
                            .get(area)
                            .map_or_else(|| areas.zone_name(area), |row| row.name.clone())
                    })
                    .unwrap_or_default();
                if !name.is_empty() {
                    transfer
                        .discovered
                        .write(crate::interface::worldmap::Discovered { name, experience });
                }
            }
        }
    }
}
