//! **The party** — who is in it, who leads it, and the six verbs the interface
//! calls.
//!
//! ```text
//! what the server says   SMSG_GROUP_LIST, whole, every time anything moves
//! what a press does      CMSG_GROUP_INVITE / ACCEPT / DECLINE / DISBAND
//! what a member is       a name, a guid and a status byte — and, when they are
//!                        near enough, an entity in the object manager as well
//! ```
//!
//! ## A member is two different things depending on where they are standing
//!
//! This is the whole of what makes the party different from every other unit
//! subject in this directory. `party1` may be:
//!
//! * **in range** — a `WorldEntity` like any other, with a model, a health field
//!   and auras, and every `Unit*` read answers off it;
//! * **out of range** — a row in [`Party::members`] and nothing else. There is
//!   no entity, no update block and no aura list. What the frame can draw is the
//!   name, the class colour, the status byte and whatever
//!   `SMSG_PARTY_MEMBER_STATS` last said about the health bar.
//!
//! Both have to answer `UnitExists("party1")` with **true**, which is why
//! [`super::super::api::Units`] holds this resource: a token that resolves to no entity
//! is not the same as a token that names nobody, and getting the two crossed
//! either hides a party frame for a member across the zone or leaves one
//! standing for a member who left.
//!
//! ## The roster arrives whole and that is the design, not a shortcut
//!
//! There is no incremental packet. `Group::SendUpdate` re-sends every member's
//! own copy on every change, so [`Party::apply_list`] replaces the list outright
//! and the events are raised off the *difference* — which is what
//! `PARTY_MEMBERS_CHANGED` means to the interface: not "somebody joined" but
//! "look again".
//!
//! **The stats are kept across a re-send** and that is deliberate: the roster
//! packet carries no health, so folding it in naively blanks every party frame's
//! bar each time anybody's loot rule changes. [`Party::apply_list`] carries the
//! old reading forward by guid.
//!
//! ## Leaving is `CMSG_GROUP_DISBAND` and there is nothing else
//!
//! One opcode covers both, and which one happens is the server's decision off
//! whether we lead — so `LeaveParty()` has nothing to decide and the interface's
//! own confirm box is the only thing that distinguishes them.

use bevy::prelude::*;

use vale_protocol::play::group::{GroupList, LootMethod, PartyMemberStats};
use vale_protocol::socket::session::PartyVerb;

use super::super::events::{
    PartyInviteRequest, PartyLeaderChanged, PartyLootMethodChanged, PartyMembersChanged,
};
use crate::world::session::Session;

/// One member of the party — everybody but us.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartyMember {
    pub guid: u64,
    pub name: String,
    /// [`vale_protocol::play::group::member_status`] — 0 is offline.
    pub status: u8,
    pub flags: u8,
    /// **What `SMSG_PARTY_MEMBER_STATS` last said**, which is the only thing
    /// that knows a member's health when they are out of range. `None` until a
    /// stats packet has arrived for them.
    pub stats: Option<PartyMemberStats>,
    /// **The `ChrClasses` id, out of the name cache** — `None` until
    /// `CMSG_NAME_QUERY` has been answered for this guid.
    ///
    /// Not on the wire anywhere else: `SMSG_GROUP_LIST` carries a name, a guid
    /// and two bytes, and `SMSG_PARTY_MEMBER_STATS` carries numbers. The
    /// reference reads it out of its own name cache at the top of
    /// `GetRaidRosterInfo` and so does this — see
    /// [`super::raid::learn_member_classes`], which is what fills it, and
    /// `ObjectManager::group_guids`, which is what makes the query go out for a
    /// member with no entity.
    pub class: Option<u8>,
}

impl PartyMember {
    pub fn online(&self) -> bool {
        self.status & vale_protocol::play::group::member_status::ONLINE != 0
    }
    pub fn dead(&self) -> bool {
        self.status
            & (vale_protocol::play::group::member_status::DEAD
                | vale_protocol::play::group::member_status::GHOST)
            != 0
    }
    pub fn ghost(&self) -> bool {
        self.status & vale_protocol::play::group::member_status::GHOST != 0
    }
    pub fn afk(&self) -> bool {
        self.status & vale_protocol::play::group::member_status::AFK != 0
    }
    pub fn dnd(&self) -> bool {
        self.status & vale_protocol::play::group::member_status::DND != 0
    }
    pub fn pvp(&self) -> bool {
        self.status & vale_protocol::play::group::member_status::PVP != 0
    }
}

