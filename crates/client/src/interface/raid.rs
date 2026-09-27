//! **The raid** — the same roster one byte different, and what the client has
//! to work out for itself.
//!
//! ```text
//! what the server says   SMSG_GROUP_LIST, with groupType 1 and a subgroup
//!                        nibble in every flags byte
//! what a press does      CMSG_GROUP_RAID_CONVERT / CHANGE_SUB_GROUP /
//!                        SWAP_SUB_GROUP / ASSISTANT_LEADER / READY_CHECK
//! what a raid *is*       everything below: the order, the rank, the count
//! ```
//!
//! ## The server does not send you your own row, so the raid roster is ours
//!
//! `Group::SendUpdate` gives every member a copy of the list with themselves
//! left out — see [`super::party`], where that fact is load-bearing for the
//! party too. A party can live with it, because `party1`..`party4` really are
//! the *other* four. A raid cannot: `raid1`..`raid40` include you, and
//! `GetNumRaidMembers()` counts you, so a client that answered
//! `members.len()` would report a raid of thirty-nine and leave one button
//! blank.
//!
//! The reference does the join: on a roster whose first byte is
//! 1, it appends **our own guid, our own subgroup byte and an online flag after
//! the last member** and hands the whole thing to the rebuild. So the order is
//! the server's, then us, and it is stable — which matters, because
//! `raid<n>` is a slot and the drag-and-drop is written against it.
//!
//! [`RaidSlot`] is that join and [`Party::raid_slot`] is the lookup.
//!
//! ## A rank is derived, not sent
//!
//! `SMSG_GROUP_LIST` states the leader's guid once and an assistant bit in each
//! member's flags. `GetRaidRosterInfo`'s second return is a *number* — 0, 1 or
//! 2 — and the interface tests it three different ways
//! (`UnitPopup.lua` hides Promote unless it is 0, Demote unless it is 1, and
//! Remove when it is 2). [`vale_protocol::play::group::rank_of`] is the
//! crossing, and the leader test comes first for a reason stated there.
//!
//! ## What a raid member is, when they are not in the world
//!
//! The same two-places-to-look [`super::party`]'s module comment is about, plus
//! one: a raid button draws a **class**, and no group packet carries one. The
//! reference reads it out of its own name cache at the top of
//! `GetRaidRosterInfo`, which is filled by `CMSG_NAME_QUERY` — so
//! this client asks about every group guid rather than only the ones with an
//! entity (`ObjectManager::group_guids`) and [`learn_member_classes`] copies
//! the answers onto the roster.
//!
//! **The player's own row needs none of it.** We are always in the world, so
//! `raid<N+1>` resolves to an entity and every read answers off it.

use bevy::prelude::*;

use vale_protocol::play::group::{MAX_RAID_SUBGROUPS, raid_rank, rank_of};
use vale_protocol::socket::session::PartyVerb;

use super::party::Party;
use crate::world::session::Session;

/// **Which row `raid<n>` names**, which is two different kinds of thing.
///
/// See the module comment: the server's list, then us. A slot rather than a
/// person — somebody leaving shifts everybody below them up one — which is the
/// same warning [`crate::interface::api::Units::guid`] carries about `party<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaidSlot {
    /// A member the server named, by index into [`Party::members`].
    Member(usize),
    /// …and us, whom it never does. **Always last.**
    Player,
}

impl Party {
    /// `GetNumRaidMembers()` — **everybody including us**, and **0 when this is
    /// not a raid**.
    ///
    /// The zero is not a stub and it is not caution: `PartyMemberFrame_UpdateMember`
    /// hides every party frame when this is non-zero, and `RaidFrame_Update`
    /// shows the Convert to Raid button when it is zero. Answering anything but
    /// zero for a party would blank the party frames and hide the button that
    /// makes a raid.
    pub fn raid_count(&self) -> usize {
        if !self.raid {
            return 0;
        }
        self.members.len() + 1
    }

    /// The slot `raid<index>` names, `index` from 1.
    pub fn raid_slot(&self, index: usize) -> Option<RaidSlot> {
        let count = self.raid_count();
        if index == 0 || index > count {
            return None;
        }
        if index == count {
            return Some(RaidSlot::Player);
        }
        Some(RaidSlot::Member(index - 1))
    }

    /// …and the inverse: **which `raid<n>` a guid is**, one-based.
    ///
    /// `own` is the local player's guid, which is the only thing that can put
    /// the last row into the answer — see the module comment.
    pub fn raid_index_of(&self, guid: u64, own: Option<u64>) -> Option<usize> {
        if !self.raid || guid == 0 {
            return None;
        }
        if let Some(index) = self.members.iter().position(|member| member.guid == guid) {
            return Some(index + 1);
        }
        (own == Some(guid)).then(|| self.raid_count())
    }

