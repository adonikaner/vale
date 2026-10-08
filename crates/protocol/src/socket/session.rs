//! A live world session: a background thread that owns the socket, keeps it
//! alive, simulates the player, and holds the world state everyone else reads.
//!
//! Unlike a snapshot (connect, read for eight seconds, drop the socket), a
//! session keeps the socket open, updates objects, and moves the character.
//!
//! Structure:
//!
//! * One thread owns the [`WorldSession`]. A world socket is a single ordered
//!   stream with a stateful header cipher, so it cannot be shared; sharing it
//!   would need a mutex around every read and would still have to arbitrate
//!   who gets the next packet.
//! * Readers get an [`ObjectManager`] behind a mutex. The thread holds the
//!   lock only while folding a packet in or advancing splines, so a UI polling
//!   at 20 Hz does not contend meaningfully. Socket writes always happen with
//!   the lock released.
//! * Input arrives as commands, not as direct calls. A keypress from a WebView
//!   must not turn into a socket write on the WebView's thread.
//!
//! The tick is 25 ms. That is short enough that the 500 ms movement heartbeat
//! and the 30 s keepalive are sent on time and a keypress is picked up within
//! half a frame, and long enough to cost little CPU. The tick does not set the
//! packet rate: every send is gated by its own interval, so halving the tick
//! does not add socket traffic.
//!
//! ## The simulation is advanced by measured time, not by [`TICK`]
//!
//! A loop that sleeps for `TICK - work` does not run every `TICK`. Windows'
//! default scheduler granularity is about 15.6 ms, so `thread::sleep` rounds up
//! to the next scheduler tick and a loop asking for 25 ms gets between 16 and
//! 32 ms. Advancing the mover by a fixed `TICK` while the wall clock advances
//! by a different amount makes the character's speed vary by tens of percent
//! from one step to the next. Each logged number is correct, so a log does not
//! show it; on screen the character judders.
//!
//! The server also checks against measured time. `CheckSpeedHack`
//! extrapolates `Unit::ExtrapolateMovement` over the client's `ctime` delta
//! and accumulates `m_overspeedDistance` only when the reported distance
//! exceeds it. Here `ctime` is real elapsed milliseconds. A fixed step under a
//! longer real tick reports less travel than the time it claims, which the
//! server accepts but which is wrong; advancing by the same clock that `ctime`
//! comes from makes the two agree exactly. (`Anticheat.MaxAllowedDesync`
//! defaults to 0, so the server does not clamp that delta.)

use crate::play::chat::{ChatType, Language};
use crate::socket::handler::{apply_packet, Incoming, LocalState, PumpStats, Replies};
use crate::play::wdb::{Caches, Kind};
use crate::play::areatrigger::{TriggerTable, TriggerWatch};
use crate::state::movement::{Controls, Footing, Input, MovementInfo, Mover, Speeds};
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

/// The ground and obstacles on a given map, which the local simulation walks
/// on. The caller supplies it because the protocol crate does not read ADT or
/// WMO files.
///
/// The map id is a parameter rather than fixed in the implementation because a
/// far teleport changes it mid-session. A lookup left pointing at the previous
/// map gives wrong answers that look valid: Kalimdor's tile (37, 47) exists
/// and has a height, so the character would be placed on Azeroth's ground
/// while standing in Tanaris, with no error. Buildings have the same problem:
/// tile coordinates repeat across continents, so a stale collider becomes an
/// invisible wall in an empty field.
///
/// This is [`crate::state::movement::Footing`] with the map id added; the
/// session binds the current map id and hands the rest to the mover.
pub trait World: Send {
    fn floor(&self, map_id: u32, x: f32, y: f32, z: f32) -> Option<f32>;

    /// The local character's position, given once a tick before anything asks
    /// about the ground there.
    ///
    /// This is the only position an implementation may centre its caches on
    /// and read ahead of. [`Self::floor`] cannot serve that purpose: it is also
    /// asked for every dead-reckoned unit, so a cache centred on the last
    /// caller would move with each unit. The default does nothing, for a world
    /// with nothing to prepare.
    fn focus(&self, _map_id: u32, _x: f32, _y: f32) {}

    fn step(&self, _map_id: u32, _from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
        to
    }

    /// The liquid surface over a point: [`Footing::liquid`] with the map id
    /// added.
    fn liquid(&self, _map_id: u32, _x: f32, _y: f32) -> Option<f32> {
        None
    }

    /// The game object whose hull is directly under this point, and where that
    /// object is now.
    ///
    /// This is the only method here that returns an object rather than a
    /// height. A character standing on a moving platform has to name it,
    /// because `MOVEFLAG_ONTRANSPORT` carries a guid and a position in that
    /// object's own frame. See [`crate::state::movement::Ferry`].
    ///
    /// It answers for a door or a chest as well as for a lift, because the
    /// collision side does not know which objects move. The caller filters on
    /// `UPDATEFLAG_TRANSPORT`, which is the only data that marks an object as
    /// a transport.
    ///
    /// The default is `None`, so the CLI's terrain-only world and tests
    /// without transports never make the character a passenger.
    fn platform(
        &self,
        _map_id: u32,
        _x: f32,
        _y: f32,
        _z: f32,
    ) -> Option<crate::state::movement::Platform> {
        None
    }

    /// The same lookup as [`Self::platform`], by guid, for a unit whose spline
    /// is stated in a transport's frame. The only caller is
    /// [`crate::state::movement::Spline::in_world`].
    ///
    /// The default is `None`, which means the platform is not loaded: the
    /// passenger is then left where it was rather than placed at the map's
    /// origin.
    fn platform_of(
        &self,
        _map_id: u32,
        _guid: u64,
    ) -> Option<crate::state::movement::Platform> {
        None
    }

    /// Whether this game object's hull is built and present, as opposed to the
    /// world only having a placement record for it.
    ///
    /// [`Self::platform_of`] answers from the placement record, which exists
    /// for every game object that has been placed, including objects with no
    /// solid geometry and objects whose hull the renderer has not built yet.
    /// This method asks the narrower question. It separates a passenger who has
    /// walked ashore from one whose deck the world has temporarily stopped
    /// holding.
    ///
    /// A character who is aboard and stands on no hull has stepped off the
    /// deck only if the deck's hull was present. With no hull there is no
    /// evidence, and treating it as a step ashore un-boards the passenger. On
    /// a same-map transport teleport that leaves the passenger behind, because
    /// `Transport::TeleportTransport` moves a passenger who does not change map
    /// with `TeleportPositionRelocation` and sends no packet.
    ///
    /// The default is `false`, meaning no evidence. The CLI's terrain-only
    /// world never boards anybody, so the code that depends on this is
    /// unreachable there.
    fn platform_hulled(&self, _map_id: u32, _guid: u64) -> bool {
        false
    }
}

/// A terrain-height closure is a [`World`] with no buildings in it. The CLI
/// supplies one; the renderer supplies a full world with buildings.
impl<F: Fn(u32, f32, f32, f32) -> Option<f32> + Send> World for F {
    fn floor(&self, map_id: u32, x: f32, y: f32, z: f32) -> Option<f32> {
        self(map_id, x, y, z)
    }
}

/// The caller's [`World`], if it has one.
pub type GroundHeight = Box<dyn World>;

/// A [`World`] with the map id already bound, which is the form the mover
/// takes.
///
/// Built on every step rather than cached, because the map id it binds is the
/// part that changes; a teleport that left a stale one behind would walk the
/// character on the continent they have left.
struct OnMap<'a> {
    world: &'a dyn World,
    map_id: u32,
    /// The character is standing on a deck the client cannot currently
    /// locate, so there is no answer about the floor. See [`Footing::floor`]
    /// below.
    ///
    /// False except for a passenger; for a passenger it is true for under a
    /// second, once a cycle. Set by [`SessionLoop::tick_movement`], which is
    /// the only place with that information.
    adrift: bool,
}

impl Footing for OnMap<'_> {
    /// A deck that has left the client's world returns `None` ("no answer"),
    /// not the floor of the sea under it.
    ///
    /// `Footing::floor` returning `None` means the client has no data for this
    /// spot, not that there is no floor, and both halves of the mover hold
    /// their altitude on it. A passenger whose platform the collision world can
    /// no longer place is that case: the character stands on a real object that
    /// this client has temporarily stopped holding. For a boat this happens for
    /// one second a cycle, between its route reaching the other continent and
    /// `ShipTransport::Update` teleporting the passengers.
    ///
    /// Answering from the world there returns the ocean floor, and the
    /// character falls off the deck for the half second in between. The fall
    /// also changes the transport offset, and the server rebuilds the position
    /// from that offset on the far side (`CalculatePassengerPosition`). A
    /// character who fell two yards arrives two yards under the deck, in the
    /// sea, which is the reported bug this prevents.
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

/// How long the character keeps standing on a deck whose entity is gone,
/// before being put back on the ground under them.
///
/// This bound applies only once the object manager no longer knows the
/// ferry's guid as a transport. While it does, the hold has no limit, because
/// the deck is real and only unplaceable. Causes include: this client's
/// schedule running ahead of or behind the server's around a teleport frame
/// (vmangos' frame times are its own imperfect `GenerateWaypoints` output with
/// only the period overridden, so the disagreement has no fixed bound), a far
/// side still loading, or a frame whose hull-rebuild budget ran out. Expiring
/// the hold in any of those cases un-boards the passenger: the next packet
/// carries no transport flag, `HandleMoverRelocation` calls
/// `RemovePassenger`, and `Transport::TeleportTransport` then moves every
/// passenger except this character, leaving it in the ocean.
///
/// The bound stops a lost boat from leaving the session stuck. A transport
/// the server destroys while somebody stands on it is removed on this side by
/// the server's out-of-range block, `moves()` becomes false, and five seconds
/// later the character swims instead of hovering at deck height for the rest
/// of the session. Five seconds is a chosen value, not a measurement.
const PLATFORM_HOLD: Duration = Duration::from_secs(5);

/// How far, in yards, one step's carry may move a passenger before it is
/// counted as a jump. See [`SessionStatus::platform_jumps`].
///
/// A transport travels 30 y/s and a step is clamped to [`MAX_STEP`], so a
/// legitimate ride carries a passenger at most 7.5 yards in one step. The
/// value 250 matches `world::motion`'s bound for the same reason: nothing
/// that travels can reach it.
const PLATFORM_JUMP: f32 = 250.0;

/// How often the loop wakes. This is not how far the simulation advances; see
/// the module comment and [`SessionLoop::step_ms`].
const TICK: Duration = Duration::from_millis(25);

/// [`TICK`] in seconds, for a reader that needs a nominal step before it has
/// two readings of [`SessionStatus::world_ms`] to subtract. A test keeps it
/// equal to `TICK`.
pub const TICK_SECS: f32 = 0.025;

/// How long a tick may spend draining the socket before it does the rest of
/// its work. Bounded so that a busy zone cannot starve movement.
const READ_SLICE: Duration = Duration::from_millis(12);

/// The largest step the local character takes in one go, however long the
/// thread was actually away.
///
/// A stall (a login burst, or the renderer starving this thread while it
/// uploads a tile) should not be walked off in a single stride. Clamping
/// under-travels relative to the `ctime` the packet carries, which is the side
/// `CheckSpeedHack` accepts: the server allows less distance than time, never
/// more.
///
/// This constant previously clamped the world clock as well, which caused a
/// cumulative desync. Everything the server moves (every spline, and so every
/// creature in view) was advanced by the same clamped number, so each stall
/// over a quarter of a second left each of them permanently that far behind
/// the server's position. A chasing creature's error is corrected by its next
/// `SMSG_MONSTER_MOVE`; a patrolling guard on a cyclic spline is never
/// corrected, because a cyclic path is sent once (see
/// [`crate::state::movement::Spline::cyclic`]). The losses add up over a
/// session (a login burst alone is several seconds), and the creature is drawn
/// yards from where it is fighting. The world clock now takes the whole
/// elapsed time and only the mover is clamped; see [`SessionLoop::step_ms`]
/// and [`SessionStatus::stalls`].
const MAX_STEP: Duration = Duration::from_millis(250);

/// The most world time one tick advances after a stall, however long the
/// thread was really away. The reason is given at [`slices`].
const MAX_CATCH_UP: Duration = Duration::from_secs(10);

/// The two clocks of one step.
///
/// They are equal in every ordinary tick and differ only after a stall; see
/// [`MAX_STEP`] for the error that a single clock caused there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Step {
    /// Real elapsed milliseconds: how far the server has advanced, and so what
    /// every spline and every dead reckoning is advanced by.
    world_ms: u32,
    /// What the local character may walk off in one stride, clamped to
    /// [`MAX_STEP`] because the anticheat measures it against real time.
    mover_ms: u32,
}

/// Movement heartbeat interval.
///
/// vmangos raises `CHEAT_TYPE_SKIPPED_HEARTBEATS` when consecutive packets from
/// a moving client are more than 1000 ms apart, with the comment "client
/// should send heartbeats every 500ms". 400 leaves room for a slow tick.
const HEARTBEAT: Duration = Duration::from_millis(400);

/// Minimum gap between updates sent because the facing changed, so that
/// holding a turn key does not send one packet per tick.
///
/// The value is 100 ms, reduced from 250, because this interval paces turning
/// while running as well as while standing (see [`SessionLoop::tick_movement`]).
/// While running it bounds how far the server's straight-line extrapolation
/// diverges from the arc actually walked. A quarter of a second of a 180°/s
/// turn at a run is 45° of unreported heading; a tenth is 18°, which keeps the
/// separation inside `CheckSpeedHack`'s 10% allowance. The 1.12.1 client also
/// sends ten packets a second while turning.
const FACING_INTERVAL: Duration = Duration::from_millis(100);

/// `CMSG_PING` interval. The ping detects a dead link early; without it a
/// stalled session shows up only later as an unexplained disconnect.
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// How often to re-send queries for entries that are visible but still have
/// no name.
const QUERY_INTERVAL: Duration = Duration::from_secs(2);

/// How long to wait for the login burst to produce a player object.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10);

/// The five requests a loot window can send to the socket.
///
/// A separate enum rather than five `Command` variants; the reason is given at
/// [`Command::Loot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LootVerb {
    /// `CMSG_LOOT` on a body.
    Open(u64),
    /// `CMSG_AUTOSTORE_LOOT_ITEM`, by the server's index, never the row.
    Take(u8),
    /// `CMSG_LOOT_MONEY`, which has no body.
    TakeMoney,
    /// `CMSG_LOOT_RELEASE`. The server discards the guid.
    Release(u64),
    /// `CMSG_LOOT_ROLL`: need, greed or pass on one row of a group roll.
    ///
    /// Carries the body and the slot, not the interface's `rollID`; see
    /// [`crate::play::lootroll`], which shows the id is a local counter.
    Roll {
        guid: u64,
        item_slot: u32,
        vote: crate::play::lootroll::RollVote,
    },
}

/// The nine requests a quest conversation sends to the socket.
///
/// A separate enum rather than nine `Command` variants, for the same reason
/// as [`LootVerb`]: one subject, and the loop distinguishes them only by which
/// method to call. See [`crate::play::quest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestVerb {
    /// `CMSG_QUESTGIVER_STATUS_QUERY`: the marker over a giver's head.
    Status(u64),
    /// `CMSG_QUESTGIVER_HELLO`: right-click a giver.
    Hello(u64),
    /// `CMSG_QUESTGIVER_QUERY_QUEST`: show one quest's details.
    Details { guid: u64, quest_id: u32 },
    /// `CMSG_QUESTGIVER_ACCEPT_QUEST`.
    Accept { guid: u64, quest_id: u32 },
    /// `CMSG_QUESTGIVER_COMPLETE_QUEST`: hand the quest in.
    Complete { guid: u64, quest_id: u32 },
    /// `CMSG_QUESTGIVER_REQUEST_REWARD`: show the reward page again.
    RequestReward { guid: u64, quest_id: u32 },
    /// `CMSG_QUESTGIVER_CHOOSE_REWARD`: take one reward, by zero-based index
    /// into the choices.
    ChooseReward {
        guid: u64,
        quest_id: u32,
        choice: u32,
    },
    /// `CMSG_QUEST_QUERY`: a quest's text and data by id (for example quest
    /// 47). The only verb with no giver.
    Query(u32),
    /// `CMSG_QUESTLOG_REMOVE_QUEST`, by zero-based quest log slot, which is not
    /// the quest id.
    Abandon(u8),
}

