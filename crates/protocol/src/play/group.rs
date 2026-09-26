//! **The party** — inviting, joining, leaving, and the roster the server keeps.
//!
//! ```text
//! CMSG_GROUP_INVITE       <name>          /invite Bram
//!   -> SMSG_PARTY_COMMAND_RESULT          …to us, always, success or not
//!   -> SMSG_GROUP_INVITE  <name>          …to them, and nothing else
//! CMSG_GROUP_ACCEPT       (empty)         they press Accept
//! CMSG_GROUP_DECLINE      (empty)         …or Decline
//!   -> SMSG_GROUP_DECLINE <name>          …which the *inviter* is told about
//! CMSG_GROUP_DISBAND      (empty)         Leave Party — and Disband, one opcode
//! CMSG_GROUP_UNINVITE     <name>          kick, by name
//! CMSG_GROUP_UNINVITE_GUID <u64>          …or by guid, which is what the menu uses
//! CMSG_GROUP_SET_LEADER   <u64>           promote
//!   -> SMSG_GROUP_SET_LEADER <name>       …broadcast as a *name*
//!   -> SMSG_GROUP_LIST                    the whole roster, to everybody, every time
//!   -> SMSG_GROUP_DESTROYED (empty)       …or this, when the last member leaves
//! CMSG_REQUEST_PARTY_MEMBER_STATS <u64>
//!   -> SMSG_PARTY_MEMBER_STATS_FULL       a member's health, mana, level, zone
//! ```
//!
//! ## `SMSG_GROUP_LIST` is the whole state and it is sent per member
//!
//! There is no incremental roster packet. `Group::SendUpdate` walks the member
//! slots and sends **each player their own copy**, with themselves left out of
//! the list — which is why the count is `members - 1` and why nothing here needs
//! to filter the local player out. Every join, leave, promotion and loot-rule
//! change re-sends it whole.
//!
//! ```text
//! u8   groupType            0 = party, 1 = raid
//! u8   own flags            subgroup | 0x80 if assistant
//! u32  count                everybody but the reader
//!   cstring name  u64 guid  u8 status  u8 flags     x count
//! u64  leader guid
//! -- and only when count > 0:
//! u8   loot method   u64 looter guid   u8 loot threshold
//! ```
//!
//! **The tail is conditional and that is the one thing here a parser gets
//! wrong.** `if (count)` guards the last three fields, so a group of one — which
//! exists, briefly, between the leader accepting and the member arriving — is
//! seventeen bytes with no loot rule at all. Reading it unconditionally walks off
//! the end and drops the whole packet.
//!
//! ## A group of one is a group of none
//!
//! When the second-to-last member leaves, the server sends
//! `SMSG_GROUP_DESTROYED` and the party is over — it does **not** send a
//! `SMSG_GROUP_LIST` with an empty list. So the two ways a roster empties are two
//! different packets and a client that reads only one of them leaves a stale
//! party frame on the screen.
//!
//! ## The status byte is what `UnitIsConnected` and its four siblings answer
//!
//! `MEMBER_STATUS_*` — online, PvP, dead, ghost, free-for-all, AFK, DND — and
//! `0x0000` is offline. It is the only place a party member's state comes from
//! when they are out of the object manager's range, which is most of the time:
//! a party member two zones away is a name, a guid and this byte.
//!
//! ## `SMSG_PARTY_COMMAND_RESULT` is an index into `GlobalStrings.lua`
//!
//! Ten codes ([`PartyResult`]), each of which the client turns into one of the
//! game's own keys — `ERR_BAD_PLAYER_NAME_S` is `"Cannot find '%s'."` and takes
//! the name the packet carries. The same shape as the cast-failure table and the
//! inventory-failure table: a byte that indexes a string the game shipped, not a
//! sentence to compose.
//!
//! ## A raid is the same roster with the first byte set, and five more verbs
//!
//! ```text
//! CMSG_GROUP_RAID_CONVERT  (empty)          Convert to Raid — leader only
//! CMSG_GROUP_CHANGE_SUB_GROUP <name> <u8>   drag somebody into group 3
//! CMSG_GROUP_SWAP_SUB_GROUP   <name> <name> …or onto somebody already in it
//! CMSG_GROUP_ASSISTANT_LEADER <u64> <u8>    promote to assistant, or demote
//! MSG_RAID_READY_CHECK     (empty)          start one…
//! MSG_RAID_READY_CHECK     <u8>             …or answer one
//!   -> MSG_RAID_READY_CHECK (empty)         …broadcast to everybody
//!   -> MSG_RAID_READY_CHECK <u64> <u8>      …and each answer, to the leader
//! ```
//!
//! **`SMSG_GROUP_LIST` does not change shape.** `groupType` becomes 1, each
//! member's flags byte carries their subgroup in the low bits and `0x80` for an
//! assistant, and `own flags` says the same about the reader. Everything a raid
//! frame draws beyond that is derived — see [`rank_of`] — or comes from the
//! same `SMSG_PARTY_MEMBER_STATS` a party member out of range is drawn from.
//!
//! **The reader is still left out of their own copy**, which is what makes the
//! raid roster the *client's* to assemble rather than the server's: the client
//! appends our own guid, our own subgroup byte and an online flag **after** the
//! list, so `raid<N+1>` is us and `GetNumRaidMembers()` is `count + 1`. Nothing
//! in this file does that — it is a rule about a roster and not about a packet,
//! so it lives in `vale_client::game::session::raid`.

