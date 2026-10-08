//! Dispatch of every received world packet this client handles, in one match.
//!
//! [`apply_packet`] is the only place an opcode is mapped to a handler.
//!
//! Earlier, [`apply_packet`] took only an [`ObjectManager`], so it could not
//! reply to the server or move the local player. The seven packets that need
//! one of those were handled by the live session loop before it, each ending in
//! an early return, and nothing recorded which packets were handled where.
//! One effect: a `SMSG_FORCE_RUN_SPEED_CHANGE` about another unit was
//! acknowledged and then dropped, so it never reached the world state.
//! `Entity::speeds` is written only by an update block's movement section, so a
//! hasted or slowed player nearby kept their old speed, and
//! [`ObjectManager::advance`] dead-reckoned them at it until the server sent a
//! full movement block. The correction then arrived on a `MSG_MOVE_*` broadcast
//! and showed as the unit jumping to a new position. The dispatch is now one
//! match so that every packet reaches the world state.
//!
//! ## Replies are queued in `Replies`, not written to the socket
//!
//! Handlers run with the world lock held. A socket write under that lock blocks
//! every reader for the duration of the network round trip; `resolve_names`
//! copies its work list out before touching the socket for the same reason. A
//! handler therefore pushes its reply into [`Replies`], and the caller flushes
//! it after releasing the lock. This also lets `WorldSession::pump`, which has
//! no session loop, send replies. It acknowledges a far teleport, without which
//! the server holds the player on no map.
//!
//! ## `LocalState` is an optional parameter
//!
//! [`LocalState`] is the client's own player: the [`Mover`], the map it is on,
//! and the session clock that stamps `ctime`. `pump` passes `None`, and the
//! handlers that need local state do the part that does not depend on it. A
//! snapshot has no simulation to resync, but the world-state update and the
//! acknowledgement are the same either way.
//!
//! ## Handler bodies live in child modules
//!
//! Each arm of the match is one call into a `pub(super) fn` in [`world`],
//! [`query`], [`chat`], [`player`] or [`acks`]. The match stays in this file as
//! the dispatch table, so there is still one place where an opcode becomes an
//! action. The child modules hold only what each arm does.
//!
//! The reason is size. Twenty-one opcodes at about fourteen lines per arm is a
//! readable match. A 1.12 client dispatches more than 150 opcodes, and the same
//! style would give a match of about two thousand lines. With the bodies moved
//! out, the table grows by one line per packet.
//!
//! Each handler's notes on what a mishandled packet costs are also rustdoc
//! documentation now: a `//` comment inside a match arm is not documented, and
//! a doc comment on a `pub(super) fn` is.

use crate::state::movement::{self, Mover};
use crate::state::objects::ObjectManager;
use crate::opcodes::Opcode;
use crate::state::update;
use crate::socket::world::{OpcodeFlow, Packet};
use std::collections::BTreeMap;
use std::time::Instant;

mod acks;
mod chat;
mod player;
mod query;
mod world;

/// How many parse warnings to keep. A field that is mis-read on every packet
/// would otherwise produce thousands of identical lines, and later copies add
/// no information.
const MAX_WARNINGS: usize = 10;

/// Replies a handler wants sent, collected rather than written.
///
/// See the module comment: a handler runs under the world lock, and the socket
/// write has to happen after it is released.
#[derive(Debug, Default)]
pub struct Replies(Vec<(Opcode, Vec<u8>)>);

impl Replies {
    pub fn push(&mut self, opcode: Opcode, body: Vec<u8>) {
        self.0.push((opcode, body));
    }