/// The requests a gossip, vendor, trainer, stable or bank window sends to the
/// socket.
///
/// See [`crate::play::gossip`], which owns the wire formats, including the
/// one body in the game whose fields are in reverse order
/// ([`crate::play::gossip::npc_text_query_body`]), and [`crate::play::trainer`],
/// which owns the two trainer verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpcVerb {
    /// `CMSG_GOSSIP_HELLO`: right-click anything with the gossip flag.
    GossipHello(u64),
    /// `CMSG_GOSSIP_SELECT_OPTION`, by the server's index.
    GossipSelect { guid: u64, option: u32 },
    /// `CMSG_NPC_TEXT_QUERY`: the text behind a menu's text id.
    TextQuery { text_id: u32, guid: u64 },
    /// `CMSG_BINDER_ACTIVATE`: confirm binding the hearthstone to this inn.
    ///
    /// The answer to `SMSG_BINDER_CONFIRM`, and the packet that performs the
    /// bind; before it arrives the server has only closed the gossip window.
    /// The guid must be the innkeeper's; see
    /// [`crate::play::bindpoint::binder_activate_body`].
    BinderActivate(u64),
    /// `CMSG_PAGE_TEXT_QUERY`: the text of page N. The guid is the object
    /// being read and is optional on the wire; see
    /// [`crate::play::pagetext::page_text_query_body`].
    PageTextQuery { page_id: u32, guid: Option<u64> },
    /// `CMSG_LIST_INVENTORY`: open the shop.
    ListInventory(u64),
    /// `CMSG_BUY_ITEM`: `count` items, by item entry.
    Buy { vendor: u64, entry: u32, count: u8 },
    /// `CMSG_SELL_ITEM`, by the item object's guid; a count of 0 sells the
    /// stack.
    Sell { vendor: u64, item: u64, count: u8 },
    /// `CMSG_BUYBACK_ITEM`, by the wire's slot number, which starts at 69.
    Buyback { vendor: u64, wire_slot: u32 },
    /// `CMSG_REPAIR_ITEM`: the armourer and one item's guid, or 0 for all
    /// items, which is what `RepairAllItems()` sends. See
    /// [`crate::play::gossip::repair_item_body`], which also explains why there
    /// is no reply.
    Repair { vendor: u64, item: u64 },
    /// `CMSG_TRAINER_LIST`: open the training window.
    TrainerList(u64),
    /// `CMSG_TRAINER_BUY_SPELL`, by the service spell id from the list, which
    /// is the teaching spell rather than the spell that is learned.
    TrainerBuy { trainer: u64, spell: u32 },
    /// `MSG_LIST_STABLED_PETS`: ask for the stable window again. It is sent on
    /// an `MSG_` opcode, the same opcode the answer arrives under.
    StableList(u64),
    /// `CMSG_STABLE_PET`: put the active pet away. The server picks the slot;
    /// the client names none.
    StablePet(u64),
    /// `CMSG_UNSTABLE_PET`: take a pet out, by pet number. Refused while a pet
    /// is summoned; [`Self::StableSwap`] covers that case.
    UnstablePet { stable: u64, pet_number: u32 },
    /// `CMSG_STABLE_SWAP_PET`: exchange the active pet for a stabled one.
    StableSwap { stable: u64, pet_number: u32 },
    /// `CMSG_BUY_STABLE_SLOT`: the next slot, at `StableSlotPrices.dbc`'s
    /// price. The client checks the money first and the server checks it again.
    BuyStableSlot(u64),
    /// `CMSG_BANKER_ACTIVATE`: open the bank, at a banker with no gossip bit.
    /// A banker with the bit opens the same window through its menu. See
    /// [`crate::play::bank`].
    BankerActivate(u64),
    /// `CMSG_BUY_BANK_SLOT`: the next bag slot, at `BankBagSlotPrices.dbc`'s
    /// price. Success has no reply: the slot count is a byte of
    /// `PLAYER_BYTES_2` and arrives as an update field.
    BuyBankSlot(u64),
}

/// The seven commands a player can give a pet, and two more requests from the
/// pet panel.
///
/// One enum rather than nine `Command` variants, for the same reason as
/// [`NpcVerb`]: they share one subject, all name the pet's guid, and the
/// dispatch is a table in [`crate::socket::world::WorldSession::pet`].
///
/// [`PetVerb::Action`] is the only verb without a fixed meaning. Pressing a
/// slot sends `CMSG_PET_ACTION` with the slot's packed word unchanged, and
/// every command and reaction button uses the same packet: the bar the server
/// sent already holds "follow" and "aggressive" as slots. A binding such as
/// `PetFollow` is this verb with the word the bar carries, not a separate
/// opcode. See [`crate::play::pet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PetVerb {
    /// `CMSG_PET_ACTION`: press a slot, at a target (0 for none).
    Action { pet: u64, data: u32, target: u64 },
    /// `CMSG_PET_NAME_QUERY`: the name the player gave the pet, by pet number.
    NameQuery { pet_number: u32, pet: u64 },
    /// `CMSG_PET_SET_ACTION`: one move, or a swap of two.
    SetAction { pet: u64, moves: Vec<(u32, u32)> },
    /// `CMSG_PET_SPELL_AUTOCAST`: the autocast toggle on a spell button.
    Autocast { pet: u64, spell_id: u32, on: bool },
    /// `CMSG_PET_STOP_ATTACK`.
    StopAttack(u64),
    /// `CMSG_PET_ABANDON`: release the pet permanently, not dismiss it.
    Abandon(u64),
    /// `CMSG_PET_UNLEARN`: reset the pet's skills for money.
    Unlearn(u64),
    /// `CMSG_PET_RENAME`.
    Rename { pet: u64, name: String },
    /// `CMSG_PET_CANCEL_AURA`: cancel one of the pet's buffs.
    CancelAura { pet: u64, spell_id: u32 },
}

/// The three requests the reputation panel sends to the server.
///
/// All three name a reputation-list id rather than a faction id, because the
/// 64-slot table on both sides is keyed by it; see
/// [`crate::play::reputation`]. None is acknowledged: at-war comes back as
/// `SMSG_SET_FACTION_ATWAR`, inactive gets no reply, and the watched faction
/// comes back as an update field. The panel draws from its own copy, and these
/// requests only make the server store the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReputationVerb {
    /// `CMSG_SET_FACTION_ATWAR`.
    AtWar { reputation_list_id: u32, at_war: bool },
    /// `CMSG_SET_FACTION_INACTIVE`: which heading the row is filed under. This
    /// is the only one of the three that is purely cosmetic: the server stores
    /// it in `characters.data` and does nothing else with it.
    Inactive { reputation_list_id: u32, inactive: bool },
    /// `CMSG_SET_WATCHED_FACTION`. `-1` clears it, which is what the
    /// interface's "Show as experience bar" checkbox sends when unchecked.
    Watched { reputation_list_id: i32 },
}

/// The sixteen channel requests the chat frame sends to the socket.
///
/// See [`crate::play::channels`], which owns the bodies. Every verb carries
/// the channel's name, because the wire keys channels by name. The numbers
/// typed in the interface (`/2 hello`) are client-side slots and are never
/// sent.
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

/// The six requests the social panel sends to the socket.
///
/// See [`crate::play::social`], which owns the bodies. The enum is not `Copy`
/// because three verbs carry a string. That follows the wire format: an
/// addition is by name and a removal is by guid, because the client has no
/// guid for somebody it has not met and the server has no name for somebody
/// it already knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocialVerb {
    /// `CMSG_FRIEND_LIST`: ask for the list again. No body. The server sends
    /// the list unprompted only at login, so this is the only way to read it
    /// again.
    List,
    /// `CMSG_ADD_FRIEND`: `/friend <name>`, and the Who panel's Add Friend
    /// button.
    AddFriend(String),
    /// `CMSG_DEL_FRIEND`, by guid, which the panel has from the list.
    DelFriend(u64),
    /// `CMSG_ADD_IGNORE`.
    AddIgnore(String),
    /// `CMSG_DEL_IGNORE`.
    DelIgnore(u64),
    /// `CMSG_WHO`: the search, already parsed from the typed line. Boxed
    /// because it is much the largest variant here and every other one is a
    /// word or a short string.
    Who(Box<crate::play::social::WhoRequest>),
}

/// The twenty-one requests the guild tab, the guild slash commands, the
/// guild popups and the tabard designer send to the socket. See [`crate::play::guild`], which has the
/// layouts.
///
/// A member is named by name in every request that is about one, because the
/// server looks the member up by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildVerb {
    /// `CMSG_GUILD_QUERY`: ask for a guild's name, rank names and emblem.
    Query(u32),
    /// `CMSG_GUILD_ROSTER`: ask for the member list.
    Roster,
    /// `CMSG_GUILD_INFO`: `/ginfo`.
    Info,
    /// `CMSG_GUILD_INVITE`.
    Invite(String),
    /// `CMSG_GUILD_ACCEPT`: accept the pending invitation.
    Accept,
    /// `CMSG_GUILD_DECLINE`: decline it.
    Decline,
    /// `CMSG_GUILD_REMOVE`.
    Remove(String),
    /// `CMSG_GUILD_PROMOTE`.
    Promote(String),
    /// `CMSG_GUILD_DEMOTE`.
    Demote(String),
    /// `CMSG_GUILD_LEADER`: hand the guild to the named member.
    Leader(String),
    /// `CMSG_GUILD_LEAVE`.
    Leave,
    /// `CMSG_GUILD_DISBAND`.
    Disband,
    /// `CMSG_GUILD_MOTD`: set the message of the day.
    Motd(String),
    /// `CMSG_GUILD_INFO_TEXT`: set the guild information text.
    InfoText(String),
    /// `CMSG_GUILD_SET_PUBLIC_NOTE`.
    PublicNote { player: String, note: String },
    /// `CMSG_GUILD_SET_OFFICER_NOTE`.
    OfficerNote { player: String, note: String },
    /// `CMSG_GUILD_RANK`: replace one rank's name and rights.
    Rank { rank: u32, rights: u32, name: String },
    /// `CMSG_GUILD_ADD_RANK`: add a rank below the lowest.
    AddRank(String),
    /// `CMSG_GUILD_DEL_RANK`: delete the lowest rank.
    DelRank,
    /// `MSG_TABARDVENDOR_ACTIVATE`: ask a tabard designer to open its window.
    TabardVendor(u64),
    /// `MSG_SAVE_GUILD_EMBLEM`: save the guild's emblem at a tabard designer.
    SaveEmblem {
        npc: u64,
        emblem: crate::play::guild::Emblem,
    },
}

/// The nine requests about a guild charter. See [`crate::play::petition`],
/// which has the layouts. `item` is the charter item's guid in every request
/// that carries one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PetitionVerb {
    /// `CMSG_PETITION_SHOWLIST`: ask a guild registrar what it sells.
    ShowList(u64),
    /// `CMSG_PETITION_BUY`: buy a charter for a guild of this name.
    Buy { npc: u64, name: String, index: u32 },
    /// `CMSG_PETITION_SHOW_SIGNATURES`: open a charter.
    ShowSignatures(u64),
    /// `CMSG_PETITION_QUERY`: ask for a petition's guild name and owner.
    Query { petition: u32, item: u64 },
    /// `CMSG_PETITION_SIGN`: the charter and the byte after it, which the
    /// 1.12.1 client sends as 1.
    Sign { item: u64, byte: u8 },
    /// `MSG_PETITION_DECLINE`: decline to sign.
    Decline(u64),
    /// `CMSG_OFFER_PETITION`: show the charter to another player.
    Offer { item: u64, player: u64 },
    /// `CMSG_TURN_IN_PETITION`: found the guild.
    TurnIn(u64),
    /// `MSG_PETITION_RENAME`: change the proposed guild name.
    Rename { item: u64, name: String },
}

/// The ten requests the trade window sends to the socket. See
/// [`crate::play::trade`], which has the layouts. Six have no body;
/// `Initiate` names the partner, `SetItem` a bag square by the server's
/// numbering, `ClearItem` a slot, and `SetGold` an amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeVerb {
    /// `CMSG_INITIATE_TRADE`: `InitiateTrade(unit)`.
    Initiate(u64),
    /// `CMSG_BEGIN_TRADE`: the `TRADE` popup's Yes.
    Begin,
    /// `CMSG_BUSY_TRADE`: the refusal a client sends while it already has a
    /// window open.
    Busy,
    /// `CMSG_IGNORE_TRADE`: the refusal the `BlockTrades` CVar sends.
    Ignore,
    /// `CMSG_ACCEPT_TRADE`: the Trade button.
    Accept,
    /// `CMSG_UNACCEPT_TRADE`: `CancelTradeAccept()`, the Cancel button while
    /// accepted.
    Unaccept,
    /// `CMSG_CANCEL_TRADE`: the popup's No, `CloseTrade()`, and closing the
    /// window.
    Cancel,
    /// `CMSG_SET_TRADE_ITEM`: `ClickTradeButton(i)` with an item held.
    SetItem { trade_slot: u8, bag: u8, slot: u8 },
    /// `CMSG_CLEAR_TRADE_ITEM`: `ClickTradeButton(i)` with nothing held, on a
    /// filled slot.
    ClearItem { trade_slot: u8 },
    /// `CMSG_SET_TRADE_GOLD`: `SetTradeMoney(copper)`.
    SetGold(u32),
}

/// The ten requests the mail window sends to the socket.
///
/// See [`crate::play::mail`], which owns the bodies. Opening a mailbox sends
/// no packet, so there is no `Open` verb. The mailbox guid is kept in the
/// client's window state, and every verb below carries it, because the server
/// checks that the character is standing at that mailbox before it acts.
///
/// Five of the verbs differ only in their opcode, so they share
/// [`crate::play::mail::mail_id_body`] rather than each having a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailVerb {
    /// `CMSG_GET_MAIL_LIST`: the inbox. The caller rate-limits it, not this
    /// code; see [`crate::play::mail`]'s note on `CheckInbox`.
    List(u64),
    /// `CMSG_MAIL_MARK_AS_READ`: sent when a letter is read rather than by a
    /// button; `GetInboxText` sends it first.
    MarkAsRead { mailbox: u64, mail_id: u32 },
    /// `CMSG_MAIL_TAKE_MONEY`: the letter's coins.
    TakeMoney { mailbox: u64, mail_id: u32 },
    /// `CMSG_MAIL_TAKE_ITEM`: the parcel. This packet also pays a COD: the
    /// server takes the money on it. The interface shows a confirmation box
    /// before sending it; this code does not.
    TakeItem { mailbox: u64, mail_id: u32 },
    /// `CMSG_MAIL_CREATE_TEXT_ITEM`: keep the letter itself, as an item.
    TakeText {
        mailbox: u64,
        mail_id: u32,
        /// The server reads and discards it; sent because build 5875 sends it
        /// and the server always reads it.
        template_id: u32,
    },
    /// `CMSG_MAIL_RETURN_TO_SENDER`.
    Return { mailbox: u64, mail_id: u32 },
    /// `CMSG_MAIL_DELETE`. Refused for a COD letter, with
    /// `MAIL_ERR_INTERNAL_ERROR` rather than a readable error.
    Delete { mailbox: u64, mail_id: u32 },
    /// `CMSG_ITEM_TEXT_QUERY`: the text of one letter.
    TextQuery { item_text_id: u32, mail_id: u32 },
    /// `CMSG_SEND_MAIL`: a whole letter. Boxed for the same reason as
    /// [`SocialVerb::Who`]: it carries three strings where every other variant
    /// carries two words.
    Send(Box<crate::play::mail::OutgoingMail>),
    /// `MSG_QUERY_NEXT_MAIL_TIME`: whether any mail is waiting. No body; the
    /// answer arrives under the same opcode.
    NextTime,
}

/// The requests the party and raid interface sends to the socket.
///
/// See [`crate::play::group`], which owns the bodies. The enum is not `Copy`
/// because some verbs carry a name. That follows the wire format: an invite
/// and the by-name kick and subgroup moves are strings, and everything else is
/// a guid or nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartyVerb {
    /// `CMSG_GROUP_INVITE`: `/invite <name>`, and all of `InviteByName`.
    Invite(String),
    /// `CMSG_GROUP_ACCEPT`: press Accept on the popup. No body.
    Accept,
    /// `CMSG_GROUP_DECLINE`: press Decline on the popup. No body.
    Decline,
    /// `CMSG_GROUP_DISBAND`: one opcode for both leaving and disbanding. The
    /// server decides which from whether this character leads, so `LeaveParty`
    /// has nothing to decide.
    Leave,
    /// `CMSG_GROUP_UNINVITE`: kick by name, which is what `UninviteUnit(name)`
    /// has and what the wire accepts.
    Uninvite(String),
    /// `CMSG_GROUP_UNINVITE_GUID`: the same kick by guid, for a caller that has
    /// one. Both exist on the wire; neither is a fallback for the other.
    UninviteGuid(u64),
    /// `CMSG_GROUP_SET_LEADER`: promote, by guid. The reply names the new
    /// leader by name.
    SetLeader(u64),
    /// `CMSG_REQUEST_PARTY_MEMBER_STATS`: the only poll for party data, for a
    /// member too far away to be in the object manager.
    RequestStats(u64),
    /// `CMSG_GROUP_RAID_CONVERT`: the Raid tab's convert button. No body. The
    /// client checks two conditions (there is a `party1`, and this character
    /// leads); the server checks them again and answers a refusal with no
    /// packet.
    ConvertToRaid,
    /// `CMSG_GROUP_CHANGE_SUB_GROUP`: move a member into a subgroup, by name,
    /// with a zero-based subgroup. See
    /// [`crate::play::group::change_sub_group_body`].
    ChangeSubGroup { name: String, subgroup: u8 },
    /// `CMSG_GROUP_SWAP_SUB_GROUP`: exchange two members, which is what a drop
    /// onto an occupied slot does.
    SwapSubGroup { name: String, swap_with: String },
    /// `CMSG_GROUP_ASSISTANT_LEADER`: promote to assistant, or demote. By guid
    /// in 1.12, while the two subgroup verbs are by name.
    SetAssistant { guid: u64, assistant: bool },
    /// `MSG_RAID_READY_CHECK` with no body: start a ready check. Leader and
    /// assistants only, which the server enforces.
    StartReadyCheck,
    /// `MSG_RAID_READY_CHECK` with one byte: answer a ready check.
    AnswerReadyCheck(bool),
}