use crate::bytes::{Reader, Writer};

/// **What the result is about** — `PartyOperation`, and vmangos only ever sends
/// two of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyOperation {
    Invite,
    Leave,
    /// Anything else the wire carries. Kept rather than refused, on this crate's
    /// "unknown values are data" rule.
    Other(u32),
}

impl PartyOperation {
    pub fn of(word: u32) -> PartyOperation {
        match word {
            0 => PartyOperation::Invite,
            2 => PartyOperation::Leave,
            other => PartyOperation::Other(other),
        }
    }
}

/// **Why a party command failed**, and which `GlobalStrings.lua` key says so.
///
/// vmangos' `PartyResult`. `Ok` is not a failure and has no key; every other
/// variant maps to a name the game shipped, and three of them take the member
/// name the packet carries as their `%s`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyResult {
    Ok,
    BadPlayerName,
    TargetNotInGroup,
    GroupFull,
    AlreadyInGroup,
    NotInGroup,
    NotLeader,
    WrongFaction,
    IgnoringYou,
    /// Codes the client has no string for — 9, and vmangos' own 10, which its
    /// comment says "does not exist client-side".
    Unknown(u32),
}

impl PartyResult {
    pub fn of(word: u32) -> PartyResult {
        match word {
            0 => PartyResult::Ok,
            1 => PartyResult::BadPlayerName,
            2 => PartyResult::TargetNotInGroup,
            3 => PartyResult::GroupFull,
            4 => PartyResult::AlreadyInGroup,
            5 => PartyResult::NotInGroup,
            6 => PartyResult::NotLeader,
            7 => PartyResult::WrongFaction,
            8 => PartyResult::IgnoringYou,
            other => PartyResult::Unknown(other),
        }
    }

    /// The `GlobalStrings.lua` key, or `None` for success and for the codes the
    /// game ships no string for.
    ///
    /// **The `_S` suffix means the key takes the member name**, which is the
    /// convention `GlobalStrings.lua` itself uses and the reason the packet
    /// carries one at all.
    pub fn key(self) -> Option<&'static str> {
        Some(match self {
            PartyResult::Ok => return None,
            PartyResult::BadPlayerName => "ERR_BAD_PLAYER_NAME_S",
            PartyResult::TargetNotInGroup => "ERR_TARGET_NOT_IN_GROUP_S",
            PartyResult::GroupFull => "ERR_GROUP_FULL",
            PartyResult::AlreadyInGroup => "ERR_ALREADY_IN_GROUP_S",
            PartyResult::NotInGroup => "ERR_NOT_IN_GROUP",
            PartyResult::NotLeader => "ERR_NOT_LEADER",
            PartyResult::WrongFaction => "ERR_PLAYER_WRONG_FACTION",
            PartyResult::IgnoringYou => "ERR_IGNORING_YOU_S",
            PartyResult::Unknown(_) => return None,
        })
    }

    /// Whether the key's text has a `%s` in it that the member name fills.
    pub fn takes_name(self) -> bool {
        self.key().is_some_and(|key| key.ends_with("_S"))
    }
}

/// `SMSG_PARTY_COMMAND_RESULT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyCommandResult {
    pub operation: PartyOperation,
    pub member: String,
    pub result: PartyResult,
}

pub fn parse_party_command_result(body: &[u8]) -> Option<PartyCommandResult> {
    let mut r = Reader::new(body);
    if r.remaining() < 4 {
        return None;
    }
    let operation = PartyOperation::of(r.u32());
    let member = r.cstring();
    if r.remaining() < 4 {
        return None;
    }
    Some(PartyCommandResult {
        operation,
        member,
        result: PartyResult::of(r.u32()),
    })
}

/// The status byte a `SMSG_GROUP_LIST` row and a stats packet both carry.
pub mod member_status {
    pub const ONLINE: u8 = 0x01;
    pub const PVP: u8 = 0x02;
    pub const DEAD: u8 = 0x04;
    pub const GHOST: u8 = 0x08;
    pub const PVP_FFA: u8 = 0x10;
    pub const AFK: u8 = 0x40;
    pub const DND: u8 = 0x80;
}

/// **The most members a 1.12 party holds.** Five including the leader, so four
/// `party1..4` tokens — which is why `GetNumPartyMembers` never exceeds 4.
pub const MAX_PARTY_MEMBERS: usize = 4;

/// One row of `SMSG_GROUP_LIST`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupMember {
    pub name: String,
    pub guid: u64,
    /// [`member_status`] — 0 is offline.
    pub status: u8,
    /// Subgroup in the low bits, `0x80` for an assistant. Only a raid uses
    /// either; a party is all subgroup 0.
    pub flags: u8,
}

impl GroupMember {
    pub fn online(&self) -> bool {
        self.status & member_status::ONLINE != 0
    }
    pub fn dead(&self) -> bool {
        self.status & (member_status::DEAD | member_status::GHOST) != 0
    }
    pub fn ghost(&self) -> bool {
        self.status & member_status::GHOST != 0
    }
    pub fn pvp(&self) -> bool {
        self.status & member_status::PVP != 0
    }
    pub fn afk(&self) -> bool {
        self.status & member_status::AFK != 0
    }
    pub fn dnd(&self) -> bool {
        self.status & member_status::DND != 0
    }
    pub fn subgroup(&self) -> u8 {
        self.flags & 0x7f
    }
    pub fn assistant(&self) -> bool {
        self.flags & 0x80 != 0
    }
}

