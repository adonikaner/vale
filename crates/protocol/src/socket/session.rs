//! A *live* world session: a background thread that owns the socket, keeps it
//! alive, simulates the player, and holds the world state everyone else reads.
//!
//! Everything before this was a snapshot — connect, read for eight seconds,
//! drop the socket. That is enough to prove the protocol but it is not a
//! client: nothing moves, nothing updates, and the character cannot walk.
//!
//! The shape here is deliberate:
//!
//! * **One thread owns the [`WorldSession`].** A world socket is a single
//!   ordered stream with a stateful header cipher, so it cannot be shared;
//!   anything else would need a mutex around every read *and* would still have
//!   to arbitrate who gets the next packet.
//! * **Readers get an [`ObjectManager`] behind a mutex.** The thread holds the
//!   lock only while folding a packet in or advancing splines, so a UI polling
//!   at 20 Hz never contends meaningfully. Socket writes always happen with the
//!   lock released.
//! * **Input arrives as commands, not as direct calls.** A keypress from a
//!   WebView must not turn into a socket write on the WebView's thread.
//!
//! The tick is 25 ms: fast enough that the 500 ms movement heartbeat and the
//! 30 s keepalive land on time and that a keypress is picked up within half a
//! frame of the screen's, slow enough to cost nothing. It does **not** set the
//! packet rate — every send is gated by its own interval, so halving the tick
//! costs the socket nothing.
//!
//! ## The simulation is advanced by measured time, not by [`TICK`]
//!
//! A loop that sleeps for `TICK - work` does not run every `TICK`: Windows'
//! default scheduler granularity is ~15.6 ms, so `thread::sleep` rounds up to
//! the next tick of it and a loop asking for 25 ms gets somewhere between 16 and
//! 32. Advancing the mover by a *fixed* `TICK` while the wall clock does
//! something else makes the character's speed vary by tens of percent from one
//! step to the next — which is invisible in a log, because every individual
//! number is right, and reads on screen as the character juddering.
//!
//! Measured time is also what the server checks against. `CheckSpeedHack`
//! extrapolates `Unit::ExtrapolateMovement` over the **client's own `ctime`
//! delta** and only accumulates `m_overspeedDistance` when the reported distance
//! exceeds it — and `ctime` here is real elapsed milliseconds. So a fixed step
//! under a longer real tick reports *less* travel than the time it claims, which
//! is safe but wrong; advancing by the same clock `ctime` comes from makes the
//! two agree exactly. (`Anticheat.MaxAllowedDesync` defaults to 0, i.e. the
//! server does not clamp that delta at all.)

use crate::play::chat::{ChatType, Language};
use crate::socket::handler::{apply_packet, Incoming, LocalState, PumpStats, Replies};
use crate::play::wdb::{Caches, Kind};
use crate::play::areatrigger::{TriggerTable, TriggerWatch};
use crate::state::movement::{Controls, Footing, MovementInfo, Mover, Speeds};
use crate::state::objects::ObjectManager;
use crate::opcodes::Opcode;
use crate::state::query;
use crate::state::update::Position;
use crate::socket::world::{CharListEntry, Packet, Traffic, WorldSession};
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// What is underfoot and in the way **on a given map** — the world the local
/// simulation walks on. Supplied by the caller because the protocol crate
/// deliberately knows nothing about ADT or WMO files.
///
/// The map id is a parameter rather than baked into the implementation because
/// a far teleport changes it mid-session, and a lookup left pointing at the map
/// the character has left is worse than no lookup at all: Kalimdor's tile
/// (37, 47) exists and has a height, so the character would be walked onto
/// Azeroth's ground while standing in Tanaris — silently, and with a plausible
/// number. The same is true of a building: tile coordinates repeat across
/// continents, so a stale collider is an invisible wall in an empty field.
///
/// This is [`crate::state::movement::Footing`] with the map id added; the session
/// binds the id it is on now and hands the rest to the mover.
pub trait World: Send {
    fn floor(&self, map_id: u32, x: f32, y: f32, z: f32) -> Option<f32>;

    /// **Where the local character is**, once a tick, before anything asks
    /// about the ground there.
    ///
    /// The one position an implementation may bound its caches around and
    /// read ahead of. [`Self::floor`] cannot serve: it is asked for every
    /// dead-reckoned unit as well, and a cache centred on whichever unit asked
    /// last moves with each of them. Defaulted to nothing, for a world with
    /// nothing to prepare.
    fn focus(&self, _map_id: u32, _x: f32, _y: f32) {}

    fn step(&self, _map_id: u32, _from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
        to
    }

    /// The liquid surface over a point — see [`Footing::liquid`], of which this
    /// is the same question with the map id added.
    fn liquid(&self, _map_id: u32, _x: f32, _y: f32) -> Option<f32> {
        None
    }

    /// **Which game object's hull is directly under this point**, and where that
    /// object is right now.
    ///
    /// The one question here that is about a *thing* rather than a height, and
    /// it exists for one population: a character standing on a moving platform
    /// has to name it, because `MOVEFLAG_ONTRANSPORT` carries a guid and a
    /// position in that object's own frame. See
    /// [`crate::state::movement::Ferry`].
    ///
    /// It answers for a door and a chest as readily as for a lift — nothing on
    /// the collision side knows which objects move — and the *caller* filters
    /// on `UPDATEFLAG_TRANSPORT`, which is the only thing in the game that says
    /// so.
    ///
    /// Defaulted to `None`, so the CLI's terrain-only world and every test that
    /// predates transports keep the behaviour they had: never a passenger.
    fn platform(
        &self,
        _map_id: u32,
        _x: f32,
        _y: f32,
        _z: f32,
    ) -> Option<crate::state::movement::Platform> {
        None
    }

    /// …and the same thing asked **by guid**, for a unit whose spline is stated
    /// in a transport's frame — see
    /// [`crate::state::movement::Spline::in_world`], which is the only caller.
    ///
    /// Defaulted to `None`, which reads as *this platform is not loaded*: the
    /// passenger is then left where it was rather than placed at the map's
    /// origin.
    fn platform_of(
        &self,
        _map_id: u32,
        _guid: u64,
    ) -> Option<crate::state::movement::Platform> {
        None
    }

    /// **Is this game object's hull actually standing here**, as opposed to
    /// merely being an object the world has a placement for?
    ///
    /// [`Self::platform_of`] answers from the placement record, which is
    /// written for every game object that has been decided about — including
    /// the ones with nothing solid in them and the ones whose hull the renderer
    /// has not built yet. This asks the narrower question, and it is what
    /// separates a passenger who has walked ashore from one whose deck the
    /// world has temporarily stopped holding.
    ///
    /// A character who is aboard and is standing on no hull has stepped off
    /// the deck **only if the deck's hull was there to step off**. With no hull
    /// there is no evidence, and reading it as a step ashore un-boards the
    /// passenger — which on a same-map transport teleport leaves them behind,
    /// because `Transport::TeleportTransport` moves a passenger who does not
    /// change map with `TeleportPositionRelocation` and sends no packet at all.
    ///
    /// Defaulted to `false`, which reads as *no evidence*. The CLI's
    /// terrain-only world never boards anybody, so the arm behind this is
    /// unreachable there.
    fn platform_hulled(&self, _map_id: u32, _guid: u64) -> bool {
        false
    }
}

/// A terrain-height closure is a [`World`] with no buildings in it. The CLI
/// supplies one; the renderer supplies the real thing.
impl<F: Fn(u32, f32, f32, f32) -> Option<f32> + Send> World for F {
    fn floor(&self, map_id: u32, x: f32, y: f32, z: f32) -> Option<f32> {
        self(map_id, x, y, z)
    }
}

/// The caller's [`World`], if it has one.
pub type GroundHeight = Box<dyn World>;

/// A [`World`] with the map id already bound, which is what the mover wants.
///
/// Built fresh on every step rather than cached, because the map id it binds is
/// the one thing about it that changes — and a teleport that left a stale one
/// behind would walk the character on the continent they have left.
struct OnMap<'a> {
    world: &'a dyn World,
    map_id: u32,
    /// **Standing on a deck the client cannot currently locate**, in which case
    /// there is no answer about the floor — see [`Footing::floor`] below.
    ///
    /// False for everything but a passenger, and for a passenger it is true for
    /// under a second, once a cycle. Set by [`SessionLoop::tick_movement`],
    /// which is the only place that knows.
    adrift: bool,
}

impl Footing for OnMap<'_> {
    /// **A deck that has gone out of the client's world is "no answer", never
    /// "the sea".**
    ///
    /// `Footing::floor`'s `None` has always meant *the client has no data for
    /// this spot* rather than *there is no floor*, and both halves of the mover
    /// hold their altitude on it. A passenger whose platform the collision
    /// world can no longer place is exactly that case: the character is
    /// standing on something real that this client has temporarily stopped
    /// holding — a boat, one second a cycle, between its own route saying it
    /// has crossed to the other continent and `ShipTransport::Update` getting
    /// round to teleporting the passengers.
    ///
    /// Answering from the world there returns the *ocean floor*, and the
    /// character falls off the deck for the half second in between. That is not
    /// only a visible drop: the offset the fall writes is the offset the server
    /// rebuilds the position from on the far side
    /// (`CalculatePassengerPosition`), so a character who fell two yards
    /// arrives two yards under the deck, in the sea. Which is the report.
    fn floor(&self, x: f32, y: f32, z: f32) -> Option<f32> {
        if self.adrift {
            return None;
        }
        self.world.floor(self.map_id, x, y, z)
    }

    fn step(&self, from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
        self.world.step(self.map_id, from, to)
    }

    fn platform_of(&self, guid: u64) -> Option<crate::state::movement::Platform> {
        self.world.platform_of(self.map_id, guid)
    }

    fn liquid(&self, x: f32, y: f32) -> Option<f32> {
        self.world.liquid(self.map_id, x, y)
    }
}

/// **How long a deck whose entity is gone is stood on anyway**, before the
/// character is put back on the ground under them.
///
/// This bound applies only once the object manager no longer knows the ferry's
/// guid as a transport. While it does, the hold is open-ended: the deck is real
/// and merely unplaceable — this client's schedule ahead of or behind the
/// server's around a teleport frame (vmangos' frame times are its own
/// imperfect `GenerateWaypoints` output with only the period overridden, so
/// the disagreement has no fixed bound), a far side still loading, a frame
/// whose hull-rebuild budget ran out. Expiring any of those un-boards the
/// passenger: the next packet carries no flag, `HandleMoverRelocation` calls
/// `RemovePassenger`, and `Transport::TeleportTransport` then moves everybody
/// but us — which is *"the ship vanished and left me in the ocean"* arriving
/// through the mechanism that was meant to prevent it.
///
/// **The bound is what stops a lost boat becoming a wedged session.** A
/// transport the server destroys while somebody is standing on it dies on this
/// side with the server's own out-of-range block, `moves()` turns false, and
/// five seconds later the character swims instead of hovering at deck height
/// for the rest of the session. A stated judgement rather than a measurement.
const PLATFORM_HOLD: Duration = Duration::from_secs(5);

/// How far one step's carry may move a passenger before it is counted as a
/// jump, in yards — see [`SessionStatus::platform_jumps`].
///
/// A transport travels 30 y/s and a step is clamped to [`MAX_STEP`], so the
/// most a legitimate ride can carry anyone in one go is 7.5 yards. The same
/// 250 as `world::motion`'s own bound, and for the same reason: nothing that
/// travels can reach it.
const PLATFORM_JUMP: f32 = 250.0;

/// How often the loop wakes. **Not** how far the simulation advances — see the
/// module comment and [`SessionLoop::step_ms`].
const TICK: Duration = Duration::from_millis(25);

/// [`TICK`] in seconds, for a reader that needs a nominal step before it has
/// seen two readings of [`SessionStatus::world_ms`] to difference. Pinned
/// against `TICK` by a test rather than left to drift.
pub const TICK_SECS: f32 = 0.025;

/// How long a tick may spend draining the socket before it must go and do the
/// rest of its work. Bounded so that a busy zone cannot starve movement.
const READ_SLICE: Duration = Duration::from_millis(12);

/// The largest step the **local character** will take in one go, however long
/// the thread was actually away.
///
/// A stall — a login burst, the renderer starving this thread while it uploads a
/// tile — should not be walked off in a single stride. Clamping under-travels
/// relative to the `ctime` the packet will carry, which is the safe side of
/// `CheckSpeedHack`: the server allows less distance than time, never more.
///
/// **It used to clamp the world clock as well, and that was a cumulative
/// desync.** Everything the *server* is moving — every spline, and therefore
/// every creature in view — was advanced by the same clamped number, so each
/// stall over a quarter of a second left every one of them permanently that far
/// behind where the server had them. A creature on a chase gets the error wiped
/// by its next `SMSG_MONSTER_MOVE`; a patrolling guard on a **cyclic** spline
/// never does, because a cyclic path is stated once and never mentioned again
/// (see [`crate::state::movement::Spline::cyclic`]). The losses add up over a
/// session — a login burst alone is seconds — and what they add up to is a
/// creature drawn standing yards from where it is really fighting. So the world
/// takes the whole of the elapsed time now and only the mover is clamped; see
/// [`SessionLoop::step_ms`] and [`SessionStatus::stalls`].
const MAX_STEP: Duration = Duration::from_millis(250);

/// The most world time one tick will walk off after a stall, however long the
/// thread was really away — see [`slices`], which is where the reason is.
const MAX_CATCH_UP: Duration = Duration::from_secs(10);

/// One step's **two** clocks.
///
/// They are the same number in every ordinary tick and differ only out of a
/// stall, which is exactly the case that used to be wrong in both directions at
/// once — see [`MAX_STEP`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Step {
    /// Real elapsed milliseconds: what the *server* has been doing, and
    /// therefore what every spline and every dead reckoning is walked by.
    world_ms: u32,
    /// …and what the local character may walk off in one stride, clamped to
    /// [`MAX_STEP`] because the anticheat measures it against real time.
    mover_ms: u32,
}

/// Movement heartbeat interval.
///
/// vmangos raises `CHEAT_TYPE_SKIPPED_HEARTBEATS` when consecutive packets from
/// a *moving* client are more than 1000 ms apart, with the comment "client
/// should send heartbeats every 500ms". 400 leaves room for a slow tick.
const HEARTBEAT: Duration = Duration::from_millis(400);

/// Minimum gap between updates sent because the *facing* changed, so that
/// holding a turn key does not become one packet per tick.
///
/// **100 ms rather than the 250 it started at**, because this now paces turning
/// while running as well as while standing (see [`SessionLoop::tick_movement`]),
/// and there it bounds how far the server's straight-line extrapolation may
/// diverge from the arc actually walked. A quarter of a second of a 180°/s turn
/// at a run is 45° of unreported heading; a tenth is 18°, which keeps the
/// separation inside `CheckSpeedHack`'s 10% allowance. Ten packets a second is
/// also what the real client sends while turning.
const FACING_INTERVAL: Duration = Duration::from_millis(100);

/// `CMSG_PING` interval. The point is to notice a dead link early — without it
/// a stalled session looks like a mysterious disconnect much later.
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// How often to ask about entries we have seen but cannot name.
const QUERY_INTERVAL: Duration = Duration::from_secs(2);

/// How long to wait for the login burst to produce a player object.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10);

/// The five things a loot window can ask the socket for.
///
/// Its own enum rather than five `Command` variants — see [`Command::Loot`],
/// which is where the reason is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LootVerb {
    /// `CMSG_LOOT` on a body.
    Open(u64),
    /// `CMSG_AUTOSTORE_LOOT_ITEM` — by the **server's** index, never the row.
    Take(u8),
    /// `CMSG_LOOT_MONEY`, which has no body.
    TakeMoney,
    /// `CMSG_LOOT_RELEASE`. The guid is a formality the server discards.
    Release(u64),
    /// `CMSG_LOOT_ROLL` — need, greed or pass on one row of a group roll.
    ///
    /// **The body and the slot, not the interface's `rollID`** — see
    /// [`crate::play::lootroll`], where the id is shown to be a local counter.
    Roll {
        guid: u64,
        item_slot: u32,
        vote: crate::play::lootroll::RollVote,
    },
}

/// The eight things a quest conversation asks the socket for.
///
/// Its own enum rather than eight `Command` variants, on the same terms
/// [`LootVerb`] is: one subject, and the loop makes no distinction between
/// them beyond which method to call. See [`crate::play::quest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestVerb {
    /// `CMSG_QUESTGIVER_STATUS_QUERY` — what is over this head?
    Status(u64),
    /// `CMSG_QUESTGIVER_HELLO` — right-click a giver.
    Hello(u64),
    /// `CMSG_QUESTGIVER_QUERY_QUEST` — show me this one.
    Details { guid: u64, quest_id: u32 },
    /// `CMSG_QUESTGIVER_ACCEPT_QUEST`.
    Accept { guid: u64, quest_id: u32 },
    /// `CMSG_QUESTGIVER_COMPLETE_QUEST` — hand it in.
    Complete { guid: u64, quest_id: u32 },
    /// `CMSG_QUESTGIVER_REQUEST_REWARD` — show me the reward page again.
    RequestReward { guid: u64, quest_id: u32 },
    /// `CMSG_QUESTGIVER_CHOOSE_REWARD` — and take *this* one, by zero-based
    /// index into the choices.
    ChooseReward {
        guid: u64,
        quest_id: u32,
        choice: u32,
    },
    /// `CMSG_QUEST_QUERY` — what *is* quest 47? The one verb with no giver.
    Query(u32),
    /// `CMSG_QUESTLOG_REMOVE_QUEST` — by **log slot**, zero-based, which is not
    /// the quest id.
    Abandon(u8),
}

/// The nine things a gossip, vendor or trainer window asks the socket for.
///
/// See [`crate::play::gossip`], which owns the wire — including the one body in the
/// game whose fields are backwards ([`crate::play::gossip::npc_text_query_body`]) —
/// and [`crate::play::trainer`], which owns the last two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpcVerb {
    /// `CMSG_GOSSIP_HELLO` — right-click anything with the gossip flag.
    GossipHello(u64),
    /// `CMSG_GOSSIP_SELECT_OPTION` — by the **server's** index.
    GossipSelect { guid: u64, option: u32 },
    /// `CMSG_NPC_TEXT_QUERY` — the words behind a menu's text id.
    TextQuery { text_id: u32, guid: u64 },
    /// **`CMSG_BINDER_ACTIVATE` — yes, make this inn my home.**
    ///
    /// The answer to `SMSG_BINDER_CONFIRM`, and the packet that actually binds:
    /// until it arrives the server has done nothing but close the gossip
    /// window. The guid must be the innkeeper's — see
    /// [`crate::play::bindpoint::binder_activate_body`].
    BinderActivate(u64),
    /// `CMSG_PAGE_TEXT_QUERY` — **what does page N say?** The guid is the
    /// object being read and is optional on the wire; see
    /// [`crate::play::pagetext::page_text_query_body`].
    PageTextQuery { page_id: u32, guid: Option<u64> },
    /// `CMSG_LIST_INVENTORY` — open the shop.
    ListInventory(u64),
    /// `CMSG_BUY_ITEM` — by item **entry**, `count` things.
    Buy { vendor: u64, entry: u32, count: u8 },
    /// `CMSG_SELL_ITEM` — by the item object's **guid**; 0 sells the stack.
    Sell { vendor: u64, item: u64, count: u8 },
    /// `CMSG_BUYBACK_ITEM` — by the **wire's** slot, 69-based.
    Buyback { vendor: u64, wire_slot: u32 },
    /// `CMSG_REPAIR_ITEM` — the armourer and one item's guid, or **0 for all of
    /// them**, which is what `RepairAllItems()` sends. See
    /// [`crate::play::gossip::repair_item_body`], including why nothing comes back.
    Repair { vendor: u64, item: u64 },
    /// `CMSG_TRAINER_LIST` — open the training window.
    TrainerList(u64),
    /// `CMSG_TRAINER_BUY_SPELL` — by the **service** spell id the list gave,
    /// which is the teaching spell rather than the one that is learned.
    TrainerBuy { trainer: u64, spell: u32 },
    /// `MSG_LIST_STABLED_PETS` — re-ask for the stable window. **Outbound on an
    /// `MSG_` opcode**, which is the same name the answer arrives under.
    StableList(u64),
    /// `CMSG_STABLE_PET` — put the pet that is out away. The server picks the
    /// slot; the client names none.
    StablePet(u64),
    /// `CMSG_UNSTABLE_PET` — take one out, by pet number. Refused while a pet
    /// is summoned, which is what [`Self::StableSwap`] is for.
    UnstablePet { stable: u64, pet_number: u32 },
    /// `CMSG_STABLE_SWAP_PET` — exchange the pet that is out for a stabled one.
    StableSwap { stable: u64, pet_number: u32 },
    /// `CMSG_BUY_STABLE_SLOT` — the next slot, at `StableSlotPrices.dbc`'s
    /// price. The client checks the money first and the server checks it again.
    BuyStableSlot(u64),
    /// `CMSG_BANKER_ACTIVATE` — open the bank, at a banker with no gossip
    /// bit. One with the bit answers the same window through its menu. See
    /// [`crate::play::bank`].
    BankerActivate(u64),
    /// `CMSG_BUY_BANK_SLOT` — the next bag slot, at `BankBagSlotPrices.dbc`'s
    /// price. **Success has no reply**: the count is a byte of
    /// `PLAYER_BYTES_2` and arrives as a field.
    BuyBankSlot(u64),
}

