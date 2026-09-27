//! **The one place a packet becomes game state**, and nothing else.
//!
//! `LiveSession` hands the renderer a queue of [`PlayerEvent`]s — everything
//! the server said about *this* character that is an edge rather than a field —
//! and one system drains it and fans it out to whichever subject the news is
//! about. Sixty-odd arms, thirteen sibling modules.
//!
//! ## Why it is a file of its own
//!
//! It lived in [`crate::interface::action`] for a dozen rounds, and every subject added to
//! this directory since — the flight map, the trainer, the party, the mirror
//! timers — was an edit to a file named after the action bar. This project
//! retired that shape twice already (`lib.rs`'s plugin list, `hud.rs`'s numbers) with
//! one sentence: *when adding a subject requires editing a file about every
//! other subject, that file is the thing to fix, not the subject.* This was the
//! third instance and the only one still costing a tax every round.
//!
//! **The tax does not disappear and is not meant to.** Adding a subject still
//! adds an arm here — this is `vale_protocol::socket::handler` one layer up, and
//! that file's own note says why one dispatch is worth keeping: *there is still
//! exactly one place an opcode becomes an action.* What changes is that the file
//! it is added to is now about the fan-out, so the edit is on-subject and the
//! action bar is 450 lines shorter.
//!
//! ## What stays in `action`
//!
//! Its **place in the frame**. `drain_events` is registered inside
//! `ActionPlugin`'s chain — the events before the input, so a cooldown the
//! server has just stated is in place before this frame's press is judged — and
//! that ordering is a statement about the action chain rather than about this
//! file. Moving the registration here would have split one ordering across two
//! modules, which is the failure the chain is written as one call to avoid.

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

/// **Writers with nothing in common but the sixteen-parameter limit.**
///
/// Unlike [`NpcAnswers`], whose three really are one subject a door apart, these
/// are the loot window, the quest log and the party — and they are bundled
/// because `drain_events` reached `SystemParam`'s sixteen when the party
/// arrived. Said plainly rather than dressed up as a subject: the alternative
/// was a grouping that reads like a claim about the code.
#[derive(bevy::ecs::system::SystemParam)]
pub struct SubjectAnswers<'w> {
    pub loot: MessageWriter<'w, crate::interface::loot::LootAnswer>,
    pub loot_roll: MessageWriter<'w, crate::interface::lootroll::RollAnswer>,
    pub quest: MessageWriter<'w, crate::interface::quest::QuestAnswer>,
    pub party: MessageWriter<'w, crate::interface::party::PartyAnswer>,
    /// …and the reputation panel's four, on the same terms — see
    /// [`crate::interface::reputation`].
    pub reputation: MessageWriter<'w, crate::interface::reputation::ReputationAnswer>,
    /// …and the social panel's four, on the same terms — see
    /// [`crate::interface::social`].
    pub social: MessageWriter<'w, crate::interface::social::SocialAnswer>,
    /// …and the chat channels' two — see [`crate::interface::channels`].
    pub channels: MessageWriter<'w, crate::interface::channels::ChannelAnswer>,
    /// …and a text emote — see [`crate::interface::emotetext`].
    pub emotes: MessageWriter<'w, crate::interface::emotetext::EmoteAnswer>,
    /// **Not a subject at all** — an instrument, bundled here for the same
    /// sixteen-parameter reason the other three are. See
    /// [`super::desync`]: the measurement has to be taken on the frame
    /// the refusal arrives, so the refusal has to leave this hub.
    pub range: MessageWriter<'w, super::desync::RangeRefused>,
    /// …and the profession windows', on the same terms — see
    /// [`crate::interface::tradeskill`]: the two SHOW events and the repeat
    /// counter both run on which of our casts released or failed.
    pub profession: MessageWriter<'w, crate::interface::tradeskill::ProfessionNews>,
    /// …and a noise the server asked for outright, on the same terms and with
    /// the least in common with any of them. See
    /// [`crate::interface::events::SoundPushed`]: the three `SMSG_PLAY_*` sound opcodes
    /// have no subject in this directory at all — nothing about the character
    /// changes, no panel opens, and the only reader is `sound::pushed`.
    pub sound: MessageWriter<'w, crate::interface::events::SoundPushed>,
    /// …and an item landing in a bag, on the same terms — see
    /// [`crate::interface::received`].
    pub received: MessageWriter<'w, crate::interface::received::ItemReceived>,
    /// …and the pet's two noises, on `sound`'s own terms: nothing about the
    /// character changes and the only reader is `sound::combat`.
    pub pet_talk: MessageWriter<'w, crate::interface::events::PetTalkHeard>,
    pub pet_dismiss: MessageWriter<'w, crate::interface::events::PetDismissHeard>,
    /// …and a duel, a summon and `/played` — see [`crate::interface::duel`],
    /// [`crate::interface::summon`] and [`crate::interface::played`]. The last is
    /// the event itself, because the answer changes nothing the client keeps.
    pub duel: MessageWriter<'w, crate::interface::duel::DuelAnswer>,
    pub summon: MessageWriter<'w, crate::interface::summon::SummonAnswer>,
    pub played: MessageWriter<'w, crate::interface::events::TimePlayedMsg>,
    /// …and a message-table line a bodiless packet named — see
    /// [`crate::interface::messages::TableLine`], which is why this is not `Announce`.
    pub table_line: MessageWriter<'w, crate::interface::messages::TableLine>,
    /// …and the two statements the server makes about what the character can
    /// *do*. See [`CapabilityAnswers`].
    pub caps: CapabilityAnswers<'w>,
}

