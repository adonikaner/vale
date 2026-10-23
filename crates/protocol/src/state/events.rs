//! The queue of one-shot events the object manager carries for the interface.
//!
//! [`PlayerEvent`] is what the handlers push and the game layer drains. It lives
//! in `state/` because [`super::objects::ObjectManager`] owns the queue; the
//! values it carries are the parsed packets of `play/`.

use crate::play::spells::{AttackRefusal, EquipRequirement, LevelUp, SpellModifier};

/// Something the server said about what this character just tried to do.
///
/// The interface drains a queue of these, in the same way as
/// `ObjectManager::take_chat`: each is an event to act on once, not state that
/// can be read again. A failed cast, a refused swing, a started cooldown:
/// none of them leaves any other trace. The swing counters rely on the same
/// reasoning.
///
/// Not `Copy` and not `Eq`. Every variant was a few numbers until
/// `SMSG_RESURRECT_REQUEST` added the caster's name and `MSG_CORPSE_QUERY`
/// added a position with floats. Both are real parts of their packets: the
/// name is what the game's `RESURRECT_REQUEST` popup shows, and no other
/// packet carries it for a creature caster, so the derives were dropped
/// rather than those fields.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    /// `CMSG_CAST_SPELL` was accepted. The cast itself arrives as
    /// `SMSG_SPELL_START`/`SMSG_SPELL_GO` like anybody else's.
    CastAccepted { spell_id: u32 },
    /// `CMSG_CAST_SPELL` was refused. `reason` is an index into
    /// [`CAST_FAILURE_KEYS`](crate::play::spells::CAST_FAILURE_KEYS).
    ///
    /// `requirement` is the argument to the message's own `%s` for the three
    /// equipped-item reasons and `None` for every other. See
    /// [`EquipRequirement`](crate::play::spells::EquipRequirement).
    CastFailed {
        spell_id: u32,
        reason: u8,
        requirement: Option<EquipRequirement>,
    },
    /// A cast in progress was interrupted. It is the player's cast if the
    /// guid is the player's.
    CastInterrupted { guid: u64, spell_id: u32 },
    /// `SMSG_SPELL_START` naming the player as the caster: the server has accepted
    /// the cast, and this shows the cast bar.
    ///
    /// vmangos states this in its comment `// will show cast bar` above the
    /// `SendSpellStart()` in `Spell::prepare`. The 1.12.1 client agrees: it
    /// raises `SPELLCAST_START` only when this packet arrives, and nothing on the
    /// key-press path raises it.
    ///
    /// Also sent for an instant cast, with `cast_time_ms` of zero: `prepare`
    /// sends it for every non-triggered spell whatever its cast time, and the
    /// `SMSG_SPELL_GO` follows immediately. This packet therefore also starts the
    /// cast animation, for the player as for any other unit.
    CastStarted { spell_id: u32, cast_time_ms: u32 },
    /// `SMSG_SPELL_GO` naming the player as the caster: one of the player's
    /// spells was released.
    ///
    /// Not the same event as [`Self::CastAccepted`], which is `SMSG_CAST_RESULT`
    /// and arrives when the server accepts the request. For nearly every spell
    /// the two are one cast time apart and nothing needs the second. For a
    /// next-swing ability the gap is large: it is accepted at the key press and
    /// released when the weapon hits, possibly seconds later. This packet is the
    /// only one that says the queued swing has been used.
    ///
    /// The world already reads the same packet for every unit's animations
    /// (`ObjectManager::apply_cast`). This variant is the caster-filtered event on
    /// the queue the interface reads, which is a separate reader with a separate
    /// need; [`Self::ChannelStart`] describes the same split.
    CastReleased { spell_id: u32 },
    /// `SMSG_SPELL_DELAYED`: the player's cast was pushed back by this many
    /// milliseconds.
    ///
    /// The only packet that says a cast will take longer than first stated. The
    /// interface has its own event for it: `CastingBarFrame_OnLoad` registers
    /// `SPELLCAST_DELAYED`, and its handler moves both ends of the bar by
    /// `arg1 / 1000`. Without it, a pushed-back cast's bar fills and stays full
    /// while the server is still casting, which was reported as "stuck casting".
    ///
    /// Only the caster receives this packet, so the variant has no guid: the
    /// handler checks the guid in the body, and the event reaching the interface
    /// is always about the player. See [`parse_spell_delayed`](crate::play::spells::parse_spell_delayed).
    CastDelayed { delay_ms: u32 },
    /// `MSG_CHANNEL_START`: one of the player's channelled spells has started,
    /// and how long it lasts.
    ///
    /// Kept separate from `CastAccepted` because a channel is the one cast whose
    /// length arrives in a later packet: `SMSG_SPELL_GO` arrives at once and says
    /// nothing about a bar, so the interface has nothing to show until this
    /// arrives. The world's copy drives the held pose
    /// ([`crate::state::objects::ObjectManager::apply_channel_start`]). This
    /// variant is the same information on the queue the interface reads, which
    /// is a separate reader with a separate need.
    ChannelStart { spell_id: u32, duration_ms: u32 },
    /// `MSG_CHANNEL_UPDATE`: the time left on the channel. Zero means it ended,
    /// which is what an interrupted or completed channel sends.
    ChannelUpdate { remaining_ms: u32 },
    /// `CMSG_ATTACKSWING` was refused; the swing did not start.
    AttackRefused(AttackRefusal),
    /// `SMSG_CANCEL_AUTO_REPEAT`: the ranged auto-repeat has stopped.
    ///
    /// The body is empty; the opcode is the message. It is the only packet the
    /// server sends about an auto-repeat. There is no start packet, because
    /// starting one is an ordinary `CMSG_CAST_SPELL` this client sent itself.
    /// Every end arrives here.
    ///
    /// It is sent for more than the player's own cancel:
    /// `SpellCaster::InterruptSpell` sends every stop through
    /// `Player::SendAutoRepeatCancel`, including the target dying, moving out of
    /// range, a `CheckCast` that stops passing, and, for a wand (category 351),
    /// moving at all. A client that cleared its state only on its own key press
    /// would keep the button flashing after the fight had ended.
    ///
    /// On the queue rather than in [`crate::socket::session::SessionStatus`]
    /// because it is an edge: it changes state this client set locally, and two
    /// arrivals mean two stops.
    AutoRepeatCancelled,
    /// A sound the server chose to play: `SMSG_PLAY_SOUND`, `SMSG_PLAY_MUSIC` and
    /// `SMSG_PLAY_OBJECT_SOUND`, the only packets that say so. See
    /// [`crate::play::sound`].
    ///
    /// On this queue rather than in [`crate::socket::session::SessionStatus`]
    /// because there is no state to hold: two arrivals are two sounds, and a
    /// status field would merge them into one. [`Self::AutoRepeatCancelled`]
    /// uses the same reasoning.
    ///
    /// Not stored on an entity, unlike the two spell visuals, although one of the
    /// three opcodes names an object. A sound is played at this client, and the
    /// server chose the listeners by choosing whom to send the packet to. A
    /// visual is a property of a unit that lasts as long as its models are
    /// loaded. The object's guid is used only as a position, and is carried as
    /// one.
    PlaySound(crate::play::sound::Cue),
    /// An item arrived in a bag: `SMSG_ITEM_PUSH_RESULT`, the only packet that
    /// says so, whatever the source. See [`crate::play::items::ItemPush`].
    ///
    /// On this queue rather than in [`crate::socket::session::SessionStatus`]
    /// for the reason every event here is: the inventory state already arrives as
    /// update fields, and this packet adds the event, for example that a stack
    /// grew by three because of loot rather than a purchase.
    ///
    /// Carries the guid rather than being filtered to the player, because for
    /// loot it is sent to the whole group and the reader needs to know whose bag
    /// received the item.
    ItemReceived(crate::play::items::ItemPush),
    /// A timer bar the server counts down for the player has started or been
    /// restated: the breath bar and its two siblings. See [`crate::play::timers`].
    ///
    /// On this queue rather than in [`crate::socket::session::SessionStatus`]
    /// for the same reason as the cancel above: the interface answers with
    /// `MirrorTimer_Show`, which takes a frame, and two arrivals are two
    /// statements about the bar even when every field matches. Storing it as
    /// state would also lose the packet's main use: the server sends a start
    /// again to mean "paused", because the FrameXML pause handler cannot work.
    MirrorTimerStarted(crate::play::timers::MirrorTimerStart),
    /// The same bar hidden: `SMSG_STOP_MIRROR_TIMER`, one `u32`.
    MirrorTimerStopped { timer: crate::play::timers::MirrorTimer },
    /// The same bar frozen at its current value. The game's FrameXML handler for
    /// this event cannot work (see [`crate::play::timers`]); the packet is read
    /// anyway so that the event matches the wire.
    MirrorTimerPaused {
        timer: crate::play::timers::MirrorTimer,
        paused: bool,
    },
    /// The server started or refreshed a cooldown: milliseconds from now. This is
    /// not the ordinary path; see [`parse_spell_cooldowns`](crate::play::spells::parse_spell_cooldowns).
    CooldownStarted { spell_id: u32, ms: u32 },
    /// A parked cooldown was released (`SPELL_ATTR_COOLDOWN_ON_EVENT`).
    CooldownReleased { spell_id: u32 },
    /// `SMSG_ITEM_COOLDOWN`: an item just equipped may not be used for
    /// [`EQUIP_COOLDOWN_MS`](crate::play::spells::EQUIP_COOLDOWN_MS). See [`parse_item_cooldown`](crate::play::spells::parse_item_cooldown).
    ItemCooldown { item: u64, spell_id: u32 },
    /// `SMSG_CLEAR_COOLDOWN`: this cooldown has ended, however long it had left.
    /// See [`parse_clear_cooldown`](crate::play::spells::parse_clear_cooldown). This is a removal, whereas
    /// [`Self::CooldownReleased`] is a start; the two names are similar and mean
    /// opposite things.
    CooldownCleared { spell_id: u32 },
    /// The pet refused an order: `SMSG_PET_ACTION_FEEDBACK`, one byte that
    /// selects which message to show. See [`crate::play::pet::feedback`].
    ///
    /// On this queue rather than in the session status for the reason every
    /// event here is: two refusals of the same order are two messages, and a
    /// field holding the last one would show it once.
    PetFeedback(u8),
    /// The pet refused a cast, which is the pet's own `SMSG_CAST_RESULT`.
    /// `reason` indexes the same 146-entry table.
    PetCastFailed { spell_id: u32, reason: u8 },
    /// `SMSG_PET_TAME_FAILURE` — a `PetTameFailureReason`.
    PetTameFailure(u8),
    /// `SMSG_PET_BROKEN` — an empty body. "Your pet has run away", which is a
    /// hunter pet whose loyalty ran out.
    PetBroken,
    /// `SMSG_PET_NAME_INVALID` — an empty body, shown as one message, like
    /// `SMSG_PET_BROKEN`.
    PetNameInvalid,
    /// `SMSG_PET_UNLEARN_CONFIRM` — the guid, and what resetting it costs.
    PetUnlearnConfirm { pet: u64, cost: u32 },
    /// `SMSG_PET_ACTION_SOUND` — the pet makes a sound. `talk` is a
    /// [`crate::play::pet::pet_talk`] selector, not a sound entry.
    PetTalk { pet: u64, talk: u32 },
    /// `SMSG_PET_DISMISS_SOUND` — a model-data id and a place; the pet itself
    /// is already gone. See [`crate::play::pet::PetDismissSound`].
    PetDismissSound(crate::play::pet::PetDismissSound),
    /// The spellbook changed after login.
    SpellLearned(u32),
    SpellRemoved(u32),
    /// A spell rank was replaced by a higher one: `SMSG_SUPERCEDED_SPELL`, the one
    /// spellbook change that also changes the action bar.
    ///
    /// `slots` lists the action bar slots that held the old id and now hold the
    /// new one, already updated in [`crate::state::objects::ObjectManager`]'s
    /// copy. It is carried rather than recomputed because the bar may hold the
    /// same spell twice and may already have held `new` in another slot, so the
    /// slots that now hold `new` are not the same as the slots this packet
    /// changed.
    ///
    /// The reader must send the server a `CMSG_SET_ACTION_BUTTON` per slot: the
    /// bar is client state that the server only stores, so a replacement that is
    /// not sent back is undone at the next login. See
    /// [`set_action_button_body`](crate::play::spells::set_action_button_body).
    SpellSuperceded { old: u32, new: u32, slots: Vec<u8> },
    /// The player gained a level: `SMSG_LEVELUP_INFO`, the only packet that says
    /// so. A change in `UNIT_FIELD_LEVEL` does not mean the same thing: it changes
    /// at every login and for every creature that comes into view.
    LevelUp(LevelUp),
    /// The server declined to transfer the player: `SMSG_TRANSFER_ABORTED`, the
    /// only packet that says an instance portal was refused deliberately.
    ///
    /// Carries the raw byte rather than a decoded reason, because three of the
    /// codes show nothing and only the reader decides whether to show a message.
    /// [`crate::play::areatrigger::TransferAbort::from_code`] decodes it, and
    /// `None` from it means "show nothing", which is the 1.12.1 client's
    /// behaviour rather than an unhandled case.
    ///
    /// On this queue for the same reason as the logout states below: it is an
    /// edge. Two refusals at two portals are two events with identical fields.
    TransferAborted { reason: u8 },
    /// The server refused a teleport trigger with a line of text:
    /// `SMSG_AREA_TRIGGER_MESSAGE`. See
    /// [`crate::play::areatrigger::parse_area_trigger_message`]. An edge for
    /// the same reason as [`Self::TransferAborted`]: walking into the same
    /// trigger twice is two refusals with the same text.
    AreaTriggerMessage { text: String },
    /// The server is about to transfer the player to another map:
    /// `SMSG_TRANSFER_PENDING`, whose purpose is to show the loading screen just
    /// before the old world is unloaded.
    ///
    /// The counterpart of [`Self::TransferAborted`] for a transfer that goes
    /// ahead, and an edge for the same reason: two teleports to the same map are
    /// two events. A client that detected the change by watching `map_id` would
    /// detect it only after `SMSG_NEW_WORLD`, which is after the gap the loading
    /// screen exists to cover. See
    /// [`crate::state::movement::parse_transfer_pending`].
    TransferPending { map_id: u32 },
    /// What the server said about logging out. See [`crate::play::logout`], which
    /// owns the four packets and explains why the client does not keep the timer.
    ///
    /// On this queue rather than in [`crate::socket::session::SessionStatus`]
    /// because each of the four is an edge: `PLAYER_CAMPING` is raised once when
    /// the request is accepted and `LOGOUT_CANCEL` once when it is cancelled, and
    /// a polled state cannot distinguish one arrival from two.
    Logout(crate::play::logout::Logout),
    /// How long before the corpse can be reclaimed:
    /// `SMSG_CORPSE_RECLAIM_DELAY`, in milliseconds, sent once at release and
    /// never restated. See [`crate::play::death`].
    CorpseReclaimDelay { ms: u32 },
    /// Where the corpse is, or `None` when there is none: the reply to this
    /// client's `MSG_CORPSE_QUERY`. It is on the queue rather than in the
    /// status because it answers a request: a second query with the same answer
    /// is a second answer.
    CorpseLocated(Option<crate::play::death::CorpseLocation>),
    /// A resurrection was offered to the player: `SMSG_RESURRECT_REQUEST`.
    ResurrectOffered(crate::play::death::ResurrectOffer),
    /// The spirit healer's form of the same offer, which costs 25% durability
    /// and causes resurrection sickness: `SMSG_SPIRIT_HEALER_CONFIRM`. The
    /// body is the healer's guid and nothing else.
    SpiritHealerOffered { healer: u64 },
    /// An item action was refused: `SMSG_INVENTORY_CHANGE_FAILURE`, the answer
    /// shared by every right-click, equip and swap. See
    /// [`crate::play::items::InventoryFailure`], which carries the reason and the
    /// one number some reasons take.
    ///
    /// On this queue rather than in the world, because it is an edge about an
    /// action the player just took and it changes no state: the item stayed where
    /// it was.
    InventoryFailed(crate::play::items::InventoryFailure),
    /// A loot window opened: `SMSG_LOOT_RESPONSE`, which is also how the server
    /// refuses to open one. See [`crate::play::loot`], which holds the byte that
    /// distinguishes the two.
    ///
    /// On this queue rather than in the world because the whole loot family is
    /// edges: the interface answers with `ShowUIPanel(LootFrame)`, and a second
    /// arrival about the same corpse opens a second window rather than repeating
    /// a value.
    LootOpened(crate::play::loot::Loot),
    /// One row was removed: `SMSG_LOOT_REMOVED`, by the server's index. It arrives
    /// for a row another player took as well as for one the player took.
    LootRemoved { index: u8 },
    /// The coins were removed from the window: `SMSG_LOOT_CLEAR_MONEY`, no body.
    /// Not the same event as [`Self::LootMoneyGained`]; treating them as one
    /// leaves a coin row on every other group member's screen.
    LootMoneyCleared,
    /// The player's share of the coins, in copper: `SMSG_LOOT_MONEY_NOTIFY`.
    LootMoneyGained { copper: u32 },
    /// The server confirms the loot window is closed:
    /// `SMSG_LOOT_RELEASE_RESPONSE`. The client's own `CMSG_LOOT_RELEASE` does not
    /// close it; see [`crate::play::loot`].
    LootClosed { guid: u64 },
    /// A group roll has started on one row: `SMSG_LOOT_START_ROLL`. The four
    /// variants below are the rest of that family. See [`crate::play::lootroll`],
    /// which covers the subject, including why the id the interface uses is not
    /// on the wire.
    ///
    /// On this queue rather than in the world for the same reason as the loot
    /// window: a roll is an edge with a timer, and a second start naming the same
    /// `(guid, slot)` is a second roll on a respawned corpse rather than a
    /// repeated value.
    LootRollStarted(crate::play::lootroll::RollStart),
    /// `SMSG_LOOT_ROLL` — a player's choice, or the result of a player's
    /// roll. Sent to everyone eligible, so it also arrives for the player's own
    /// choice.
    LootRollCast(crate::play::lootroll::RollCast),
    /// `SMSG_LOOT_ROLL_WON` — the roll was won. The item is already in the
    /// winner's bags.
    LootRollWon(crate::play::lootroll::RollWon),
    /// `SMSG_LOOT_ALL_PASSED`: the one outcome that leaves the item on the
    /// corpse, and the only statement that its row can be clicked again.
    LootRollAllPassed(crate::play::lootroll::RollAllPassed),

    // --- quests ---
    //
    // See [`crate::play::quest`], which owns the wire. Every one of these is an edge
    // about a conversation the player is having: a second arrival with
    // identical fields is a second page of dialogue, not a value that happens
    // to match, so none of them can be read as state.

    /// The quest marker over a quest giver: `SMSG_QUESTGIVER_STATUS`, one guid
    /// and one of eight values.
    ///
    /// The one member of this family that is also state (the marker stays until
    /// it changes). It arrives as an edge and is stored in the world; see
    /// [`crate::state::objects::ObjectManager::quest_status`].
    QuestStatus {
        guid: u64,
        status: crate::play::quest::DialogStatus,
    },
    /// `SMSG_QUESTGIVER_QUEST_LIST`: the greeting and the quests on offer.
    ///
    /// The four page variants are boxed and the others are not. A `QuestTemplate`
    /// is 384 bytes of strings and arrays, where an `AttackRefused` is one byte.
    /// Unboxed, every value on this queue (one per packet the session reads)
    /// would be as large as the largest variant, which is also the least
    /// frequent.
    QuestGreeting(Box<crate::play::quest::QuestGreeting>),
    /// `SMSG_QUESTGIVER_QUEST_DETAILS` — the page shown before the quest is
    /// accepted.
    QuestDetails(Box<crate::play::quest::QuestDetails>),
    /// `SMSG_QUESTGIVER_OFFER_REWARD`: the page shown when the quest is complete.
    QuestReward(Box<crate::play::quest::QuestReward>),
    /// `SMSG_QUESTGIVER_REQUEST_ITEMS`: the page shown when it is not. Not sent
    /// for every unfinished quest; see [`crate::play::quest::QuestProgress`].
    QuestProgress(Box<crate::play::quest::QuestProgress>),
    /// `SMSG_QUESTGIVER_QUEST_COMPLETE` — the rewards given for handing the
    /// quest in.
    QuestComplete(crate::play::quest::QuestComplete),
    /// `SMSG_QUEST_QUERY_RESPONSE`: the quest's definition. Cached per id, in the
    /// same way as an item template.
    QuestTemplate(Box<crate::play::quest::QuestTemplate>),
    /// `SMSG_QUESTUPDATE_ADD_KILL` — one more kill counted toward an
    /// objective.
    QuestKill(crate::play::quest::QuestKill),
    /// `SMSG_QUESTUPDATE_ADD_ITEM`: an item objective changed, as
    /// `(entry, added)` plus the bag count when it arrived. See
    /// [`crate::play::quest::parse_quest_item`]: the second word is an increment,
    /// and the count on screen is computed by the client from its bags.
    ///
    /// `have` is carried rather than looked up by the reader, because it is
    /// correct only on the session thread. See
    /// [`crate::socket::handler::player::quest_item`] for the reasoning.
    QuestItem { entry: u32, added: u32, have: u32 },
    /// `SMSG_QUESTUPDATE_COMPLETE`: every objective is done. The quest log's
    /// state bit also says so; this event plays the sound and writes the chat
    /// line.
    QuestObjectivesDone { quest_id: u32 },
    /// `SMSG_QUESTUPDATE_FAILED` and its timed-out counterpart; `timed_out`
    /// says which.
    QuestFailed { quest_id: u32, timed_out: bool },
    /// `SMSG_QUESTGIVER_QUEST_INVALID`: a reason, not a quest id.
    /// `SendCanTakeQuestResponse` writes the refusal code where every other
    /// packet in this family writes an id.
    QuestRefused { reason: u32 },

    // --- talking to an NPC ---
    //
    // See [`crate::play::gossip`]. The same edge argument as the quest family's: a
    // second identical menu is a second page of conversation.

    /// `SMSG_GOSSIP_MESSAGE`: the menu. Its text needs one more request.
    GossipShow(Box<crate::play::gossip::GossipMenu>),
    /// `SMSG_GAMEOBJECT_PAGETEXT`: this object has pages. The packet says nothing
    /// else; the page id is in the template the client already holds. See
    /// [`crate::play::pagetext`].
    GameObjectPageText { guid: u64 },
    /// `SMSG_PAGE_TEXT_QUERY_RESPONSE` — one page, and the id of the next.
    /// The server answers the whole chain in a burst, so these arrive several
    /// at a time and each names its successor.
    PageText(crate::play::pagetext::Page),
    /// `SMSG_GOSSIP_POI`: a place marked on the world map. The client keeps
    /// exactly one. See [`crate::play::gossip::parse_gossip_poi`].
    GossipPoi {
        flags: u32,
        position: (f32, f32),
        icon: u32,
        data: u32,
        name: String,
    },
    /// `SMSG_GOSSIP_COMPLETE` — the server closed the window. No body.
    GossipClosed,
    /// `SMSG_NPC_TEXT_UPDATE` — the text for a menu's text id.
    NpcText { text_id: u32, text: String },

    /// `SMSG_BINDER_CONFIRM`: an innkeeper asks whether to set the player's home.
    ///
    /// The guid is the innkeeper's and must be kept: the answer is
    /// `CMSG_BINDER_ACTIVATE` naming the same guid, which
    /// `HandleBinderActivateOpcode` looks up with `GetNPCIfCanInteractWith(…,
    /// UNIT_NPC_FLAG_INNKEEPER)`. It drops the request without a reply if the
    /// guid is not an innkeeper in range.
    ///
    /// Nothing is bound when this arrives. vmangos closes the gossip window
    /// before sending it, so a client that drops this packet shows the window
    /// closing and sets no home. See [`crate::play::bindpoint`].
    BinderConfirm { guid: u64 },
    /// `SMSG_PLAYERBOUND`: the bind succeeded, at this area.
    /// `SMSG_BINDPOINTUPDATE` carries the same area id with the position; this
    /// packet says which innkeeper set it.
    PlayerBound { guid: u64, area_id: u32 },
    /// `SMSG_DUEL_REQUESTED` — a duel was requested, by the player or of the
    /// player. Comparing the initiator guid with the player's own tells which;
    /// see [`crate::play::duel`].
    DuelRequested { arbiter: u64, initiator: u64 },
    /// `SMSG_DUEL_COUNTDOWN` — the duel was accepted and starts in this many
    /// milliseconds.
    DuelCountdown { ms: u32 },
    /// `SMSG_DUEL_OUTOFBOUNDS` / `SMSG_DUEL_INBOUNDS` — the player left the
    /// duel flag's area, or returned to it. `true` is out.
    DuelBounds { out: bool },
    /// `SMSG_DUEL_COMPLETE` — the duel ended. `started` is false for a duel
    /// declined or abandoned before the countdown ended.
    DuelComplete { started: bool },
    /// `SMSG_DUEL_WINNER` — who won, broadcast to everyone nearby.
    DuelWinner(crate::play::duel::DuelWinner),
    /// `SMSG_SUMMON_REQUEST` — another player asks to summon the player. See
    /// [`crate::play::summon`].
    SummonRequest(crate::play::summon::SummonRequest),
    /// `SMSG_PLAYED_TIME` — the answer to `/played`, in seconds.
    PlayedTime { total: u32, level: u32 },
    /// `MSG_INSPECT_HONOR_STATS` — the honor tab of the player being
    /// inspected. See [`crate::play::inspect`].
    InspectHonor(crate::play::inspect::InspectHonor),
    /// `SMSG_FISH_NOT_HOOKED` / `SMSG_FISH_ESCAPED` — the bobber was clicked
    /// with nothing on it, or too late. `true` is escaped. Both are bodiless and
    /// each is one line of the message table.
    Fish { escaped: bool },
    /// `SMSG_LIST_INVENTORY` — the vendor window.
    VendorShow(Box<crate::play::gossip::VendorList>),
    /// `SMSG_BUY_ITEM` — a purchase succeeded; the slot's stock changed.
    VendorSold {
        guid: u64,
        slot: u32,
        left: Option<u32>,
    },
    /// `SMSG_BUY_FAILED` — a purchase was refused, with the reason.
    BuyFailed {
        entry: u32,
        reason: Option<crate::play::gossip::BuyFailure>,
    },
    /// `SMSG_SELL_ITEM`: a sale was refused. A successful sale has no packet: the
    /// money and the item's removal arrive as ordinary update fields.
    SellFailed {
        item: u64,
        reason: Option<crate::play::gossip::SellFailure>,
    },
    /// `SMSG_TRAINER_LIST` — what an NPC will teach. See [`crate::play::trainer`].
    TrainerShow(Box<crate::play::trainer::TrainerList>),
    /// `SMSG_TRAINER_BUY_SUCCEEDED`: a service was learned. The spell itself
    /// arrives separately as an ordinary `SMSG_LEARNED_SPELL`, so this packet only
    /// says which row to recolour.
    TrainerBought { spell: u32 },
    /// `SMSG_TRAINER_BUY_FAILED` — a service was not learned, with the
    /// reason.
    TrainerBuyFailed {
        spell: u32,
        reason: Option<crate::play::trainer::TrainFailure>,
    },

    // --- the stable ---
    //
    // See [`crate::play::stable`]. Two packets. The second answers all four
    // stable requests.

    /// `MSG_LIST_STABLED_PETS` — the whole stable window. Boxed for the same
    /// reason as [`Self::TrainerShow`]: three named pets is the largest variant
    /// in this enum by a wide margin, and unboxed it would set the size of
    /// every value of the enum.
    StableList(Box<crate::play::stable::StableList>),
    /// `SMSG_STABLE_RESULT` — one byte, kept raw beside its reading so an
    /// unrecognised code is data rather than a dropped packet.
    StableResult {
        byte: u8,
        result: Option<crate::play::stable::StableResult>,
    },

    // --- the bank ---
    //
    // See [`crate::play::bank`]. Two packets: a guid and a refusal. The
    // bank's contents arrive as update fields.

    /// `SMSG_SHOW_BANK` — the window opens, at this banker.
    BankShow(u64),
    /// `SMSG_BUY_BANK_SLOT_RESULT` — a slot purchase refused. Kept raw beside
    /// its reading, like the stable's byte.
    BankSlotResult {
        code: u32,
        result: Option<crate::play::bank::BankSlotResult>,
    },

    // --- the party ---
    //
    // See [`crate::play::group`]. Five of the six are edges rather than state: an
    // invitation is a popup, a decline is a chat line, a refusal is a message.
    // Only [`Self::GroupList`] is the roster, and it arrives complete every time
    // anything about the group changes.

    /// `SMSG_GROUP_INVITE` — another player invites the player to a group.
    /// The body is the inviter's name and nothing else; the popup shows it.
    GroupInvite { name: String },
    /// `SMSG_GROUP_DECLINE` — a player this player invited declined. Only the
    /// inviter is told.
    GroupDecline { name: String },
    /// `SMSG_GROUP_LIST`: the whole roster, everybody except the player. Boxed
    /// for the same reason as `TrainerShow`: a party of four with names is the
    /// largest variant here by an order of magnitude.
    GroupList(Box<crate::play::group::GroupList>),
    /// `SMSG_GROUP_DESTROYED`: the party has ended. No body, and not the same as
    /// an empty roster, which the server never sends.
    GroupDestroyed,
    /// `SMSG_GROUP_SET_LEADER`: the new leader, by name, although the request
    /// that caused it named a guid.
    GroupNewLeader { name: String },
    /// `SMSG_PARTY_COMMAND_RESULT` — the result of an invite or a leave, as an
    /// index into `GlobalStrings.lua` and the name it applies to.
    PartyResult(crate::play::group::PartyCommandResult),
    /// `SMSG_PARTY_MEMBER_STATS` / `_FULL` — a member's health, mana, level and
    /// zone, for members too far away to be in the object manager.
    PartyMemberStats(crate::play::group::PartyMemberStats),
    /// `MSG_RAID_READY_CHECK`: one opcode with two meanings, distinguished by
    /// whether it has a body. See [`crate::play::group::ReadyCheck`].
    RaidReadyCheck(crate::play::group::ReadyCheck),

    // --- reputation ---
    //
    // See [`crate::play::reputation`]. All four are statements about the 64
    // reputation-list slots, and none can be displayed on its own: the standings
    // are deltas from a `Faction.dbc` base, and which rows exist is decided by a
    // rule the client applies and the packets do not carry.

    /// `SMSG_INITIALIZE_FACTIONS`: the whole standing table, 64 slots, replacing
    /// whatever was held. Boxed for the same reason as `GroupList`: 64 pairs is
    /// 512 bytes and every other variant here is a few.
    FactionsInitialized(Box<crate::play::reputation::FactionStates>),
    /// `SMSG_SET_FACTION_STANDING` — one or more slots' deltas moved. A list
    /// rather than a pair, because reputation spills onto parent factions and
    /// arrives as one packet naming both.
    FactionStandings(Vec<(u32, i32)>),
    /// `SMSG_SET_FACTION_VISIBLE`: a faction met for the first time, which adds a
    /// row to the panel rather than changing one.
    FactionVisible { reputation_list_id: u32 },
    /// `SMSG_SET_FACTION_ATWAR` — the server's statement of the at-war
    /// checkbox, carried as the whole flag byte rather than a boolean.
    FactionAtWar { reputation_list_id: u32, flags: u8 },
    /// `SMSG_SET_FORCED_REACTIONS`: the whole forced-reaction map, as
    /// `(factionId, rank)`, replacing whatever was held. See
    /// [`crate::play::reputation::parse_forced_reactions`], which explains why a
    /// merge would be wrong.
    ForcedReactions(Vec<(u32, u32)>),

    /// `SMSG_EXPLORATION_EXPERIENCE`: a newly discovered area, and the experience
    /// it gave.
    ///
    /// `{u32 areaId, u32 xp}`. It is the only announcement of a discovery:
    /// `PLAYER_EXPLORED_ZONES` is a `PRIVATE` field with no event of its own, so a
    /// client that does not read this packet fills in the world map silently and
    /// never shows a "Discovered:" line.
    ///
    /// Sent even when the experience is zero. vmangos' comment above the call
    /// (`Player.cpp`, "Exploration packet should be sent even if no XP is
    /// gained") says so, so the event does not depend on the player's level.
    Discovered { area: u32, experience: u32 },

    // --- the flight master ---
    //
    // See [`crate::play::taxi`]. Three edges and no state: the map arrives whole
    // and is replaced whole, and the flight it buys is an `SMSG_MONSTER_MOVE`
    // that reaches the movement code through a separate path.
    /// `SMSG_SHOWTAXINODES` — the flight map: a master, the node under it, and
    /// the 256-bit mask of everywhere this character has been.
    TaxiShow(crate::play::taxi::TaxiMenu),
    /// `SMSG_TAXINODE_STATUS` — whether this master's own node is known. Also
    /// the second half of a discovery, in which case the byte is already 1.
    TaxiNodeStatus { guid: u64, known: bool },
    /// `SMSG_NEW_TAXI_PATH`: a flight point was discovered. The packet has no
    /// body; its arrival is the whole message. It plays the `TaxiNodeDiscovered`
    /// sound and, in the 1.12.1 client, refreshes any open map.
    NewTaxiPath,
    /// `SMSG_ACTIVATETAXIREPLY` — accepted, or one of twelve refusal reasons.
    TaxiReply(crate::play::taxi::TaxiReply),

    // --- friends, ignore list and /who ---
    //
    // See [`crate::play::social`]. The two lists are guids without names, so
    // none of the three can be displayed until `CMSG_NAME_QUERY` has answered.
    // That is why they are events carrying the packet's contents rather than
    // resolved values.
    /// `SMSG_FRIEND_LIST`: the whole friends list, replacing whatever was held.
    /// It arrives once, at login. Every later change is a [`Self::FriendStatus`]
    /// and nothing sends this again.
    FriendList(Vec<crate::play::social::Friend>),
    /// `SMSG_IGNORE_LIST` — the whole ignore list. Like the friends list, it
    /// arrives once, at login.
    IgnoreList(Vec<u64>),
    /// `SMSG_FRIEND_STATUS` — one answer about one player: an add, a removal, a
    /// refusal, or a friend logging in or out.
    FriendStatus(crate::play::social::FriendStatus),
    /// `SMSG_WHO` — the search results and the online total.
    WhoResults(crate::play::social::WhoResults),

    // --- the guild ---
    //
    // See [`crate::play::guild`]. The character's own membership is not
    // here: it is two update fields on the player object.
    /// `SMSG_GUILD_QUERY_RESPONSE`: one guild's name, rank names and emblem.
    GuildQuery(crate::play::guild::GuildQuery),
    /// `SMSG_GUILD_ROSTER`: the whole member list, replacing the one held.
    GuildRoster(crate::play::guild::Roster),
    /// `SMSG_GUILD_EVENT`: a member joined, left, changed rank or logged in,
    /// or the message of the day was stated.
    GuildEvent(crate::play::guild::GuildEvent),
    /// `SMSG_GUILD_COMMAND_RESULT`: the answer to a guild request.
    GuildCommandResult(crate::play::guild::CommandResult),
    /// `SMSG_GUILD_INVITE`: an invitation to join a guild.
    GuildInvite(crate::play::guild::Invite),
    /// `SMSG_GUILD_DECLINE`: the named player declined the invitation.
    GuildDecline(String),
    /// `SMSG_GUILD_INFO`: the answer to `/ginfo`.
    GuildInfo(crate::play::guild::GuildInfo),
    /// `MSG_TABARDVENDOR_ACTIVATE`: a tabard designer opened its window. The
    /// value is the designer's guid.
    TabardVendor(u64),
    /// `MSG_SAVE_GUILD_EMBLEM`: the answer to saving an emblem, one of
    /// [`crate::play::guild::emblem_result`].
    GuildEmblemResult(u32),

    // --- the guild charter ---
    //
    // See [`crate::play::petition`].
    /// `SMSG_PETITION_SHOWLIST`: a guild registrar's charter offer.
    PetitionShowList(crate::play::petition::ShowList),
    /// `SMSG_PETITION_SHOW_SIGNATURES`: a charter and who has signed it.
    PetitionSignatures(crate::play::petition::Signatures),
    /// `SMSG_PETITION_QUERY_RESPONSE`: a petition's guild name and owner.
    PetitionQuery(crate::play::petition::PetitionQuery),
    /// `SMSG_PETITION_SIGN_RESULTS`: the answer to a signature.
    PetitionSignResult(crate::play::petition::SignResult),
    /// `SMSG_TURN_IN_PETITION_RESULTS`: one of
    /// [`crate::play::petition::result`].
    PetitionTurnInResult(u32),
    /// `MSG_PETITION_DECLINE`: the guid of the player who declined to sign.
    PetitionDeclined(u64),
    /// `MSG_PETITION_RENAME`: a charter's guild name changed.
    PetitionRenamed { item: u64, name: String },

    // --- mail ---
    //
    // See [`crate::play::mail`]. As with the two social lists, none of this can
    // be displayed on arrival: a letter's sender is a guid that `CMSG_NAME_QUERY`
    // has to resolve, and its text needs a second request.
    /// `SMSG_MAIL_LIST_RESULT`: the whole inbox, replacing whatever was held.
    /// Boxed for the same reason as [`Self::TrainerShow`]: it is by far the
    /// largest variant in this enum and arrives at most once a minute.
    MailList(Box<Vec<crate::play::mail::MailHeader>>),

    // --- the trade window ---
    //
    // See [`crate::play::trade`]. Two packets: the trade's state, and one
    // side's offer as the server states it.

    /// `SMSG_TRADE_STATUS` — a request, an open, an accept, a refusal or a
    /// close; see [`crate::play::trade::TradeStatus`].
    TradeStatus(crate::play::trade::TradeStatusPacket),
    /// `SMSG_TRADE_STATUS_EXTENDED` — all of one side's seven slots and
    /// money. Boxed for the same reason as [`Self::MailList`].
    TradeOffer(Box<crate::play::trade::TradeOffer>),
    /// `SMSG_SEND_MAIL_RESULT` — the answer shared by all seven mail
    /// requests.
    MailResult(crate::play::mail::MailResponse),
    /// `SMSG_RECEIVED_MAIL` — new mail has arrived. The packet carries
    /// nothing beyond its arrival.
    MailReceived,
    /// `MSG_QUERY_NEXT_MAIL_TIME`: seconds until the next letter, where 0 means
    /// one is already waiting; see [`crate::play::mail::parse_next_mail_time`].
    MailNextTime(f32),
    /// `SMSG_ITEM_TEXT_QUERY_RESPONSE` — the text of one letter, by the text
    /// id that was requested.
    ItemText { id: u32, text: String },
    /// `SMSG_CHANNEL_NOTIFY` — see [`crate::play::channels`].
    ChannelNotify(Box<crate::play::channels::ChannelNotify>),
    /// `SMSG_CHANNEL_LIST` — who is on a channel.
    ChannelList(Box<crate::play::channels::ChannelList>),
    /// `SMSG_TEXT_EMOTE` — a player's text emote, such as `/dance`; see
    /// [`crate::play::emotetext`].
    TextEmote(Box<crate::play::emotetext::TextEmote>),

    // ---- proficiency and spell modifiers ----
    /// `SMSG_SET_PROFICIENCY` — one item class's whole mask; see
    /// [`crate::play::skills::Proficiency`]. Replaces that class, never merges.
    Proficiency(crate::play::skills::Proficiency),
    /// `SMSG_SET_FLAT_SPELL_MODIFIER` / `_PCT_` — one bit of one operation's
    /// running total; see [`SpellModifier`](crate::play::spells::SpellModifier): the values are totals, not
    /// deltas.
    SpellModifier(SpellModifier),

    // ---- server notices ----
    //
    // See [`crate::play::notices`]. Each is shown as one line and changes no
    // state.

    /// `SMSG_CHAT_PLAYER_NOT_FOUND`: a whisper was addressed to this name and
    /// nobody by that name is playing.
    PlayerNotFound { name: String },
    /// `SMSG_SERVER_MESSAGE`: a shutdown or restart countdown, its
    /// cancellation, or a free text.
    ServerMessage(crate::play::notices::ServerMessage),
    /// `SMSG_ZONE_UNDER_ATTACK`: a guard or a PvP creature in this
    /// `AreaTable.dbc` zone was killed by a player of the other team.
    ZoneUnderAttack { area: u32 },
    /// `SMSG_DEFENSE_MESSAGE`: an Eastern Plaguelands tower's announcement.
    DefenseMessage(crate::play::notices::DefenseMessage),

    // ---- the group: rolls, pings, raid target icons and shared quests ----

    /// `MSG_RANDOM_ROLL`: somebody's `/roll`, the player's included. See
    /// [`crate::play::randomroll`].
    RandomRoll(crate::play::randomroll::RandomRoll),
    /// `MSG_MINIMAP_PING`: a group member clicked their minimap. See
    /// [`crate::play::minimap`].
    MinimapPing(crate::play::minimap::MinimapPing),
    /// `MSG_RAID_TARGET_UPDATE`: one icon changed, or the whole list. See
    /// [`crate::play::raidtarget`].
    RaidTargets(crate::play::raidtarget::RaidTargetUpdate),
    /// `MSG_QUEST_PUSH_RESULT`: what happened when the player shared a quest
    /// with one member. See [`crate::play::questshare`].
    QuestPushResult(crate::play::questshare::PushOutcome),
    /// `SMSG_QUEST_CONFIRM_ACCEPT`: a member accepted a party quest and the
    /// player is offered it too. Boxed because the title makes it one of the
    /// larger variants and it is rare.
    QuestConfirmAccept(Box<crate::play::questshare::ConfirmAccept>),

    // ---- world states and tutorials ----

    /// `SMSG_INIT_WORLD_STATES`: the zone's whole table, replacing the one
    /// held. Boxed because it carries about a hundred pairs. See
    /// [`crate::play::worldstate`].
    WorldStatesInit(Box<crate::play::worldstate::WorldStatesInit>),
    /// `SMSG_UPDATE_WORLD_STATE`: one value of the table changed.
    WorldStateUpdate { state: u32, value: i32 },
    /// `SMSG_TUTORIAL_FLAGS`: the account's tutorial mask, replacing the one
    /// held. See [`crate::play::tutorial`].
    TutorialFlags(crate::play::tutorial::TutorialFlags),
}