/// **The seven things a player can tell a pet**, and the two more the panel
/// asks for.
///
/// One enum rather than nine `Command` variants for the reason [`NpcVerb`] is
/// one: they are one subject, they all name the pet's guid, and the dispatch is
/// a table in [`crate::socket::world::WorldSession::pet`].
///
/// **The first is the only one that is not a fixed opcode**: pressing a slot is
/// `CMSG_PET_ACTION` with the slot's own packed word sent back unchanged, and
/// the same packet is how *every* command and reaction button works — the bar
/// the server sent already holds "follow" and "aggressive" as slots. So a
/// binding like `PetFollow` is this verb with the word the bar carries rather
/// than an opcode of its own. See [`crate::play::pet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PetVerb {
    /// `CMSG_PET_ACTION` — press a slot, at a target (0 for none).
    Action { pet: u64, data: u32, target: u64 },
    /// `CMSG_PET_NAME_QUERY` — what the player called it, by pet number.
    NameQuery { pet_number: u32, pet: u64 },
    /// `CMSG_PET_SET_ACTION` — one move, or a swap of two.
    SetAction { pet: u64, moves: Vec<(u32, u32)> },
    /// `CMSG_PET_SPELL_AUTOCAST` — the little dot.
    Autocast { pet: u64, spell_id: u32, on: bool },
    /// `CMSG_PET_STOP_ATTACK`.
    StopAttack(u64),
    /// `CMSG_PET_ABANDON` — released for good, not dismissed.
    Abandon(u64),
    /// `CMSG_PET_UNLEARN` — reset its skills for money.
    Unlearn(u64),
    /// `CMSG_PET_RENAME`.
    Rename { pet: u64, name: String },
    /// `CMSG_PET_CANCEL_AURA` — cancel one of the pet's own buffs.
    CancelAura { pet: u64, spell_id: u32 },
}

/// **The three things the reputation panel asks the server for.**
///
/// All three name a *reputation-list id* rather than a faction id, because that
/// is what the 64-slot table on both sides is keyed by — see
/// [`crate::play::reputation`]. None of them is acknowledged: at-war comes back
/// as `SMSG_SET_FACTION_ATWAR`, inactive comes back as nothing at all, and the
/// watched faction comes back as an update field. So the panel's own copy is
/// what draws, and these are told-so-it-is-remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReputationVerb {
    /// `CMSG_SET_FACTION_ATWAR`.
    AtWar { reputation_list_id: u32, at_war: bool },
    /// `CMSG_SET_FACTION_INACTIVE` — which heading the row is filed under, and
    /// **the only one of the three that is purely cosmetic**: the server stores
    /// it in `characters.data` and does nothing else with it.
    Inactive { reputation_list_id: u32, inactive: bool },
    /// `CMSG_SET_WATCHED_FACTION` — **`-1` clears it**, which is what the
    /// interface's "Show as experience bar" tick sends when it is unticked.
    Watched { reputation_list_id: i32 },
}

/// **The sixteen things the chat frame asks the socket for about a channel.**
///
/// See [`crate::play::channels`], which owns the bodies. Every one carries
/// the channel's name, because that is the wire's own key — the numbers the
/// interface types (`/2 hello`) are the client's slots and never leave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelVerb {
    Join { name: String, password: String },
    Leave(String),
    /// `/chatlist <name>`, answered by `SMSG_CHANNEL_LIST`.
    List(String),
    Password { name: String, password: String },
    SetOwner { name: String, player: String },
    /// Who owns it, answered by a `CHANNEL_OWNER` notice.
    Owner(String),
    Moderator { name: String, player: String },
    Unmoderator { name: String, player: String },
    Mute { name: String, player: String },
    Unmute { name: String, player: String },
    Invite { name: String, player: String },
    Kick { name: String, player: String },
    Ban { name: String, player: String },
    Unban { name: String, player: String },
    Announcements(String),
    Moderate(String),
}

/// **The six things the social panel asks the socket for.**
///
/// See [`crate::play::social`], which owns the bodies. Not `Copy`, because three
/// of them carry a string — and that is the wire's own asymmetry rather than a
/// choice here: **an addition is by name and a removal is by guid**, because the
/// client has no guid for somebody it has not met and the server has no name
/// for somebody it already knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocialVerb {
    /// `CMSG_FRIEND_LIST` — ask for the list again. No body. The server sends
    /// it unprompted at login and never again, so this is the only way to
    /// re-read it.
    List,
    /// `CMSG_ADD_FRIEND` — `/friend <name>`, and the Who panel's Add Friend
    /// button.
    AddFriend(String),
    /// `CMSG_DEL_FRIEND` — by guid, which the panel has because the list gave
    /// it one.
    DelFriend(u64),
    /// `CMSG_ADD_IGNORE`.
    AddIgnore(String),
    /// `CMSG_DEL_IGNORE`.
    DelIgnore(u64),
    /// `CMSG_WHO` — the search, already parsed out of the line somebody typed.
    /// Boxed because it is by some way the largest variant here and every other
    /// one is a word or a short string.
    Who(Box<crate::play::social::WhoRequest>),
}

/// **The nine things the mail window asks the socket for.**
///
/// See [`crate::play::mail`], which owns the bodies — and which carries the one
/// thing that is *not* here: **opening a mailbox crosses no wire at all**, so
/// there is no `Open` verb. What holds the guid is the client's own window
/// state, and every verb below stamps it, because the server checks that the
/// character is standing at that mailbox before it will do anything.
///
/// Five of the nine differ only in their opcode, which is why they share
/// [`crate::play::mail::mail_id_body`] rather than each getting a function.
/// **The trade window's ten verbs** — see [`crate::play::trade`], which
/// carries the layouts. Six are bodiless; `Initiate` names the partner,
/// `SetItem` a bag square by the server's numbering, `ClearItem` a slot and
/// `SetGold` an amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeVerb {
    /// `CMSG_INITIATE_TRADE` — `InitiateTrade(unit)`.
    Initiate(u64),
    /// `CMSG_BEGIN_TRADE` — the `TRADE` popup's Yes.
    Begin,
    /// `CMSG_BUSY_TRADE` — the refusal a client sends while it has a window
    /// up already.
    Busy,
    /// `CMSG_IGNORE_TRADE` — the refusal the `BlockTrades` CVar sends.
    Ignore,
    /// `CMSG_ACCEPT_TRADE` — the Trade button.
    Accept,
    /// `CMSG_UNACCEPT_TRADE` — `CancelTradeAccept()`, the Cancel button while
    /// accepted.
    Unaccept,
    /// `CMSG_CANCEL_TRADE` — the popup's No, `CloseTrade()`, and the window's
    /// close.
    Cancel,
    /// `CMSG_SET_TRADE_ITEM` — `ClickTradeButton(i)` with something held.
    SetItem { trade_slot: u8, bag: u8, slot: u8 },
    /// `CMSG_CLEAR_TRADE_ITEM` — …and with nothing held on a filled slot.
    ClearItem { trade_slot: u8 },
    /// `CMSG_SET_TRADE_GOLD` — `SetTradeMoney(copper)`.
    SetGold(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailVerb {
    /// `CMSG_GET_MAIL_LIST` — the inbox. **Rate-limited by the caller**, not
    /// here: see [`crate::play::mail`]'s note on `CheckInbox`.
    List(u64),
    /// `CMSG_MAIL_MARK_AS_READ` — sent by *reading* a letter rather than by any
    /// button, which is `GetInboxText`'s own first act.
    MarkAsRead { mailbox: u64, mail_id: u32 },
    /// `CMSG_MAIL_TAKE_MONEY` — the coins in it.
    TakeMoney { mailbox: u64, mail_id: u32 },
    /// `CMSG_MAIL_TAKE_ITEM` — the parcel. **Also what pays a COD**: the server
    /// takes the money on this packet, which is why the interface puts a
    /// confirmation box in front of it and this does not.
    TakeItem { mailbox: u64, mail_id: u32 },
    /// `CMSG_MAIL_CREATE_TEXT_ITEM` — keep the letter itself, as an item.
    TakeText {
        mailbox: u64,
        mail_id: u32,
        /// Echoed back and thrown away by the server; sent because 5875 sends
        /// it and the read is unconditional.
        template_id: u32,
    },
    /// `CMSG_MAIL_RETURN_TO_SENDER`.
    Return { mailbox: u64, mail_id: u32 },
    /// `CMSG_MAIL_DELETE` — **refused for a COD letter**, with
    /// `MAIL_ERR_INTERNAL_ERROR` rather than anything readable.
    Delete { mailbox: u64, mail_id: u32 },
    /// `CMSG_ITEM_TEXT_QUERY` — the words of one letter.
    TextQuery { item_text_id: u32, mail_id: u32 },
    /// `CMSG_SEND_MAIL` — a whole letter. Boxed for the reason
    /// [`SocialVerb::Who`] is: three strings against everything else's two
    /// words.
    Send(Box<crate::play::mail::OutgoingMail>),
    /// `MSG_QUERY_NEXT_MAIL_TIME` — is anything waiting? No body, and the
    /// answer comes back under the same opcode.
    NextTime,
}

/// **The six things the party interface asks the socket for.**
///
/// See [`crate::play::group`], which owns the bodies. Not `Copy`, because two of them
/// carry a name — which is the wire's own asymmetry rather than a choice here:
/// an invite and a by-name kick are strings and everything else is a guid or
/// nothing at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartyVerb {
    /// `CMSG_GROUP_INVITE` — `/invite <name>`, and the whole of `InviteByName`.
    Invite(String),
    /// `CMSG_GROUP_ACCEPT` — press Accept on the popup. No body.
    Accept,
    /// `CMSG_GROUP_DECLINE` — …or Decline. No body.
    Decline,
    /// **`CMSG_GROUP_DISBAND` — leaving *and* disbanding, one opcode.** Which
    /// one happens is the server's decision off whether we lead, so `LeaveParty`
    /// has nothing to decide.
    Leave,
    /// `CMSG_GROUP_UNINVITE` — kick, **by name**, which is what
    /// `UninviteUnit(name)` has to hand and what the wire happens to accept.
    Uninvite(String),
    /// …and `CMSG_GROUP_UNINVITE_GUID`, the same verb by guid, for a caller that
    /// has one. Both exist on the wire; neither is a fallback for the other.
    UninviteGuid(u64),
    /// `CMSG_GROUP_SET_LEADER` — promote, likewise by guid. What comes back
    /// names the new leader by *name*.
    SetLeader(u64),
    /// `CMSG_REQUEST_PARTY_MEMBER_STATS` — the one poll this subject has, for a
    /// member too far away to be in the object manager.
    RequestStats(u64),
    /// `CMSG_GROUP_RAID_CONVERT` — the Raid tab's one button. No body, and the
    /// two conditions on it (there is a `party1`, and we lead) are
    /// the client's own — the server checks them again and answers a refusal
    /// with silence.
    ConvertToRaid,
    /// `CMSG_GROUP_CHANGE_SUB_GROUP` — move a member into a subgroup, **by name
    /// and zero-based**. See [`crate::play::group::change_sub_group_body`].
    ChangeSubGroup { name: String, subgroup: u8 },
    /// `CMSG_GROUP_SWAP_SUB_GROUP` — …or exchange two of them, which is what a
    /// drop onto an occupied slot is.
    SwapSubGroup { name: String, swap_with: String },
    /// `CMSG_GROUP_ASSISTANT_LEADER` — promote to assistant, or demote from it.
    /// **By guid in 1.12**, where the two subgroup verbs are by name.
    SetAssistant { guid: u64, assistant: bool },
    /// `MSG_RAID_READY_CHECK` with no body — start one. Leader and assistants
    /// only, which the server enforces.
    StartReadyCheck,
    /// …and the same opcode with one byte — answer one.
    AnswerReadyCheck(bool),
}

/// **The three things a flight master's window asks the socket for.**
///
/// See [`crate::play::taxi`]. Its own enum rather than three `NpcVerb` variants for
/// the reason [`PartyVerb`] is one: a multi-hop flight carries a *list* of
/// nodes, and `NpcVerb` is `Copy`.
///
/// **There is no close verb**, exactly as there is none for the gossip menu or
/// the trainer: `CloseTaxiMap()` clears the window locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaxiVerb {
    /// `CMSG_TAXIQUERYAVAILABLENODES` — right-click a flight master. The server
    /// answers with the map, or with a discovery first if this node is new.
    QueryNodes(u64),
    /// `CMSG_TAXINODE_STATUS_QUERY` — is this master's own node one we know?
    /// Asked about a unit rather than about a window, which is why it is here
    /// and not on the board.
    NodeStatus(u64),
    /// `CMSG_ACTIVATETAXI` — fly, by two node ids. The source is the map's
    /// current node and not anything the player chose.
    Activate { guid: u64, from: u32, to: u32 },
    /// `CMSG_ACTIVATETAXIEXPRESS` — …or by a whole route, source first, when no
    /// direct path exists.
    ActivateExpress {
        guid: u64,
        cost: u32,
        nodes: Vec<u32>,
    },
}

/// What a UI sends the session thread.
#[derive(Debug, Clone)]
pub enum Command {
    /// Which movement keys are held now.
    Controls(Controls),
    // (see [`Command::Loot`] for the one below that carries a nested enum)
    /// Absolute facing in radians — mouse-look, rather than the turn keys.
    Face(f32),
    /// **Absolute body pitch in radians, up-positive** — which is the camera's,
    /// and which only means anything in the water.
    ///
    /// Separate from [`Command::Face`] because the two are not the same
    /// quantity on the wire: the yaw is in every movement block and announces
    /// itself with `MSG_MOVE_SET_FACING`, and the pitch is serialised **only**
    /// under `MOVEFLAG_SWIMMING` and announces nothing. See
    /// [`Mover::set_pitch`].
    Pitch(f32),
    /// Leave the ground. An edge, not a held control: the server allows exactly
    /// one `MSG_MOVE_JUMP` between landings and rejects the rest, so a held key
    /// has to become one command and not sixty.
    Jump,
    /// Start swinging at a unit, or `None` to stop.
    Attack(Option<u64>),
    /// **Which unit is selected**, or `None` for nothing.
    ///
    /// The selection itself is the *client's*, decided the instant the player
    /// clicks — the ring appears with no round trip, which is the real client's
    /// behaviour. This only tells the server, which needs to know for the half
    /// dozen things it does with a selection (see
    /// [`WorldSession::set_selection`]).
    Target(Option<u64>),
    /// Cast a spell at a unit, or at nobody.
    ///
    /// **The target is decided before it gets here.** Which unit a spell binds
    /// to — the selection, ourselves, or nothing at all — is a rule read off
    /// `Spell.dbc`, and it lives in `vale_assets::tables::spell` where it can be
    /// unit-tested without a socket. This loop only sends what it is given.
    Cast {
        spell_id: u32,
        target: crate::play::spells::CastTarget,
    },
    /// Stop a cast that is still winding up.
    CancelCast(u32),
    /// **Drop a buff we are carrying**, by spell id — the right-click on an
    /// icon in the buff bar.
    ///
    /// A command rather than a direct send for the same reason every other one
    /// here is: the interface runs on the render thread and the socket belongs
    /// to the session thread. Nothing is predicted — the aura leaves the bar
    /// when the server says the slot is empty, which is one round trip and is
    /// what the real client shows too.
    CancelAura(u32),
    /// **Right-click something in a bag or on the paper doll**, in whichever of
    /// its two forms the item's own prototype decided.
    ///
    /// The decision — use it or wear it, and which of the five spell blocks
    /// fires — is made before it gets here, on the same terms [`Command::Cast`]
    /// states: it is a rule about the item, readable off
    /// [`crate::state::query::ItemInfo`] with no socket. The slot pair is already the
    /// server's numbering ([`crate::play::items::server_container_slot`]).
    UseItem {
        bag: u8,
        slot: u8,
        /// `None` sends `CMSG_AUTOEQUIP_ITEM` instead — the item is worn rather
        /// than used, and the server picks the paper-doll slot itself.
        spell_index: Option<u8>,
        target: crate::play::spells::CastTarget,
    },
    /// **Open something in a bag that holds loot** — `CMSG_OPEN_ITEM`, the
    /// third thing a right-click can be.
    ///
    /// Its own command rather than a third shape of [`Command::UseItem`]
    /// because it is not a use: no spell index, no target, and what comes back
    /// is a loot window rather than a cast. The decision that this is the one
    /// to send is `ItemInfo::is_openable`, a rule about the prototype with no
    /// socket in it. The slot pair is already the server's numbering.
    OpenItem { bag: u8, slot: u8 },
    /// **Move an item from one place to another** — the packet a *drop* sends,
    /// where [`Command::UseItem`] is what a right-click sends.
    ///
    /// One command for five opcodes, because the choice between them is
    /// arithmetic on the two ends rather than a decision: a swap with a bag on
    /// either side is `CMSG_SWAP_ITEM` and one without is `CMSG_SWAP_INV_ITEM`,
    /// a partial stack is `CMSG_SPLIT_ITEM`, a destination with no slot is
    /// `CMSG_AUTOSTORE_BAG_ITEM`, and no destination at all is
    /// `CMSG_DESTROYITEM`. Both ends are already the server's numbering
    /// ([`crate::play::items::server_container_slot`]).
    ///
    /// **Nothing is predicted.** The item moves when the update block says the
    /// slots changed, which is one round trip — and the client's own cursor is
    /// let go of at the *send*, so a refused move puts the item back where it
    /// was rather than leaving it in mid-air.
    MoveItem {
        src_bag: u8,
        src_slot: u8,
        /// `None` destroys it: `CMSG_DESTROYITEM`, which is what
        /// `DeleteCursorItem` is.
        dst: Option<(u8, u8)>,
        /// `None` moves the whole stack. `Some(n)` is `CMSG_SPLIT_ITEM` with
        /// that count, which the server drops outright when it is zero.
        count: Option<u8>,
    },
    /// **The four verbs of a loot window** — open it, take a row, take the
    /// coins, close it. See [`crate::play::loot`].
    ///
    /// One command with a small enum rather than four, because all four are the
    /// same subject and three of them carry nothing: the alternative is four
    /// variants of which two are unit structs, and a match arm apiece in the
    /// loop for no distinction the loop makes.
    Loot(LootVerb),
    /// **Use something in the world that is not a person** — `CMSG_GAMEOBJ_USE`
    /// on a door, a chest, an ore vein, a mailbox or a lever. See
    /// [`crate::play::object`].
    ///
    /// One guid and no verb enum beside it, because there is exactly one thing
    /// a client may do to a game object: the *kind* of thing that then happens —
    /// a state change, a loot window, a gathering cast, a quest page — is the
    /// server's decision off the template, and it arrives through whichever of
    /// those paths it belongs to rather than as a reply to this.
    UseObject(u64),
    /// **The quest conversation**, in the same shape and for the same reason —
    /// see [`QuestVerb`], where the eight are.
    Quest(QuestVerb),
    /// **…and the gossip and vendor one**, one enum for the same reason again.
    Npc(NpcVerb),
    /// **…and the pet's nine** — see [`PetVerb`]. Not `Copy`, because a rename
    /// carries a name and a set-action carries a list.
    Pet(PetVerb),
    /// **…and the party's six** — see [`PartyVerb`]. The one command family in
    /// this list that is not `Copy`, because two of its verbs are a name.
    Party(PartyVerb),
    /// **…and the flight master's three** — see [`TaxiVerb`]. Not `Copy` either,
    /// for the same kind of reason: a multi-hop flight is a list of nodes.
    Taxi(TaxiVerb),
    /// **…and the reputation panel's three** — see [`ReputationVerb`].
    Reputation(ReputationVerb),
    /// **…and the social panel's six** — see [`SocialVerb`]. Not `Copy` either:
    /// three of them are a name and one is a whole search.
    Social(SocialVerb),
    /// **…and the sixteen a chat channel takes** — see [`ChannelVerb`].
    Channel(ChannelVerb),
    /// **…and the mailbox's nine** — see [`MailVerb`]. Not `Copy`: sending a
    /// letter carries three strings, and it is boxed inside the verb for the
    /// same reason a `TrainerList` is.
    Mail(MailVerb),
    /// **…and the trade window's ten** — see [`TradeVerb`].
    Trade(TradeVerb),
    /// **Wear whatever is at this position**, wherever it fits —
    /// `CMSG_AUTOEQUIP_ITEM`, which names no destination because
    /// `CanEquipItem(NULL_SLOT, …)` picks the free finger.
    ///
    /// Separate from [`Command::MoveItem`] rather than a `dst` of "the paper
    /// doll", because it is the one move whose destination the *server* chooses
    /// among several.
    EquipItem { bag: u8, slot: u8 },
    /// **Move an item across the bank counter**, wherever it fits on the
    /// other side — `CMSG_AUTOBANK_ITEM` into the bank, `CMSG_AUTOSTORE_BANK_ITEM`
    /// out of it, one body between them. The right-click on a square while
    /// the window is open; see [`crate::play::bank`]. The direction is
    /// [`crate::play::items::is_bank_position`] of the source, which is the
    /// server's own fork.
    BankItem { bag: u8, slot: u8 },
    /// **Put something on an action button, or take it off** —
    /// `CMSG_SET_ACTION_BUTTON`.
    ///
    /// The opposite direction from every other command here: the bar is the
    /// *client's* state and this only asks the server to remember it, so the
    /// local half has already happened by the time this is sent and nothing
    /// waits for an answer (there is none — see
    /// [`crate::play::spells::set_action_button_body`]).
    ///
    /// The slot is zero-based and `packed` is already `(kind << 24) | action`,
    /// with **zero meaning "empty this one"** — built by
    /// [`crate::play::spells::ActionButton::packed`] so that the two halves cannot be
    /// swapped at a call site.
    SetActionButton { slot: u8, packed: u32 },
    /// **Remember which of the four extra action bars are on** —
    /// `CMSG_SET_ACTIONBAR_TOGGLES`, whose whole body is this byte.
    ///
    /// Like [`Command::SetActionButton`] it is the client telling the server
    /// about the client's own state, and unlike it the server *echoes* — the
    /// byte lands in `PLAYER_FIELD_BYTES` and comes back in the next values
    /// block, which is what makes the bars survive a logout. Nothing waits for
    /// it: the interface moved its own frames before this was ever sent.
    SetActionBarToggles(u8),
    /// **Stop the ranged auto-repeat** — see
    /// [`WorldSession::cancel_auto_repeat`], which is why there is no matching
    /// start: one begins with an ordinary [`Command::Cast`].
    CancelAutoRepeat,
    /// **Tell the server we have drawn or put away the weapons.**
    ///
    /// The decision is entirely the client's — see
    /// [`WorldSession::set_sheathed`] and `vale_assets::look::sheath` — and this
    /// only volunteers it, so that other people see the sword in our hand. The
    /// local character's own appearance has already changed by the time this is
    /// sent, which is why nothing waits for the echo.
    SetSheathed(u8),
    /// Say something, or run a GM command — the same packet either way.
    ///
    /// **No language here.** The caller states the *kind* of message and who it
    /// is for; which language to speak is a protocol rule with two silent
    /// failure modes (see [`crate::play::chat`]), so the loop fills it in from the
    /// character's race rather than trusting a UI to know.
    Chat {
        kind: ChatType,
        /// The whisper's recipient or the channel's name.
        target: Option<String>,
        text: String,
    },
    /// Play a text emote — `/dance`, `/wave`.
    ///
    /// **The only way this client can make an `SMSG_EMOTE` happen**, which is
    /// what makes the animation path testable at all: otherwise the packet
    /// arrives only when somebody else emotes, and "we never animate emotes" is
    /// indistinguishable from "nobody emoted".
    /// `/dance` — the `EmotesText` row, the `Emotes` id beside it and the
    /// target's guid; see [`crate::play::emotetext`].
    TextEmote { text_emote: u32, emote_num: u32, target: u64 },
    /// **Ask to leave the world**, or take the ask back — `GameMenuFrame`'s own
    /// Logout button and the CAMP popup's Cancel.
    ///
    /// Not [`Command::Shutdown`], which closes the socket: this is a request the
    /// *server* answers, and the session goes on running — sat down and rooted —
    /// until it does. See [`crate::play::logout`].
    Logout {
        /// `false` is `CMSG_LOGOUT_CANCEL`.
        leaving: bool,
    },
    /// **Release the spirit** — `CMSG_REPOP_REQUEST`, the game's own `RepopMe`.
    ///
    /// Bodiless, and the reply is not a packet: what comes back is
    /// `PLAYER_FLAGS_GHOST` appearing in an ordinary values block, plus one
    /// `SMSG_CORPSE_RECLAIM_DELAY`. See [`crate::play::death`].
    Repop,
    /// **Reset every instance this character is saved to** —
    /// `CMSG_RESET_INSTANCES`, bodiless, and there is no success reply:
    /// `HandleResetInstancesOpcode` reads nothing, acts on `_player` (or on the
    /// group when we lead one) and answers only a *failure*, with
    /// `SMSG_INSTANCE_RESET_FAILED`. See [`crate::socket::world::WorldSession::reset_instances`].
    ResetInstances,
    /// **Ask where the body is** — `MSG_CORPSE_QUERY`, bodiless. The reply comes
    /// back under the same opcode.
    CorpseQuery,
    /// **Stand up on the body** — `CMSG_RECLAIM_CORPSE`, the game's own
    /// `RetrieveCorpse`. Refused silently by the server if the reclaim delay has
    /// not elapsed or the ghost is further than
    /// [`crate::play::death::CORPSE_RECLAIM_RADIUS`].
    ReclaimCorpse,
    /// Accept or decline a resurrection somebody offered —
    /// `CMSG_RESURRECT_RESPONSE`. The guid is **theirs**, echoed from the offer.
    ResurrectResponse { caster: u64, accept: bool },
    /// **Answer a duel** — `CMSG_DUEL_ACCEPTED` or `CMSG_DUEL_CANCELLED`, both
    /// naming the flag. Cancelling is also how a duel in progress is forfeited
    /// (`/forfeit`), which the server tells apart by whether it has started.
    DuelAnswer { arbiter: u64, accept: bool },
    /// **Go to the summoner** — `CMSG_SUMMON_RESPONSE`. There is no decline on
    /// the wire; see [`crate::play::summon`].
    SummonResponse { summoner: u64 },
    /// **`/played`** — `CMSG_PLAYED_TIME`, bodiless.
    RequestPlayedTime,
    /// Take the spirit healer's offer — `CMSG_SPIRIT_HEALER_ACTIVATE`, at 25%
    /// durability on everything and ten minutes of sickness.
    SpiritHealerActivate { healer: u64 },
    /// **Load the ammo slot with an entry, or 0 to unload it** —
    /// `CMSG_SET_AMMO`. Not a move: see [`crate::play::items::set_ammo_body`].
    SetAmmo { entry: u32 },
    /// **Spend a talent point** — `CMSG_LEARN_TALENT`, by talent id and the
    /// zero-based rank being asked for.
    ///
    /// Which rank that is, and the one refusal the client makes for itself, are
    /// `vale_assets::tables::talent::TalentTree::learn`'s — a rule over
    /// `Talent.dbc` with no socket in it. Nothing waits for an answer, because
    /// there is none: see [`crate::play::talents`].
    LearnTalent { talent_id: u32, rank: u32 },
    /// **Arm or disarm the recent-packet capture** — see
    /// [`crate::socket::world::Capture`].
    ///
    /// A command rather than a shared flag because the ring lives on the
    /// session's own socket, and because arming clears it: two threads doing
    /// that at once would be two captures interleaved into one buffer.
    CaptureTraffic(bool),
    /// Leave the world and close the socket.
    Shutdown,
}