/// The party, as the interface reads it.
#[derive(Resource, Debug, Default)]
pub struct Party {
    /// **Everybody but us**, in the order the server sent them — which is what
    /// `party1`..`party4` index. The server's own order, not sorted here: the
    /// reference does not sort it either, and a re-order would make the frames
    /// swap places for no reason a player can see.
    pub members: Vec<PartyMember>,
    /// The leader's guid — **ours when we lead**, which is the one case where a
    /// guid in this struct is not in [`Self::members`].
    pub leader: u64,
    pub loot: LootMethod,
    pub looter: u64,
    pub threshold: u8,
    /// Whether this is a raid rather than a party — `SMSG_GROUP_LIST`'s first
    /// byte. See [`super::raid`], which is everything derived from it.
    pub raid: bool,
    /// **Our own subgroup and assistant bit**, the roster packet's second byte:
    /// the subgroup in the low seven bits and `0x80` when we are an assistant.
    ///
    /// Here rather than in [`Self::members`] because the server leaves the
    /// reader out of their own copy — this byte is the only thing that says
    /// which of the eight columns our own raid button belongs in.
    pub own_flags: u8,
    /// **Which raid row was last picked up or clicked**, one-based, 0 for none.
    ///
    /// `SetRaidRosterSelection(index)`, and it is pure client state: the
    /// reference stores it and nothing in 1.12's own directory reads it back. It is here because `RaidGroupButton_OnDragStart` calls it,
    /// and an unregistered name there takes the whole drag body down.
    pub raid_selection: usize,
    /// **Who is asking us to join theirs**, and the whole of the popup's state.
    /// `None` when there is no invitation outstanding.
    pub invited_by: Option<String>,
    /// Guids a `CMSG_REQUEST_PARTY_MEMBER_STATS` is already out for — see
    /// [`request_missing_stats`], which is what fills a party frame for somebody
    /// who has not moved since we joined.
    asked: std::collections::HashSet<u64>,
}

impl Party {
    /// `GetNumPartyMembers()` — **never counts us**, which is why an empty party
    /// and no party are the same answer.
    pub fn count(&self) -> usize {
        self.members.len()
    }

    /// The member `party<n>` names, `n` from 1.
    pub fn member(&self, index: usize) -> Option<&PartyMember> {
        self.members.get(index.checked_sub(1)?)
    }

    /// `GetPartyLeaderIndex()` — **0 when *we* lead**, which is the client's own
    /// convention and what `PartyMemberFrame_UpdateLeader` tests against.
    /// **Is this guid one of the group's?** — the set the reference's group
    /// test answers over, and the one friend-or-foe consults before it looks at
    /// either faction.
    ///
    /// Ourselves are not in [`Self::members`], so this is "somebody else in the
    /// group" rather than "in the group", which is exactly the question the
    /// reference asks: it walks the roster for the *other* unit's guid having
    /// already established that one of the two is the local character.
    pub fn holds(&self, guid: u64) -> bool {
        guid != 0 && self.members.iter().any(|member| member.guid == guid)
    }

    pub fn leader_index(&self, own_guid: Option<u64>) -> usize {
        if self.leader == 0 {
            return 0;
        }
        if own_guid == Some(self.leader) {
            return 0;
        }
        self.members
            .iter()
            .position(|member| member.guid == self.leader)
            .map_or(0, |index| index + 1)
    }