/// How a party decides who gets what — `GetLootMethod`'s own five words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LootMethod {
    #[default]
    FreeForAll,
    RoundRobin,
    MasterLoot,
    GroupLoot,
    NeedBeforeGreed,
}

impl LootMethod {
    pub fn of(byte: u8) -> LootMethod {
        match byte {
            1 => LootMethod::RoundRobin,
            2 => LootMethod::MasterLoot,
            3 => LootMethod::GroupLoot,
            4 => LootMethod::NeedBeforeGreed,
            _ => LootMethod::FreeForAll,
        }
    }

    /// **The word `GetLootMethod()` answers**, which is what
    /// `PartyMemberFrame.lua` compares against — a string, not a number.
    pub fn word(self) -> &'static str {
        match self {
            LootMethod::FreeForAll => "freeforall",
            LootMethod::RoundRobin => "roundrobin",
            LootMethod::MasterLoot => "master",
            LootMethod::GroupLoot => "group",
            LootMethod::NeedBeforeGreed => "needbeforegreed",
        }
    }
}

/// `SMSG_GROUP_LIST` — the whole party, as this reader sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupList {
    /// 0 for a party, 1 for a raid.
    pub group_type: u8,
    /// Our own subgroup and assistant bit.
    pub own_flags: u8,
    /// **Everybody but us** — the server leaves the reader out of their own
    /// copy, which is what makes this list `party1..N` directly.
    pub members: Vec<GroupMember>,
    pub leader: u64,
    /// `None` for a group of one, where the server sends no loot tail at all.
    pub loot: Option<(LootMethod, u64, u8)>,
}

impl GroupList {
    pub fn is_raid(&self) -> bool {
        self.group_type != 0
    }
}

pub fn parse_group_list(body: &[u8]) -> Option<GroupList> {
    let mut r = Reader::new(body);
    if r.remaining() < 6 {
        return None;
    }
    let group_type = r.u8();
    let own_flags = r.u8();
    let count = r.u32() as usize;
    // A bound on nonsense rather than on the game: a raid is 40 and a corrupt
    // length would otherwise reserve a gigabyte.
    if count > 64 {
        return None;
    }
    let mut members = Vec::with_capacity(count);
    for _ in 0..count {
        let name = r.cstring();
        if r.remaining() < 10 {
            return None;
        }
        members.push(GroupMember {
            name,
            guid: r.u64(),
            status: r.u8(),
            flags: r.u8(),
        });
    }
    if r.remaining() < 8 {
        return None;
    }
    let leader = r.u64();
    // **Only when there is somebody else in the list** — see the module comment.
    let loot = (count > 0 && r.remaining() >= 10).then(|| {
        let method = LootMethod::of(r.u8());
        let looter = r.u64();
        (method, looter, r.u8())
    });
    Some(GroupList {
        group_type,
        own_flags,
        members,
        leader,
        loot,
    })
}

/// `SMSG_GROUP_INVITE` / `SMSG_GROUP_DECLINE` / `SMSG_GROUP_SET_LEADER` — three
/// packets whose whole body is one name.
pub fn parse_name(body: &[u8]) -> Option<String> {
    let name = Reader::new(body).cstring();
    (!name.is_empty()).then_some(name)
}

/// **What a member's stats packet says**, in the shapes the party frame reads
/// them in.
///
/// **Every field is an `Option` because the packet is a *diff*.**
/// `SMSG_PARTY_MEMBER_STATS` carries only what the mask names, and the mask is
/// whatever moved since the last one — so a member who took a step sends a body
/// with `POSITION` and nothing else. A reader that *replaces* its last reading
/// with one of those loses the health, the mana, the level and the zone, which
/// is a party frame that fills in on join and empties itself a few seconds
/// later. See [`PartyMemberStats::fold`], which is the only correct way to
/// apply one.
///
/// `SMSG_PARTY_MEMBER_STATS_FULL` is the same body with every bit set and is
/// what `CMSG_REQUEST_PARTY_MEMBER_STATS` is answered with; nothing here tells
/// the two apart, because the mask already does.
///
/// The two **aura** masks are parsed past rather than into: a party member's
/// buffs are read off the object manager, which is the only time the frame draws
/// them. The **pet block is kept** — see [`PartyPetStats`], which is what
/// `partypet<n>` answers off for a member who is out of range.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartyMemberStats {
    pub guid: u64,
    pub status: Option<u8>,
    pub health: Option<u16>,
    pub max_health: Option<u16>,
    pub power_type: Option<u8>,
    pub power: Option<u16>,
    pub max_power: Option<u16>,
    pub level: Option<u16>,
    pub zone: Option<u16>,
    pub position: Option<(i16, i16)>,
    /// …and the member's pet, which is a second unit over the same opcode.
    pub pet: PartyPetStats,
}