/// The requests a flight master's window sends to the socket.
///
/// See [`crate::play::taxi`]. A separate enum rather than `NpcVerb` variants,
/// for the same reason as [`PartyVerb`]: a multi-hop flight carries a list of
/// nodes, and `NpcVerb` is `Copy`.
///
/// There is no close verb, as there is none for the gossip menu or the
/// trainer: `CloseTaxiMap()` clears the window locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaxiVerb {
    /// `CMSG_TAXIQUERYAVAILABLENODES`: right-click a flight master. The server
    /// answers with the map, or first with a discovery if this node is new.
    QueryNodes(u64),
    /// `CMSG_TAXINODE_STATUS_QUERY`: whether this character knows the
    /// master's node. It is asked about a unit, not about a window, so it is
    /// here and not on the taxi board.
    NodeStatus(u64),
    /// `CMSG_ACTIVATETAXI`: fly, by two node ids. The source is the map's
    /// current node, not a node the player chose.
    Activate { guid: u64, from: u32, to: u32 },
    /// `CMSG_ACTIVATETAXIEXPRESS`: fly a whole route, source first, when no
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
    /// Which movement keys are held from the given moment on.
    ///
    /// The moment is when the keys were read, so the session can apply the
    /// change where the character was at that moment rather than at the start
    /// of the tick that drains it; see [`Mover::advance_through`].
    Controls(Controls, Instant),
    // Several variants below carry a nested verb enum; the reason is given at
    // [`Command::Loot`].
    /// Absolute facing in radians, from mouse-look rather than the turn keys.
    Face(f32),
    /// Absolute body pitch in radians, positive upward. It is the camera's
    /// pitch and has an effect only in water.
    ///
    /// Separate from [`Command::Face`] because the two are different
    /// quantities on the wire: the yaw is in every movement block and is
    /// announced with `MSG_MOVE_SET_FACING`, while the pitch is serialised only
    /// under `MOVEFLAG_SWIMMING` and has no announcing opcode. See
    /// [`Mover::set_pitch`].
    Pitch(f32),
    /// Leave the ground. This is an edge, not a held control: the server allows
    /// one `MSG_MOVE_JUMP` between landings and rejects the rest, so a held key
    /// must produce one command, not sixty.
    ///
    /// Stamped with the moment of the press, for the reason
    /// [`Command::Controls`] is.
    Jump(Instant),
    /// Start swinging at a unit, or `None` to stop.
    Attack(Option<u64>),
    /// The selected unit, or `None` for no selection.
    ///
    /// The client owns the selection and sets it when the player clicks; the
    /// selection ring appears with no round trip, as in the 1.12.1 client. This
    /// command only informs the server, which uses the selection for a few
    /// actions (see [`WorldSession::set_selection`]).
    Target(Option<u64>),
    /// Cast a spell at a unit, or at nobody.
    ///
    /// The target is chosen before the command is sent. Which unit a spell
    /// binds to (the selection, the caster, or none) is a rule read from
    /// `Spell.dbc`, implemented in `vale_assets::tables::spell` where it can be
    /// unit-tested without a socket. This loop sends what it is given.
    Cast {
        spell_id: u32,
        target: crate::play::spells::CastTarget,
    },
    /// Stop a cast that is still in its cast time.
    CancelCast(u32),
    /// Remove one of the character's buffs, by spell id: the right-click on an
    /// icon in the buff bar.
    ///
    /// A command rather than a direct send for the same reason as every other
    /// command here: the interface runs on the render thread and the socket
    /// belongs to the session thread. Nothing is predicted: the aura leaves the
    /// bar when the server reports the slot empty, one round trip later, as in
    /// the 1.12.1 client.
    CancelAura(u32),
    /// Right-click an item in a bag or on the paper doll, in the form the
    /// item's prototype selects.
    ///
    /// The choice (use or wear, and which of the five spell blocks fires) is
    /// made before the command is sent, as for [`Command::Cast`]: it is a rule
    /// about the item, read from [`crate::state::query::ItemInfo`] with no
    /// socket. The slot pair is already in the server's numbering
    /// ([`crate::play::items::server_container_slot`]).
    UseItem {
        bag: u8,
        slot: u8,
        /// `None` sends `CMSG_AUTOEQUIP_ITEM` instead: the item is worn rather
        /// than used, and the server picks the paper-doll slot.
        spell_index: Option<u8>,
        target: crate::play::spells::CastTarget,
    },
    /// Open a bag item that holds loot: `CMSG_OPEN_ITEM`, the third result a
    /// right-click can have.
    ///
    /// A separate command rather than a third form of [`Command::UseItem`]
    /// because it is not a use: it has no spell index and no target, and the
    /// result is a loot window rather than a cast. `ItemInfo::is_openable`, a
    /// rule about the prototype with no socket, decides when to send it. The
    /// slot pair is already in the server's numbering.
    OpenItem { bag: u8, slot: u8 },
    /// Move an item from one place to another. A drop sends this;
    /// a right-click sends [`Command::UseItem`].
    ///
    /// One command covers five opcodes, because the choice between them follows
    /// from the two ends: a swap with a bag on either side is `CMSG_SWAP_ITEM`
    /// and one without is `CMSG_SWAP_INV_ITEM`, a partial stack is
    /// `CMSG_SPLIT_ITEM`, a destination with no slot is
    /// `CMSG_AUTOSTORE_BAG_ITEM`, and no destination is `CMSG_DESTROYITEM`.
    /// Both ends are already in the server's numbering
    /// ([`crate::play::items::server_container_slot`]).
    ///
    /// Nothing is predicted. The item moves when the update block reports the
    /// slots changed, one round trip later. The client releases its cursor
    /// when the packet is sent, so a refused move leaves the item where it was
    /// rather than on the cursor.
    MoveItem {
        src_bag: u8,
        src_slot: u8,
        /// `None` destroys the item: `CMSG_DESTROYITEM`, which is what
        /// `DeleteCursorItem` sends.
        dst: Option<(u8, u8)>,
        /// `None` moves the whole stack. `Some(n)` is `CMSG_SPLIT_ITEM` with
        /// that count; the server ignores it when the count is zero.
        count: Option<u8>,
    },
    /// The loot window's verbs: open it, take a row, take the coins, close it,
    /// and roll on a group-loot row. See [`crate::play::loot`].
    ///
    /// One command with a small enum rather than separate variants, because
    /// the verbs share one subject and several carry nothing. Separate variants
    /// would include unit structs and one match arm each in the loop, for no
    /// distinction the loop makes.
    Loot(LootVerb),
    /// Use a game object: `CMSG_GAMEOBJ_USE` on a door, a chest, an ore vein,
    /// a mailbox or a lever. See [`crate::play::object`].
    ///
    /// One guid and no verb enum, because a client can do only one thing to a
    /// game object. What happens next (a state change, a loot window, a
    /// gathering cast, a quest page) is the server's decision from the
    /// template, and it arrives through the matching path rather than as a
    /// reply to this packet.
    UseObject(u64),
    /// The quest conversation, grouped for the same reason as
    /// [`Command::Loot`]; see [`QuestVerb`] for the nine verbs.
    Quest(QuestVerb),
    /// The gossip, vendor, trainer, stable and bank verbs; see [`NpcVerb`].
    Npc(NpcVerb),
    /// The pet's nine verbs; see [`PetVerb`]. Not `Copy`, because a rename
    /// carries a name and a set-action carries a list.
    Pet(PetVerb),
    /// The party verbs; see [`PartyVerb`]. Not `Copy`, because some of its
    /// verbs carry a name.
    Party(PartyVerb),
    /// The flight master's verbs; see [`TaxiVerb`]. Not `Copy`, because a
    /// multi-hop flight is a list of nodes.
    Taxi(TaxiVerb),
    /// The reputation panel's three verbs; see [`ReputationVerb`].
    Reputation(ReputationVerb),
    /// The social panel's six verbs; see [`SocialVerb`]. Not `Copy`: three
    /// carry a name and one carries a whole search.
    Social(SocialVerb),
    /// The guild's twenty-one verbs; see [`GuildVerb`].
    Guild(GuildVerb),
    /// The guild charter's nine verbs; see [`PetitionVerb`].
    Petition(PetitionVerb),
    /// The sixteen chat channel verbs; see [`ChannelVerb`].
    Channel(ChannelVerb),
    /// The mailbox verbs; see [`MailVerb`]. Not `Copy`: sending a letter
    /// carries three strings, and it is boxed inside the verb for the same
    /// reason as [`SocialVerb::Who`].
    Mail(MailVerb),
    /// The trade window's ten verbs; see [`TradeVerb`].
    Trade(TradeVerb),
    /// Equip the item at this position, wherever it fits:
    /// `CMSG_AUTOEQUIP_ITEM`, which names no destination because
    /// `CanEquipItem(NULL_SLOT, …)` picks the free finger.
    ///
    /// Separate from [`Command::MoveItem`] rather than a `dst` of "the paper
    /// doll", because it is the only move whose destination the server
    /// chooses from several.
    EquipItem { bag: u8, slot: u8 },
    /// Move an item to the other side of the bank, wherever it fits:
    /// `CMSG_AUTOBANK_ITEM` into the bank, `CMSG_AUTOSTORE_BANK_ITEM` out of
    /// it, with the same body. Sent by a right-click on a square while the bank
    /// window is open; see [`crate::play::bank`]. The direction is
    /// [`crate::play::items::is_bank_position`] of the source, which is the
    /// same test the server uses.
    BankItem { bag: u8, slot: u8 },
    /// Put an action on an action button, or clear it:
    /// `CMSG_SET_ACTION_BUTTON`.
    ///
    /// This runs in the opposite direction from the other commands: the bar is
    /// client state and this only asks the server to store it. The local change
    /// has already happened when this is sent, and nothing waits for an answer
    /// (there is none; see [`crate::play::spells::set_action_button_body`]).
    ///
    /// The slot is zero-based and `packed` is already `(kind << 24) | action`,
    /// where zero means "clear this button". It is built by
    /// [`crate::play::spells::ActionButton::packed`] so that a caller cannot
    /// swap the two halves.
    SetActionButton { slot: u8, packed: u32 },
    /// Store which of the four extra action bars are shown:
    /// `CMSG_SET_ACTIONBAR_TOGGLES`, whose whole body is this byte.
    ///
    /// Like [`Command::SetActionButton`], it reports client state to the
    /// server. Unlike it, the server echoes the value: the byte is stored in
    /// `PLAYER_FIELD_BYTES` and returns in the next values block, which is how
    /// the bars persist across a logout. Nothing waits for it; the interface
    /// has already moved its frames before this is sent.
    SetActionBarToggles(u8),
    /// Stop the ranged auto-repeat. See [`WorldSession::cancel_auto_repeat`]
    /// for why there is no matching start: auto-repeat begins with an ordinary
    /// [`Command::Cast`].
    CancelAutoRepeat,
    /// Tell the server the character has drawn or sheathed its weapons.
    ///
    /// The client makes the decision (see [`WorldSession::set_sheathed`] and
    /// `vale_assets::look::sheath`), and this reports it so that other players
    /// see the weapon in the character's hand. The local character's
    /// appearance has already changed when this is sent, so nothing waits for
    /// the echo.
    SetSheathed(u8),
    /// Say something, or run a GM command; both use the same packet.
    ///
    /// The command carries no language. The caller states the kind of message
    /// and its recipient. The language is a protocol rule with two failure
    /// modes that produce no error (see [`crate::play::chat`]), so the loop
    /// fills it in from the character's race rather than relying on the UI.
    Chat {
        kind: ChatType,
        /// The whisper's recipient or the channel's name.
        target: Option<String>,
        text: String,
    },
    /// Play a text emote such as `/dance` or `/wave`.
    ///
    /// This is the only way this client can cause an `SMSG_EMOTE`, which makes
    /// the emote animation path testable. Otherwise the packet arrives only
    /// when another player emotes, and a client that never animates emotes
    /// cannot be told apart from one that has seen no emote.
    /// The fields are the `EmotesText` row, the `Emotes` id beside it and the
    /// target's guid; see [`crate::play::emotetext`].
    TextEmote { text_emote: u32, emote_num: u32, target: u64 },
    /// Ask to leave the world, or cancel that request: `GameMenuFrame`'s
    /// Logout button and the CAMP popup's Cancel.
    ///
    /// Not [`Command::Shutdown`], which closes the socket. This is a request
    /// the server answers, and the session keeps running (seated and rooted)
    /// until it does. See [`crate::play::logout`].
    Logout {
        /// `false` is `CMSG_LOGOUT_CANCEL`.
        leaving: bool,
    },
    /// Release the spirit: `CMSG_REPOP_REQUEST`, sent by the game's `RepopMe`.
    ///
    /// No body, and no reply packet: the result is `PLAYER_FLAGS_GHOST` set in
    /// an ordinary values block, plus one `SMSG_CORPSE_RECLAIM_DELAY`. See
    /// [`crate::play::death`].
    Repop,
    /// Reset every instance this character is saved to:
    /// `CMSG_RESET_INSTANCES`, with no body and no success reply.
    /// `HandleResetInstancesOpcode` reads nothing, acts on `_player` (or on the
    /// group when this character leads one) and answers only a failure, with
    /// `SMSG_INSTANCE_RESET_FAILED`. See
    /// [`crate::socket::world::WorldSession::reset_instances`].
    ResetInstances,
    /// Ask where the corpse is: `MSG_CORPSE_QUERY`, with no body. The reply
    /// arrives under the same opcode.
    CorpseQuery,
    /// Resurrect at the corpse: `CMSG_RECLAIM_CORPSE`, sent by the game's
    /// `RetrieveCorpse`. The server refuses it without a reply if the reclaim
    /// delay has not elapsed or the ghost is further than
    /// [`crate::play::death::CORPSE_RECLAIM_RADIUS`].
    ReclaimCorpse,
    /// Accept or decline a resurrection another player offered:
    /// `CMSG_RESURRECT_RESPONSE`. The guid is the caster's, echoed from the
    /// offer.
    ResurrectResponse { caster: u64, accept: bool },
    /// Answer a duel: `CMSG_DUEL_ACCEPTED` or `CMSG_DUEL_CANCELLED`, both
    /// naming the duel flag object. Cancelling also forfeits a duel in progress
    /// (`/forfeit`); the server distinguishes the two by whether the duel has
    /// started.
    DuelAnswer { arbiter: u64, accept: bool },
    /// Accept a summon: `CMSG_SUMMON_RESPONSE`. The wire has no decline; see
    /// [`crate::play::summon`].
    SummonResponse { summoner: u64 },
    /// `/played`: `CMSG_PLAYED_TIME`, with no body.
    RequestPlayedTime,
    /// `/roll`: `MSG_RANDOM_ROLL` with the range. See [`crate::play::randomroll`].
    RandomRoll { min: u32, max: u32 },
    /// A click on the minimap: `MSG_MINIMAP_PING` with the world position. See [`crate::play::minimap`].
    MinimapPing { x: f32, y: f32 },
    /// Place raid target icon `icon` (0..8) on `guid`, or clear it with guid 0: `MSG_RAID_TARGET_UPDATE`. See [`crate::play::raidtarget`].
    RaidTargetSet { icon: u8, guid: u64 },
    /// Ask for the group's raid target icons: `MSG_RAID_TARGET_UPDATE` with `0xFF`.
    RaidTargetList,
    /// Share a quest with the group: `CMSG_PUSHQUESTTOPARTY`. See [`crate::play::questshare`].
    PushQuest { quest_id: u32 },
    /// Take the party quest another member accepted: `CMSG_QUEST_CONFIRM_ACCEPT`.
    QuestConfirmAccept { quest_id: u32 },
    /// Answer a shared quest's page: `MSG_QUEST_PUSH_RESULT` to the sharer, which the client sends with `Declined` when the page is closed.
    QuestPushResult { sharer: u64, result: crate::play::questshare::PushResult },
    /// Mark tutorial `id` (1-based) as seen: `CMSG_TUTORIAL_FLAG`. See [`crate::play::tutorial`].
    TutorialFlag { id: u32 },
    /// Mark every tutorial as seen, turning the tips off: `CMSG_TUTORIAL_CLEAR`, with no body.
    TutorialClear,
    /// Mark no tutorial as seen, turning the tips on: `CMSG_TUTORIAL_RESET`, with no body.
    TutorialReset,
    /// Flip `PLAYER_FLAGS_HIDE_HELM` (`helm`) or `PLAYER_FLAGS_HIDE_CLOAK`:
    /// `CMSG_TOGGLE_HELM` or `CMSG_TOGGLE_CLOAK`, with no body. The server
    /// toggles the bit (vmangos' `HandleShowingHelmOpcode`), so the caller
    /// sends one only when the current flag differs from the one wanted.
    ToggleWorn { helm: bool },
    /// Inspect a player: `CMSG_INSPECT`. See [`crate::play::inspect`].
    Inspect { guid: u64 },
    /// Ask for an inspected player's honor: `MSG_INSPECT_HONOR_STATS`.
    InspectHonor { guid: u64 },
    /// Accept the spirit healer's offer: `CMSG_SPIRIT_HEALER_ACTIVATE`, at the
    /// cost of 25% durability on all items and ten minutes of resurrection
    /// sickness.
    SpiritHealerActivate { healer: u64 },
    /// Put an item entry in the ammo slot, or 0 to empty it: `CMSG_SET_AMMO`.
    /// This is not an item move; see [`crate::play::items::set_ammo_body`].
    SetAmmo { entry: u32 },
    /// Spend a talent point: `CMSG_LEARN_TALENT`, by talent id and the
    /// zero-based rank requested.
    ///
    /// `vale_assets::tables::talent::TalentTree::learn` decides the rank and
    /// the one refusal the client makes itself, as a rule over `Talent.dbc`
    /// with no socket. Nothing waits for an answer, because there is none; see
    /// [`crate::play::talents`].
    LearnTalent { talent_id: u32, rank: u32 },
    /// Start or stop the recent-packet capture; see
    /// [`crate::socket::world::Capture`].
    ///
    /// A command rather than a shared flag because the ring buffer belongs to
    /// the session's socket, and because starting a capture clears it: two
    /// threads doing that at once would interleave two captures in one buffer.
    CaptureTraffic(bool),
    /// Leave the world and close the socket.
    Shutdown,
}