    /// **Fold a whole roster in, keeping what the stats packets said.**
    ///
    /// Answers what changed, so the caller raises only the events that mean
    /// something — see [`RosterChange`].
    pub fn apply_list(&mut self, list: &GroupList) -> RosterChange {
        let before: Vec<(u64, u8)> = self
            .members
            .iter()
            .map(|member| (member.guid, member.status))
            .collect();
        // **The flags byte separately**, because it is the raid's own news and
        // nothing else reads it: a subgroup move and a promotion to assistant
        // both arrive here and in nothing else the packet carries.
        let before_flags: Vec<u8> = self.members.iter().map(|member| member.flags).collect();
        let old_own_flags = self.own_flags;
        let old_leader = self.leader;
        let old_loot = (self.loot, self.looter, self.threshold);

        self.members = list
            .members
            .iter()
            .map(|row| PartyMember {
                guid: row.guid,
                name: row.name.clone(),
                status: row.status,
                flags: row.flags,
                // **Carried forward by guid** — the roster packet has no health
                // in it, so a fresh row would blank every bar. See the module
                // comment.
                stats: self
                    .members
                    .iter()
                    .find(|old| old.guid == row.guid)
                    .and_then(|old| old.stats.clone()),
                // …and so is the class, for the same reason and one step
                // further out: it comes from a name query rather than from any
                // group packet at all.
                class: self
                    .members
                    .iter()
                    .find(|old| old.guid == row.guid)
                    .and_then(|old| old.class),
            })
            .collect();
        self.leader = list.leader;
        let was_raid = self.raid;
        self.raid = list.is_raid();
        self.own_flags = list.own_flags;
        if let Some((method, looter, threshold)) = list.loot {
            self.loot = method;
            self.looter = looter;
            self.threshold = threshold;
        }

        let now: Vec<(u64, u8)> = self
            .members
            .iter()
            .map(|member| (member.guid, member.status))
            .collect();
        RosterChange {
            // **The membership, not the statuses** — a member going AFK must not
            // rebuild every frame in the party.
            members: before.iter().map(|(guid, _)| *guid).ne(now.iter().map(|(guid, _)| *guid)),
            // …and the online bit on its own is what the two `PARTY_MEMBER_*`
            // names are for.
            connection: before
                .iter()
                .zip(now.iter())
                .any(|((_, was), (_, is))| (was & 1) != (is & 1))
                || before.len() != now.len(),
            leader: self.leader != old_leader,
            loot: (self.loot, self.looter, self.threshold) != old_loot,
            // **Converting is a roster change even when nobody joined**, which
            // is the one case the membership test above cannot see: the same
            // four people, one byte different, and every party frame has to come
            // off the screen. See [`super::raid`].
            raid: self.raid != was_raid,
            // …and so is a drag from one subgroup column to another, which moves
            // nothing but one nibble of one byte.
            flags: self.own_flags != old_own_flags
                || before_flags
                    .iter()
                    .ne(self.members.iter().map(|member| &member.flags)),
        }
    }

    /// Fold in one member's stats. Ignores a guid the roster does not have,
    /// which happens for a beat after somebody leaves.
    ///
    /// **The health in here is not `PARTY_MEMBERS_CHANGED` news** — see
    /// [`StatsChange`]. The bars ride `UNIT_HEALTH` and its nine siblings like
    /// every other unit frame in the game, raised by
    /// [`crate::game::character::vitals`] off the reading this leaves behind.
    pub fn apply_stats(&mut self, stats: PartyMemberStats) -> StatsChange {
        self.asked.remove(&stats.guid);
        let Some(member) = self.members.iter_mut().find(|m| m.guid == stats.guid) else {
            return StatsChange::default();
        };
        let was = member.status;
        if let Some(status) = stats.status {
            member.status = status;
        }
        // **Folded, not replaced.** The packet is a diff — see
        // [`PartyMemberStats::fold`] — and replacing the reading with one is a
        // frame that fills in on join and empties itself the moment the member
        // takes a step, which is exactly what it did.
        member.stats.get_or_insert_default().fold(&stats);
        StatsChange {
            known: true,
            status: member.status != was,
        }
    }

    /// The party is over — [`vale_protocol::play::group::GroupList`] never says so
    /// and `SMSG_GROUP_DESTROYED` is the only thing that does.
    pub fn destroyed(&mut self) -> bool {
        let had = !self.members.is_empty() || self.leader != 0;
        self.members.clear();
        self.leader = 0;
        self.looter = 0;
        self.threshold = 0;
        self.raid = false;
        self.own_flags = 0;
        self.raid_selection = 0;
        self.loot = LootMethod::default();
        had
    }
}