/// Everything about the session that is not per-entity state.
#[derive(Debug, Clone, Default)]
pub struct SessionStatus {
    pub character: String,
    pub map_id: u32,
    /// The client's own idea of where it is — updated every tick, unlike the
    /// player entity's position, which only moves when the server says so.
    pub position: Position,
    pub controls: Controls,
    pub speeds: Speeds,
    /// **The movement block the local simulation is running on**, verbatim.
    ///
    /// [`SessionStatus::position`], `moving`, `speed` and `direction` are all
    /// readings taken off this; it is published whole as well because a
    /// renderer that wants to *continue* the simulation between steps — rather
    /// than merely describe it — needs the same inputs [`Mover::advance`] has.
    /// See `vale_protocol::state::movement::strode`, which is the expression both
    /// then integrate.
    pub movement: MovementInfo,
    /// Whether the local simulation currently has the character moving, and how
    /// fast in yards per second — the two numbers the anticheat judges us by,
    /// and the ones a renderer needs to pick Stand, Walk or Run.
    ///
    /// The player is the one entity [`ObjectManager::advance`] does not touch:
    /// [`Mover`] owns its position, so [`crate::state::objects::Entity::is_moving`] has
    /// nothing to read for it and the state has to come from here.
    pub moving: bool,
    pub speed: f32,
    /// **The server is driving this character, and which way** — a Charge, a
    /// taxi flight, and anything else that reaches [`Mover::ride`]. Yards per
    /// second, in the world's own axes.
    ///
    /// A reader that *continues* the simulation between steps needs this the way
    /// it needs [`Self::restraint`]: the keys have no say while a ride runs, so
    /// a prediction that strode from them would fight the ride sixty times a
    /// second. The velocity rides along because none of it can be recovered from
    /// the block — the speed is not one of the six in [`Speeds`], and the
    /// orientation is a *bearing*, which is flat. A flight walked from those two
    /// holds its altitude between readings and steps down to each new one, which
    /// is a 40 Hz sawtooth on the one axis nothing else in the frame is moving
    /// along. See [`crate::state::movement::Spline::velocity`].
    pub riding: Option<[f32; 3]>,
    /// **The moving platform this character is standing on**, or `None` for the
    /// overwhelming majority of a session — see
    /// [`crate::state::movement::Ferry`].
    ///
    /// Published for the same reason [`Self::riding`] is, and the failure it
    /// closes is the larger of the two. A reader that continues the simulation
    /// between steps draws the local player at `base + velocity * age`, and a
    /// passenger's velocity is not in the block at all: the deck's travel
    /// arrives as a *correction* to the position on each tick, so the drawn
    /// character stands still for a whole step and then jumps by however far
    /// the platform moved. On the Deeprun Tram that is 0.43 yards forty times a
    /// second, and since the camera is framed on the same number the whole
    /// world does it.
    ///
    /// What a reader wants is the pair — the offset and the placement it was
    /// taken against — so it can put the passenger back onto whatever reading
    /// of the platform it is *drawing* this frame. See `vale-client`'s
    /// `world::predict`.
    pub ferry: Option<crate::state::movement::Ferry>,
    /// Off the ground, and whether that began with a jump rather than with a
    /// step off an edge. The renderer needs both: `JumpStart`/`Jump`/`JumpEnd`
    /// and `Fall` are four different sequences, and only the mover knows which
    /// arc this is — the player is the one entity whose movement flags never
    /// come back from the server.
    pub airborne: bool,
    pub jumping: bool,
    /// **Whose body every other field here is about** — the mover, which is the
    /// character's own guid except while a possess lasts.
    ///
    /// [`Self::position`], [`Self::controls`], [`Self::movement`],
    /// [`Self::moving`] and the rest are the *mover's*, not the character's, so
    /// a reader that continues the simulation has to know which entity to
    /// continue. `crate::world::session::place_entities` draws it ahead of the
    /// simulation and leaves the character to the ordinary interpolation, which
    /// is what makes a possessed body drivable and the possessing one stand
    /// still.
    ///
    /// **Zero means nobody**: feared or confused, with the server walking the
    /// body itself. See [`crate::socket::handler::LocalState::mover_guid`].
    pub mover: u64,
    /// **What the server has stopped this character doing** — a stun and death,
    /// the two halves that are not in [`SessionStatus::movement`] because
    /// neither is a movement flag. A renderer that *continues* the simulation
    /// needs them for the same reason it needs the block itself: without them
    /// its prediction turns a character the mover is holding still, which is
    /// the drawn body spinning on the spot while the confirmed one does not
    /// move. See `movement::Restraint`.
    pub restraint: crate::state::movement::Restraint,
    /// The world's date and hour, already run forward from what the server said
    /// at login. `None` until `SMSG_LOGIN_SETTIMESPEED` arrives, which it does
    /// in the login burst — see [`crate::play::time`].
    ///
    /// **What reads it is the light chain**, which is indexed by time of day:
    /// the sun, the sky and the fog are all different at dawn.
    pub game_time: Option<crate::play::time::GameTime>,
    /// **What the sky is doing** — the last `SMSG_WEATHER`, copied off the
    /// object manager with the clock. See [`crate::play::weather`].
    pub weather: Option<crate::play::weather::Weather>,
    /// Simulated milliseconds since the thread started — the clock everything
    /// in [`SessionStatus::position`] and in the [`ObjectManager`] was advanced
    /// by, and the only honest answer to "how much world time do these
    /// positions cover?".
    ///
    /// **A reader needs this to interpolate.** The renderer draws at 60–130 Hz
    /// against a simulation stepping at ~40, and the two clocks are unrelated:
    /// polling on a timer of its own means some polls catch a tick that has not
    /// happened yet and the next catches two, so an interpolator keyed on *poll*
    /// intervals alternately stalls and surges — a beat between two free-running
    /// clocks, which on screen is the whole world jittering back and forth. This
    /// is monotonic and advances by exactly the time the positions moved
    /// through, so a reader can key on it instead: unchanged means there is
    /// nothing new to draw, and the difference is how long the step covered.
    pub world_ms: u64,
    /// The **real** instant the simulation stood at [`Self::world_ms`].
    ///
    /// `world_ms` says how much world time a reading covers; this says when
    /// that reading happened, and a reader that continues the simulation
    /// forward needs the second as much as the first. Without it the only
    /// timestamp available is the moment the reading was *noticed*, which is a
    /// whole frame later at worst and a different amount later every time —
    /// so a prediction anchored on it advances by the interval between
    /// *notices* while the base advances by the interval between *steps*, and
    /// the difference comes out as the character alternately hurrying and
    /// dawdling several times a second. See `vale-client`'s
    /// `world::predict`, which is the reader this exists for.
    ///
    /// `None` before the first step. Both clocks are wall-clock, so the age of
    /// a reading is `taken.elapsed()` and nothing has to be synchronised.
    pub taken: Option<Instant>,
    /// **Ticks whose socket-draining slice ran out**, against the total number
    /// of ticks.
    ///
    /// The other way the world can arrive late, and the one that costs
    /// *unbounded* time rather than a quarter of a second at a go:
    /// [`SessionLoop::read_socket`] drains for at most [`READ_SLICE`] of every
    /// [`TICK`], so a stream arriving faster than that leaves its backlog in
    /// the kernel and this client reads a world that is seconds old — with
    /// every number on screen consistent and simply late. A localhost server
    /// never produces it and a busy realm might, which is why it is a counter
    /// rather than a thing anyone could have noticed. Read as a *proportion* of
    /// [`Self::ticks`]; see [`SessionLoop::read_socket`] for the one class it
    /// over-counts.
    pub read_slice_overruns: u32,
    pub ticks: u32,
    /// **Steps that took longer than [`MAX_STEP`]**, and the worst of them in
    /// milliseconds.
    ///
    /// The session thread means to run every 25 ms and is not always allowed
    /// to: a login burst, an archive read, the renderer holding the world lock
    /// while it uploads a tile. Each stall is time the server spent moving
    /// everything in view and this client spent doing nothing about it, and
    /// until now nothing counted them — the only symptom was creatures drawn
    /// somewhere they no longer are, which reads as a network fault and is not
    /// one. Zero is the ordinary state; a session that accumulates them is one
    /// whose world is being starved, and the number to look at next is what the
    /// thread was doing.
    pub stalls: u32,
    pub worst_stall_ms: u32,
    /// **Steps in which the deck under the character moved further than a
    /// transport can travel**, and the worst of them in yards.
    ///
    /// A passenger is placed by composing their offset with wherever the client
    /// says the platform is, so a platform placement that is wrong by a mile
    /// puts the character a mile away — silently, because every count on the
    /// panel still reads correctly and the position is a perfectly valid one.
    /// Two rounds went on that: the deck's interpolation history survived a map
    /// change, `world::motion` answered a point between two continents for a
    /// frame, and the carry took the character there.
    ///
    /// **A count of zero is not the only passing state.** A transport really
    /// does jump: two of the nine routes teleport 13,700 yards without changing
    /// map and no packet states it, so riding one of those puts exactly one
    /// here per cycle and that is the feature working. What this separates is
    /// *a jump* from *a jump nobody can account for* — the number to read is
    /// whether it happened at a moment the route says it should have.
    pub platform_jumps: u32,
    pub worst_platform_jump: f32,
    /// False before the login burst has produced a player, and after the socket
    /// closes.
    pub in_world: bool,
    /// Set once, when the thread stops for a reason other than being asked to.
    pub error: Option<String>,
    pub packets: u64,
    /// `MSG_MOVE_*` packets sent.
    pub movement_sent: u64,
    /// `SMSG_FORCE_*_SPEED_CHANGE` recorded and acknowledged.
    ///
    /// **An instrument for the thing it was easiest to get wrong.** These used
    /// to be intercepted by this loop, acknowledged, and dropped — so one about
    /// another unit never reached the world state and that player went on being
    /// dead-reckoned at their old speed. It is not an error to see these; what
    /// the count is for is answering "did that path ever run?", which a unit
    /// test can pin and a live session could not previously show at all.
    pub speed_changes: u32,
    /// …and the twelve about a unit we do **not** control:
    /// `MSG_MOVE_SET_*_SPEED` and `SMSG_SPLINE_SET_*`.
    ///
    /// These are the ones that matter in a live session, because the forced
    /// family only ever fires for one character and this one fires for every
    /// mount, sprint, daze and enrage in sight. **Zero in a session where
    /// anybody mounted means the family is being dropped again**, which is what
    /// it was doing before it had a counter: every one of the twelve went into
    /// the unhandled bucket, and the visible consequence was somebody else's
    /// character sliding back and forth twice a second.
    pub speed_broadcasts: u32,
    /// …and the twelve `SMSG_SPLINE_MOVE_*` about a unit no player is moving.
    ///
    /// The creature half of the same subject: root, water walking, feather
    /// fall, hover, walk/run mode. Rarer than the speeds by an order of
    /// magnitude — a fight with a root in it, a patrol told to walk — so a zero
    /// here is only evidence when one of those is known to have happened.
    pub spline_flag_changes: u32,
    /// **Sounds and visuals the server asked for outright** — the three
    /// `SMSG_PLAY_*` sound opcodes, and the two that put a `SpellVisualKit` on
    /// a unit. Nothing else on the wire implies either family, so a zero in a
    /// session where anything scripted happened, or where anybody sat down to
    /// eat, means they are being dropped.
    pub pushed_sounds: u32,
    pub pushed_visuals: u32,
    /// **`SMSG_COMPRESSED_MOVES` bags, and the packets taken out of them** —
    /// see [`crate::socket::handler::PumpStats::bagged_moves`], which is where
    /// the reason this needs a counter is written down. Zero says nothing; a
    /// large number says most of the world's movement is arriving inside one
    /// opcode, which is the state a client that ignores it desyncs in.
    pub bagged_moves: u32,
    pub bagged_packets: u32,
    /// **`CMSG_AREATRIGGER` sent this session** — see
    /// [`crate::socket::handler::PumpStats::area_triggers`], which is where the reason
    /// this needs a counter at all is written down.
    pub area_triggers: u32,
    /// **`CMSG_MOVE_SPLINE_DONE` sent this session** — see
    /// [`crate::socket::handler::PumpStats::rides`], which is where the reason this
    /// needs a counter at all is written down.
    pub rides: u32,
    /// `SMSG_FORCE_MOVE_ROOT` and its five siblings, plus knockbacks —
    /// acknowledged. Zero for a whole session is normal; what is *not* normal is
    /// a kick with `PendingAckDelay` beside a zero here, which is what this
    /// number exists to make visible.
    pub flag_changes: u32,
    /// The three packets that drive an animation and leave no other trace.
    ///
    /// **An instrument for the same reason `speed_changes` is one.** A swing,
    /// an emote and a cast are events: they are folded into a counter on an
    /// entity and then they are gone, so an emote that arrived and was dropped
    /// and an emote that never arrived are indistinguishable from the world
    /// state. Since nothing about animating the wrong thing produces an error,
    /// "did that path ever run?" is a question worth being able to answer.
    pub attacks: u32,
    pub emotes: u32,
    pub casts: u32,
    /// …and the fourth, which is the same argument taken one step further:
    /// `SMSG_AI_REACTION` drives **only** a sound, so it leaves no trace in the
    /// world state at all. Zero in a session where anything attacked you means
    /// the aggro barks are being dropped, and nothing on screen would show it.
    pub ai_reactions: u32,
    /// **Who the server thinks we are swinging at**, and the only statement it
    /// makes about our own auto-attack: `SMSG_ATTACKSTART` sets it and
    /// `SMSG_ATTACKSTOP` clears it. A pressed Attack button that never lights
    /// up is a swing the server refused — and it will have said which of the
    /// five reasons through [`LiveSession::take_events`].
    pub attacking: Option<u64>,
    /// Bumped whenever the spellbook or the action bar changes, so a reader can
    /// rebuild the bar on a difference rather than every frame.
    pub spellbook_version: u32,
    /// The two counters the login burst is judged by. **Zero spells known is a
    /// failure with no other symptom** — the packet is sent once and never
    /// restated, so the bar is simply empty for the whole session.
    pub spells_known: u32,
    pub action_buttons: u32,
    /// `SMSG_CAST_RESULT` seen, and the five swing refusals. Both are events
    /// that leave no other trace, which is the same argument `attacks` and
    /// `emotes` above are here for.
    pub cast_results: u32,
    pub attack_refusals: u32,
    /// Round-trip time of the last `CMSG_PING`.
    pub latency_ms: u32,
    /// Parse warnings, capped — see [`PumpStats`].
    pub warnings: Vec<String>,
    /// Opcodes received but not handled, with counts.
    pub unhandled: BTreeMap<String, u32>,
    /// **The wire itself** — bytes and packets each way, and every opcode's
    /// share of both. See [`Traffic`].
    ///
    /// Behind an `Arc` and rebuilt only every [`TRAFFIC_INTERVAL`], because the
    /// renderer clones the whole of this struct once a frame: the two maps
    /// inside run to a hundred-odd `String` keys between them, and copying them
    /// a hundred times a second for a window that is shut is the trade
    /// [`LiveSession::world_ms`] exists to avoid. Cloning the status is an
    /// atomic increment either way.
    pub traffic: Arc<Traffic>,
    /// **The last few hundred packets, whole**, while the capture is armed —
    /// see [`crate::socket::world::Capture`].
    ///
    /// Behind an `Arc` for the reason [`Self::traffic`] is, and rebuilt on the
    /// same interval. Disarming leaves the last snapshot in place, so the
    /// packets that preceded a fault can still be read after the capture has
    /// been stopped.
    pub capture: Arc<crate::socket::world::CaptureSnapshot>,
}