    /// Take everything queued, leaving the buffer empty and reusable.
    pub fn take(&mut self) -> Vec<(Opcode, Vec<u8>)> {
        std::mem::take(&mut self.0)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The client's own player, as the packet handlers need to see it.
///
/// Split out of the session loop so that the handlers which move the local
/// character (a teleport, a far teleport, a forced speed change) are ordinary
/// arms of the one dispatch rather than a second one.
pub struct LocalState {
    /// The local simulation. A server correction always overrides dead
    /// reckoning.
    pub mover: Mover,
    /// Which map the character is on. `SMSG_NEW_WORLD` changes it, and the
    /// ground lookup is bound to it. A lookup left pointing at the previous map
    /// does not fail; it returns heights from the wrong map.
    pub map_id: u32,
    /// Session clock, and the source of `MovementInfo::time`.
    pub started: Instant,
    /// Round-trip time of the last `CMSG_PING`.
    pub latency_ms: u32,
    /// When the outstanding ping went out.
    pub ping_sent_at: Option<Instant>,
    /// How many times a packet has relocated the player.
    ///
    /// A counter for the same reason `Entity::position_updates` is one. The
    /// caller re-baselines its own record of where the server has the player,
    /// and it cannot tell a relocation it made from one it did not by comparing
    /// positions, because the mover has already adopted the new position.
    pub relocations: u32,
    /// The guid the movement packets are about: the mover, and the guid
    /// `CMSG_SET_ACTIVE_MOVER` last named.
    ///
    /// It is the character's own guid for an ordinary session, a possessed
    /// unit's guid while Eye of Kilrogg, Mind Control or Eyes of the Beast
    /// lasts, and zero while the server has taken control away without giving
    /// it to another unit (a fear or a confusion). The 1.12.1 client also
    /// enters the zero state: it records a zero guid and sends no
    /// `CMSG_SET_ACTIVE_MOVER` for it.
    ///
    /// Derived each tick rather than latched from a packet: see
    /// [`crate::socket::session::SessionLoop::tick_view`], which follows the
    /// client's per-frame view update. [`Self::mover`] simulates whatever unit
    /// this names.
    pub mover_guid: u64,
    /// Whether the client has told the server it is looking through another
    /// object. The client's view update keeps this latch, and it stops
    /// `CMSG_FAR_SIGHT` being sent on every tick of a far sight.
    pub looking_through: bool,
    /// Whether the transfer in flight began on a transport, as the two extra
    /// dwords of `SMSG_TRANSFER_PENDING` say. vmangos'
    /// `Player::ExecuteTeleportFar` writes them only for a character standing
    /// on a transport.
    ///
    /// Read only by the `SMSG_NEW_WORLD` arm. It separates two cases. A
    /// teleport that leaves a boat makes the recorded transport offset
    /// meaningless. `Transport::TeleportTransport` moves the boat with the
    /// character on it, and afterwards the transport offset is the only part of
    /// the position still valid. See [`movement::Mover::transfer_aboard`].
    pub transfer_on_transport: bool,
}

impl LocalState {
    pub fn new(mover: Mover, map_id: u32) -> LocalState {
        LocalState {
            mover,
            map_id,
            started: Instant::now(),
            latency_ms: 0,
            ping_sent_at: None,
            relocations: 0,
            mover_guid: 0,
            looking_through: false,
            transfer_on_transport: false,
        }
    }

    /// The movement block to send right now, stamped with the session clock.
    ///
    /// `ctime` must be non-zero and must never decrease: zero is
    /// `CHEAT_TYPE_NULL_CLIENT_TIME` and any decrease is `CHEAT_TYPE_TIME_BACK`.
    /// It is therefore milliseconds since the session started, plus one so the
    /// first packet is not zero.
    pub fn movement_info(&mut self) -> movement::MovementInfo {
        self.mover.info.time = self.started.elapsed().as_millis() as u32 + 1;
        self.mover.info
    }
}

/// Everything one packet is allowed to touch.
pub struct Incoming<'a> {
    pub world: &'a mut ObjectManager,
    pub stats: &'a mut PumpStats,
    pub replies: &'a mut Replies,
    /// `None` for a caller with no simulation of its own, which is the CLI's
    /// snapshot pump. The handlers then skip the local part of their work
    /// rather than branching into a second dispatch.
    pub local: Option<&'a mut LocalState>,
}

/// Fold one received packet into the world state, and queue whatever it owes
/// the server.
///
/// This is the only place an opcode is turned into an action. Both the CLI's
/// snapshot pump and the live session loop call it, so a packet cannot be
/// handled by one and not the other. Such a divergence would show as the
/// renderer being stale while the CLI output is correct.
pub fn apply_packet(ctx: &mut Incoming, pkt: &Packet) {
    ctx.stats.packets += 1;
    ctx.stats.saw(pkt);

    let Some(op) = pkt.opcode() else {
        ctx.stats.unhandled(pkt);
        return;
    };

    match op {
        // ---- world state: objects, movement, combat, visuals ---------------
        Opcode::SMSG_UPDATE_OBJECT => world::update(ctx, pkt),
        Opcode::SMSG_COMPRESSED_UPDATE_OBJECT => world::compressed_update(ctx, pkt),
        // A compressed bag of movement packets, which carries most of the
        // movement on a busy realm. See [`world::compressed_moves`] for the
        // server's rate threshold and the effect of dropping it.
        Opcode::SMSG_COMPRESSED_MOVES => world::compressed_moves(ctx, pkt),
        Opcode::SMSG_MONSTER_MOVE => world::monster_move(ctx, pkt),
        // The same move on a transport: one extra packed guid, and a path in
        // the transport's frame rather than the world's. Read as a plain
        // `SMSG_MONSTER_MOVE`, it places every boat passenger within a few
        // yards of the map's origin. See [`world::monster_move_transport`].
        Opcode::SMSG_MONSTER_MOVE_TRANSPORT => world::monster_move_transport(ctx, pkt),
        Opcode::SMSG_ATTACKERSTATEUPDATE => world::attack(ctx, pkt),
        Opcode::SMSG_SPELLNONMELEEDAMAGELOG => world::spell_damage(ctx, pkt),
        Opcode::SMSG_SPELLHEALLOG => world::spell_heal(ctx, pkt),
        Opcode::SMSG_EMOTE => world::emote(ctx, pkt),
        Opcode::SMSG_AI_REACTION => world::ai_reaction(ctx, pkt),
        // ---- sounds and spell visuals the server pushes --------------------
        //
        // Five opcodes, and no other packet implies any of them: a scripted
        // sound, a music track, a sound at an object, and the two that put a
        // `SpellVisualKit` on a unit. They carry every boss line, every gate
        // sound, and the eating and drinking animations. See
        // [`crate::play::sound`].
        Opcode::SMSG_PLAY_SOUND | Opcode::SMSG_PLAY_MUSIC | Opcode::SMSG_PLAY_OBJECT_SOUND => {
            world::play_sound(ctx, pkt, op)
        }
        Opcode::SMSG_PLAY_SPELL_VISUAL => world::play_spell_visual(ctx, pkt, false),
        Opcode::SMSG_PLAY_SPELL_IMPACT => world::play_spell_visual(ctx, pkt, true),
        // A game object's one-shot animations: a fishing bobber's bite, a
        // trap firing, a despawn. See [`crate::play::object`].
        Opcode::SMSG_GAMEOBJECT_CUSTOM_ANIM => player::gameobject_custom_anim(ctx, pkt),
        Opcode::SMSG_GAMEOBJECT_DESPAWN_ANIM => player::gameobject_despawn_anim(ctx, pkt),
        // ---- casts, object removal, time, weather, bind point, duels, -----
        // ---- summons, played time and fishing -----------------------------
        Opcode::SMSG_SPELL_START => world::cast(ctx, pkt, true),
        Opcode::SMSG_SPELL_GO => world::cast(ctx, pkt, false),
        Opcode::SMSG_DESTROY_OBJECT => world::destroy(ctx, pkt),
        Opcode::SMSG_LOGIN_SETTIMESPEED => world::game_time(ctx, pkt),
        Opcode::SMSG_WEATHER => world::weather(ctx, pkt),
        Opcode::SMSG_TRIGGER_CINEMATIC => world::cinematic(ctx, pkt),
        Opcode::SMSG_BINDPOINTUPDATE => player::bind_point(ctx, pkt),
        Opcode::SMSG_PLAYERBOUND => player::player_bound(ctx, pkt),
        Opcode::SMSG_BINDER_CONFIRM => player::binder_confirm(ctx, pkt),
        Opcode::SMSG_DUEL_REQUESTED => player::duel_requested(ctx, pkt),
        Opcode::SMSG_DUEL_COUNTDOWN => player::duel_countdown(ctx, pkt),
        Opcode::SMSG_DUEL_OUTOFBOUNDS => player::duel_bounds(ctx, true),
        Opcode::SMSG_DUEL_INBOUNDS => player::duel_bounds(ctx, false),
        Opcode::SMSG_DUEL_COMPLETE => player::duel_complete(ctx, pkt),
        Opcode::SMSG_DUEL_WINNER => player::duel_winner(ctx, pkt),
        Opcode::SMSG_SUMMON_REQUEST => player::summon_request(ctx, pkt),
        Opcode::SMSG_PLAYED_TIME => player::played_time(ctx, pkt),
        Opcode::SMSG_INSPECT => player::inspect(ctx, pkt),
        Opcode::MSG_INSPECT_HONOR_STATS => player::inspect_honor(ctx, pkt),
        Opcode::SMSG_FISH_NOT_HOOKED => player::fish(ctx, false),
        Opcode::SMSG_FISH_ESCAPED => player::fish(ctx, true),

        // ---- combat log only -----------------------------------------------
        //
        // Thirteen opcodes that change nothing on screen. Each one is only a
        // line in the combat log, so dropping one is silent: the fight looks
        // the same and the combat log omits the event.
        Opcode::SMSG_LOG_XPGAIN => world::xp_gain(ctx, pkt),
        Opcode::SMSG_PARTYKILLLOG => world::party_kill(ctx, pkt),
        Opcode::SMSG_ENVIRONMENTALDAMAGELOG => world::environmental_damage(ctx, pkt),
        Opcode::SMSG_SPELLDAMAGESHIELD => world::damage_shield(ctx, pkt),
        Opcode::SMSG_SPELLLOGMISS => world::spell_miss_log(ctx, pkt),
        Opcode::SMSG_SPELLENERGIZELOG => world::energize_log(ctx, pkt),
        Opcode::SMSG_PERIODICAURALOG => world::periodic_aura_log(ctx, pkt),
        Opcode::SMSG_SPELLLOGEXECUTE => world::spell_execute_log(ctx, pkt),
        Opcode::SMSG_SPELLDISPELLOG => world::spell_dispel_log(ctx, pkt),
        Opcode::SMSG_SPELLORDAMAGE_IMMUNE => world::immune_log(ctx, pkt),
        Opcode::SMSG_PROCRESIST => world::proc_resist_log(ctx, pkt),
        Opcode::SMSG_DISPEL_FAILED => world::dispel_failed_log(ctx, pkt),
        Opcode::SMSG_SPELLINSTAKILLLOG => world::instakill_log(ctx, pkt),

        // ---- pet, then the player's spells, casts, attacks and items -------
        //
        // The pet opcodes come first. Hunters and warlocks cannot be played
        // without them. The pet bar (`SMSG_PET_SPELLS`) is state and the rest
        // are one-off events; see `crate::play::pet`.
        Opcode::SMSG_PET_SPELLS => player::pet_spells(ctx, pkt),
        Opcode::SMSG_PET_MODE => player::pet_mode(ctx, pkt),
        Opcode::SMSG_PET_NAME_QUERY_RESPONSE => player::pet_name(ctx, pkt),
        Opcode::SMSG_PET_ACTION_FEEDBACK => player::pet_feedback(ctx, pkt),
        Opcode::SMSG_PET_CAST_FAILED => player::pet_cast_failed(ctx, pkt),
        Opcode::SMSG_PET_TAME_FAILURE => player::pet_tame_failure(ctx, pkt),
        Opcode::SMSG_PET_BROKEN => player::pet_broken(ctx, pkt),
        Opcode::SMSG_PET_NAME_INVALID => player::pet_name_invalid(ctx, pkt),
        Opcode::SMSG_PET_UNLEARN_CONFIRM => player::pet_unlearn_confirm(ctx, pkt),
        Opcode::SMSG_PET_ACTION_SOUND => player::pet_action_sound(ctx, pkt),
        Opcode::SMSG_PET_DISMISS_SOUND => player::pet_dismiss_sound(ctx, pkt),

        Opcode::SMSG_INITIAL_SPELLS => player::initial_spells(ctx, pkt),
        Opcode::SMSG_ACTION_BUTTONS => player::action_buttons(ctx, pkt),
        Opcode::SMSG_SUPERCEDED_SPELL => player::superceded_spell(ctx, pkt),
        Opcode::SMSG_CAST_RESULT => player::cast_result(ctx, pkt),
        Opcode::SMSG_SPELL_FAILED_OTHER => player::cast_interrupted(ctx, pkt),
        Opcode::SMSG_SPELL_DELAYED => player::cast_delayed(ctx, pkt),
        Opcode::SMSG_SPELL_COOLDOWN => player::spell_cooldown(ctx, pkt),
        Opcode::SMSG_COOLDOWN_EVENT => player::cooldown_event(ctx, pkt),
        Opcode::SMSG_CLEAR_COOLDOWN => player::clear_cooldown(ctx, pkt),
        Opcode::SMSG_ITEM_COOLDOWN => player::item_cooldown(ctx, pkt),
        Opcode::SMSG_ATTACKSTART => player::attack_state(ctx, pkt, true),
        Opcode::SMSG_ATTACKSTOP => player::attack_state(ctx, pkt, false),
        Opcode::SMSG_CANCEL_AUTO_REPEAT => player::auto_repeat_cancelled(ctx),
        Opcode::MSG_CHANNEL_START => player::channel_start(ctx, pkt),
        Opcode::MSG_CHANNEL_UPDATE => player::channel_update(ctx, pkt),
        Opcode::SMSG_UPDATE_AURA_DURATION => player::aura_duration(ctx, pkt),
        Opcode::SMSG_LEVELUP_INFO => player::levelup(ctx, pkt),
        // The refusal shared by every item action. It is the only packet that
        // answers an item right-click the server rejected.
        Opcode::SMSG_INVENTORY_CHANGE_FAILURE => player::inventory_failed(ctx, pkt),
        // An item arrived in a bag. No update field says this: a stack growing
        // by three does not say whether the items were looted, bought,
        // crafted, mailed or traded. See [`player::item_received`].
        Opcode::SMSG_ITEM_PUSH_RESULT => player::item_received(ctx, pkt),
        // The two timers a carried item can have, which no update field counts
        // down. See [`player::enchant_time`].
        Opcode::SMSG_ITEM_ENCHANT_TIME_UPDATE => player::enchant_time(ctx, pkt),
        Opcode::SMSG_ITEM_TIME_UPDATE => player::item_time(ctx, pkt),

        // ---- reputation -----------------------------------------------------
        // Four packets about the 64 reputation-list slots, then
        // `SMSG_SET_FORCED_REACTIONS`. See
        // [`crate::play::reputation`]: the standings on the wire are deltas
        // from a `Faction.dbc` base value, so none of them can be drawn
        // without that table.
        Opcode::SMSG_INITIALIZE_FACTIONS => player::initialize_factions(ctx, pkt),
        Opcode::SMSG_SET_FACTION_STANDING => player::faction_standing(ctx, pkt),
        Opcode::SMSG_SET_FACTION_VISIBLE => player::faction_visible(ctx, pkt),
        Opcode::SMSG_SET_FACTION_ATWAR => player::faction_at_war(ctx, pkt),
        Opcode::SMSG_SET_FORCED_REACTIONS => player::forced_reactions(ctx, pkt),

        // ---- quests ---------------------------------------------------------
        // The quest-giver conversation and the quest log. See
        // [`crate::play::quest`], which handles the log's packed six-bit
        // counters and the top bit that marks an objective's target as a game
        // object. Misreading either gives wrong values rather than a parse
        // failure.
        Opcode::SMSG_QUESTGIVER_STATUS => player::quest_status(ctx, pkt),
        Opcode::SMSG_QUESTGIVER_QUEST_LIST => player::quest_greeting(ctx, pkt),
        Opcode::SMSG_QUESTGIVER_QUEST_DETAILS => player::quest_details(ctx, pkt),
        Opcode::SMSG_QUESTGIVER_OFFER_REWARD => player::quest_reward(ctx, pkt),
        Opcode::SMSG_QUESTGIVER_REQUEST_ITEMS => player::quest_progress(ctx, pkt),
        Opcode::SMSG_QUESTGIVER_QUEST_COMPLETE => player::quest_complete(ctx, pkt),
        Opcode::SMSG_QUEST_QUERY_RESPONSE => player::quest_template(ctx, pkt),
        Opcode::SMSG_QUESTUPDATE_ADD_KILL => player::quest_kill(ctx, pkt),
        Opcode::SMSG_QUESTUPDATE_ADD_ITEM => player::quest_item(ctx, pkt),
        Opcode::SMSG_QUESTUPDATE_COMPLETE => player::quest_objectives_done(ctx, pkt),
        Opcode::SMSG_QUESTUPDATE_FAILED => player::quest_failed(ctx, pkt, false),
        Opcode::SMSG_QUESTUPDATE_FAILEDTIMER => player::quest_failed(ctx, pkt, true),
        // The body is a reason code, not a quest id. It is the only packet in
        // this group whose body is not about a quest.
        Opcode::SMSG_QUESTGIVER_QUEST_INVALID => player::quest_refused(ctx, pkt),

        // ---- talking to an NPC ----------------------------------------------
        // The gossip menu, whose text needs a separate query round trip, and
        // the vendor window opened from it. See [`crate::play::gossip`].
        Opcode::SMSG_GOSSIP_MESSAGE => player::gossip_show(ctx, pkt),
        Opcode::SMSG_GOSSIP_COMPLETE => player::gossip_closed(ctx),
        Opcode::SMSG_GOSSIP_POI => player::gossip_poi(ctx, pkt),
        Opcode::SMSG_GAMEOBJECT_PAGETEXT => player::gameobject_pagetext(ctx, pkt),
        Opcode::SMSG_PAGE_TEXT_QUERY_RESPONSE => player::page_text(ctx, pkt),
        Opcode::SMSG_NPC_TEXT_UPDATE => player::npc_text(ctx, pkt),
        Opcode::SMSG_LIST_INVENTORY => player::vendor_list(ctx, pkt),
        Opcode::SMSG_BUY_ITEM => player::vendor_sold(ctx, pkt),
        Opcode::SMSG_BUY_FAILED => player::buy_failed(ctx, pkt),
        Opcode::SMSG_SELL_ITEM => player::sell_failed(ctx, pkt),
        // The trainer window has the same structure as the vendor: an opening
        // request, a list, and one action with a success and a failure reply.
        // See [`crate::play::trainer`].
        Opcode::SMSG_TRAINER_LIST => player::trainer_list(ctx, pkt),
        Opcode::SMSG_TRAINER_BUY_SUCCEEDED => player::trainer_bought(ctx, pkt),
        Opcode::SMSG_TRAINER_BUY_FAILED => player::trainer_buy_failed(ctx, pkt),
        // The stable master. Its window is one packet, and its four actions
        // share one reply opcode. The list opcode is an `MSG_` and is sent in
        // both directions. See [`crate::play::stable`].
        Opcode::MSG_LIST_STABLED_PETS => player::stable_list(ctx, pkt),
        Opcode::SMSG_STABLE_RESULT => player::stable_result(ctx, pkt),
        // The banker. Its window packet carries only a guid, and its one
        // action (buying a bank slot) is answered only when refused. See
        // [`crate::play::bank`].
        Opcode::SMSG_SHOW_BANK => player::bank_show(ctx, pkt),
        Opcode::SMSG_BUY_BANK_SLOT_RESULT => player::bank_slot_result(ctx, pkt),
        // The flight master has the same structure, with one difference: the
        // flight it sells arrives as an ordinary `SMSG_MONSTER_MOVE`, handled
        // above, and none of these arms is involved in it. See
        // [`crate::play::taxi`].
        Opcode::SMSG_SHOWTAXINODES => player::taxi_show(ctx, pkt),
        Opcode::SMSG_TAXINODE_STATUS => player::taxi_node_status(ctx, pkt),
        Opcode::SMSG_NEW_TAXI_PATH => player::new_taxi_path(ctx),
        Opcode::SMSG_ACTIVATETAXIREPLY => player::taxi_reply(ctx, pkt),

        // ---- the party -------------------------------------------------------
        // Eight packets, and only one of them is state. `SMSG_GROUP_LIST` is the
        // whole roster, re-sent to every member whenever anything about the
        // group changes, so a client that reads only it still has the correct
        // roster. The other six are one-off events. See [`crate::play::group`],
        // which handles the conditional loot tail and explains why a destroyed
        // group has its own packet.
        Opcode::SMSG_GROUP_INVITE => player::group_invite(ctx, pkt),
        Opcode::SMSG_GROUP_DECLINE => player::group_decline(ctx, pkt),
        Opcode::SMSG_GROUP_LIST => player::group_list(ctx, pkt),
        Opcode::SMSG_GROUP_DESTROYED => player::group_destroyed(ctx),
        Opcode::SMSG_GROUP_SET_LEADER => player::group_new_leader(ctx, pkt),
        Opcode::SMSG_PARTY_COMMAND_RESULT => player::party_result(ctx, pkt),
        Opcode::SMSG_PARTY_MEMBER_STATS | Opcode::SMSG_PARTY_MEMBER_STATS_FULL => {
            player::party_member_stats(ctx, pkt)
        }
        Opcode::MSG_RAID_READY_CHECK => player::raid_ready_check(ctx, pkt),
        // A minimap ping, the raid target icons and quest sharing: what the
        // group marks for each other. See [`crate::play::minimap`],
        // [`crate::play::raidtarget`] and [`crate::play::questshare`].
        Opcode::MSG_MINIMAP_PING => player::minimap_ping(ctx, pkt),
        Opcode::MSG_RAID_TARGET_UPDATE => player::raid_targets(ctx, pkt),
        Opcode::MSG_QUEST_PUSH_RESULT => player::quest_push_result(ctx, pkt),
        Opcode::SMSG_QUEST_CONFIRM_ACCEPT => player::quest_confirm_accept(ctx, pkt),

        // ---- friends, ignore and /who ----------------------------------------
        // The friends list, the ignore list and the /who search. Both lists
        // arrive once and are then patched by `SMSG_FRIEND_STATUS`, which also
        // carries every refusal. See [`crate::play::social`], which explains
        // why the two lists contain no names.
        Opcode::SMSG_FRIEND_LIST => player::friend_list(ctx, pkt),
        Opcode::SMSG_IGNORE_LIST => player::ignore_list(ctx, pkt),
        Opcode::SMSG_FRIEND_STATUS => player::friend_status(ctx, pkt),
        Opcode::SMSG_WHO => player::who_results(ctx, pkt),

        // ---- the guild -------------------------------------------------------
        // See [`crate::play::guild`]. Each is forwarded as an event; the
        // character's own guild id and rank are update fields.
        Opcode::SMSG_GUILD_QUERY_RESPONSE => player::guild_query(ctx, pkt),
        Opcode::SMSG_GUILD_ROSTER => player::guild_roster(ctx, pkt),
        Opcode::SMSG_GUILD_EVENT => player::guild_event(ctx, pkt),
        Opcode::SMSG_GUILD_COMMAND_RESULT => player::guild_command_result(ctx, pkt),
        Opcode::SMSG_GUILD_INVITE => player::guild_invite(ctx, pkt),
        Opcode::SMSG_GUILD_DECLINE => player::guild_decline(ctx, pkt),
        Opcode::SMSG_GUILD_INFO => player::guild_info(ctx, pkt),
        Opcode::MSG_TABARDVENDOR_ACTIVATE => player::tabard_vendor(ctx, pkt),
        Opcode::MSG_SAVE_GUILD_EMBLEM => player::guild_emblem_result(ctx, pkt),

        // ---- the guild charter -----------------------------------------------
        // See [`crate::play::petition`]. The signatures name their signers by
        // guid, so that handler also asks for the names.
        Opcode::SMSG_PETITION_SHOWLIST => player::petition_show_list(ctx, pkt),
        Opcode::SMSG_PETITION_SHOW_SIGNATURES => player::petition_signatures(ctx, pkt),
        Opcode::SMSG_PETITION_QUERY_RESPONSE => player::petition_query(ctx, pkt),
        Opcode::SMSG_PETITION_SIGN_RESULTS => player::petition_sign_result(ctx, pkt),
        Opcode::SMSG_TURN_IN_PETITION_RESULTS => player::petition_turn_in(ctx, pkt),
        Opcode::MSG_PETITION_DECLINE => player::petition_declined(ctx, pkt),
        Opcode::MSG_PETITION_RENAME => player::petition_renamed(ctx, pkt),

        // ---- mail ------------------------------------------------------------
        // Five packets; one of them (`SMSG_SEND_MAIL_RESULT`) answers seven
        // different actions. The other four arms follow the trade arms below.
        // See [`crate::play::mail`], which handles the union in the header's
        // sender field and explains why opening a mailbox sends no packet.
        Opcode::SMSG_MAIL_LIST_RESULT => player::mail_list(ctx, pkt),

        // ---- trade (its two arms follow the block below) ----------------------
        // Two packets, and the second states both players' offers. See
        // [`crate::play::trade`].
        // ---- proficiencies, spell modifiers and enchantments ----------------
        // Three types of packet that each state a value rather than report an
        // event: a class's whole proficiency mask, a spell-modifier bit's
        // running total, and an enchantment being applied or expiring.
        Opcode::SMSG_SET_PROFICIENCY => player::proficiency(ctx, pkt),
        Opcode::SMSG_SET_FLAT_SPELL_MODIFIER => player::spell_modifier(ctx, pkt, false),
        Opcode::SMSG_SET_PCT_SPELL_MODIFIER => player::spell_modifier(ctx, pkt, true),
        Opcode::SMSG_ENCHANTMENTLOG => player::enchantment_log(ctx, pkt),

        Opcode::SMSG_TRADE_STATUS => player::trade_status(ctx, pkt),
        Opcode::SMSG_TRADE_STATUS_EXTENDED => player::trade_offer(ctx, pkt),
        Opcode::SMSG_SEND_MAIL_RESULT => player::mail_result(ctx, pkt),
        Opcode::SMSG_RECEIVED_MAIL => player::mail_received(ctx),
        Opcode::MSG_QUERY_NEXT_MAIL_TIME => player::mail_next_time(ctx, pkt),
        Opcode::SMSG_ITEM_TEXT_QUERY_RESPONSE => player::item_text(ctx, pkt),

        // ---- exploration -----------------------------------------------------
        // The only packet that says an area was discovered. See
        // [`crate::play::explored`] for the update field that goes with it.
        Opcode::SMSG_EXPLORATION_EXPERIENCE => player::discovered(ctx, pkt),

        // ---- loot -----------------------------------------------------------
        // Five packets. `SMSG_LOOT_RESPONSE` is both the loot window and the
        // refusal to open one; a loot type of zero marks the refusal. See
        // [`crate::play::loot`], which also explains why money takes two
        // packets and why the server confirms the release.
        Opcode::SMSG_LOOT_RESPONSE => player::loot_response(ctx, pkt),
        Opcode::SMSG_LOOT_RELEASE_RESPONSE => player::loot_released(ctx, pkt),
        Opcode::SMSG_LOOT_REMOVED => player::loot_removed(ctx, pkt),
        Opcode::SMSG_LOOT_CLEAR_MONEY => player::loot_money_cleared(ctx),
        Opcode::SMSG_LOOT_MONEY_NOTIFY => player::loot_money_gained(ctx, pkt),

        // ---- group loot rolls -----------------------------------------------
        // Four packets. None of them identifies the roll the way the interface
        // does: the packets use `(guid, item slot)` and the interface uses a
        // `rollID` the client assigns. See [`crate::play::lootroll`], which
        // also implements a rule no packet states: the end of a roll is what
        // makes a blocked loot row clickable.
        Opcode::SMSG_LOOT_START_ROLL => player::loot_roll_started(ctx, pkt),
        Opcode::SMSG_LOOT_ROLL => player::loot_roll_cast(ctx, pkt),
        Opcode::SMSG_LOOT_ROLL_WON => player::loot_roll_won(ctx, pkt),
        Opcode::SMSG_LOOT_ALL_PASSED => player::loot_roll_all_passed(ctx, pkt),

        // ---- mirror timers (breath, fatigue, feign death) -------------------
        // The breath meter, the fatigue meter and the feign-death bar. Three
        // packets; this client keeps no state or clock of its own for them.
        // See [`crate::play::timers`]. The shipped interface cannot act on the
        // pause packet, which is why vmangos resends a start instead.
        Opcode::SMSG_START_MIRROR_TIMER => player::mirror_timer_start(ctx, pkt),
        Opcode::SMSG_STOP_MIRROR_TIMER => player::mirror_timer_stop(ctx, pkt),
        Opcode::SMSG_PAUSE_MIRROR_TIMER => player::mirror_timer_pause(ctx, pkt),

        // ---- death and resurrection -----------------------------------------
        // None of these announces the death itself. Death is health reaching
        // zero, which arrives in an ordinary values block. These four carry
        // what the client learns only by querying or by being offered it. See
        // [`crate::play::death`].
        Opcode::SMSG_CORPSE_RECLAIM_DELAY => player::corpse_reclaim_delay(ctx, pkt),
        Opcode::MSG_CORPSE_QUERY => player::corpse_located(ctx, pkt),
        Opcode::SMSG_RESURRECT_REQUEST => player::resurrect_offered(ctx, pkt),
        Opcode::SMSG_SPIRIT_HEALER_CONFIRM => player::spirit_healer_offered(ctx, pkt),

        // ---- logout ---------------------------------------------------------
        // Three of the four logout packets have no body. The server runs the
        // twenty-second logout timer.
        Opcode::SMSG_LOGOUT_RESPONSE => player::logout_response(ctx, pkt),
        Opcode::SMSG_LOGOUT_CANCEL_ACK => {
            player::logout_state(ctx, crate::play::logout::Logout::Cancelled)
        }
        Opcode::SMSG_LOGOUT_COMPLETE => player::logout_state(ctx, crate::play::logout::Logout::Complete),

        // ---- name and template query responses -----------------------------
        Opcode::SMSG_CREATURE_QUERY_RESPONSE => query::creature(ctx, pkt),
        Opcode::SMSG_NAME_QUERY_RESPONSE => query::name(ctx, pkt),
        Opcode::SMSG_GAMEOBJECT_QUERY_RESPONSE => query::gameobject(ctx, pkt),
        Opcode::SMSG_ITEM_QUERY_SINGLE_RESPONSE => query::item(ctx, pkt),

        // ---- chat, channels and text emotes --------------------------------
        Opcode::SMSG_MESSAGECHAT => chat::message(ctx, pkt),
        Opcode::SMSG_NOTIFICATION => chat::notification(ctx, pkt),
        Opcode::SMSG_CHANNEL_NOTIFY => chat::channel_notify(ctx, pkt),
        Opcode::SMSG_CHANNEL_LIST => chat::channel_list(ctx, pkt),
        Opcode::SMSG_TEXT_EMOTE => chat::text_emote(ctx, pkt),
        // Four server statements shown as a line and kept nowhere. See
        // [`crate::play::notices`].
        Opcode::SMSG_CHAT_PLAYER_NOT_FOUND => chat::player_not_found(ctx, pkt),
        Opcode::SMSG_SERVER_MESSAGE => chat::server_message(ctx, pkt),
        Opcode::SMSG_ZONE_UNDER_ATTACK => chat::zone_under_attack(ctx, pkt),
        Opcode::SMSG_DEFENSE_MESSAGE => chat::defense_message(ctx, pkt),
        Opcode::MSG_RANDOM_ROLL => chat::random_roll(ctx, pkt),

        // ---- packets that must be answered --------------------------------
        Opcode::SMSG_PONG => acks::pong(ctx),
        Opcode::MSG_MOVE_TELEPORT_ACK => acks::teleport(ctx, pkt),
        Opcode::SMSG_NEW_WORLD => acks::new_world(ctx, pkt),
        // The map statement sent at login, with no teleport before it. It is
        // not answered. It is handled because it is the only packet in which
        // the server states which map the character is on. See
        // [`player::login_verify_world`].
        Opcode::SMSG_LOGIN_VERIFY_WORLD => player::login_verify_world(ctx, pkt),
        Opcode::SMSG_MOVE_KNOCK_BACK => acks::knock_back(ctx, pkt),
        // Says which unit the player's input controls. A possess begins and
        // ends with this packet. See [`acks::client_control`].
        Opcode::SMSG_CLIENT_CONTROL_UPDATE => acks::client_control(ctx, pkt),
        // A refused transfer. No other packet says an instance portal was
        // refused, so without this arm a full dungeon looks the same as a
        // client that never sent `CMSG_AREATRIGGER`.
        Opcode::SMSG_TRANSFER_ABORTED => player::transfer_aborted(ctx, pkt),
        Opcode::SMSG_AREA_TRIGGER_MESSAGE => player::area_trigger_message(ctx, pkt),
        // A transfer about to happen, sent one packet before `SMSG_NEW_WORLD`
        // so the loading screen is shown before the old world is torn down.
        // It is not answered; it is handled only to show the loading screen.
        Opcode::SMSG_TRANSFER_PENDING => player::transfer_pending(ctx, pkt),

        // ---- world states, tutorials and the rest of the login burst -------
        // The zone's world state table and its changes, which the frame above
        // the minimap shows; the account's tutorial mask; and two packets read
        // only so that a malformed body is reported. See
        // [`crate::play::worldstate`], [`crate::play::tutorial`],
        // [`crate::play::accountdata`] and [`crate::play::rest`].
        Opcode::SMSG_INIT_WORLD_STATES => player::init_world_states(ctx, pkt),
        Opcode::SMSG_UPDATE_WORLD_STATE => player::update_world_state(ctx, pkt),
        Opcode::SMSG_TUTORIAL_FLAGS => player::tutorial_flags(ctx, pkt),
        Opcode::SMSG_ACCOUNT_DATA_MD5 => player::account_data_md5(ctx, pkt),
        Opcode::SMSG_SET_REST_START => player::set_rest_start(ctx, pkt),

        // ---- opcode families matched by a guard rather than a constant -----
        // Among them are the forced speed change and the forced movement flag
        // change (root, water walking, hover, feather fall). Each is a family
        // of opcodes rather than one, which is why they are guards. Both must
        // be acknowledged with the counter they arrived with, or the server
        // keeps the change pending and then fires `OnFailedToAckChange`.
        //
        // A swing the server refused: five opcodes with no body, so the opcode
        // itself is the message.
        op if crate::play::spells::AttackRefusal::of(op).is_some() => {
            player::attack_refused(ctx, crate::play::spells::AttackRefusal::of(op).expect("just matched"))
        }
        // Spell learned and spell unlearned, whose single field differs in
        // width.
        op if player::is_spell_change(op).is_some() => {
            player::spell_change(ctx, pkt, player::is_spell_change(op).expect("just matched"))
        }
        op if movement::speed_change_slot(op).is_some() => acks::speed_change(ctx, pkt, op),
        // The other two thirds of vmangos' `moveTypeToOpcode`: the same six
        // speeds about a unit this client does not control, which is every
        // other unit. Twelve opcodes, not acknowledged. They are the only
        // restatement of a unit's speed after the movement block it was
        // created with.
        op if movement::broadcast_speed_slot(op).is_some() => {
            world::speed_broadcast(ctx, pkt, op)
        }
        op if movement::FlagChange::of(op).is_some() => acks::flag_change(ctx, pkt, op),
        // Movement flag changes about a unit no player is moving, which is
        // every creature. Twelve opcodes, each a packed guid with no further
        // body, not acknowledged. They are the only restatement of a
        // server-controlled unit's flags after its create block. See
        // [`movement::SplineFlagChange`].
        op if movement::SplineFlagChange::of(op).is_some() => {
            world::spline_flag(ctx, pkt, op)
        }
        // Another player moved, on one of the `MSG_MOVE_*` opcodes.
        op if movement::is_broadcast_movement(op) => world::moved(ctx, pkt),

        _ => ctx.stats.unhandled(pkt),
    }
}

/// A parsed body, or a warning that it could not be read.
///
/// A packet whose body will not parse usually indicates a length bug, and
/// length bugs show up late. Call sites used to decide individually whether to
/// report a parse failure: a mis-read `SMSG_MONSTER_MOVE` produced a frozen
/// creature and no diagnostic, while a mis-read movement broadcast produced a
/// warning. Every handler now reports through this function, and the warnings
/// reach the HUD through `SessionStatus::warnings`.
fn read<T>(stats: &mut PumpStats, pkt: &Packet, parsed: Option<T>) -> Option<T> {
    if parsed.is_none() {
        stats.unreadable(pkt);
    }
    parsed
}

/// What a run of [`apply_packet`] saw. Useful for confirming that parsing
/// actually understood the stream rather than silently skipping it.
#[derive(Debug, Default)]
pub struct PumpStats {
    pub packets: u32,
    pub updates: u32,
    pub compressed: u32,
    /// `SMSG_COMPRESSED_MOVES` bags seen, and the packets taken out of them.
    ///
    /// Counted apart from [`Self::compressed`], which counts the update-object
    /// stream, because the two answer different questions. The server sends no
    /// bag until its movement rate passes
    /// `CONFIG_UINT32_COMPRESSION_MOVEMENT_COUNT`. A session with zero here
    /// therefore says nothing; a session with hundreds is one where most of
    /// the world's movement arrived inside this opcode. Before the opcode was
    /// handled, one session showed 428 unhandled packets and 35.7 KiB on the
    /// `F4` net tab, and creatures stood still.
    ///
    /// Compare the inner count against `traffic[SMSG_MONSTER_MOVE]`: they are
    /// the same packets, and the ratio is the share of the world's movement
    /// that is being bagged.
    pub bagged_moves: u32,
    pub bagged_packets: u32,
    pub creatures_resolved: u32,
    pub gameobjects_resolved: u32,
    pub players_resolved: u32,
    /// Item templates resolved. These cover a player's equipment, which the
    /// game archives do not describe.
    pub items_resolved: u32,
    /// Queries answered from the on-disk cache instead of being sent; see
    /// [`crate::play::wdb`]. Each one saves a round trip. The answer still goes
    /// through this dispatch, so the `*_resolved` counters include them.
    pub cache_answered: u32,
    pub monster_moves: u32,
    /// `MSG_MOVE_*` broadcasts applied: other players moving.
    pub player_moves: u32,
    /// `SMSG_ATTACKERSTATEUPDATE`: melee swings, by or against anyone.
    pub attacks: u32,
    /// Spell damage and heal logs. Counted apart from [`Self::attacks`]
    /// because a single total would hide the case where weapon swings arrive
    /// and spell logs do not, which this client once had for a long time.
    pub spell_logs: u32,
    /// `SMSG_EMOTE`: one-shot emotes played by any visible unit.
    pub emotes: u32,
    /// `SMSG_AI_REACTION`: creatures noticing and engaging.
    ///
    /// Counted separately for the same reason as `speed_broadcasts`. The
    /// packet only drives a sound, so this counter is its only trace. Zero in
    /// a session where something attacked the player means the aggro sounds
    /// are being dropped, which nothing on screen would show.
    pub ai_reactions: u32,
    /// `SMSG_SPELL_START` and `SMSG_SPELL_GO` together.
    pub casts: u32,
    /// `SMSG_FORCE_*_SPEED_CHANGE` recorded and acknowledged: the six opcodes
    /// about a unit this client controls.
    pub speed_changes: u32,
    /// The twelve speed opcodes about a unit this client does not control:
    /// `MSG_MOVE_SET_*_SPEED` and `SMSG_SPLINE_SET_*`, recorded and
    /// deliberately not acknowledged.
    ///
    /// Counted apart from [`Self::speed_changes`] because they answer a
    /// different question. The forced changes are about one character and are
    /// rare. These cover every mount, sprint, daze, enrage and flee within
    /// sight, so a zero here in a session where anyone mounted means the
    /// family is being dropped.
    pub speed_broadcasts: u32,
    /// The twelve `SMSG_SPLINE_MOVE_*` opcodes about a unit no player is
    /// moving: root, water walking, feather fall, hover and walk/run mode.
    ///
    /// Counted separately for the same reason as [`Self::speed_broadcasts`].
    /// This family is the only restatement of a server-controlled unit's
    /// movement flags after its create block, so a zero in a session where
    /// anything was rooted or set to walk means it is being dropped. See
    /// [`crate::state::movement::SplineFlagChange`].
    pub spline_flag_changes: u32,
    /// Sounds and visuals the server requested directly: the three
    /// `SMSG_PLAY_*` sound opcodes and the two `SMSG_PLAY_SPELL_*` ones.
    ///
    /// Two counters because the two fail differently. A dropped sound is
    /// silence and leaves no other trace. A dropped visual is a character who
    /// sits down to eat and plays no animation. For both families a zero
    /// counter is the only evidence of the fault.
    pub pushed_sounds: u32,
    pub pushed_visuals: u32,
    /// Intro cinematics offered and ended: `SMSG_TRIGGER_CINEMATIC` received,
    /// `CMSG_COMPLETE_CINEMATIC` sent back immediately. See
    /// [`self::world::cinematic`].
    ///
    /// This happens once per character, on its first login, and the server
    /// holds the world until the reply arrives. A 1 here in a session whose
    /// world is populated means the reply worked. A 1 in the unhandled list
    /// instead means the packet was dropped.
    pub cinematics: u32,
    /// `SMSG_WEATHER` packets received: one per zone change and one per
    /// weather grade the server rolls. A zero after a zone change means the
    /// packet is not arriving, which is a fault for a zone with a
    /// `game_weather` row.
    pub weather: u32,
    /// `SMSG_ITEM_PUSH_RESULT` packets: items that arrived in a bag. The
    /// inventory's update fields change whether or not this packet is read, so
    /// a zero over a session in which anything was looted means the event is
    /// being dropped while the bag still fills. That failure has no other
    /// symptom.
    pub items_received: u32,
    /// `CMSG_AREATRIGGER` packets sent. This is the only packet this client
    /// sends unprompted.
    ///
    /// A portal that does nothing looks the same whether the client never
    /// detected the trigger, detected it and latched it, or sent it and the
    /// server refused. A zero here after walking through a dungeon entrance
    /// means the fault is in this client. A one means the next thing to check
    /// is `SMSG_TRANSFER_ABORTED`.
    pub area_triggers: u32,
    /// `CMSG_MOVE_SPLINE_DONE` packets sent: a server-driven movement of this
    /// character (a Charge) started, completed and acknowledged.
    ///
    /// No packet reports a server-driven movement that the client ignored. The
    /// symptom of ignoring one is not a wrong position; it is that every later
    /// movement packet from this client is discarded. See
    /// [`movement::Mover::ride`]. A zero here after a charge means the spline
    /// was never followed. A one means the next thing to check is the server's
    /// response.
    pub rides: u32,
    /// Forced movement flag changes and knockbacks, recorded and acknowledged.
    ///
    /// Before these were handled, every one went into the unhandled count, and
    /// the only visible effect was a kick several seconds later for a cheat the
    /// client never attempted. The count shows whether this code path ran at
    /// all in a session.
    pub flag_changes: u32,
    /// Chat lines and notifications applied.
    pub chat: u32,
    /// How many spells `SMSG_INITIAL_SPELLS` named. The packet is sent once in
    /// the login burst and never restated, so a client that misreads it has an
    /// empty action bar and no other symptom. A zero here shows that failure.
    pub spells_known: u32,
    /// How many action-bar slots `SMSG_ACTION_BUTTONS` filled.
    pub action_buttons: u32,
    /// `SMSG_CAST_RESULT` packets, accepted and refused together. This shows
    /// whether the server received a cast, which the world state does not.
    pub cast_results: u32,
    /// The five `SMSG_ATTACKSWING_*` refusals. A nonzero count is normal,
    /// because most fights start with a swing out of range. The count is the
    /// only trace a refused swing leaves.
    pub attack_refusals: u32,
    /// Opcodes seen but not handled, with counts.
    pub other: BTreeMap<String, u32>,
    /// Every opcode the dispatch saw, handled or not, with its packet count and
    /// byte total.
    ///
    /// [`Self::other`] is the unhandled subset of this and is kept separately:
    /// the CLI's login summary prints it, and the count of opcodes this client
    /// does not handle is taken from it. This map records the traffic itself:
    /// which opcodes a session contains, in what proportion, and each one's
    /// share of the bytes. A stream that is 90% `SMSG_MONSTER_MOVE` and one
    /// that is 90% `SMSG_UPDATE_OBJECT` point to different bugs, and no other
    /// counter here separates them.
    ///
    /// Keyed by raw opcode, not by name. `Packet::name` calls `format!` and
    /// allocates, and this map is updated for every packet in the session.
    /// `other` is only updated for the few packets with no handler, so it can
    /// use a `String` key. See [`crate::socket::world::named`], which resolves
    /// the names when the table is read, five times a second.
    pub traffic: BTreeMap<u32, OpcodeFlow>,
    pub warnings: Vec<String>,
    /// The map `SMSG_LOGIN_VERIFY_WORLD` named, for a caller with no
    /// [`LocalState`] to write it into.
    ///
    /// `None` in every run that did not contain a login, which is every run
    /// except the first. The session loop reads the map from `LocalState` and
    /// does not use this field. It exists for `vale login`, which pumps an
    /// [`crate::state::objects::ObjectManager`] with no local state and had
    /// been printing the character list's map, which can be wrong. See
    /// [`crate::state::movement::LoginVerifyWorld`].
    pub landed_on_map: Option<u32>,
}

impl PumpStats {
    fn warn(&mut self, message: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(message);
        }
    }