/// What [`Party::apply_list`] found had moved. Each field is one of the game's
/// own event names, and raising all four on every roster packet is what makes a
/// party frame rebuild itself sixty times while somebody is AFK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RosterChange {
    pub members: bool,
    pub connection: bool,
    pub leader: bool,
    pub loot: bool,
    /// **The group became a raid, or stopped being one** — which no other field
    /// here can be: a Convert to Raid changes nobody's guid, nobody's status and
    /// nobody's loot rule. See [`super::raid`].
    pub raid: bool,
    /// **Somebody's subgroup or assistant bit moved**, ours included.
    ///
    /// The raid's own change and no use to the party, which is all subgroup 0
    /// and has no assistants. It is separate from [`Self::members`] because that
    /// field compares *guids*: a drag from column 3 to column 4 leaves the
    /// membership identical and is the whole point of the panel.
    pub flags: bool,
}

/// What one `SMSG_PARTY_MEMBER_STATS` moved.
///
/// The split exists because the packet carries two different kinds of news over
/// one opcode, and only one of them is an event the party frames want:
///
/// * **the status byte** — online, dead, ghost, AFK — is what
///   `PARTY_MEMBERS_CHANGED` means, and it is what greys a frame out
///   (`UnitFrameManaBar_Update` paints a disconnected member's bar 0.5 grey);
/// * **the numbers** — health, power, level, zone, position — are
///   `UNIT_HEALTH`'s and its siblings', raised by
///   [`crate::game::character::vitals`], and the map dot is *polled* by
///   `WorldMapFrame_OnUpdate` rather than pushed.
///
/// Raising the roster event for the numbers is not merely redundant: vmangos
/// pushes this packet on **every position change** of an out-of-range member,
/// so it ran `PartyMemberFrame_UpdateMember` — a full rebuild, buffs and
/// portrait and background included — several times a second for as long as
/// anybody in the party was walking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatsChange {
    /// The guid was in the roster, so the reading landed somewhere. `false` for
    /// a member who left between the change and the packet — and the request
    /// mark is cleared either way, which is what keeps them from being asked
    /// for ever.
    pub known: bool,
    /// **The status byte moved**, which is the only half of a stats packet the
    /// roster event is about.
    pub status: bool,
}

/// What the session thread said about the party, forwarded by
/// [`super::super::combat::action::drain_events`] on the same terms as every other subject's.
#[derive(Message, Debug, Clone)]
pub enum PartyAnswer {
    Invited(String),
    Declined(String),
    List(Box<GroupList>),
    Destroyed,
    NewLeader(String),
    Stats(PartyMemberStats),
    Result(vale_protocol::play::group::PartyCommandResult),
    /// `MSG_RAID_READY_CHECK` — see [`super::raid`], and
    /// [`vale_protocol::play::group::ReadyCheck`] for why one opcode carries
    /// two different things.
    ReadyCheck(vale_protocol::play::group::ReadyCheck),
}

/// A verb the interface called, queued the way every other write in `lua/` is —
/// it happens inside a handler with the world borrowed.
pub type PartyQueue = std::rc::Rc<std::cell::RefCell<Vec<PartyVerb>>>;

pub struct PartyPlugin;

impl Plugin for PartyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Party>()
            .add_message::<PartyAnswer>()
            .add_systems(
                Update,
                (
                    apply_answers,
                    request_missing_stats,
                    send_verbs,
                    leave_world,
                )
                    .chain()
                    .in_set(super::super::GameSet),
            );
    }
}

/// **The four names a roster change raises**, as one parameter.
///
/// The same shape `action::NpcAnswers` has and for the same reason: four
/// writers, and [`apply_answers`] was over `too_many_arguments` with them
/// spelled out. These four really are one subject — they are the party's own
/// event set — so the grouping is a description rather than an accident.
#[derive(bevy::ecs::system::SystemParam)]
pub struct PartyEvents<'w> {
    members: MessageWriter<'w, PartyMembersChanged>,
    leader: MessageWriter<'w, PartyLeaderChanged>,
    loot: MessageWriter<'w, PartyLootMethodChanged>,
    invited: MessageWriter<'w, PartyInviteRequest>,
    /// **The raid's own two**, raised from here rather than from
    /// [`super::raid`] because the decision of *when* is [`RosterChange`]'s and
    /// this is the one place that record exists. What a raid *is* is still next
    /// door.
    raid: MessageWriter<'w, super::super::events::RaidRosterUpdate>,
    ready: MessageWriter<'w, super::super::events::ReadyCheck>,
}