/// **What the character may hold, and what their talents do to their spells.**
///
/// **Bundled because [`drain_events`] is at Bevy's sixteen parameters** — the
/// limit is on a system's *arguments*, so a new writer has to arrive inside a
/// group that is already there rather than beside it. `SystemParam` structs
/// themselves have no such bound.
///
/// They are grouped honestly rather than by convenience: both are the server
/// stating a *capability* rather than an event, both replace rather than
/// accumulate, and both are in no shipped file. Each keeps its own message and
/// its own module.
#[derive(bevy::ecs::system::SystemParam)]
pub struct CapabilityAnswers<'w> {
    /// `SMSG_SET_PROFICIENCY` — see [`super::proficiency`].
    pub proficiency: MessageWriter<'w, super::proficiency::ProficiencyAnswer>,
    /// `SMSG_SET_*_SPELL_MODIFIER` — see [`super::spellmods`].
    pub spell_mods: MessageWriter<'w, super::spellmods::SpellModAnswer>,
    /// …and the modifiers **as they stand**, which one arm below has to read
    /// rather than write: a cooldown started locally when our own
    /// `SMSG_SPELL_GO` returns is the talented one or it is wrong for as long
    /// as it runs.
    ///
    /// **A `Res` in a bundle of writers, and it is here for the count.**
    /// [`drain_events`] is at Bevy's sixteen parameters and this is the field
    /// that was already about the subject. It reads the previous frame's
    /// answer, which is right: the systems that fold these messages into the
    /// resource run in the same `Update`, and a modifier landing on the same
    /// frame as a cast release is a coincidence rather than an ordering.
    pub held: Res<'w, super::spellmods::SpellMods>,
}

/// Bundled for the same sixteen-parameter reason as [`SubjectAnswers`], and
/// unlike that one these two genuinely are a subject: `SMSG_TRANSFER_PENDING`
/// says the transfer is happening and `SMSG_TRANSFER_ABORTED` says it is not,
/// they are mutually exclusive, and their arms in the drain are three lines
/// apart.
///
/// **The two map writers joined them when the seventeenth arrived**, which is
/// the same limit this group was made for. They are the same subject read one
/// step wider: everything here is the server saying something about *where the
/// character is or is going* — the transfer, the place discovered on the way,
/// and the one place a guard can name on the map.
#[derive(bevy::ecs::system::SystemParam)]
pub struct TransferAnswers<'w> {
    pub aborted: MessageWriter<'w, super::areatrigger::TransferAborted>,
    pub pending: MessageWriter<'w, crate::glue::loading::TransferPending>,
    pub discovered: MessageWriter<'w, crate::interface::worldmap::Discovered>,
    pub poi: MessageWriter<'w, crate::interface::worldmap::PoiAnswer>,
}