/// **A party member's pet, as the group packet states it** — the ten pet bits of
/// `GROUP_UPDATE_FULL`, which are half of what that mask is.
///
/// This block used to be walked past on the grounds that "this client has no
/// pet", and that is the wrong half of the reason it exists. The pet the packet
/// describes is not ours and not one the object manager has: vmangos sends
/// `SMSG_PARTY_MEMBER_STATS` **only for a member out of range**
/// (`Player::SendUpdateToOutOfRangeGroupMembers`), so this is precisely the
/// reading for a pet with no entity — the one case a `partypet<n>` frame cannot
/// draw any other way.
///
/// The client keeps it too, and in the same shape. Its `partypet<n>` token
/// looks the *owner* up in the object manager first and reads their live
/// `UNIT_FIELD_CHARM`/`SUMMON`; only when the owner is absent does it fall back
/// to a cached guid in that member's party row, gated on the row's own valid
/// bit. So the order is
/// **world first, this second**, and this struct is that cache.
///
/// Every field is an `Option` on the same terms as [`PartyMemberStats`]': the
/// packet is a diff and `None` means *unchanged*, never *gone*. A member whose
/// pet was dismissed is announced by `GROUP_UPDATE_FLAG_PET_GUID` carrying a
/// **zero** guid, which is why [`Self::guid`] is `Option<u64>` holding
/// `Some(0)` rather than `None` for that case — see [`Self::fold`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartyPetStats {
    /// `GROUP_UPDATE_FLAG_PET_GUID`. **`Some(0)` is "the pet is gone"** and
    /// `None` is "the packet did not say".
    pub guid: Option<u64>,
    /// `GROUP_UPDATE_FLAG_PET_NAME` — what the pet frame's name plate reads.
    /// Empty for a member with no pet, which the builder writes as a bare NUL.
    pub name: Option<String>,
    pub display_id: Option<u16>,
    pub health: Option<u16>,
    pub max_health: Option<u16>,
    pub power_type: Option<u8>,
    pub power: Option<u16>,
    pub max_power: Option<u16>,
}

impl PartyPetStats {
    /// Fold a newer reading in, field by field — the same rule as
    /// [`PartyMemberStats::fold`] and for the same reason.
    pub fn fold(&mut self, newer: &PartyPetStats) {
        macro_rules! take {
            ($($field:ident),* $(,)?) => {
                $( if newer.$field.is_some() { self.$field = newer.$field.clone(); } )*
            };
        }
        take!(guid, name, display_id, health, max_health, power_type, power, max_power);
    }

    /// **Whether this says there is a pet at all** — the roster half of
    /// `UnitExists("partypet<n>")`.
    ///
    /// A guid of zero is a real answer and means no pet, which is the shape
    /// the client's row test has too: it checks the row's valid bit and then
    /// compares the guid, so a zeroed one matches nothing.
    pub fn exists(&self) -> bool {
        self.guid.is_some_and(|guid| guid != 0)
    }
}

impl PartyMemberStats {
    /// **Fold a newer reading into this one** — a field the packet carried wins,
    /// a field it did not is left alone.
    ///
    /// This is what makes the `Option`s mean "unchanged" rather than "gone", and
    /// it is the whole of why they are `Option`s. The guid comes from the newer
    /// one, which is the same guid by construction.
    pub fn fold(&mut self, newer: &PartyMemberStats) {
        self.guid = newer.guid;
        macro_rules! take {
            ($($field:ident),* $(,)?) => {
                $( if newer.$field.is_some() { self.$field = newer.$field; } )*
            };
        }
        take!(
            status,
            health,
            max_health,
            power_type,
            power,
            max_power,
            level,
            zone,
            position,
        );
        self.pet.fold(&newer.pet);
    }
}

/// The mask bits, in the order the body writes them.
mod update_flag {
    pub const STATUS: u32 = 0x0000_0001;
    pub const CUR_HP: u32 = 0x0000_0002;
    pub const MAX_HP: u32 = 0x0000_0004;
    pub const POWER_TYPE: u32 = 0x0000_0008;
    pub const CUR_POWER: u32 = 0x0000_0010;
    pub const MAX_POWER: u32 = 0x0000_0020;
    pub const LEVEL: u32 = 0x0000_0040;
    pub const ZONE: u32 = 0x0000_0080;
    pub const POSITION: u32 = 0x0000_0100;
    pub const AURAS: u32 = 0x0000_0200;
    pub const AURAS_NEGATIVE: u32 = 0x0000_0400;
    pub const PET_GUID: u32 = 0x0000_0800;
    pub const PET_NAME: u32 = 0x0000_1000;
    pub const PET_MODEL_ID: u32 = 0x0000_2000;
    pub const PET_CUR_HP: u32 = 0x0000_4000;
    pub const PET_MAX_HP: u32 = 0x0000_8000;
    pub const PET_POWER_TYPE: u32 = 0x0001_0000;
    pub const PET_CUR_POWER: u32 = 0x0002_0000;
    pub const PET_MAX_POWER: u32 = 0x0004_0000;
    pub const PET_AURAS: u32 = 0x0008_0000;
    pub const PET_AURAS_NEGATIVE: u32 = 0x0010_0000;
}

