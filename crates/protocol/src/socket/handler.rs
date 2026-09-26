//! **Every packet this client understands, in one match.**
//!
//! The dispatch used to be in two places. [`apply_packet`] took only an
//! [`ObjectManager`], so it could neither answer the server nor move the local
//! player — and the seven packets that need to do one of those were therefore
//! intercepted by the live session loop *before* it, each ending in an early
//! return. The split was real and the rule behind it was correct; it was also
//! written down nowhere, so "does this client handle `SMSG_EMOTE`?" was a
//! question you answered by grepping two files, and "why is this one over here?"
//! was a question you answered by inferring it.
//!
//! What that cost, concretely: a `SMSG_FORCE_RUN_SPEED_CHANGE` about **another**
//! unit was acknowledged and then returned from, so it never reached the world
//! state at all. `Entity::speeds` is written only by an update block's movement
//! section, so a hasted or slowed player nearby kept their old speed and
//! [`ObjectManager::advance`] dead-reckoned them at it until the server happened
//! to send them a full movement block. Nothing failed. They drifted, and were
//! corrected — which arrives as a jump on a `MSG_MOVE_*` broadcast, i.e. as
//! "mobs teleport around" for the third time.
//!
//! ## A handler answers by pushing, not by writing
//!
//! Handlers run with the world lock held, and a socket write under that lock
//! blocks every reader on network latency — the reason `resolve_names` copies
//! its work list out before touching the socket. So a reply goes into
//! [`Replies`] and the caller flushes it after unlocking. That also removes the
//! asymmetry a socket handle would have introduced: `WorldSession::pump` has no
//! session loop to answer with, but it *should* still acknowledge a teleport
//! (until it does, the server holds the player on no map), and now it does.
//!
//! ## Local state is a parameter, not a reason to have a second dispatch
//!
//! [`LocalState`] is the client's own player: the [`Mover`], the map it is on,
//! the session clock that stamps `ctime`. `pump` passes `None` and the handlers
//! that need it do the half they can — a snapshot has no simulation to resync,
//! but the world state and the acknowledgement are the same either way.
//!
//! ## The arm bodies live in child modules; the match does not
//!
//! Each arm below is **one call** into a `pub(super) fn` in [`world`], [`query`],
//! [`chat`] or [`acks`], and the match itself stays whole as the dispatch table.
//! That distinction is the whole of it, and it is not the same change as
//! splitting the dispatch — which is what this module already paid for once,
//! above. There is still exactly one place an opcode becomes an action; what
//! moved is only *what each arm does once it gets there*.
//!
//! The reason to move it is arithmetic. Twenty-one opcodes at fourteen lines an
//! arm is a match you can read; this client is aiming at a faithful 1.12
//! client, where the dispatched population is north of a hundred and fifty, and
//! the same style would be a two-thousand-line match with the table buried in
//! it. Bodies out, table in — the table stays one screen and grows by a line per
//! packet.
//!
//! It also puts the prose where rustdoc can see it. Every one of these arms
//! carried a comment explaining what the packet costs when it is mishandled, and
//! as `//` inside a match arm none of that was documentation. On a
//! `pub(super) fn` it is.

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

/// How many parse warnings to keep. A systematically mis-read field would
/// otherwise produce thousands of identical lines, and the tenth says nothing
/// the first did not.
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
/// character — a teleport, a far teleport, a forced speed change — are ordinary
/// arms of the one dispatch rather than a second one.
pub struct LocalState {
    /// The local simulation. A server correction always beats dead reckoning.
    pub mover: Mover,
    /// Which map the character is on. `SMSG_NEW_WORLD` changes it, and the
    /// ground lookup is bound to it — a lookup left pointing at the map just
    /// left does not fail, it *answers*.
    pub map_id: u32,
    /// Session clock, and the source of `MovementInfo::time`.
    pub started: Instant,
    /// Round-trip time of the last `CMSG_PING`.
    pub latency_ms: u32,
    /// When the outstanding ping went out.
    pub ping_sent_at: Option<Instant>,
    /// How many times a packet has **relocated** the player.
    ///
    /// A counter for the same reason `Entity::position_updates` is one: the
    /// caller re-baselines its own idea of where the server has us, and cannot
    /// tell a relocation it did from one it did not by comparing positions —
    /// the mover has already adopted it.
    pub relocations: u32,
    /// **Whose body the movement packets are about** — the mover, and the guid
    /// `CMSG_SET_ACTIVE_MOVER` last named.
    ///
    /// The character's own for the whole of an ordinary session, a possessed
    /// unit while Eye of Kilrogg, Mind Control or Eyes of the Beast lasts, and
    /// **zero** while the server has taken control away without giving it to
    /// anybody — which is a fear or a confusion, and is a state the reference
    /// can be in too (it writes a zero guid, and skips the send for one).
    ///
    /// Derived rather than latched from a packet: see
    /// [`crate::socket::session::SessionLoop::tick_view`], which follows the
    /// client's per-frame view update. [`Self::mover`] simulates whatever this
    /// names.
    pub mover_guid: u64,
    /// **The client has told the server it is looking through something** — the
    /// latch the client's view update keeps, and what stops
    /// `CMSG_FAR_SIGHT` going out on every tick of a far sight.
    pub looking_through: bool,
    /// **The transfer now in flight began on a boat**, as
    /// `SMSG_TRANSFER_PENDING`'s two extra dwords say — `Player::ExecuteTeleportFar`
    /// writes them only for a character standing on a transport.
    ///
    /// Read by the `SMSG_NEW_WORLD` arm and by nothing else: it is what
    /// separates a teleport that *leaves* a boat, after which the offset on
    /// record means nothing, from `Transport::TeleportTransport`, which is the
    /// boat taking the character with it and after which the offset is the only
    /// half still true. See [`movement::Mover::transfer_aboard`].
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
    /// `ctime` must be non-zero and must never decrease — zero is
    /// `CHEAT_TYPE_NULL_CLIENT_TIME` and any decrease is `CHEAT_TYPE_TIME_BACK`
    /// — so it is milliseconds since the session started, offset by one so the
    /// very first packet is not zero.
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
    /// `None` for a caller with no simulation of its own — the CLI's snapshot
    /// pump. The handlers degrade rather than branching into a second dispatch.
    pub local: Option<&'a mut LocalState>,
}