/// A handle to the running session. Dropping it shuts the thread down.
pub struct LiveSession {
    commands: Sender<Command>,
    world: Arc<Mutex<ObjectManager>>,
    status: Arc<Mutex<SessionStatus>>,
    thread: Option<JoinHandle<()>>,
    /// **Where the loop leaves its socket when it stops cleanly**, so a logout
    /// can go back to character select on the connection it came in on.
    ///
    /// The thread owns the `WorldSession` for the life of the session — one
    /// ordered stream with a stateful header cipher cannot be shared — so the
    /// only moment it can change hands is the one after `run` returns. Filled
    /// there, and only for a stop the loop was *asked* for: a socket that failed
    /// a write is not one to re-enumerate on. See [`LiveSession::reclaim`].
    handback: Arc<Mutex<Option<WorldSession>>>,
}

impl LiveSession {
    /// Take over an authenticated session: enter the world as `character` and
    /// keep running until told to stop.
    ///
    /// Returns immediately. Use [`LiveSession::wait_until_in_world`] when the
    /// caller needs to report login failures synchronously.
    /// …and `caches` is the on-disk query answers — see [`crate::play::wdb`].
    /// [`Caches::none`] for a caller with nowhere to put them; the session is
    /// identical either way except that every window naming something it has
    /// never met is blank for its first opening.
    pub fn spawn(
        session: WorldSession,
        character: CharListEntry,
        ground: Option<GroundHeight>,
        triggers: Option<TriggerTable>,
        caches: Caches,
    ) -> LiveSession {
        let world = Arc::new(Mutex::new(ObjectManager::new()));
        // Seeded before the thread starts, so the first `LOOT_OPENED` of the
        // session already has names in it: the interface reads a row's name
        // once and there is no event to raise when a late template lands. The
        // bodies go through the socket's own parsers — see `wdb`'s module doc.
        lock(&world).seed_cache(&caches);
        // **The player's own name never crosses the wire again after the
        // character list**, and `unresolved_player_guids` deliberately skips
        // the local GUID — so an unseeded world names its own player
        // "Player 450" for the whole session, which is exactly what the
        // game's own `PlayerFrame` then displays. The character list is the
        // authority the real client uses too.
        lock(&world).players.insert(
            character.guid,
            crate::state::query::PlayerInfo {
                guid: character.guid,
                name: character.name.clone(),
                race: character.race as u32,
                gender: character.gender as u32,
                class: character.class as u32,
            },
        );
        let start_position = Position {
            x: character.x,
            y: character.y,
            z: character.z,
            orientation: 0.0,
        };
        let status = Arc::new(Mutex::new(SessionStatus {
            character: character.name.clone(),
            map_id: character.map,
            position: start_position,
            ..Default::default()
        }));
        let (tx, rx) = mpsc::channel();

        let handback: Arc<Mutex<Option<WorldSession>>> = Arc::new(Mutex::new(None));

        let thread = {
            let world = Arc::clone(&world);
            let status = Arc::clone(&status);
            let handback = Arc::clone(&handback);
            thread::Builder::new()
                .name("world-session".into())
                .spawn(move || {
                    let mut session_loop = SessionLoop::new(
                        session,
                        character,
                        ground,
                        triggers,
                        world,
                        Arc::clone(&status),
                        rx,
                        caches,
                    );
                    let outcome = session_loop.run();
                    let mut st = lock(&status);
                    st.in_world = false;
                    match outcome {
                        // **A clean stop leaves the socket behind.** `run`
                        // returns `Ok` only when the command loop was told to
                        // stop — a shutdown or a dropped handle — so the stream
                        // is intact and framed at a packet boundary. Anything
                        // else is an IO or protocol failure and the connection
                        // goes with the thread, which is why this is a `match`
                        // and not an unconditional move.
                        Ok(()) => *lock(&handback) = Some(session_loop.session),
                        Err(e) => st.error = Some(e),
                    }
                })
                .expect("spawning the world session thread")
        };

        LiveSession {
            commands: tx,
            world,
            status,
            thread: Some(thread),
            handback,
        }
    }

    /// The world state. Lock it briefly; the session thread wants it too.
    pub fn world(&self) -> &Arc<Mutex<ObjectManager>> {
        &self.world
    }

    pub fn status(&self) -> SessionStatus {
        lock(&self.status).clone()
    }

    /// [`SessionStatus::world_ms`] alone, without cloning the rest.
    ///
    /// A renderer asks this **every frame** to find out whether there is
    /// anything new, so it has to be cheaper than the full status — which
    /// carries a `Vec` of warnings and a map of unhandled opcodes, and cloning
    /// those a hundred times a second to learn nothing would be a poor trade.
    pub fn world_ms(&self) -> u64 {
        lock(&self.status).world_ms
    }

    /// [`SessionStatus::game_time`] alone, on the same terms and for the same
    /// reason: the renderer asks for the hour every frame to light the world,
    /// and cloning the warnings and the unhandled-opcode map to learn it would
    /// be the trade `world_ms` exists to avoid.
    pub fn game_time(&self) -> Option<crate::play::time::GameTime> {
        lock(&self.status).game_time
    }

    /// [`SessionStatus::spellbook_version`] and [`SessionStatus::attacking`],
    /// on exactly the same terms as the two above.
    ///
    /// **Both are read every frame** — the first to decide whether the action bar
    /// needs rebuilding, the second to light the Attack button — and both are one
    /// word. Going through [`Self::status`] for them would clone a `Vec` of
    /// warnings and a `BTreeMap` of unhandled opcodes a hundred times a second to
    /// learn nothing, which is the trade `world_ms` was split out to avoid.
    pub fn spellbook_version(&self) -> u32 {
        lock(&self.status).spellbook_version
    }

    pub fn attacking(&self) -> Option<u64> {
        lock(&self.status).attacking
    }

    pub fn send(&self, command: Command) {
        // A closed channel means the thread has already stopped; the status
        // carries the reason, so there is nothing useful to do with the error.
        let _ = self.commands.send(command);
    }

    pub fn set_controls(&self, controls: Controls) {
        self.send(Command::Controls(controls));
    }

    pub fn face(&self, orientation: f32) {
        self.send(Command::Face(orientation));
    }

    /// The camera's pitch, which is the body's while swimming — see
    /// [`Command::Pitch`].
    pub fn pitch(&self, pitch: f32) {
        self.send(Command::Pitch(pitch));
    }

    pub fn jump(&self) {
        self.send(Command::Jump);
    }

    /// Attack a unit, or `None` to stop attacking.
    pub fn attack(&self, guid: Option<u64>) {
        self.send(Command::Attack(guid));
    }

    /// Tell the server which unit is selected, or `None` for nothing.
    pub fn target(&self, guid: Option<u64>) {
        self.send(Command::Target(guid));
    }

    /// **Ask to cast a spell at an already-resolved target** — see
    /// [`Command::Cast`].
    ///
    /// **An ask and nothing more, which is the whole of the rule.** Nothing is
    /// drawn here, no bar is started and no counter moves: a cast happens when
    /// `SMSG_SPELL_START` says it has. That is 5875's own shape — the press path
    /// runs its local refusals, records a pending cast, starts the
    /// **global cooldown** (which is the client's, and vmangos says so in a
    /// comment) and sends — and it is what stops a refused cast from playing a
    /// wind-up and a release that never happened.
    ///
    /// So this one method now covers the three presses that used to be three
    /// methods: an ordinary cast, a **next-swing** ability the server holds until
    /// the weapon lands, and a ranged **auto-repeat** whose first shot leaves on
    /// the ranged attack timer. They differ in when the server answers, and in
    /// nothing this function does.
    pub fn cast(&self, spell_id: u32, target: crate::play::spells::CastTarget) {
        self.send(Command::Cast { spell_id, target });
    }

    /// **Stop casting — and stop drawing it**, on the same terms [`Self::cast`]
    /// started it.
    ///
    /// The server acknowledges `CMSG_CANCEL_CAST` with nothing at all, so the
    /// local half is the whole of it: without this, pressing Escape mid-Fireball
    /// cleared the cast bar and left the character holding the wind-up until its
    /// own timer ran out. See [`ObjectManager::apply_cast_cancelled`].
    pub fn cancel_cast(&self, spell_id: u32) {
        {
            let mut world = lock(&self.world);
            if let Some(guid) = world.player_guid {
                world.apply_cast_cancelled(guid, spell_id);
            }
        }
        self.send(Command::CancelCast(spell_id));
    }

    /// **Stop the ranged auto-repeat** — see
    /// [`WorldSession::cancel_auto_repeat`].
    ///
    /// Nothing local, deliberately: the server answers with
    /// `SMSG_CANCEL_AUTO_REPEAT` and the caller's own state comes off *that*,
    /// so the press and every other way the loop can end are one code path.
    /// Predicting it here would leave the two disagreeing on the one case a
    /// prediction cannot cover — the server stopping it on its own.
    pub fn cancel_auto_repeat(&self) {
        self.send(Command::CancelAutoRepeat);
    }

    /// Right-click a buff off — see [`Command::CancelAura`]. Nothing local
    /// changes; the icon goes when the server empties the slot.
    pub fn cancel_aura(&self, spell_id: u32) {
        self.send(Command::CancelAura(spell_id));
    }

    /// **Right-click an item** — see [`Command::UseItem`]. Nothing local
    /// changes: the stack shrinks, the garment moves and the cooldown starts
    /// when the server's own update block says so, which is one round trip and
    /// is what the real client shows too.
    pub fn use_item(
        &self,
        bag: u8,
        slot: u8,
        spell_index: Option<u8>,
        target: crate::play::spells::CastTarget,
    ) {
        self.send(Command::UseItem {
            bag,
            slot,
            spell_index,
            target,
        });
    }

    /// **Right-click something that holds loot** — see [`Command::OpenItem`].
    /// Nothing local changes; the window arrives as an ordinary
    /// `SMSG_LOOT_RESPONSE` whose guid is the item's.
    pub fn open_item(&self, bag: u8, slot: u8) {
        self.send(Command::OpenItem { bag, slot });
    }

    /// **Open the window on a body** — see [`crate::play::loot`]. Nothing is shown
    /// until `SMSG_LOOT_RESPONSE` comes back, which is also how the server
    /// refuses.
    pub fn loot(&self, guid: u64) {
        self.send(Command::Loot(LootVerb::Open(guid)));
    }

    /// …take one row, by the **server's** index. See
    /// [`crate::play::loot::Loot::at`], which is the one place that index and the
    /// interface's row are crossed.
    pub fn loot_item(&self, index: u8) {
        self.send(Command::Loot(LootVerb::Take(index)));
    }

    /// …take the coins.
    pub fn loot_money(&self) {
        self.send(Command::Loot(LootVerb::TakeMoney));
    }

    /// …and close it. **The window does not shut here** — see
    /// [`crate::play::loot`], where the reason is.
    pub fn loot_release(&self, guid: u64) {
        self.send(Command::Loot(LootVerb::Release(guid)));
    }

    /// …and vote in a group roll. **The window is not what this is about**: a
    /// roll is named by the body and the slot, and the `rollID` the interface
    /// passes is a counter [`crate::play::lootroll`] explains.
    pub fn loot_roll(&self, guid: u64, item_slot: u32, vote: crate::play::lootroll::RollVote) {
        self.send(Command::Loot(LootVerb::Roll {
            guid,
            item_slot,
            vote,
        }));
    }

    /// **The quest conversation's eight verbs** — see [`QuestVerb`].
    pub fn quest(&self, verb: QuestVerb) {
        self.send(Command::Quest(verb));
    }

    /// **Right-click a door, a chest, an ore vein or a mailbox** — see
    /// [`Command::UseObject`], and `vale_assets::look::object` for the rule
    /// that decides a game object is worth clicking in the first place.
    pub fn use_object(&self, guid: u64) {
        self.send(Command::UseObject(guid));
    }

    /// **…and the gossip and vendor seven** — see [`NpcVerb`].
    /// **Where the hearthstone returns the character to**, off the world.
    ///
    /// Through the world lock rather than the status, because
    /// `SMSG_BINDPOINTUPDATE` is state the handler writes to
    /// [`crate::state::objects::ObjectManager`] and nothing on the status
    /// mirrors it. Asked once per session by the renderer rather than every
    /// frame — see `game::npc::binder`, which explains why it is polled at all
    /// instead of delivered as an event.
    pub fn bind_point(&self) -> Option<crate::play::bindpoint::BindPoint> {
        lock(&self.world).bind_point
    }

    pub fn npc(&self, verb: NpcVerb) {
        self.send(Command::Npc(verb));
    }

    /// **…and the pet's nine** — see [`PetVerb`].
    pub fn pet(&self, verb: PetVerb) {
        self.send(Command::Pet(verb));
    }

    /// **Arm or disarm the recent-packet capture** — see
    /// [`crate::socket::world::Capture`]. Nothing crosses the wire; the ring
    /// lives on this side of the socket.
    pub fn capture_traffic(&self, on: bool) {
        self.send(Command::CaptureTraffic(on));
    }

    /// **…and the party's six** — see [`PartyVerb`].
    pub fn party(&self, verb: PartyVerb) {
        self.send(Command::Party(verb));
    }

    /// **…and the flight master's three** — see [`TaxiVerb`].
    pub fn taxi(&self, verb: TaxiVerb) {
        self.send(Command::Taxi(verb));
    }

    /// **…and the reputation panel's three** — see [`ReputationVerb`].
    pub fn reputation(&self, verb: ReputationVerb) {
        self.send(Command::Reputation(verb));
    }

    /// **Ask about, add to or remove from the two lists** — see [`SocialVerb`].
    pub fn social(&self, verb: SocialVerb) {
        self.send(Command::Social(verb));
    }

    /// **Join, leave, list or moderate a chat channel** — see [`ChannelVerb`].
    pub fn channel(&self, verb: ChannelVerb) {
        self.send(Command::Channel(verb));
    }

    /// **…and the trade window's ten** — see [`TradeVerb`].
    pub fn trade(&self, verb: TradeVerb) {
        self.send(Command::Trade(verb));
    }

    /// **…and the mailbox's nine** — see [`MailVerb`].
    pub fn mail(&self, verb: MailVerb) {
        self.send(Command::Mail(verb));
    }

    /// **Put an item down somewhere else** — see [`Command::MoveItem`], which
    /// carries the whole of why five opcodes are one command.
    pub fn move_item(
        &self,
        src_bag: u8,
        src_slot: u8,
        dst: Option<(u8, u8)>,
        count: Option<u8>,
    ) {
        self.send(Command::MoveItem {
            src_bag,
            src_slot,
            dst,
            count,
        });
    }

    /// …and wear it, wherever it fits — see [`Command::EquipItem`].
    pub fn equip_item(&self, bag: u8, slot: u8) {
        self.send(Command::EquipItem { bag, slot });
    }

    /// …and across the bank counter, either way — see [`Command::BankItem`].
    pub fn bank_item(&self, bag: u8, slot: u8) {
        self.send(Command::BankItem { bag, slot });
    }

    /// **Set — or clear — one action button, locally and then on the wire.**
    ///
    /// The order is the opposite of every item verb's and it is the client's
    /// own: it writes `actionButtons[slot]` and *then* builds the
    /// packet, because the bar belongs to the client and the server is only
    /// being told. So the world's copy is updated here, under the same lock
    /// every other reader takes, rather than waiting for an echo that never
    /// comes — see [`ObjectManager::set_action_button`] for why forgetting that
    /// half is invisible until the next rebuild.
    ///
    /// `kind` of `None` empties the slot, which is a zero word on the wire.
    pub fn set_action_button(&self, slot: u8, action: u32, kind: Option<u8>) {
        lock(&self.world).set_action_button(slot, action, kind);
        let packed = kind.map_or(0, |kind| crate::play::spells::ActionButton::packed(action, kind));
        self.send(Command::SetActionButton { slot, packed });
    }

    /// **Remember which extra bars are on**, and nothing else — see
    /// [`Command::SetActionBarToggles`].
    ///
    /// Deliberately *not* the shape of [`Self::set_action_button`] above: no
    /// local write, because the reference makes none (it packs and sends) and
    /// because the field is `PRIVATE` and comes straight back. A
    /// local write here would be a second copy of a number the server is about
    /// to state.
    pub fn set_actionbar_toggles(&self, mask: u8) {
        self.send(Command::SetActionBarToggles(mask));
    }

    /// **How long our own buffs have left**, as `(slot, seconds since the
    /// reading, remaining ms at that moment, seq)`.
    ///
    /// Read rather than drained, on the terms
    /// [`ObjectManager::aura_durations`] states. The elapsed time is measured
    /// here, under the lock, because the reader's clock is the frame clock and
    /// this crate's is `Instant` — one subtraction on this side means the two
    /// bases never have to be reconciled.
    pub fn aura_durations(&self) -> Vec<(u8, f32, u32, u32)> {
        lock(&self.world)
            .aura_durations()
            .map(|(slot, held)| {
                (
                    slot,
                    held.received.elapsed().as_secs_f32(),
                    held.remaining_ms,
                    held.seq,
                )
            })
            .collect()
    }

    /// **Ask the server to let this character go**, or take the ask back.
    ///
    /// Nothing local changes and nothing is predicted: the world is left when
    /// `SMSG_LOGOUT_COMPLETE` arrives on [`Self::take_events`], which for a
    /// character standing in a field is twenty seconds later and for one in an
    /// inn is immediate. See [`crate::play::logout`].
    pub fn logout(&self, leaving: bool) {
        self.send(Command::Logout { leaving });
    }

    /// **Release the spirit.** `RepopMe()`, the game's own name for it.
    ///
    /// Nothing local changes and nothing is predicted, for the same reason
    /// [`Self::logout`] predicts nothing: the answer is the server's, and it
    /// arrives as `PLAYER_FLAGS_GHOST` in the next values block rather than as a
    /// packet of its own. See [`crate::play::death`].
    pub fn repop(&self) {
        self.send(Command::Repop);
    }

    /// **Reset every instance this character is saved to** — the self menu's
    /// `RESET_INSTANCES` row, through the popup that confirms it.
    ///
    /// Nothing local changes and there is nothing to predict: the server acts
    /// and says nothing unless it refuses. See [`Command::ResetInstances`].
    pub fn reset_instances(&self) {
        self.send(Command::ResetInstances);
    }

    /// **Ask where the body is.** The reply raises
    /// [`crate::play::spells::PlayerEvent::CorpseLocated`].
    pub fn corpse_query(&self) {
        self.send(Command::CorpseQuery);
    }

    /// **Stand up on the body.** `RetrieveCorpse()`.
    pub fn reclaim_corpse(&self) {
        self.send(Command::ReclaimCorpse);
    }

    /// Answer a resurrection offer. `caster` is the guid the offer arrived with
    /// — the server compares it and drops a mismatch silently, and a **zero**
    /// guid is logged as an "Instant resurrect hack" before anything else is
    /// read (`HandleResurrectResponseOpcode`).
    pub fn resurrect_response(&self, caster: u64, accept: bool) {
        self.send(Command::ResurrectResponse { caster, accept });
    }

    /// Accept or decline a duel — see [`Command::DuelAnswer`].
    pub fn duel_answer(&self, arbiter: u64, accept: bool) {
        self.send(Command::DuelAnswer { arbiter, accept });
    }

    /// Accept a summon — see [`Command::SummonResponse`].
    pub fn summon_response(&self, summoner: u64) {
        self.send(Command::SummonResponse { summoner });
    }

    /// Ask for the played time — see [`Command::RequestPlayedTime`].
    pub fn request_played_time(&self) {
        self.send(Command::RequestPlayedTime);
    }

    /// Take the spirit healer's offer. `AcceptXPLoss()`.
    pub fn spirit_healer_activate(&self, healer: u64) {
        self.send(Command::SpiritHealerActivate { healer });
    }

    /// Load the ammo slot, or unload it with 0 — see [`Command::SetAmmo`].
    pub fn set_ammo(&self, entry: u32) {
        self.send(Command::SetAmmo { entry });
    }

    /// Volunteer a sheath change — see [`Command::SetSheathed`]. Clamped here
    /// rather than at the send, because vmangos drops an out-of-range value
    /// **silently** and the two ends would then disagree for the session.
    pub fn set_sheathed(&self, state: u8) {
        self.send(Command::SetSheathed(
            state.min(crate::socket::world::MAX_SHEATH_STATE - 1),
        ));
    }