/// Everything about the session that is not per-entity state.
#[derive(Debug, Clone, Default)]
pub struct SessionStatus {
    pub character: String,
    pub map_id: u32,
    /// The client's simulated position, updated every tick. The player
    /// entity's position, by contrast, changes only when the server sends one.
    pub position: Position,
    pub controls: Controls,
    pub speeds: Speeds,
    /// The movement block the local simulation is running on, unchanged.
    ///
    /// [`SessionStatus::position`], `moving`, `speed` and `direction` are all
    /// derived from it. It is published whole because a renderer that
    /// continues the simulation between steps, rather than only displaying it,
    /// needs the same inputs as [`Mover::advance`]. Both integrate
    /// `vale_protocol::state::movement::strode`.
    pub movement: MovementInfo,
    /// Whether the local simulation has the character moving, and its speed in
    /// yards per second. The anticheat judges these two values, and a renderer
    /// needs them to pick Stand, Walk or Run.
    ///
    /// The player is the only entity [`ObjectManager::advance`] does not
    /// update: [`Mover`] owns its position, so
    /// [`crate::state::objects::Entity::is_moving`] has no data for it and the
    /// state comes from here.
    pub moving: bool,
    pub speed: f32,
    /// The velocity at which the server is moving this character, in yards per
    /// second in world axes: a Charge, a taxi flight, or anything else that
    /// reaches [`Mover::ride`].
    ///
    /// A reader that continues the simulation between steps needs this as it
    /// needs [`Self::restraint`]: the keys have no effect during a ride, so a
    /// prediction that moved from them would oppose the ride sixty times a
    /// second. The velocity is published because it cannot be recovered from
    /// the movement block: the speed is not one of the six in [`Speeds`], and
    /// the orientation is a horizontal bearing. A flight predicted from those
    /// two holds its altitude between readings and steps down to each new one,
    /// producing a 40 Hz sawtooth on the vertical axis. See
    /// [`crate::state::movement::Spline::velocity`].
    pub riding: Option<[f32; 3]>,
    /// The moving platform this character is standing on, or `None` for most
    /// of a session. See [`crate::state::movement::Ferry`].
    ///
    /// Published for the same reason as [`Self::riding`], and without it the
    /// error is larger. A reader that continues the simulation between steps
    /// draws the local player at `base + velocity * age`, and a passenger's
    /// velocity is not in the movement block: the deck's travel arrives as a
    /// correction to the position on each tick. The drawn character then
    /// stands still for a whole step and jumps by the platform's movement. On
    /// the Deeprun Tram that is 0.43 yards forty times a second, and because
    /// the camera follows the same position, the whole world shakes.
    ///
    /// A reader needs the pair (the offset, and the platform placement it was
    /// measured against) so it can place the passenger on the platform position
    /// it is drawing in this frame. See `vale-client`'s `world::predict`.
    pub ferry: Option<crate::state::movement::Ferry>,
    /// Off the ground, and whether that began with a jump rather than a step
    /// off an edge. The renderer needs both: `JumpStart`/`Jump`/`JumpEnd` and
    /// `Fall` are four different sequences, and only the mover knows which arc
    /// this is, because the player is the only entity whose movement flags the
    /// server never sends back.
    pub airborne: bool,
    pub jumping: bool,
    /// The guid of the body every other field here describes: the mover, which
    /// is the character's own guid except during a possess.
    ///
    /// [`Self::position`], [`Self::controls`], [`Self::movement`],
    /// [`Self::moving`] and the rest describe the mover, not the character, so
    /// a reader that continues the simulation has to know which entity to
    /// continue. `crate::world::session::place_entities` draws the mover ahead
    /// of the simulation and leaves the character to ordinary interpolation.
    /// That makes a possessed body drivable while the possessing character
    /// stands still.
    ///
    /// Zero means no mover: the character is feared or confused and the server
    /// moves the body. See [`crate::socket::handler::LocalState::mover_guid`].
    pub mover: u64,
    /// What the server has stopped this character doing: stun and death,
    /// which are not in [`SessionStatus::movement`] because neither is a
    /// movement flag. A renderer that continues the simulation needs them for
    /// the same reason it needs the movement block: without them its
    /// prediction turns a character the mover is holding still, and the drawn
    /// body spins in place while the confirmed one does not move. See
    /// `movement::Restraint`.
    pub restraint: crate::state::movement::Restraint,
    /// The world's date and hour, advanced from the value the server sent at
    /// login. `None` until `SMSG_LOGIN_SETTIMESPEED` arrives in the login
    /// burst; see [`crate::play::time`].
    ///
    /// The light chain reads it, because lighting is indexed by time of day:
    /// the sun, sky and fog all differ at dawn.
    pub game_time: Option<crate::play::time::GameTime>,
    /// The current weather: the last `SMSG_WEATHER`, copied from the object
    /// manager with the clock. See [`crate::play::weather`].
    pub weather: Option<crate::play::weather::Weather>,
    /// Simulated milliseconds since the thread started. Everything in
    /// [`SessionStatus::position`] and in the [`ObjectManager`] was advanced by
    /// this clock, so it states how much world time the positions cover.
    ///
    /// A reader needs this to interpolate. The renderer draws at 60–130 Hz and
    /// the simulation steps at about 40 Hz, and the two clocks are
    /// independent. A reader polling on its own timer sometimes sees no new
    /// tick and sometimes two, so an interpolator keyed on poll intervals
    /// alternately stalls and speeds up; on screen the whole world jitters.
    /// This value is monotonic and advances by exactly the time the positions
    /// moved through, so a reader can key on it instead: unchanged means
    /// nothing new to draw, and the difference is how long the step covered.
    pub world_ms: u64,
    /// The real instant at which the simulation reached [`Self::world_ms`].
    ///
    /// `world_ms` gives how much world time a reading covers; this gives when
    /// the reading was taken, and a reader that continues the simulation needs
    /// both. Without it the only timestamp is the moment the reader noticed the
    /// reading, which is up to a frame later and varies each time. A prediction
    /// anchored on that advances by the interval between notices while the base
    /// advances by the interval between steps, and the character alternately
    /// speeds up and slows down several times a second. The reader this exists
    /// for is `vale-client`'s `world::predict`.
    ///
    /// `None` before the first step. Both clocks are wall-clock, so the age of
    /// a reading is `taken.elapsed()` and nothing needs synchronising.
    pub taken: Option<Instant>,
    /// Ticks whose socket-draining slice ran out, alongside the total number
    /// of ticks.
    ///
    /// This is the second way world state can arrive late, and its delay has
    /// no bound, unlike a stall's quarter of a second.
    /// [`SessionLoop::read_socket`] drains for at most [`READ_SLICE`] of every
    /// [`TICK`], so a stream arriving faster than that leaves a backlog in the
    /// kernel and this client shows a world that is seconds old, with every
    /// number on screen consistent but late. A localhost server does not
    /// produce this and a busy realm might, so it is counted rather than left
    /// to be noticed. Read it as a proportion of [`Self::ticks`]; see
    /// [`SessionLoop::read_socket`] for the one case it over-counts.
    pub read_slice_overruns: u32,
    pub ticks: u32,
    /// Steps that took longer than [`MAX_STEP`], and the worst of them in
    /// milliseconds.
    ///
    /// The session thread is meant to run every 25 ms and sometimes cannot:
    /// during a login burst, an archive read, or the renderer holding the world
    /// lock while it uploads a tile. During each stall the server moves
    /// everything in view and this client does not. Without this counter the
    /// only symptom is creatures drawn where they no longer are, which looks
    /// like a network fault but is not. Zero is the normal state; a session
    /// that accumulates stalls is starving its world thread, and the next thing
    /// to check is what the thread was doing.
    pub stalls: u32,
    pub worst_stall_ms: u32,
    /// Steps in which the deck under the character moved further than a
    /// transport can travel, and the worst of them in yards.
    ///
    /// A passenger is placed by composing their offset with the client's
    /// platform position, so a platform placement wrong by a mile puts the
    /// character a mile away. Nothing reports it, because every count on the
    /// panel is still correct and the position is valid. Two causes have been
    /// seen: the deck's interpolation history survived a map change, and
    /// `world::motion` returned a point between two continents for a frame;
    /// in both the carry moved the character there.
    ///
    /// A count of zero is not the only correct state. Transports do jump: two
    /// of the nine routes teleport 13,700 yards without changing map and no
    /// packet announces it, so riding one adds exactly one jump per cycle. This
    /// counter distinguishes a jump from an unexplained jump: check whether it
    /// happened when the route says it should.
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
    /// `SMSG_FORCE_*_SPEED_CHANGE` packets recorded and acknowledged.
    ///
    /// This loop previously intercepted these, acknowledged them, and dropped
    /// them, so a speed change for another unit never reached the world state
    /// and that player was still dead-reckoned at the old speed. Seeing these
    /// is not an error; the count shows whether the path has run, which a unit
    /// test can check and which a live session had no way to show before.
    pub speed_changes: u32,
    /// The twelve speed opcodes for units this client does not control:
    /// `MSG_MOVE_SET_*_SPEED` and `SMSG_SPLINE_SET_*`.
    ///
    /// These matter most in a live session, because the forced family is sent
    /// only for one character and this family is sent for every mount, sprint,
    /// daze and enrage in view. Zero in a session where anyone mounted means
    /// the family is being dropped. Before this counter existed, all twelve
    /// went to the unhandled bucket, and other players' characters slid back
    /// and forth twice a second.
    pub speed_broadcasts: u32,
    /// The twelve `SMSG_SPLINE_MOVE_*` opcodes, for units no player is moving.
    ///
    /// These are the creature counterpart of the speed opcodes: root, water
    /// walking, feather fall, hover, walk/run mode. They are about ten times
    /// rarer than the speed opcodes (a fight with a root, a patrol switched to
    /// walking), so a zero here is evidence only when one of those is known to
    /// have happened.
    pub spline_flag_changes: u32,
    /// Sounds and visuals the server requested directly: the three
    /// `SMSG_PLAY_*` sound opcodes, and the two that put a `SpellVisualKit` on
    /// a unit. Nothing else on the wire implies either family, so a zero in a
    /// session where anything scripted happened, or where anyone sat down to
    /// eat, means they are being dropped.
    pub pushed_sounds: u32,
    pub pushed_visuals: u32,
    /// `SMSG_COMPRESSED_MOVES` bags, and the packets unpacked from them. The
    /// reason for the counter is given at
    /// [`crate::socket::handler::PumpStats::bagged_moves`]. Zero carries no
    /// information; a large number means most of the world's movement arrives
    /// inside this one opcode, which is when a client that ignores it desyncs.
    pub bagged_moves: u32,
    pub bagged_packets: u32,
    /// `CMSG_AREATRIGGER` packets sent this session. The reason for the counter
    /// is given at [`crate::socket::handler::PumpStats::area_triggers`].
    pub area_triggers: u32,
    /// `CMSG_MOVE_SPLINE_DONE` packets sent this session. The reason for the
    /// counter is given at [`crate::socket::handler::PumpStats::rides`].
    pub rides: u32,
    /// `SMSG_FORCE_MOVE_ROOT` and its five related opcodes, plus knockbacks,
    /// acknowledged. Zero for a whole session is normal. A kick with
    /// `PendingAckDelay` while this is zero is not, and this number makes that
    /// case visible.
    pub flag_changes: u32,
    /// The three packets that drive an animation and leave no other trace.
    ///
    /// Counted for the same reason as `speed_changes`. A swing, an emote and a
    /// cast are events: each is folded into a counter on an entity and then
    /// discarded, so the world state cannot distinguish an emote that arrived
    /// and was dropped from one that never arrived. Animating the wrong thing
    /// produces no error, so these counts show whether each path has run.
    pub attacks: u32,
    pub emotes: u32,
    pub casts: u32,
    /// `SMSG_AI_REACTION`, which drives only a sound and so leaves no trace in
    /// the world state. Zero in a session where a creature attacked the
    /// character means the aggro sounds are being dropped; nothing on screen
    /// would show it.
    pub ai_reactions: u32,
    /// The unit the server reports this character is attacking. This is the
    /// server's only statement about the character's auto-attack:
    /// `SMSG_ATTACKSTART` sets it and `SMSG_ATTACKSTOP` clears it. An Attack
    /// button that is pressed but never lights up is a swing the server
    /// refused, and the reason (one of five) arrives through
    /// [`LiveSession::take_events`].
    pub attacking: Option<u64>,
    /// Incremented whenever the spellbook or the action bar changes, so a
    /// reader can rebuild the bar when it changes rather than every frame.
    pub spellbook_version: u32,
    /// The two counters that show whether the login burst succeeded. Zero
    /// spells known is a failure with no other symptom: the packet is sent once
    /// and not repeated, so the bar stays empty for the whole session.
    pub spells_known: u32,
    pub action_buttons: u32,
    /// `SMSG_CAST_RESULT` packets seen, and the five swing refusals. Both are
    /// events that leave no other trace, so they are counted for the same
    /// reason as `attacks` and `emotes` above.
    pub cast_results: u32,
    pub attack_refusals: u32,
    /// Round-trip time of the last `CMSG_PING`.
    pub latency_ms: u32,
    /// Parse warnings, capped; see [`PumpStats`].
    pub warnings: Vec<String>,
    /// Opcodes received but not handled, with counts.
    pub unhandled: BTreeMap<String, u32>,
    /// Socket traffic: bytes and packets in each direction, and each opcode's
    /// share of both. See [`Traffic`].
    ///
    /// Behind an `Arc` and rebuilt only every [`TRAFFIC_INTERVAL`], because the
    /// renderer clones this whole struct once a frame. The two maps inside hold
    /// about a hundred `String` keys between them, and copying them a hundred
    /// times a second for a closed window is the cost
    /// [`LiveSession::world_ms`] exists to avoid. With the `Arc`, cloning the
    /// status costs one atomic increment for this field.
    pub traffic: Arc<Traffic>,
    /// The last few hundred packets, complete, while the capture is running;
    /// see [`crate::socket::world::Capture`].
    ///
    /// Behind an `Arc` for the same reason as [`Self::traffic`], and rebuilt on
    /// the same interval. Stopping the capture leaves the last snapshot in
    /// place, so the packets before a fault can still be read afterwards.
    pub capture: Arc<crate::socket::world::CaptureSnapshot>,
}

/// A handle to the running session. Dropping it shuts the thread down.
pub struct LiveSession {
    commands: Sender<Command>,
    world: Arc<Mutex<ObjectManager>>,
    status: Arc<Mutex<SessionStatus>>,
    thread: Option<JoinHandle<()>>,
    /// Where the loop leaves its socket when it stops cleanly, so a logout can
    /// return to character select on the same connection.
    ///
    /// The thread owns the `WorldSession` for the life of the session, because
    /// one ordered stream with a stateful header cipher cannot be shared. The
    /// socket can change owner only after `run` returns. It is stored there,
    /// and only for a requested stop: a socket that failed a write is not
    /// reused for the character list. See [`LiveSession::reclaim`].
    handback: Arc<Mutex<Option<WorldSession>>>,
}

impl LiveSession {
    /// Take over an authenticated session: enter the world as `character` and
    /// keep running until told to stop.
    ///
    /// Returns immediately. Use [`LiveSession::wait_until_in_world`] when the
    /// caller needs to report login failures synchronously.
    /// `caches` holds the on-disk query answers; see [`crate::play::wdb`].
    /// Pass [`Caches::none`] when the caller has nowhere to store them. The
    /// session behaves the same either way, except that a window naming
    /// something not yet queried is blank the first time it opens.
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
        // once, and there is no event to raise when a template arrives later.
        // The bodies go through the socket's parsers; see `wdb`'s module doc.
        lock(&world).seed_cache(&caches);
        // The server does not send the player's own name again after the
        // character list, and `unresolved_player_guids` skips the local GUID
        // on purpose. An unseeded world therefore names its own player
        // "Player 450" for the whole session, and the game's `PlayerFrame`
        // displays that. The 1.12.1 client also takes the name from the
        // character list.
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
                        // A clean stop leaves the socket for reuse. `run`
                        // returns `Ok` only when the command loop was told to
                        // stop (a shutdown or a dropped handle), so the stream
                        // is intact and at a packet boundary. Any other result
                        // is an IO or protocol failure and the connection is
                        // dropped with the thread, so this is a `match` and not
                        // an unconditional move.
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

    /// The world state. Lock it briefly; the session thread needs it too.
    pub fn world(&self) -> &Arc<Mutex<ObjectManager>> {
        &self.world
    }

    pub fn status(&self) -> SessionStatus {
        lock(&self.status).clone()
    }

    /// [`SessionStatus::world_ms`] alone, without cloning the rest.
    ///
    /// A renderer calls this every frame to find out whether there is anything
    /// new, so it has to be cheaper than the full status. The full status
    /// carries a `Vec` of warnings and a map of unhandled opcodes, and cloning
    /// those a hundred times a second would be wasted work.
    pub fn world_ms(&self) -> u64 {
        lock(&self.status).world_ms
    }

    /// [`SessionStatus::game_time`] alone, for the same reason as
    /// [`Self::world_ms`]: the renderer reads the hour every frame to light the
    /// world, and cloning the warnings and the unhandled-opcode map for it
    /// would be the same wasted work.
    pub fn game_time(&self) -> Option<crate::play::time::GameTime> {
        lock(&self.status).game_time
    }

    /// [`SessionStatus::spellbook_version`] and [`SessionStatus::attacking`],
    /// for the same reason as the two above.
    ///
    /// Both are read every frame (the first to decide whether the action bar
    /// needs rebuilding, the second to light the Attack button), and each is
    /// one word. Reading them through [`Self::status`] would clone a `Vec` of
    /// warnings and a `BTreeMap` of unhandled opcodes a hundred times a second,
    /// which is the cost `world_ms` was separated out to avoid.
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

    /// The movement keys held from now on. The command is stamped with
    /// this moment; see [`Command::Controls`].
    pub fn set_controls(&self, controls: Controls) {
        self.send(Command::Controls(controls, Instant::now()));
    }

    pub fn face(&self, orientation: f32) {
        self.send(Command::Face(orientation));
    }

    /// The camera's pitch, which is also the body's pitch while swimming; see
    /// [`Command::Pitch`].
    pub fn pitch(&self, pitch: f32) {
        self.send(Command::Pitch(pitch));
    }

    pub fn jump(&self) {
        self.send(Command::Jump(Instant::now()));
    }

    /// Attack a unit, or `None` to stop attacking.
    pub fn attack(&self, guid: Option<u64>) {
        self.send(Command::Attack(guid));
    }

    /// Tell the server which unit is selected, or `None` for nothing.
    pub fn target(&self, guid: Option<u64>) {
        self.send(Command::Target(guid));
    }

    /// Request a cast at an already-resolved target; see [`Command::Cast`].
    ///
    /// This only sends the request. Nothing is drawn, no cast bar starts and
    /// no counter changes here: a cast begins when `SMSG_SPELL_START` arrives.
    /// Build 5875 behaves the same way: a press runs the client's local
    /// refusals, records a pending cast, starts the global cooldown (which is
    /// client-side, as a vmangos comment notes) and sends the request. That
    /// keeps a refused cast from playing a wind-up and release that never
    /// happened.
    ///
    /// This one method covers three kinds of press: an ordinary cast, a
    /// next-swing ability the server holds until the weapon lands, and a ranged
    /// auto-repeat whose first shot fires on the ranged attack timer. They
    /// differ in when the server answers, not in what this function does.
    pub fn cast(&self, spell_id: u32, target: crate::play::spells::CastTarget) {
        self.send(Command::Cast { spell_id, target });
    }

    /// Stop casting and stop drawing the cast. [`Self::cast`] starts nothing
    /// locally, but cancelling does change local state.
    ///
    /// The server sends no reply to `CMSG_CANCEL_CAST`, so the local change is
    /// the only one. Without it, pressing Escape during a Fireball cleared the
    /// cast bar and left the character in the wind-up pose until its timer ran
    /// out. See [`ObjectManager::apply_cast_cancelled`].
    pub fn cancel_cast(&self, spell_id: u32) {
        {
            let mut world = lock(&self.world);
            if let Some(guid) = world.player_guid {
                world.apply_cast_cancelled(guid, spell_id);
            }
        }
        self.send(Command::CancelCast(spell_id));
    }