    /// A packet this client knows about but could not read.
    ///
    /// The warning names the likely cause rather than saying "parse failed":
    /// the block layouts here are flag-gated with no length prefix, so the
    /// cause is almost always a field read at the wrong offset or width.
    fn unreadable(&mut self, pkt: &Packet) {
        self.warn(format!(
            "{} could not be read — a field is being taken at the wrong offset or width",
            pkt.name()
        ));
    }

    /// An opcode with no handler. This is recorded as data, not as an error:
    /// the server legitimately sends opcodes this client does not implement,
    /// and failing on one would end the session for no reason.
    fn unhandled(&mut self, pkt: &Packet) {
        *self.other.entry(pkt.name()).or_insert(0) += 1;
        // Also mark the traffic row, which `apply_packet` created as handled
        // just before dispatch. The fact is deliberately recorded in both
        // maps; see the note on `traffic`.
        self.traffic.entry(pkt.code).or_default().handled = false;
    }

    /// Record one packet against its opcode's row, marked as handled until
    /// [`Self::unhandled`] says otherwise.
    ///
    /// The four bytes are the framed header, which no body length includes, so
    /// this and [`crate::socket::world::WireCounts::bytes_in`] agree to the byte
    /// for every packet that reaches the dispatch.
    fn saw(&mut self, pkt: &Packet) {
        let flow = self.traffic.entry(pkt.code).or_insert(OpcodeFlow {
            count: 0,
            bytes: 0,
            handled: true,
        });
        flow.count += 1;
        flow.bytes += (pkt.body.len() + 4) as u64;
    }