/// Fold one received packet into the world state, and queue whatever it owes
/// the server.
///
/// **The one place an opcode is turned into an action.** Both the CLI's
/// snapshot pump and the live session loop go through here, so a packet cannot
/// be understood by one and not the other — a divergence that would surface as
/// "the renderer is stale but the CLI is fine".
pub fn apply_packet(ctx: &mut Incoming, pkt: &Packet) {
    ctx.stats.packets += 1;
    ctx.stats.saw(pkt);

    let Some(op) = pkt.opcode() else {
        ctx.stats.unhandled(pkt);
        return;
    };

    match op {
        // ---- the world ----------------------------------------------------
        Opcode::SMSG_UPDATE_OBJECT => world::update(ctx, pkt),
        Opcode::SMSG_COMPRESSED_UPDATE_OBJECT => world::compressed_update(ctx, pkt),
        // **A bag of movement packets, and it is most of the movement on a busy
        // realm** — see [`world::compressed_moves`], where the server's own
        // rate switch and what dropping it costs are written down.
        Opcode::SMSG_COMPRESSED_MOVES => world::compressed_moves(ctx, pkt),
        Opcode::SMSG_MONSTER_MOVE => world::monster_move(ctx, pkt),
        // …and the same move on a moving floor, which is one extra packed guid
        // and a path in that transport's frame rather than in the world's. Read
        // as the plain one it places every passenger of every boat within a few
        // yards of the map's origin. See [`world::monster_move_transport`].
        Opcode::SMSG_MONSTER_MOVE_TRANSPORT => world::monster_move_transport(ctx, pkt),
        Opcode::SMSG_ATTACKERSTATEUPDATE => world::attack(ctx, pkt),
        Opcode::SMSG_SPELLNONMELEEDAMAGELOG => world::spell_damage(ctx, pkt),
        Opcode::SMSG_SPELLHEALLOG => world::spell_heal(ctx, pkt),
        Opcode::SMSG_EMOTE => world::emote(ctx, pkt),
        Opcode::SMSG_AI_REACTION => world::ai_reaction(ctx, pkt),
        // ---- …and the five the server plays *at* you ----------------------
        //
        // Nothing else on the wire implies any of them. A scripted noise, a
        // music track, a noise at an object, and the two that put a
        // `SpellVisualKit` on a unit — which between them are every boss line,
        // every gate, and the whole of what eating and drinking look like. See
        // [`crate::play::sound`].
        Opcode::SMSG_PLAY_SOUND | Opcode::SMSG_PLAY_MUSIC | Opcode::SMSG_PLAY_OBJECT_SOUND => {
            world::play_sound(ctx, pkt, op)
        }
        Opcode::SMSG_PLAY_SPELL_VISUAL => world::play_spell_visual(ctx, pkt, false),
        Opcode::SMSG_PLAY_SPELL_IMPACT => world::play_spell_visual(ctx, pkt, true),
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
        Opcode::SMSG_FISH_NOT_HOOKED => player::fish(ctx, false),
        Opcode::SMSG_FISH_ESCAPED => player::fish(ctx, true),

        // ---- …and the nine a fight is only *narrated* by --------------------
        //
        // None of these moves anything on screen. Every one of them is a line
        // in the combat log and nothing else, which is why they went unread for
        // so long and why dropping any of them is silent: the fight looks
        // identical and the window simply never mentions what happened.
        Opcode::SMSG_LOG_XPGAIN => world::xp_gain(ctx, pkt),
        Opcode::SMSG_PARTYKILLLOG => world::party_kill(ctx, pkt),
        Opcode::SMSG_ENVIRONMENTALDAMAGELOG => world::environmental_damage(ctx, pkt),
        Opcode::SMSG_SPELLDAMAGESHIELD => world::damage_shield(ctx, pkt),
        Opcode::SMSG_SPELLLOGMISS => world::spell_miss_log(ctx, pkt),
        Opcode::SMSG_SPELLENERGIZELOG => world::energize_log(ctx, pkt),
        Opcode::SMSG_PERIODICAURALOG => world::periodic_aura_log(ctx, pkt),
        Opcode::SMSG_SPELLLOGEXECUTE => world::spell_execute_log(ctx, pkt),
        Opcode::SMSG_SPELLDISPELLOG => world::spell_dispel_log(ctx, pkt),

        // ---- what *we* can do, and what came of trying ---------------------
        // ---- and what is *with* us: the pet's own family --------------------
        //
        // Two of the nine classes are unplayable without these. The bar is
        // state and the rest are edges; see `crate::play::pet`.
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
        Opcode::SMSG_ATTACKSTART => player::attack_state(ctx, pkt, true),
        Opcode::SMSG_ATTACKSTOP => player::attack_state(ctx, pkt, false),
        Opcode::SMSG_CANCEL_AUTO_REPEAT => player::auto_repeat_cancelled(ctx),
        Opcode::MSG_CHANNEL_START => player::channel_start(ctx, pkt),
        Opcode::MSG_CHANNEL_UPDATE => player::channel_update(ctx, pkt),
        Opcode::SMSG_UPDATE_AURA_DURATION => player::aura_duration(ctx, pkt),
        Opcode::SMSG_LEVELUP_INFO => player::levelup(ctx, pkt),
        // …and the refusal every *item* verb shares, which is the only thing on
        // the wire that answers a right-click the server threw away.
        Opcode::SMSG_INVENTORY_CHANGE_FAILURE => player::inventory_failed(ctx, pkt),
        // …and the one that says something *arrived*, which no update field
        // does: a stack growing by three says nothing about whether it was
        // looted, bought, crafted, mailed or traded. See [`player::item_received`].
        Opcode::SMSG_ITEM_PUSH_RESULT => player::item_received(ctx, pkt),

        // ---- reputation -----------------------------------------------------
        // Four packets about the 64 reputation-list slots. See
        // [`crate::play::reputation`]: the standings on the wire are *deltas*
        // from a `Faction.dbc` base, so none of these is drawable here.
        Opcode::SMSG_INITIALIZE_FACTIONS => player::initialize_factions(ctx, pkt),
        Opcode::SMSG_SET_FACTION_STANDING => player::faction_standing(ctx, pkt),
        Opcode::SMSG_SET_FACTION_VISIBLE => player::faction_visible(ctx, pkt),
        Opcode::SMSG_SET_FACTION_ATWAR => player::faction_at_war(ctx, pkt),
        Opcode::SMSG_SET_FORCED_REACTIONS => player::forced_reactions(ctx, pkt),

        // ---- quests ---------------------------------------------------------
        // Eleven packets and a conversation. See [`crate::play::quest`], which owns
        // the log's packed six-bit counters and the top bit that makes an
        // objective's target a game object — the two things in this family that
        // read plausibly wrong rather than failing.
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
        // **A reason, not a quest id**, which is the one arm in the family whose
        // body means something different from its neighbours'.
        Opcode::SMSG_QUESTGIVER_QUEST_INVALID => player::quest_refused(ctx, pkt),

        // ---- talking to an NPC ----------------------------------------------
        // The gossip menu (whose text is a round trip of its own), and the
        // vendor window one click deeper. See [`crate::play::gossip`].
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
        // …and the trainer window, which is the same shape one door along:
        // a hello, a list, and a verb with two answers. See [`crate::play::trainer`].
        Opcode::SMSG_TRAINER_LIST => player::trainer_list(ctx, pkt),
        Opcode::SMSG_TRAINER_BUY_SUCCEEDED => player::trainer_bought(ctx, pkt),
        Opcode::SMSG_TRAINER_BUY_FAILED => player::trainer_buy_failed(ctx, pkt),
        // …and the stable master, whose window is one packet and whose four
        // verbs share one answer. The list opcode is an `MSG_` and travels both
        // ways. See [`crate::play::stable`].
        Opcode::MSG_LIST_STABLED_PETS => player::stable_list(ctx, pkt),
        Opcode::SMSG_STABLE_RESULT => player::stable_result(ctx, pkt),
        // …and the banker, whose window is a guid and whose one verb only
        // ever answers with a refusal. See [`crate::play::bank`].
        Opcode::SMSG_SHOW_BANK => player::bank_show(ctx, pkt),
        Opcode::SMSG_BUY_BANK_SLOT_RESULT => player::bank_slot_result(ctx, pkt),
        // …and the flight master, which is the same shape again with one
        // difference worth the line: what its verb buys arrives as
        // `SMSG_MONSTER_MOVE`, three arms up, and nothing here knows about it.
        // See [`crate::play::taxi`].
        Opcode::SMSG_SHOWTAXINODES => player::taxi_show(ctx, pkt),
        Opcode::SMSG_TAXINODE_STATUS => player::taxi_node_status(ctx, pkt),
        Opcode::SMSG_NEW_TAXI_PATH => player::new_taxi_path(ctx),
        Opcode::SMSG_ACTIVATETAXIREPLY => player::taxi_reply(ctx, pkt),

        // ---- the party -------------------------------------------------------
        // Eight packets and only one of them is state: `SMSG_GROUP_LIST` is the
        // whole roster, re-sent to everybody whenever anything about the group
        // moves, and it is what a client that reads no other one still gets
        // right. The other six are edges. See [`crate::play::group`], which carries
        // the conditional loot tail and why a destroyed group is its own packet.
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

        // ---- and who you know ------------------------------------------------
        // The friends list, the ignore list and the /who search. Both lists
        // arrive once and are patched by `SMSG_FRIEND_STATUS`, which is also
        // where every refusal comes back. See [`crate::play::social`], which
        // carries the reason the two lists have no names in them.
        Opcode::SMSG_FRIEND_LIST => player::friend_list(ctx, pkt),
        Opcode::SMSG_IGNORE_LIST => player::ignore_list(ctx, pkt),
        Opcode::SMSG_FRIEND_STATUS => player::friend_status(ctx, pkt),
        Opcode::SMSG_WHO => player::who_results(ctx, pkt),

        // ---- and what they have been sent -------------------------------------
        // Five packets and one of them answers seven different verbs. See
        // [`crate::play::mail`], which carries the union in the header's sender
        // field and why *opening* a mailbox crosses no wire at all.
        Opcode::SMSG_MAIL_LIST_RESULT => player::mail_list(ctx, pkt),

        // ---- and what they hand to each other ---------------------------------
        // Two packets, and the second states *both* offers. See
        // [`crate::play::trade`].
        // ---- …and what the character may hold, and what a talent changes ----
        // Three packets with nothing else in common but that each is a
        // *statement* rather than an event: a class's whole proficiency mask, a
        // modifier bit's running total, and an enchant landing or fading.
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

        // ---- and where the character has been ---------------------------------
        // The only thing on the wire that says a place was discovered. See
        // [`crate::play::explored`] for the field it goes with.
        Opcode::SMSG_EXPLORATION_EXPERIENCE => player::discovered(ctx, pkt),

        // ---- what is on a body, and taking it -------------------------------
        // Five packets and the first of them is two: `SMSG_LOOT_RESPONSE` is
        // both the window and the refusal to open one, told apart by a loot
        // type of zero. See [`crate::play::loot`], which also carries why the money
        // is two packets and why the *release* is the server's to confirm.
        Opcode::SMSG_LOOT_RESPONSE => player::loot_response(ctx, pkt),
        Opcode::SMSG_LOOT_RELEASE_RESPONSE => player::loot_released(ctx, pkt),
        Opcode::SMSG_LOOT_REMOVED => player::loot_removed(ctx, pkt),
        Opcode::SMSG_LOOT_CLEAR_MONEY => player::loot_money_cleared(ctx),
        Opcode::SMSG_LOOT_MONEY_NOTIFY => player::loot_money_gained(ctx, pkt),

        // ---- …and, in a group, who gets it ----------------------------------
        // Four more, and none of them names the roll the way the interface
        // does: the wire says `(guid, item slot)` and the interface says a
        // `rollID` the client invents. See [`crate::play::lootroll`], which also
        // carries the rule with no packet behind it — the *end* of a roll is
        // what makes a blocked row clickable, and nothing on the wire says so.
        Opcode::SMSG_LOOT_START_ROLL => player::loot_roll_started(ctx, pkt),
        Opcode::SMSG_LOOT_ROLL => player::loot_roll_cast(ctx, pkt),
        Opcode::SMSG_LOOT_ROLL_WON => player::loot_roll_won(ctx, pkt),
        Opcode::SMSG_LOOT_ALL_PASSED => player::loot_roll_all_passed(ctx, pkt),

        // ---- the bars the server counts for us ------------------------------
        // The breath meter, the fatigue meter and the feign-death bar. Three
        // packets, no state and no clock of ours at all — see [`crate::play::timers`],
        // and note that the *pause* is the one the shipped interface cannot act
        // on, which is why vmangos resends a start instead.
        Opcode::SMSG_START_MIRROR_TIMER => player::mirror_timer_start(ctx, pkt),
        Opcode::SMSG_STOP_MIRROR_TIMER => player::mirror_timer_stop(ctx, pkt),
        Opcode::SMSG_PAUSE_MIRROR_TIMER => player::mirror_timer_pause(ctx, pkt),

        // ---- dying, and getting up again ------------------------------------
        // **Nothing here announces the death itself** — that is health reaching
        // zero, and it arrives inside an ordinary values block. These are the
        // four the client can only learn by asking or by being offered. See
        // [`crate::play::death`].
        Opcode::SMSG_CORPSE_RECLAIM_DELAY => player::corpse_reclaim_delay(ctx, pkt),
        Opcode::MSG_CORPSE_QUERY => player::corpse_located(ctx, pkt),
        Opcode::SMSG_RESURRECT_REQUEST => player::resurrect_offered(ctx, pkt),
        Opcode::SMSG_SPIRIT_HEALER_CONFIRM => player::spirit_healer_offered(ctx, pkt),

        // ---- leaving --------------------------------------------------------
        // Three of the four are bodiless; the server owns the twenty seconds.
        Opcode::SMSG_LOGOUT_RESPONSE => player::logout_response(ctx, pkt),
        Opcode::SMSG_LOGOUT_CANCEL_ACK => {
            player::logout_state(ctx, crate::play::logout::Logout::Cancelled)
        }
        Opcode::SMSG_LOGOUT_COMPLETE => player::logout_state(ctx, crate::play::logout::Logout::Complete),

        // ---- what things are called ---------------------------------------
        Opcode::SMSG_CREATURE_QUERY_RESPONSE => query::creature(ctx, pkt),
        Opcode::SMSG_NAME_QUERY_RESPONSE => query::name(ctx, pkt),
        Opcode::SMSG_GAMEOBJECT_QUERY_RESPONSE => query::gameobject(ctx, pkt),
        Opcode::SMSG_ITEM_QUERY_SINGLE_RESPONSE => query::item(ctx, pkt),

        // ---- what people say ----------------------------------------------
        Opcode::SMSG_MESSAGECHAT => chat::message(ctx, pkt),
        Opcode::SMSG_NOTIFICATION => chat::notification(ctx, pkt),
        Opcode::SMSG_CHANNEL_NOTIFY => chat::channel_notify(ctx, pkt),
        Opcode::SMSG_CHANNEL_LIST => chat::channel_list(ctx, pkt),
        Opcode::SMSG_TEXT_EMOTE => chat::text_emote(ctx, pkt),

        // ---- packets that must be answered --------------------------------
        Opcode::SMSG_PONG => acks::pong(ctx),
        Opcode::MSG_MOVE_TELEPORT_ACK => acks::teleport(ctx, pkt),
        Opcode::SMSG_NEW_WORLD => acks::new_world(ctx, pkt),
        // …and the map change that arrives with no teleport in front of it,
        // which is a login. Answered with nothing; it is here because it is the
        // only statement of which map the character is on that the server ever
        // makes. See [`player::login_verify_world`].
        Opcode::SMSG_LOGIN_VERIFY_WORLD => player::login_verify_world(ctx, pkt),
        Opcode::SMSG_MOVE_KNOCK_BACK => acks::knock_back(ctx, pkt),
        // …and the packet that says which body the keys are pointed at, which
        // is how a possess begins and ends. See [`acks::client_control`].
        Opcode::SMSG_CLIENT_CONTROL_UPDATE => acks::client_control(ctx, pkt),
        // …and the transfer that does *not* happen. Nothing else says an
        // instance portal was refused, so without this arm a full dungeon is
        // indistinguishable from a client that never sent `CMSG_AREATRIGGER`.
        Opcode::SMSG_TRANSFER_ABORTED => player::transfer_aborted(ctx, pkt),
        // …and the transfer that *is* happening, announced one packet ahead of
        // itself so that the loading screen is up before the old world is torn
        // down. Answered with nothing; it is here for the picture.
        Opcode::SMSG_TRANSFER_PENDING => player::transfer_pending(ctx, pkt),

        // ---- the ones matched by a predicate rather than a constant --------
        // A forced speed change, and a forced movement *flag* change — root,
        // water walking, hover, feather fall. Both are a family of opcodes
        // rather than one, which is why they are guards; both must be
        // acknowledged with the counter they arrived with, or the change stays
        // pending forever and then fires `OnFailedToAckChange`.
        // A swing the server refused: five opcodes, no body on any of them, and
        // the opcode itself is the message.
        op if crate::play::spells::AttackRefusal::of(op).is_some() => {
            player::attack_refused(ctx, crate::play::spells::AttackRefusal::of(op).expect("just matched"))
        }
        // Learned and unlearned, which differ in the *width* of their one field.
        op if player::is_spell_change(op).is_some() => {
            player::spell_change(ctx, pkt, player::is_spell_change(op).expect("just matched"))
        }
        op if movement::speed_change_slot(op).is_some() => acks::speed_change(ctx, pkt, op),
        // …and the *other two thirds* of `moveTypeToOpcode`: the same six speeds
        // about a unit we do not control, which is every other unit in the game.
        // Twelve opcodes, unacknowledged, and the only restatement of a speed
        // that exists after the movement block an object was created with.
        op if movement::broadcast_speed_slot(op).is_some() => {
            world::speed_broadcast(ctx, pkt, op)
        }
        op if movement::FlagChange::of(op).is_some() => acks::flag_change(ctx, pkt, op),
        // …and the *third* audience for the same subject: a movement flag about
        // a unit **no player is moving**, which is every creature in the world.
        // Twelve opcodes, a packed guid and no body, unacknowledged, and the
        // only restatement of a server-controlled unit's flags that exists
        // after its create block. See [`movement::SplineFlagChange`].
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
/// **A packet whose body will not parse is a length bug, and length bugs
/// surface late.** Every one of these used to be dropped in silence: a mis-read
/// `SMSG_MONSTER_MOVE` produced a frozen creature and no diagnostic at all,
/// where a mis-read movement broadcast produced a warning, for no reason other
/// than which call site happened to have one written. One channel now, and it
/// reaches the HUD through `SessionStatus::warnings`.
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
    /// **`SMSG_COMPRESSED_MOVES` bags seen, and the packets taken out of
    /// them.**
    ///
    /// Counted apart from [`Self::compressed`], which is the *update* stream's,
    /// because they answer different questions and one is a diagnosis. The
    /// server sends no bag at all until its own movement rate passes
    /// `CONFIG_UINT32_COMPRESSION_MOVEMENT_COUNT` — so a session with zero here
    /// has told you nothing, and a session with hundreds is one where **most of
    /// the movement in the world arrived inside this opcode**. Before it was
    /// handled that read as 428 unhandled packets and 35.7 KiB on the `F4` net
    /// tab and as creatures standing still everywhere else.
    ///
    /// The inner count is the one to compare against
    /// `traffic[SMSG_MONSTER_MOVE]`: they are the same packets and the ratio is
    /// how much of the world's movement is being bagged.
    pub bagged_moves: u32,
    pub bagged_packets: u32,
    pub creatures_resolved: u32,
    pub gameobjects_resolved: u32,
    pub players_resolved: u32,
    /// Item templates resolved — a player's equipment, which is the one thing
    /// the archives cannot answer for.
    pub items_resolved: u32,
    /// Queries answered out of the on-disk cache instead of being sent — see
    /// [`crate::play::wdb`]. Each one is a round trip the session did not
    /// make; the answer still went through this dispatch, so the `*_resolved`
    /// counters include them.
    pub cache_answered: u32,
    pub monster_moves: u32,
    /// `MSG_MOVE_*` broadcasts folded in — other players walking around.
    pub player_moves: u32,
    /// `SMSG_ATTACKERSTATEUPDATE` — melee swings, both ends of them.
    pub attacks: u32,
    /// Spell damage and heal logs. Beside [`Self::attacks`] and counted apart
    /// from it, because "the weapon swings arrive and the spells do not" is a
    /// state this client was in for a long time and is invisible in one total.
    pub spell_logs: u32,
    /// `SMSG_EMOTE` — one-shot emotes played by anyone in sight.
    pub emotes: u32,
    /// `SMSG_AI_REACTION` — creatures noticing and engaging.
    ///
    /// Worth its own counter for the same reason `speed_broadcasts` is: it is
    /// the *only* trace this packet leaves, since what it drives is a sound.
    /// Zero in a session where anything attacked you means the aggro barks are
    /// being dropped again, and no picture would show it.
    pub ai_reactions: u32,
    /// `SMSG_SPELL_START` and `SMSG_SPELL_GO` together.
    pub casts: u32,
    /// `SMSG_FORCE_*_SPEED_CHANGE` recorded and acknowledged — the six about a
    /// unit **we** control.
    pub speed_changes: u32,
    /// …and the twelve about one we do not: `MSG_MOVE_SET_*_SPEED` and
    /// `SMSG_SPLINE_SET_*`, recorded and deliberately *not* acknowledged.
    ///
    /// Counted apart because they are the answer to a different question. The
    /// forced changes are about one character and are rare; these are every
    /// mount, sprint, daze, enrage and flee within sight, and a zero here in a
    /// session where anybody mounted means the family is being dropped again.
    pub speed_broadcasts: u32,
    /// …and the twelve `SMSG_SPLINE_MOVE_*` about a unit no player is moving —
    /// root, water walking, feather fall, hover and walk/run mode.
    ///
    /// A third counter for a third audience, on the same argument the second
    /// one is here for. This family is the *only* restatement of a
    /// server-controlled unit's movement flags that exists after its create
    /// block, so a zero in a session where anything was rooted or told to walk
    /// means it is being dropped — see
    /// [`crate::state::movement::SplineFlagChange`].
    pub spline_flag_changes: u32,
    /// **Sounds and visuals the server asked for outright** — the three
    /// `SMSG_PLAY_*` sound opcodes and the two `SMSG_PLAY_SPELL_*` ones.
    ///
    /// Two counters because they fail differently. A dropped sound is silence
    /// and leaves no other trace at all; a dropped visual is a character who
    /// sits down to eat and does nothing. Both are families where **a zero is
    /// the only evidence**, which is this whole table's argument.
    pub pushed_sounds: u32,
    pub pushed_visuals: u32,
    /// **Intro cinematics offered and ended** — `SMSG_TRIGGER_CINEMATIC` in,
    /// `CMSG_COMPLETE_CINEMATIC` straight back out. See
    /// [`self::world::cinematic`].
    ///
    /// One per character, on its first login and never again, and the whole
    /// world hangs on the reply — so the useful reading is not the number but
    /// where it is: a 1 here on a session whose world is populated is the fix
    /// working, and a 1 on the unhandled list instead is the bug.
    pub cinematics: u32,
    /// **Weather stated** — `SMSG_WEATHER`, once per zone change and once per
    /// grade the server rolls. A zero after a zone change means the packet is
    /// not arriving, which for a zone with a `game_weather` row is a fault.
    pub weather: u32,
    /// **Items that arrived in a bag** — `SMSG_ITEM_PUSH_RESULT`. Counted here
    /// because the alternative reading of it is the inventory's own fields,
    /// which move whether or not this packet is read: a zero over a session
    /// that looted anything means the *event* is being dropped while the bag
    /// still fills, which is exactly the failure that has no other symptom.
    pub items_received: u32,
    /// **`CMSG_AREATRIGGER` sent** — the one packet this client volunteers.
    ///
    /// The instrument for a subject with no failure of its own: a portal that
    /// does nothing looks identical whether the client never noticed the
    /// trigger, noticed and latched, or sent it and the server declined. A zero
    /// here after walking through a dungeon entrance says the fault is on this
    /// side of the socket; a one says look at `SMSG_TRANSFER_ABORTED`.
    pub area_triggers: u32,
    /// **`CMSG_MOVE_SPLINE_DONE` sent** — a server-driven ride of this character
    /// begun, walked and handed back. A Charge.
    ///
    /// The instrument for the same kind of subject: nothing on the wire reports
    /// a ride that was ignored, and the symptom of ignoring one is not a wrong
    /// position but a session in which **every** subsequent movement packet is
    /// discarded — see [`movement::Mover::ride`]. A zero here after a charge
    /// says the packet was never ridden; a one says look at what came back.
    pub rides: u32,
    /// The forced *flag* changes and knockbacks, likewise.
    ///
    /// **An instrument for a failure that is otherwise silent by design.** Every
    /// one of these went into the unhandled bucket until now, and the visible
    /// consequence was a kick several seconds later naming a cheat the client
    /// never attempted. A count answers "did that path ever run?", which is the
    /// only question a session can be asked about it.
    pub flag_changes: u32,
    /// Chat lines and notifications folded in.
    pub chat: u32,
    /// How many spells `SMSG_INITIAL_SPELLS` named. **Zero is the failure this
    /// exists to show**: the packet is sent once in the login burst and nothing
    /// restates it, so a client that misread it has an empty action bar and no
    /// other symptom at all.
    pub spells_known: u32,
    /// …and how many action-bar slots were occupied.
    pub action_buttons: u32,
    /// `SMSG_CAST_RESULT` seen, accepted and refused together — the instrument
    /// for "did the server hear the cast?", which is otherwise unanswerable
    /// from the world state.
    pub cast_results: u32,
    /// The five `SMSG_ATTACKSWING_*` refusals. Nonzero is ordinary — swinging
    /// out of range is how every fight starts — and it is the *only* trace a
    /// refused swing leaves.
    pub attack_refusals: u32,
    /// Opcodes seen but not handled, with counts.
    pub other: BTreeMap<String, u32>,
    /// **Every opcode the dispatch saw**, handled or not, with its packet count
    /// and the bytes it cost.
    ///
    /// [`Self::other`] is the unhandled *subset* of this and is kept separately
    /// because it answers a different question — it is what the CLI's login
    /// summary prints, and what "this client is deaf to N opcodes" is counted
    /// off. This one is the traffic itself: which opcodes a session is actually
    /// made of, in what proportion, and what share of the bytes each is. A
    /// stream that is 90% `SMSG_MONSTER_MOVE` and a stream that is 90%
    /// `SMSG_UPDATE_OBJECT` describe two completely different bugs, and no
    /// counter here separated them before this one.
    ///
    /// **Keyed by raw opcode, not by name.** `Packet::name` is a `format!` and
    /// allocates; this is on the path every packet in the session takes, where
    /// `other` above is only reached by the handful that have no handler and
    /// can afford a `String`. See [`crate::socket::world::named`], which
    /// resolves them on the way out, five times a second.
    pub traffic: BTreeMap<u32, OpcodeFlow>,
    pub warnings: Vec<String>,
    /// **The map `SMSG_LOGIN_VERIFY_WORLD` named**, for a caller with no
    /// [`LocalState`] to write it into.
    ///
    /// `None` in every run that did not contain a login, which is all of them
    /// but the first. The session loop reads the map off `LocalState` and never
    /// looks here; this exists for `vale login`, which pumps an
    /// [`crate::state::objects::ObjectManager`] with no local state at all and
    /// was printing the character list's map — the one value in this whole
    /// subject that can be wrong. See
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
    /// Named as what it is rather than as "parse failed": the block layouts here
    /// are flag-gated with no length prefix, so the cause is almost always a
    /// field being read at the wrong offset or at the wrong width.
    fn unreadable(&mut self, pkt: &Packet) {
        self.warn(format!(
            "{} could not be read — a field is being taken at the wrong offset or width",
            pkt.name()
        ));
    }

    /// An opcode with no handler. **Data, not an error**: the server
    /// legitimately sends opcodes this client does not implement, and turning
    /// one into a failure would take the session down for no reason.
    fn unhandled(&mut self, pkt: &Packet) {
        *self.other.entry(pkt.name()).or_insert(0) += 1;
        // …and mark the traffic row, which `apply_packet` created optimistically
        // one line earlier. The two are one fact recorded twice on purpose: see
        // the field's own note.
        self.traffic.entry(pkt.code).or_default().handled = false;
    }

    /// Record one packet against its opcode's row, assuming for now that
    /// something will read it — [`Self::unhandled`] is what says otherwise.
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

    /// **A bag of movement packets, built exactly as vmangos builds one.**
    ///
    /// `MovementData::AddPacket` writes `u8(body + 2); u16(opcode); body` into a
    /// buffer, and `BuildPacket` prefixes the buffer's own length and deflates
    /// it — see `world::compressed_moves`, where the whole of why this opcode
    /// matters is written down.
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

    /// **The desync, as a test.** `WorldSession::SendMovementPacket` batches
    /// every movement packet into this one opcode once the rate passes
    /// `CONFIG_UINT32_COMPRESSION_MOVEMENT_COUNT` — so a client that drops it
    /// works against a quiet localhost server and loses every creature, every
    /// other player and every speed change on a busy realm. Measured at **428
    /// packets and 35.7 KiB in one session** before it was handled.
    ///
    /// The assertion is that the packets *inside* it reached the world, which
    /// is the only thing that matters: nothing about the bag itself is visible.
    /// **The intro cinematic is answered, and the answer needs no local
    /// player.**
    ///
    /// It arrives during the login burst, before `CMSG_SET_ACTIVE_MOVER` and
    /// before anything has a mover, so a handler that reached for `ctx.local`
    /// would drop exactly the packet the world depends on. See
    /// [`self::world::cinematic`] for what dropping it costs.
    #[test]
    fn the_intro_cinematic_is_ended_at_once() {
        let mut world = ObjectManager::new();
        let packet = Packet {
            code: Opcode::SMSG_TRIGGER_CINEMATIC.code(),
            // `ChrRaces.dbc`'s `CinematicSequence` — 81 is the human one.
            body: 81u32.to_le_bytes().to_vec(),
        };
        let (stats, replies) = apply(&mut world, None, &packet);
        assert_eq!(stats.cinematics, 1);
        assert!(stats.other.is_empty(), "still unhandled: {:?}", stats.other);
        assert_eq!(replies, vec![(Opcode::CMSG_COMPLETE_CINEMATIC, Vec::new())]);
    }

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
        // …and the *destinations* are the two the bag carried, so the bodies
        // were cut at the right boundaries rather than merely counted.
        assert_eq!(world.get(9).unwrap().spline.as_ref().unwrap().to()[0], 24.0);
        assert_eq!(world.get(11).unwrap().spline.as_ref().unwrap().to()[0], -18.0);

        // **Each one is an ordinary packet from the census's point of view**,
        // which is the whole reason this goes back through the one dispatch:
        // the traffic table has to show what the stream is really made of.
        let moves = stats.traffic.get(&Opcode::SMSG_MONSTER_MOVE.code());
        assert_eq!(moves.map(|f| f.count), Some(2));
        assert!(stats.traffic.contains_key(&Opcode::SMSG_COMPRESSED_MOVES.code()));
        assert!(stats.other.is_empty(), "something went unhandled: {:?}", stats.other);
        assert_eq!(stats.bagged_moves, 1);
        assert_eq!(stats.bagged_packets, 2);
        assert_eq!(stats.compressed, 0, "the update stream's counter is a different question");
    }

    /// **A damaged tail costs the rest of the bag and says so**, and the
    /// packets in front of it still land — the rule every reader in this crate
    /// follows, and the one that matters most here because there is no way to
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
        // …and the whole one still reads clean, so the warning above is about
        // the tear rather than about the shape.
        let mut world = ObjectManager::new();
        assert!(apply(&mut world, None, &good).0.warnings.is_empty());
    }

    /// **A bag inside a bag is refused rather than recursed into.** The server
    /// never produces one — only movement packets reach the compressor — and
    /// unbounded recursion off the wire is not a thing to leave open.
    #[test]
    fn a_nested_bag_is_refused() {
        let mut world = ObjectManager::new();
        let inner = bag(&[(Opcode::SMSG_MONSTER_MOVE, walk(9, 24.0))]);
        // Truncated only so it fits the u8 length; the guard is on the opcode
        // and never looks at the body, which is the point.
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

    /// **Every packet lands in the traffic table, and the ones nothing reads
    /// are marked.**
    ///
    /// The distinction is the whole instrument: an opcode arriving a thousand
    /// times with no handler is a subject this client is deaf to, and from the
    /// packet count alone it is indistinguishable from one that is being read
    /// perfectly. The bytes include the four-byte framed header, which no body
    /// length carries — so this and `WireCounts::bytes_in` agree per packet.
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
            // …and one with none. `SMSG_AUTH_CHALLENGE` belongs to the
            // handshake and never reaches this dispatch in a real session,
            // which is exactly why it is the honest stand-in for an opcode
            // nothing here reads.
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

        // `other` is the unhandled *subset*, and it must stay exactly that —
        // the CLI's login summary is written against it.
        assert_eq!(stats.other.len(), 1);
        assert_eq!(stats.other["SMSG_AUTH_CHALLENGE"], 1);
        assert_eq!(stats.packets, 3);
    }

    /// **A forced speed change is recorded on the unit it is about**, not only
    /// when it is about us.
    ///
    /// This is the bug the two-site dispatch had: the live loop intercepted
    /// these packets, acknowledged them and returned, so one about another
    /// player never reached the world at all. `Entity::speeds` is otherwise
    /// written only by an update block's movement section, so the client went on
    /// dead-reckoning a hasted player at their old run speed until the server
    /// happened to describe them again — and then corrected them, which arrives
    /// as a jump and reads as "mobs teleport around".
    #[test]
    fn a_speed_change_about_someone_else_still_reaches_the_world() {
        // Another player, known because they walked into view — which is the
        // only way this client hears about one.
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
        // …and it is still acknowledged, or the server leaves the change
        // pending forever.
        assert_eq!(
            replies.first().map(|(op, _)| *op),
            Some(Opcode::CMSG_FORCE_RUN_SPEED_CHANGE_ACK)
        );
        // Ours is untouched: the packet was about somebody else.
        assert_eq!(state.mover.speeds.run(), movement::Speeds::default().run());
    }

    /// **`SMSG_MONSTER_MOVE` may be about *us*, and then it is a ride.**
    ///
    /// The one packet in the movement family that carries no hint of who it is
    /// for beyond the guid: a Charge moves the caster through the same
    /// `MoveSpline` machinery a patrolling creature is walked with
    /// (`Spell::OnSpellLaunch` → `MoveCharge`), so the packet is identical and
    /// only the guid says it is ours. Applied to the world and nothing else, the
    /// local simulation walks on from where it was while the server has us at the
    /// target — and, worse, holds us as spline-pending for the rest of the
    /// session. See [`movement::Mover::ride`].
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

        // Somebody else's: the world hears it, the mover does not.
        let mut state = local();
        apply(&mut world, Some(&mut state), &charge(9));
        assert!(!state.mover.is_riding());

        let mut state = local();
        let (stats, replies) = apply(&mut world, Some(&mut state), &charge(42));
        assert_eq!(stats.monster_moves, 1);
        assert!(state.mover.is_riding(), "the player's own spline was ignored");
        // **Nothing is answered here.** The ack is owed when the ride *ends*,
        // with the position it ends at — see `Mover::take_finished_ride`.
        assert!(replies.is_empty(), "{replies:?}");
        // The relocation counter moves, which is what stops the session loop
        // reading the world's own copy of this move as a correction to resync to.
        assert_eq!(state.relocations, 1);

        for _ in 0..41 {
            state.mover.advance(0.025, None);
        }
        assert_eq!(state.mover.take_finished_ride(), Some(41));
        // …facing the unit it charged, which only the world could resolve.
        assert!(state.mover.position().orientation.abs() < 0.01);
    }

    /// **The observer's copy of that same change, which is the one the client
    /// had never read at all** — and the one that matters, because it is the
    /// only one that ever arrives about somebody else.
    ///
    /// `SMSG_FORCE_*_SPEED_CHANGE` goes to whoever *controls* the unit and to
    /// nobody else (`SendSpeedChangeToController` sends to `mover`), so the test
    /// above only ever fires for a unit we are moving. Everyone else is told on
    /// `MSG_MOVE_SET_*_SPEED`, which carries a whole movement block between the
    /// guid and the speed — parse it as an ordinary heartbeat and the position
    /// lands while the speed falls off the end.
    ///
    /// A player mounting at +60% is 11.2 y/s against the 7.0 the block they were
    /// created with said, so a client that misses this dead-reckons them 2.1
    /// yards short every half-second and snaps them forward on the heartbeat,
    /// twice a second, for as long as they are mounted — and does it again in
    /// the other direction when they get off.
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
        // **Not answered.** The ack belongs to the controller and we are not it;
        // sending one is `OnWrongAckData`, and three of those is a kick.
        assert!(replies.is_empty(), "acknowledged a change about someone else");
    }

    /// The other observer family: a **server-driven** unit, which is every
    /// creature in the game.
    ///
    /// `SendSpeedChangeToAll` uses `moveTypeToOpcode[mtype][0]` —
    /// `SMSG_SPLINE_SET_*` — and its body is the packed guid and the speed with
    /// **no movement block at all**, because a creature mid-path has no
    /// position worth stating. Every enrage, every daze, every fleeing mob and
    /// every aura that touches a speed comes this way.
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

    /// All twelve resolve, to the slot [`movement::Speeds`] holds and to the
    /// family whose body shape that opcode actually has.
    ///
    /// A transposition here is silent and plausible: writing a run speed into
    /// the swim slot leaves a runner reckoned correctly on land and wrongly the
    /// moment they enter water, which is a bug nobody would trace back to a
    /// table.
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
        // …and the controller's own family is *not* in it, or a forced change
        // would be recorded twice and never acknowledged once.
        assert_eq!(
            movement::broadcast_speed_slot(Opcode::SMSG_FORCE_RUN_SPEED_CHANGE),
            None
        );
    }

    /// **A root is answered, and the answer carries the flag.**
    ///
    /// Two separate rules, both of which end the session. Not answering leaves
    /// the change pending: `CheckPendingMovementChanges` fires
    /// `OnFailedToAckChange` after four seconds — `PendingAckDelay`, threshold
    /// 3, kick — and then applies the root anyway, after which every step is
    /// `CHEAT_TYPE_ROOT_MOVE`. Answering *without* `MOVEFLAG_ROOT` in the block
    /// is worse and faster: `HandleMoveRootAck` calls `KickPlayer()` outright.
    /// `Anticheat.log` on the reference server carries both, on this client's
    /// own characters.
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
        // …and the local simulation is actually rooted, not merely reported so.
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
    /// `Player::TeleportTo` raises `SetSemaphoreTeleportFar` *before* sending
    /// `SMSG_NEW_WORLD` and only this ack lowers it, so a client that does not
    /// answer is held on no map — nothing it sends is processed and nothing
    /// comes back. The CLI's snapshot pump has no `Mover` to resync and used to
    /// reach a dispatch that did not handle this packet at all; it now answers,
    /// which is the half that matters.
    ///
    /// The body is empty on purpose: `HandleMoveWorldportAckOpcode(WorldPacket&
    /// /*recvData*/)` does not read a byte of it.
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

        // With one: the map and the mover move too, and the relocation is
        // counted so the caller can re-baseline.
        let mut state = local();
        let mut world = ObjectManager::new();
        let (_, replies) = apply(&mut world, Some(&mut state), &pkt);
        assert_eq!(state.map_id, 1);
        assert_eq!(state.mover.position().x, -8000.0);
        assert_eq!(state.relocations, 1);
        assert_eq!(replies.len(), 1);
    }

    /// **A login that landed somewhere else says so, and it is not a
    /// teleport.**
    ///
    /// `SMSG_LOGIN_VERIFY_WORLD` is the first packet of the login burst and the
    /// only statement the server ever makes of which map a character is on.
    /// It is usually a restatement of the character-list row and worth
    /// nothing; the case it exists for is `Player::LoadFromDB` relocating a
    /// character out of an instance that has been reset, which it does with a
    /// bare `Relocate` and no teleport packet of any kind.
    ///
    /// Three things are pinned here. The map is taken from the packet rather
    /// than left alone; **no ack goes out**, because there is no semaphore up
    /// — that is `SMSG_NEW_WORLD`'s; and the **position is not adopted**,
    /// because the character's own create block states it a few packets later
    /// together with the speeds and flags that have to agree with it.
    #[test]
    fn a_login_that_landed_on_another_map_moves_the_map_and_answers_nothing() {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_le_bytes()); // Azeroth — the entrance
        for f in [-7178.4f32, -922.2, 166.1, 2.0] {
            body.extend_from_slice(&f.to_le_bytes());
        }
        let pkt = packet(Opcode::SMSG_LOGIN_VERIFY_WORLD, body);

        let mut state = local();
        // What the character list said: Blackrock Depths, which is where this
        // character logged out and is not where it has come back.
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

        // A caller with no `LocalState` at all — `vale login`'s snapshot
        // pump — still learns it, which is the whole of why the stat exists.
        let (stats, _) = apply(&mut ObjectManager::new(), None, &pkt);
        assert_eq!(stats.landed_on_map, Some(0));

        // …and a body that will not read leaves the row's map standing, which
        // is right in every case but the one above and is the conservative
        // side of it.
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

    /// **An unreadable body is reported, not dropped.**
    ///
    /// Every `parse_*` here returns `Option`, and each call site used to decide
    /// for itself whether to say so — the movement broadcast did, the monster
    /// move did not. So a length bug in the packet that moves every creature in
    /// the world produced a frozen creature and complete silence.
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

        // …and an opcode with no handler is *not* a warning. The server
        // legitimately sends packets this client does not implement, and
        // counting them beside the failures would bury the failures.
        //
        // This used to be `SMSG_EMOTE`, then `SMSG_WEATHER`, then
        // `SMSG_PLAYED_TIME`, each the module comment's own example of a packet
        // nobody could tell you the fate of. All three are handled now, so the
        // example has to be one that genuinely is not: `SMSG_GMTICKET_GETTICKET`
        // answers a GM ticket, and this client has no ticket window.
        let (stats, _) =
            apply(&mut world, None, &packet(Opcode::SMSG_GMTICKET_GETTICKET, Vec::new()));
        assert!(stats.warnings.is_empty());
        assert_eq!(stats.other.values().sum::<u32>(), 1);
    }

    /// **The login burst is the only time the spellbook is stated**, and the
    /// action bar with it. Both land in the world state and both bump the
    /// version a reader rebuilds the bar on.
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

    /// A refused swing has **no body at all** — the opcode is the message — and
    /// it is the only trace the refusal leaves. Dropping it is a player pressing
    /// attack at twenty yards and being told nothing.
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
        // …and taking them empties the queue, exactly as the chat one does.
        assert!(world.take_events().is_empty());
    }

    /// **The only thing the server ever says about a ranged volley is that it
    /// has stopped** — and, like the refusal above, the opcode is the whole
    /// message.
    ///
    /// Pinned because the asymmetry is the finding: there is no "it started"
    /// packet at all (starting one is an ordinary `CMSG_CAST_SPELL`), so a
    /// client that dropped this one could turn Auto Shot on and never
    /// legitimately turn it off — the button would keep flashing through a
    /// fight that had already ended, since `SpellCaster::InterruptSpell` routes
    /// the target dying, the range breaking and a wand-user moving through this
    /// same packet.
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

    /// **Both halves of a cast are broadcast about everybody, and both are news
    /// about our own casting when the caster is us.**
    ///
    /// `SMSG_SPELL_START` is what puts the bar up — it is the only thing that
    /// raises `SPELLCAST_START` in the 1.12 client, and this client no longer
    /// draws a cast at the press at all — and `SMSG_SPELL_GO` is the only thing
    /// on the wire that empties the next-swing queue. See
    /// [`crate::play::spells::PlayerEvent::CastStarted`].
    ///
    /// Three things are pinned and each is a one-line mistake: the caster is the
    /// **second** packed guid in the body (the first is the cast *item*), a
    /// stranger's cast raises nothing, and the two halves must not raise each
    /// other's event.
    #[test]
    fn both_halves_of_our_own_cast_reach_the_event_queue() {
        use crate::bytes::Writer;
        let mut world = ObjectManager::new();
        world.player_guid = Some(7);
        // `SMSG_SPELL_START` carries the timer and `SMSG_SPELL_GO` a hit list,
        // which is the one place the two bodies differ.
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

    /// **A burst of our own releases raises one event *each*, in order, and each
    /// one names its own spell** — which is the whole reason the cast bar is
    /// ended from this queue rather than from the entity's `last_spell`.
    ///
    /// `last_spell` is one field, so the second release below overwrites the
    /// first: a reader polling `casts_released` and then asking which spell it
    /// was gets 21084 and never 78. That is not a hypothetical shape —
    /// [`crate::state::objects::Entity::recent_spells`] exists because Charge is
    /// measured as two releases inside one 25 ms tick, and a seal proc on every
    /// swing is the same thing at melee speed. When it lost that race nothing
    /// ever cleared the bar again, so `in_progress` stayed true and every press
    /// for the rest of the session was refused with "another action is in
    /// progress".
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
        // Ours, then the one it triggered, with nothing looking in between.
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
        // The other half of this — that the *field* a poll would have read holds
        // only the second — is
        // `state::objects::tests::a_burst_of_releases_leaves_only_the_last_in_the_field`,
        // where the manager's own fixtures are.
    }

    /// **`SMSG_ATTACKSTART` is broadcast about everybody**, so the one thing
    /// the state may not do is adopt somebody else's fight as our own.
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

        // The stop is *packed* where the start is plain, and its victim may be
        // an empty packed guid — "stop attacking, nobody in particular".
        let mut stop = Writer::new();
        stop.packed_guid(7).packed_guid(0).u32(0);
        apply(&mut world, None, &packet(Opcode::SMSG_ATTACKSTOP, stop.buf));
        assert_eq!(world.attacking, None);
    }

    /// The warning list is capped, because a systematically mis-read field
    /// produces one per packet and the tenth says nothing the first did not.
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

    /// **The third audience: a movement flag about a unit no player is moving.**
    ///
    /// `SendMovementFlagChangeToAll` is the branch `Unit::SetRooted` takes for a
    /// creature, and under 1.9.4 its body is a **packed guid and nothing else** —
    /// so the opcode is the entire message and the flag comes from a table. It
    /// is not acknowledged: nobody is being asked to obey it.
    ///
    /// Dropping it is silent and permanent. A creature's flags arrive once, in
    /// its create block; no values update carries one and `SMSG_MONSTER_MOVE`
    /// carries a path with no flags at all. These twelve are the only
    /// restatement that exists.
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
        // **Not answered**, for the same reason the speed broadcast is not: the
        // server has no pending change to close and three wrong acks is a kick.
        assert!(replies.is_empty(), "acknowledged a change nobody asked us to obey");

        // …and it is the *only* statement, so the unroot has to come through the
        // same door and win.
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

    /// A near teleport about somebody else is an ordinary movement broadcast,
    /// and it is the only thing that moves a player who is standing still.
    ///
    /// `SendTeleportToObservers` fires twice per teleport, around the old
    /// position and the new, both carrying the destination — so it is applied
    /// twice and must be idempotent.
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

        // The second copy lands on the same place rather than doubling it.
        apply(&mut world, Some(&mut state), &pkt);
        assert_eq!(world.get(7).and_then(|e| e.position).map(|p| p.x), Some(300.0));
    }

    /// **The three packets whose whole content is a noise**, and the fact that
    /// two of them are laid out opposite ways round.
    ///
    /// Nothing else on the wire implies any of them: every scripted line, gate
    /// and zone-wide announcement is `WorldObject::PlayDirectSound` or one of
    /// its two siblings, or it is silent. They go on the player queue rather
    /// than into the world because there is no state to fold — two arrivals are
    /// two noises.
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

        // …and the placed one, whose body is sound-then-guid where the two
        // spell visuals below are guid-then-sound.
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

    /// **A `SpellVisualKit` pushed at a unit**, which is what eating looks like.
    ///
    /// vmangos sends kit 406 (food) or 438 (drink) on every regeneration tick a
    /// character spends sitting with either, and both read `animID 61`,
    /// `EmoteEat`. Nothing else on the wire says a character is eating at all.
    ///
    /// Recorded on the entity rather than pushed on the queue — it is about a
    /// unit and it stays true for as long as its models are up — and **on two
    /// counters**, because the visual is about what a unit did and the impact
    /// about what was done to it.
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

        // **A guid this client has never been sent conjures nothing**, which is
        // the reference's own behaviour — vmangos' comment on the sibling packet
        // says the 1.12 client ignores one for a unit it has not loaded.
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