/// Fold the server's answers into the roster and raise the game's own names.
fn apply_answers(
    mut answers: MessageReader<PartyAnswer>,
    mut party: ResMut<Party>,
    mut say: super::super::messages::Announce,
    mut raise: PartyEvents,
) {
    for answer in answers.read() {
        match answer {
            // **The popup's whole state is a name**, and there is no second
            // packet: a declined or expired invitation is never announced, so
            // the box is closed by the press or by `SMSG_GROUP_LIST` arriving.
            PartyAnswer::Invited(name) => {
                party.invited_by = Some(name.clone());
                raise.invited.write(PartyInviteRequest {
                    from: name.clone(),
                });
            }
            // …and somebody we asked said no, which is a chat line and nothing
            // else. The game's own words: `ERR_DECLINE_GROUP_S`.
            PartyAnswer::Declined(name) => {
                // `ERR_DECLINE_GROUP_S` — the game's own sentence, not one
                // composed here, which is this client's rule for every message
                // it puts on the screen.
                say.formatted("ERR_DECLINE_GROUP_S", name);
            }
            PartyAnswer::List(list) => {
                party.invited_by = None;
                let was_raid = party.raid;
                let change = party.apply_list(list);
                if change.members {
                    raise.members.write(PartyMembersChanged);
                }
                if change.connection {
                    raise.members.write(PartyMembersChanged);
                }
                if change.leader {
                    raise.leader.write(PartyLeaderChanged);
                }
                if change.loot {
                    raise.loot.write(PartyLootMethodChanged);
                }
                // **`RAID_ROSTER_UPDATE` is not `PARTY_MEMBERS_CHANGED` under
                // another name.** It is what loads `Blizzard_RaidUI`, and it is
                // what `UIParent.lua` answers by deciding whether the five party
                // frames belong on screen at all — so a client that raises only
                // the party's name draws a party over its own raid. Raised for a
                // *subgroup* change too, which no other field here sees.
                //
                // `was_raid` and not only `party.raid`: the group ceasing to be
                // one is exactly as much news as it becoming one.
                if (was_raid || party.raid)
                    && (change.members || change.leader || change.raid || change.flags)
                {
                    raise.raid.write(super::super::events::RaidRosterUpdate);
                }
            }
            PartyAnswer::Destroyed => {
                party.invited_by = None;
                let was_raid = party.raid;
                if party.destroyed() {
                    raise.members.write(PartyMembersChanged);
                    raise.leader.write(PartyLeaderChanged);
                    if was_raid {
                        raise.raid.write(super::super::events::RaidRosterUpdate);
                    }
                }
            }
            // **Only the start of one is an event, and that is 1.12 rather
            // than an omission.** The answers are forwarded to the leader alone
            // and the shipped interface draws nothing at all for them:
            // `GlobalStrings.lua` has six `READY_CHECK*` keys and not one of
            // them is "somebody answered", so there is no line to print and no
            // frame to fill. Kept as a read rather than left unparsed, because
            // an opcode nothing reads is one the net tab reports as unhandled.
            PartyAnswer::ReadyCheck(check) => {
                if *check == vale_protocol::play::group::ReadyCheck::Started {
                    raise.ready.write(super::super::events::ReadyCheck);
                }
            }
            // **The broadcast names the new leader by *name*** where the request
            // was by guid, so the roster's own leader guid is what is corrected
            // — and the `SMSG_GROUP_LIST` that follows it is what actually moves
            // it. This is the chat line.
            PartyAnswer::NewLeader(name) => {
                say.formatted("ERR_NEW_LEADER_S", name);
            }
            // **Only the status byte is roster news** — see [`StatsChange`].
            // The health, power and level this packet carries reach the frames
            // as `UNIT_*` events off the reading it leaves behind, which is the
            // path `PartyFrameTemplates.xml` actually wires its bars to.
            PartyAnswer::Stats(stats) => {
                if party.apply_stats(stats.clone()).status {
                    raise.members.write(PartyMembersChanged);
                }
            }
            // **A refusal is a `GlobalStrings.lua` key**, and three of the nine
            // take the member's name as their `%s` — see
            // [`vale_protocol::play::group::PartyResult`].
            PartyAnswer::Result(result) => {
                let Some(key) = result.result.key() else {
                    continue;
                };
                if result.result.takes_name() {
                    say.formatted(key, &result.member);
                } else {
                    say.key(key);
                }
            }
        }
    }
}