    fn note_warning(&mut self, parsed: &update::ObjectUpdate) {
        if let Some(w) = &parsed.warning {
            self.warn(w.clone());
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::update::Position;

    fn packet(op: Opcode, body: Vec<u8>) -> Packet {
        Packet {
            code: op.code(),
            body,
        }
    }

    fn local() -> LocalState {
        LocalState::new(
            Mover::new(Position::default(), movement::Speeds::default()),
            0,
        )
    }

    /// Run one packet through the dispatch and hand back everything it touched.
    fn apply(
        world: &mut ObjectManager,
        local: Option<&mut LocalState>,
        pkt: &Packet,
    ) -> (PumpStats, Vec<(Opcode, Vec<u8>)>) {
        let mut stats = PumpStats::default();
        let mut replies = Replies::default();
        apply_packet(
            &mut Incoming {
                world,
                stats: &mut stats,
                replies: &mut replies,
                local,
            },
            pkt,
        );
        (stats, replies.take())
    }

    /// A bag of movement packets, built the same way vmangos builds one.
    ///
    /// `MovementData::AddPacket` writes `u8(body + 2); u16(opcode); body` into a
    /// buffer, and `BuildPacket` prefixes the buffer's length and deflates it.
    /// See `world::compressed_moves` for why this opcode matters.
    fn bag(inner: &[(Opcode, Vec<u8>)]) -> Packet {
        use flate2::write::ZlibEncoder;
        use std::io::Write;

        let mut buffer: Vec<u8> = Vec::new();
        for (op, body) in inner {
            assert!(body.len() + 2 <= 0xFF, "vmangos refuses to bag one this big");
            buffer.push((body.len() + 2) as u8);
            buffer.extend_from_slice(&(op.code() as u16).to_le_bytes());
            buffer.extend_from_slice(body);
        }
        let mut out = (buffer.len() as u32).to_le_bytes().to_vec();
        let mut z = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&buffer).unwrap();
        out.extend_from_slice(&z.finish().unwrap());
        Packet { code: Opcode::SMSG_COMPRESSED_MOVES.code(), body: out }
    }

    /// One `SMSG_MONSTER_MOVE` body: a two-node walk from the origin to `to`.
    fn walk(guid: u8, to: f32) -> Vec<u8> {
        let mut body = crate::bytes::Writer::new();
        body.u8(0x01).u8(guid); // packed guid
        body.f32(0.0).f32(0.0).f32(0.0); // spline start
        body.u32(7); // spline id
        body.u8(0); // MonsterMoveNormal
        body.u32(0).u32(1_000).u32(2); // flags, duration, node count
        body.f32(to).f32(0.0).f32(0.0); // the destination
        body.buf
    }

    /// The intro cinematic is answered, and the answer needs no local player.
    ///
    /// `SMSG_TRIGGER_CINEMATIC` arrives during the login burst, before
    /// `CMSG_SET_ACTIVE_MOVER` and before anything has a mover. A handler that
    /// required `ctx.local` would drop it, and the server holds the world until
    /// the reply arrives. See [`self::world::cinematic`] for the effect of dropping it.
    #[test]
    fn the_intro_cinematic_is_ended_at_once() {
        let mut world = ObjectManager::new();
        let packet = Packet {
            code: Opcode::SMSG_TRIGGER_CINEMATIC.code(),
            // `ChrRaces.dbc`'s `CinematicSequence`; 81 is the human one.
            body: 81u32.to_le_bytes().to_vec(),
        };
        let (stats, replies) = apply(&mut world, None, &packet);
        assert_eq!(stats.cinematics, 1);
        assert!(stats.other.is_empty(), "still unhandled: {:?}", stats.other);
        assert_eq!(replies, vec![(Opcode::CMSG_COMPLETE_CINEMATIC, Vec::new())]);
    }

    /// A bag of movement packets is unpacked and every packet in it is applied.
    ///
    /// `WorldSession::SendMovementPacket` batches every movement packet into
    /// `SMSG_COMPRESSED_MOVES` once the rate passes
    /// `CONFIG_UINT32_COMPRESSION_MOVEMENT_COUNT`. A client that drops the
    /// opcode works against a quiet localhost server and, on a busy realm,
    /// loses every creature movement, every other player's movement and every
    /// speed change. One session measured 428 packets and 35.7 KiB of it
    /// before it was handled.
    ///
    /// The test asserts that the packets inside the bag reached the world,
    /// because nothing about the bag itself is visible.
    #[test]
    fn a_bag_of_movement_packets_is_unpacked_and_every_one_of_them_lands() {
        let mut world = ObjectManager::new();
        let packet = bag(&[
            (Opcode::SMSG_MONSTER_MOVE, walk(9, 24.0)),
            (Opcode::SMSG_MONSTER_MOVE, walk(11, -18.0)),
        ]);
        let (stats, _) = apply(&mut world, None, &packet);

        assert_eq!(stats.monster_moves, 2, "the bag was dropped: {:?}", stats.other);
        assert!(world.get(9).is_some_and(|e| e.spline.is_some()));
        assert!(world.get(11).is_some_and(|e| e.spline.is_some()));
        // The destinations are the two the bag carried, so the bodies were
        // split at the right boundaries and not only counted.
        assert_eq!(world.get(9).unwrap().spline.as_ref().unwrap().to()[0], 24.0);
        assert_eq!(world.get(11).unwrap().spline.as_ref().unwrap().to()[0], -18.0);

        // Each inner packet is counted in the traffic table as an ordinary
        // packet. That is why the bag's contents go back through the one
        // dispatch: the traffic table must show what the stream contains.
        let moves = stats.traffic.get(&Opcode::SMSG_MONSTER_MOVE.code());
        assert_eq!(moves.map(|f| f.count), Some(2));
        assert!(stats.traffic.contains_key(&Opcode::SMSG_COMPRESSED_MOVES.code()));
        assert!(stats.other.is_empty(), "something went unhandled: {:?}", stats.other);
        assert_eq!(stats.bagged_moves, 1);
        assert_eq!(stats.bagged_packets, 2);
        assert_eq!(stats.compressed, 0, "the update stream's counter is a different question");
    }

    /// A damaged tail loses the rest of the bag and produces a warning, and the
    /// packets before it are still applied. Every reader in this crate follows
    /// this rule. It matters most here because there is no way to
    /// resynchronise past a bad length byte.
    #[test]
    fn a_truncated_bag_keeps_what_it_read_and_warns() {
        let mut world = ObjectManager::new();
        let good = bag(&[(Opcode::SMSG_MONSTER_MOVE, walk(9, 24.0))]);
        // Rebuild the buffer by hand with a length that runs off the end.
        use flate2::write::ZlibEncoder;
        use std::io::Write;
        let mut buffer: Vec<u8> = Vec::new();
        let body = walk(9, 24.0);
        buffer.push((body.len() + 2) as u8);
        buffer.extend_from_slice(&(Opcode::SMSG_MONSTER_MOVE.code() as u16).to_le_bytes());
        buffer.extend_from_slice(&body);
        buffer.push(0xFF); // a 255-byte block with nothing behind it
        buffer.extend_from_slice(&[0u8; 4]);
        let mut out = (buffer.len() as u32).to_le_bytes().to_vec();
        let mut z = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&buffer).unwrap();
        out.extend_from_slice(&z.finish().unwrap());
        let torn = Packet { code: Opcode::SMSG_COMPRESSED_MOVES.code(), body: out };

        let (stats, _) = apply(&mut world, None, &torn);
        assert_eq!(stats.monster_moves, 1, "the good packet in front was lost");
        assert!(world.get(9).is_some_and(|e| e.spline.is_some()));
        assert!(
            stats.warnings.iter().any(|w| w.contains("runs past")),
            "a torn bag went unreported: {:?}",
            stats.warnings
        );
        // The undamaged bag reads without a warning, so the warning above is
        // caused by the truncation and not by the bag's layout.
        let mut world = ObjectManager::new();
        assert!(apply(&mut world, None, &good).0.warnings.is_empty());
    }