/// `SMSG_PARTY_MEMBER_STATS` and `SMSG_PARTY_MEMBER_STATS_FULL` — the same body.
///
/// **The guid is packed**, which is the one thing about this packet that is not
/// obvious from the mask: `BuildPartyMemberStatsPacket` writes `GetPackGUID()`
/// for every build above 1.8.4.
///
/// The variable-length blocks — the two aura masks, the pet's name — have to be
/// *walked* even though nothing here keeps them, because every field after one
/// of them would otherwise be read from the wrong offset. A body that runs out
/// mid-walk answers what it has rather than nothing: the fields are written in
/// mask order, so a truncated tail costs only the fields after the cut.
pub fn parse_party_member_stats(body: &[u8]) -> Option<PartyMemberStats> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    if r.remaining() < 4 {
        return None;
    }
    let mask = r.u32();
    let mut out = PartyMemberStats {
        guid,
        ..Default::default()
    };
    macro_rules! need {
        ($n:expr) => {
            if r.remaining() < $n {
                return Some(out);
            }
        };
    }
    if mask & update_flag::STATUS != 0 {
        need!(1);
        out.status = Some(r.u8());
    }
    if mask & update_flag::CUR_HP != 0 {
        need!(2);
        out.health = Some(r.u16());
    }
    if mask & update_flag::MAX_HP != 0 {
        need!(2);
        out.max_health = Some(r.u16());
    }
    if mask & update_flag::POWER_TYPE != 0 {
        need!(1);
        out.power_type = Some(r.u8());
    }
    if mask & update_flag::CUR_POWER != 0 {
        need!(2);
        out.power = Some(r.u16());
    }
    if mask & update_flag::MAX_POWER != 0 {
        need!(2);
        out.max_power = Some(r.u16());
    }
    if mask & update_flag::LEVEL != 0 {
        need!(2);
        out.level = Some(r.u16());
    }
    if mask & update_flag::ZONE != 0 {
        need!(2);
        out.zone = Some(r.u16());
    }
    if mask & update_flag::POSITION != 0 {
        need!(4);
        out.position = Some((r.u16() as i16, r.u16() as i16));
    }
    // The rest is walked and discarded — see the doc comment.
    if mask & update_flag::AURAS != 0 {
        need!(4);
        let auras = r.u32();
        for _ in 0..auras.count_ones() {
            need!(2);
            r.u16();
        }
    }
    if mask & update_flag::AURAS_NEGATIVE != 0 {
        need!(2);
        let auras = r.u16();
        for _ in 0..auras.count_ones() {
            need!(2);
            r.u16();
        }
    }
    // **The pet block, in mask order** — which puts the name between the guid
    // and the model id. A member with no pet still writes every field the mask
    // names: a zero guid, a bare NUL for the name and zeroes for the numbers,
    // which is what makes `PET_GUID` carrying zero the statement that the pet
    // is gone rather than an absent field.
    if mask & update_flag::PET_GUID != 0 {
        need!(8);
        out.pet.guid = Some(r.u64());
    }
    if mask & update_flag::PET_NAME != 0 {
        need!(1);
        out.pet.name = Some(r.cstring());
    }
    if mask & update_flag::PET_MODEL_ID != 0 {
        need!(2);
        out.pet.display_id = Some(r.u16());
    }
    if mask & update_flag::PET_CUR_HP != 0 {
        need!(2);
        out.pet.health = Some(r.u16());
    }
    if mask & update_flag::PET_MAX_HP != 0 {
        need!(2);
        out.pet.max_health = Some(r.u16());
    }
    if mask & update_flag::PET_POWER_TYPE != 0 {
        need!(1);
        out.pet.power_type = Some(r.u8());
    }
    if mask & update_flag::PET_CUR_POWER != 0 {
        need!(2);
        out.pet.power = Some(r.u16());
    }
    if mask & update_flag::PET_MAX_POWER != 0 {
        need!(2);
        out.pet.max_power = Some(r.u16());
    }
    if mask & update_flag::PET_AURAS != 0 {
        need!(4);
        let auras = r.u32();
        for _ in 0..auras.count_ones() {
            need!(2);
            r.u16();
        }
    }
    if mask & update_flag::PET_AURAS_NEGATIVE != 0 {
        need!(2);
        let auras = r.u16();
        for _ in 0..auras.count_ones() {
            need!(2);
            r.u16();
        }
    }
    Some(out)
}

/// **The most members a raid holds**, and the two numbers the grid is drawn
/// from — `MAX_RAID_MEMBERS`, `NUM_RAID_GROUPS` and `MEMBERS_PER_RAID_GROUP`,
/// which `RaidFrame.lua` states in its own first three lines.
pub const MAX_RAID_MEMBERS: usize = 40;

/// …and how many subgroups those forty are dealt into. vmangos calls it
/// `MAX_RAID_SUBGROUPS` and refuses a `CMSG_GROUP_CHANGE_SUB_GROUP` naming
/// anything higher.
pub const MAX_RAID_SUBGROUPS: usize = 8;

/// …and how many fit in one of them, which is what makes a full subgroup refuse
/// a drop.
pub const MEMBERS_PER_RAID_GROUP: usize = 5;

/// **What a raid member may be**, as `GetRaidRosterInfo`'s second return.
///
/// A number rather than a name because that is what the interface compares
/// against: `RaidGroupFrame_Update` tests `rank == 2` for the leader token and
/// `rank == 1` for the assistant's, and `UnitPopup.lua` hides Promote unless
/// `rank == 0`.
///
/// It is **derived, not sent**. `SMSG_GROUP_LIST` carries the leader's guid once
/// and an assistant bit in each member's flags byte; the client crosses the two.
/// See [`rank_of`].
pub mod raid_rank {
    /// An ordinary member.
    pub const MEMBER: u32 = 0;
    /// `0x80` of the row's flags byte.
    pub const ASSISTANT: u32 = 1;
    /// The guid `SMSG_GROUP_LIST` names as leader — **which outranks the
    /// assistant bit**: vmangos sets both on a leader who was promoted from
    /// assistant, and the interface's `rank ~= 1` test would then hide Demote
    /// on the leader's own row.
    pub const LEADER: u32 = 2;
}