/// **Ask for a member's health once, when nothing else will say it.**
///
/// vmangos pushes `SMSG_PARTY_MEMBER_STATS` at an out-of-range member's health,
/// mana, level, zone and position on **every change**
/// (`Player::SendUpdateToOutOfRangeGroupMembers`), so nothing here needs a
/// clock. What it does not cover is the case where nothing has changed yet: a
/// member standing still across the zone when we join is a name in the roster
/// and no numbers at all, which is a party frame with a blank bar until they
/// take a step.
///
/// `CMSG_REQUEST_PARTY_MEMBER_STATS` is the answer and the server replies with
/// `GROUP_UPDATE_FULL`. Asked **once per guid** — [`Party::apply_stats`] clears
/// the mark when the answer lands, and a member who never answers (offline) is
/// asked once and then left alone, because the server's reply for one is a
/// status-only packet that still counts as an answer.
fn request_missing_stats(
    session: Res<Session>,
    mut party: ResMut<Party>,
    // **A bare query rather than [`super::super::api::Units`]**, which holds
    // `Res<Party>` — and this system writes it, which is Bevy's B0002. That is
    // checked when the schedule is built rather than by the compiler, so it is
    // a panic at startup; both of the systems added this round tripped it.
    units: Query<&crate::world::session::WorldEntity>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // A member the world can see needs nothing: their entity carries everything
    // a frame reads, and the request would be answered with what we already
    // have.
    let wanted: Vec<u64> = party
        .members
        .iter()
        .filter(|member| member.stats.is_none())
        .filter(|member| !units.iter().any(|unit| unit.guid == member.guid))
        .map(|member| member.guid)
        .filter(|guid| !party.asked.contains(guid))
        .collect();
    for guid in wanted {
        party.asked.insert(guid);
        active.live.party(PartyVerb::RequestStats(guid));
    }
}

/// Send what the interface asked for.
fn send_verbs(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    mut party: ResMut<Party>,
) {
    let Some(mut host) = host else { return };
    let verbs = host.take_party_verbs();
    if verbs.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    for request in verbs {
        use crate::lua::panels::party::PartyRequest as R;
        // **The popup closes on the press, not on a packet.** Neither answer
        // has one — the server tells the *inviter* about a decline and nobody
        // about an accept — so a box left open would sit there for ever.
        if matches!(request, R::Accept | R::Decline) {
            party.invited_by = None;
        }
        let verb = match request {
            // **A blank name is dropped rather than sent**: it reaches the
            // server as a lookup that always fails and comes back as
            // `ERR_BAD_PLAYER_NAME_S` over an empty `%s`.
            R::Invite(name) if name.is_empty() => continue,
            R::Invite(name) => PartyVerb::Invite(name),
            R::Accept => PartyVerb::Accept,
            R::Decline => PartyVerb::Decline,
            R::Leave => PartyVerb::Leave,
            // The wire has a by-name kick, so `/uninvite` passes straight
            // through as the name that was typed.
            R::UninviteByName(name) if name.is_empty() => continue,
            R::UninviteByName(name) => PartyVerb::Uninvite(name),
            // …and the dropdown's form names a *unit*, which only the roster can
            // turn into anybody: `party2` is not in the world when its member is
            // across the zone.
            R::UninviteUnit(token) => match guid_of(&party, &token) {
                Some(guid) => PartyVerb::UninviteGuid(guid),
                None => continue,
            },
            // Promotion has no by-name opcode at all, so both of its forms end
            // at a guid. A name nobody in the party has sends nothing, which is
            // what the server would have answered anyway.
            R::PromoteByName(name) => match party
                .members
                .iter()
                .find(|member| member.name.eq_ignore_ascii_case(&name))
            {
                Some(member) => PartyVerb::SetLeader(member.guid),
                None => continue,
            },
            R::PromoteUnit(token) => match guid_of(&party, &token) {
                Some(guid) => PartyVerb::SetLeader(guid),
                None => continue,
            },
        };
        active.live.party(verb);
    }
}

/// **The guid behind a unit token, for the two verbs that take one.**
///
/// Only `party<n>` is answered, and that is the whole set the dropdown can
/// reach: `UnitPopup` builds its party entries against those four tokens, and a
/// kick or a promotion aimed at anything else has no meaning. Out of the roster
/// rather than out of the world, because a member across the zone has no entity.
fn guid_of(party: &Party, token: &str) -> Option<u64> {
    let index: usize = token
        .to_ascii_lowercase()
        .strip_prefix("party")?
        .parse()
        .ok()?;
    Some(party.member(index)?.guid)
}