    /// A bag inside a bag is refused rather than recursed into. The server
    /// never produces one, because only movement packets reach the compressor,
    /// and recursion depth must not be controlled by received data.
    #[test]
    fn a_nested_bag_is_refused() {
        let mut world = ObjectManager::new();
        let inner = bag(&[(Opcode::SMSG_MONSTER_MOVE, walk(9, 24.0))]);
        // Truncated only so it fits the u8 length. The guard checks the opcode
        // and never reads the body, so the truncated body does not matter.
        let short = inner.body[..inner.body.len().min(64)].to_vec();
        let nested = bag(&[(Opcode::SMSG_COMPRESSED_MOVES, short)]);
        let (stats, _) = apply(&mut world, None, &nested);
        assert_eq!(stats.monster_moves, 0);
        assert!(
            stats.warnings.iter().any(|w| w.contains("nested bag")),
            "{:?}",
            stats.warnings
        );
    }

    /// Every packet is recorded in the traffic table, and opcodes with no
    /// handler are marked.
    ///
    /// Without the mark, an opcode that arrives a thousand times with no
    /// handler cannot be told apart by packet count from one that is read
    /// correctly. The bytes include the four-byte framed header, which no body
    /// length includes, so this table and `WireCounts::bytes_in` agree per
    /// packet.
    #[test]
    fn the_traffic_table_counts_every_opcode_and_marks_the_unread() {
        let mut world = ObjectManager::new();
        // An opcode with a handler, sent twice, whose bodies differ in length.
        let (stats, _) = {
            let mut stats = PumpStats::default();
            let mut replies = Replies::default();
            for body in [vec![0u8; 6], vec![0u8; 10]] {
                apply_packet(
                    &mut Incoming {
                        world: &mut world,
                        stats: &mut stats,
                        replies: &mut replies,
                        local: None,
                    },
                    &packet(Opcode::SMSG_EMOTE, body),
                );
            }
            // An opcode with no handler. `SMSG_AUTH_CHALLENGE` belongs to the
            // handshake and never reaches this dispatch in a real session, so
            // it will stay unhandled here and is a stable example.
            apply_packet(
                &mut Incoming {
                    world: &mut world,
                    stats: &mut stats,
                    replies: &mut replies,
                    local: None,
                },
                &packet(Opcode::SMSG_AUTH_CHALLENGE, vec![0u8; 4]),
            );
            (stats, replies.take())
        };

        let emote = stats.traffic[&Opcode::SMSG_EMOTE.code()];
        assert_eq!(emote.count, 2);
        assert_eq!(emote.bytes, (6 + 4) + (10 + 4), "the header is in the bytes");
        assert!(emote.handled, "SMSG_EMOTE has an arm");

        let deaf = stats.traffic[&Opcode::SMSG_AUTH_CHALLENGE.code()];
        assert_eq!(deaf.count, 1);
        assert!(!deaf.handled, "nothing in the dispatch reads it");

        // `other` holds only the unhandled subset. The CLI's login summary
        // depends on that.
        assert_eq!(stats.other.len(), 1);
        assert_eq!(stats.other["SMSG_AUTH_CHALLENGE"], 1);
        assert_eq!(stats.packets, 3);
    }