    /// Stop the ranged auto-repeat; see [`WorldSession::cancel_auto_repeat`].
    ///
    /// No local state changes, on purpose: the server answers with
    /// `SMSG_CANCEL_AUTO_REPEAT` and the caller's state is updated from that,
    /// so a press and every other way auto-repeat can end use one code path.
    /// Predicting it here would let the two disagree in the case a prediction
    /// cannot cover: the server stopping auto-repeat itself.
    pub fn cancel_auto_repeat(&self) {
        self.send(Command::CancelAutoRepeat);
    }

    /// Right-click a buff to remove it; see [`Command::CancelAura`]. No local
    /// state changes; the icon disappears when the server empties the slot.
    pub fn cancel_aura(&self, spell_id: u32) {
        self.send(Command::CancelAura(spell_id));
    }

    /// Right-click an item; see [`Command::UseItem`]. No local state changes:
    /// the stack shrinks, the equipment moves and the cooldown starts when the
    /// server's update block reports it, one round trip later, as in the
    /// 1.12.1 client.
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

    /// Right-click an item that holds loot; see [`Command::OpenItem`]. No local
    /// state changes; the window arrives as an ordinary `SMSG_LOOT_RESPONSE`
    /// whose guid is the item's.
    pub fn open_item(&self, bag: u8, slot: u8) {
        self.send(Command::OpenItem { bag, slot });
    }

    /// Open the loot window on a body; see [`crate::play::loot`]. Nothing is
    /// shown until `SMSG_LOOT_RESPONSE` arrives, which is also how the server
    /// refuses.
    pub fn loot(&self, guid: u64) {
        self.send(Command::Loot(LootVerb::Open(guid)));
    }

    /// Take one loot row, by the server's index. See
    /// [`crate::play::loot::Loot::at`], the one place that converts between
    /// that index and the interface's row.
    pub fn loot_item(&self, index: u8) {
        self.send(Command::Loot(LootVerb::Take(index)));
    }

    /// Take the loot's coins.
    pub fn loot_money(&self) {
        self.send(Command::Loot(LootVerb::TakeMoney));
    }

    /// Release the loot. The window does not close here; the reason is given
    /// in [`crate::play::loot`].
    pub fn loot_release(&self, guid: u64) {
        self.send(Command::Loot(LootVerb::Release(guid)));
    }

    /// Vote in a group roll. The roll is identified by the body and the slot,
    /// not by a window; the `rollID` the interface passes is a counter,
    /// explained in [`crate::play::lootroll`].
    pub fn loot_roll(&self, guid: u64, item_slot: u32, vote: crate::play::lootroll::RollVote) {
        self.send(Command::Loot(LootVerb::Roll {
            guid,
            item_slot,
            vote,
        }));
    }

    /// Send one of the quest conversation's nine verbs; see [`QuestVerb`].
    pub fn quest(&self, verb: QuestVerb) {
        self.send(Command::Quest(verb));
    }

    /// Right-click a door, a chest, an ore vein or a mailbox; see
    /// [`Command::UseObject`], and `vale_assets::look::object` for the rule
    /// that decides whether a game object is clickable.
    pub fn use_object(&self, guid: u64) {
        self.send(Command::UseObject(guid));
    }

    /// Where the hearthstone returns the character to, read from the world.
    ///
    /// Read through the world lock rather than the status, because the handler
    /// writes `SMSG_BINDPOINTUPDATE` to
    /// [`crate::state::objects::ObjectManager`] and no status field mirrors it.
    /// The renderer asks once per session rather than every frame; see
    /// `game::npc::binder`, which explains why it is polled instead of
    /// delivered as an event.
    pub fn bind_point(&self) -> Option<crate::play::bindpoint::BindPoint> {
        lock(&self.world).bind_point
    }

    /// Send one of the gossip, vendor, trainer, stable or bank verbs; see
    /// [`NpcVerb`].
    pub fn npc(&self, verb: NpcVerb) {
        self.send(Command::Npc(verb));
    }

    /// Send one of the pet's nine verbs; see [`PetVerb`].
    pub fn pet(&self, verb: PetVerb) {
        self.send(Command::Pet(verb));
    }

    /// Start or stop the recent-packet capture; see
    /// [`crate::socket::world::Capture`]. Nothing is sent on the wire; the ring
    /// buffer is on this side of the socket.
    pub fn capture_traffic(&self, on: bool) {
        self.send(Command::CaptureTraffic(on));
    }

    /// Send one of the party verbs; see [`PartyVerb`].
    pub fn party(&self, verb: PartyVerb) {
        self.send(Command::Party(verb));
    }

    /// Send one of the flight master's verbs; see [`TaxiVerb`].
    pub fn taxi(&self, verb: TaxiVerb) {
        self.send(Command::Taxi(verb));
    }

    /// Send one of the reputation panel's three verbs; see [`ReputationVerb`].
    pub fn reputation(&self, verb: ReputationVerb) {
        self.send(Command::Reputation(verb));
    }

    /// Query, add to or remove from the friend and ignore lists, or run a Who
    /// search; see [`SocialVerb`].
    pub fn social(&self, verb: SocialVerb) {
        self.send(Command::Social(verb));
    }

    /// Send one of the guild's requests; see [`GuildVerb`].
    pub fn guild(&self, verb: GuildVerb) {
        self.send(Command::Guild(verb));
    }

    /// Send one of the guild charter's requests; see [`PetitionVerb`].
    pub fn petition(&self, verb: PetitionVerb) {
        self.send(Command::Petition(verb));
    }

    /// Join, leave, list or moderate a chat channel; see [`ChannelVerb`].
    pub fn channel(&self, verb: ChannelVerb) {
        self.send(Command::Channel(verb));
    }

    /// Send one of the trade window's ten verbs; see [`TradeVerb`].
    pub fn trade(&self, verb: TradeVerb) {
        self.send(Command::Trade(verb));
    }

    /// Send one of the mailbox verbs; see [`MailVerb`].
    pub fn mail(&self, verb: MailVerb) {
        self.send(Command::Mail(verb));
    }

    /// Move an item to another position; see [`Command::MoveItem`], which
    /// explains why five opcodes share one command.
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

    /// Equip an item wherever it fits; see [`Command::EquipItem`].
    pub fn equip_item(&self, bag: u8, slot: u8) {
        self.send(Command::EquipItem { bag, slot });
    }

    /// Move an item into or out of the bank; see [`Command::BankItem`].
    pub fn bank_item(&self, bag: u8, slot: u8) {
        self.send(Command::BankItem { bag, slot });
    }

    /// Set or clear one action button, locally first and then on the wire.
    ///
    /// The order is the opposite of the item verbs', and it matches the 1.12.1
    /// client: it updates its action bar and then sends the packet, because
    /// the bar is client state and the server is only informed. The world's
    /// copy is therefore updated here, under the same lock every other reader
    /// takes, rather than on an echo that never comes. See
    /// [`ObjectManager::set_action_button`] for why omitting the local update
    /// goes unnoticed until the next rebuild.
    ///
    /// A `kind` of `None` empties the slot, which is a zero word on the wire.
    pub fn set_action_button(&self, slot: u8, action: u32, kind: Option<u8>) {
        lock(&self.world).set_action_button(slot, action, kind);
        let packed = kind.map_or(0, |kind| crate::play::spells::ActionButton::packed(action, kind));
        self.send(Command::SetActionButton { slot, packed });
    }

    /// Store which extra action bars are shown, and nothing else; see
    /// [`Command::SetActionBarToggles`].
    ///
    /// Unlike [`Self::set_action_button`] above, this makes no local write.
    /// The 1.12.1 client makes none either; it only sends the packet. The field
    /// is `PRIVATE` and the server sends it straight back, so a local write
    /// would be a second copy of a value the server is about to send.
    pub fn set_actionbar_toggles(&self, mask: u8) {
        self.send(Command::SetActionBarToggles(mask));
    }

    /// The remaining time on the character's own buffs, as `(slot, seconds
    /// since the reading, remaining ms at that moment, seq)`.
    ///
    /// Read rather than drained, as [`ObjectManager::aura_durations`]
    /// describes. The elapsed time is measured here, under the lock, because
    /// the reader uses the frame clock and this crate uses `Instant`; one
    /// subtraction on this side means the two clocks never have to be
    /// reconciled.
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

    /// Ask the server to log this character out, or cancel the request.
    ///
    /// No local state changes and nothing is predicted: the character leaves
    /// the world when `SMSG_LOGOUT_COMPLETE` arrives on [`Self::take_events`],
    /// twenty seconds later for a character standing in a field and
    /// immediately for one in an inn. See [`crate::play::logout`].
    pub fn logout(&self, leaving: bool) {
        self.send(Command::Logout { leaving });
    }

    /// Release the spirit. The game's Lua name for this is `RepopMe()`.
    ///
    /// No local state changes and nothing is predicted, for the same reason as
    /// [`Self::logout`]: the server decides, and the result arrives as
    /// `PLAYER_FLAGS_GHOST` in the next values block rather than as a separate
    /// packet. See [`crate::play::death`].
    pub fn repop(&self) {
        self.send(Command::Repop);
    }

    /// Reset every instance this character is saved to: the self menu's
    /// `RESET_INSTANCES` row, after its confirmation popup.
    ///
    /// No local state changes and there is nothing to predict: the server acts
    /// and replies only if it refuses. See [`Command::ResetInstances`].
    pub fn reset_instances(&self) {
        self.send(Command::ResetInstances);
    }

    /// Ask where the corpse is. The reply raises
    /// [`crate::play::spells::PlayerEvent::CorpseLocated`].
    pub fn corpse_query(&self) {
        self.send(Command::CorpseQuery);
    }

    /// Resurrect at the corpse: `RetrieveCorpse()`.
    pub fn reclaim_corpse(&self) {
        self.send(Command::ReclaimCorpse);
    }

    /// Answer a resurrection offer. `caster` is the guid the offer arrived
    /// with. The server compares it and drops a mismatch without a reply, and
    /// logs a zero guid as an "Instant resurrect hack" before reading anything
    /// else (`HandleResurrectResponseOpcode`).
    pub fn resurrect_response(&self, caster: u64, accept: bool) {
        self.send(Command::ResurrectResponse { caster, accept });
    }

    /// Accept or decline a duel; see [`Command::DuelAnswer`].
    pub fn duel_answer(&self, arbiter: u64, accept: bool) {
        self.send(Command::DuelAnswer { arbiter, accept });
    }

    /// Accept a summon; see [`Command::SummonResponse`].
    pub fn summon_response(&self, summoner: u64) {
        self.send(Command::SummonResponse { summoner });
    }

    /// Ask for the played time; see [`Command::RequestPlayedTime`].
    pub fn request_played_time(&self) {
        self.send(Command::RequestPlayedTime);
    }

    /// See [`Command::RandomRoll`].
    pub fn random_roll(&self, min: u32, max: u32) {
        self.send(Command::RandomRoll { min, max });
    }

    /// See [`Command::MinimapPing`].
    pub fn minimap_ping(&self, x: f32, y: f32) {
        self.send(Command::MinimapPing { x, y });
    }

    /// See [`Command::RaidTargetSet`].
    pub fn raid_target_set(&self, icon: u8, guid: u64) {
        self.send(Command::RaidTargetSet { icon, guid });
    }

    /// See [`Command::RaidTargetList`].
    pub fn raid_target_list(&self) {
        self.send(Command::RaidTargetList);
    }

    /// See [`Command::PushQuest`].
    pub fn push_quest(&self, quest_id: u32) {
        self.send(Command::PushQuest { quest_id });
    }

    /// See [`Command::QuestConfirmAccept`].
    pub fn quest_confirm_accept(&self, quest_id: u32) {
        self.send(Command::QuestConfirmAccept { quest_id });
    }

    /// See [`Command::QuestPushResult`].
    pub fn quest_push_result(&self, sharer: u64, result: crate::play::questshare::PushResult) {
        self.send(Command::QuestPushResult { sharer, result });
    }

    /// See [`Command::TutorialFlag`].
    pub fn tutorial_flag(&self, id: u32) {
        self.send(Command::TutorialFlag { id });
    }

    /// See [`Command::TutorialClear`].
    pub fn tutorial_clear(&self) {
        self.send(Command::TutorialClear);
    }

    /// See [`Command::TutorialReset`].
    pub fn tutorial_reset(&self) {
        self.send(Command::TutorialReset);
    }

    /// Flip the hide-helm or hide-cloak flag; see [`Command::ToggleWorn`].
    pub fn toggle_worn(&self, helm: bool) {
        self.send(Command::ToggleWorn { helm });
    }

    /// Inspect a player; see [`Command::Inspect`].
    pub fn inspect(&self, guid: u64) {
        self.send(Command::Inspect { guid });
    }

    /// Ask for an inspected player's honor; see [`Command::InspectHonor`].
    pub fn inspect_honor(&self, guid: u64) {
        self.send(Command::InspectHonor { guid });
    }

    /// Accept the spirit healer's offer: `AcceptXPLoss()`.
    pub fn spirit_healer_activate(&self, healer: u64) {
        self.send(Command::SpiritHealerActivate { healer });
    }

    /// Fill the ammo slot, or empty it with 0; see [`Command::SetAmmo`].
    pub fn set_ammo(&self, entry: u32) {
        self.send(Command::SetAmmo { entry });
    }

    /// Report a sheath change; see [`Command::SetSheathed`]. The value is
    /// clamped here rather than at the send, because vmangos drops an
    /// out-of-range value without a reply and the two ends would then disagree
    /// for the rest of the session.
    pub fn set_sheathed(&self, state: u8) {
        self.send(Command::SetSheathed(
            state.min(crate::socket::world::MAX_SHEATH_STATE - 1),
        ));
    }

    /// Take the server's answers to this character's actions that have arrived
    /// since the last call: a cast refused, a swing out of range, a cooldown
    /// started.
    ///
    /// Drained rather than cloned, for the same reason as [`Self::take_chat`]:
    /// each event must be handled exactly once. See
    /// [`ObjectManager::take_events`].
    pub fn take_events(&self) -> Vec<crate::play::spells::PlayerEvent> {
        lock(&self.world).take_events()
    }

    /// Play a text emote: an `EmotesText.dbc` id, which the server turns into
    /// an `Emotes.dbc` id and broadcasts as `SMSG_EMOTE`.
    pub fn text_emote(&self, text_emote: u32, emote_num: u32, target: u64) {
        self.send(Command::TextEmote { text_emote, emote_num, target });
    }

    /// Spend a talent point; see [`Command::LearnTalent`].
    pub fn learn_talent(&self, talent_id: u32, rank: u32) {
        self.send(Command::LearnTalent { talent_id, rank });
    }

    /// Say something. A leading `.` makes it a GM command; that is the
    /// server's convention, not this client's. See [`crate::play::chat`].
    pub fn say(&self, kind: ChatType, target: Option<String>, text: String) {
        self.send(Command::Chat { kind, target, text });
    }