    /// Take the answers to our own presses that have arrived since the last
    /// call — a cast refused, a swing out of range, a cooldown started.
    ///
    /// Draining rather than cloning, for the same reason [`Self::take_chat`]
    /// drains: each of these has to be acted on exactly once. See
    /// [`ObjectManager::take_events`].
    pub fn take_events(&self) -> Vec<crate::play::spells::PlayerEvent> {
        lock(&self.world).take_events()
    }

    /// Play a text emote: an `EmotesText.dbc` id, which the server turns into
    /// an `Emotes.dbc` id and broadcasts as `SMSG_EMOTE`.
    pub fn text_emote(&self, text_emote: u32, emote_num: u32, target: u64) {
        self.send(Command::TextEmote { text_emote, emote_num, target });
    }

    /// **Spend a talent point** — see [`Command::LearnTalent`].
    pub fn learn_talent(&self, talent_id: u32, rank: u32) {
        self.send(Command::LearnTalent { talent_id, rank });
    }

    /// Say something. A leading `.` makes it a GM command, which is the server's
    /// own convention rather than this client's — see [`crate::play::chat`].
    pub fn say(&self, kind: ChatType, target: Option<String>, text: String) {
        self.send(Command::Chat { kind, target, text });
    }

    /// Take the chat that has arrived since the last call.
    ///
    /// Draining rather than cloning, because a line has to be shown exactly
    /// once — see [`ObjectManager::take_chat`]. Sender names are resolved here,
    /// while the lock is held and the caches are to hand.
    pub fn take_chat(&self) -> Vec<(String, crate::play::chat::ChatMessage)> {
        // **Our own name is the one the object manager cannot supply.** A say is
        // echoed to its sender, so most of what this client hears is itself — and
        // `CMSG_NAME_QUERY` is never sent for the player (there is nothing to ask:
        // the name came from the character list), so `name_of` falls all the way
        // back to "Player 319". Taken before the world lock rather than inside
        // it, so the two are never held at once.
        let me = lock(&self.status).character.clone();
        let mut world = lock(&self.world);
        let player = world.player_guid;
        world
            .take_chat()
            .into_iter()
            .map(|m| {
                let who = match player {
                    Some(guid) if guid == m.sender && !me.is_empty() => me.clone(),
                    _ => world.chat_sender(&m),
                };
                (who, m)
            })
            .collect()
    }