    /// **What subgroup a slot is in**, one-based as `GetRaidRosterInfo` returns
    /// it and as `RaidGroup<n>` is named.
    ///
    /// The low seven bits of a flags byte — ours out of [`Party::own_flags`],
    /// which is the only place the server states it for the reader. Clamped
    /// into 1..=8 rather than trusted: `getglobal("RaidGroup"..subgroup)` is a
    /// nil index in Lua for anything else, and that takes the whole rebuild
    /// down.
    pub fn raid_subgroup(&self, slot: RaidSlot) -> usize {
        let flags = match slot {
            RaidSlot::Member(index) => self.members.get(index).map_or(0, |member| member.flags),
            RaidSlot::Player => self.own_flags,
        };
        (usize::from(flags & 0x7f)).min(MAX_RAID_SUBGROUPS - 1) + 1
    }

    /// **What rank a slot holds** — 0, 1 or 2. See
    /// [`vale_protocol::play::group::raid_rank`].
    pub fn raid_rank(&self, slot: RaidSlot, own: Option<u64>) -> u32 {
        match slot {
            RaidSlot::Member(index) => match self.members.get(index) {
                Some(member) => rank_of(member.guid, self.leader, member.flags & 0x80 != 0),
                None => raid_rank::MEMBER,
            },
            RaidSlot::Player => match own {
                Some(guid) => rank_of(guid, self.leader, self.own_flags & 0x80 != 0),
                None => raid_rank::MEMBER,
            },
        }
    }

    /// `IsRaidOfficer()` — **assistant *or* leader**, which is the test
    /// the reference makes on our own row: `rank > 0`.
    ///
    /// Not "we hold the assistant bit". vmangos gates the four verbs a raid
    /// panel sends on `IsLeader || IsAssistant`, and a leader who never held the
    /// bit would otherwise find Add Member greyed out on their own raid.
    pub fn is_raid_officer(&self, own: Option<u64>) -> bool {
        self.raid
            && self
                .raid_slot(self.raid_count())
                .is_some_and(|slot| self.raid_rank(slot, own) > raid_rank::MEMBER)
    }

    /// **The name behind a raid index**, which is what three of the five verbs
    /// put on the wire.
    ///
    /// `own_name` is the local player's, for the one row the roster does not
    /// carry. `None` for an index nobody occupies — and the caller must send
    /// nothing rather than an empty name, which the server answers with a
    /// lookup failure over an empty `%s`.
    pub fn raid_name(&self, index: usize, own_name: Option<&str>) -> Option<String> {
        match self.raid_slot(index)? {
            RaidSlot::Member(at) => Some(self.members.get(at)?.name.clone()),
            RaidSlot::Player => own_name.map(str::to_string),
        }
    }

    /// …and the guid, for the two that want one.
    pub fn raid_guid(&self, index: usize, own: Option<u64>) -> Option<u64> {
        match self.raid_slot(index)? {
            RaidSlot::Member(at) => Some(self.members.get(at)?.guid),
            RaidSlot::Player => own,
        }
    }
}

pub struct RaidPlugin;

impl Plugin for RaidPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (learn_member_classes, send_verbs).in_set(super::GameSet),
        );
    }
}