    /// Take the chat that has arrived since the last call.
    ///
    /// Drained rather than cloned, because each line must be shown exactly
    /// once; see [`ObjectManager::take_chat`]. Sender names are resolved here,
    /// while the lock is held and the caches are available.
    pub fn take_chat(&self) -> Vec<(String, crate::play::chat::ChatMessage)> {
        // The object manager cannot supply the player's own name. A say is
        // echoed to its sender, so much of the chat this client receives is its
        // own, and `CMSG_NAME_QUERY` is never sent for the player (the name
        // came from the character list), so `name_of` falls back to
        // "Player 319". The name is read before taking the world lock, so the
        // two locks are never held at once.
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
    /// If the thread has already failed, returns the failure's reason, which
    /// is more useful to show a user than "timed out".
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

    /// Stop, and take the world socket back. Returns `None` if the session
    /// ended for any reason other than a requested stop.
    ///
    /// This is the only conversion out of a `LiveSession`, and it has one
    /// caller. 1.12 returns to character select on the same connection, and
    /// the server answers `CMSG_PLAYER_LOGIN` only on the connection the
    /// character list came from. A client that closed the socket on
    /// `SMSG_LOGOUT_COMPLETE` could return only by logging on again.
    ///
    /// Takes `self` because nothing is left afterwards: the thread is joined
    /// and the socket has moved. Do not call it while the character is still
    /// in the world: the server keeps the session, but this client's world
    /// state is dropped with the handle.
    ///
    /// The socket is returned in the mode the loop left it, which is
    /// non-blocking; a caller that wants to block for a reply must set that.
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
/// is a cache of the server's, so continuing with it is better than stopping
/// the session as well.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The thread's state. A separate struct so `run` reads as straight-line code
/// rather than a closure with a dozen captured variables.
struct SessionLoop {
    session: WorldSession,
    character: CharListEntry,
    ground: Option<GroundHeight>,
    /// The volumes the character reports standing in; see
    /// [`crate::play::areatrigger`]. Optional for the same reason as
    /// [`Self::ground`]: a session with no game data still runs, but cannot
    /// enter a dungeon.
    triggers: Option<TriggerTable>,
    /// Which triggers the character is standing in now, and so which may not
    /// fire again. This is client-side latching; see [`TriggerWatch`].
    trigger_watch: TriggerWatch,
    /// When the trigger check last ran. It runs on its own interval,
    /// [`crate::play::areatrigger::CHECK_INTERVAL`], rather than every session
    /// tick, which is four times as frequent.
    last_trigger_check: Instant,
    world: Arc<Mutex<ObjectManager>>,
    status: Arc<Mutex<SessionStatus>>,
    commands: Receiver<Command>,

    /// The client's own player: the mover, its map, and the session clock.
    ///
    /// Held as one value rather than as separate fields because the packet
    /// handlers need exactly this state. See [`crate::socket::handler`], which
    /// holds the teleport and speed-change handling that was previously a
    /// second dispatch in this file.
    local: LocalState,
    /// Replies the last packet queued, reused rather than reallocated.
    replies: Replies,
    /// When the simulation was last advanced, so the next step can be the time
    /// that actually passed. See [`SessionLoop::step_ms`].
    last_advance: Instant,
    /// Movement keys and jumps drained from the channel and not yet applied,
    /// each with the moment it was given. [`SessionLoop::tick_movement`]
    /// applies them part-way through the step, at those moments.
    inputs: Vec<(Instant, Input)>,
    /// Simulated milliseconds, published as [`SessionStatus::world_ms`].
    world_ms: u64,
    /// The sub-millisecond remainder of the last step.
    ///
    /// The world advances in whole milliseconds (`Spline::elapsed_ms` is a
    /// `u32`), so truncating each step would lose up to 1 ms per step. At forty
    /// steps a second that makes every spline in the world, and nothing else,
    /// run up to 4% slow, and creatures would gradually fall behind their paths
    /// while every individual number looked correct.
    carry_ms: f64,
    /// Steps that overran [`MAX_STEP`], and the worst one, published as
    /// [`SessionStatus::stalls`], which gives the reason.
    stalls: u32,
    worst_stall_ms: u32,
    /// Carries that moved the character further than a transport can travel,
    /// and the worst one, published as [`SessionStatus::platform_jumps`].
    platform_jumps: u32,
    worst_platform_jump: f32,
    /// Ticks whose [`READ_SLICE`] ran out with the socket still delivering, and
    /// total ticks, published as [`SessionStatus::read_slice_overruns`].
    read_slice_overruns: u32,
    ticks: u32,
    last_movement_sent: Instant,
    last_facing_sent: Instant,
    /// When the deck under the character became unplaceable, or `None` while
    /// the world can place it. See [`PLATFORM_HOLD`], which bounds it and gives
    /// the reason for the bound.
    platform_held_since: Option<Instant>,
    last_ping: Instant,
    ping_sequence: u32,
    last_query: Instant,
    /// Every entry and guid this session has already sent a query for, so that
    /// [`SessionLoop::resolve_names`] can send a new query immediately and
    /// re-send an unanswered one on the query interval. That function gives
    /// the reasons for both.
    asked_creatures: HashSet<u32>,
    asked_gameobjects: HashSet<u32>,
    asked_items: HashSet<u32>,
    asked_players: HashSet<u64>,
    asked_guilds: HashSet<u32>,
    /// `Entity::position_updates` for the player at the last resync, so a
    /// server correction can be distinguished from local dead reckoning.
    seen_position_updates: u32,
    /// `SMSG_LOGOUT_COMPLETE` has arrived: stop cleanly after this tick and
    /// leave the socket for reuse. See [`SessionLoop::handle_packet`].
    left_world: bool,
    /// The on-disk query answers. Seeded into the world before this thread
    /// started, and appended to on the query interval; see
    /// [`crate::play::wdb`].
    caches: Caches,
    stats: PumpStats,
    /// When [`SessionStatus::traffic`] was last rebuilt. `None` until the first
    /// publish, so the panel has a map to read from the first tick rather than
    /// from 200 ms in.
    traffic_published: Option<Instant>,
}

/// How often [`SessionStatus::traffic`] is rebuilt.
///
/// 200 ms, the interval at which the debug panel samples every other count;
/// see `vale_client::ui::debug`. The traffic maps are the only part of the
/// snapshot larger than a machine word, and rebuilding them every tick would
/// allocate a hundred `String`s forty times a second for a reader that is
/// usually absent.
const TRAFFIC_INTERVAL: Duration = Duration::from_millis(200);

/// Whether this query should be sent on this pass. This implements the split
/// between new and unanswered queries that [`SessionLoop::resolve_names`] is
/// built on.
///
/// A free function rather than a method so the rule can be tested with no
/// socket and no world, as this crate does for every decision a handler makes.
/// It has one intended side effect: the first call records the key, so a
/// later pass over the same entry can distinguish "never asked" from "asked
/// and still waiting".
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
            // Replaced with the server's position once the login burst
            // arrives; the character-list entry is only an approximate
            // starting point until then.
            local: LocalState::new(Mover::new(start, Speeds::default()), map_id),
            replies: Replies::default(),
            last_advance: now,
            inputs: Vec::new(),
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
            // Set one interval in the past so the first tick queries names
            // rather than waiting an interval with placeholders such as
            // "entry 448" on screen.
            last_query: now - QUERY_INTERVAL,
            asked_creatures: HashSet::new(),
            asked_gameobjects: HashSet::new(),
            asked_items: HashSet::new(),
            asked_players: HashSet::new(),
            asked_guilds: HashSet::new(),
            seen_position_updates: 0,
            left_world: false,
            caches,
            stats: PumpStats::default(),
            traffic_published: None,
        }
    }

    fn run(&mut self) -> Result<(), String> {
        // A polling loop must not rely on SO_RCVTIMEO; see
        // `WorldSession::set_nonblocking` for what Winsock does in that case.
        self.session
            .set_nonblocking(true)
            .map_err(|e| format!("socket setup: {e}"))?;
        self.enter_world()?;

        loop {
            let tick_start = Instant::now();

            self.read_socket().map_err(|e| format!("world stream: {e}"))?;
            // The character has left the world, so nothing below may run. The
            // calls that follow write to the socket about a character that no
            // longer exists; see `handle_packet`, which sets this on
            // `SMSG_LOGOUT_COMPLETE`. The status is published once more so the
            // client sees `in_world` become false on the same tick.
            if self.left_world {
                self.publish_status();
                break;
            }
            // Drained after the socket and immediately before the step is
            // timed, so every key change stamped before the step's end is in
            // this step. Drained before `read_socket`, a change made while the
            // socket was being read (up to `READ_SLICE`) fell into the next
            // step and was applied at its start, later than the renderer's
            // prediction applied it, which drew a snap back on the next
            // reading.
            if matches!(self.drain_commands(), Flow::Stop) {
                break;
            }
            // Sampled here rather than at the top of the loop, so the step is
            // timestamped when it is taken: `read_socket` may have spent a
            // whole `READ_SLICE` draining a busy zone, and that time belongs to
            // this step, not the previous one.
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

    /// Send `CMSG_PLAYER_LOGIN`, then read until the server has sent the
    /// player object.
    fn enter_world(&mut self) -> Result<(), String> {
        self.session
            .player_login(self.character.guid)
            .map_err(|e| format!("player login: {e}"))?;

        // The burst is large and arrives over a second or two; anything that
        // has not arrived by the time the player object has will arrive later
        // as an ordinary update.
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

        // The mover is corrected, not replaced.
        //
        // The code previously did `self.local.mover = Mover::new(position,
        // speeds)`, because everything before this point was estimated from
        // the character-list entry. But the burst above is handled, not only
        // read: `read_socket` runs the full dispatch for a `READ_SLICE` window,
        // so anything the server sent while waiting for the self object has
        // already been applied to this mover, and that can include a ride.
        //
        // `Player::ContinueTaxiFlight` runs at the end of `HandlePlayerLogin`,
        // after `Map::Add`, so logging in mid-flight puts an
        // `SMSG_MONSTER_MOVE` naming the player's guid in the same burst as the
        // create block that ends this loop, usually in the same read. A new
        // `Mover` discarded both parts of the ride: the spline, so the
        // character never flew, and `finished_ride`, so the
        // `CMSG_MOVE_SPLINE_DONE` the server was waiting for was never sent.
        //
        // The missing `CMSG_MOVE_SPLINE_DONE` made the fault permanent.
        // `MoveSplineInit::Launch` sets `SetSplineDonePending(true)` and
        // `HandleMovementOpcodes` discards every movement packet while it is
        // set, so the character walked around locally for the rest of the
        // session while the server kept them on a gryphon at the last known
        // waypoint.
        self.local.mover.resync(position);
        self.local.mover.speeds = speeds;
        self.seen_position_updates = self.position_updates_of(self.character.guid);

        // Without this the server treats the client as a "fake client" and
        // discards every movement packet without a reply; see
        // `WorldSession::set_active_mover`.
        self.session
            .set_active_mover(self.character.guid)
            .map_err(|e| format!("set active mover: {e}"))?;
        // Set the two local values that the packet above asserts. The mover is
        // the character from here on, and `Entity::client_controlled` records
        // it. [`Self::tick_view`] derives the mover from that flag every tick,
        // and the server does not send `SMSG_CLIENT_CONTROL_UPDATE` at login.
        // Without this seed the first tick would take the mover away from the
        // character.
        self.local.mover_guid = self.character.guid;
        if let Some(player) = lock(&self.world).get_mut(self.character.guid) {
            player.client_controlled = true;
        }

        // An immediate heartbeat confirms the mover was accepted and gives the
        // anticheat a non-zero `ctime` to measure the next packet against.
        self.send_movement(Opcode::MSG_MOVE_HEARTBEAT)
            .map_err(|e| format!("first heartbeat: {e}"))?;

        let mut status = lock(&self.status);
        status.in_world = true;
        status.position = position;
        status.speeds = speeds;
        // The map is the one field here that the character-list row can have
        // wrong. `SMSG_LOGIN_VERIFY_WORLD` arrived in the burst above and
        // changed `local.map_id` if the server relocated the character out of
        // a reset instance; see [`crate::state::movement::LoginVerifyWorld`].
        //
        // Published here rather than by the first `publish_status`, because
        // `wait_until_in_world` returns once `in_world` is set above. A caller
        // that reads the map as soon as it is told there is a world would
        // otherwise get the seeded map, on about one tick in three.
        status.map_id = self.local.map_id;
        Ok(())
    }

    fn drain_commands(&mut self) -> Flow {
        loop {
            match self.commands.try_recv() {
                Ok(Command::Controls(controls, at)) => {
                    // Queued rather than applied, so the change takes effect at
                    // the moment it was read; see [`Self::tick_movement`]. The
                    // renderer sends the keys every frame, so only a change is
                    // queued.
                    let held = self
                        .inputs
                        .iter()
                        .rev()
                        .find_map(|(_, input)| match input {
                            Input::Controls(c) => Some(*c),
                            Input::Jump => None,
                        })
                        .unwrap_or_else(|| self.local.mover.controls());
                    if controls != held {
                        self.inputs.push((at, Input::Controls(controls)));
                    }
                }
                Ok(Command::Face(orientation)) => {
                    // Through the mover rather than writing its field:
                    // mouse-look is a turn and follows the same rule as the
                    // turn keys, so a stunned or dead character does not turn
                    // with the mouse.
                    self.local.mover.face(orientation);
                }
                Ok(Command::Pitch(pitch)) => {
                    // No packet: the pitch is a field of the movement block, not
                    // a transition, and is carried only under
                    // `MOVEFLAG_SWIMMING`, so it goes out with the next
                    // heartbeat or facing update. `MSG_MOVE_SET_PITCH` exists
                    // (219) and this client does not send it: the value is
                    // already in every block, and the server reads nothing from
                    // the opcode that the block does not carry. The ascent
                    // controlled by keys is separate and has its own packets;
                    // see [`Mover::pitch_flags`].
                    self.local.mover.set_pitch(pitch);
                }
                Ok(Command::Jump(at)) => {
                    // Queued with the keys, in the order given, so a key
                    // pressed in the same frame as the jump is held when the
                    // jump takes its speed and heading. The packet is sent in
                    // the same tick rather than with the next heartbeat: the
                    // server anchors the whole parabola at the first packet
                    // carrying `MOVEFLAG_JUMPING` (`MovementInfo::Read`), so a
                    // jump reported a tick late is placed a tick further along.
                    self.inputs.push((at, Input::Jump));
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
                    // Zero means "nothing selected" on the wire; there is no
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
                    // The autocast toggle gets no reply (vmangos records it and
                    // sends no packet), so the bar's copy is flipped here.
                    // Otherwise the autocast marker never changes and the next
                    // press sends the same state again. See
                    // [`crate::state::objects::ObjectManager::apply_pet_autocast`].
                    // A reaction or command press works the same way; see
                    // [`crate::state::objects::ObjectManager::apply_pet_press`].
                    match &verb {
                        PetVerb::Autocast { spell_id, on, .. } => {
                            lock(&self.world).apply_pet_autocast(*spell_id, *on);
                        }
                        PetVerb::Action { data, target, .. } => {
                            lock(&self.world).apply_pet_press(*data, *target != 0);
                        }
                        PetVerb::StopAttack(_) => lock(&self.world).apply_pet_stop_attack(),
                        // A drag-and-drop also gets no reply
                        // (`HandlePetSetAction` ends at `SetActionBar`), so the
                        // moved slots are written to the copy here; otherwise
                        // the next rebuild reverts the drop on screen.
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
                Ok(Command::Guild(verb)) => {
                    if self.session.guild(&verb).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Petition(verb)) => {
                    if self.session.petition(&verb).is_err() {
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
                // The loop keeps running after a logout request. The character
                // is seated and rooted until `SMSG_LOGOUT_COMPLETE` or a
                // cancel, and both arrive on this socket, so stopping the
                // thread here would leave nothing to receive the answer.
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
                    // The language the character speaks, which is the only one
                    // the server accepts from them.
                    let language = Language::for_race(self.character.race);
                    if self
                        .session
                        .say(kind, language, target.as_deref(), &text)
                        .is_err()
                    {
                        return Flow::Stop;
                    }
                }
                // Death and resurrection commands. None is predicted: each is
                // answered by a values block rather than a reply packet
                // (`PLAYER_FLAGS_GHOST` set, health restored), so there is no
                // local state to change and nothing to roll back if the server
                // refuses. See `crate::play::death`.
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
                Ok(Command::RandomRoll { min, max }) => {
                    if self.session.random_roll(min, max).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::MinimapPing { x, y }) => {
                    if self.session.minimap_ping(x, y).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::RaidTargetSet { icon, guid }) => {
                    if self.session.raid_target_set(icon, guid).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::RaidTargetList) => {
                    if self.session.raid_target_list().is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::PushQuest { quest_id }) => {
                    if self.session.push_quest(quest_id).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::QuestConfirmAccept { quest_id }) => {
                    if self.session.quest_confirm_accept(quest_id).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::QuestPushResult { sharer, result }) => {
                    if self.session.quest_push_result(sharer, result).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::TutorialFlag { id }) => {
                    if self.session.tutorial_flag(id).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::TutorialClear) => {
                    if self.session.tutorial_clear().is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::TutorialReset) => {
                    if self.session.tutorial_reset().is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::ToggleWorn { helm }) => {
                    if self.session.toggle_worn(helm).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::Inspect { guid }) => {
                    if self.session.inspect(guid).is_err() {
                        return Flow::Stop;
                    }
                }
                Ok(Command::InspectHonor { guid }) => {
                    if self.session.inspect_honor(guid).is_err() {
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
                // The tick used its whole read budget and left packets
                // unread. This is counted because the failure has no other
                // symptom: if the stream arrives faster than a 12 ms slice of
                // each 25 ms tick can drain it, the backlog sits in the
                // kernel's receive buffer and this client shows the world as
                // it was some seconds ago, with every position, animation and
                // counter on screen consistent but late. A busy realm can
                // cause this and a localhost server does not, which matches
                // the desync reports this counter was added for.
                //
                // It over-counts one case: a slice that ran to the deadline
                // and would have found the socket empty on its next read. The
                // error is on the side of reporting a backlog, and the useful
                // reading is the proportion of ticks, not the count.
                self.read_slice_overruns = self.read_slice_overruns.saturating_add(1);
                return Ok(());
            }
        }
    }

    /// Answer a query from the on-disk cache instead of sending it.
    ///
    /// `cached` is the kind and key a command asks for, or `None` for a
    /// command that is not a query. A hit runs the cached body through
    /// [`Self::handle_packet`] under the response opcode, so every reader (the
    /// event queue, the arrival edges, the counters) sees exactly what it would
    /// have seen from the server, one tick later rather than one round trip
    /// later. Returns `Ok(true)` when the packet need not be sent.
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
    /// All dispatch is in [`crate::socket::handler`]. This function does two
    /// things: it publishes to the status, and it detects that a packet
    /// relocated the player.
    ///
    /// An earlier version held a second dispatch here. It intercepted seven
    /// opcodes and returned early, because the shared `apply_packet` took only
    /// an `ObjectManager` and could neither answer the server nor move the
    /// local player. With two dispatches, `SMSG_PONG` was handled in both
    /// places, `stats.packets` was counted by hand in each branch, and a speed
    /// change for another unit was acknowledged and then dropped.
    ///
    /// The replies are sent after the world lock is released, the same rule
    /// `resolve_names` follows: a socket write under that lock makes every
    /// reader wait on network latency.
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

        // `SMSG_LOGOUT_COMPLETE` is the only packet that ends the loop from the
        // server's side, for two reasons. The character is out of the world,
        // so every heartbeat and facing update `tick_movement` would keep
        // sending is an opcode vmangos logs as unexpected. And the socket is
        // about to change owner: 1.12 returns to character select on it (see
        // [`LiveSession::reclaim`]), and a thread still writing to a
        // connection it has handed over desynchronises the stream permanently.
        //
        // Detected from the opcode rather than from the event queue on
        // purpose: the queue has exactly one consumer, and it is the client.
        if pkt.is(Opcode::SMSG_LOGOUT_COMPLETE) {
            self.left_world = true;
        }

        // A packet moved the player. The mover has already taken the position,
        // so comparing positions cannot distinguish this from local dead
        // reckoning; the relocation counter does, as `Entity::position_updates`
        // does for entities.
        //
        // Resetting the baseline matters because the player's own create block
        // arrives again in a far teleport's burst and increments
        // `position_updates`. Without this, `tick_movement` would treat that as
        // a server correction and resync the mover to a position on the map
        // just left.
        if self.local.relocations != relocations {
            self.seen_position_updates = self.position_updates_of(self.mover_guid());
            // A map crossing restarts the platform hold. A character carried
            // across on a boat has already used part of `PLATFORM_HOLD`
            // waiting for the server's teleport, and the far side then needs
            // the whole hold again for the deck's create block, its query and
            // its `.wmo` staging. Measuring both waits on one clock put
            // characters in the sea at Theramore. After a teleport the deck is
            // a different placement, so the hold starts again. See
            // [`platform_under`].
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
    /// This is the loop's real clock. `TICK` is only how long the loop tries to
    /// sleep, and on Windows a `thread::sleep` shorter than the ~15.6 ms
    /// scheduler granularity rounds up to it. The true period therefore varies,
    /// and a fixed step would turn that variation into a varying speed. See the
    /// module comment for why the server's checks also need measured time.
    fn step_ms(&mut self) -> Step {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_advance);
        self.last_advance = now;

        // The world clock is real time and is not clamped. Every spline is
        // advanced by it and the renderer's interpolator keys on it, and both
        // assume world time and real time run at the same rate; see
        // [`SessionStatus::world_ms`] and the renderer's `world::motion`.
        // Clamping it broke that assumption without any error; the visible
        // effect is described at [`MAX_STEP`].
        let step = split_step(&mut self.carry_ms, elapsed);
        self.world_ms += u64::from(step.world_ms);
        if elapsed > MAX_STEP {
            let stalled = elapsed.as_millis().min(u128::from(u32::MAX)) as u32;
            self.stalls = self.stalls.saturating_add(1);
            self.worst_stall_ms = self.worst_stall_ms.max(stalled);
        }
        step
    }

    /// Advance every object the server is moving.
    ///
    /// Objects move on the same ground the local character walks on, bound to
    /// the current map for the same reason [`Self::tick_movement`] rebinds it
    /// every step: a stale map id would collide everyone in view against a
    /// continent they have left. Without the ground, every dead-reckoned player
    /// walks through the wall their own client stopped them at; see
    /// [`ObjectManager::advance`].
    fn tick_world(&mut self, dt_ms: u32) {
        if dt_ms == 0 {
            return;
        }
        let bound = self.ground.as_ref().map(|world| OnMap {
            world: world.as_ref(),
            map_id: self.local.map_id,
            // Other units' dead reckoning, which has no ferry.
            adrift: false,
        });
        let bound = bound.as_ref().map(|b| b as &dyn Footing);
        let mut world = lock(&self.world);
        // A stall is advanced in tick-sized slices rather than one stride, and
        // the slicing is done here rather than inside `advance`. The two kinds
        // of movement in `advance` need different things from a long step: a
        // spline must get all of it, because the server moved the unit the
        // whole way and a cyclic path is never re-sent; a dead reckoning must
        // not get it in one stride, because the stride is this client's
        // estimate and `Footing::step` is designed for 0.2-yard strides.
        // Slicing satisfies both: the splines see the same total, and the dead
        // reckoning sees the same sequence of steps a thread without the stall
        // would have taken. See `ObjectManager::advance`.
        for slice in slices(dt_ms) {
            world.advance(slice, bound);
        }
    }

    /// Advance the local simulation and send whatever it owes the server.
    fn tick_movement(&mut self, dt_ms: u32) -> io::Result<()> {
        // The body this function moves: the character's, except during a
        // possess. See [`Self::tick_view`], which derives it.
        let mover = self.mover_guid();
        // No mover: the character is feared or confused and the server moves
        // the body. The client sends no `CMSG_SET_ACTIVE_MOVER` for a zero
        // guid, and there is no movement to send. The controls are zeroed
        // rather than ignored, so a key held when control was taken away is
        // not still held when it returns.
        if mover == 0 {
            self.local.mover.set_controls(Controls::default());
            self.inputs.clear();
            return Ok(());
        }
        // Let the ground load ahead: the tile under the character and the
        // ring around it, before the stride below asks for a height there.
        if let Some(ground) = self.ground.as_ref() {
            let at = self.local.mover.position();
            ground.focus(self.local.map_id, at.x, at.y);
        }

        // A server correction (teleport, knockback, a fall the server
        // resolved) always overrides dead reckoning.
        let updates = self.position_updates_of(mover);
        if updates != self.seen_position_updates {
            self.seen_position_updates = updates;
            if let Some((position, speeds)) = self.snapshot_of(mover) {
                self.local.mover.resync(position);
                self.local.mover.speeds = speeds;
            }
        }

        // What the server has stopped this character doing, read again every
        // tick. Neither state has its own packet: a stun is a change to
        // `UNIT_FIELD_FLAGS` inside the values block that carried the aura,
        // and death is `UNIT_FIELD_HEALTH` reaching zero. There is no packet
        // to hook, so the state is read here, next to the step that obeys it.
        // See `movement::Restraint`.
        let restraint = {
            let world = lock(&self.world);
            world.get(mover).map(|e| crate::state::movement::Restraint {
                stunned: e.is_stunned(),
                dead: e.is_dead().unwrap_or(false),
                // The one of the three that persists across sessions, so it is
                // read here with the other two rather than left to the ride;
                // see `Restraint::on_taxi`.
                on_taxi: e.is_on_taxi(),
            })
        };
        if let Some(restraint) = restraint {
            self.local.mover.set_restraint(restraint);
        }

        // Boarding, carrying and stepping off happen before the step, not
        // after.
        //
        // A character standing on a moving platform must first be moved by the
        // platform's movement, so that their own stride starts where the deck
        // has put them. Done afterwards, it is a correction rather than a ride.
        // On the Deeprun Tram, whose car travels 2,482 yards, that was the
        // reported bug: the floor moved away and the character stayed behind.
        //
        // The transport filter is here rather than in the world, because
        // `UPDATEFLAG_TRANSPORT` is the only data that marks an object as
        // moving and only the object manager holds it. Without the filter a
        // character standing on any door or chest would be boarded onto it,
        // and the server would refuse the guid.
        //
        // The rule is [`platform_under`], a free function so that it can be
        // tested without a socket.
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
            // Detect a wrong platform placement. This is the only place it can
            // be seen; see [`SessionStatus::platform_jumps`].
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

        // Bound to the character's current map, so a far teleport changes the
        // ground under it as well as moving the character.
        let bound = self.ground.as_ref().map(|world| OnMap {
            world: world.as_ref(),
            map_id: self.local.map_id,
            adrift,
        });
        let bound = bound.as_ref().map(|b| b as &dyn Footing);
        // Each queued input is applied at its own moment in the step: its
        // distance from the step's start. The 1.12.1 client does the same, so
        // a key released mid-step stops the character where it was released,
        // and the renderer's prediction, which applies the key in the frame it
        // was read, agrees with the next reading instead of being corrected by
        // it.
        //
        // The step is the last `dt_ms` before `last_advance`. After a stall
        // longer than `MAX_STEP` that is the end of the stall, which is also
        // the part of a long frame the 1.12.1 client simulates: it drops the
        // excess at the start and moves the character over the last 250 ms.
        let step_from = self
            .last_advance
            .checked_sub(Duration::from_millis(u64::from(dt_ms)))
            .unwrap_or(self.last_advance);
        let inputs: Vec<(f32, Input)> = self
            .inputs
            .drain(..)
            .map(|(at, input)| (at.saturating_duration_since(step_from).as_secs_f32(), input))
            .collect();
        let owed = self
            .local
            .mover
            .advance_through(dt_ms as f32 / 1000.0, &inputs, bound);

        // Record where the stride left the passenger, in the platform's frame.
        //
        // This completes the carry above and must come after the stride, not
        // next to the carry: the server takes the offset on the wire as the
        // passenger's position, so an offset taken before the stride describes
        // where they stood a tick ago and the next carry puts them back there.
        // See `Mover::stow_platform`. It does nothing in a session that never
        // boards a transport.
        self.local.mover.stow_platform();

        // Write the simulated position to the mover's entity, so that readers
        // of the object manager see where the client is rather than the last
        // position the server sent. The character's own entity is not updated
        // while another body is being driven, which leaves the character
        // standing where the possess began.
        let position = self.local.mover.position();
        if let Some(entity) = lock(&self.world).get_mut(mover) {
            entity.position = Some(position);
        }

        // Leaving the ground and landing are reported immediately; see
        // `Mover::advance`, and `CHEAT_TYPE_BAD_FALL_STOP` for the anticheat
        // result of a landing reported in an ordinary heartbeat.
        // Key changes and jumps are reported the same way, each with the
        // position at its own moment.
        if !owed.is_empty() {
            for owed in owed {
                self.send_owed(owed)?;
            }
            self.last_facing_sent = Instant::now();
            return Ok(());
        }

        // A server-driven ride has ended; acknowledge it. Until this packet
        // arrives, `HandleMovementOpcodes` discards every movement packet
        // below at its first check, so a Charge would block movement for the
        // rest of the session; the only other thing that clears the flag is a
        // login. See `movement::Mover::ride`.
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
        // While a ride is still running there is nothing to send: the server
        // moves its own copy of the same spline and drops anything sent.
        if self.local.mover.is_riding() {
            return Ok(());
        }

        // A facing change is reported whether or not the character is moving,
        // because of the anticheat. `CheckSpeedHack` re-runs
        // `ExtrapolateMovement` from the last packet's orientation over the
        // client-time delta and accumulates `m_overspeedDistance` beyond a 10%
        // allowance. Turning while running therefore moves the server's
        // straight line away from the arc actually walked for as long as the
        // gap lasts: at 7.6 y/s over a 400 ms heartbeat, a half-turn separates
        // them by a couple of yards each time, against `Threshold = 30` yards
        // in total. On screen the client drifts and is snapped back; the
        // server treats it as a speed hack.
        let turned = self.local.mover.facing_changed()
            && self.last_facing_sent.elapsed() >= FACING_INTERVAL;

        // Boarding and stepping ashore are reported immediately, because they
        // are the only packets that tell the server a character is on a
        // transport; see `Mover::ferry_changed`, which describes the cost of
        // each when late. Not rate-limited: it is one packet per boarding, and
        // the anticheat's transport checks read the previous packet's flag, so
        // a transition reported in the next heartbeat is measured in the wrong
        // frame.
        if self.local.mover.ferry_changed() {
            let boarded = self.local.mover.ferry().is_some();
            self.send_movement(Opcode::MSG_MOVE_HEARTBEAT)?;
            self.last_facing_sent = Instant::now();
            // A boarding also requests the deck's clock again, and this is the
            // only packet that can correct it. A transport's position is not
            // sent on the wire: both ends run the same schedule over the same
            // shipped table and agree on timing through
            // `UPDATEFLAG_TRANSPORT`'s path-progress word. This client receives
            // that word once, in the create block `Map::SendInitTransports`
            // sends at login, and advances it on its own clock afterwards. Any
            // world time this client drops after that (`slices` caps a stall
            // at `MAX_CATCH_UP`) leaves its boat permanently early, and an
            // early boat crosses to the other continent before the server's.
            //
            // `HandleMoveTimeSkippedOpcode` re-sends the transport's
            // out-of-range and create blocks to a player it has just boarded
            // (the `SetJustBoarded` pair in `HandleMoverRelocation`, whose
            // comment calls it a 1.12 client fix), and that create block
            // carries the phase again. The schedule is therefore
            // resynchronised when it matters: when somebody steps aboard.
            if boarded {
                self.session.send(
                    Opcode::CMSG_MOVE_TIME_SKIPPED,
                    &crate::state::movement::time_skipped_body(mover, 0),
                )?;
            }
            return Ok(());
        }

        // A passenger sends heartbeats whether or not it is walking. Standing
        // still on a deck sets no flag in `MOVEFLAG_MASK_MOVING`, so the two
        // branches below would send nothing for the whole crossing, and the
        // server measures the next packet against the last one it received,
        // which by then is an ocean away. The test is `carried` (the deck moved
        // this tick) rather than being aboard, so a character standing on a
        // docked boat or a stopped lift sends nothing extra: the server's copy
        // of their position cannot go stale while the platform is still.
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

    /// Report standing in an area trigger. This is how a character enters a
    /// dungeon, and it is the only packet this client sends without a prompt
    /// from the user or the server. See [`crate::play::areatrigger`] for why
    /// the client is responsible for it.
    ///
    /// Runs after [`Self::tick_movement`] so it reads the position just
    /// simulated rather than the previous tick's. The server re-validates this
    /// packet against its own 5-yard tolerance, and a stale position makes a
    /// portal refuse at its boundary.
    fn tick_triggers(&mut self) -> io::Result<()> {
        if self.last_trigger_check.elapsed() < crate::play::areatrigger::CHECK_INTERVAL {
            return Ok(());
        }
        self.last_trigger_check = Instant::now();
        // A trigger applies to the character, and during a possess the mover
        // is not the character. `HandleAreaTriggerOpcode` acts on `_player`,
        // so an Eye of Kilrogg flown into an instance portal would make the
        // server move the warlock through it. The character stands still for
        // the duration, so there is nothing to poll.
        if self.mover_guid() != self.character.guid {
            return Ok(());
        }
        let Some(triggers) = self.triggers.as_deref() else {
            return Ok(());
        };
        // The mover's position, not the object manager's. They differ by up to
        // a tick of dead reckoning, and this one is what the movement packets
        // report, so the server validates the trigger against the position it
        // has received.
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

    /// Where this client views the world from, and which body the keys drive.
    /// The 1.12.1 client re-evaluates both continuously, and this function
    /// does so every tick.
    ///
    /// Both come from one field. `PLAYER_FARSIGHT` names either a far-sight
    /// `DynamicObject` (Eagle Eye, Far Sight, Bird's Eye) or a possessed unit
    /// (Eye of Kilrogg, Mind Control, Eyes of the Beast). The rule is:
    ///
    /// * No far-sight guid, or the guid is the character's own: the view is
    ///   the character's, and the mover is the character if the character is
    ///   client-controlled, otherwise none.
    /// * A guid the object manager cannot find: the same as above; the view
    ///   does not switch.
    /// * A guid naming a unit that is client-controlled: both the view and the
    ///   mover are that unit.
    /// * A guid naming anything else: the view is that object and the mover
    ///   is unchanged.
    ///
    /// For the client, possession is therefore a form of far sight. That is
    /// not obvious: `SMSG_CLIENT_CONTROL_UPDATE` sets a flag on a unit and
    /// never changes the mover itself (see
    /// [`super::handler::acks::client_control`]); the mover is set by this
    /// function, from that flag together with this field. The last case is
    /// Eagle Eye: the view moves and the player keeps driving the character.
    ///
    /// Two packets result. `CMSG_FAR_SIGHT` is one byte, sent each time the
    /// view switches, tracked by the latch this function keeps. It makes the
    /// server move its own visibility source (`Camera::SetView(obj, false)`);
    /// without it the camera arrives a hundred yards away with nothing loaded
    /// around it. `CMSG_SET_ACTIVE_MOVER` names the new mover. It is not sent
    /// for a zero mover, matching the 1.12.1 client, because the server's
    /// `GetConfirmedMover` treats a zero guid as "no mover client side, is
    /// this a fake client?".
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
            // The field can arrive a packet or two before the object it names.
            // For those frames the view stays the character's own, which is
            // what the 1.12.1 client does with an unresolved guid.
            Some(_) if !resolves => own_eyes(),
            Some(guid) if is_unit && controls_it => (true, guid),
            // Far sight: the view moves and the mover does not. The mover is
            // kept rather than recomputed, because the 1.12.1 client does not
            // change the mover under far sight.
            Some(_) => (true, self.local.mover_guid),
        };

        if looking != self.local.looking_through {
            self.local.looking_through = looking;
            self.session.send(Opcode::CMSG_FAR_SIGHT, &[u8::from(looking)])?;
        }
        self.set_mover(mover)
    }

    /// Point the local simulation at a body and tell the server.
    ///
    /// The mover is replaced rather than corrected. This differs on purpose
    /// from the `resync` in [`SessionLoop::enter_world`]: that is the same
    /// character, possibly with a ride already running, while this is a
    /// different body. Carrying the old body's spline, platform or fall arc
    /// over would move the new body relative to whatever the old one was
    /// standing on.
    fn set_mover(&mut self, guid: u64) -> io::Result<()> {
        if guid == self.local.mover_guid {
            return Ok(());
        }
        self.local.mover_guid = guid;
        if guid == 0 {
            // Feared or confused: the server moves the body and there is no
            // mover to name. No packet; see [`Self::tick_view`].
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

    /// Query every visible entry that does not have a name yet. Replies arrive
    /// asynchronously and are applied by [`apply_packet`].
    ///
    /// ## A new entry is queried on the next tick
    ///
    /// This function previously ran only on [`QUERY_INTERVAL`], which put an
    /// average of one second, and at worst two, between the moment a name was
    /// needed and the moment the query was sent. That was the cause of vendors
    /// being slow to show their goods: `SMSG_LIST_INVENTORY` names no item,
    /// the merchant panel queues every entry the item cache cannot answer, and
    /// then nothing was sent for up to two seconds, before a round trip that
    /// takes a few milliseconds against a local server (measured at 40
    /// templates in 156 ms).
    ///
    /// The pass is therefore split in two. New entries, which this session has
    /// never queried, are sent on the tick they appear, so the delay is
    /// [`TICK`] rather than [`QUERY_INTERVAL`]. Unanswered entries, already
    /// queried, are re-sent on the interval. That is the interval's purpose: a
    /// query has no acknowledgement, so an answer that never arrives must be
    /// requested again.
    ///
    /// The `asked_*` sets make the split possible, and they also fix a second
    /// fault: without them, every unresolved entry was re-sent every two
    /// seconds until it resolved, so a vendor with twenty unknown items sent
    /// twenty packets every two seconds until the answers arrived. The sets
    /// last for the session and are never pruned; they are bounded in the same
    /// way as [`ObjectManager::wanted_items`], by the number of distinct
    /// entries one character meets.
    fn resolve_names(&mut self) -> io::Result<()> {
        let retry = self.last_query.elapsed() >= QUERY_INTERVAL;
        if retry {
            self.last_query = Instant::now();
        }

        // Copy the work list out before touching the socket: holding the world
        // lock across a write would block every reader on network latency.
        let (creatures, gameobjects, players, guilds, items, pets, learned) = {
            let mut world = lock(&self.world);
            // The four scans below cover every entity in view, and this
            // function runs every tick rather than on the query interval, so
            // the O(1) check runs first and an idle tick costs one lock and one
            // bool. See [`ObjectManager::take_query_hint`].
            if !world.take_query_hint() && !retry {
                return Ok(());
            }
            // Drained only on the query interval. The disk write below is the
            // only expensive operation in this function, and a login seeds
            // hundreds of templates; batching them every two seconds is the
            // interval's remaining purpose now that new queries do not wait
            // for it.
            let learned = if retry {
                std::mem::take(&mut world.learned)
            } else {
                Vec::new()
            };
            (
                world.unresolved_creature_entries(),
                world.unresolved_gameobject_entries(),
                world.unresolved_player_guids(),
                world.unresolved_guild_ids(),
                world.unresolved_item_entries(),
                world.unresolved_pet_names(),
                learned,
            )
        };
        // Written outside the lock and before the sends, for the same reason
        // the sends are outside it: a disk write under the world lock blocks
        // every reader of the world. A write failure is ignored: an unwritten
        // cache only means the next session starts with fewer cached answers,
        // which does not justify stopping a live session.
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
        // A player object carries a guild id and no guild name. The name
        // under a player's own name and in the unit tooltip is the answer to
        // this query.
        for guild in guilds {
            if !worth_asking(&mut self.asked_guilds, guild, retry) {
                continue;
            }
            self.session
                .send(Opcode::CMSG_GUILD_QUERY, &crate::play::guild::query_body(guild))?;
        }
        // Equipment. A `PLAYER_VISIBLE_ITEM` field carries only the entry, and
        // `Item.dbc` is not in the archives, so only the server can say what an
        // item looks like. The query's GUID is zero, because another player's
        // equipped item is only an entry number here, not an object this
        // client has seen.
        //
        // The loot window is where the query latency matters most: a corpse's
        // rows carry a display id and no name, and `LootFrame` has no refresh
        // event. See [`crate::play::loot`], which describes the consequence.
        for entry in items {
            if !worth_asking(&mut self.asked_items, entry, retry) {
                continue;
            }
            self.session
                .send(Opcode::CMSG_ITEM_QUERY_SINGLE, &query::item_query_body(entry, 0))?;
        }
        // The pet's name, the only name that is not keyed by a guid or an
        // entry: `SMSG_CREATURE_QUERY_RESPONSE` answers a pet with its species
        // ("Wolf"), and the name the player gave it comes back only from
        // `CMSG_PET_NAME_QUERY` with `UNIT_FIELD_PETNUMBER`. Queried once per
        // number rather than through `worth_asking`: the answer empties the
        // set, and a pet number stays the same across dismissal and recall, so
        // a retry loop would query forever for a number the server has already
        // refused.
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

    /// Send a packet the mover owed part-way through a step, with the block
    /// from that moment. The clock is stamped now rather than taken from the
    /// block, so `ctime` never decreases from one packet to the next.
    fn send_owed(&mut self, owed: crate::state::movement::Owed) -> io::Result<()> {
        let mut info = owed.info;
        info.time = self.local.movement_info().time;
        self.session.send_movement(owed.opcode, &info)?;
        self.local.mover.mark_sent();
        self.last_movement_sent = Instant::now();
        lock(&self.status).movement_sent += 1;
        Ok(())
    }

    /// The block to send right now, stamped with the session clock.
    ///
    /// `ctime` must be non-zero and must never decrease (see the anticheat
    /// notes in [`crate::state::movement`]), so it is milliseconds since the
    /// thread started, plus one so the first packet is not zero.
    fn movement_info(&mut self) -> MovementInfo {
        // The `ctime` stamping has one implementation, in `LocalState`. The
        // packet handlers need it to build a speed-change ack, and two
        // implementations of a monotonic clock are two places where it can go
        // backwards. A decrease is `CHEAT_TYPE_TIME_BACK`.
        self.local.movement_info()
    }

    fn publish_status(&mut self) {
        let position = self.local.mover.position();
        let controls = self.local.mover.controls();
        let moving = self.local.mover.info.is_moving();
        // The speed the local simulation is advancing at, not the run speed:
        // walking and swimming have their own speeds, and a server-driven ride
        // uses none of the six; see `Mover::travel_speed`.
        let speed = self.local.mover.travel_speed();
        let riding = self.local.mover.ride_velocity();
        let ferry = self.local.mover.ferry();
        let packets = self.stats.packets as u64;
        let warnings = self.stats.warnings.clone();
        let unhandled = self.stats.other.clone();
        let world_ms = self.world_ms;
        // The game clock is advanced here rather than by its readers. The
        // server sends the time once at login, so the client must advance it,
        // and this loop has the only clock that is monotonic and is the one the
        // world's positions were advanced by. A reader advancing it from its
        // own frame timer would be a second, independent clock, which is the
        // fault `world_ms` was introduced to avoid.
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
        // Published with its position and under the same lock, because a
        // reader uses it to decide whether the position is new. Published
        // separately, a poll could see a new clock beside an old position,
        // causing a one-step hitch that appears only under load.
        status.world_ms = world_ms;
        // The real instant `world_ms` refers to, which cannot be recovered
        // from a reading afterwards. `last_advance` is the instant `step_ms`
        // measured this step from, so it is exactly when the mover was at
        // `position`.
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
        // The movement block from which the gait, the strafe body offset and
        // the prediction are all derived. Published whole rather than as three
        // derived values: the client's rules are defined on the flags, and a
        // derived value is a place for the two to diverge.
        status.movement = self.local.mover.info;
        status.airborne = self.local.mover.is_airborne();
        status.jumping = self.local.mover.is_jumping();
        status.mover = self.mover_guid();
        // Published next to the block for the same reason the block is
        // published whole: the prediction applies the mover's two restraint
        // rules, and it cannot derive them from the flags alone.
        status.restraint = self.local.mover.restraint();
        status.packets = packets;
        status.warnings = warnings;
        status.unhandled = unhandled;
        // Written by packet handlers rather than by this loop, so they are
        // copied here rather than published when they change: the handlers run
        // under the world lock and must not also take the status lock. Both
        // are read-only outside this thread.
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
        // Rebuilt five times a second rather than on every publish. This is
        // the only field in the snapshot larger than a word: two `BTreeMap`s of
        // opcode names, which would otherwise be rebuilt at 40 Hz for a panel
        // that samples at 5 Hz and is usually closed. See the field's doc; the
        // reader is the debug panel's net tab, which sees no difference.
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
            // Only while the capture is running, so that stopping it leaves the
            // last snapshot readable rather than clearing the panel when the
            // box is unchecked, which is when the packets are needed.
            let capture = self.session.capture();
            if capture.armed() || status.capture.armed {
                // The second condition covers the last publish after the
                // capture stops, so the final packets are in the snapshot and
                // its flag matches the ring buffer.
                status.capture = Arc::new(capture.snapshot());
            }
        }
    }

    // --- accessors for the mover and entity snapshots --------------------

    /// The guid of the body the movement packets describe: the character's
    /// own except during a possess; see [`Self::tick_view`]. Zero means the
    /// server is moving the body and nothing may be sent.
    ///
    /// Set to the character in [`Self::enter_world`] rather than defaulted
    /// here, because it is the character only once this client has sent
    /// `CMSG_SET_ACTIVE_MOVER`, which happens there unconditionally.
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

/// A step, cut into pieces no longer than [`MAX_STEP`]; see
/// [`SessionLoop::tick_world`].
///
/// The total is bounded at [`MAX_CATCH_UP`], the one place time is still
/// dropped. A thread that was away for an hour (a suspended process, a
/// debugger) would otherwise spend thousands of slices moving every entity in
/// the world through terrain lookups whose results are discarded, and by then
/// the session's next packets restate everything anyway. The slicing is meant
/// for stalls of 250 to 2000 ms, not for stalls long enough to reach the bound.
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
    /// The deck is being held although the world has no hull for it, so there
    /// is no floor either; see `OnMap::floor`, which reads this.
    pub adrift: bool,
}

/// Which moving platform is carrying the character this tick.
///
/// A passenger stays aboard until the world can show they stepped off, and it
/// can show that only while the deck's hull is present. [`World::platform`]
/// returns nothing in three situations with different meanings:
///
/// * the character has walked ashore, and the deck's hull is present with
///   nobody standing on it;
/// * the deck's hull is not built: a frame whose hull budget ran out, or a
///   `.wmo` transport the renderer stopped building a hull for because the
///   character is no longer near the transport's new position;
/// * the deck is no longer on this map, so the world has no data about it.
///
/// Only the first is a step ashore, and [`World::platform_hulled`] separates
/// it from the other two. Treating either of the others as a step ashore
/// un-boards the passenger, and `Transport::TeleportTransport` moves only
/// `m_passengers`, so the deck leaves and the character stays where it was.
/// The other two cases therefore keep the passenger aboard:
///
/// * Hull absent, deck still placed: carry against the deck's current
///   position. This is a same-map transport teleport, and only this client
///   following it moves the passenger, because the server sends nothing:
///   `TeleportTransport` relocates a passenger who does not change map with
///   `Unit::TeleportPositionRelocation`, which is server side only. The
///   Grom'Gol–Undercity zeppelin jumps 13,700 yards on one map 173.4 s into
///   its cycle, and a client that does not follow it leaves the character in
///   the air over Stranglethorn.
/// * Not placed at all: hold the last placement, which is the stationary case
///   of the carry and moves nobody. This covers the interval between this
///   client's route reaching the other continent and `Player::TeleportTo`
///   arriving.
///
/// In both cases there is no floor either. Keeping the passenger aboard while
/// returning the sea bed as the height made the character fall off the deck
/// they were still recorded as standing on. Both cases last as long as the
/// object manager still knows the deck as a transport, and are bounded by
/// [`PLATFORM_HOLD`] only after that. A deck that never returns is removed by
/// the server's out-of-range block, and a deck that is only elsewhere must not
/// be un-boarded on a timer (see the constant).
///
/// `moves` is the [`UPDATEFLAG_TRANSPORT`] filter, which the object manager
/// holds and the collision world does not. Without it a character standing on
/// any door or chest would be boarded onto it and the server would refuse the
/// guid. It is a closure because this function must not take the world lock;
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
        // The deck's hull is present and the character is not on it.
        if ground.platform_hulled(map_id, ferry.guid) {
            return None;
        }
        // The hold is bounded only once the deck's entity is gone. While the
        // object manager still knows the entity and flags it as a transport,
        // the deck is real and only elsewhere: this client's schedule is ahead
        // of or behind the server's around a teleport frame, or the far side is
        // still loading. Un-boarding on a timer then sends a packet without
        // the transport flag: `RemovePassenger` runs, and `TeleportTransport`
        // then moves every passenger except this character. vmangos' frame
        // times are the imperfect output of `GenerateWaypoints` with only the
        // period overridden from the database, so the two ends can disagree on
        // the teleport moment by more than any fixed grace period. A character
        // who really is left behind still ends up swimming rather than
        // hovering: the entity is removed by the server's out-of-range block,
        // and [`PLATFORM_HOLD`] runs from there.
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

/// One elapsed duration as the two clocks a step runs on; see [`Step`] and
/// [`MAX_STEP`].
///
/// A free function next to [`whole_ms`] rather than a method, so the rule can
/// be tested without a socket. The rule is that the world takes the real time
/// and the mover takes the clamped time; a test that repeats the clamp instead
/// of calling this function only tests its own arithmetic.
fn split_step(carry: &mut f64, elapsed: Duration) -> Step {
    let world_ms = whole_ms(carry, elapsed);
    // The local character's step is clamped, because the anticheat compares
    // the distance walked against the `ctime` the packet carries, and a
    // multi-second stride after a stall looks like a teleport to it.
    let mover_ms = world_ms.min(MAX_STEP.as_millis() as u32);
    Step { world_ms, mover_ms }
}

/// `elapsed` in whole milliseconds, keeping the remainder in `carry` for the
/// next call.
///
/// Separate from [`SessionLoop::step_ms`] so the accounting can be tested
/// without a clock. The error it prevents is a slow drift, which is invisible
/// in any single step and shows only after a few hundred.
fn whole_ms(carry: &mut f64, elapsed: Duration) -> u32 {
    *carry += elapsed.as_secs_f64() * 1000.0;
    let whole = carry.floor();
    *carry -= whole;
    whole as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test world holding one game object: where it is, whether its hull is
    /// present, and whether the character is on that hull. The three cases
    /// [`platform_under`] distinguishes are combinations of the last two.
    struct Deck {
        /// Where the object is, or `None` for an object this map has no data
        /// about.
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

    /// A game object that does not move never boards anybody. The filter is
    /// `UPDATEFLAG_TRANSPORT`, which only the object manager holds; without it
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

    /// Walking ashore un-boards the character, and the present hull is the
    /// evidence for it. The deck is still placed and still has a hull; the
    /// character is not on it.
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

    /// A same-map transport teleport carries the passenger with it. The Grom'Gol
    /// to Undercity zeppelin jumps 13,700 yards on one map with no packet,
    /// because `TeleportTransport` relocates a passenger who does not change map
    /// on the server side only and sends nothing.
    ///
    /// The client sees the deck's placement jump while its hull stops being
    /// built: the renderer builds a transport's hull only within 150 yards of
    /// the character, and after the jump it is 13,700 yards away.
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

        // The carry that uses this result puts the character on the deck
        // rather than leaving them where they were standing.
        let mut mover = crate::state::movement::Mover::new(standing, Speeds::default());
        mover.carry_platform(Some(gromgol));
        mover.stow_platform();
        assert!(mover.carry_platform(out.platform), "the carry did not happen");
        let landed = mover.position();
        assert!((landed.x - (undercity.position[0] + 5.0)).abs() < 1e-2, "{landed:?}");
        assert!((landed.y - undercity.position[1]).abs() < 1e-2, "{landed:?}");
    }

    /// A deck this map has no data about is still stood on. This covers the
    /// interval between this client's route reaching the other continent and
    /// `TeleportTo` arriving: the boat is not drawn here, so its hull and its
    /// placement are both gone, and un-boarding leaves the character in the sea.
    ///
    /// The deck is held for as long as the world still knows its entity. The
    /// two ends disagree on the teleport moment by no fixed bound (vmangos'
    /// frame times are its own `GenerateWaypoints` output with only the period
    /// overridden), and un-boarding on a timer sends a packet without the
    /// transport flag: `RemovePassenger` runs and the boat crosses without the
    /// character. [`PLATFORM_HOLD`] bounds only the case where the deck's
    /// entity is gone.
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

        // Well past the hold time, with the entity still present: still held.
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

        // The hold is bounded once the entity is gone; otherwise a boat the
        // server destroys leaves the character hovering over open water for
        // the rest of the session.
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

    /// A query a window is waiting on is sent immediately; an unanswered one is
    /// re-sent on the query interval and not before.
    ///
    /// This tests all of [`worth_asking`], which is a free function because the
    /// rule involves no socket. It covers the fault that made a vendor take
    /// seconds to show its goods, and the second fault the sets fix: the same
    /// twenty packets sent every two seconds for as long as the answers were
    /// late.
    #[test]
    fn a_fresh_query_goes_out_at_once_and_a_stale_one_waits_for_the_beat() {
        let mut asked: HashSet<u32> = HashSet::new();
        // Entry 2589 seen for the first time, on an ordinary tick: it is sent.
        assert!(worth_asking(&mut asked, 2589, false));
        // It is not sent again on every tick while the answer is pending.
        assert!(!worth_asking(&mut asked, 2589, false));
        assert!(!worth_asking(&mut asked, 2589, false));
        // On the query interval it is re-sent, because a query has no
        // acknowledgement and an answer that never came must be requested again.
        assert!(worth_asking(&mut asked, 2589, true));
        // A different entry is new regardless of the interval.
        assert!(worth_asking(&mut asked, 858, false));
    }

    /// The latency gained by the split: a new query previously waited up to
    /// [`QUERY_INTERVAL`] and now waits up to one [`TICK`].
    #[test]
    fn the_beat_is_far_longer_than_a_tick() {
        assert!(QUERY_INTERVAL > TICK * 40);
    }

    /// The sub-millisecond remainder has to be carried, not dropped.
    ///
    /// A step is whatever the scheduler allowed (15.6 ms on Windows, rarely a
    /// whole number), and the world advances in `u32` milliseconds. Truncating
    /// each step loses up to 1 ms per step, which slows every spline in the
    /// world, and nothing else, by several percent. One step does not show it;
    /// after a minute a patrolling creature is seconds behind the server's
    /// position.
    #[test]
    fn the_sub_millisecond_remainder_is_carried_rather_than_lost() {
        let mut carry = 0.0;
        let step = Duration::from_nanos(15_625_000); // 15.625 ms, the Windows scheduler period
        let total: u64 = (0..64).map(|_| u64::from(whole_ms(&mut carry, step))).sum();
        // 64 x 15.625 ms is exactly one second, and no millisecond of it may
        // be lost.
        assert_eq!(total, 1000, "the world clock drifts slow");
    }

    /// A stall produces two different step lengths. Giving both clocks the
    /// clamped one caused a cumulative desync.
    ///
    /// The character walks off a stall over several steps rather than in one
    /// stride, and clamping errs short: the server's `CheckSpeedHack` allows
    /// less distance than the packet's `ctime` claims, never more.
    ///
    /// The world is not clamped, because this client does not control its
    /// movement: the server advanced every spline by the whole five seconds,
    /// and a client that advances them by a quarter of a second puts every
    /// creature in view permanently 4.75 s behind. A chase spline's error is
    /// corrected by its next `SMSG_MONSTER_MOVE`; a cyclic spline, which the
    /// server sends once, keeps the error for the rest of the session and adds
    /// each later stall's error to it.
    #[test]
    fn a_stall_is_clamped_for_the_character_and_not_for_the_world() {
        let mut carry = 0.0;
        let step = split_step(&mut carry, Duration::from_secs(5));
        assert_eq!(step.mover_ms, MAX_STEP.as_millis() as u32);
        assert_eq!(step.world_ms, 5_000, "the world lost {} ms", 5_000 - step.world_ms);
    }

    /// A stall reaches the world in pieces of at most [`MAX_STEP`] that add up
    /// to the whole stall. The splines get the whole step, and no dead
    /// reckoning takes a 38-yard stride through a wall.
    #[test]
    fn a_stall_is_sliced_and_the_slices_add_up() {
        let slice = MAX_STEP.as_millis() as u32;
        for stall in [1u32, 25, 250, 251, 600, 5_000] {
            let cut: Vec<u32> = slices(stall).collect();
            assert_eq!(cut.iter().sum::<u32>(), stall, "{stall} ms lost time");
            assert!(cut.iter().all(|s| *s <= slice && *s > 0), "{stall} ms: {cut:?}");
        }
        // An hour-long stall is bounded rather than advanced. This is the one
        // place time is still dropped, on purpose. See `slices`.
        let capped: u32 = slices(3_600_000).sum();
        assert_eq!(capped, MAX_CATCH_UP.as_millis() as u32);
        assert_eq!(slices(0).count(), 0);
    }

    /// In every ordinary tick the two clocks are equal, so the split has no
    /// effect outside a stall.
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