    /// Is the session thread still running?
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Block until the character is in the world, or until `timeout`.
    ///
    /// Returns the reason it will never happen if the thread has already
    /// failed — that is the message worth showing a user, not "timed out".
    pub fn wait_until_in_world(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let status = self.status();
            if status.in_world {
                return Ok(());
            }
            if let Some(e) = status.error {
                return Err(e);
            }
            if Instant::now() >= deadline {
                return Err("timed out waiting to enter the world".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Ask the thread to stop, and wait for it.
    pub fn shutdown(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    /// **Stop, and take the world socket back** — `None` if the session ended
    /// for any reason other than being asked to.
    ///
    /// This is the one thing a `LiveSession` can be turned back into, and it
    /// exists for exactly one caller: **1.12 returns to character select on the
    /// same connection**, and `CMSG_PLAYER_LOGIN` is only answered on the
    /// connection the character list came from — so a client that closed the
    /// socket on `SMSG_LOGOUT_COMPLETE` had no route back but a fresh logon.
    ///
    /// Takes `self` because there is nothing left afterwards: the thread is
    /// joined and the socket has moved. **Do not call it while the character is
    /// still in the world** — the server keeps the session, but this client's
    /// whole world state goes with the handle.
    ///
    /// The socket comes back in whatever mode the loop left it, which is
    /// non-blocking; a caller that wants to wait for one reply should say so.
    /// See [`WorldSession::set_nonblocking`].
    pub fn reclaim(mut self) -> Option<WorldSession> {
        self.shutdown();
        lock(&self.handback).take()
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A poisoned lock means another thread panicked while holding it. This state
/// is a cache of the server's, so carrying on with it is strictly better than
/// taking the session down too.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The thread's own state. Split out so `run` reads as straight-line code
/// rather than a closure with a dozen captured variables.
struct SessionLoop {
    session: WorldSession,
    character: CharListEntry,
    ground: Option<GroundHeight>,
    /// The volumes the character reports standing in — see
    /// [`crate::play::areatrigger`]. Optional for the same reason [`Self::ground`] is:
    /// a session with no game data still runs, it just cannot enter a dungeon.
    triggers: Option<TriggerTable>,
    /// Which of them it is standing in now, and therefore which may not fire
    /// again. The client's own latch; see [`TriggerWatch`].
    trigger_watch: TriggerWatch,
    /// When the trigger check last ran. Its own clock at
    /// [`crate::play::areatrigger::CHECK_INTERVAL`] rather than the session tick,
    /// which is four times faster.
    last_trigger_check: Instant,
    world: Arc<Mutex<ObjectManager>>,
    status: Arc<Mutex<SessionStatus>>,
    commands: Receiver<Command>,

    /// The client's own player: the mover, the map it is on, the session clock.
    ///
    /// Held as one value rather than as loose fields because the packet handlers
    /// need exactly this much and nothing else — see [`crate::socket::handler`], where
    /// the teleports and the speed changes that used to be a second dispatch in
    /// this file now live.
    local: LocalState,
    /// Replies the last packet queued, reused rather than reallocated.
    replies: Replies,
    /// When the simulation was last advanced, so the next step can be the time
    /// that actually passed. See [`SessionLoop::step_ms`].
    last_advance: Instant,
    /// Simulated milliseconds, published as [`SessionStatus::world_ms`].
    world_ms: u64,
    /// The sub-millisecond remainder of the last step.
    ///
    /// The world advances in whole milliseconds (`Spline::elapsed_ms` is a
    /// `u32`), so truncating each step would lose up to 1 ms of every one —
    /// which at forty steps a second is a systematic **4% slow**, applied to
    /// every spline in the world and to nothing else, so creatures would
    /// gradually fall behind their own paths with every number looking right.
    carry_ms: f64,
    /// Steps that overran [`MAX_STEP`], and the worst — published as
    /// [`SessionStatus::stalls`], which is where the reason is.
    stalls: u32,
    worst_stall_ms: u32,
    /// Carries that moved the character further than a transport can travel,
    /// and the worst — published as [`SessionStatus::platform_jumps`].
    platform_jumps: u32,
    worst_platform_jump: f32,
    /// Ticks whose [`READ_SLICE`] ran out with the socket still delivering, and
    /// ticks in total — published as [`SessionStatus::read_slice_overruns`].
    read_slice_overruns: u32,
    ticks: u32,
    last_movement_sent: Instant,
    last_facing_sent: Instant,
    /// **Since when the deck under the character has been unplaceable**, or
    /// `None` while the world can answer — see [`PLATFORM_HOLD`], which is the
    /// bound on it and says why there has to be one.
    platform_held_since: Option<Instant>,
    last_ping: Instant,
    ping_sequence: u32,
    last_query: Instant,
    /// **Every entry and guid this session has already put a query on the wire
    /// for**, so that [`SessionLoop::resolve_names`] can send a *new* one at
    /// once and re-send an unanswered one on the beat. See that function, which
    /// is where both halves of the reason are.
    asked_creatures: HashSet<u32>,
    asked_gameobjects: HashSet<u32>,
    asked_items: HashSet<u32>,
    asked_players: HashSet<u64>,
    /// `Entity::position_updates` for the player at the last resync, so a
    /// server correction can be told apart from our own dead reckoning.
    seen_position_updates: u32,
    /// `SMSG_LOGOUT_COMPLETE` has arrived: stop after this tick, cleanly, and
    /// leave the socket for whoever wants it. See [`SessionLoop::handle_packet`].
    left_world: bool,
    /// The on-disk query answers. Seeded into the world before this thread
    /// started; appended to on the query interval — see [`crate::play::wdb`].
    caches: Caches,
    stats: PumpStats,
    /// When [`SessionStatus::traffic`] was last rebuilt. `None` until the first
    /// publish, so the panel has a map to read from the first tick rather than
    /// a fifth of a second in.
    traffic_published: Option<Instant>,
}

/// How often [`SessionStatus::traffic`] is rebuilt.
///
/// A fifth of a second, which is the cadence the debug panel samples every
/// other count at — see `vale_client::ui::debug`. The maps behind it are the
/// only part of the snapshot that is not a machine word, and rebuilding them at
/// the tick rate would be forty allocations of a hundred `String`s a second for
/// a reader that is usually not there.
const TRAFFIC_INTERVAL: Duration = Duration::from_millis(200);

/// **Should this query go out on this pass?** — the whole of the fresh/stale
/// split [`SessionLoop::resolve_names`] is written around.
///
/// A free function rather than a method so the rule can be tested with no
/// socket and no world, which is this crate's standing shape for anything a
/// handler decides. It has one side effect and it is the point of it: the first
/// ask *records* itself, so the second pass over the same entry can tell "never
/// asked" from "asked and still waiting".
fn worth_asking<T: Copy + Eq + std::hash::Hash>(
    asked: &mut HashSet<T>,
    key: T,
    retry: bool,
) -> bool {
    // `insert` answers whether it was new, so the fresh case is one lookup
    // rather than a `contains` and an `insert`.
    asked.insert(key) || retry
}

impl SessionLoop {
    fn new(
        session: WorldSession,
        character: CharListEntry,
        ground: Option<GroundHeight>,
        triggers: Option<TriggerTable>,
        world: Arc<Mutex<ObjectManager>>,
        status: Arc<Mutex<SessionStatus>>,
        commands: Receiver<Command>,
        caches: Caches,
    ) -> SessionLoop {
        let now = Instant::now();
        let start = Position {
            x: character.x,
            y: character.y,
            z: character.z,
            orientation: 0.0,
        };
        let map_id = character.map;
        SessionLoop {
            session,
            character,
            ground,
            triggers,
            trigger_watch: TriggerWatch::new(),
            last_trigger_check: now,
            world,
            status,
            commands,
            // Replaced with the server's own position once the login burst
            // lands; the character-list entry is only a plausible starting
            // point for the moments before that.
            local: LocalState::new(Mover::new(start, Speeds::default()), map_id),
            replies: Replies::default(),
            last_advance: now,
            world_ms: 0,
            stalls: 0,
            platform_jumps: 0,
            worst_platform_jump: 0.0,
            worst_stall_ms: 0,
            read_slice_overruns: 0,
            ticks: 0,
            carry_ms: 0.0,
            last_movement_sent: now,
            last_facing_sent: now,
            platform_held_since: None,
            last_ping: now,
            ping_sequence: 0,
            // Rewound so the first tick asks about names rather than waiting
            // out an interval with a screen full of "entry 448".
            last_query: now - QUERY_INTERVAL,
            asked_creatures: HashSet::new(),
            asked_gameobjects: HashSet::new(),
            asked_items: HashSet::new(),
            asked_players: HashSet::new(),
            seen_position_updates: 0,
            left_world: false,
            caches,
            stats: PumpStats::default(),
            traffic_published: None,
        }
    }

    fn run(&mut self) -> Result<(), String> {
        // A polling loop must not lean on SO_RCVTIMEO — see
        // `WorldSession::set_nonblocking` for what Winsock does if it does.
        self.session
            .set_nonblocking(true)
            .map_err(|e| format!("socket setup: {e}"))?;
        self.enter_world()?;

        loop {
            let tick_start = Instant::now();

            if matches!(self.drain_commands(), Flow::Stop) {
                break;
            }
            self.read_socket().map_err(|e| format!("world stream: {e}"))?;
            // **Out of the world, so nothing below this line may run.** Every
            // one of the four calls that follow writes to the socket, and the
            // character they would be writing about no longer exists — see
            // `handle_packet`, which sets this on `SMSG_LOGOUT_COMPLETE`. The
            // status is published one last time so the client sees `in_world`
            // fall on the same tick.
            if self.left_world {
                self.publish_status();
                break;
            }
            // Sampled *here* rather than at the top of the loop, so the step is
            // stamped at the moment it is taken: `read_socket` may have spent a
            // whole `READ_SLICE` draining a busy zone, and that time belongs to
            // this step rather than to the previous one.
            self.ticks = self.ticks.saturating_add(1);
            let step = self.step_ms();
            self.tick_world(step.world_ms);
            self.tick_movement(step.mover_ms).map_err(|e| format!("movement: {e}"))?;
            self.tick_triggers().map_err(|e| format!("area trigger: {e}"))?;
            self.tick_view().map_err(|e| format!("view point: {e}"))?;
            self.keepalive().map_err(|e| format!("keepalive: {e}"))?;
            self.resolve_names().map_err(|e| format!("name query: {e}"))?;
            self.publish_status();

            if let Some(rest) = TICK.checked_sub(tick_start.elapsed()) {
                thread::sleep(rest);
            }
        }
        Ok(())
    }

    /// `CMSG_PLAYER_LOGIN`, then read until the server has told us who we are.
    fn enter_world(&mut self) -> Result<(), String> {
        self.session
            .player_login(self.character.guid)
            .map_err(|e| format!("player login: {e}"))?;

        // The burst is large and arrives over a second or two; anything that
        // has not turned up by the time the player object has will arrive as an
        // ordinary update later.
        let deadline = Instant::now() + LOGIN_TIMEOUT;
        while Instant::now() < deadline {
            self.read_socket().map_err(|e| format!("login burst: {e}"))?;
            if self.player_snapshot().is_some() {
                break;
            }
            thread::sleep(TICK);
        }

        let (position, speeds) = self
            .player_snapshot()
            .ok_or("server never sent an object flagged UPDATEFLAG_SELF")?;

        // **The mover is *corrected*, not replaced, and that is not tidiness.**
        //
        // It used to be `self.local.mover = Mover::new(position, speeds)`, on
        // the reasonable-sounding grounds that everything before this point was
        // guesswork off the character-list entry. What that missed is that the
        // burst above is **handled**, not merely read: `read_socket` runs the
        // whole dispatch for a `READ_SLICE` window, so anything the server sent
        // while we were waiting for the self object has already been applied to
        // this mover — and one of the things it can send is a **ride**.
        //
        // `Player::ContinueTaxiFlight` runs at the end of `HandlePlayerLogin`,
        // after `Map::Add`, so **logging in mid-flight puts an
        // `SMSG_MONSTER_MOVE` naming our own guid in the same burst as the
        // create block that ends this loop** — usually in the same read. A fresh
        // `Mover` threw away both halves of it: the spline, so the character
        // never flew; and `finished_ride`, so the `CMSG_MOVE_SPLINE_DONE` the
        // server was waiting for was never sent.
        //
        // That second half is what made it permanent rather than merely wrong.
        // `MoveSplineInit::Launch` raises `SetSplineDonePending(true)` and
        // `HandleMovementOpcodes` discards **every** movement packet while it is
        // set, so the character walked around locally for the rest of the
        // session while the server held them on a gryphon at the last waypoint
        // it knew about — reported as "stuck on a taxi but walking on the
        // ground forever", and it is exactly that.
        self.local.mover.resync(position);
        self.local.mover.speeds = speeds;
        self.seen_position_updates = self.position_updates_of(self.character.guid);

        // Without this the server treats us as a "fake client" and silently
        // discards every movement packet — see `WorldSession::set_active_mover`.
        self.session
            .set_active_mover(self.character.guid)
            .map_err(|e| format!("set active mover: {e}"))?;
        // **…and the two latches that packet is the assertion of.** The mover
        // is the character from here on, and `Entity::client_controlled` says
        // so — which matters because [`Self::tick_view`] derives the mover from
        // that flag every tick and the server never sends a
        // `SMSG_CLIENT_CONTROL_UPDATE` at login. Without the seed the first
        // tick would take the mover away from a character nobody had said
        // anything about.
        self.local.mover_guid = self.character.guid;
        if let Some(player) = lock(&self.world).get_mut(self.character.guid) {
            player.client_controlled = true;
        }

        // An immediate heartbeat both proves the mover took and gives the
        // anticheat a non-zero `ctime` to measure the next packet against.
        self.send_movement(Opcode::MSG_MOVE_HEARTBEAT)
            .map_err(|e| format!("first heartbeat: {e}"))?;

        let mut status = lock(&self.status);
        status.in_world = true;
        status.position = position;
        status.speeds = speeds;
        // **And the map, which is the one field here that the character-list
        // row can have wrong.** `SMSG_LOGIN_VERIFY_WORLD` arrived in the burst
        // above and moved `local.map_id` if the server relocated us out of a
        // reset instance — see [`crate::state::movement::LoginVerifyWorld`].
        //
        // Published *here* rather than left to the first `publish_status`,
        // because `wait_until_in_world` returns on the line above: a caller
        // that reads the map the moment it is told there is a world would
        // otherwise get the seeded one, one tick out of three.
        status.map_id = self.local.map_id;
        Ok(())
    }

    fn drain_commands(&mut self) -> Flow {
        loop {
            match self.commands.try_recv() {
                Ok(Command::Controls(controls)) => {
                    // One packet per flag change, in order: the server rejects
                    // a packet whose opcode does not match the flag it added.
                    for event in self.local.mover.set_controls(controls) {
                        self.local.mover.info.flags = event.flags;
                        if self.send_movement(event.opcode).is_err() {
                            return Flow::Stop;
                        }
                    }
                }
                Ok(Command::Face(orientation)) => {
                    // Through the mover rather than into its field: mouse-look
                    // is a turn and obeys the same rule the turn keys do. A
                    // stunned or dead character does not spin on the mouse.
                    self.local.mover.face(orientation);
                }
                Ok(Command::Pitch(pitch)) => {
                    // No packet: the pitch is a *field* of the movement block
                    // rather than a transition, carried only under
                    // `MOVEFLAG_SWIMMING`, so it rides out on whatever the next
                    // heartbeat or facing update happens to be. `MSG_MOVE_SET_PITCH`
                    // exists (219) and this client does not send it: the value
                    // is in every block already, and nothing on the server side
                    // reads the opcode for anything the block does not say.
                    // The *keyed* ascent is a different thing entirely and does
                    // have its own packets — see [`Mover::pitch_flags`].
                    self.local.mover.set_pitch(pitch);
                }
                Ok(Command::Jump) => {
                    // Sent at once rather than folded into the next heartbeat:
                    // the server anchors the whole parabola at the first packet
                    // carrying `MOVEFLAG_JUMPING` (`MovementInfo::Read`), so a
                    // jump reported a tick late is a jump the server thinks
                    // began a tick further along.
                    
                    if let Some(event) = self.local.mover.jump() {
                        if self.send_movement(event.opcode).is_err() {
                            return Flow::Stop;
                        }
                    }
                }
                Ok(Command::Attack(guid)) => {
                    let sent = match guid {
                        Some(guid) => self.session.attack_swing(guid),
                        None => self.session.attack_stop(),
                    };
                    if sent.is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Target(guid)) => {
                    // Zero is the wire's own "nothing selected"; there is no
                    // separate clear opcode.
                    if self.session.set_selection(guid.unwrap_or(0)).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Cast { spell_id, target }) => {
                    if self.session.cast_spell(spell_id, target).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::CancelCast(spell_id)) => {
                    if self.session.cancel_cast(spell_id).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::CancelAura(spell_id)) => {
                    if self.session.cancel_aura(spell_id).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Quest(verb)) => {
                    let cached = match verb {
                        QuestVerb::Query(id) => Some((Kind::Quest, u64::from(id))),
                        _ => None,
                    };
                    match self.answer_from_cache(cached) {
                        Ok(true) => {}
                        Ok(false) => {
                            if self.session.quest(verb).is_err() {
                                return Flow::Stop;
                            }
                        }
                        Err(_) => return Flow::Stop,
                    }
                }
                Ok(Command::UseObject(guid)) => {
                    if self.session.use_object(guid).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Npc(verb)) => {
                    let cached = match verb {
                        NpcVerb::TextQuery { text_id, .. } => {
                            Some((Kind::NpcText, u64::from(text_id)))
                        }
                        NpcVerb::PageTextQuery { page_id, .. } => {
                            Some((Kind::PageText, u64::from(page_id)))
                        }
                        _ => None,
                    };
                    match self.answer_from_cache(cached) {
                        Ok(true) => {}
                        Ok(false) => {
                            if self.session.npc(verb).is_err() {
                                return Flow::Stop;
                            }
                        }
                        Err(_) => return Flow::Stop,
                    }
                }
                Ok(Command::Pet(verb)) => {
                    // The autocast toggle is answered by nothing — vmangos
                    // records it and sends no packet — so the bar's copy is
                    // flipped here or the dot never moves and the next press
                    // sends the same state again. See
                    // [`crate::state::objects::ObjectManager::apply_pet_autocast`].
                    // A reaction or command press is the same shape — see
                    // [`crate::state::objects::ObjectManager::apply_pet_press`].
                    match &verb {
                        PetVerb::Autocast { spell_id, on, .. } => {
                            lock(&self.world).apply_pet_autocast(*spell_id, *on);
                        }
                        PetVerb::Action { data, target, .. } => {
                            lock(&self.world).apply_pet_press(*data, *target != 0);
                        }
                        PetVerb::StopAttack(_) => lock(&self.world).apply_pet_stop_attack(),
                        // The drag's drop is answered by nothing either —
                        // `HandlePetSetAction` ends at `SetActionBar` — so the
                        // moved slots land in the copy here or the next rebuild
                        // undoes the drop on screen.
                        PetVerb::SetAction { moves, .. } => {
                            lock(&self.world).apply_pet_set_action(moves);
                        }
                        _ => {}
                    }
                    if self.session.pet(&verb).is_err() {
                        return Flow::Stop;
                    }
                }
                // No packet: the capture is a reader on this side of the
                // socket. Published on the traffic interval like the counters.
                Ok(Command::CaptureTraffic(on)) => self.session.capture().arm(on),
                Ok(Command::Party(verb)) => {
                    if self.session.party(&verb).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Taxi(verb)) => {
                    if self.session.taxi(&verb).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Reputation(verb)) => {
                    if self.session.reputation(verb).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Social(verb)) => {
                    if self.session.social(&verb).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Channel(verb)) => {
                    if self.session.channel(&verb).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Mail(verb)) => {
                    let cached = match &verb {
                        MailVerb::TextQuery { item_text_id, .. } => {
                            Some((Kind::ItemText, u64::from(*item_text_id)))
                        }
                        _ => None,
                    };
                    match self.answer_from_cache(cached) {
                        Ok(true) => {}
                        Ok(false) => {
                            if self.session.mail(&verb).is_err() {
                                return Flow::Stop;
                            }
                        }
                        Err(_) => return Flow::Stop,
                    }
                }
                Ok(Command::Trade(verb)) => {
                    if self.session.trade(verb).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Loot(verb)) => {
                    let sent = match verb {
                        LootVerb::Open(guid) => self.session.loot(guid),
                        LootVerb::Take(index) => self.session.loot_item(index),
                        LootVerb::TakeMoney => self.session.loot_money(),
                        LootVerb::Release(guid) => self.session.loot_release(guid),
                        LootVerb::Roll {
                            guid,
                            item_slot,
                            vote,
                        } => self.session.loot_roll(guid, item_slot, vote),
                    };
                    if sent.is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::UseItem {
                    bag,
                    slot,
                    spell_index,
                    target,
                }) => {
                    let sent = match spell_index {
                        Some(index) => self.session.use_item(bag, slot, index, target),
                        None => self.session.auto_equip_item(bag, slot),
                    };
                    if sent.is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::OpenItem { bag, slot }) => {
                    if self.session.open_item(bag, slot).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::MoveItem {
                    src_bag,
                    src_slot,
                    dst,
                    count,
                }) => {
                    if self.session.move_item(src_bag, src_slot, dst, count).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::EquipItem { bag, slot }) => {
                    if self.session.auto_equip_item(bag, slot).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::BankItem { bag, slot }) => {
                    if self.session.bank_item(bag, slot).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::SetActionButton { slot, packed }) => {
                    if self.session.set_action_button(slot, packed).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::SetActionBarToggles(mask)) => {
                    if self.session.set_actionbar_toggles(mask).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::CancelAutoRepeat) => {
                    if self.session.cancel_auto_repeat().is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::SetSheathed(state)) => {
                    if self.session.set_sheathed(u32::from(state)).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::TextEmote { text_emote, emote_num, target }) => {
                    if self.session.text_emote(text_emote, emote_num, target).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::LearnTalent { talent_id, rank }) => {
                    if self.session.learn_talent(talent_id, rank).is_err() {
                        return Flow::Stop;
                    }
                }
                // **The loop keeps running afterwards, and that is the point.**
                // The character is sat down and rooted from here until either
                // `SMSG_LOGOUT_COMPLETE` or a cancel, and every one of those
                // arrives on this socket — so a logout that stopped the thread
                // would take away the only thing that could hear the answer.
                Ok(Command::Logout { leaving }) => {
                    let sent = if leaving {
                        self.session.logout_request()
                    } else {
                        self.session.logout_cancel()
                    };
                    if sent.is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Chat { kind, target, text }) => {
                    // The language the *character* speaks, which is the only
                    // one the server will accept from them.
                    let language = Language::for_race(self.character.race);
                    if self
                        .session
                        .say(kind, language, target.as_deref(), &text)
                        .is_err()
                    {
                        return Flow::Stop;
                    }
                }
                // **The death family, and none of it is predicted.** Every one
                // of these is answered by a values block rather than by a
                // packet — `PLAYER_FLAGS_GHOST` appearing, the health coming
                // back — so there is nothing local to change and nothing to
                // roll back if the server refuses. See `crate::play::death`.
                Ok(Command::Repop) => {
                    if self.session.repop_request().is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::ResetInstances) => {
                    if self.session.reset_instances().is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::CorpseQuery) => {
                    if self.session.corpse_query().is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::ReclaimCorpse) => {
                    let guid = self.character.guid;
                    if self.session.reclaim_corpse(guid).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::ResurrectResponse { caster, accept }) => {
                    if self.session.resurrect_response(caster, accept).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::DuelAnswer { arbiter, accept }) => {
                    if self.session.duel_answer(arbiter, accept).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::SummonResponse { summoner }) => {
                    if self.session.summon_response(summoner).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::RequestPlayedTime) => {
                    if self.session.request_played_time().is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::SpiritHealerActivate { healer }) => {
                    if self.session.spirit_healer_activate(healer).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::SetAmmo { entry }) => {
                    if self.session.set_ammo(entry).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Shutdown) => return Flow::Stop,
                Err(TryRecvError::Empty) => return Flow::Continue,
                // The handle was dropped without a shutdown; same outcome.
                Err(TryRecvError::Disconnected) => return Flow::Stop,
            }
        }
    }

    /// Drain whatever the socket has, up to [`READ_SLICE`].
    fn read_socket(&mut self) -> io::Result<()> {
        let until = Instant::now() + READ_SLICE;
        loop {
            let wait = until
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO)
                // Zero would mean "block forever" to the socket layer; one
                // millisecond still returns anything already buffered.
                .max(Duration::from_millis(1));
            let Some(pkt) = self.session.next_packet(wait)? else {
                return Ok(());
            };
            self.handle_packet(&pkt)?;
            if Instant::now() >= until {
                // **The tick spent its whole packet budget and left packets
                // behind.** Counted because the failure it belongs to has no
                // other symptom: if the stream is arriving faster than a
                // 12-of-25-millisecond slice can drain it, the backlog is in
                // the kernel's receive buffer and this client is reading the
                // world as it was some seconds ago — with every position,
                // every animation and every counter on screen internally
                // consistent and simply late. A busy realm is where that would
                // happen and a localhost server is where it never will, which
                // is exactly the shape of the desync report this exists for.
                //
                // It over-counts by one class: a slice that ran to the deadline
                // and would have found the socket empty on its next look. That
                // is the safe direction — the number errs towards saying "we
                // are behind" — and what makes it readable is the *proportion*
                // of ticks, not the count.
                self.read_slice_overruns = self.read_slice_overruns.saturating_add(1);
                return Ok(());
            }
        }
    }

    /// **Answer a query out of the on-disk cache instead of sending it.**
    ///
    /// `cached` is the kind and key a command asks for, or `None` for a
    /// command that is not a query. A hit runs the cached body through
    /// [`Self::handle_packet`] under the response opcode, so every reader —
    /// the event queue, the arrival edges, the counters — sees exactly what it
    /// would have seen from the server, one tick later rather than one round
    /// trip later. Answers `Ok(true)` when the packet need not be sent.
    ///
    /// The four queries this answers have no side effect on the server:
    /// vmangos' `HandleQuestQueryOpcode`, `HandleNpcTextQueryOpcode`,
    /// `HandlePageTextQueryOpcode` and `HandleItemTextQuery` each look a row
    /// up and reply. Marking a letter read is `CMSG_MAIL_MARK_AS_READ`, a
    /// packet of its own.
    fn answer_from_cache(&mut self, cached: Option<(Kind, u64)>) -> io::Result<bool> {
        let Some((kind, key)) = cached else {
            return Ok(false);
        };
        let body = lock(&self.world)
            .cached_answer(kind, key)
            .map(<[u8]>::to_vec);
        let Some(body) = body else {
            return Ok(false);
        };
        self.stats.cache_answered += 1;
        self.handle_packet(&Packet {
            code: kind.opcode().code(),
            body,
        })?;
        Ok(true)
    }


    /// Hand one packet to the dispatch, then send whatever it queued.
    ///
    /// **This used to be a second dispatch.** Seven opcodes were intercepted
    /// here and returned early, because the shared `apply_packet` took only an
    /// `ObjectManager` and so could neither answer the server nor move the
    /// local player. The rule was correct and unwritten, `SMSG_PONG` ended up in
    /// both, `stats.packets` was counted by hand in each branch, and a speed
    /// change about another unit was acknowledged and then dropped. All of it is
    /// [`crate::socket::handler`]'s now; what is left here is the two things that are
    /// genuinely this loop's — publishing to the status, and noticing that a
    /// packet relocated us.
    ///
    /// The replies are sent **after** the world lock is released, which is the
    /// same rule `resolve_names` follows: a socket write under that lock blocks
    /// every reader on network latency.
    fn handle_packet(&mut self, pkt: &Packet) -> io::Result<()> {
        let relocations = self.local.relocations;
        {
            let mut world = lock(&self.world);
            apply_packet(
                &mut Incoming {
                    world: &mut world,
                    stats: &mut self.stats,
                    replies: &mut self.replies,
                    local: Some(&mut self.local),
                },
                pkt,
            );
        }
        self.session.flush(&mut self.replies)?;

        // **`SMSG_LOGOUT_COMPLETE` is the one packet that ends the loop from the
        // server's side**, and it has to, for two reasons. The character is out
        // of the world, so every heartbeat and facing update `tick_movement`
        // would go on sending is an opcode vmangos logs as unexpected — and the
        // socket is about to change hands: 1.12 goes back to character select on
        // it, which is [`LiveSession::reclaim`], and a thread still writing to a
        // connection it has handed over is a stream desynchronised for good.
        //
        // Read off the opcode rather than off the event queue on purpose: the
        // queue has exactly one drain and it belongs to the client.
        if pkt.is(Opcode::SMSG_LOGOUT_COMPLETE) {
            self.left_world = true;
        }

        // A packet moved the player. The mover has already adopted the position,
        // so comparing positions cannot tell this from our own dead reckoning —
        // hence the counter, the same instrument `Entity::position_updates` is.
        //
        // Re-baselining matters because the player's own create block arrives
        // again with a far teleport's burst and bumps `position_updates`;
        // without this, `tick_movement` would read that as a server correction
        // and resync the mover to a position from the map just left.
        if self.local.relocations != relocations {
            self.seen_position_updates = self.position_updates_of(self.mover_guid());
            // **A crossing starts the platform hold over.** A character carried
            // across on a boat has already spent part of `PLATFORM_HOLD`
            // waiting for the server's teleport, and the far side then wants
            // the whole of it again for the deck's create block, its query and
            // its `.wmo` staging. Measuring both windows on one clock lands
            // them in the sea at Theramore, and a teleport is a new start by
            // any reading: the deck under them now is a different placement of
            // it. See [`platform_under`].
            self.platform_held_since = None;
            let mut status = lock(&self.status);
            status.map_id = self.local.map_id;
            status.position = self.local.mover.position();
        }
        Ok(())
    }

    /// How far to advance the simulation: the time that has actually passed
    /// since the last step, in whole milliseconds, with the remainder carried.
    ///
    /// This is the loop's real clock. `TICK` is only how long it tries to sleep,
    /// and on Windows a `thread::sleep` shorter than the ~15.6 ms scheduler
    /// granularity rounds up to it — so the true period wanders and a fixed step
    /// would turn that wander into a wandering *speed*. See the module comment
    /// for why the server prefers this too.
    fn step_ms(&mut self) -> Step {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_advance);
        self.last_advance = now;

        // **The world clock is real time and is not clamped.** It is what every
        // spline is walked by and what the renderer's interpolator keys on, and
        // both of those are written against "world time and real time run at
        // the same rate" — see [`SessionStatus::world_ms`] and the renderer's
        // `world::motion`. Clamping it broke that quietly, and the visible half
        // of it is in [`MAX_STEP`]'s own note.
        let step = split_step(&mut self.carry_ms, elapsed);
        self.world_ms += u64::from(step.world_ms);
        if elapsed > MAX_STEP {
            let stalled = elapsed.as_millis().min(u128::from(u32::MAX)) as u32;
            self.stalls = self.stalls.saturating_add(1);
            self.worst_stall_ms = self.worst_stall_ms.max(stalled);
        }
        step
    }

    /// Advance everything the server is moving for us.
    ///
    /// **On the same ground the local character walks on**, and bound to the map
    /// it is on now for the reason [`Self::tick_movement`] rebinds it every step:
    /// a stale map id would clip everyone in view against a continent they have
    /// left. Without it every dead-reckoned player walks straight through the
    /// wall their own client stopped at — see [`ObjectManager::advance`].
    fn tick_world(&mut self, dt_ms: u32) {
        if dt_ms == 0 {
            return;
        }
        let bound = self.ground.as_ref().map(|world| OnMap {
            world: world.as_ref(),
            map_id: self.local.map_id,
            // Everybody else's dead reckoning, which has no ferry of its own.
            adrift: false,
        });
        let bound = bound.as_ref().map(|b| b as &dyn Footing);
        let mut world = lock(&self.world);
        // **A stall is walked off in tick-sized slices rather than in one
        // stride, and this is where that belongs rather than inside
        // `advance`.** The two populations there want opposite things from a
        // long step: a spline must get *all* of it, because the server really
        // walked all of it and a cyclic path is never restated; a dead
        // reckoning must not get it in one go, because it is a stride this
        // client is inventing and `Footing::step` is written for 0.2-yard ones.
        // Slicing gives both — the splines see the same total either way, and
        // the reckoning sees exactly the sequence of steps an unstalled thread
        // would have taken. See `ObjectManager::advance`.
        for slice in slices(dt_ms) {
            world.advance(slice, bound);
        }
    }

    /// Advance the local simulation and send whatever it owes the server.
    fn tick_movement(&mut self, dt_ms: u32) -> io::Result<()> {
        // **Whose body this whole function is about** — the character's, except
        // while a possess lasts. See [`Self::tick_view`], which derives it.
        let mover = self.mover_guid();
        // **Nobody to drive**: feared or confused, and the server is walking the
        // body itself. The client sends no `CMSG_SET_ACTIVE_MOVER` for a zero
        // guid and there is nothing here to send movement about either. The
        // controls are zeroed rather than merely ignored, so a key held at the
        // moment control was taken away is not still held when it comes back.
        if mover == 0 {
            self.local.mover.set_controls(Controls::default());
            return Ok(());
        }
        // The ground's own read-ahead: the tile under the character and the
        // ring around it, before the stride below asks for a height on it.
        if let Some(ground) = self.ground.as_ref() {
            let at = self.local.mover.position();
            ground.focus(self.local.map_id, at.x, at.y);
        }

        // A server correction — teleport, knockback, a fall the server
        // resolved — always beats dead reckoning.
        let updates = self.position_updates_of(mover);
        if updates != self.seen_position_updates {
            self.seen_position_updates = updates;
            if let Some((position, speeds)) = self.snapshot_of(mover) {
                self.local.mover.resync(position);
                self.local.mover.speeds = speeds;
            }
        }

        // **What the server has stopped this character doing**, re-read every
        // tick. Neither field has a packet of its own — a stun is
        // `UNIT_FIELD_FLAGS` moving inside whatever values block carried the
        // aura, and death is `UNIT_FIELD_HEALTH` reaching zero — so there is
        // nothing to hook and the honest place to ask is here, beside the step
        // that obeys the answer. See `movement::Restraint`.
        let restraint = {
            let world = lock(&self.world);
            world.get(mover).map(|e| crate::state::movement::Restraint {
                stunned: e.is_stunned(),
                dead: e.is_dead().unwrap_or(false),
                // …and the one of the three that outlives a session, which is
                // why it is read here beside the other two rather than left to
                // the ride: see `Restraint::on_taxi`.
                on_taxi: e.is_on_taxi(),
            })
        };
        if let Some(restraint) = restraint {
            self.local.mover.set_restraint(restraint);
        }

        // **Boarded, carried or stepped off — before the step, not after.**
        //
        // A character standing on a moving platform has to be moved by however
        // far it moved *first*, so that their own stride is taken from where
        // the deck has put them. Done afterwards it is a correction rather than
        // a ride, and on the Deeprun Tram — whose car travels 2,482 yards — the
        // difference is the whole report: the floor leaves and the character
        // does not.
        //
        // **The filter is here rather than in the world**, because
        // `UPDATEFLAG_TRANSPORT` is the only thing in the game that says an
        // object moves and only the object manager holds it. Without it a
        // character standing on any door or chest would be boarded onto it, and
        // the server would refuse the guid.
        //
        // The rule is [`platform_under`], which is a free function so that it
        // can be asserted without a socket.
        let mut carried = false;
        let mut adrift = false;
        if let Some(ground) = self.ground.as_ref() {
            let at = self.local.mover.position();
            let world = &self.world;
            let under = platform_under(
                ground.as_ref(),
                self.local.map_id,
                at,
                self.local.mover.ferry(),
                |guid| {
                    lock(world)
                        .get(guid)
                        .is_some_and(|e| e.transport_phase_ms.is_some())
                },
                Instant::now(),
                &mut self.platform_held_since,
            );
            adrift = under.adrift;
            let before = self.local.mover.position();
            carried = self.local.mover.carry_platform(under.platform);
            // **What a wrong platform placement looks like from here**, and the
            // only place it can be seen — see [`SessionStatus::platform_jumps`].
            let after = self.local.mover.position();
            let moved = ((after.x - before.x).powi(2)
                + (after.y - before.y).powi(2)
                + (after.z - before.z).powi(2))
            .sqrt();
            if moved > PLATFORM_JUMP {
                self.platform_jumps = self.platform_jumps.saturating_add(1);
                self.worst_platform_jump = self.worst_platform_jump.max(moved);
            }
        }

        // Bound to the map the character is on *now*, so a far teleport moves
        // the ground under it as well as the character.
        let bound = self.ground.as_ref().map(|world| OnMap {
            world: world.as_ref(),
            map_id: self.local.map_id,
            adrift,
        });
        let bound = bound.as_ref().map(|b| b as &dyn Footing);
        let owed = self.local.mover.advance(dt_ms as f32 / 1000.0, bound);

        // **…and where that stride left them, in the platform's own frame.**
        //
        // The other half of the carry above, and it has to be here rather than
        // beside it: the offset the wire carries *is* the passenger's position
        // as far as the server is concerned, so one taken before the stride
        // describes where they were standing a tick ago and the next carry puts
        // them back there. See `Mover::stow_platform`. A no-op for the whole of
        // a session that never boards anything.
        self.local.mover.stow_platform();

        // Publish our position onto **the mover's** entity so that anything
        // reading the object manager sees where the client actually is, rather
        // than the last place the server happened to mention. The character's
        // own entity is left alone while somebody else is being driven, which
        // is what leaves the body standing where the possess began.
        let position = self.local.mover.position();
        if let Some(entity) = lock(&self.world).get_mut(mover) {
            entity.position = Some(position);
        }

        // Leaving the ground and landing are reported the instant they happen —
        // see `Mover::advance`, and `CHEAT_TYPE_BAD_FALL_STOP` for what a
        // landing folded into an ordinary heartbeat costs.
        if let Some(opcode) = owed {
            self.send_movement(opcode)?;
            self.last_facing_sent = Instant::now();
            return Ok(());
        }

        // **A ride the server drove, ended and acknowledged.** Until this
        // packet arrives `HandleMovementOpcodes` discards everything below on
        // its first line — so the charge that started it would cost the rest of
        // the session, and the only other thing that clears the flag is a login.
        // See `movement::Mover::ride`.
        if let Some(spline_id) = self.local.mover.take_finished_ride() {
            self.stats.rides += 1;
            let info = self.movement_info();
            self.session
                .send(Opcode::CMSG_MOVE_SPLINE_DONE, &crate::state::movement::spline_done_body(&info, spline_id))?;
            self.local.mover.mark_sent();
            self.last_movement_sent = Instant::now();
            self.last_facing_sent = Instant::now();
            lock(&self.status).movement_sent += 1;
            return Ok(());
        }
        // …and while one is still running there is nothing to say: the server is
        // walking its own copy of the same spline and drops anything we send.
        if self.local.mover.is_riding() {
            return Ok(());
        }

        // **A facing change is reported whether or not the character is
        // moving**, and this is an anticheat rule rather than a nicety.
        // `CheckSpeedHack` re-runs `ExtrapolateMovement` from the *last packet's
        // orientation* over the client-time delta and accumulates
        // `m_overspeedDistance` past a 10% allowance. Turning while running
        // therefore walks the server's straight line away from our arc for as
        // long as the gap lasts: at 7.6 y/s over a 400 ms heartbeat a half-turn
        // separates them by a couple of yards a time, against `Threshold = 30`
        // yards total. It reads as "the client is drifting and being snapped
        // back", and to the server it reads as a speed hack.
        let turned = self.local.mover.facing_changed()
            && self.last_facing_sent.elapsed() >= FACING_INTERVAL;

        // **Boarding and stepping ashore are reported at once**, because they
        // are the only two things that tell the server a character is on a
        // transport at all — see `Mover::ferry_changed`, which carries what
        // each of them costs when it is late. Not rate-limited: it is one
        // packet per gangplank, and the anticheat's own transport tests read
        // the *previous* packet's flag, so an edge folded into the next
        // heartbeat is measured in the wrong frame.
        if self.local.mover.ferry_changed() {
            let boarded = self.local.mover.ferry().is_some();
            self.send_movement(Opcode::MSG_MOVE_HEARTBEAT)?;
            self.last_facing_sent = Instant::now();
            // **…and a boarding asks for the deck's clock back**, which is the
            // one packet that can correct it. A transport's position is not on
            // the wire: both ends run the same schedule over the same shipped
            // table and agree about *when* through `UPDATEFLAG_TRANSPORT`'s
            // path-progress word, which this client is told once — in the
            // create block `Map::SendInitTransports` sends at login — and
            // advances on its own clock from then on. Any world time this
            // client drops after that (`slices` caps a stall at
            // `MAX_CATCH_UP`) leaves its boat permanently early, and an early
            // boat crosses to the other continent before the server's does.
            //
            // `HandleMoveTimeSkippedOpcode` re-sends the transport's
            // out-of-range and create blocks to a player it has just boarded —
            // the `SetJustBoarded` pair in `HandleMoverRelocation`, whose own
            // comment calls it a 1.12 client fix — and that create block
            // restates the phase. So the schedule is resynchronised at the one
            // moment it matters, which is the moment somebody steps aboard.
            if boarded {
                self.session.send(
                    Opcode::CMSG_MOVE_TIME_SKIPPED,
                    &crate::state::movement::time_skipped_body(mover, 0),
                )?;
            }
            return Ok(());
        }

        // **A passenger keeps talking whether or not it is walking.** Standing
        // still on a deck sets no flag in `MOVEFLAG_MASK_MOVING`, so the two
        // arms below would say nothing for the whole crossing — and the server
        // measures the next packet against the last one it heard, which by then
        // is an ocean away. The test is `carried` — the deck moved this tick —
        // rather than merely being aboard, so a character standing on a docked
        // boat or a parked lift pays nothing: the server's copy of where they
        // are cannot go stale while the thing they are standing on is still.
        if self.local.mover.info.is_moving() || carried {
            if turned || self.last_movement_sent.elapsed() >= HEARTBEAT {
                self.send_movement(Opcode::MSG_MOVE_HEARTBEAT)?;
                self.last_facing_sent = Instant::now();
            }
        } else if turned {
            self.send_movement(Opcode::MSG_MOVE_SET_FACING)?;
            self.last_facing_sent = Instant::now();
        }

        Ok(())
    }

    /// **Report standing in an area trigger** — which is the whole of walking
    /// into a dungeon, and the only packet this client sends that nothing asked
    /// it for. See [`crate::play::areatrigger`] for why it is the client's job at all.
    ///
    /// Placed after [`Self::tick_movement`] so it reads the position that was
    /// just simulated rather than the previous tick's — the packet the server
    /// re-validates against its own 5-yard tolerance is this one, and a stale
    /// position is a portal that refuses at the boundary.
    fn tick_triggers(&mut self) -> io::Result<()> {
        if self.last_trigger_check.elapsed() < crate::play::areatrigger::CHECK_INTERVAL {
            return Ok(());
        }
        self.last_trigger_check = Instant::now();
        // **A trigger is about the character, and while a possess lasts the
        // mover is not the character.** `HandleAreaTriggerOpcode` acts on
        // `_player`, so an Eye of Kilrogg flown into an instance portal would
        // ask the server to move the warlock through it. The character is
        // standing still for the duration, so there is nothing to poll.
        if self.mover_guid() != self.character.guid {
            return Ok(());
        }
        let Some(triggers) = self.triggers.as_deref() else {
            return Ok(());
        };
        // **The mover's position, not the object manager's.** They differ by up
        // to a tick of dead reckoning, and this one is what the movement packets
        // are stating — so the server is validating the trigger against the
        // position it has, rather than against one nothing ever sent it.
        let at = self.local.mover.position();
        let Some(entered) = self
            .trigger_watch
            .poll(triggers, self.local.map_id, [at.x, at.y, at.z])
        else {
            return Ok(());
        };
        self.stats.area_triggers += 1;
        self.session.send(
            Opcode::CMSG_AREATRIGGER,
            &crate::play::areatrigger::area_trigger_body(entered),
        )
    }

    /// **Where this client is looking from, and which body the keys drive** —
    /// the same derivation the reference runs every frame.
    ///
    /// Both answers come out of one field. `PLAYER_FARSIGHT` names either a
    /// far-sight `DynamicObject` (Eagle Eye, Far Sight, Bird's Eye) or a
    /// possessed unit (Eye of Kilrogg, Mind Control, Eyes of the Beast), and
    /// the reference's rule is:
    ///
    /// ```text
    /// no farsight guid, or it names us      -> view: our own eyes
    ///                                          mover: us, if we are controlled
    /// a guid the object manager cannot find -> the same, and do not switch
    /// it names a UNIT we are controlled of  -> view: it     mover: it
    /// it names anything else                -> view: it     mover: unchanged
    /// ```
    ///
    /// **So possession *is* far sight as far as the client is concerned**, and
    /// that is worth stating because it does not look like it should be:
    /// `SMSG_CLIENT_CONTROL_UPDATE` sets a flag on a unit and never moves the
    /// mover itself (see [`super::handler::acks::client_control`]); the mover
    /// is set from *this* function, off the
    /// conjunction of that flag and this field. The last branch is Eagle Eye:
    /// the view moves and the character goes on being the one you drive.
    ///
    /// Two packets come out of it. `CMSG_FAR_SIGHT` is one byte on each edge of
    /// the view, from the latch this keeps; what it buys
    /// is the *server* moving its own visibility source
    /// (`Camera::SetView(obj, false)`), without which the camera arrives a
    /// hundred yards away with nothing streamed in around it. And
    /// `CMSG_SET_ACTIVE_MOVER` names the new mover — skipped for a zero one,
    /// exactly as the client skips it, because a zero guid is what
    /// `GetConfirmedMover` reads as "no mover client side, is this a fake
    /// client?".
    fn tick_view(&mut self) -> io::Result<()> {
        let me = self.character.guid;
        let (farsight, resolves, is_unit, controls_it, controls_me) = {
            let world = lock(&self.world);
            let farsight = world.player().and_then(crate::state::objects::Entity::farsight);
            let target = farsight.and_then(|guid| world.get(guid));
            (
                farsight,
                target.is_some(),
                target.is_some_and(crate::state::objects::Entity::is_unit_like),
                target.is_some_and(|t| t.client_controlled),
                world.player().is_some_and(|p| p.client_controlled),
            )
        };
        let own_eyes = || (false, if controls_me { me } else { 0 });
        let (looking, mover) = match farsight {
            None => own_eyes(),
            Some(guid) if guid == me => own_eyes(),
            // **Stated a packet or two before the object it names arrives**, and
            // the honest picture for those frames is the character's own — which
            // is what the client does with an unresolved guid.
            Some(_) if !resolves => own_eyes(),
            Some(guid) if is_unit && controls_it => (true, guid),
            // Far sight: the view moves and the mover does not. Left where it
            // was rather than recomputed, because the client does not change
            // the mover on this path at all.
            Some(_) => (true, self.local.mover_guid),
        };

        if looking != self.local.looking_through {
            self.local.looking_through = looking;
            self.session.send(Opcode::CMSG_FAR_SIGHT, &[u8::from(looking)])?;
        }
        self.set_mover(mover)
    }

    /// **Point the local simulation at a body and tell the server**.
    ///
    /// The mover is re-seated rather than corrected, which is the one place
    /// this deliberately differs from [`SessionLoop::enter_world`]'s
    /// `resync`: that is the *same* character with a ride possibly already in
    /// flight, and this is a different body altogether, so carrying the old
    /// one's spline, its platform or its fall arc across would walk the new one
    /// off the last thing the old one was standing on.
    fn set_mover(&mut self, guid: u64) -> io::Result<()> {
        if guid == self.local.mover_guid {
            return Ok(());
        }
        self.local.mover_guid = guid;
        if guid == 0 {
            // Feared or confused: the server is walking the body itself and
            // there is nobody to name. No packet — see the note above.
            return Ok(());
        }
        if let Some((position, speeds)) = self.snapshot_of(guid) {
            self.local.mover = Mover::new(position, speeds);
        }
        self.seen_position_updates = self.position_updates_of(guid);
        self.session
            .set_active_mover(guid)
            .and_then(|()| self.send_movement(Opcode::MSG_MOVE_HEARTBEAT))
    }

    fn keepalive(&mut self) -> io::Result<()> {
        if self.last_ping.elapsed() < PING_INTERVAL {
            return Ok(());
        }
        self.ping_sequence = self.ping_sequence.wrapping_add(1);
        self.session.ping(self.ping_sequence, self.local.latency_ms)?;
        self.last_ping = Instant::now();
        // The `SMSG_PONG` handler takes this and turns it into the round trip.
        self.local.ping_sent_at = Some(self.last_ping);
        Ok(())
    }

    /// Ask about anything visible we still cannot name. Replies arrive
    /// asynchronously and are folded in by [`apply_packet`].
    ///
    /// ## An entry nobody has asked about yet goes out on the next tick
    ///
    /// This used to run **only** on [`QUERY_INTERVAL`], which put an average of
    /// a second — and a worst case of two — between the moment something wanted
    /// a name and the moment the packet asking for it left. That is the whole of
    /// "a vendor takes a long time to show what it is selling":
    /// `SMSG_LIST_INVENTORY` names no item, the merchant panel queues every
    /// entry the item cache cannot answer, and then nothing at all happened for
    /// up to two seconds before a round trip that costs a few milliseconds
    /// against a server on this machine — measured at 40 templates in 156 ms.
    ///
    /// So the pass is split in two. **Fresh** entries — ones this session has
    /// never sent a query for — go out on the tick they appear, which is
    /// [`TICK`] rather than [`QUERY_INTERVAL`]. **Stale** ones — asked and not
    /// answered — are re-asked on the old interval, which is what that interval
    /// was always for: a query is fire-and-forget with no ack, so an answer that
    /// never comes has to be asked for again.
    ///
    /// The `asked_*` sets are what makes the split possible, and they retire a
    /// second fault worth naming: without them **every** unresolved entry was
    /// re-sent every two seconds for as long as it stayed unresolved, so a
    /// vendor with twenty unknown items put twenty packets on the wire every two
    /// seconds until the answers landed. They are per-session and unpruned,
    /// bounded by the same thing [`ObjectManager::wanted_items`] is: the number
    /// of distinct entries one character meets.
    fn resolve_names(&mut self) -> io::Result<()> {
        let retry = self.last_query.elapsed() >= QUERY_INTERVAL;
        if retry {
            self.last_query = Instant::now();
        }

        // Copy the work list out before touching the socket: holding the world
        // lock across a write would block every reader on network latency.
        let (creatures, gameobjects, players, items, pets, learned) = {
            let mut world = lock(&self.world);
            // **The four walks below are of every entity in view**, and this
            // function now runs on the tick rather than on the beat — so the
            // O(1) question is asked first and an idle tick costs a lock and a
            // bool. See [`ObjectManager::take_query_hint`].
            if !world.take_query_hint() && !retry {
                return Ok(());
            }
            // **Only drained on the beat.** The disk write below is the one
            // expensive thing in this function and a login seeds hundreds of
            // templates; batching them at two seconds is what the beat is still
            // here for now that the queries no longer need it.
            let learned = if retry {
                std::mem::take(&mut world.learned)
            } else {
                Vec::new()
            };
            (
                world.unresolved_creature_entries(),
                world.unresolved_gameobject_entries(),
                world.unresolved_player_guids(),
                world.unresolved_item_entries(),
                world.unresolved_pet_names(),
                learned,
            )
        };
        // **Written outside the lock and before the sends**, for the same reason
        // the sends are: a disk write held against every reader of the world is
        // the fault this function's own comment is about. A failure is dropped
        // — a cache that cannot be written is a colder next session and nothing
        // worse, and taking a live session down over it would be absurd.
        if !learned.is_empty() {
            let _ = self.caches.append(&learned);
        }
        for (entry, guid) in creatures {
            if !worth_asking(&mut self.asked_creatures, entry, retry) {
                continue;
            }
            self.session.query_creature(entry, guid)?;
        }
        for (entry, guid) in gameobjects {
            if !worth_asking(&mut self.asked_gameobjects, entry, retry) {
                continue;
            }
            self.session.send(
                Opcode::CMSG_GAMEOBJECT_QUERY,
                &query::gameobject_query_body(entry, guid),
            )?;
        }
        for guid in players {
            if !worth_asking(&mut self.asked_players, guid, retry) {
                continue;
            }
            self.session
                .send(Opcode::CMSG_NAME_QUERY, &query::name_query_body(guid))?;
        }
        // Equipment. The entry is all a `PLAYER_VISIBLE_ITEM` field carries and
        // `Item.dbc` is not in the archives, so what a sword looks like is a
        // question only the server can answer — and the GUID it is asked about
        // is zero, because another player's sword is a number and not an object
        // this client has ever seen.
        //
        // …and it is the loot window that makes the latency here matter most:
        // a corpse's rows carry a display id and no name, and `LootFrame` has no
        // refresh event at all — see [`crate::play::loot`], which is where the
        // consequence is written down.
        for entry in items {
            if !worth_asking(&mut self.asked_items, entry, retry) {
                continue;
            }
            self.session
                .send(Opcode::CMSG_ITEM_QUERY_SINGLE, &query::item_query_body(entry, 0))?;
        }
        // **The pet's own name**, which is the one name in the world that is not
        // keyed by a guid or an entry: `SMSG_CREATURE_QUERY_RESPONSE` answers a
        // pet with its *species* ("Wolf"), and what the player called it comes
        // back only from `CMSG_PET_NAME_QUERY` against
        // `UNIT_FIELD_PETNUMBER`. Asked once per number rather than through
        // `worth_asking`: the set is emptied by the answer, and a pet number is
        // stable across being dismissed and called back, so a retry loop would
        // ask for ever about one the server has already refused.
        for (pet_number, guid) in pets {
            self.session.pet(&PetVerb::NameQuery {
                pet_number,
                pet: guid,
            })?;
        }
        Ok(())
    }

    fn send_movement(&mut self, opcode: Opcode) -> io::Result<()> {
        let info = self.movement_info();
        self.session.send_movement(opcode, &info)?;
        self.local.mover.mark_sent();
        self.last_movement_sent = Instant::now();
        lock(&self.status).movement_sent += 1;
        Ok(())
    }

    /// The block to send right now, stamped with the session clock.
    ///
    /// `ctime` must be non-zero and must never decrease — see the anticheat
    /// notes in [`crate::state::movement`] — so it is milliseconds since the thread
    /// started, offset by one so the very first packet is not zero.
    fn movement_info(&mut self) -> MovementInfo {
        // One copy of the `ctime` stamping, in `LocalState` — the packet
        // handlers need the same thing to build a speed-change ack, and two
        // implementations of a monotonic clock is two places for it to go
        // backwards. A decrease is `CHEAT_TYPE_TIME_BACK`.
        self.local.movement_info()
    }

    fn publish_status(&mut self) {
        let position = self.local.mover.position();
        let controls = self.local.mover.controls();
        let moving = self.local.mover.info.is_moving();
        // The speed the local simulation is actually advancing at, not the run
        // speed: holding shift walks, swimming is a different number again, and
        // a server-driven ride is none of the six — see `Mover::travel_speed`.
        let speed = self.local.mover.travel_speed();
        let riding = self.local.mover.ride_velocity();
        let ferry = self.local.mover.ferry();
        let packets = self.stats.packets as u64;
        let warnings = self.stats.warnings.clone();
        let unhandled = self.stats.other.clone();
        let world_ms = self.world_ms;
        // **The clock is run forward here rather than by whoever reads it.**
        // The server states the time once at login and never again, so somebody
        // has to advance it — and this loop is the only place with a clock that
        // is already monotonic and already the one the world's positions were
        // stepped by. A reader advancing it from its own frame timer would be a
        // second clock beating against this one, which is the exact fault
        // `world_ms` exists to have avoided once already.
        let (game_time, weather, attacking, spellbook_version) = {
            let world = lock(&self.world);
            (
                world.game_time.map(|t| t.advanced(world_ms)),
                world.weather,
                world.attacking,
                world.spellbook_version,
            )
        };

        let mut status = lock(&self.status);
        // Published *with* the position it belongs to and under the same lock,
        // because a reader uses it to decide whether the position is new: split
        // them and a poll can see a fresh clock beside a stale position, which
        // is a one-step hitch that only shows up under load.
        status.world_ms = world_ms;
        // …and the real moment `world_ms` names, which is the one thing about a
        // reading that cannot be recovered afterwards. `last_advance` is the
        // instant `step_ms` measured this step from, so it is exactly when the
        // mover stood where `position` says.
        status.taken = Some(self.last_advance);
        status.stalls = self.stalls;
        status.platform_jumps = self.platform_jumps;
        status.worst_platform_jump = self.worst_platform_jump;
        status.worst_stall_ms = self.worst_stall_ms;
        status.read_slice_overruns = self.read_slice_overruns;
        status.ticks = self.ticks;
        status.map_id = self.local.map_id;
        status.position = position;
        status.controls = controls;
        status.moving = moving;
        status.speed = speed;
        status.riding = riding;
        status.ferry = ferry;
        // The block the gait, the strafe body offset and the prediction are all
        // read off. Published whole rather than as three readings taken here:
        // the flags are what the client's own rules are written on, and a
        // reading is a place for the two to drift apart.
        status.movement = self.local.mover.info;
        status.airborne = self.local.mover.is_airborne();
        status.jumping = self.local.mover.is_jumping();
        status.mover = self.mover_guid();
        // Beside the block for the same reason the block is published whole: the
        // prediction runs the mover's own two rules, and it cannot run them off
        // the flags alone.
        status.restraint = self.local.mover.restraint();
        status.packets = packets;
        status.warnings = warnings;
        status.unhandled = unhandled;
        // Written by packet handlers rather than by this loop, so they are
        // copied out here rather than published at the moment they change — the
        // handlers run under the world lock and must not also take the status
        // one. Both are read-only to everything outside this thread.
        status.speeds = self.local.mover.speeds;
        status.latency_ms = self.local.latency_ms;
        status.speed_changes = self.stats.speed_changes;
        status.speed_broadcasts = self.stats.speed_broadcasts;
        status.spline_flag_changes = self.stats.spline_flag_changes;
        status.pushed_sounds = self.stats.pushed_sounds;
        status.pushed_visuals = self.stats.pushed_visuals;
        status.bagged_moves = self.stats.bagged_moves;
        status.bagged_packets = self.stats.bagged_packets;
        status.area_triggers = self.stats.area_triggers;
        status.rides = self.stats.rides;
        status.flag_changes = self.stats.flag_changes;
        status.attacks = self.stats.attacks;
        status.emotes = self.stats.emotes;
        status.casts = self.stats.casts;
        status.ai_reactions = self.stats.ai_reactions;
        status.cast_results = self.stats.cast_results;
        status.attack_refusals = self.stats.attack_refusals;
        status.spells_known = self.stats.spells_known;
        status.action_buttons = self.stats.action_buttons;
        status.attacking = attacking;
        status.spellbook_version = spellbook_version;
        status.game_time = game_time;
        status.weather = weather;
        // **Rebuilt a few times a second rather than every publish.** This is
        // the one field in the snapshot that is not a word: two `BTreeMap`s of
        // opcode names, rebuilt at 40 Hz for a panel that samples at five and
        // is usually not open at all. See the field's own note; the reader is
        // the debug panel's net tab, which cannot tell the difference.
        if self
            .traffic_published
            .is_none_or(|at| at.elapsed() >= TRAFFIC_INTERVAL)
        {
            self.traffic_published = Some(Instant::now());
            status.traffic = Arc::new(Traffic {
                wire: self.session.wire(),
                inbound: crate::socket::world::named(&self.stats.traffic),
                outbound: self.session.sent(),
            });
            // **Only while it is armed**, so that disarming leaves the last
            // snapshot readable rather than clearing the panel the moment the
            // box is unticked — which is when the packets are wanted.
            let capture = self.session.capture();
            if capture.armed() || status.capture.armed {
                // The second arm is the last publish after a disarm, so the
                // final packets are in the snapshot and its flag agrees with
                // the ring.
                status.capture = Arc::new(capture.snapshot());
            }
        }
    }

    // --- small accessors -------------------------------------------------

    /// **Whose body the movement packets are about**, which is the character's
    /// own except while a possess lasts — see [`Self::tick_view`]. Zero means
    /// the server is driving the body itself and nothing may be sent.
    ///
    /// Seeded to the character in [`Self::enter_world`] rather than defaulted
    /// here, because the *reason* it is the character is a packet this client
    /// sends: `CMSG_SET_ACTIVE_MOVER` goes out there unconditionally.
    fn mover_guid(&self) -> u64 {
        self.local.mover_guid
    }

    fn snapshot_of(&self, guid: u64) -> Option<(Position, Speeds)> {
        let world = lock(&self.world);
        let entity = world.get(guid)?;
        Some((entity.position?, entity.speeds.unwrap_or_default()))
    }

    fn position_updates_of(&self, guid: u64) -> u32 {
        lock(&self.world)
            .get(guid)
            .map(|e| e.position_updates)
            .unwrap_or(0)
    }

    fn player_snapshot(&self) -> Option<(Position, Speeds)> {
        let world = lock(&self.world);
        let player = world.player()?;
        Some((player.position?, player.speeds.unwrap_or_default()))
    }
}

enum Flow {
    Continue,
    Stop,
}

/// `elapsed` in whole milliseconds, keeping the remainder in `carry` for the
/// next call.
///
/// Split out of [`SessionLoop::step_ms`] so the accounting can be tested without
/// a clock — the failure it exists to prevent is a *slow drift*, which is
/// invisible in any single step and only shows up after a few hundred.
/// One elapsed duration as the two clocks a step runs on — see [`Step`] and
/// [`MAX_STEP`].
///
/// A free function beside [`whole_ms`] rather than a method, so the rule can be
/// asserted without a socket: the whole content of it is that the world takes
/// the real time and the mover takes the clamped one, and a test that restates
/// the clamp instead of calling this is a test of its own arithmetic.
/// A step, cut into pieces no longer than a nominal tick — see
/// [`SessionLoop::tick_world`].
///
/// **Bounded at [`MAX_CATCH_UP`] in total**, which is the one place time is
/// still dropped: a thread that was away for an hour (a suspended process, a
/// debugger) would otherwise spend thousands of slices walking every entity in
/// the world through terrain lookups it will discard, and at that point the
/// session's next packet restates everything anyway. A stall long enough to
/// hit it is not the case this exists for; the 250-to-2000 ms one is.
fn slices(dt_ms: u32) -> impl Iterator<Item = u32> {
    let slice = MAX_STEP.as_millis() as u32;
    let mut left = dt_ms.min(MAX_CATCH_UP.as_millis() as u32);
    std::iter::from_fn(move || {
        if left == 0 {
            return None;
        }
        let take = left.min(slice);
        left -= take;
        Some(take)
    })
}

/// What [`platform_under`] decided.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Underfoot {
    /// The platform to carry the character against, or `None` for a character
    /// on the ground.
    pub platform: Option<crate::state::movement::Platform>,
    /// The deck is being held without the world holding a hull for it, so there
    /// is no floor either — see `OnMap::floor`, which reads this.
    pub adrift: bool,
}

/// **Which moving platform is carrying the character this tick.**
///
/// A passenger stays aboard until the world can show they stepped off, and it
/// can only show that while the deck's own hull is standing.
/// [`World::platform`] answers nothing in three different situations and they
/// do not mean the same thing:
///
/// * the character has walked ashore, and the deck's hull is right there,
///   unstood-on;
/// * the deck's hull is not built — a frame whose hull budget ran out, or a
///   `.wmo` transport the renderer stopped hulling because the character is no
///   longer near where it now is;
/// * the deck is not on this map any more, so the world holds nothing about it
///   at all.
///
/// Only the first is a step ashore, and [`World::platform_hulled`] is what
/// separates it from the other two. Reading either of the others as a step
/// ashore un-boards the passenger, and `Transport::TeleportTransport` moves
/// `m_passengers` and nobody else — so the deck goes and the character is left
/// where it was. The two remaining arms therefore keep the relationship:
///
/// * **hull absent, deck still placed**: carry against where it is now. This
///   is a same-map transport teleport, and this client following it is the only
///   thing that moves the passenger, because the server sends nothing for one:
///   `TeleportTransport` relocates a passenger who does not change map with
///   `Unit::TeleportPositionRelocation`, which is server side only. The
///   Grom'Gol–Undercity zeppelin jumps 13,700 yards on one map 173.4 s into its
///   cycle, and a client that does not follow it leaves the character hanging
///   in the air over Stranglethorn.
/// * **not placed at all**: hold the last placement, which is the "standing
///   still" arm of the carry and moves nobody. This is the window between this
///   client's own route saying a boat has crossed to the other continent and
///   `Player::TeleportTo` arriving.
///
/// While either arm is taken there is no floor either — keeping the
/// relationship while answering the sea bed for the height left the character
/// falling off the deck they were still officially standing on. Both hold for
/// as long as the object manager still knows the deck as a transport, and are
/// bounded by [`PLATFORM_HOLD`] only once it does not — a deck that never comes
/// back dies with the server's own out-of-range block, and one that is merely
/// elsewhere must not be un-boarded on a clock (see the constant).
///
/// `moves` is the [`UPDATEFLAG_TRANSPORT`] filter, which the object manager
/// holds and the collision world does not: without it a character standing on
/// any door or chest would be boarded onto it and the server would refuse the
/// guid. It is a closure because this function must not take the world lock —
/// the caller already holds what it needs.
///
/// [`UPDATEFLAG_TRANSPORT`]: crate::state::update::update_flags::TRANSPORT
pub(crate) fn platform_under(
    ground: &dyn World,
    map_id: u32,
    at: crate::state::update::Position,
    ferry: Option<crate::state::movement::Ferry>,
    moves: impl Fn(u64) -> bool,
    now: Instant,
    held_since: &mut Option<Instant>,
) -> Underfoot {
    if let Some(platform) = ground
        .platform(map_id, at.x, at.y, at.z)
        .filter(|platform| moves(platform.guid))
    {
        *held_since = None;
        return Underfoot { platform: Some(platform), adrift: false };
    }
    let held = (|| {
        let ferry = ferry?;
        // The deck is standing here and the character is not on it.
        if ground.platform_hulled(map_id, ferry.guid) {
            return None;
        }
        // **The hold is bounded only once the deck itself is gone.** While the
        // object manager still knows the entity and still flags it a transport,
        // the deck is real and merely elsewhere — this client's schedule ahead
        // of or behind the server's around a teleport frame, or a far side
        // still loading — and un-boarding on a clock is a no-flag packet:
        // `RemovePassenger`, and `TeleportTransport` then moves everybody but
        // us. vmangos' own frame times are `GenerateWaypoints`' imperfect ones
        // with only the *period* overridden from the database, so the two ends
        // can disagree about the teleport moment by more than any fixed grace.
        // A character genuinely left behind still swims rather than hovering:
        // the entity dies with the server's own out-of-range block, and
        // [`PLATFORM_HOLD`] runs from there.
        let since = *held_since.get_or_insert(now);
        if !moves(ferry.guid) && now.saturating_duration_since(since) >= PLATFORM_HOLD {
            return None;
        }
        Some(
            ground
                .platform_of(map_id, ferry.guid)
                .unwrap_or_else(|| ferry.platform()),
        )
    })();
    if held.is_none() {
        *held_since = None;
    }
    Underfoot { platform: held, adrift: held.is_some() }
}

fn split_step(carry: &mut f64, elapsed: Duration) -> Step {
    let world_ms = whole_ms(carry, elapsed);
    // …and the local character's own step is clamped, because the anticheat
    // compares the distance walked against the `ctime` the packet carries and a
    // multi-second stride out of a stall is a teleport to it.
    let mover_ms = world_ms.min(MAX_STEP.as_millis() as u32);
    Step { world_ms, mover_ms }
}

fn whole_ms(carry: &mut f64, elapsed: Duration) -> u32 {
    *carry += elapsed.as_secs_f64() * 1000.0;
    let whole = carry.floor();
    *carry -= whole;
    whole as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in world holding one game object: where it is, whether its hull
    /// is standing, and whether the character is on that hull. The three states
    /// [`platform_under`] has to tell apart are combinations of the last two.
    struct Deck {
        /// Where the object is, or `None` for one this map holds nothing about.
        placed: Option<crate::state::movement::Platform>,
        /// Whether its hull is in the collision world.
        hulled: bool,
        /// Whether the character is standing on that hull.
        under: bool,
    }

    impl World for Deck {
        fn floor(&self, _map_id: u32, _x: f32, _y: f32, _z: f32) -> Option<f32> {
            Some(0.0)
        }
        fn platform(
            &self,
            _map_id: u32,
            _x: f32,
            _y: f32,
            _z: f32,
        ) -> Option<crate::state::movement::Platform> {
            (self.under && self.hulled).then_some(self.placed?)
        }
        fn platform_of(
            &self,
            _map_id: u32,
            guid: u64,
        ) -> Option<crate::state::movement::Platform> {
            self.placed.filter(|at| at.guid == guid)
        }
        fn platform_hulled(&self, _map_id: u32, guid: u64) -> bool {
            self.hulled && self.placed.is_some_and(|at| at.guid == guid)
        }
    }

    const DECK: u64 = 0xF110_0000_0000_0007;

    fn deck_at(position: [f32; 3]) -> crate::state::movement::Platform {
        crate::state::movement::Platform { guid: DECK, position, facing: 0.0 }
    }

    /// The character, standing five yards along the deck from its origin.
    fn aboard(deck: crate::state::movement::Platform) -> crate::state::movement::Ferry {
        crate::state::movement::Ferry::aboard(
            deck,
            crate::state::update::Position {
                x: deck.position[0] + 5.0,
                y: deck.position[1],
                z: deck.position[2],
                orientation: 0.0,
            },
        )
    }

    fn standing_on(ferry: crate::state::movement::Ferry) -> crate::state::update::Position {
        ferry.carry(ferry.platform())
    }

    /// Standing on the deck: the hull answers and nothing is being held.
    #[test]
    fn a_character_on_a_hull_is_carried_by_it() {
        let deck = deck_at([100.0, 0.0, 10.0]);
        let ferry = aboard(deck);
        let world = Deck { placed: Some(deck), hulled: true, under: true };
        let mut held = None;
        let out = platform_under(
            &world,
            0,
            standing_on(ferry),
            Some(ferry),
            |_| true,
            Instant::now(),
            &mut held,
        );
        assert_eq!(out, Underfoot { platform: Some(deck), adrift: false });
        assert!(held.is_none(), "nothing is being held");
    }

    /// **A game object that does not move never boards anybody.** The filter is
    /// `UPDATEFLAG_TRANSPORT`, which only the object manager holds — without it
    /// a character standing on a door or a chest would be boarded onto it and
    /// the server would refuse the guid.
    #[test]
    fn a_stationary_game_object_is_not_a_platform() {
        let deck = deck_at([100.0, 0.0, 10.0]);
        let world = Deck { placed: Some(deck), hulled: true, under: true };
        let mut held = None;
        let out = platform_under(
            &world,
            0,
            crate::state::update::Position { x: 105.0, y: 0.0, z: 10.0, orientation: 0.0 },
            None,
            |_| false,
            Instant::now(),
            &mut held,
        );
        assert_eq!(out, Underfoot { platform: None, adrift: false });
    }

    /// **Walking ashore un-boards, and the hull standing there is the whole of
    /// the evidence.** The deck is still placed and still hulled; the character
    /// is simply not on it.
    #[test]
    fn a_character_who_steps_off_a_standing_hull_is_ashore() {
        let deck = deck_at([100.0, 0.0, 10.0]);
        let ferry = aboard(deck);
        let world = Deck { placed: Some(deck), hulled: true, under: false };
        let mut held = None;
        let out = platform_under(
            &world,
            0,
            crate::state::update::Position { x: 60.0, y: 0.0, z: 0.0, orientation: 0.0 },
            Some(ferry),
            |_| true,
            Instant::now(),
            &mut held,
        );
        assert_eq!(out, Underfoot { platform: None, adrift: false });
    }

    /// **A same-map transport teleport carries the passenger with it.** That is
    /// the Grom'Gol to Undercity zeppelin: 13,700 yards on one map, with no
    /// packet behind it, because `TeleportTransport` relocates a passenger who
    /// does not change map server side and tells nobody.
    ///
    /// The client sees it as the deck's placement jumping while its hull stops
    /// being built — the renderer only hulls a transport within 150 yards of
    /// the character, and after the jump it is thirteen thousand away.
    #[test]
    fn a_deck_that_teleports_on_one_map_takes_its_passenger() {
        let gromgol = deck_at([-11440.0, -401.0, 210.7]);
        let ferry = aboard(gromgol);
        let standing = standing_on(ferry);

        let undercity = deck_at([2301.1, -944.4, 266.6]);
        let world = Deck { placed: Some(undercity), hulled: false, under: false };
        let mut held = None;
        let out = platform_under(
            &world,
            0,
            standing,
            Some(ferry),
            |_| true,
            Instant::now(),
            &mut held,
        );
        assert_eq!(out.platform, Some(undercity), "left behind over Stranglethorn");
        assert!(out.adrift, "there is no floor under a deck the world is not holding");

        // …and the carry that answer feeds puts the character on the deck
        // rather than leaving them where they were standing.
        let mut mover = crate::state::movement::Mover::new(standing, Speeds::default());
        mover.carry_platform(Some(gromgol));
        mover.stow_platform();
        assert!(mover.carry_platform(out.platform), "the carry did not happen");
        let landed = mover.position();
        assert!((landed.x - (undercity.position[0] + 5.0)).abs() < 1e-2, "{landed:?}");
        assert!((landed.y - undercity.position[1]).abs() < 1e-2, "{landed:?}");
    }

    /// **A deck this map holds nothing about is stood on anyway.** That is the
    /// window between this client's own route saying a boat has crossed to the
    /// other continent and `TeleportTo` arriving: the boat is not drawn here,
    /// so its hull and its placement are both gone, and un-boarding leaves the
    /// character in the sea.
    ///
    /// **And it is held for as long as the world still knows the deck.** The
    /// two ends do not agree about the teleport moment to any fixed bound —
    /// vmangos' frame times are its own `GenerateWaypoints` output with only
    /// the period overridden — and an un-board on a clock is a no-flag packet:
    /// `RemovePassenger`, and the boat crosses without us. [`PLATFORM_HOLD`]
    /// bounds only the case where the deck's entity itself is gone.
    #[test]
    fn a_deck_the_world_has_lost_is_held_while_its_entity_lives() {
        let deck = deck_at([-3700.0, -575.0, 0.0]);
        let ferry = aboard(deck);
        let world = Deck { placed: None, hulled: false, under: false };
        let mut held = None;
        let start = Instant::now();
        let out = platform_under(
            &world,
            0,
            standing_on(ferry),
            Some(ferry),
            |_| true,
            start,
            &mut held,
        );
        assert_eq!(out.platform, Some(deck), "the last placement is the held one");
        assert!(out.adrift);
        assert_eq!(held, Some(start), "the hold started here");

        // Well past the grace, with the entity still alive: still held.
        let still = platform_under(
            &world,
            0,
            standing_on(ferry),
            Some(ferry),
            |_| true,
            start + PLATFORM_HOLD * 12,
            &mut held,
        );
        assert_eq!(still.platform, Some(deck), "un-boarded on a clock");

        // …and it is bounded once the entity is gone, or a boat the server
        // destroys leaves the character hovering over open water for the rest
        // of the session.
        let expired = platform_under(
            &world,
            0,
            standing_on(ferry),
            Some(ferry),
            |_| false,
            start + PLATFORM_HOLD,
            &mut held,
        );
        assert_eq!(expired, Underfoot { platform: None, adrift: false });
        assert!(held.is_none(), "the clock is cleared with the deck");
    }

    #[test]
    fn intervals_satisfy_the_servers_heartbeat_rule() {
        // vmangos raises CHEAT_TYPE_SKIPPED_HEARTBEATS above 1000 ms between
        // packets from a moving client. A heartbeat interval plus a whole tick
        // of overrun is the worst gap this loop can produce.
        assert!(HEARTBEAT + TICK < Duration::from_millis(1000));
    }

    #[test]
    fn a_tick_leaves_time_for_work_after_reading() {
        assert!(READ_SLICE < TICK);
    }

    /// **A name a window is waiting on leaves at once; an unanswered one is
    /// re-asked on the beat and not before.**
    ///
    /// The whole of [`worth_asking`], and the reason it is a free function: this
    /// is a rule with no socket in it. The fault it pins is the one that made a
    /// vendor take seconds to say what it was selling — and the second, quieter
    /// one it retires, which is the same twenty packets going out every two
    /// seconds for as long as the answers were late.
    #[test]
    fn a_fresh_query_goes_out_at_once_and_a_stale_one_waits_for_the_beat() {
        let mut asked: HashSet<u32> = HashSet::new();
        // First sight of 2589, on an ordinary tick: it goes.
        assert!(worth_asking(&mut asked, 2589, false));
        // …and it does not go again on every tick until the answer lands.
        assert!(!worth_asking(&mut asked, 2589, false));
        assert!(!worth_asking(&mut asked, 2589, false));
        // On the beat it is re-asked, because a query has no ack and an answer
        // that never came has to be asked for again.
        assert!(worth_asking(&mut asked, 2589, true));
        // A different entry is still fresh whatever the beat is doing.
        assert!(worth_asking(&mut asked, 858, false));
    }

    /// The latency the split is worth, stated rather than left to the reader to
    /// divide: a fresh query used to wait out [`QUERY_INTERVAL`] and now waits
    /// out one [`TICK`].
    #[test]
    fn the_beat_is_far_longer_than_a_tick() {
        assert!(QUERY_INTERVAL > TICK * 40);
    }

    /// The sub-millisecond remainder has to be carried, not dropped.
    ///
    /// A step is whatever the scheduler gave us — 15.6 ms on Windows, rarely a
    /// whole number — and the world advances in `u32` milliseconds. Truncating
    /// each step loses up to 1 ms of every one, which is a *systematic* slow of
    /// several percent applied to every spline in the world and to nothing else.
    /// It cannot be seen in one step; over a minute a patrolling creature is
    /// seconds behind where the server has it.
    #[test]
    fn the_sub_millisecond_remainder_is_carried_rather_than_lost() {
        let mut carry = 0.0;
        let step = Duration::from_nanos(15_625_000); // 15.625 ms, Windows' own
        let total: u64 = (0..64).map(|_| u64::from(whole_ms(&mut carry, step))).sum();
        // 64 x 15.625 ms is exactly one second, and not one millisecond of it
        // may go missing.
        assert_eq!(total, 1000, "the world clock drifts slow");
    }

    /// **A stall is two different answers, and giving both of them the clamped
    /// one was a cumulative desync.**
    ///
    /// The *character* walks off a stall over several steps rather than in one
    /// stride, and clamping errs short — the server's `CheckSpeedHack` allows
    /// less distance than the packet's `ctime` claims, never more.
    ///
    /// The *world* does not get that treatment, because nothing in it is this
    /// client's to under-travel: the server really did advance every spline by
    /// the whole five seconds, and a client that advances them by a quarter of
    /// one has put every creature in view permanently 4.75 s behind. A chase
    /// spline gets that wiped by its next `SMSG_MONSTER_MOVE`; a **cyclic**
    /// one, which the server states once and never mentions again, keeps it for
    /// the rest of the session and adds the next stall's to it.
    #[test]
    fn a_stall_is_clamped_for_the_character_and_not_for_the_world() {
        let mut carry = 0.0;
        let step = split_step(&mut carry, Duration::from_secs(5));
        assert_eq!(step.mover_ms, MAX_STEP.as_millis() as u32);
        assert_eq!(step.world_ms, 5_000, "the world lost {} ms", 5_000 - step.world_ms);
    }

    /// **A stall reaches the world in tick-sized pieces that add up to all of
    /// it**, which is what lets the splines have the whole step without any
    /// dead reckoning taking a thirty-eight-yard stride through a wall.
    #[test]
    fn a_stall_is_sliced_and_the_slices_add_up() {
        let slice = MAX_STEP.as_millis() as u32;
        for stall in [1u32, 25, 250, 251, 600, 5_000] {
            let cut: Vec<u32> = slices(stall).collect();
            assert_eq!(cut.iter().sum::<u32>(), stall, "{stall} ms lost time");
            assert!(cut.iter().all(|s| *s <= slice && *s > 0), "{stall} ms: {cut:?}");
        }
        // …and an absurd one is bounded rather than walked, which is the one
        // place time is still dropped and is deliberate. See `slices`.
        let capped: u32 = slices(3_600_000).sum();
        assert_eq!(capped, MAX_CATCH_UP.as_millis() as u32);
        assert_eq!(slices(0).count(), 0);
    }

    /// …and in every ordinary tick the two are the same number, so nothing
    /// about the split can be felt outside a stall.
    #[test]
    fn an_ordinary_tick_is_one_number_twice() {
        let mut carry = 0.0;
        for _ in 0..64 {
            let step = split_step(&mut carry, Duration::from_nanos(15_625_000));
            assert_eq!(step.world_ms, step.mover_ms);
        }
    }

    #[test]
    fn the_published_tick_matches_the_real_one() {
        assert_eq!(TICK_SECS, TICK.as_secs_f32());
    }

    /// The wire rate is set by the send intervals, not by the tick, so halving
    /// the tick must not have moved the server's heartbeat rule.
    #[test]
    fn a_faster_tick_does_not_change_the_packet_rate() {
        assert!(HEARTBEAT > TICK, "the heartbeat is what paces the socket");
        assert!(FACING_INTERVAL > TICK, "a held turn would flood the socket");
    }
}