/// **The three windows one right-click opens, one door apart** — and the
/// fourth, which is the same gesture at a flight master.
///
/// Bundled for the reason [`SubjectAnswers`] is, plus a hard one:
/// [`drain_events`] had reached Bevy's sixteen-parameter limit and the
/// seventeenth is what forced the grouping. These four belong together anyway.
#[derive(bevy::ecs::system::SystemParam)]
pub struct NpcAnswers<'w> {
    pub gossip: MessageWriter<'w, crate::interface::gossip::GossipAnswer>,
    pub merchant: MessageWriter<'w, crate::interface::merchant::MerchantAnswer>,
    pub trainer: MessageWriter<'w, crate::interface::trainer::TrainerAnswer>,
    pub taxi: MessageWriter<'w, crate::interface::taxi::TaxiAnswer>,
    /// …and the fifth, which is the same gesture at something that is not a
    /// person at all — see [`crate::interface::mail`].
    pub mail: MessageWriter<'w, crate::interface::mail::MailAnswer>,
    /// …and the sixth, one gossip option deeper than the rest — see
    /// [`crate::interface::stable`].
    pub stable: MessageWriter<'w, crate::interface::stable::StableAnswer>,
    /// …and the seventh, which is a *popup* rather than a window: the
    /// innkeeper's answer arrives after the gossip window has already closed —
    /// see [`crate::interface::binder`].
    pub binder: MessageWriter<'w, crate::interface::binder::BinderAnswer>,
    /// …and the eighth, the same shape one trainer over: the pet untrainer's
    /// question, whose gossip window has also closed — see
    /// [`crate::interface::untrainer`].
    pub untrainer: MessageWriter<'w, crate::interface::untrainer::UntrainerAnswer>,
    /// …and the ninth, which is not an NPC's window at all but takes the same
    /// `"npc"` token: another player's — see [`crate::interface::trade`].
    pub trade: MessageWriter<'w, crate::interface::trade::TradeAnswer>,
    /// …and the tenth, whose window is a guid and whose contents are fields
    /// — see [`crate::interface::bank`].
    pub bank: MessageWriter<'w, crate::interface::bank::BankAnswer>,
    /// …and the eleventh, whose subject is not a person at all: what is
    /// written on a sign, a plaque or a book — see [`crate::interface::pagetext`].
    pub pagetext: MessageWriter<'w, crate::interface::pagetext::PageTextAnswer>,
}