/// **Copy each member's class off the name cache**, once it has arrived.
///
/// The class is the one thing a raid button draws that no group packet carries,
/// and it is the whole of what `RAID_CLASS_COLORS` is keyed by. The query itself
/// is the session thread's — `ObjectManager::note_group` marks the roster's
/// guids wanted and `resolve_names` sends `CMSG_NAME_QUERY` for the ones the
/// cache has never answered — so all this does is read the answers across.
///
/// **Locks the world only when something is missing**, which after the first
/// second of a group is never: the guard is a scan of at most forty `Option`s
/// against a mutex a session thread is writing forty times a second.
fn learn_member_classes(session: Res<Session>, mut party: ResMut<Party>) {
    if party.members.iter().all(|member| member.class.is_some()) {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let Ok(world) = active.live.world().lock() else {
        return;
    };
    for member in &mut party.members {
        if member.class.is_some() {
            continue;
        }
        if let Some(info) = world.players.get(&member.guid) {
            member.class = Some(info.class as u8);
        }
    }
}

/// Send what the raid panel asked for.
///
/// The sibling of [`super::party::send_verbs`] and separate from it for the
/// reason this whole file is: three of the five verbs need the *roster* to turn
/// a raid index into a name or a guid, and one of them needs the local player's
/// own name — which is a row the roster does not have.
fn send_verbs(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    mut party: ResMut<Party>,
    units: Query<&crate::world::session::WorldEntity>,
) {
    let Some(mut host) = host else { return };
    let requests = host.take_raid_verbs();
    if requests.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // **A bare query rather than `interface::api::Units`**, which holds `Res<Party>`
    // — and this system writes it. See [`super::party::request_missing_stats`],
    // which pays the same price for the same reason.
    let me = units.iter().find(|unit| unit.is_self);
    let own_guid = me.map(|unit| unit.guid);
    let own_name = me.map(|unit| unit.name.as_str());

    for request in requests {
        use crate::lua::panels::raid::RaidRequest as R;
        let verb = match request {
            R::Convert => PartyVerb::ConvertToRaid,
            // **The one write here that is not a packet.** See
            // [`Party::raid_selection`].
            R::Select(index) => {
                party.raid_selection = index;
                continue;
            }
            // **The subgroup goes out zero-based**, where the interface counts
            // its eight columns from one — see
            // [`vale_protocol::play::group::change_sub_group_body`]. A member
            // already in that column sends nothing, which is the reference's own
            // guard and not politeness: the server answers a no-op move with a
            // whole fresh `SMSG_GROUP_LIST` to everybody in the raid.
            R::SetSubgroup { index, subgroup } => {
                let Some(slot) = party.raid_slot(index) else {
                    continue;
                };
                if subgroup == 0 || subgroup > MAX_RAID_SUBGROUPS {
                    continue;
                }
                if party.raid_subgroup(slot) == subgroup {
                    continue;
                }
                match party.raid_name(index, own_name) {
                    Some(name) => PartyVerb::ChangeSubGroup {
                        name,
                        subgroup: (subgroup - 1) as u8,
                    },
                    None => continue,
                }
            }
            // …and a drop onto an occupied slot is the *other* opcode, which
            // exists because two moves cannot both be made when both columns are
            // full.
            R::SwapSubgroup { index, with } => {
                match (
                    party.raid_name(index, own_name),
                    party.raid_name(with, own_name),
                ) {
                    (Some(name), Some(swap_with)) if name != swap_with => {
                        PartyVerb::SwapSubGroup { name, swap_with }
                    }
                    _ => continue,
                }
            }
            // **By guid, unlike the two above it**, which is 1.12's own split
            // and not a choice — see
            // [`vale_protocol::play::group::assistant_leader_body`].
            R::SetAssistant { name, assistant } => {
                match party
                    .members
                    .iter()
                    .find(|member| member.name.eq_ignore_ascii_case(&name))
                {
                    Some(member) => PartyVerb::SetAssistant {
                        guid: member.guid,
                        assistant,
                    },
                    // Ourselves included: vmangos refuses `player == GetPlayer()`
                    // outright, so a promotion aimed at our own row is dropped
                    // here rather than sent to be ignored.
                    None => continue,
                }
            }
            // `UninviteFromRaid(index)` — a raid index where the party's kick is
            // a token or a name. The guid form of the opcode, because that is
            // what the roster can always produce.
            R::Uninvite(index) => match party.raid_guid(index, own_guid) {
                Some(guid) => PartyVerb::UninviteGuid(guid),
                None => continue,
            },
            R::StartReadyCheck => PartyVerb::StartReadyCheck,
            R::AnswerReadyCheck(ready) => PartyVerb::AnswerReadyCheck(ready),
        };
        active.live.party(verb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_protocol::play::group::{GroupList, GroupMember, member_status};

    /// A raid roster as the server sends it: everybody but the reader.
    fn raid(members: &[(u64, &str, u8)], leader: u64, own_flags: u8) -> GroupList {
        GroupList {
            group_type: 1,
            own_flags,
            members: members
                .iter()
                .map(|(guid, name, flags)| GroupMember {
                    guid: *guid,
                    name: (*name).to_string(),
                    status: member_status::ONLINE,
                    flags: *flags,
                })
                .collect(),
            leader,
            loot: Some((vale_protocol::play::group::LootMethod::GroupLoot, 0, 2)),
        }
    }

    /// **We are the last row and we are counted**, which is the whole of what
    /// makes a raid roster different from the packet it comes out of.
    #[test]
    fn the_local_player_is_the_last_raid_slot() {
        let mut party = Party::default();
        party.apply_list(&raid(&[(7, "Bram", 0), (8, "Merrick", 1)], 7, 2));
        assert_eq!(party.raid_count(), 3, "two members and us");
        assert_eq!(party.raid_slot(1), Some(RaidSlot::Member(0)));
        assert_eq!(party.raid_slot(2), Some(RaidSlot::Member(1)));
        assert_eq!(party.raid_slot(3), Some(RaidSlot::Player));
        assert_eq!(party.raid_slot(4), None);
        assert_eq!(party.raid_slot(0), None, "the token is one-based");
        assert_eq!(party.raid_name(3, Some("Alden")), Some("Alden".to_string()));
        assert_eq!(party.raid_index_of(8, Some(99)), Some(2));
        assert_eq!(party.raid_index_of(99, Some(99)), Some(3), "our own row");
        assert_eq!(party.raid_index_of(1234, Some(99)), None);
    }

    /// **A party answers zero to all of it**, which is what keeps the five party
    /// frames on the screen and the Convert to Raid button enabled.
    #[test]
    fn a_party_is_not_a_small_raid() {
        let mut party = Party::default();
        let mut list = raid(&[(7, "Bram", 0)], 7, 0);
        list.group_type = 0;
        party.apply_list(&list);
        assert_eq!(party.raid_count(), 0);
        assert_eq!(party.raid_slot(1), None);
        assert_eq!(party.raid_index_of(7, Some(99)), None);
        assert!(!party.is_raid_officer(Some(7)));
    }

    /// **The subgroup is the low nibble and the assistant bit is the top one**,
    /// and reading the byte whole puts every assistant in column 129 — which is
    /// a nil index in `getglobal("RaidGroup"..subgroup)` and takes the rebuild
    /// down.
    #[test]
    fn a_flags_byte_is_a_column_and_a_rank() {
        let mut party = Party::default();
        party.apply_list(&raid(
            &[(7, "Bram", 0x80 | 2), (8, "Merrick", 4)],
            9,
            0x80 | 3,
        ));
        assert_eq!(party.raid_subgroup(RaidSlot::Member(0)), 3, "column 2 is the third");
        assert_eq!(party.raid_subgroup(RaidSlot::Member(1)), 5);
        assert_eq!(party.raid_subgroup(RaidSlot::Player), 4, "our own byte");
        assert_eq!(party.raid_rank(RaidSlot::Member(0), Some(99)), raid_rank::ASSISTANT);
        assert_eq!(party.raid_rank(RaidSlot::Member(1), Some(99)), raid_rank::MEMBER);
        assert_eq!(party.raid_rank(RaidSlot::Player, Some(99)), raid_rank::ASSISTANT);
        // …and the leader outranks the bit, whoever holds it.
        assert_eq!(party.raid_rank(RaidSlot::Player, Some(9)), raid_rank::LEADER);
        assert!(party.is_raid_officer(Some(9)), "the leader is an officer");
        assert!(party.is_raid_officer(Some(99)), "…and so is the assistant");
    }

    /// **An officer is a rank, not a bit** — and a leader who never held the
    /// assistant bit is one.
    #[test]
    fn a_leader_without_the_bit_is_still_an_officer() {
        let mut party = Party::default();
        party.apply_list(&raid(&[(7, "Bram", 0)], 99, 0));
        assert_eq!(party.raid_rank(RaidSlot::Player, Some(99)), raid_rank::LEADER);
        assert!(party.is_raid_officer(Some(99)));
        // …and an ordinary member is not.
        party.apply_list(&raid(&[(7, "Bram", 0)], 7, 0));
        assert!(!party.is_raid_officer(Some(99)));
    }

    /// **Converting is a roster change that moves nothing else**, which is the
    /// one case a membership comparison cannot see — and the reason
    /// `RAID_ROSTER_UPDATE` has a field of its own to be raised off.
    #[test]
    fn converting_a_party_is_a_change_with_no_membership_in_it() {
        let mut party = Party::default();
        let mut as_party = raid(&[(7, "Bram", 0)], 7, 0);
        as_party.group_type = 0;
        party.apply_list(&as_party);
        let change = party.apply_list(&raid(&[(7, "Bram", 0)], 7, 0));
        assert!(!change.members, "the same one person");
        assert!(!change.leader);
        assert!(change.raid, "…and yet everything about the screen moved");
        // …and a drag from one column to another is the other invisible change.
        let change = party.apply_list(&raid(&[(7, "Bram", 3)], 7, 0));
        assert!(!change.members);
        assert!(!change.raid);
        assert!(change.flags);
        // …and an unchanged roster is none of them.
        let change = party.apply_list(&raid(&[(7, "Bram", 3)], 7, 0));
        assert!(!change.members && !change.raid && !change.flags && !change.leader);
    }

    /// **A raid that ends leaves nothing behind**, our own subgroup byte and the
    /// picked-up row included.
    #[test]
    fn a_destroyed_raid_clears_the_bytes_only_a_raid_has() {
        let mut party = Party::default();
        party.apply_list(&raid(&[(7, "Bram", 0)], 7, 0x80 | 4));
        party.raid_selection = 2;
        assert!(party.destroyed());
        assert_eq!(party.raid_count(), 0);
        assert_eq!(party.own_flags, 0);
        assert_eq!(party.raid_selection, 0);
    }
}