/// **The rank a guid holds**, from the two things the wire states separately.
///
/// See [`raid_rank`] for why the leader test comes first.
pub fn rank_of(guid: u64, leader: u64, assistant: bool) -> u32 {
    if guid != 0 && guid == leader {
        raid_rank::LEADER
    } else if assistant {
        raid_rank::ASSISTANT
    } else {
        raid_rank::MEMBER
    }
}

/// **What `MSG_RAID_READY_CHECK` is**, which depends on whether it has a body.
///
/// One opcode, three directions and no discriminator but the length —
/// `RaidReadyCheckFromClient::ReadFromWorldPacket` is literally
/// `if (!recv_data.empty())`. A client sends the empty form to *start* a check
/// and the one-byte form to answer one; the server broadcasts the empty form to
/// everybody and forwards each answer, with the answerer's guid in front of it,
/// to the leader alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyCheck {
    /// The leader started one. **This is the whole packet** — who started it is
    /// not in the body, which is why `ShowReadyCheck` walks the roster for the
    /// row whose rank is 2 rather than being told.
    Started,
    /// …and somebody answered, which only the leader is sent.
    Answered { guid: u64, ready: bool },
}

/// `MSG_RAID_READY_CHECK` in the incoming direction.
///
/// **A zero-length body is not a parse failure here**, which is the one thing to
/// get right: it is the commonest form of the packet.
pub fn parse_ready_check(body: &[u8]) -> Option<ReadyCheck> {
    if body.is_empty() {
        return Some(ReadyCheck::Started);
    }
    let mut r = Reader::new(body);
    if r.remaining() < 9 {
        return None;
    }
    let guid = r.u64();
    Some(ReadyCheck::Answered {
        guid,
        ready: r.u8() != 0,
    })
}

// --- the bodies this client sends ---

/// `CMSG_GROUP_CHANGE_SUB_GROUP` — move one member into one subgroup.
///
/// **By name, where every other raid verb with a target is by guid.** vmangos
/// looks the name up in the world and then in the character table, so a member
/// who is offline can still be moved; and the subgroup byte is **zero-based**
/// where `SetRaidSubgroup`'s argument is one-based, which is the client's
/// decrement and the `groupNr >= MAX_RAID_SUBGROUPS` refusal on the other side.
pub fn change_sub_group_body(name: &str, subgroup: u8) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(name).u8(subgroup);
    w.buf
}

/// `CMSG_GROUP_SWAP_SUB_GROUP` — exchange two members' subgroups, by name.
///
/// A separate opcode rather than two of the above, because two moves cannot be
/// made atomically when both subgroups are full: `HasFreeSlotSubGroup` would
/// refuse the first one.
pub fn swap_sub_group_body(name: &str, swap_with: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(name).cstring(swap_with);
    w.buf
}

/// `CMSG_GROUP_ASSISTANT_LEADER` — promote to assistant, or demote from it.
///
/// **A guid in 1.12 and a name before it.** vmangos switches on
/// `CLIENT_BUILD_1_11_2`, so a body written the older way is read as a string
/// and silently matches nobody.
pub fn assistant_leader_body(guid: u64, assistant: bool) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid).u8(u8::from(assistant));
    w.buf
}

/// `MSG_RAID_READY_CHECK` — **answer** one.
///
/// The *start* has no body at all and goes out as [`EMPTY`], which is what makes
/// the two directions one opcode. See [`ReadyCheck`].
pub fn ready_check_answer_body(ready: bool) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(u8::from(ready));
    w.buf
}



/// `CMSG_GROUP_INVITE` — a name and nothing else.
pub fn invite_body(name: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(name);
    w.buf
}

/// `CMSG_GROUP_UNINVITE` — likewise, and the by-name half of a kick.
pub fn uninvite_body(name: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(name);
    w.buf
}

/// `CMSG_GROUP_UNINVITE_GUID` and `CMSG_GROUP_SET_LEADER` — eight bytes of guid.
pub fn guid_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// `CMSG_REQUEST_PARTY_MEMBER_STATS` — the same shape, and the one poll this
/// subject has.
pub fn request_stats_body(guid: u64) -> Vec<u8> {
    guid_body(guid)
}

/// `CMSG_GROUP_ACCEPT`, `CMSG_GROUP_DECLINE` and `CMSG_GROUP_DISBAND` are all
/// **empty**, and the last is the one worth a name: leaving a party and
/// disbanding one are the same opcode, and which happens is the server's
/// decision off whether you are the leader.
pub const EMPTY: &[u8] = &[];

#[cfg(test)]
mod tests {
    use super::*;