/// **Drain the session's event queue and give each piece of news to whoever
/// the news is about.**
///
/// Sixteen parameters, which is Bevy's limit, and three of them are bundles
/// that exist because of it. That is the shape of a fan-out and it is why this
/// is a file rather than a function in one — see the module comment.
///
/// The queue has exactly **one** reader: an event drained here and not passed
/// on is an event nothing else will ever see, which is why every arm ends in a
/// write rather than in a decision.
///
/// **Clippy's `too_many_arguments` is left standing on purpose.** It was there
/// before this file existed and it is the one automatic signal that this is a
/// hub; silencing it would make the move look like a fix for the thing the move
/// deliberately keeps.
pub(crate) fn drain_events(
    session: Res<Session>,
    assets: Res<GameAssets>,
    mut cooldowns: ResMut<Cooldowns>,
    mut casting: ResMut<Casting>,
    mut auto_repeat: ResMut<AutoRepeat>,
    // **Not [`crate::interface::messages::Announce`], and it was measured rather than
    // assumed.** Both key populations this system says reach the red frame
    // anyway: the cast failures are `SPELL_FAILED_*` and have no row in the
    // message table at all, and all nine attack refusals *have* rows that say
    // red with no sound (`vale messages ERR_BADATTACKPOS`). So the table
    // would change nothing here, and this system is already at Bevy's
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
    // **Named for where it comes from, not for what it is.** `incoming` is the
    // session thread's queue of things the *server* said; `events` is what this
    // module announces to the interface. They were both called `events` for one
    // compile and the shadowing read as a plausible bug in a match arm.
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
            // Accepted: the cast bar keeps running. Nothing to do, and that is
            // the point of the packet being read at all.
            // **Accepted, which is an answer and therefore the end of the
            // ask.** It is not the start of anything: `Spell::cast` sends this
            // when the wind-up *finishes*, so for a slow spell it lands two and
            // a half seconds after the `SMSG_SPELL_START` that put the bar up.
            // The bar is not touched here for that reason.
            PlayerEvent::CastAccepted { .. } => {
                casting.pending = None;
            }
            PlayerEvent::CastFailed { spell_id, reason, requirement } => {
                if casting.spell_id == spell_id {
                    casting.end();
                }
                // …and the queued swing, which is a different slot with the
                // same refusal packet: not enough rage, the target walked away,
                // the server dropped it. See [`Casting::next_swing`].
                disarm_next_swing(&mut casting, spell_id, &mut events);
                // **…and the volley**, which is the one end of an
                // auto-repeat that no packet announces — see
                // [`crate::interface::action::disarm_auto_repeat`], which is where
                // the whole argument for it is. Short version: the server
                // refuses the press without ever installing the loop, so the
                // `SMSG_CANCEL_AUTO_REPEAT` every other end arrives as is never
                // sent, and Auto Shot pressed with no target stayed lit for the
                // session.
                disarm_auto_repeat(&mut auto_repeat, spell_id, &mut events);
                // **…and the ask, whatever it was about.** The client clears the
                // pending record here too, and it must be unconditional on the
                // id: `SendCastResult` sends the *original* spell for a
                // triggered chain, so a pending cast held against a mismatched
                // id would wedge the button until something else answered.
                casting.pending = None;
                events.cast_failed.write(SpellcastFailed);
                // …and the repeat counter, which must not go on pressing a
                // button the server just refused — see
                // [`crate::interface::tradeskill`].
                subjects.profession.write(
                    crate::interface::tradeskill::ProfessionNews::Failed { spell_id },
                );
                // The cast never reached its `SPELL_GO`, so the only thing to
                // undo is the global cooldown it started locally.
                cooldowns.clear_gcd(spell_id);
                events.cooldown_moved();
                // **The one refusal worth measuring, on the frame it lands.**
                // `SPELL_FAILED_OUT_OF_RANGE` is the server saying the two of
                // us are further apart than this client thinks — see
                // [`super::desync`], which takes the reading. Recorded
                // before the sentence, because the sentence is what the player
                // sees and this is what the next round reads.
                if reason == spells::SPELL_FAILED_OUT_OF_RANGE {
                    subjects.range.write(super::desync::RangeRefused {
                        spell_id: Some(spell_id),
                        // …with the limit the server just applied, which only
                        // this side has: the catalogue is already in hand here
                        // and `desync` holds no tables of its own.
                        range_yards: catalog
                            .and_then(|c| c.info(spell_id))
                            .map(|info| info.range_yards),
                    });
                }
                if let Some(key) = spells::cast_failure_key(reason) {
                    // Two layers: the reason's own key, unless it is one of the
                    // dozen the client replaces outright.
                    let power = catalog
                        .and_then(|c| c.info(spell_id))
                        .map_or(0, |info| info.power_type);
                    let key = vale_assets::tables::spellbook::failure_override(key, power).unwrap_or(key);
                    // …and a third: three of those keys are written with a `%s`
                    // in them, and the server sends the argument. Drawn through
                    // `key` they read "Must have a %s equipped in the main
                    // hand", which is the warrior report. A requirement we
                    // cannot name falls back to the plain key rather than to an
                    // empty hole, since the sentence still says which hand.
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
            // **The server took the cast: this is where the bar goes up.**
            //
            // Not at the press — see [`Casting`], and note that the *length* is
            // the server's `m_timer` rather than `Spell.dbc`'s base cast time,
            // so haste and every talent this client does not model are already
            // in it.
            //
            // **An instant raises no `SPELLCAST_START` even though the packet
            // arrives for one**, and the game's own `CastingBarFrame.lua` is the
            // evidence: the handler sets `maxValue = GetTime() + arg2/1000` and
            // `this.casting = 1`, so with `arg2` zero the bar shows at
            // min == max and `OnUpdate` never clears `casting` again. It is the
            // same gate `Casting::begin` applies to its own clock.
            PlayerEvent::CastStarted { spell_id, cast_time_ms } => {
                let name = catalog
                    .and_then(|c| c.info(spell_id))
                    .map_or_else(|| format!("spell {spell_id}"), |info| info.label());
                casting.begin(spell_id, name.clone(), cast_time_ms);
                if cast_time_ms > 0 {
                    // `SPELLCAST_START`'s own argument order: the name, then
                    // the milliseconds.
                    events.cast_start.write(SpellcastStart {
                        name,
                        duration_ms: cast_time_ms,
                    });
                }
            }
            // **The release of one of ours** — the end of the cast bar, the
            // start of the spell's own recovery, and for a queued swing the only
            // thing that empties the queue. See
            // [`vale_protocol::play::spells::PlayerEvent::CastReleased`].
            //
            // It clears the **pending** record too, which is what an instant's
            // press is waiting on: an instant gets a `SPELL_START` of zero
            // length and then this, and neither leaves anything on the bar.
            //
            // **The bar and the cooldown are ended here, per packet, and that is
            // the whole of a bug that wedged the character.** They used to be
            // done by a system polling the player entity's release *counter* and
            // then reading `last_spell` to find out which spell it was — and
            // `last_spell` is one field, so any release landing in the same poll
            // after ours overwrote it. `Entity::recent_spells` exists because
            // that is not hypothetical: Charge is two releases inside one 25 ms
            // tick, measured against a live server, and *everything that triggers
            // a second spell has the same shape* — a paladin's seal proc on every
            // swing, a warrior's, an item's. The art path was given the ring; the
            // bar was left reading the field.
            //
            // When it lost that race nothing else ever cleared `started`, so
            // `in_progress` stayed true for the rest of the session and every
            // press after it was refused with `SPELL_FAILED_SPELL_IN_PROGRESS` —
            // "another action is in progress", stuck on the character, which is
            // exactly the report. A per-packet event carries the id it is about
            // and cannot lose that race at all.
            PlayerEvent::CastReleased { spell_id } => {
                // The profession windows first: an opening spell's release is
                // what shows one, and a recipe's is what the repeat counter
                // counts — see [`crate::interface::tradeskill`].
                subjects.profession.write(
                    crate::interface::tradeskill::ProfessionNews::Released { spell_id },
                );
                disarm_next_swing(&mut casting, spell_id, &mut events);
                if casting.pending == Some(spell_id) {
                    casting.pending = None;
                }
                // The client's own cooldown anchor is this packet,
                // and it is this packet's spell rather than whichever was last.
                if let Some(info) = catalog.and_then(|c| c.info(spell_id)) {
                    cooldowns.start_recovery(&info, &subjects.caps.held);
                    events.cooldown_moved();
                }
                // **A channel's release is its *start*, not its end.**
                // `SMSG_SPELL_GO` fires the instant an Evocation is pressed and
                // `MSG_CHANNEL_START` follows it, so ending the bar here would
                // take down the one that is about to appear — or, when the two
                // arrive in the other order, the one that already has. The
                // channel's own end is `MSG_CHANNEL_UPDATE` with zero.
                if casting.channelling.is_some() {
                    continue;
                }
                // **And only a bar that was actually running raises a stop.** An
                // instant raises no `SPELLCAST_START` and must raise no
                // `SPELLCAST_STOP` either: `CastingBarFrame_OnEvent`'s stop
                // branch acts on whatever is shown, so a stop for a cast that
                // never had a bar colours and fades whatever else is up there.
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
                    // A channel that is cut off ends through its own event: the
                    // interrupt branch of `CastingBarFrame_OnEvent` is gated on
                    // `not this.channeling` and would do nothing at all.
                    if channelled {
                        events.channel_stop.write(SpellcastChannelStop);
                    } else {
                        events.cast_interrupted.write(SpellcastInterrupted);
                    }
                }
            }
            // **A pushback lengthens the cast rather than ending it**, which is
            // why it is neither of the two arms above. `SMSG_SPELL_DELAYED` is
            // the server saying its own `m_timer` has moved; the client's bar
            // was started at the press off `Spell.dbc`'s base cast time and
            // nothing else in the wire ever restates it, so a Fireball knocked
            // back twice sat at full for a second while the server was still
            // casting — which is the "stuck casting a spell that was
            // interrupted" report seen from this side.
            //
            // **A channel is not pushed back this way.** vmangos' `DelayedChannel`
            // shortens `m_timer` and sends `MSG_CHANNEL_UPDATE`, which is the arm
            // below; `CastingBarFrame_OnEvent`'s own `SPELLCAST_DELAYED` branch
            // slides a *casting* bar and would do the wrong arithmetic to a
            // channelling one.
            PlayerEvent::CastDelayed { delay_ms } => {
                if casting.delay(delay_ms) {
                    events.cast_delayed.write(SpellcastDelayed { delay_ms });
                }
            }
            // **The channel, which is the one cast whose length arrives after
            // it has already started.** See [`PlayerEvent::ChannelStart`]: the
            // wire's `SMSG_SPELL_GO` fires at once and says nothing about a
            // duration, so until this packet lands there is nothing to put on a
            // bar — which is why an Evocation ran for eight seconds with the
            // cast bar empty.
            PlayerEvent::ChannelStart { spell_id, duration_ms } => {
                casting.spell_id = spell_id;
                casting.name = catalog
                    .and_then(|c| c.info(spell_id))
                    .map_or_else(|| format!("spell {spell_id}"), |info| info.label());
                casting.duration = Duration::from_millis(u64::from(duration_ms));
                // **Not `started`**: a channel has no wind-up and `progress`
                // must not read one. See [`Casting::channelling`].
                casting.started = None;
                casting.channelling = Some(spell_id);
                events.channel_start.write(SpellcastChannelStart {
                    duration_ms,
                    name: casting.name.clone(),
                });
            }
            // Zero is the end — an interrupted or completed channel — and
            // anything else is a restatement of the time left, which slides the
            // bar rather than restarting it.
            PlayerEvent::ChannelUpdate { remaining_ms } => {
                if casting.channelling.is_none() {
                    // An update for a channel this client never saw start. The
                    // bar has nothing to slide and `CastingBarFrame` would
                    // green out whatever *is* on it.
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
            // …and the melee half of the same verdict, which is a whole
            // opcode rather than a reason byte — see
            // [`super::desync`].
            PlayerEvent::AttackRefused(refusal) => {
                if refusal == spells::AttackRefusal::NotInRange {
                    // No `range_yards`: a swing's limit is not a table lookup
                    // but both units' own bulk — see `desync::melee_reach`.
                    subjects.range.write(super::desync::RangeRefused {
                        spell_id: None,
                        range_yards: None,
                    });
                }
                errors.key(refusal.key());
            }
            // **The one packet that ends a volley, whoever ended it.** See
            // [`AutoRepeat`]: the press, the target dying, walking out of
            // range and a wand-user moving all come through here, so the
            // client's own cancel is answered on the same line as the four it
            // could not have predicted.
            PlayerEvent::AutoRepeatCancelled => auto_repeat.stop(&mut events),
            // **The pet's four refusals, and the two of them that are not pet
            // keys at all.** The client maps the byte to a message id and two
            // of the four are the ordinary attack-refusal rows — see
            // [`vale_protocol::play::pet::feedback::key`], which carries the
            // table. A byte outside 1..4 says nothing.
            //
            // `UiErrors` rather than `Announce` for the reason this system's
            // own parameter note gives: all six pet keys here are
            // `Surface::Error` with no sound, so the message table would change
            // nothing.
            PlayerEvent::PetFeedback(message) => {
                if let Some(key) = vale_protocol::play::pet::feedback::key(message) {
                    errors.key(key);
                }
            }
            // …and the cast, which is the pet's own `SMSG_CAST_RESULT` and
            // indexes the same 146-entry table.
            PlayerEvent::PetCastFailed { spell_id, reason } => {
                // The same 146-entry table `SMSG_CAST_RESULT` indexes, and the
                // same two-layer lookup: the reason's own key, unless the power
                // type replaces it. Three of the keys carry a `%s` the *pet's*
                // packet has no argument for, so the plain key is drawn — the
                // reference has the same hole.
                if let Some(key) = spells::cast_failure_key(reason) {
                    let power = catalog
                        .and_then(|c| c.info(spell_id))
                        .map_or(0, |info| info.power_type);
                    let key = vale_assets::tables::spellbook::failure_override(key, power)
                        .unwrap_or(key);
                    errors.key(key);
                }
            }
            // **A tame always says something** — a reason out of range falls
            // to `PETTAME_UNKNOWNERROR` rather than to silence. The sentence is substituted into `ERR_TAME_FAILED`, which
            // is the whole of `"%s."`.
            PlayerEvent::PetTameFailure(reason) => {
                let key = vale_protocol::play::pet::tame_failure::key(reason);
                errors.substituted("ERR_TAME_FAILED", key);
            }
            // The whole handler is one message id.
            PlayerEvent::PetBroken => errors.key("ERR_PET_BROKEN"),
            // **The server did not like the name.** An empty body, so the
            // opcode is the message — the same shape the five attack-swing
            // refusals have. `ERR_PET_NOT_RENAMEABLE` is the row the message
            // table carries for it, which is what the rename box's failure
            // reads as; the reference re-opens the box, which wants a
            // `StaticPopup` this client does not raise yet.
            PlayerEvent::PetNameInvalid => errors.key("ERR_PET_NOT_RENAMEABLE"),
            // **The pet trainer's question**, whose gossip window has already
            // closed by the time it arrives — the same arrangement the
            // innkeeper's bind has, one trainer over. See
            // [`crate::interface::untrainer`], which holds the pet's guid between
            // this and the person pressing Accept.
            PlayerEvent::PetUnlearnConfirm { pet, cost } => {
                npc.untrainer
                    .write(crate::interface::untrainer::UntrainerAnswer { pet, cost });
            }
            // **The pet's two noises** — edges with no subject in this
            // directory, on the same terms as `PlaySound` below: the only
            // reader is `sound::combat`.
            PlayerEvent::PetTalk { pet, talk } => {
                subjects.pet_talk.write(crate::interface::events::PetTalkHeard { pet, talk });
            }
            PlayerEvent::PetDismissSound(sound) => {
                subjects.pet_dismiss.write(crate::interface::events::PetDismissHeard(sound));
            }
            // **The one arm here that changes nothing.** A pushed sound is not
            // a fact about the character, a panel or the bar — it is a noise
            // the server decided on, and it goes straight back out to
            // `sound::pushed`. It passes through this hub rather than being
            // read off the session queue in the sound directory because there
            // is exactly one drain of that queue, which is this file's whole
            // argument.
            PlayerEvent::PlaySound(cue) => {
                subjects.sound.write(crate::interface::events::SoundPushed(cue));
            }
            // **The only packet that says something arrived in a bag.** Handed
            // on whole rather than filtered here: a loot is broadcast to the
            // group, so whose bag it went into is part of the news. See
            // [`crate::interface::received`].
            PlayerEvent::ItemReceived(push) => {
                subjects.received.write(crate::interface::received::ItemReceived(push));
            }
            PlayerEvent::CooldownStarted { spell_id, ms } => {
                // **With the spell's own categories when the catalogue has
                // them** — see `Cooldowns::set`, where the record this used to
                // make without them is the bug.
                cooldowns.set(spell_id, ms, catalog.and_then(|c| c.info(spell_id)).as_ref());
                events.cooldown_moved();
            }
            PlayerEvent::CooldownReleased { spell_id } => {
                cooldowns.release(spell_id);
                events.cooldown_moved();
            }
            // **…and the opposite one**, which nothing read until this round:
            // a school lockout lifted early, or any other removal. Without it
            // the swirl runs to the length the *start* packet stated — right
            // whenever a lockout runs its course and wrong whenever it does
            // not, which is the "displays inconsistently" report.
            PlayerEvent::CooldownCleared { spell_id } => {
                cooldowns.clear(spell_id);
                events.cooldown_moved();
            }
            PlayerEvent::SpellLearned(_) | PlayerEvent::SpellRemoved(_) => {}
            // **A rank was replaced, and the bar has to be told the server
            // about it.** The book half already happened in the object manager
            // — it bumped `spellbook_version`, which is what rebuilds the book
            // and raises `SPELLS_CHANGED` — but the *bar* is client state that
            // the server only stores, so a swap that is not sent back is one
            // that comes undone at the next login and leaves the button dead
            // again. See [`PlayerEvent::SpellSuperceded`].
            PlayerEvent::SpellSuperceded { new, slots, .. } => {
                for slot in slots {
                    // The world's copy already holds `new`; this goes through
                    // the same door every other bar change does so that there
                    // is one place a slot reaches the socket, and it is
                    // idempotent on the local half.
                    active.live.set_action_button(slot, new, Some(action_kind::SPELL));
                    // Zero-based on the wire, one-based in the interface — the
                    // same crossing [`crate::interface::cursor`]'s `commit` makes.
                    events.slot_changed.write(ActionbarSlotChanged(slot + 1));
                }
            }
            // **Forwarded rather than acted on here.** The level-up is not an
            // answer to a press and has nothing to do with the bar; it arrives
            // through this drain only because `take_events` is a queue with one
            // reader, and everything that cares about it — the chat line, the
            // glow — reads the message.
            PlayerEvent::LevelUp(gained) => {
                level_up.write(PlayerLevelUp(gained));
            }
            // …and the same forwarding for the same reason: what the server
            // said about leaving is [`crate::interface::logout`]'s, and this is the one
            // drain of the queue it arrives on.
            PlayerEvent::Logout(answer) => {
                logout.write(crate::interface::logout::LogoutAnswer(answer));
            }
            // …and the four about dying, forwarded on exactly the same terms to
            // [`crate::interface::death`], which owns the two clocks and the three edges.
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
            // …and the refusal an item verb gets, forwarded to
            // [`crate::interface::items`] on exactly the same terms: the swing's refusal
            // above is answered here because a swing is this module's subject,
            // and an item is not.
            PlayerEvent::InventoryFailed(failure) => {
                refused.write(crate::interface::items::ItemRefused(failure));
            }
            // …and the loot window's five, forwarded to [`crate::interface::loot`] on the
            // same terms: what is on a body is not this module's subject, and
            // one of these five is a *refusal* wearing the same opcode as an
            // answer — which is a distinction that belongs beside the window it
            // is about.
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
            // …and the quest family's eleven, on the same terms — see
            // [`crate::interface::quest`]. The status is the odd one: it is about *another
            // unit* and the world already keeps it, so nothing here forwards
            // it and `render::questmarks` reads the world instead.
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
            // **…and the item objective's own packet**, which was named and
            // never dispatched — see [`crate::interface::quest::item_progress`].
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
            // …and the NPC windows' seven, split between their two modules.
            PlayerEvent::GossipShow(menu) => {
                npc.gossip.write(crate::interface::gossip::GossipAnswer::Show(menu));
            }
            PlayerEvent::GossipClosed => {
                npc.gossip.write(crate::interface::gossip::GossipAnswer::Closed);
            }
            // **…and the one that outlives the window**: the flag a guard's
            // directions put on the map stays after the conversation ends, so
            // it goes to the map rather than to the gossip panel. See
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
            // …and the one gossip option whose answer arrives after the window
            // has shut — see [`crate::interface::binder`].
            PlayerEvent::BinderConfirm { guid } => {
                npc.binder.write(crate::interface::binder::BinderAnswer::Confirm { guid });
            }
            PlayerEvent::PlayerBound { guid, area_id } => {
                npc.binder.write(crate::interface::binder::BinderAnswer::Bound { guid, area_id });
            }
            // **A duel, in six packets** — see [`crate::interface::duel`], where
            // what each one says is decided.
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
            // **Two lines of the message table and nothing else** — the
            // reference's shared handler sends 456 to message 318 and
            // 457 to 319, both on the yellow frame.
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
            // …and the trainer's three, which are the same shape again.
            PlayerEvent::TrainerShow(list) => {
                npc.trainer.write(crate::interface::trainer::TrainerAnswer::Show(list));
            }
            PlayerEvent::TrainerBought { spell } => {
                npc.trainer.write(crate::interface::trainer::TrainerAnswer::Bought { spell });
            }
            PlayerEvent::TrainerBuyFailed { spell, reason } => {
                npc.trainer.write(crate::interface::trainer::TrainerAnswer::BuyFailed { spell, reason });
            }
            // …and the stable master's two. **The result byte is not routed by
            // what it answers**, because it does not say — see
            // [`crate::interface::stable`], which re-asks on any success.
            PlayerEvent::StableList(list) => {
                npc.stable.write(crate::interface::stable::StableAnswer::List(list));
            }
            PlayerEvent::StableResult { result, .. } => {
                npc.stable.write(crate::interface::stable::StableAnswer::Result(result));
            }
            // …and the banker's two. What is *in* the bank is not here: it is
            // fields, and the inventory diff raises its events — see
            // [`crate::interface::bank`].
            PlayerEvent::BankShow(banker) => {
                npc.bank.write(crate::interface::bank::BankAnswer::Show(banker));
            }
            PlayerEvent::BankSlotResult { result, .. } => {
                npc.bank.write(crate::interface::bank::BankAnswer::SlotResult(result));
            }
            // …and the flight master's four. **What its verb buys is not
            // here**: a flight arrives as `SMSG_MONSTER_MOVE` and goes to the
            // mover, so this family is the window and nothing else. See
            // [`crate::interface::taxi`].
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
            // …and the three bars the server counts for us, which are nobody's
            // action at all — see [`crate::interface::timers`], which is where the caption
            // is resolved and the argument lists are.
            // …and the one thing that ever says an instance portal was refused
            // on purpose, forwarded on the same terms to [`super::areatrigger`].
            PlayerEvent::TransferAborted { reason } => {
                transfer
                    .aborted
                    .write(super::areatrigger::TransferAborted(reason));
            }
            // …and the one that says a transfer *is* happening, which is
            // forwarded for one reason only: it is the last frame before the
            // world it announces the end of goes away. See [`crate::glue::loading`].
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
            // …and the group's eight, all of them forwarded to the one module
            // that holds a roster — see [`crate::interface::party`], and
            // [`crate::interface::raid`] for what the last of them is about.
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
            // **The four the reputation panel is made of.** One arm rather than
            // four, because the fan-out is a `match` in the subject's own file
            // and this hub has no opinion about which of them it is — see
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
            // **…and the four the social panel is made of**, folded the same
            // way and for the same reason — see
            // [`crate::interface::social::answer_of`].
            // **…and the five the mailbox is made of**, folded the same way —
            // see [`crate::interface::mail::answer_of`].
            // …and the trade window's two — see [`crate::interface::trade`].
            PlayerEvent::TradeStatus(status) => {
                npc.trade.write(crate::interface::trade::TradeAnswer::Status(status));
            }
            // **Both replace rather than accumulate**, and both modules'
            // own notes say why: a proficiency packet is that class's whole
            // mask, and a modifier packet is that bit's running total. See
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
            // **"Discovered: Stranglethorn Vale"**, in the game's own words.
            //
            // Two keys, chosen on whether the place paid anything:
            // `ERR_ZONE_EXPLORED_XP` is "Discovered %s: %d experience gained"
            // and `ERR_ZONE_EXPLORED` is "Discovered: %s" — and vmangos sends
            // the packet either way, so a max-level character still gets the
            // line.
            //
            // **The two keys go to two different windows** — `ERR_ZONE_EXPLORED`
            // is `UI_INFO_MESSAGE` and `ERR_ZONE_EXPLORED_XP` is a system chat
            // line, which is the message table's `+0x04` column and is read
            // rather than guessed. This note used to say the destination field
            // had not been read out and that a chat line was what a player
            // sees; half of that was wrong. See
            // [`crate::interface::worldmap::announce_discovery`]'s own comment.
            PlayerEvent::Discovered { area, experience } => {
                // **The area's own name, not its zone's** — the server sends
                // the id behind the area *flag*, which is usually a sub-area,
                // and "Discovered: Northshire Valley" is what the reference
                // says. The zone is the fallback for an id `AreaTable` does not
                // have.
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