    /// A forced speed change is recorded on the unit it names, including a unit
    /// other than the local player.
    ///
    /// When the dispatch was split across two places, the live loop
    /// intercepted these packets, acknowledged them and returned, so one about
    /// another player never reached the world. `Entity::speeds` is otherwise
    /// written only by an update block's movement section, so the client kept
    /// dead-reckoning a hasted player at their old run speed until the server
    /// described them again. The correction then showed as the unit jumping to
    /// a new position.
    #[test]
    fn a_speed_change_about_someone_else_still_reaches_the_world() {
        // Another player, known because they walked into view, which is the
        // only way this client learns about one.
        let mut world = ObjectManager::new();
        world.apply_movement(7, &movement::MovementInfo::default());

        let mut body = Vec::new();
        // Packed guid: one mask byte with the low byte present.
        body.push(0x01);
        body.push(7);
        body.extend_from_slice(&1u32.to_le_bytes()); // movement counter
        body.extend_from_slice(&11.0f32.to_le_bytes()); // the new run speed

        let mut state = local();
        let (stats, replies) = apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::SMSG_FORCE_RUN_SPEED_CHANGE, body),
        );

        assert_eq!(stats.speed_changes, 1);
        assert_eq!(
            world.get(7).and_then(|e| e.speeds).map(|s| s.run()),
            Some(11.0),
            "the unit the packet named kept its old speed"
        );
        // It is still acknowledged; otherwise the server leaves the change
        // pending.
        assert_eq!(
            replies.first().map(|(op, _)| *op),
            Some(Opcode::CMSG_FORCE_RUN_SPEED_CHANGE_ACK)
        );
        // Ours is untouched: the packet was about somebody else.
        assert_eq!(state.mover.speeds.run(), movement::Speeds::default().run());
    }

    /// An `SMSG_MONSTER_MOVE` naming the local player is a server-driven
    /// movement (a ride) that the local simulation must follow.
    ///
    /// This is the one packet in the movement family that says nothing about
    /// its target beyond the guid. A Charge moves the caster through the same
    /// `MoveSpline` code that walks a patrolling creature
    /// (`Spell::OnSpellLaunch` → `MoveCharge` in vmangos), so the packet is
    /// identical and only the guid identifies the local player. If it is
    /// applied to the world only, the local simulation continues from its old
    /// position while the server has the player at the target, and the server
    /// also keeps the player spline-pending for the rest of the session. See
    /// [`movement::Mover::ride`].
    #[test]
    fn a_monster_move_naming_the_player_is_ridden_by_the_local_simulation() {
        let mut world = ObjectManager::new();
        world.player_guid = Some(42);
        // The unit being charged, so the packet's `Final_Target` resolves.
        world.apply_movement(
            7,
            &movement::MovementInfo {
                position: Position { x: 30.0, y: 0.0, z: 0.0, orientation: 0.0 },
                ..Default::default()
            },
        );

        let charge = |guid: u8| {
            let mut body = crate::bytes::Writer::new();
            body.u8(0x01).u8(guid); // packed guid
            body.f32(0.0).f32(0.0).f32(0.0); // spline start
            body.u32(41); // spline id
            body.u8(3).u64(7); // MonsterMoveFacingTarget, the victim
            body.u32(0).u32(1_000).u32(2); // flags, duration, node count
            body.f32(24.0).f32(0.0).f32(0.0); // the destination
            packet(Opcode::SMSG_MONSTER_MOVE, body.buf)
        };

        // Another unit's move: the world applies it, the mover does not.
        let mut state = local();
        apply(&mut world, Some(&mut state), &charge(9));
        assert!(!state.mover.is_riding());

        let mut state = local();
        let (stats, replies) = apply(&mut world, Some(&mut state), &charge(42));
        assert_eq!(stats.monster_moves, 1);
        assert!(state.mover.is_riding(), "the player's own spline was ignored");
        // Nothing is sent yet. The acknowledgement is due when the ride ends,
        // with the end position; see `Mover::take_finished_ride`.
        assert!(replies.is_empty(), "{replies:?}");
        // The relocation counter increments. This stops the session loop from
        // treating the world's copy of this move as a correction to resync to.
        assert_eq!(state.relocations, 1);

        for _ in 0..41 {
            state.mover.advance(0.025, None);
        }
        assert_eq!(state.mover.take_finished_ride(), Some(41));
        // The player ends facing the charged unit, whose position only the
        // world state knows.
        assert!(state.mover.position().orientation.abs() < 0.01);
    }

    /// The observers' copy of a speed change, which this client previously did
    /// not read. It is the only speed change that arrives about a unit this
    /// client does not control.
    ///
    /// `SMSG_FORCE_*_SPEED_CHANGE` goes only to the unit's controller
    /// (vmangos' `SendSpeedChangeToController` sends to `mover`), so
    /// `a_speed_change_about_someone_else_still_reaches_the_world` covers only
    /// a unit this client is moving. Every other client receives
    /// `MSG_MOVE_SET_*_SPEED`, which carries a whole movement block between the
    /// guid and the speed. Parsed as an ordinary heartbeat, the position is
    /// applied and the trailing speed is ignored.
    ///
    /// A player mounted at +60% moves at 11.2 y/s, against the 7.0 in the
    /// movement block they were created with. A client that misses this packet
    /// dead-reckons them 2.1 yards short every half-second and snaps them
    /// forward on each heartbeat, twice a second, while they are mounted, and
    /// does the reverse when they dismount.
    #[test]
    fn a_mount_speed_about_another_player_is_recorded_and_never_acknowledged() {
        let mut world = ObjectManager::new();
        world.player_guid = Some(42);
        world.apply_movement(7, &movement::MovementInfo::default());

        let mut body = crate::bytes::Writer::new();
        body.u8(0x01).u8(7); // packed guid
        let info = movement::MovementInfo {
            position: Position { x: 100.0, y: 200.0, z: 50.0, orientation: 1.0 },
            ..Default::default()
        };
        info.write(&mut body);
        body.f32(11.2); // the new run speed, flat rather than a rate

        let mut state = local();
        let (stats, replies) = apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::MSG_MOVE_SET_RUN_SPEED, body.buf),
        );

        assert_eq!(stats.speed_broadcasts, 1);
        assert_eq!(
            world.get(7).and_then(|e| e.speeds).map(|s| s.run()),
            Some(11.2),
            "the mounted player is still being reckoned at 7.0"
        );
        // The block is part of the packet and is applied, which is why this is
        // not simply `speed_change` with a different offset.
        assert_eq!(world.get(7).and_then(|e| e.position).map(|p| p.x), Some(100.0));
        // Not answered. The acknowledgement belongs to the unit's controller,
        // which is not this client; sending one triggers `OnWrongAckData`, and
        // three of those get the player kicked.
        assert!(replies.is_empty(), "acknowledged a change about someone else");
    }

    /// The other observer family: speed changes about a server-driven unit,
    /// which is every creature.
    ///
    /// vmangos' `SendSpeedChangeToAll` uses `moveTypeToOpcode[mtype][0]`, which
    /// is `SMSG_SPLINE_SET_*`. Its body is the packed guid and the speed with no
    /// movement block, because a creature on a path has no position worth
    /// stating. Every enrage, daze, fleeing mob and speed-changing aura on a
    /// creature arrives this way.
    #[test]
    fn a_creature_speed_change_carries_no_movement_block() {
        let mut world = ObjectManager::new();
        world.apply_movement(9, &movement::MovementInfo::default());

        let mut body = crate::bytes::Writer::new();
        body.u8(0x01).u8(9);
        body.f32(3.5); // half speed: a fleeing mob that has been hamstrung

        let (stats, replies) = apply(
            &mut world,
            None,
            &packet(Opcode::SMSG_SPLINE_SET_RUN_SPEED, body.buf),
        );

        assert_eq!(stats.speed_broadcasts, 1);
        assert_eq!(world.get(9).and_then(|e| e.speeds).map(|s| s.run()), Some(3.5));
        assert!(replies.is_empty());
    }

    /// All twelve observer speed opcodes resolve to the right slot in
    /// [`movement::Speeds`] and to the family whose body layout that opcode
    /// has.
    ///
    /// A transposed entry fails silently. For example, writing a run speed into
    /// the swim slot leaves a runner reckoned correctly on land and wrongly as
    /// soon as they enter water, and the symptom does not point at this table.
    #[test]
    fn every_observer_speed_opcode_names_its_own_slot() {
        use movement::SpeedBroadcast::{Observed, Spline};
        let table = [
            (Opcode::MSG_MOVE_SET_WALK_SPEED, 0, Observed),
            (Opcode::MSG_MOVE_SET_RUN_SPEED, 1, Observed),
            (Opcode::MSG_MOVE_SET_RUN_BACK_SPEED, 2, Observed),
            (Opcode::MSG_MOVE_SET_SWIM_SPEED, 3, Observed),
            (Opcode::MSG_MOVE_SET_SWIM_BACK_SPEED, 4, Observed),
            (Opcode::MSG_MOVE_SET_TURN_RATE, 5, Observed),
            (Opcode::SMSG_SPLINE_SET_WALK_SPEED, 0, Spline),
            (Opcode::SMSG_SPLINE_SET_RUN_SPEED, 1, Spline),
            (Opcode::SMSG_SPLINE_SET_RUN_BACK_SPEED, 2, Spline),
            (Opcode::SMSG_SPLINE_SET_SWIM_SPEED, 3, Spline),
            (Opcode::SMSG_SPLINE_SET_SWIM_BACK_SPEED, 4, Spline),
            (Opcode::SMSG_SPLINE_SET_TURN_RATE, 5, Spline),
        ];
        for (op, slot, kind) in table {
            assert_eq!(
                movement::broadcast_speed_slot(op),
                Some((slot, kind)),
                "{op:?}"
            );
        }
        // The controller's own family is not in the table; otherwise a forced
        // change would be recorded twice and never acknowledged.
        assert_eq!(
            movement::broadcast_speed_slot(Opcode::SMSG_FORCE_RUN_SPEED_CHANGE),
            None
        );
    }

    /// A root is acknowledged, and the acknowledgement carries the root flag.
    ///
    /// These are two separate server rules, and breaking either ends the
    /// session. Not answering leaves the change pending:
    /// `CheckPendingMovementChanges` fires `OnFailedToAckChange` after four
    /// seconds (`PendingAckDelay`; at a threshold of 3 the player is kicked)
    /// and then applies the root anyway, after which every step is
    /// `CHEAT_TYPE_ROOT_MOVE`. Answering without `MOVEFLAG_ROOT` in the block
    /// ends the session sooner: `HandleMoveRootAck` calls `KickPlayer()`
    /// directly. `Anticheat.log` on the reference server records both, for
    /// this client's own characters.
    #[test]
    fn a_root_is_acknowledged_with_the_flag_the_server_demands() {
        let mut world = ObjectManager::new();
        world.player_guid = Some(42);
        world.apply_movement(42, &movement::MovementInfo::default());
        let mut state = local();
        state
            .mover
            .set_controls(movement::Controls { forward: true, ..Default::default() });

        let mut body = crate::bytes::Writer::new();
        body.u8(0x01).u8(42); // packed guid
        body.u32(9); // movement counter
        let (stats, replies) = apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::SMSG_FORCE_MOVE_ROOT, body.buf),
        );

        assert_eq!(stats.flag_changes, 1);
        let (op, body) = replies.first().expect("an ack");
        assert_eq!(*op, Opcode::CMSG_FORCE_MOVE_ROOT_ACK);

        let mut r = crate::bytes::Reader::new(body);
        assert_eq!(r.u64(), 42, "the guid goes back plain, not packed");
        assert_eq!(r.u32(), 9, "the counter is what retires the pending change");
        let info = movement::MovementInfo::read(&mut r).expect("a movement block");
        assert!(
            info.has(movement::move_flags::ROOT),
            "HandleMoveRootAck kicks on a root ack with no root flag"
        );
        assert!(
            !info.is_moving(),
            "moving while rooted is CHEAT_TYPE_ROOT_MOVE, and it is sticky"
        );
        // The local simulation is rooted too, not only the reported flags.
        state.mover.advance(1.0, None);
        assert_eq!(state.mover.position().x, 0.0);

        // The unroot puts the held keys back, so the character resumes without
        // the player having to let go and press again.
        let mut body = crate::bytes::Writer::new();
        body.u8(0x01).u8(42);
        body.u32(10);
        let (_, replies) = apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::SMSG_FORCE_MOVE_UNROOT, body.buf),
        );
        assert_eq!(
            replies.first().map(|(op, _)| *op),
            Some(Opcode::CMSG_FORCE_MOVE_UNROOT_ACK)
        );
        assert!(state.mover.info.has(movement::move_flags::FORWARD));
    }

    /// A far teleport is answered even by a caller with no simulation.
    ///
    /// vmangos' `Player::TeleportTo` raises `SetSemaphoreTeleportFar` before
    /// sending `SMSG_NEW_WORLD`, and only this acknowledgement lowers it. A
    /// client that does not answer is held on no map: nothing it sends is
    /// processed and nothing comes back. The CLI's snapshot pump has no `Mover`
    /// to resync, and its dispatch previously did not handle this packet. It
    /// now sends the acknowledgement, which is the part the server requires.
    ///
    /// The body is empty on purpose: `HandleMoveWorldportAckOpcode(WorldPacket&
    /// /*recvData*/)` does not read any of it.
    #[test]
    fn a_far_teleport_is_acknowledged_with_or_without_a_simulation() {
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_le_bytes()); // map id — Kalimdor
        for f in [-8000.0f32, 1500.0, 20.0, 0.5] {
            body.extend_from_slice(&f.to_le_bytes());
        }
        let pkt = packet(Opcode::SMSG_NEW_WORLD, body);

        // No simulation: the ack still goes out.
        let (_, replies) = apply(&mut ObjectManager::new(), None, &pkt);
        assert_eq!(
            replies.first().map(|(op, body)| (*op, body.len())),
            Some((Opcode::MSG_MOVE_WORLDPORT_ACK, 0)),
            "the ack is a signal and its body is empty"
        );

        // With a simulation, the map and the mover are updated as well, and the
        // relocation is counted so the caller can re-baseline.
        let mut state = local();
        let mut world = ObjectManager::new();
        let (_, replies) = apply(&mut world, Some(&mut state), &pkt);
        assert_eq!(state.map_id, 1);
        assert_eq!(state.mover.position().x, -8000.0);
        assert_eq!(state.relocations, 1);
        assert_eq!(replies.len(), 1);
    }

    /// A login on a different map from the character list updates the map,
    /// and is not treated as a teleport.
    ///
    /// `SMSG_LOGIN_VERIFY_WORLD` is the first packet of the login burst and the
    /// only packet in which the server states which map a character is on.
    /// It usually repeats the character-list row. It matters when vmangos'
    /// `Player::LoadFromDB` moves a character out of an instance that has been
    /// reset, which it does with a bare `Relocate` and no teleport packet.
    ///
    /// The test checks three things. The map is taken from the packet. No
    /// acknowledgement is sent, because no teleport semaphore is raised (that
    /// belongs to `SMSG_NEW_WORLD`). The position is not adopted, because the
    /// character's create block states it a few packets later, together with
    /// the speeds and flags that must agree with it.
    #[test]
    fn a_login_that_landed_on_another_map_moves_the_map_and_answers_nothing() {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_le_bytes()); // Azeroth — the entrance
        for f in [-7178.4f32, -922.2, 166.1, 2.0] {
            body.extend_from_slice(&f.to_le_bytes());
        }
        let pkt = packet(Opcode::SMSG_LOGIN_VERIFY_WORLD, body);

        let mut state = local();
        // The character list's map: Blackrock Depths, where this character
        // logged out. The server has placed it elsewhere.
        state.map_id = 230;
        let was = state.mover.position();
        let mut world = ObjectManager::new();
        let (stats, replies) = apply(&mut world, Some(&mut state), &pkt);
        assert_eq!(state.map_id, 0, "the map is the server's, not the row's");
        assert_eq!(state.mover.position().x, was.x, "the create block owns the position");
        assert_eq!(state.relocations, 0, "nothing was relocated — this is a login");
        assert!(replies.is_empty(), "there is no semaphore to lower: {replies:?}");
        assert!(stats.warnings.is_empty());
        assert_eq!(stats.landed_on_map, Some(0), "…and a caller with no simulation can read it");

        // A caller with no `LocalState` (`vale login`'s snapshot pump) still
        // receives the map. That is the reason `landed_on_map` exists.
        let (stats, _) = apply(&mut ObjectManager::new(), None, &pkt);
        assert_eq!(stats.landed_on_map, Some(0));

        // A body that does not parse leaves the character-list map in place.
        // That map is correct in every case except the one above, so keeping
        // it is the conservative choice.
        let mut state = local();
        state.map_id = 230;
        let (stats, _) = apply(
            &mut ObjectManager::new(),
            Some(&mut state),
            &packet(Opcode::SMSG_LOGIN_VERIFY_WORLD, vec![0; 8]),
        );
        assert_eq!(state.map_id, 230);
        assert_eq!(stats.landed_on_map, None);
        assert_eq!(stats.warnings.len(), 1, "{:?}", stats.warnings);
    }

    /// An unreadable body is reported, not dropped.
    ///
    /// Every `parse_*` here returns `Option`, and each call site used to decide
    /// for itself whether to report a `None`. The movement broadcast did and
    /// the monster move did not, so a length bug in the packet that moves every
    /// creature produced a frozen creature and no warning.
    #[test]
    fn a_body_that_will_not_read_says_so() {
        let mut world = ObjectManager::new();
        let (stats, _) = apply(
            &mut world,
            None,
            &packet(Opcode::SMSG_MONSTER_MOVE, vec![0x01, 7]),
        );
        assert_eq!(stats.monster_moves, 0);
        assert_eq!(stats.warnings.len(), 1, "{:?}", stats.warnings);
        assert!(stats.warnings[0].contains("SMSG_MONSTER_MOVE"));

        // An opcode with no handler is not a warning. The server legitimately
        // sends packets this client does not implement, and counting them with
        // the parse failures would hide the failures.
        //
        // The example must be an opcode with no handler.
        // `SMSG_GMTICKET_GETTICKET` answers a GM ticket, and this client has
        // no ticket window. (`SMSG_EMOTE`, `SMSG_WEATHER` and
        // `SMSG_PLAYED_TIME` were used here earlier and are all handled now.)
        let (stats, _) =
            apply(&mut world, None, &packet(Opcode::SMSG_GMTICKET_GETTICKET, Vec::new()));
        assert!(stats.warnings.is_empty());
        assert_eq!(stats.other.values().sum::<u32>(), 1);
    }

    /// The login burst is the only time the server states the spellbook and
    /// the action bar. Both are stored in the world state, and each increments
    /// `spellbook_version`, which readers use to rebuild the bar.
    #[test]
    fn the_login_burst_fills_the_spellbook_and_the_bar() {
        use crate::bytes::Writer;
        let mut world = ObjectManager::new();

        let mut book = Writer::new();
        book.u8(0).u16(2);
        book.u16(133).u16(0).u16(crate::play::spells::SPELL_ATTACK as u16).u16(0);
        book.u16(0);
        let (stats, _) = apply(&mut world, None, &packet(Opcode::SMSG_INITIAL_SPELLS, book.buf));
        assert_eq!(stats.spells_known, 2);
        assert_eq!(world.spellbook.known, vec![133, crate::play::spells::SPELL_ATTACK]);

        let mut bar = Writer::new();
        bar.u32(crate::play::spells::SPELL_ATTACK).u32(0).u32(133);
        let (stats, _) = apply(&mut world, None, &packet(Opcode::SMSG_ACTION_BUTTONS, bar.buf));
        assert_eq!(stats.action_buttons, 2);
        assert_eq!(world.action_buttons[1].slot, 2);
        assert_eq!(world.spellbook_version, 2, "each statement bumps the version");
    }

    /// A refused swing has no body; the opcode is the message. It is the only
    /// trace the refusal leaves. If it is dropped, a player who presses attack
    /// at twenty yards gets no feedback.
    #[test]
    fn a_refused_swing_reaches_the_event_queue() {
        let mut world = ObjectManager::new();
        let (stats, _) = apply(
            &mut world,
            None,
            &packet(Opcode::SMSG_ATTACKSWING_NOTINRANGE, Vec::new()),
        );
        assert_eq!(stats.attack_refusals, 1);
        assert!(stats.warnings.is_empty(), "an empty body is the packet");
        assert_eq!(
            world.take_events(),
            vec![crate::play::spells::PlayerEvent::AttackRefused(
                crate::play::spells::AttackRefusal::NotInRange
            )]
        );
        // Taking the events empties the queue, as it does for the chat queue.
        assert!(world.take_events().is_empty());
    }

    /// The only packet the server sends about an auto-repeat ranged attack is
    /// the one saying it has stopped. As with the swing refusal above, the
    /// opcode is the whole message.
    ///
    /// There is no packet for the start (starting one is an ordinary
    /// `CMSG_CAST_SPELL`). A client that dropped `SMSG_CANCEL_AUTO_REPEAT`
    /// could turn Auto Shot on and never turn it off, and the button would
    /// keep flashing after the fight ended. vmangos'
    /// `SpellCaster::InterruptSpell` sends this packet when the target dies,
    /// when the target leaves range, and when a wand user moves.
    #[test]
    fn the_end_of_a_volley_reaches_the_event_queue() {
        let mut world = ObjectManager::new();
        let (stats, _) = apply(
            &mut world,
            None,
            &packet(Opcode::SMSG_CANCEL_AUTO_REPEAT, Vec::new()),
        );
        assert!(stats.warnings.is_empty(), "an empty body is the packet");
        assert!(
            stats.other.is_empty(),
            "an unread opcode is counted apart and this one is read"
        );
        assert_eq!(
            world.take_events(),
            vec![crate::play::spells::PlayerEvent::AutoRepeatCancelled]
        );
    }

    /// `SMSG_SPELL_START` and `SMSG_SPELL_GO` are broadcast for every caster,
    /// and each raises a player event when the caster is the local player.
    ///
    /// `SMSG_SPELL_START` shows the cast bar: it is the only thing that raises
    /// `SPELLCAST_START` in the 1.12 client, and this client does not draw a
    /// cast bar when the key is pressed. `SMSG_SPELL_GO` is the only packet that
    /// empties the next-swing queue. See
    /// [`crate::play::spells::PlayerEvent::CastStarted`].
    ///
    /// The test checks three things, each of which a one-line mistake would
    /// break: the caster is the second packed guid in the body (the first is
    /// the cast item), another unit's cast raises no event, and neither packet
    /// raises the other's event.
    #[test]
    fn both_halves_of_our_own_cast_reach_the_event_queue() {
        use crate::bytes::Writer;
        let mut world = ObjectManager::new();
        world.player_guid = Some(7);
        // `SMSG_SPELL_START` carries the cast timer and `SMSG_SPELL_GO` a hit
        // list. That is the only difference between the two bodies.
        let start = |caster: u64, timer: u32| {
            let mut w = Writer::new();
            w.packed_guid(0).packed_guid(caster).u32(78).u16(0).u32(timer);
            w.buf
        };
        let go = |caster: u64| {
            let mut w = Writer::new();
            w.packed_guid(0).packed_guid(caster).u32(78).u16(0).u8(0);
            w.buf
        };

        apply(&mut world, None, &packet(Opcode::SMSG_SPELL_GO, go(99)));
        apply(&mut world, None, &packet(Opcode::SMSG_SPELL_START, start(99, 2500)));
        assert!(world.take_events().is_empty(), "somebody else's spell");

        apply(&mut world, None, &packet(Opcode::SMSG_SPELL_START, start(7, 2500)));
        assert_eq!(
            world.take_events(),
            vec![crate::play::spells::PlayerEvent::CastStarted {
                spell_id: 78,
                cast_time_ms: 2500
            }]
        );

        apply(&mut world, None, &packet(Opcode::SMSG_SPELL_GO, go(7)));
        assert_eq!(
            world.take_events(),
            vec![crate::play::spells::PlayerEvent::CastReleased { spell_id: 78 }]
        );
    }

    /// Several of the local player's releases in a row raise one event each,
    /// in order, each naming its own spell. This is why the cast bar is ended
    /// from this queue rather than from the entity's `last_spell`.
    ///
    /// `last_spell` is one field, so the second release below overwrites the
    /// first: a reader polling `casts_released` and then reading the spell gets
    /// 21084 and never 78. This happens in play.
    /// [`crate::state::objects::Entity::recent_spells`] exists because Charge
    /// was measured as two releases inside one 25 ms tick, and a seal proc on
    /// every swing does the same at melee speed. When the bar was ended from
    /// `last_spell` and missed the first release, nothing cleared the bar, so
    /// `in_progress` stayed true and every press for the rest of the session was
    /// refused with "another action is in progress".
    #[test]
    fn a_burst_of_our_own_releases_keeps_one_event_per_spell() {
        use crate::bytes::Writer;
        let mut world = ObjectManager::new();
        world.player_guid = Some(7);
        let go = |spell: u32| {
            let mut w = Writer::new();
            w.packed_guid(0).packed_guid(7).u32(spell).u16(0).u8(0);
            w.buf
        };
        // The player's spell, then the spell it triggered, with no read of the
        // queue in between.
        apply(&mut world, None, &packet(Opcode::SMSG_SPELL_GO, go(78)));
        apply(&mut world, None, &packet(Opcode::SMSG_SPELL_GO, go(21084)));
        assert_eq!(
            world.take_events(),
            vec![
                crate::play::spells::PlayerEvent::CastReleased { spell_id: 78 },
                crate::play::spells::PlayerEvent::CastReleased { spell_id: 21084 },
            ],
            "both releases, each naming its own spell"
        );
        // The check that the `last_spell` field holds only the second release
        // is `state::objects::tests::a_burst_of_releases_leaves_only_the_last_in_the_field`,
        // next to the object manager's own fixtures.
    }

    /// `SMSG_ATTACKSTART` is broadcast for every attacker, so the world state
    /// must record it as the local player's attack only when the attacker is
    /// the local player.
    #[test]
    fn only_our_own_swing_becomes_our_own_attack_state() {
        use crate::bytes::Writer;
        let mut world = ObjectManager::new();
        world.player_guid = Some(7);

        let mut theirs = Writer::new();
        theirs.u64(99).u64(1234);
        apply(&mut world, None, &packet(Opcode::SMSG_ATTACKSTART, theirs.buf));
        assert_eq!(world.attacking, None, "somebody else's fight");

        let mut ours = Writer::new();
        ours.u64(7).u64(1234);
        apply(&mut world, None, &packet(Opcode::SMSG_ATTACKSTART, ours.buf));
        assert_eq!(world.attacking, Some(1234));

        // The stop uses packed guids where the start uses plain ones, and its
        // victim may be an empty packed guid, meaning no particular target.
        let mut stop = Writer::new();
        stop.packed_guid(7).packed_guid(0).u32(0);
        apply(&mut world, None, &packet(Opcode::SMSG_ATTACKSTOP, stop.buf));
        assert_eq!(world.attacking, None);
    }

    /// The warning list is capped, because a field mis-read on every packet
    /// produces one warning per packet and later copies add no information.
    #[test]
    fn warnings_are_capped() {
        let mut world = ObjectManager::new();
        let mut stats = PumpStats::default();
        let mut replies = Replies::default();
        for _ in 0..100 {
            apply_packet(
                &mut Incoming {
                    world: &mut world,
                    stats: &mut stats,
                    replies: &mut replies,
                    local: None,
                },
                &packet(Opcode::SMSG_MONSTER_MOVE, vec![0x01, 7]),
            );
        }
        assert_eq!(stats.warnings.len(), MAX_WARNINGS);
        assert_eq!(stats.packets, 100, "every packet is still counted");
    }

    /// A movement flag change about a unit no player is moving.
    ///
    /// vmangos' `Unit::SetRooted` calls `SendMovementFlagChangeToAll` for a
    /// creature, and under 1.9.4 its body is only a packed guid. The opcode is
    /// the entire message and the flag comes from a table. It is not
    /// acknowledged, because no client is being asked to obey it.
    ///
    /// Dropping it is silent and permanent. A creature's flags arrive once, in
    /// its create block. No values update carries them, and `SMSG_MONSTER_MOVE`
    /// carries a path with no flags. These twelve opcodes are the only
    /// restatement.
    #[test]
    fn a_root_about_a_creature_is_a_packed_guid_and_is_never_acknowledged() {
        let mut world = ObjectManager::new();
        world.player_guid = Some(42);
        world.apply_movement(
            9,
            &movement::MovementInfo {
                flags: movement::move_flags::FORWARD,
                ..Default::default()
            },
        );

        let mut state = local();
        let (stats, replies) = apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::SMSG_SPLINE_MOVE_ROOT, vec![0x01, 9]),
        );

        assert_eq!(stats.spline_flag_changes, 1);
        assert!(
            world.get(9).map(|e| e.move_flags() & movement::move_flags::ROOT != 0) == Some(true),
            "the creature is still being drawn as one that can move"
        );
        // Not answered, for the same reason as the speed broadcast: the server
        // has no pending change to close, and three wrong acknowledgements get
        // the player kicked.
        assert!(replies.is_empty(), "acknowledged a change nobody asked us to obey");

        // This family is the only statement of the flag, so the unroot must
        // arrive the same way and clear it.
        let (stats, _) = apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::SMSG_SPLINE_MOVE_UNROOT, vec![0x01, 9]),
        );
        assert_eq!(stats.spline_flag_changes, 1);
        assert_eq!(
            world.get(9).map(|e| e.move_flags() & movement::move_flags::ROOT),
            Some(0)
        );
    }

    /// A near teleport about another unit is an ordinary movement broadcast,
    /// and it is the only packet that moves a player who is standing still.
    ///
    /// vmangos' `SendTeleportToObservers` sends it twice per teleport, to
    /// observers around the old position and around the new one, and both
    /// copies carry the destination. It is therefore applied twice and must be
    /// idempotent.
    #[test]
    fn a_near_teleport_moves_the_unit_it_names() {
        let mut world = ObjectManager::new();
        world.player_guid = Some(42);
        world.apply_movement(7, &movement::MovementInfo::default());

        let mut body = crate::bytes::Writer::new();
        body.u8(0x01).u8(7);
        movement::MovementInfo {
            position: Position { x: 300.0, y: -50.0, z: 12.0, orientation: 2.0 },
            ..Default::default()
        }
        .write(&mut body);

        let mut state = local();
        let pkt = packet(Opcode::MSG_MOVE_TELEPORT, body.buf);
        let (_, replies) = apply(&mut world, Some(&mut state), &pkt);
        assert_eq!(world.get(7).and_then(|e| e.position).map(|p| p.x), Some(300.0));
        assert!(replies.is_empty(), "the ack belongs to whoever was teleported");

        // The second copy sets the same position rather than moving it again.
        apply(&mut world, Some(&mut state), &pkt);
        assert_eq!(world.get(7).and_then(|e| e.position).map(|p| p.x), Some(300.0));
    }

    /// The three packets whose only content is a sound, and the reversed field
    /// order between `SMSG_PLAY_OBJECT_SOUND` and the spell-visual packets.
    ///
    /// No other packet implies any of them: every scripted line, gate sound
    /// and zone-wide announcement is sent by vmangos'
    /// `WorldObject::PlayDirectSound` or one of its two siblings, or it is
    /// silent. They go on the player event queue rather than into the world
    /// state because there is no state to update: two arrivals are two sounds.
    #[test]
    fn a_pushed_sound_reaches_the_queue_and_keeps_its_placement() {
        use crate::play::sound::Cue;
        let mut world = ObjectManager::new();
        world.player_guid = Some(42);
        let mut state = local();

        let (stats, replies) = apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::SMSG_PLAY_SOUND, 11803u32.to_le_bytes().to_vec()),
        );
        assert_eq!(stats.pushed_sounds, 1);
        assert!(replies.is_empty(), "a noise is not answered");
        assert_eq!(
            world.take_events(),
            vec![crate::play::spells::PlayerEvent::PlaySound(Cue::Direct(11803))]
        );

        apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::SMSG_PLAY_MUSIC, 53u32.to_le_bytes().to_vec()),
        );
        assert_eq!(
            world.take_events(),
            vec![crate::play::spells::PlayerEvent::PlaySound(Cue::Music(53))],
            "music is a different channel, not a different volume"
        );

        // The positioned sound. Its body is sound id then guid, where the two
        // spell-visual packets in the next test are guid then kit id.
        let mut body = crate::bytes::Writer::new();
        body.u32(1129).u64(0xF130_0000_0000_002A);
        apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::SMSG_PLAY_OBJECT_SOUND, body.buf),
        );
        assert_eq!(
            world.take_events(),
            vec![crate::play::spells::PlayerEvent::PlaySound(Cue::Object {
                sound_id: 1129,
                guid: 0xF130_0000_0000_002A,
            })]
        );
    }

    /// A `SpellVisualKit` sent for a unit, which is how eating and drinking are
    /// shown.
    ///
    /// vmangos sends kit 406 (food) or 438 (drink) on every regeneration tick
    /// a character spends sitting with either, and both kits use `animID 61`,
    /// `EmoteEat`. No other packet says a character is eating.
    ///
    /// Recorded on the entity rather than pushed on the event queue, because
    /// it is about a unit and stays valid while that unit's models exist.
    /// Recorded on two counters, because the visual describes what a unit did
    /// and the impact describes what was done to it.
    #[test]
    fn a_pushed_visual_lands_on_the_unit_it_names_and_keeps_the_two_apart() {
        let mut world = ObjectManager::new();
        world.player_guid = Some(42);
        world.apply_movement(9, &movement::MovementInfo::default());
        let mut state = local();

        let mut body = crate::bytes::Writer::new();
        body.u64(9).u32(406);
        let (stats, replies) = apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::SMSG_PLAY_SPELL_VISUAL, body.buf),
        );
        assert_eq!(stats.pushed_visuals, 1);
        assert!(replies.is_empty());
        assert!(world.take_events().is_empty(), "not queue news");
        assert_eq!(world.get(9).map(|e| (e.spell_visuals, e.last_spell_visual)), Some((1, 406)));
        assert_eq!(world.get(9).map(|e| e.spell_impacts), Some(0));

        // The impact is the same body on a different counter.
        let mut body = crate::bytes::Writer::new();
        body.u64(9).u32(285);
        apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::SMSG_PLAY_SPELL_IMPACT, body.buf),
        );
        assert_eq!(world.get(9).map(|e| (e.spell_impacts, e.last_spell_impact)), Some((1, 285)));
        assert_eq!(
            world.get(9).map(|e| (e.spell_visuals, e.last_spell_visual)),
            Some((1, 406)),
            "an impact must not overwrite what the unit is doing"
        );

        // A guid this client has not been sent creates no entity. This matches
        // the 1.12 client: vmangos' comment on the sibling packet says the
        // client ignores one for a unit it has not loaded.
        let mut body = crate::bytes::Writer::new();
        body.u64(777).u32(406);
        apply(
            &mut world,
            Some(&mut state),
            &packet(Opcode::SMSG_PLAY_SPELL_VISUAL, body.buf),
        );
        assert!(world.get(777).is_none(), "a kit id is not a reason to invent a unit");
    }
}