    fn list_body(members: &[(&str, u64, u8, u8)], leader: u64, loot: bool) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(0).u8(0).u32(members.len() as u32);
        for (name, guid, status, flags) in members {
            w.cstring(name).u64(*guid).u8(*status).u8(*flags);
        }
        w.u64(leader);
        if loot {
            w.u8(2).u64(77).u8(3);
        }
        w.buf
    }

    /// **The roster, and the tail that is only there when somebody else is.**
    #[test]
    fn a_group_list_is_everybody_but_the_reader() {
        let body = list_body(&[("Bram", 42, 0x03, 0), ("Merrick", 43, 0x01, 0)], 42, true);
        let list = parse_group_list(&body).expect("it parses");
        assert!(!list.is_raid());
        assert_eq!(list.members.len(), 2);
        assert_eq!(list.members[0].name, "Bram");
        assert_eq!(list.members[0].guid, 42);
        assert!(list.members[0].online() && list.members[0].pvp());
        assert!(!list.members[0].dead());
        assert_eq!(list.leader, 42);
        assert_eq!(list.loot, Some((LootMethod::MasterLoot, 77, 3)));
    }

    /// **A group of one has no loot tail at all**, and reading it
    /// unconditionally is what drops the packet — see the module comment.
    #[test]
    fn a_group_of_one_carries_no_loot_rule() {
        let body = list_body(&[], 42, false);
        assert_eq!(body.len(), 1 + 1 + 4 + 8);
        let list = parse_group_list(&body).expect("it parses");
        assert!(list.members.is_empty());
        assert_eq!(list.leader, 42);
        assert_eq!(list.loot, None);
        // …and a body that stops mid-roster is refused rather than half-read.
        assert_eq!(parse_group_list(&body[..8]), None);
        assert_eq!(parse_group_list(&[]), None);
    }

    /// **The status byte is what five of the unit reads answer**, for a member
    /// nothing else in this client knows about.
    #[test]
    fn the_status_byte_answers_the_five_unit_reads() {
        let ghost = GroupMember {
            status: member_status::ONLINE | member_status::GHOST | member_status::DEAD,
            ..Default::default()
        };
        assert!(ghost.online() && ghost.dead() && ghost.ghost());
        let away = GroupMember {
            status: member_status::ONLINE | member_status::AFK,
            ..Default::default()
        };
        assert!(away.afk() && !away.dnd() && !away.dead());
        let offline = GroupMember::default();
        assert!(!offline.online());
    }

    /// **A failure is a `GlobalStrings.lua` key**, and the `_S` suffix is what
    /// says the name goes in it.
    #[test]
    fn a_party_failure_is_a_key_and_sometimes_a_name() {
        let mut w = Writer::new();
        w.u32(0).cstring("Bram").u32(1);
        let result = parse_party_command_result(&w.buf).expect("it parses");
        assert_eq!(result.operation, PartyOperation::Invite);
        assert_eq!(result.member, "Bram");
        assert_eq!(result.result, PartyResult::BadPlayerName);
        assert_eq!(result.result.key(), Some("ERR_BAD_PLAYER_NAME_S"));
        assert!(result.result.takes_name());
        // Success has no key at all, and neither does a code the game ships
        // nothing for.
        assert_eq!(PartyResult::of(0).key(), None);
        assert_eq!(PartyResult::of(10).key(), None);
        assert!(!PartyResult::GroupFull.takes_name());
    }

    /// **The stats body is mask-ordered and its guid is packed**, and the
    /// variable blocks have to be walked or every field after one is read from
    /// the wrong offset.
    #[test]
    fn a_stats_body_is_read_in_mask_order_past_the_blocks() {
        let mut w = Writer::new();
        w.packed_guid(0x1234);
        // status | cur hp | max hp | level | auras | pet name | pet cur hp
        let mask = update_flag::STATUS
            | update_flag::CUR_HP
            | update_flag::MAX_HP
            | update_flag::LEVEL
            | update_flag::AURAS
            | update_flag::PET_NAME
            | update_flag::PET_CUR_HP;
        w.u32(mask);
        w.u8(member_status::ONLINE);
        w.u16(800).u16(1200).u16(31);
        // two auras set, so two spell ids follow the mask
        w.u32(0b101).u16(1234).u16(5678);
        w.cstring("Bear");
        w.u16(4000);
        let stats = parse_party_member_stats(&w.buf).expect("it parses");
        assert_eq!(stats.guid, 0x1234);
        assert_eq!(stats.status, Some(member_status::ONLINE));
        assert_eq!((stats.health, stats.max_health), (Some(800), Some(1200)));
        assert_eq!(stats.level, Some(31));
        // A mask bit that was never set stays absent.
        assert_eq!(stats.zone, None);
        // **And the pet block lands where the mask put it**, which is the half
        // this parser used to walk past: the name sits between the guid and the
        // model id, so a reader that skipped it by width would take "Bear"'s
        // five bytes as two fields and read the health from the wrong offset.
        assert_eq!(stats.pet.name.as_deref(), Some("Bear"));
        assert_eq!(stats.pet.health, Some(4000));
        assert_eq!(stats.pet.guid, None, "the mask did not ask for one");
        assert!(!stats.pet.exists());

        // …and a body cut short answers what it had rather than nothing, which
        // is the parsers-tolerate-damage rule one packet up.
        let short = parse_party_member_stats(&[0x01, 0x34, 0x06, 0x00, 0x00, 0x00, 0x01])
            .expect("the head parses");
        assert_eq!(short.health, None, "the mask asked, the body did not carry");
    }

    /// The loot method is a **word** to the interface, not a number.
    #[test]
    fn the_loot_method_is_one_of_five_words() {
        assert_eq!(LootMethod::of(0).word(), "freeforall");
        assert_eq!(LootMethod::of(2).word(), "master");
        assert_eq!(LootMethod::of(4).word(), "needbeforegreed");
        // An unknown byte is free-for-all, which is the rule with no rule in it.
        assert_eq!(LootMethod::of(200), LootMethod::FreeForAll);
    }

    /// **A partial update keeps everything it did not mention**, which is the
    /// whole contract of the mask — and getting it wrong is a party frame that
    /// fills in on join and blanks itself the moment the member takes a step.
    #[test]
    fn a_later_reading_only_overwrites_what_it_carried() {
        let mut held = PartyMemberStats {
            guid: 7,
            status: Some(member_status::ONLINE),
            health: Some(800),
            max_health: Some(1200),
            power_type: Some(0),
            power: Some(300),
            max_power: Some(900),
            level: Some(31),
            zone: Some(12),
            position: Some((100, -200)),
            pet: PartyPetStats::default(),
        };
        // A step: `GROUP_UPDATE_FLAG_POSITION` and nothing else.
        held.fold(&PartyMemberStats {
            guid: 7,
            position: Some((105, -204)),
            ..Default::default()
        });
        assert_eq!(held.position, Some((105, -204)), "the new field lands");
        assert_eq!(held.health, Some(800), "…and the old ones survive it");
        assert_eq!(held.level, Some(31));
        assert_eq!(held.zone, Some(12));
        assert_eq!(held.max_power, Some(900));
        // …and a field that arrives for the first time is taken.
        let mut empty = PartyMemberStats::default();
        empty.fold(&held);
        assert_eq!(empty, held);
    }

    /// The three bodies this client sends, byte for byte.
    #[test]
    fn the_outbound_bodies_are_a_name_or_a_guid() {
        assert_eq!(invite_body("Bram"), b"Bram\0");
        assert_eq!(guid_body(0x42), 0x42u64.to_le_bytes());
        assert!(EMPTY.is_empty());
    }

    /// …and the four the raid adds, which differ from the party's in exactly the
    /// two ways a reader gets wrong: the subgroup byte is **zero-based** and the
    /// assistant verb is a **guid** rather than a name.
    #[test]
    fn the_raid_bodies_are_the_shapes_vmangos_reads() {
        assert_eq!(change_sub_group_body("Bram", 2), b"Bram\0\x02");
        assert_eq!(
            swap_sub_group_body("Bram", "Merrick"),
            b"Bram\0Merrick\0"
        );
        let mut want = 0x1234_5678_9abc_def0u64.to_le_bytes().to_vec();
        want.push(1);
        assert_eq!(assistant_leader_body(0x1234_5678_9abc_def0, true), want);
        assert_eq!(ready_check_answer_body(false), vec![0]);
    }

    /// **A ready check with no body is the ordinary one**, and a reader that
    /// treats an empty packet as damaged never shows the box.
    #[test]
    fn an_empty_ready_check_is_the_start_of_one() {
        assert_eq!(parse_ready_check(&[]), Some(ReadyCheck::Started));
        let mut body = 7u64.to_le_bytes().to_vec();
        body.push(1);
        assert_eq!(
            parse_ready_check(&body),
            Some(ReadyCheck::Answered { guid: 7, ready: true })
        );
        // …and a body that is neither is refused rather than read as a zero guid.
        assert_eq!(parse_ready_check(&[1, 2, 3]), None);
    }

    /// **The leader test comes first**, because vmangos leaves the assistant bit
    /// set on a leader who was promoted from one — and `rank ~= 1` is what hides
    /// Demote in `UnitPopup.lua`.
    #[test]
    fn a_leader_who_is_also_an_assistant_ranks_as_the_leader() {
        assert_eq!(rank_of(7, 7, false), raid_rank::LEADER);
        assert_eq!(rank_of(7, 7, true), raid_rank::LEADER);
        assert_eq!(rank_of(8, 7, true), raid_rank::ASSISTANT);
        assert_eq!(rank_of(8, 7, false), raid_rank::MEMBER);
        // A group with no leader guid at all ranks nobody, including guid 0.
        assert_eq!(rank_of(0, 0, false), raid_rank::MEMBER);
    }

    /// **The subgroup and the assistant bit share one byte**, and reading the
    /// whole byte as a subgroup puts every assistant in group 129.
    #[test]
    fn a_members_flags_byte_is_a_subgroup_and_a_bit() {
        let member = GroupMember {
            name: "Bram".to_string(),
            guid: 7,
            status: member_status::ONLINE,
            flags: 0x80 | 3,
        };
        assert_eq!(member.subgroup(), 3);
        assert!(member.assistant());
    }

    /// **A raid roster is the party's with the first byte set**, and both halves
    /// of every flags byte survive the parse.
    #[test]
    fn a_raid_roster_parses_as_one() {
        let mut w = Writer::new();
        w.u8(1).u8(0x80 | 2).u32(2);
        w.cstring("Bram").u64(7).u8(member_status::ONLINE).u8(0);
        w.cstring("Merrick")
            .u64(8)
            .u8(member_status::ONLINE)
            .u8(0x80 | 1);
        w.u64(7);
        w.u8(3).u64(0).u8(2);
        let list = parse_group_list(&w.buf).expect("a raid roster parses");
        assert!(list.is_raid());
        assert_eq!(list.own_flags & 0x7f, 2, "our own subgroup");
        assert!(list.own_flags & 0x80 != 0, "…and we are an assistant");
        assert_eq!(list.members[1].subgroup(), 1);
        assert!(list.members[1].assistant());
        assert_eq!(list.leader, 7);
    }
}