/// Let go of the last character's party — see [`super`]'s note on why each
/// module resets its own.
fn leave_world(
    mut leaving: MessageReader<super::super::events::PlayerLeavingWorld>,
    mut party: ResMut<Party>,
) {
    if leaving.read().next().is_some() {
        *party = Party::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_protocol::play::group::{GroupMember, member_status};

    fn list(members: &[(u64, &str, u8)], leader: u64) -> GroupList {
        GroupList {
            group_type: 0,
            own_flags: 0,
            members: members
                .iter()
                .map(|(guid, name, status)| GroupMember {
                    guid: *guid,
                    name: (*name).to_string(),
                    status: *status,
                    flags: 0,
                })
                .collect(),
            leader,
            loot: Some((LootMethod::GroupLoot, 0, 2)),
        }
    }

    /// **The roster is replaced whole and the health is carried across it**,
    /// which is the one thing `SMSG_GROUP_LIST` cannot say for itself.
    #[test]
    fn a_re_sent_roster_keeps_what_the_stats_packets_said() {
        let mut party = Party::default();
        party.apply_list(&list(&[(7, "Bram", member_status::ONLINE)], 7));
        assert!(party.apply_stats(PartyMemberStats {
            guid: 7,
            health: Some(400),
            max_health: Some(900),
            ..Default::default()
        })
        .known);
        // The same roster again — a loot-rule change, say.
        let change = party.apply_list(&list(&[(7, "Bram", member_status::ONLINE)], 7));
        assert!(!change.members, "nobody joined or left");
        assert_eq!(
            party.member(1).and_then(|m| m.stats.as_ref()).and_then(|s| s.health),
            Some(400),
            "the bar was blanked by a roster packet"
        );
        // …and a *new* member is a change.
        let change = party.apply_list(&list(
            &[
                (7, "Bram", member_status::ONLINE),
                (8, "Merrick", member_status::ONLINE),
            ],
            7,
        ));
        assert!(change.members);
        assert_eq!(party.count(), 2);
        assert_eq!(party.member(2).map(|m| m.name.as_str()), Some("Merrick"));
        assert_eq!(party.member(3), None);
    }

    /// **`GetPartyLeaderIndex()` is 0 when we lead**, and the member's index
    /// otherwise — one-based, like the token.
    #[test]
    fn the_leader_index_is_zero_for_us_and_one_based_for_everybody_else() {
        let mut party = Party::default();
        party.apply_list(&list(
            &[(7, "Bram", 1), (8, "Merrick", 1)],
            8,
        ));
        assert_eq!(party.leader_index(Some(99)), 2, "party2 leads");
        // …and when the leader guid is ours it is zero, whoever else is in it.
        party.apply_list(&list(&[(7, "Bram", 1), (8, "Merrick", 1)], 99));
        assert_eq!(party.leader_index(Some(99)), 0);
        // A leader nobody in the list has and that is not us reads as 0 too,
        // which is the safe direction: no crown rather than the wrong one.
        assert_eq!(party.leader_index(Some(1)), 0);
    }

    /// **A status change is not a membership change**, which is the difference
    /// between a frame that redraws and a party that rebuilds.
    #[test]
    fn going_afk_does_not_rebuild_the_party() {
        let mut party = Party::default();
        party.apply_list(&list(&[(7, "Bram", member_status::ONLINE)], 7));
        let change = party.apply_list(&list(
            &[(7, "Bram", member_status::ONLINE | member_status::AFK)],
            7,
        ));
        assert!(!change.members, "the roster did not move");
        assert!(!change.connection, "…and neither did the online bit");
        assert!(party.member(1).is_some_and(|m| m.afk() && m.online()));
        // …but going offline is one of the two `PARTY_MEMBER_*` names.
        let change = party.apply_list(&list(&[(7, "Bram", 0)], 7));
        assert!(change.connection);
        assert!(party.member(1).is_some_and(|m| !m.online()));
    }

    /// **A member's frame survives them taking a step**, which is what it did
    /// not do: the packet is a *diff*, and replacing the held reading with one
    /// blanked the health, the level, the zone and therefore the map dot a few
    /// seconds after joining.
    #[test]
    fn a_partial_stats_update_does_not_blank_a_member() {
        let mut party = Party::default();
        party.apply_list(&list(&[(7, "Bram", member_status::ONLINE)], 7));
        // The answer to `CMSG_REQUEST_PARTY_MEMBER_STATS`: everything.
        party.apply_stats(PartyMemberStats {
            guid: 7,
            status: Some(member_status::ONLINE),
            health: Some(800),
            max_health: Some(1200),
            level: Some(31),
            zone: Some(12),
            position: Some((100, -200)),
            ..Default::default()
        });
        // …and then a step, which is `GROUP_UPDATE_FLAG_POSITION` alone.
        party.apply_stats(PartyMemberStats {
            guid: 7,
            position: Some((105, -204)),
            ..Default::default()
        });
        let held = party.member(1).and_then(|m| m.stats.as_ref()).expect("a reading");
        assert_eq!(held.position, Some((105, -204)));
        assert_eq!(held.health, Some(800), "the bar blanked");
        assert_eq!(held.level, Some(31), "the level blanked");
        assert_eq!(held.zone, Some(12), "the map dot and the zone line went");
        // …and the status byte, which is kept on the member rather than in the
        // reading, is left alone by an update that does not carry one.
        assert!(party.member(1).is_some_and(|m| m.online()));
    }

    /// **A member's health moving is not a roster change**, which is the
    /// difference between a bar that slides and every party frame rebuilding
    /// itself — buffs, portrait and background — on every step an out-of-range
    /// member takes. The status byte is the half that *is* roster news.
    #[test]
    fn a_health_change_is_not_party_members_changed_but_a_status_change_is() {
        let mut party = Party::default();
        party.apply_list(&list(&[(7, "Bram", member_status::ONLINE)], 7));
        let change = party.apply_stats(PartyMemberStats {
            guid: 7,
            status: Some(member_status::ONLINE),
            health: Some(900),
            max_health: Some(900),
            ..Default::default()
        });
        assert!(change.known);
        assert!(!change.status, "the status byte did not move");
        // …taking damage is `UNIT_HEALTH` and nothing else.
        let change = party.apply_stats(PartyMemberStats {
            guid: 7,
            health: Some(400),
            ..Default::default()
        });
        assert!(!change.status, "a rebuild for a health tick");
        assert_eq!(
            party.member(1).and_then(|m| m.stats.as_ref()).and_then(|s| s.health),
            Some(400)
        );
        // …and dying across the zone is, because it greys the whole frame.
        let change = party.apply_stats(PartyMemberStats {
            guid: 7,
            status: Some(member_status::ONLINE | member_status::DEAD),
            health: Some(0),
            ..Default::default()
        });
        assert!(change.status);
        assert!(party.member(1).is_some_and(PartyMember::dead));
    }

    /// **A stats packet answers a request and stops it being re-asked** — see
    /// [`request_missing_stats`], whose whole job is to fire once per member.
    #[test]
    fn a_stats_answer_clears_the_request_mark() {
        let mut party = Party::default();
        party.apply_list(&list(&[(7, "Bram", member_status::ONLINE)], 7));
        party.asked.insert(7);
        assert!(
            party
                .apply_stats(PartyMemberStats {
                    guid: 7,
                    health: Some(1),
                    ..Default::default()
                })
                .known
        );
        assert!(!party.asked.contains(&7), "the mark outlived the answer");
        // …and an answer for a guid the roster does not have still clears the
        // mark, which is what keeps a member who left from being asked for ever.
        party.asked.insert(99);
        assert!(
            !party
                .apply_stats(PartyMemberStats {
                    guid: 99,
                    ..Default::default()
                })
                .known
        );
        assert!(!party.asked.contains(&99));
    }

    /// **Only `SMSG_GROUP_DESTROYED` empties a party**, and it says so once.
    #[test]
    fn a_destroyed_group_empties_the_roster_once() {
        let mut party = Party::default();
        party.apply_list(&list(&[(7, "Bram", 1)], 7));
        assert!(party.destroyed());
        assert_eq!(party.count(), 0);
        assert_eq!(party.leader_index(Some(1)), 0);
        assert!(!party.destroyed(), "the second one says nothing");
    }
}
