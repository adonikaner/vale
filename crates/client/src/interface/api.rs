//! **The surface an interface asks questions through**, named the way the game
//! names it.
//!
//! Everything in `interface/` is state the client owns; this module is how that state
//! is *addressed*. The distinction matters because the real client's interface
//! never touches the state directly — it calls a fixed set of C functions, and
//! every one of them that concerns a creature takes a **unit token**:
//!
//! ```lua
//! UnitHealth("target")           UnitName("targettarget")     UnitIsUnit("player", "target")
//! TargetUnit("player")           AssistUnit("target")         FollowUnit("target")
//! ```
//!
//! `Bindings.xml` is full of them — `TARGETSELF` is literally
//! `if ( UnitIsUnit("player", "target") ) then TargetUnit("pet") else TargetUnit("player") end`
//! — so the token is not an interface convenience layered on top of a guid API.
//! It **is** the API, and a guid never appears in FrameXML at all.
//!
//! ## Why that is worth having before the Lua host
//!
//! Because half the tokens are *derived rather than stored*, and the derivation is
//! the client's job. `target` is a selection this client holds; `targettarget` is
//! a field on whatever that resolves to and is a second lookup; `mouseover` is
//! this frame's hover and lives for one frame. An interface written against
//! `Entity` would have to do all three itself, differently each time — which is
//! how the same rule ends up written twice with one of them out of date, the
//! failure `assets::dress` was moved to fix.
//!
//! So the guid stays the internal identity (it is what goes on the wire) and the
//! token is the address. [`Units`] is the join.
//!
//! ## What is here and what is next door
//!
//! **Questions here, verbs in the module that owns the state.** `UnitHealth` and
//! `GetActionCooldown` are pure reads and live in this file; `UseAction` mutates a
//! cooldown and sends a packet, so it is [`super::action::use_action`], and
//! `TargetUnit` is [`super::target::target_unit`]. The game makes no such split —
//! it is one flat C API — but a Rust read is a `&` and a verb needs half the
//! world mutably, and pretending otherwise would mean every query dragging a
//! `ResMut` behind it. The names are the game's either way, which is the part
//! that has to survive.
//!
//! ## The numbers are the interface's, not the wire's
//!
//! One trap, and it is the same one that made a warrior's rage bar read
//! "1000/1000": **`UnitMana` returns what the game would display**, and rage is
//! stored in tenths. `UnitMana("player")` on a warrior is 0..100 and
//! `UNIT_FIELD_POWER2` is 0..1000. A function named after the game's must answer
//! what the game's answers, or every consumer has to remember the exception.

use super::action::{ActionBar, Cooldowns, Slot};
use super::target::{Hovered, Selection};
use crate::world::session::WorldEntity;
use vale_protocol::state::objects::power_type;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// One of the game's unit tokens.
///
/// Not the whole set — the game has `party1`..`party4`, `partypet1`..`4`, `pet`,
/// `raid1`..`raid40` and a `player`/`target` pair of *modifier* forms
/// (`targettarget`, `pettarget`). What is here is what this client has state for.
/// A token it cannot answer is [`UnitId::parse`]-able to `None` rather than to a
/// wrong unit, because "there is no party" and "the party is empty" are the same
/// answer to `UnitExists` and different answers to everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitId {
    /// `"player"` — us.
    Player,
    /// `"target"` — what we have selected.
    Target,
    /// `"targettarget"` — what *it* has selected, which is how a healer watches a
    /// tank's aggro and how this client knows it is being attacked.
    TargetTarget,
    /// `"mouseover"` — whatever the cursor is over **this frame**.
    Mouseover,
    /// `"npc"` — whoever the character is talking to: the gossip menu's giver,
    /// the vendor, the quest page's NPC. Derived by
    /// [`crate::interface::gossip::point_the_token`] from whichever window is up,
    /// and `"questnpc"` parses here too — the reference distinguishes them only
    /// for the trainer, which this client does not have.
    Npc,
    /// `"party1"`..`"party4"` — **one-based**, and the one token here whose unit
    /// may not be in the world at all.
    ///
    /// A party member across the zone has no `WorldEntity`: what this client
    /// knows about them is a row in [`crate::interface::party::Party`] and whatever
    /// `SMSG_PARTY_MEMBER_STATS` last said. So [`Units::resolve`] answers `None`
    /// for one and [`Units::exists`] still answers **true** — see that method,
    /// which is where the difference is paid for.
    Party(usize),
    /// `"pet"` — **our charm if we have one, otherwise our summon**, which is
    /// the precedence the client reads them in and not a guess. See
    /// [`vale_protocol::state::objects::Entity::pet_guid`].
    ///
    /// Not a stored token: it is derived from the local player's own fields
    /// every time it is asked, like `targettarget`. `PetFrame_Update` hides the
    /// whole frame unless `UnitExists("pet")`, so a client that never answers
    /// this draws no pet frame at all — which is what it did.
    Pet,
    /// `"raid1"`..`"raid40"` — **one-based, and *we* are one of them**, which is
    /// the whole difference from [`Self::Party`].
    ///
    /// The server leaves the reader out of their own `SMSG_GROUP_LIST`, so the
    /// last slot is the local player and the client makes the join — see
    /// [`crate::interface::raid`], which is where the order comes from and why
    /// it is stable. Everything else about the token is `party<n>`'s: a member
    /// across the zone has no entity, [`Units::resolve`] answers `None` for one,
    /// and [`Units::exists`] still answers **true**.
    Raid(usize),
    /// `"partypet1"`..`"partypet4"` — **one-based**, like [`Self::Party`], and
    /// with the same two-places-to-look rule one level deeper.
    ///
    /// The client resolves it by looking the *owner* up in the object manager
    /// and reading their live charm/summon; only when the owner is not there
    /// does it fall back to the pet guid `SMSG_PARTY_MEMBER_STATS` cached on
    /// that member's row. So this token exists for a pet whose owner is across
    /// the zone, and both halves are in [`Units::pet_guid_for`].
    PartyPet(usize),
}

/// **`raid1`..`raid40` as literals**, because [`UnitId::token`] answers a
/// `&'static str` and forty `format!`s a frame is not what a token is for.
const RAID_TOKENS: [&str; vale_protocol::play::group::MAX_RAID_MEMBERS] = [
    "raid1", "raid2", "raid3", "raid4", "raid5", "raid6", "raid7", "raid8",
    "raid9", "raid10", "raid11", "raid12", "raid13", "raid14", "raid15",
    "raid16", "raid17", "raid18", "raid19", "raid20", "raid21", "raid22",
    "raid23", "raid24", "raid25", "raid26", "raid27", "raid28", "raid29",
    "raid30", "raid31", "raid32", "raid33", "raid34", "raid35", "raid36",
    "raid37", "raid38", "raid39", "raid40",
];

impl UnitId {
    /// The token as the game spells it. Lower case, no separators — `targettarget`
    /// really is one word.
    pub fn token(&self) -> &'static str {
        match self {
            UnitId::Player => "player",
            UnitId::Target => "target",
            UnitId::TargetTarget => "targettarget",
            UnitId::Mouseover => "mouseover",
            UnitId::Npc => "npc",
            // Four literals rather than `format!`, which is what keeps this a
            // `&'static str` for the twenty call sites that want one.
            UnitId::Party(index) => match index {
                1 => "party1",
                2 => "party2",
                3 => "party3",
                _ => "party4",
            },
            // **A table rather than forty literals in a match**, which is the
            // same trade the four above make: the token has to outlive the call
            // for the twenty consumers that want a `&'static str`.
            UnitId::Raid(index) => RAID_TOKENS
                .get(index.saturating_sub(1))
                .copied()
                .unwrap_or("raid40"),
            UnitId::Pet => "pet",
            UnitId::PartyPet(index) => match index {
                1 => "partypet1",
                2 => "partypet2",
                3 => "partypet3",
                _ => "partypet4",
            },
        }
    }

    /// Parse a token. **Case-insensitive**, because the game's own is: FrameXML
    /// writes `"player"` throughout but the C side folds case, and an addon
    /// author who writes `"Player"` gets a working call rather than a silent nil.
    pub fn parse(token: &str) -> Option<UnitId> {
        match token.to_ascii_lowercase().as_str() {
            "player" => Some(UnitId::Player),
            "target" => Some(UnitId::Target),
            "targettarget" => Some(UnitId::TargetTarget),
            "mouseover" => Some(UnitId::Mouseover),
            "npc" | "questnpc" => Some(UnitId::Npc),
            "pet" => Some(UnitId::Pet),
            "party1" => Some(UnitId::Party(1)),
            "party2" => Some(UnitId::Party(2)),
            "party3" => Some(UnitId::Party(3)),
            "party4" => Some(UnitId::Party(4)),
            // **`raidpet<n>` is not this and must not fall through to it.**
            // The client tests the `"raidpet"` prefix *before* the `"raid"`
            // prefix, and a parser that
            // checked the shorter prefix first would read `raidpet3` as raid
            // slot 0 and answer with somebody's owner. This client keeps no
            // raid-pet state, so the token is `None` — see [`UnitId`]'s own note
            // about answering nothing rather than the wrong unit.
            token if token.starts_with("raidpet") => None,
            token if token.starts_with("raid") => {
                let index: usize = token.strip_prefix("raid")?.parse().ok()?;
                (1..=vale_protocol::play::group::MAX_RAID_MEMBERS)
                    .contains(&index)
                    .then_some(UnitId::Raid(index))
            }
            "partypet1" => Some(UnitId::PartyPet(1)),
            "partypet2" => Some(UnitId::PartyPet(2)),
            "partypet3" => Some(UnitId::PartyPet(3)),
            "partypet4" => Some(UnitId::PartyPet(4)),
            _ => None,
        }
    }

    /// **Whose pet this token names**, or `None` for a token that is not a
    /// pet's — the join the client makes by falling through `"pet"` to the
    /// local player and `"partypet<n>"` to `"party<n>"`.
    pub fn owner(&self) -> Option<UnitId> {
        match self {
            UnitId::Pet => Some(UnitId::Player),
            UnitId::PartyPet(index) => Some(UnitId::Party(*index)),
            _ => None,
        }
    }

    /// …and the inverse: **the token this one's pet would be named by**, or
    /// `None` for a token that cannot own one.
    ///
    /// Two tokens can, and they are exactly the two the game raises `UNIT_PET`
    /// at — `PetFrame_OnEvent` opens on `arg1 == "player"` and
    /// `PartyMemberFrame_OnEvent` on `arg1 == "party<n>"`. So this is the
    /// event's own guard as well as a lookup, and it is why a `partypet<n>` row
    /// raises nothing: a pet does not have a pet, and no body in the ninety
    /// files would hear it if it did.
    pub fn pet(&self) -> Option<UnitId> {
        match self {
            UnitId::Player => Some(UnitId::Pet),
            UnitId::Party(index) => Some(UnitId::PartyPet(*index)),
            _ => None,
        }
    }
}

/// **Two units' gap, in the pieces the server tests separately** — see
/// [`Units::separation`], which is where the reason is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Separation {
    /// Centre to centre in the horizontal plane.
    pub across: f32,
    /// …and the vertical part, signed: positive when the second unit is above
    /// the first.
    pub up: f32,
    /// Both units' `UNIT_FIELD_COMBATREACH`, in the order they were asked for.
    /// **Raw**, not floored — the melee rule's own `max(reach, 1.5)` belongs to
    /// the caller applying that rule, and a spell's does not apply it at all.
    pub reaches: (f32, f32),
}

/// Every unit in the world, addressable by token.
///
/// A [`SystemParam`] rather than a resource because resolving a token needs the
/// entity query as well as the two selection resources, and a system that wants
/// to ask one question should not have to name all three.
#[derive(SystemParam)]
pub struct Units<'w, 's> {
    /// Everything the server has told us about, for the scans below.
    ///
    /// **The `Transform` is `Option` and that is load-bearing**, not caution: a
    /// `WorldEntity` exists from the update block that names it and is *placed*
    /// by the renderer some frames later, so a plain `&Transform` in this query
    /// would filter an unplaced unit out of the query altogether — and every
    /// read in this file goes through it, so `UnitName` and `UnitHealth` would
    /// start answering nil for a unit the server has already described. The one
    /// read that needs a position ([`Units::reach`]) looks at it through
    /// [`Units::placed`] and answers `None` rather than a distance of zero.
    pub all: Query<'w, 's, (Entity, &'static WorldEntity, Option<&'static Transform>)>,
    selection: Res<'w, Selection>,
    hovered: Res<'w, Hovered>,
    /// Whoever the character is talking to — see [`crate::interface::gossip::NpcUnit`].
    npc: Res<'w, crate::interface::gossip::NpcUnit>,
    /// …and who is in the group, which is the only thing that knows a
    /// `party<n>` token names anybody at all — see [`crate::interface::party`].
    pub party: Res<'w, crate::interface::party::Party>,
    /// …and what the character's own reputation says, which is the half of
    /// friend-or-foe that is in no file — see
    /// [`crate::interface::reputation::PlayerStanding`]. **Nothing that
    /// takes this param may take it `ResMut` as well**; the one system that
    /// writes it does not take `Units`.
    standing: Res<'w, crate::interface::reputation::PlayerStanding>,
}

/// **One side of the friend-or-foe question, off an entity** — the same
/// [`vale_assets::tables::faction::Party`] [`Units`] builds from a token.
///
/// Free rather than a method because four passes hold `WorldEntity`s and no
/// tokens: the pointer's judgement, the Tab-target scan, the spell cursor and
/// the cast's binding. **They must build it the same way this does**, or the
/// picker and the interface disagree about who is a friend — which is the one
/// property [`Units::rank`]'s own comment is about.
///
/// The group flag is the only field that is not a straight read: a unit is "in
/// the local character's group" when the roster names its guid.
pub fn faction_party(
    unit: &WorldEntity,
    party: &crate::interface::party::Party,
) -> vale_assets::tables::faction::Party {
    vale_assets::tables::faction::Party {
        guid: unit.guid,
        faction: unit.faction,
        unit_flags: unit.unit_flags,
        player_flags: unit.player_flags,
        duel_arbiter: unit.duel_arbiter,
        duel_team: unit.duel_team,
        is_local: unit.is_self,
        in_local_group: !unit.is_self && party.holds(unit.guid),
    }
}

/// **The two resources friend-or-foe needs beyond the tables**, bundled.
///
/// One reference rather than two, because the cast path threads them through
/// four call sites and a pair of `&Res` arguments repeated down a chain is the
/// shape that gets one of them dropped. Copy, so passing it costs nothing.
#[derive(Clone, Copy)]
pub struct Friendship<'a> {
    pub party: &'a crate::interface::party::Party,
    pub standing: &'a crate::interface::reputation::PlayerStanding,
}

impl Friendship<'_> {
    /// [`rank_between`], over the pair this bundle holds.
    pub fn rank(
        &self,
        tables: &vale_assets::tables::dbc::DisplayTables,
        a: &WorldEntity,
        b: &WorldEntity,
    ) -> vale_assets::tables::faction::Rank {
        rank_between(tables, self.party, self.standing, a, b)
    }

    /// …and the same fold every three-way consumer wants.
    pub fn reaction(
        &self,
        tables: &vale_assets::tables::dbc::DisplayTables,
        a: &WorldEntity,
        b: &WorldEntity,
    ) -> vale_assets::tables::faction::Reaction {
        self.rank(tables, a, b).into()
    }
}

/// **How `a` stands towards `b`**, for those same four passes.
///
/// A missing `FactionTemplate.dbc` reads Neutral, which is
/// [`vale_assets::tables::faction::Factions::template_rank`]'s own answer for
/// a template it does not have.
pub fn rank_between(
    tables: &vale_assets::tables::dbc::DisplayTables,
    party: &crate::interface::party::Party,
    standing: &crate::interface::reputation::PlayerStanding,
    a: &WorldEntity,
    b: &WorldEntity,
) -> vale_assets::tables::faction::Rank {
    match tables.factions() {
        Some(factions) => factions.rank(
            &faction_party(a, party),
            &faction_party(b, party),
            &standing.lend(),
        ),
        None => vale_assets::tables::faction::Rank::Neutral,
    }
}

/// …and whether `a` may swing at `b`, which is not the reaction alone — see
/// [`vale_assets::tables::faction::Factions::can_attack`].
pub fn can_attack_between(
    tables: &vale_assets::tables::dbc::DisplayTables,
    party: &crate::interface::party::Party,
    standing: &crate::interface::reputation::PlayerStanding,
    a: &WorldEntity,
    b: &WorldEntity,
) -> bool {
    match tables.factions() {
        Some(factions) => factions.can_attack(
            &faction_party(a, party),
            &faction_party(b, party),
            &standing.lend(),
        ),
        None => vale_assets::tables::faction::can_attack(
            b.unit_flags,
            vale_assets::tables::faction::Reaction::Neutral,
        ),
    }
}

impl Units<'_, '_> {
    /// The entity a token names, if it currently names one.
    pub fn resolve(&self, id: UnitId) -> Option<Entity> {
        match id {
            // **By the flag rather than by a `With<LocalPlayer>` query.** The
            // marker component and the field say the same thing and the field is
            // already in the query this param holds; a second query would make
            // every caller pay for a filter it never reads.
            UnitId::Player => self
                .all
                .iter()
                .find(|(_, unit, _)| unit.is_self)
                .map(|(entity, _, _)| entity),
            UnitId::Target => self.selection.entity,
            // Derived, and the reason this module exists: two lookups, one of
            // them by guid, which is exactly what a consumer would otherwise
            // reimplement.
            UnitId::TargetTarget => {
                let target = self.get(UnitId::Target)?;
                let of = target.target?;
                self.all
                    .iter()
                    .find(|(_, unit, _)| unit.guid == of)
                    .map(|(entity, _, _)| entity)
            }
            UnitId::Mouseover => self.hovered.entity,
            // By guid, like target-of-target: the token is the server's
            // identity and the entity is looked up fresh, so a despawned NPC
            // answers nothing rather than a stale handle.
            UnitId::Npc => {
                let of = self.npc.0?;
                self.all
                    .iter()
                    .find(|(_, unit, _)| unit.guid == of)
                    .map(|(entity, _, _)| entity)
            }
            // **By guid through the roster, and `None` is ordinary here.** A
            // party member out of range has no entity; every read below that
            // can be answered off the roster instead says so on its own.
            UnitId::Party(index) => {
                let of = self.party.member(index)?.guid;
                self.all
                    .iter()
                    .find(|(_, unit, _)| unit.guid == of)
                    .map(|(entity, _, _)| entity)
            }
            // **The same, plus the one row the server never sends.** The last
            // raid slot is us — see [`crate::interface::raid`] — so this
            // token resolves through the local player rather than through the
            // roster, which does not have us in it.
            UnitId::Raid(index) => match self.party.raid_slot(index)? {
                crate::interface::raid::RaidSlot::Player => self.resolve(UnitId::Player),
                crate::interface::raid::RaidSlot::Member(at) => {
                    let of = self.party.members.get(at)?.guid;
                    self.all
                        .iter()
                        .find(|(_, unit, _)| unit.guid == of)
                        .map(|(entity, _, _)| entity)
                }
            },
            // **Derived twice over** — whose pet, then which guid — and the
            // entity looked up fresh like every other derived token. A pet the
            // world has not streamed in is `None` here and still *exists*; see
            // [`Units::exists`].
            UnitId::Pet | UnitId::PartyPet(_) => {
                let of = self.pet_guid_for(id)?;
                self.all
                    .iter()
                    .find(|(_, unit, _)| unit.guid == of)
                    .map(|(entity, _, _)| entity)
            }
        }
    }

    /// **What a pet token names, as a guid** — the two places the client looks,
    /// in the order it looks in them.
    ///
    /// The client's lookup behind `partypet<n>` is four steps: take the
    /// owner's guid out of the party array, ask the object manager for them,
    /// and if it answers read their live `UNIT_FIELD_CHARM`/`SUMMON`. Only when
    /// the object manager has *no* owner does it fall back to the pet guid
    /// `SMSG_PARTY_MEMBER_STATS` cached on that member's row — gated on the
    /// row's own valid bit, which is why [`vale_protocol::play::group::PartyPetStats::exists`]
    /// treats a zero guid as "no pet" rather than as no answer.
    ///
    /// **The order is load-bearing and it is not the obvious one.** A cache
    /// consulted first would keep showing a dismissed pet for as long as the
    /// member stayed out of range and nothing re-sent the block; the live
    /// fields win wherever they exist, and an owner who is present with no pet
    /// answers `None` rather than falling through to a stale row.
    ///
    /// `"pet"` has only the first half: it reads the local
    /// player's own two fields and stops, because there is no cache for our own
    /// pet and no case in which we are out of our own range.
    pub fn pet_guid_for(&self, id: UnitId) -> Option<u64> {
        let owner = id.owner()?;
        if let Some(unit) = self.get(owner) {
            return unit.pet;
        }
        self.party_pet_row(id)
            .and_then(|pet| pet.guid)
            .filter(|guid| *guid != 0)
    }

    /// The **pet** half of a party member's cached stats, for a `partypet<n>`
    /// token — `None` for every other token, including `pet`, which has no
    /// cache. The sibling of [`Units::party_row`], one unit further out.
    fn party_pet_row(&self, id: UnitId) -> Option<&vale_protocol::play::group::PartyPetStats> {
        match id {
            UnitId::PartyPet(index) => Some(&self.party.member(index)?.stats.as_ref()?.pet),
            _ => None,
        }
    }

    /// What the server has told us about the unit a token names.
    pub fn get(&self, id: UnitId) -> Option<&WorldEntity> {
        let entity = self.resolve(id)?;
        self.all.get(entity).ok().map(|(_, unit, _)| unit)
    }

    /// Where a unit is standing, or `None` for one the renderer has not placed
    /// yet — see the note on [`Units::all`].
    pub fn placed(&self, id: UnitId) -> Option<(&WorldEntity, &Transform)> {
        let entity = self.resolve(id)?;
        let (_, unit, at) = self.all.get(entity).ok()?;
        Some((unit, at?))
    }

    /// **How far apart two units are, less both their bulk** — vmangos'
    /// `GetCombatDistance`, which is what `Spell::CheckRange` is written
    /// against, so a client measuring anything else disagrees with the server
    /// about every range in the game.
    ///
    /// `None` when either end is absent or unplaced. That is not the same as
    /// "far away" and callers must not treat it as one: a missing measurement
    /// reads as *do not answer*, which is what keeps a unit that arrived this
    /// frame from flashing an out-of-range hotkey.
    ///
    /// The same arithmetic as [`crate::interface::action::reach_between`], which
    /// measures for a *press* out of its own query rather than for a read —
    /// they are two callers of one rule and the rule is one subtraction.
    pub fn reach(&self, from: UnitId, to: UnitId) -> Option<f32> {
        let (me, here) = self.placed(from)?;
        let (other, there) = self.placed(to)?;
        if me.guid == other.guid {
            return Some(0.0);
        }
        let centres = here.translation.distance(there.translation);
        Some((centres - me.combat_reach - other.combat_reach).max(0.0))
    }

    /// **A unit's name by guid**, for the one window whose other party has no
    /// token until it opens: the trade request names who asked and nothing
    /// else. `None` for a guid not in view.
    pub fn name_of_guid(&self, guid: u64) -> Option<String> {
        self.all
            .iter()
            .find(|(_, unit, _)| unit.guid == guid)
            .map(|(_, unit, _)| unit.name.clone())
            .filter(|name| !name.is_empty())
    }

    /// [`Units::reach`] to a unit named by **guid** rather than by token — the
    /// spirit healer whose offer is on the table has no token of its own, and
    /// the client's own range check measures against the guid the confirm
    /// packet carried. `None` on the same terms as `reach`.
    pub fn reach_to_guid(&self, from: UnitId, guid: u64) -> Option<f32> {
        let (me, here) = self.placed(from)?;
        let (_, other, there) = self.all.iter().find(|(_, unit, _)| unit.guid == guid)?;
        let there = there?;
        if me.guid == other.guid {
            return Some(0.0);
        }
        let centres = here.translation.distance(there.translation);
        Some((centres - me.combat_reach - other.combat_reach).max(0.0))
    }

    /// **The gap between two units, taken apart**, for a caller that has to
    /// apply the server's own range rule rather than this client's.
    ///
    /// [`Units::reach`] is the right answer to "how far apart are they" and the
    /// wrong shape for that: it folds the horizontal distance, the vertical
    /// distance and both units' bulk into one number, and **vmangos does not
    /// test them together**. `Unit::CanReachWithMeleeAutoAttackAtPosition` ends
    /// in
    ///
    /// ```text
    /// (dx*dx + dy*dy < reach*reach) && (dz*dz < zReach)
    /// ```
    ///
    /// — a *two-dimensional* distance against the summed reaches and a separate
    /// height test — where `Spell::CheckRange` is the 3D `GetCombatDistance`
    /// [`Units::reach`] already mirrors. A caller measuring melee in 3D would
    /// disagree with the server on any slope, which is exactly the class of
    /// wrongness the instrument that wants this exists to detect.
    ///
    /// `None` on the same terms as [`Units::reach`]: either end absent or
    /// unplaced is *do not answer*, never zero.
    pub fn separation(&self, from: UnitId, to: UnitId) -> Option<Separation> {
        let (me, here) = self.placed(from)?;
        let (other, there) = self.placed(to)?;
        // **Bevy's Y is up** — `render::axes::to_bevy` is `(-y, z, -x)`, so the
        // world's horizontal plane is Bevy's XZ and the height is its Y.
        let (a, b) = (here.translation, there.translation);
        Some(Separation {
            across: ((b.x - a.x).powi(2) + (b.z - a.z).powi(2)).sqrt(),
            up: b.y - a.y,
            reaches: (me.combat_reach, other.combat_reach),
        })
    }

    /// `UnitExists` — is there anything at this token at all?
    ///
    /// **A `party<n>` token exists whether or not the member is in the world**,
    /// which is the whole of what [`UnitId::Party`]'s note is about: hiding a
    /// party frame because its member walked out of range is a bug a player sees
    /// every time somebody rounds a corner.
    pub fn exists(&self, id: UnitId) -> bool {
        // **A pet exists when its guid does, entity or no.** `UnitExists`
        // asks the object manager and then falls back to a walk over our own
        // charm/summon and every party row's pet guid — so a pet that has been summoned but not yet streamed in,
        // or one whose owner is across the zone, is *there*. Answering off the
        // entity alone hides `PetFrame` for the first seconds of every summon.
        if id.owner().is_some() {
            return self.pet_guid_for(id).is_some();
        }
        self.get(id).is_some() || self.party_row(id).is_some()
    }

    /// **Who the token names, as a guid** — the one identity that outlives the
    /// entity, so it is what tells a *slot* apart from its occupant.
    ///
    /// `party1` is a slot rather than a person: somebody leaving shifts
    /// everybody below them up one, and the token then names a different
    /// character whose numbers may happen to match. A watcher that compares
    /// only the numbers announces nothing there — see
    /// [`crate::interface::vitals`], which holds this beside the vitals for
    /// exactly that reason. Falls back to the roster, like every other read a
    /// member across the zone can still answer.
    pub fn guid(&self, id: UnitId) -> Option<u64> {
        if id.owner().is_some() {
            return self.pet_guid_for(id);
        }
        match self.get(id) {
            Some(unit) => Some(unit.guid),
            None => self.party_row(id).map(|member| member.guid),
        }
    }

    /// The roster row behind a `party<n>` token, or `None` for every other
    /// token — the fallback for the four reads that can be answered without an
    /// entity.
    fn party_row(&self, id: UnitId) -> Option<&crate::interface::party::PartyMember> {
        match id {
            UnitId::Party(index) => self.party.member(index),
            // …and a raid slot, which is the same row reached a different way —
            // except for the last one, which is the local player and has no row
            // at all. That is not a gap: we are always in the world, so every
            // read that would fall back here is already answered off the entity.
            UnitId::Raid(index) => match self.party.raid_slot(index)? {
                crate::interface::raid::RaidSlot::Member(at) => {
                    self.party.members.get(at)
                }
                crate::interface::raid::RaidSlot::Player => None,
            },
            _ => None,
        }
    }

    /// `UnitName`. **Falls back to the roster**, which carries a name for a
    /// member nothing else in this client knows about.
    pub fn name(&self, id: UnitId) -> Option<&str> {
        match self.get(id) {
            Some(unit) => Some(unit.name.as_str()),
            // …and a party *pet* out of range has one too, off the same packet
            // — `GROUP_UPDATE_FLAG_PET_NAME`. Empty for a member with no pet,
            // which the builder writes as a bare NUL, so it is filtered here
            // rather than drawn as a blank plate.
            None => match self.party_pet_row(id) {
                Some(pet) => pet.name.as_deref().filter(|name| !name.is_empty()),
                None => self.party_row(id).map(|member| member.name.as_str()),
            },
        }
    }

    /// `UnitLevel`. **`-1` for a unit whose level we do not know**, which is the
    /// game's own answer and is what the interface draws as a skull.
    pub fn level(&self, id: UnitId) -> i32 {
        if let Some(unit) = self.get(id) {
            return unit.level.map_or(-1, |level| level as i32);
        }
        self.party_row(id)
            .and_then(|member| member.stats.as_ref()?.level)
            .map_or(-1, i32::from)
    }

    /// **The level as the interface is allowed to see it** — `-1` where the
    /// reference refuses to say, which is what draws the `??` skull.
    ///
    /// [`Self::level`] above is the raw number and stays raw: five callers want
    /// the player's own level for arithmetic (the skill ranks, the two craft
    /// windows, the trainer) and a `-1` there would be a bug rather than a
    /// skull. This is the one every *display* goes through.
    ///
    /// ## The rule `UnitLevel` follows
    ///
    /// A target hostile to the player whose level is ten or more above the
    /// player's answers `-1`; so does a world boss (classification 3), at any
    /// level; everything else answers its true level.
    ///
    /// Two things in that which a plain reading of the report would miss:
    ///
    /// * **The ten-level rule is gated on hostility.** The reaction the client
    ///   tests is `UnitReaction` minus one, and `<= 1` is Hated or Hostile. A *neutral*
    ///   unit reports its true level however far above the player it is, which
    ///   is why a level 60 quest giver in a starting zone shows its number.
    ///   This client's [`vale_assets::tables::faction::Reaction`] collapses
    ///   the eight ranks to three and maps `Hostile` onto `UnitReaction` 2, so
    ///   the gate is exactly that variant.
    /// * **The boss clause is not gated at all** and is tested either way,
    ///   so a worldboss is `??` at any level and any reaction.
    ///
    /// The comparison is `playerLevel <= targetLevel - 10`, so a unit exactly
    /// nine levels above still shows its number and one ten above does not.
    ///
    /// `None` tables is no opinion about the reaction, for the reason
    /// [`crate::lua::api::UnitAnswers::unit_rank`] gives: with no `FactionTemplate`
    /// every unit reads Neutral, and a neutral reading here would silently turn
    /// the ten-level rule off for the whole session. The boss clause needs no
    /// table and applies either way.
    pub fn level_shown(
        &self,
        tables: Option<&vale_assets::tables::dbc::DisplayTables>,
        id: UnitId,
    ) -> i32 {
        let level = self.level(id);
        let Some(unit) = self.get(id) else {
            return level;
        };
        if unit.classification == BOSS_CLASSIFICATION {
            return -1;
        }
        let hostile = tables.is_some_and(|tables| {
            self.reaction(tables, id, UnitId::Player)
                == Some(vale_assets::tables::faction::Reaction::Hostile)
        });
        // **Both levels have to be known.** The client reaches the comparison
        // only with a player object in hand; here a level nothing has stated is
        // `-1`, and `-1 <= level - 10` is true for anything above level 9 — so
        // an unguarded test would mask every unit in the world for the frames
        // between entering it and the player's own fields arriving.
        let player = self.level(UnitId::Player);
        if hostile && level > 0 && player > 0 && player <= level - 10 {
            return -1;
        }
        level
    }

    /// `UnitXP` / `UnitXPMax`, and **zero for anybody but the player** — both
    /// fields are `PRIVATE` and the server sends nobody else's.
    ///
    /// The pair is answered together because it is read together and because a
    /// maximum of zero is load-bearing: `TextStatusBar_UpdateTextString` hides a
    /// bar whose maximum is zero, so a client that answered a real `UnitXP` and
    /// a stubbed `UnitXPMax` would take the XP bar off the screen — which is
    /// exactly what a pair of stubs did.
    pub fn experience(&self, id: UnitId) -> (u32, u32) {
        self.get(id).and_then(|unit| unit.experience).unwrap_or((0, 0))
    }

    /// `GetXPExhaustion()` — the rested pool, `None` when there is none.
    pub fn rested_experience(&self, id: UnitId) -> Option<u32> {
        self.get(id).and_then(|unit| unit.rested)
    }

    /// `UnitHealth`.
    ///
    /// **This is not always hit points**, and that is the server's doing rather
    /// than a shortcut here: vmangos sends another player's health as a percentage
    /// of maximum, with `UnitHealthMax` reading 100 to match. The pair is always
    /// self-consistent, so a fraction is always right and an absolute number is
    /// only meaningful where the server sent one.
    pub fn health(&self, id: UnitId) -> u32 {
        if let Some(unit) = self.get(id) {
            return unit.health_value.map_or(0, |(now, _)| now);
        }
        // …and off `SMSG_PARTY_MEMBER_STATS` for a member out of range, which is
        // the only thing that carries one.
        self.party_row(id)
            .and_then(|member| member.stats.as_ref()?.health)
            .or_else(|| self.party_pet_row(id)?.health)
            .map_or(0, u32::from)
    }

    /// `UnitHealthMax` — see [`Units::health`] on what it is a maximum *of*.
    pub fn health_max(&self, id: UnitId) -> u32 {
        if let Some(unit) = self.get(id) {
            return unit.health_value.map_or(0, |(_, max)| max);
        }
        self.party_row(id)
            .and_then(|member| member.stats.as_ref()?.max_health)
            .or_else(|| self.party_pet_row(id)?.max_health)
            .map_or(0, u32::from)
    }

    /// `UnitMana` — the current power, **as the interface shows it**.
    ///
    /// The name is the game's and so is the generality: `UnitMana` answers for
    /// rage, energy and focus too, which is why the bar in FrameXML is one widget.
    /// Rage is stored in tenths and this divides it out — see the module comment.
    pub fn mana(&self, id: UnitId) -> u32 {
        if let Some(unit) = self.get(id) {
            return unit
                .power_value
                .map_or(0, |(now, _, kind)| power_type::display(kind, now));
        }
        self.party_power(id)
            .map_or(0, |(now, _, kind)| power_type::display(kind, now))
    }

    /// `UnitManaMax`.
    pub fn mana_max(&self, id: UnitId) -> u32 {
        if let Some(unit) = self.get(id) {
            return unit
                .power_value
                .map_or(0, |(_, max, kind)| power_type::display(kind, max));
        }
        self.party_power(id)
            .map_or(0, |(_, max, kind)| power_type::display(kind, max))
    }

    /// `UnitPowerType` — which of the five bars this unit runs on.
    pub fn power_type(&self, id: UnitId) -> Option<u8> {
        match self.get(id) {
            Some(unit) => unit.power_value.map(|(_, _, kind)| kind),
            None => self.party_power(id).map(|(_, _, kind)| kind),
        }
    }

    /// **A party member's power, out of the stats packet** — `(now, max, kind)`
    /// in the same shape `WorldEntity::power_value` carries, so the three reads
    /// above fall through to it unchanged.
    ///
    /// `None` unless all three arrived: `UnitFrame_UpdateManaType` colours the
    /// bar off the *kind*, and answering a value with no kind paints a rogue's
    /// energy blue.
    fn party_power(&self, id: UnitId) -> Option<(u32, u32, u8)> {
        // The member's own row, or their pet's — the two carry the same three
        // fields over the same opcode and the token says which is wanted.
        let (power, max_power, kind) = match self.party_pet_row(id) {
            Some(pet) => (pet.power, pet.max_power, pet.power_type),
            None => {
                let stats = self.party_row(id)?.stats.as_ref()?;
                (stats.power, stats.max_power, stats.power_type)
            }
        };
        Some((u32::from(power?), u32::from(max_power?), kind?))
    }

    /// `UnitIsDead`. **Falls back to the roster's status byte**, which is the
    /// only thing that says a member across the zone has died — and it is what
    /// greys their frame out.
    pub fn is_dead(&self, id: UnitId) -> bool {
        match self.get(id) {
            Some(unit) => unit.dead,
            None => self.party_row(id).is_some_and(|member| member.dead()),
        }
    }

    /// `UnitIsConnected` — **false only for a party member the roster says is
    /// offline**. Every unit the world knows about is connected by construction,
    /// which is the real client's answer too.
    pub fn is_connected(&self, id: UnitId) -> bool {
        // **A pet has no roster row, so it is connected only while it is in the
        // world** — which is `UnitIsConnected`'s own answer and not a simplification:
        // that function returns 1 for anything the object manager holds, then
        // looks the guid up in the party and raid rows, and a pet is in
        // neither. So a party pet across the zone reads *not connected* and
        // `UnitFrameManaBar_Update` paints its bar grey, which is what the
        // reference draws.
        if id.owner().is_some() {
            return self.get(id).is_some();
        }
        match self.party_row(id) {
            Some(member) => member.online(),
            None => true,
        }
    }

    /// `UnitIsGhost` — the spirit has been released.
    ///
    /// **Not the same question as [`Self::is_dead`]**, and neither implies the
    /// other: a ghost's health is 1, so it reads as alive. See
    /// [`vale_protocol::play::death`].
    pub fn is_ghost(&self, id: UnitId) -> bool {
        match self.get(id) {
            Some(unit) => unit.is_ghost,
            None => self.party_row(id).is_some_and(|member| member.ghost()),
        }
    }

    /// `UnitAffectingCombat`.
    pub fn affecting_combat(&self, id: UnitId) -> bool {
        self.get(id).is_some_and(|unit| unit.in_combat)
    }

    /// **The character sheet's whole population**, or `None` for a unit whose
    /// stat block never crossed the wire — which is everyone but us. See
    /// [`vale_protocol::play::stats`], where the decode and its authority are.
    pub fn stats(&self, id: UnitId) -> Option<&vale_protocol::play::stats::UnitStats> {
        self.get(id).and_then(|unit| unit.stats.as_deref())
    }

    /// **Unspent talent points and unspent profession points**, or `None` for
    /// anybody but the player — both fields are `PRIVATE`.
    ///
    /// `UnitCharacterPoints`' pair, and the whole of what the wire says about
    /// talents: how many points are *spent* is worked back out of the known
    /// spells. See [`crate::interface::talents`].
    pub fn character_points(&self, id: UnitId) -> Option<(u32, u32)> {
        self.get(id).and_then(|unit| unit.character_points)
    }

    /// **Where the character has been**, or `None` for a unit that carries no
    /// exploration mask — which is everyone but us, on the same terms as
    /// [`Self::stats`]. See [`vale_protocol::play::explored`].
    pub fn explored(&self, id: UnitId) -> Option<&vale_protocol::play::explored::Explored> {
        self.get(id).and_then(|unit| unit.explored.as_deref())
    }

    /// **Every skill the character has**, or `None` for a unit that carries no
    /// block — which is everyone but us. See
    /// [`vale_protocol::play::skills`].
    pub fn skills(&self, id: UnitId) -> Option<&vale_protocol::play::skills::Skills> {
        self.get(id).and_then(|unit| unit.skills.as_deref())
    }

    /// **`UnitSex`'s answer**, already mapped onto the three numbers the
    /// interface uses.
    ///
    /// The mapping is a four-entry table — `[2, 3, 1, 6]` — indexed by `UNIT_FIELD_BYTES_0`'s gender byte. So male
    /// (byte 0) is **2** and female (byte 1) is **3**, which is what
    /// `GetText(key, gender)` picks the `_MALE`/`_FEMALE` variant with.
    ///
    /// **A unit that resolves to nobody answers 2, not nil** — the client's own
    /// fallback is 2.0. That
    /// matters more than it looks: `ReputationFrame_Update` calls this on line
    /// 44, *before* its loop, and a nil there takes the whole panel down before
    /// a single bar is hidden.
    pub fn sex(&self, id: UnitId) -> u32 {
        // The table's third and fourth entries are unreachable from a byte the
        // server sends — vmangos writes 0 or 1 — so they are named rather than
        // indexed: 1 is the neuter the interface calls "unknown".
        match self.get(id).and_then(|unit| unit.gender) {
            Some(0) => 2,
            Some(1) => 3,
            Some(_) => 1,
            None => 2,
        }
    }

    /// **Which faction's bar sits over the action bar**, or `None` for a unit
    /// that carries no such field — which is everyone but us, on the same terms
    /// as [`Self::explored`]. See [`crate::interface::reputation`].
    pub fn watched_faction(&self, id: UnitId) -> Option<i32> {
        self.get(id).and_then(|unit| unit.watched_faction)
    }

    /// `UnitRace` — `(localised, fileName)`, e.g. `("Night Elf", "NightElf")`.
    ///
    /// **The second is the one with a rule**, and `DressUpFrame.lua` is where
    /// the rule is legible: it compares against `"Gnome"` *and* `"GNOME"` on
    /// the same line, so 1.12's own answer is the un-spaced name in mixed case
    /// and every consumer folds case anyway.
    pub fn race(&self, id: UnitId) -> Option<(&'static str, &'static str)> {
        let (race, _) = self.get(id)?.race_class?;
        let name = vale_protocol::state::query::race_name(u32::from(race));
        (!name.is_empty()).then(|| (name, unspaced(name)))
    }

    /// `UnitClass` — `(localised, fileName)`. Both call sites in the directory
    /// (`PaperDollStatTooltip` composing `WARRIOR_STRENGTH_TOOLTIP`, and
    /// `UIOptionsFrame.lua` testing for a rogue) `strupper` the second before
    /// using it, so the case here is not load-bearing.
    pub fn class(&self, id: UnitId) -> Option<(&'static str, &'static str)> {
        let (_, class) = self.get(id)?.race_class?;
        let name = vale_protocol::state::query::class_name(u32::from(class));
        (!name.is_empty()).then_some((name, name))
    }

    /// …and the same pair as **ids**, which is what a requirement mask is tested
    /// against.
    ///
    /// `AllowableRace`/`AllowableClass` are bit masks over these numbers, so the
    /// name is no use there: an item plate has to ask "is bit `race - 1` set",
    /// and a word cannot answer that.
    pub fn race_class_ids(&self, id: UnitId) -> Option<(u32, u32)> {
        let (race, class) = self.get(id)?.race_class?;
        Some((u32::from(race), u32::from(class)))
    }

    /// `UnitIsUnit` — do two tokens name the same creature?
    ///
    /// **Compared by guid, not by entity**, which is the one that stays true: an
    /// entity is this client's handle and a guid is the server's identity, and
    /// nothing guarantees a unit keeps its entity across a stream-out and back.
    /// Two tokens that both resolve to nothing are **not** the same unit — the
    /// game answers nil there, and `TARGETSELF`'s body would otherwise target the
    /// pet whenever nothing at all was selected.
    pub fn is_unit(&self, a: UnitId, b: UnitId) -> bool {
        match (self.get(a), self.get(b)) {
            (Some(a), Some(b)) => a.guid == b.guid,
            _ => false,
        }
    }

    /// **One side of the friend-or-foe question**, as the rule wants it —
    /// [`vale_assets::tables::faction::Party`].
    ///
    /// The group flag is the only field that is not a straight read: a unit is
    /// "in the local character's group" when the roster names its guid, which
    /// is the same set the party and raid tokens are built from.
    /// The two resources friend-or-foe needs beyond the tables, for a pass
    /// that already holds this param and calls out to a function that does not
    /// — see [`Friendship`], and the cast path, which is the one caller.
    pub fn friendship(&self) -> Friendship<'_> {
        Friendship { party: &self.party, standing: &self.standing }
    }

    fn party_of(&self, id: UnitId) -> Option<vale_assets::tables::faction::Party> {
        Some(faction_party(self.get(id)?, &self.party))
    }

    /// **How one unit stands towards another** — the join `UnitIsFriend`,
    /// `UnitIsEnemy`, `UnitReaction` and `UnitCanAttack` are all decided by, on
    /// the client's own eight-rank scale.
    ///
    /// The same call Tab-targeting makes ([`super::target`]), through the same
    /// table, so the interface and the picker cannot disagree about who is a
    /// friend. `None` when either token names nothing, which is the nil every
    /// one of those four answers for an absent unit.
    ///
    /// **It is the whole cascade and not the faction table alone** — see
    /// [`vale_assets::tables::faction`], where the eleven legs are written
    /// out. A duel, a free-for-all flag, a forced reaction and the at-war bit
    /// each beat the table, and three of the four can make two units of one
    /// faction hostile.
    pub fn rank(
        &self,
        tables: &vale_assets::tables::dbc::DisplayTables,
        a: UnitId,
        b: UnitId,
    ) -> Option<vale_assets::tables::faction::Rank> {
        let (a, b) = (self.party_of(a)?, self.party_of(b)?);
        let Some(factions) = tables.factions() else {
            // No table is no opinion, which resolves to Neutral exactly as a
            // template neither side has does.
            return Some(vale_assets::tables::faction::Rank::Neutral);
        };
        Some(factions.rank(&a, &b, &self.standing.lend()))
    }

    /// …and the same answer folded to the three readings this client paints.
    pub fn reaction(
        &self,
        tables: &vale_assets::tables::dbc::DisplayTables,
        a: UnitId,
        b: UnitId,
    ) -> Option<vale_assets::tables::faction::Reaction> {
        self.rank(tables, a, b).map(Into::into)
    }

    /// `UnitCanAttack(a, b)` — **may the first swing at the second?**
    ///
    /// Not the reaction alone: the client's attack test folds in both units'
    /// `UNIT_FIELD_FLAGS` — which is what takes a non-attackable quest giver
    /// standing in a hostile camp out of the set — and, between two players, a
    /// duel, the PvP flag and the free-for-all pair. One rule, in the crate
    /// that owns it, called from both here and the picker.
    pub fn can_attack(
        &self,
        tables: &vale_assets::tables::dbc::DisplayTables,
        a: UnitId,
        b: UnitId,
    ) -> bool {
        let (Some(a), Some(b)) = (self.party_of(a), self.party_of(b)) else {
            return false;
        };
        let Some(factions) = tables.factions() else {
            return vale_assets::tables::faction::can_attack(
                b.unit_flags,
                vale_assets::tables::faction::Reaction::Neutral,
            );
        };
        factions.can_attack(&a, &b, &self.standing.lend())
    }

    /// `UnitPlayerControlled` — is there a person behind this unit?
    ///
    /// **The object type, which is the whole of the question for this client.**
    /// The real answer is `UNIT_FLAG_PLAYER_CONTROLLED`, which is also set on a
    /// pet, a charmed creature and a mind-controlled one; this client models
    /// none of those, so the two agree everywhere it can currently look, and the
    /// day a pet exists this is the line to change.
    pub fn player_controlled(&self, id: UnitId) -> bool {
        self.get(id)
            .is_some_and(|unit| unit.kind == vale_protocol::state::update::ObjectType::Player)
    }

    /// `UnitIsPVP` — is this unit flagged for open combat?
    ///
    /// **`UNIT_FIELD_FLAGS` bit 12**, which is the field the tooltip builder
    /// reads: `(UNIT_FIELD_FLAGS >> 12) & 1`. Its player branch also takes `PLAYER_FLAGS`' own bit 8, so a
    /// player who is flagged but whose unit bit has not caught up still shows
    /// the line; that second read is deliberately not made here, because this
    /// client parses `PLAYER_FLAGS` for exactly one bit (the release timer) and
    /// a second reading of the same word is a fact to establish rather than to
    /// assume.
    pub fn is_pvp(&self, id: UnitId) -> bool {
        self.get(id)
            .is_some_and(|unit| unit.unit_flags & UNIT_FLAG_PVP != 0)
    }

    /// `UnitCreatureType` — `"Humanoid"`, `"Beast"`, and `None` for a player,
    /// which is the game's own answer (`UnitCreatureType("player")` is nil).
    pub fn creature_type(&self, id: UnitId) -> Option<&'static str> {
        let unit = self.get(id)?;
        let name = vale_protocol::state::query::creature_type_name(unit.creature_type);
        (!name.is_empty()).then_some(name)
    }

    /// **Everything the unit plate is made of, in one pass.**
    ///
    /// One call rather than eleven because the plate is composed inside a single
    /// `GameTooltip:SetUnit` and every cell of it is about the same unit — see
    /// [`crate::lua::widgets::tooltip`], where the composition law is, and
    /// [`UnitTip`] for what each field is.
    pub fn unit_tip(
        &self,
        tables: Option<&vale_assets::tables::dbc::DisplayTables>,
        id: UnitId,
    ) -> Option<UnitTip> {
        let Some(unit) = self.get(id) else {
            // **A party member with no entity still has a plate**, built out of
            // the roster and whatever stats have arrived — which is what the
            // report about a blank frame was half about. Race, class and
            // classification are absent because nothing on the wire carries
            // them for a member out of range, and the level/class line falls
            // through to `TOOLTIP_UNIT_LEVEL` on its own.
            let member = self.party_row(id)?;
            return Some(UnitTip {
                name: member.name.clone(),
                sub_name: String::new(),
                level: self.level(id).max(0),
                race: None,
                class: None,
                creature_type: None,
                classification: "",
                player_controlled: true,
                pvp: member.pvp(),
                dead: member.dead(),
                health: member
                    .stats
                    .as_ref()
                    .and_then(|s| Some((u32::from(s.health?), u32::from(s.max_health?)))),
                zone: String::new(),
            });
        };
        let player = unit.kind == vale_protocol::state::update::ObjectType::Player;
        let level_shown = self.level_shown(tables, id);
        Some(UnitTip {
            name: unit.name.clone(),
            sub_name: unit.sub_name.clone(),
            // **The plate's own `??`**, which is the same rule the frame's is
            // — see [`Units::level_shown`]. `0` rather than `-1` because the
            // builder treats any level `<= 0` as unknown and this side has
            // always spelt its unknown as zero; both compose the same cell.
            level: level_shown.max(0),
            race: self.race(id).map(|(localised, _)| localised),
            class: self.class(id).map(|(localised, _)| localised),
            creature_type: self.creature_type(id),
            classification: vale_protocol::state::query::classification_key(unit.classification),
            player_controlled: player,
            pvp: unit.unit_flags & UNIT_FLAG_PVP != 0,
            dead: unit.dead,
            health: unit.health_value,
            // **Filled in by the caller**, which is the only side holding
            // `AreaTable` and the only side that knows where *we* are standing.
            // See [`UnitTip::zone`].
            zone: String::new(),
        })
    }

    /// **Which zone a party member is standing in**, as an `AreaTable` id —
    /// `None` for every other unit and for one no stats packet has arrived for.
    ///
    /// The only thing on the wire that knows: `SMSG_PARTY_MEMBER_STATS` carries
    /// it, and a member across the world is in no other list this client keeps.
    pub fn party_zone(&self, id: UnitId) -> Option<u32> {
        let zone = self.party_row(id)?.stats.as_ref()?.zone?;
        (zone != 0).then_some(u32::from(zone))
    }
}

/// **`worldboss`** — the `creature_template.rank` this client draws as `(Boss)`
/// and the reference reports no level for, at any level and any reaction.
///
/// The same 3 `UnitClassification` maps to `"worldboss"` and
/// `vale_protocol::state::query::classification_key` maps to `"BOSS"`;
/// named here because [`Units::level_shown`] compares against it and a bare 3
/// beside a `classification` field reads as one of the other four ranks just as
/// easily.
///
/// **It is 0 until the creature query lands.** `WorldEntity::classification` is
/// filled from `SMSG_CREATURE_QUERY_RESPONSE`, so a boss states its real level
/// for the round trip and then goes to `??`. That is a fault of the order the
/// wire arrives in rather than of this rule, and it is the same window every
/// other template-derived cell of the plate has.
pub const BOSS_CLASSIFICATION: u32 = 3;

/// `UNIT_FLAG_PVP` — vmangos' own `UNIT_FLAG_PVP = 0x00001000`, and the bit the
/// client tests to draw the "PvP" line and to pick the ring's colour.
pub const UNIT_FLAG_PVP: u32 = 0x0000_1000;

/// **What `GameTooltip:SetUnit` draws**, resolved once.
///
/// Deliberately words and numbers rather than sentences: this side answers *what
/// the unit is* and [`crate::lua::widgets::tooltip`] composes it into the game's own
/// format strings, which is the same split every other plate in this client
/// takes ([`SpellTip`]). A cell nothing can answer is `None`/empty and the
/// composition drops it, which is how the four `TOOLTIP_UNIT_LEVEL*` formats
/// choose between themselves.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitTip {
    pub name: String,
    /// `<Innkeeper>` — a creature's template tag, empty for a player.
    pub sub_name: String,
    /// `0` for a unit whose level never crossed the wire, which the builder
    /// turns into `"??"`, as it does any level `<= 0`.
    pub level: i32,
    /// A player's race and class, both `None` for a creature.
    pub race: Option<&'static str>,
    pub class: Option<&'static str>,
    /// …and a creature's `CreatureType.dbc` word, `None` for a player.
    pub creature_type: Option<&'static str>,
    /// The `GlobalStrings.lua` key for the classification cell, `""` for none.
    pub classification: &'static str,
    pub player_controlled: bool,
    pub pvp: bool,
    pub dead: bool,
    /// `(current, maximum)` for the plate's own status bar, `None` for anything
    /// that never had health.
    pub health: Option<(u32, u32)>,
    /// **Where a party member is, when it is not where we are** — the bare zone
    /// line the reference adds to a group mate's plate, and empty for every unit
    /// that should not have one.
    ///
    /// Empty rather than `Option` because the composition's test is "is there a
    /// line to draw", and because the *decision* is made before it gets here:
    /// [`Units::party_zone`] answers the id and the caller resolves it and drops
    /// it when it matches the character's own zone. A member in the same zone
    /// says nothing, which is the whole information in the line.
    pub zone: String,
}

/// A race's file name from its localised one — `UnitRace`'s second answer.
///
/// **Night Elf is the only one of the eight with a space in it**, so this is a
/// one-arm match rather than a second table to keep in step with
/// [`vale_protocol::state::query::race_name`].
fn unspaced(name: &'static str) -> &'static str {
    match name {
        "Night Elf" => "NightElf",
        other => other,
    }
}

/// `GetTime` — **the interface's clock**, in seconds since the client started.
///
/// A float, monotonic, and *not* wall-clock time — `date()` is the game's
/// function for that. Half of FrameXML is written against it:
/// `CastingBarFrame_OnUpdate` scrubs the bar between `this.startTime` and
/// `this.maxValue`, both of which are `GetTime()` values, and every
/// `CooldownFrame` is `(start, duration)` in the same base.
///
/// **The base being one base is the whole point of routing it through here.** A
/// second time origin invented somewhere else would produce a cast bar that is
/// full or empty and never in between, with no error and nothing in the log — so
/// [`get_action_cooldown`] takes its `now` from this and nothing else.
///
/// Bevy's `Time::elapsed` is already seconds since the app started, so this is a
/// rename. That is deliberate: the rename is the fact.
pub fn get_time(time: &Time) -> f64 {
    time.elapsed_secs_f64()
}

/// `GetQuestGreenRange()` — **how far below you a thing stays green**, in levels.
///
/// One number, about the player and nothing else, and the only input to the
/// bottom of `GetDifficultyColor`'s cascade: `QuestLogFrame.lua` colours a level
/// red / orange / yellow off the difference outright, and then asks *this* to
/// decide between green and grey (`if ( -levelDiff <= GetQuestGreenRange() )`).
/// So it is what separates "worth killing" from "not worth killing" on the
/// target frame's level number and on every quest title in the log.
///
/// **A table, not a formula, and it is the client's own**: the player's level,
/// divided by ten and clamped to 19, indexes twenty entries, and those twenty
/// are [`QUEST_GREEN_RANGE`]. That matters because the
/// obvious reconstruction is the *server's* — vmangos derives a grey level
/// arithmetically in `MaNGOS::XP::GetGrayLevel` — and the two do not agree: at
/// level 60 the server's rule gives 9 and the client's table gives **7**. This
/// number never crosses the wire, so the client's is the only one that decides
/// what is on screen.
///
/// **No player is `0`**, which is the client's own early return and not a
/// guess: with no character there is nothing a
/// level can be measured against, and green collapses onto grey.
pub fn quest_green_range(player_level: i32) -> i32 {
    if player_level <= 0 {
        return 0;
    }
    let index = (player_level / 10).clamp(0, QUEST_GREEN_RANGE.len() as i32 - 1) as usize;
    QUEST_GREEN_RANGE[index]
}

/// The client's twenty entries, indexed by `playerLevel / 10`.
///
/// Only the first seven are reachable in 1.12 — the level cap is 60 — and the
/// rest are transcribed rather than trimmed, because a table read at an index
/// the client clamps is a table whose length is part of the rule.
const QUEST_GREEN_RANGE: [i32; 20] = [
    4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 12, 12, 12, 12, 12, 12, 12,
];

/// `HasAction` — is there anything in this slot?
///
/// Slots are **one-based** here and everywhere the game touches them:
/// `ActionButtonDown(1)` is the first button, and `ACTIONBAR_SLOT_CHANGED`'s
/// `arg1 == 0` is free to mean "all of them" precisely because no real slot is 0.
pub fn has_action(bar: &ActionBar, slot: u8) -> bool {
    action(bar, slot).is_some()
}

/// The slot's contents, or `None`. The one-based-to-zero-based conversion, in one
/// place.
pub fn action(bar: &ActionBar, slot: u8) -> Option<&Slot> {
    let index = usize::from(slot.checked_sub(1)?);
    bar.slots.get(index)?.as_ref()
}

/// `GetActionText` — what to write on the button, **and for a spell that is
/// nothing**. The real client answers only a *macro's* name here; a spell or
/// an item answers nil, and `ActionButton_Update` hands the answer straight to
/// the button's Name font string. Answering the spell's name — the old
/// fallback from before the bar drew icons — printed a label over every icon,
/// twelve overlapping names along the bottom of the screen.
pub fn get_action_text(bar: &ActionBar, slot: u8) -> Option<String> {
    action(bar, slot)
        .filter(|held| held.kind == vale_protocol::play::spells::action_kind::MACRO)
        .map(Slot::label)
}

/// `GetActionTexture` — the icon path out of `SpellIcon.dbc`.
///
/// **A spell's only.** An item slot's icon is its prototype's and wants the
/// inventory as well as the bar, so it is answered one layer up where both are
/// in hand — see `lua::api`'s `Live::action_texture` and [`action_item`].
pub fn get_action_texture(bar: &ActionBar, slot: u8) -> Option<&str> {
    action(bar, slot)
        .and_then(|slot| slot.spell.as_ref())
        .map(|info| info.icon.as_str())
        .filter(|icon| !icon.is_empty())
}

/// **The item entry in a slot**, or `None` for anything that is not an item.
///
/// The one thing `SMSG_ACTION_BUTTONS` says about an item slot, and the input to
/// every other question about one: what it looks like, how many are left,
/// whether it is worn, and what pressing it does — see
/// [`super::items::UseCarriedItem`]. Kept here rather than inlined at four call
/// sites because the test is the *kind* byte and not the presence of a
/// `SpellInfo`: a spell `Spell.dbc` does not carry has no info either.
pub fn action_item(bar: &ActionBar, slot: u8) -> Option<u32> {
    action(bar, slot)
        .filter(|held| held.kind == vale_protocol::play::spells::action_kind::ITEM)
        .map(|held| held.action)
}

/// `IsAttackAction` — is this the auto-attack toggle rather than a cast?
///
/// `ActionButton.lua` reads it to decide whether the button should *flash* while
/// in combat, which is the melee player's only feedback that the swing took.
pub fn is_attack_action(bar: &ActionBar, slot: u8) -> bool {
    action(bar, slot).is_some_and(Slot::is_auto_attack)
}

/// `GetActionCooldown` — the game's own `(start, duration, enable)`.
///
/// `start` is a [`get_time`] value and `duration` is the whole length, which is
/// exactly what `CooldownFrame_SetTimer(cooldown, start, duration, enable)` takes
/// in `ActionButton_UpdateCooldown`:
///
/// ```lua
/// local start, duration, enable = GetActionCooldown(ActionButton_GetPagedID(this));
/// ```
///
/// **This used to return `(remaining, duration)`** because there was no clock to
/// express a start in; there is one now, and the conversion is the one that
/// comment stated — `start = now - (duration - remaining)`. Both halves are
/// derived from the same instant, so the pair is self-consistent even though
/// `Cooldowns` counts in `Instant`s and `now` comes from the frame clock.
///
/// A slot with nothing on cooldown answers `(0, 0, 1)`, which is the game's own
/// "no timer" and is what a `CooldownFrame` hides on — not `nil`.
///
/// **`enable` is always 1 here**, and that is a stated simplification. In the real
/// client it is 0 for an action whose cooldown exists but should not be *drawn* —
/// the case is an enchant or an item proc — and this client has neither on a bar.
/// Erring towards drawing is the visible direction, which is the one to err in.
pub fn get_action_cooldown(
    bar: &ActionBar,
    cooldowns: &Cooldowns,
    slot: u8,
    now: f64,
    // **The item's own on-use spell**, for an item slot — see the branch
    // below. `None` for a spell slot, and for an item whose template has not
    // arrived yet.
    item_spell: Option<&vale_assets::tables::spellbook::SpellInfo>,
) -> (f64, f64, bool) {
    let idle = (0.0, 0.0, true);
    // **An item's swirl is its on-use spell's cooldown**, which is where a
    // potion's two minutes actually live: the timer is on the spell's *category*
    // (every healing potion shares one), not on the item entry. Without this an
    // item slot answered `(0, 0, 1)` for ever — no swirl at all — so a potion
    // just drunk looked ready, and pressing it again sent a `CMSG_USE_ITEM` the
    // server silently refused. That is the "unreactive" half of the report.
    match slot_spell(bar, slot, item_spell) {
        Some(spell) => cooldown_of(cooldowns, spell, now),
        None => idle,
    }
}

/// **One spell's `(start, duration, enable)`**, in `now`'s base — the shape both
/// `GetActionCooldown` and `GetSpellCooldown` answer in, because both are asking
/// the same question about the same three clocks.
///
/// Shared rather than copied: the arithmetic that turns "how long is left" into
/// "when did it start" is the part a second copy would get subtly wrong, and the
/// symptom would be a cooldown swirl in the spellbook running at a different
/// rate from the one on the bar.
pub fn cooldown_of(
    cooldowns: &Cooldowns,
    spell: &vale_assets::tables::spellbook::SpellInfo,
    now: f64,
) -> (f64, f64, bool) {
    match cooldowns.remaining(spell) {
        Some((remaining, duration)) => {
            let duration = f64::from(duration);
            (now - (duration - f64::from(remaining)), duration, true)
        }
        None => (0.0, 0.0, true),
    }
}

/// **Everything the client knows about what a slot would cast** — the data
/// half of `GameTooltip:SetAction`, which composes it into lines with the
/// game's own format strings (see `lua::tooltip`).
///
/// Data rather than text on purpose: the formats (`MANA_COST`, `SPELL_RANGE`)
/// live in the Lua environment `GlobalStrings.lua` filled, so the composition
/// belongs on that side of the split and the *facts* belong here, where
/// `vale spellbook` can check them with no interpreter running.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpellTip {
    pub name: String,
    /// "Rank 1", or empty — the gray right column of the name line.
    pub rank: String,
    /// **`(rank, maxRank)` for a talent**, which is a different line from
    /// [`Self::rank`] and replaces it.
    ///
    /// The reference composes the talent plate through the *same* function the
    /// spellbook's hover uses, exactly as `SetSpell` and `SetPetAction` do, with the `TalentRec` passed alongside
    /// the spell id, and that record is what adds `TOOLTIP_TALENT_RANK`
    /// ("Rank %d/%d") as a line of its own under the name.
    ///
    /// So it is here rather than in a plate of its own: one composer, one place
    /// the layout is decided. `None` for every plate that is not a talent's,
    /// which is all but one.
    ///
    /// **It replaces the grey cell rather than joining it**, and that is the
    /// report: `Spell.dbc`'s `Rank` column for a talent's rank-5 spell is the
    /// string "Rank 5", so a plate that printed both said "Rank 5" on the right
    /// and nothing about the five — which reads as a talent with no maximum.
    pub talent_rank: Option<(u32, u32)>,
    /// `enum Powers`: 0 mana, 1 rage, 3 energy — and the cost **as displayed**,
    /// so a rage cost has already had the wire's tenths divided out.
    pub power_type: u32,
    pub power_cost: u32,
    pub range_yards: f32,
    pub cast_time_ms: u32,
    /// The spell's own recovery or its category's, whichever is longer — the
    /// number the tooltip's "cooldown" cell prints.
    pub cooldown_ms: u32,
    /// **The sentence, with its `$` variables already substituted** — see
    /// [`vale_assets::tables::spelltext`]. Empty for a spell whose row carries none,
    /// which is most of them.
    ///
    /// Substituted here rather than in `lua::tooltip` because it needs the
    /// caster's level and the spell catalog, neither of which the interface
    /// side holds — and because it is a decision about text taken with no
    /// window, which is this project's own test for which side of the split
    /// something belongs on.
    pub description: String,
    /// What the cast consumes, already named: `[("Rune of Teleportation", 1)]`.
    ///
    /// **Names, not entries, and a name this client does not have yet is
    /// missing from this list rather than printed as a number.** `Spell.dbc`
    /// holds item entries and `Item.dbc` is not in the archives, so the name is
    /// a `CMSG_ITEM_QUERY_SINGLE` away — the first hover on a cold cache asks
    /// and shows nothing, and the next one shows the reagent. That is the real
    /// client's own behaviour with an empty item cache.
    pub reagents: Vec<(String, u32)>,
}

/// **What the sentence and the reagents need that a spell row does not carry.**
///
/// The caster's level (the effect values scale on it), the catalog (a `$<id>`
/// token quotes another spell's row), and whatever item names have arrived. A
/// parameter object rather than three arguments because every caller has all
/// three or none: the audit's stub has none and passes [`Self::none`].
pub struct TipContext<'a> {
    pub level: u32,
    /// The player's own `ChrRaces`/`ChrClasses` ids, or `(0, 0)` before there is
    /// a player. **What an item's requirement lines are tested against**, and
    /// the reason they are ids rather than words: `AllowableRace` and
    /// `AllowableClass` are bit masks over these numbers.
    ///
    /// Zero matches nothing, so a plate composed with no player draws its
    /// requirement lines in the colour of an *unmet* requirement — which is the
    /// safe direction and is what the headless harness sees.
    pub race: u32,
    pub class: u32,
    pub catalog: Option<&'a vale_assets::tables::spellbook::Spells>,
    /// `entry -> name`, as far as `CMSG_ITEM_QUERY_SINGLE` has answered.
    pub item_names: &'a dyn Fn(u32) -> Option<String>,
    /// **Where the hearthstone points**, for the `$z` in its own sentence and
    /// in Astral Recall's — see
    /// [`vale_assets::tables::spelltext`], which is where the token's
    /// meaning is written down.
    ///
    /// A resolved name rather than an area id, because the crate that
    /// substitutes it holds no DBC and no `GlobalStrings.lua`. `None` is a
    /// character whose bind point has not arrived, which drops the token; the
    /// caller that has the strings puts `HOME_INN` here instead, as the
    /// reference does.
    pub home: Option<String>,
}

impl TipContext<'_> {
    /// The empty context: level 1, no catalog, no names. Used by the headless
    /// harnesses, and the reason a description with a `$<id>` in it comes back
    /// missing that clause rather than failing.
    pub fn none() -> TipContext<'static> {
        TipContext {
            level: 1,
            race: 0,
            class: 0,
            catalog: None,
            item_names: &|_| None,
            home: None,
        }
    }
}

/// **An item's plate, as values** — the same division [`SpellTip`] makes.
///
/// Everything here is a number or a resolved string; not one field is a
/// sentence composed here. The *words* come out of `GlobalStrings.lua` in
/// [`crate::lua::widgets::tooltip`], read from the live globals table exactly as the
/// real client reads them, so a key the shipped file does not carry draws as
/// nothing rather than as something this repo made up.
///
/// **The line order is a reconstruction and is marked as one** — see
/// `lua::tooltip::item_lines`. What *is* measured is every value below: each is
/// a named column of `SMSG_ITEM_QUERY_SINGLE_RESPONSE` or of the update field
/// block the slot came from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemTip {
    pub name: String,
    /// `ITEM_QUALITY_*` 0..6 — the name's colour, through the interface's own
    /// `GetItemQualityColor`.
    pub quality: u32,
    /// Whether the *stack in hand* is already bound, as opposed to what the
    /// prototype says it will do. `ITEM_FIELD_FLAGS` bit 0, so it is only ever
    /// true for something actually carried.
    pub soulbound: bool,
    /// `ItemPrototype::Bonding`: 1 on pickup, 2 on equip, 3 on use, 4 quest.
    pub bonding: u32,
    /// `ITEM_FLAG_CONJURED`, bit 0.
    pub conjured: bool,
    /// The number of slots, if this is a bag — which turns the type line into
    /// `CONTAINER_SLOTS` ("%d Slot %s").
    pub container_slots: u32,
    /// `ItemClass.dbc`'s and `ItemSubClass.dbc`'s words, already looked up.
    pub class_name: String,
    pub subclass_name: String,
    /// `INVTYPE_*` — the *number*, because the word is a `GlobalStrings` key
    /// and this side does not read that file.
    pub inventory_type: u32,
    /// Every damage entry with a non-zero maximum, as `(min, max, school)`.
    pub damage: Vec<(f32, f32, u32)>,
    /// Swing time in seconds, 0 for anything that is not a weapon.
    pub speed: f32,
    /// …and the damage per second the plate prints under it, which the client
    /// computes rather than reads.
    pub dps: f32,
    pub armor: i32,
    pub block: u32,
    /// The non-zero `(ITEM_MOD_*, value)` pairs, in table order.
    pub stats: Vec<(u32, i32)>,
    /// Holy..arcane, in `RESISTANCE1_NAME`..`RESISTANCE6_NAME` order.
    pub resistances: [i32; 6],
    /// `(current, maximum)`, or `None` for something that cannot be damaged.
    pub durability: Option<(u32, u32)>,
    /// `ItemPrototype::RequiredLevel`. **The line is drawn only above 1**, so
    /// an item requiring level 1 says nothing rather than "Requires Level 1".
    pub required_level: u32,
    /// …and whether the character *meets* it, which is the only thing that
    /// decides the colour. The client compares the requirement against the
    /// player's own level and picks between two colour globals.
    pub level_met: bool,
    /// **The races allowed, in `ChrRaces` order, or empty when everyone is.**
    ///
    /// The client walks the race table twice: once to find out whether *any*
    /// bit is clear — and if none is, the line is skipped outright,
    /// which is why 99% of items have no "Races:" line — and once to build the
    /// list. Empty here means the mask covers everyone, not that nobody may use
    /// it.
    pub races_allowed: Vec<&'static str>,
    /// …and whether the character's own race is among them.
    pub race_allowed: bool,
    /// The same pair for `AllowableClass`.
    pub classes_allowed: Vec<&'static str>,
    pub class_allowed: bool,
    /// Each of the item's spells that has one, as `(trigger, sentence)` — the
    /// trigger picking between `ITEM_SPELL_TRIGGER_ONUSE`, `ONEQUIP` and
    /// `ONPROC`, and the sentence already substituted by
    /// [`vale_assets::tables::spelltext`].
    pub spells: Vec<(u32, String)>,
    /// The flavour text, in gold and quoted — the item's own, not a spell's.
    pub description: String,
    /// `<Right Click to Read>`.
    pub readable: bool,
    /// `<Right Click to Open>`, **and it takes precedence over the line above**
    /// — the reference picks one key and adds one line. See
    /// `ItemInfo::says_right_click_to_open`, which is where the lock gate is:
    /// a strongbox nobody has picked promises nothing.
    pub openable: bool,
    /// `This Item Begins a Quest`.
    pub starts_quest: bool,
}

/// Compose one from a template and, when there is one, the stack in hand.
///
/// `carried` is `None` for a plate reached from a link or a merchant, where
/// there is a prototype and no object — the two fields it contributes are the
/// current durability and whether *this* copy is already soulbound.
pub fn item_tip(
    info: &vale_protocol::state::query::ItemInfo,
    carried: Option<&vale_protocol::play::items::ItemSlot>,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    context: &TipContext,
) -> ItemTip {
    let words = tables.map(vale_assets::tables::dbc::DisplayTables::item_tables);
    let damage: Vec<(f32, f32, u32)> = info
        .damage
        .iter()
        .filter(|d| d.max > 0.0)
        .map(|d| (d.min, d.max, d.school))
        .collect();
    // **Summed over every school, which is what the real plate does**: a
    // weapon with a physical entry and a fire entry prints one dps line
    // covering both. Guarded on the delay rather than on the damage, since a
    // zero swing time is what a non-weapon has.
    let speed = info.delay as f32 / 1000.0;
    let dps = if speed > 0.0 {
        let total: f32 = damage.iter().map(|(min, max, _)| (min + max) / 2.0).sum();
        total / speed
    } else {
        0.0
    };
    ItemTip {
        name: info.name.clone(),
        quality: info.quality,
        soulbound: carried.is_some_and(|c| c.soulbound()),
        bonding: info.bonding,
        // `ITEM_FLAG_CONJURED`, bit 0 of the flag word.
        conjured: info.flags & 0x1 != 0,
        container_slots: info.container_slots,
        class_name: words.map(|w| w.class_name(info.class)).unwrap_or_default().to_string(),
        subclass_name: words
            .map(|w| w.subclass_name(info.class, info.subclass))
            .unwrap_or_default()
            .to_string(),
        inventory_type: info.inventory_type,
        damage,
        speed,
        dps,
        armor: info.armor,
        block: info.block,
        stats: info
            .stats
            .iter()
            .filter(|s| s.value != 0)
            .map(|s| (s.kind, s.value))
            .collect(),
        resistances: info.resistances,
        // **The prototype's maximum and the object's current**, which is the
        // one number a template alone cannot answer — an unworn copy of a sword
        // and the one in your hand have different durability and the same row.
        durability: (info.max_durability > 0).then(|| {
            (
                carried.map_or(info.max_durability, |c| c.durability),
                info.max_durability,
            )
        }),
        required_level: info.required_level,
        // **The line is coloured by whether it is met, and this is the only
        // place that decides.** `>=` rather than `>`: the item wants that level
        // and having it is enough.
        level_met: context.level >= info.required_level,
        races_allowed: allowed(
            info.allowable_race,
            PLAYABLE_RACES,
            vale_protocol::state::query::race_name,
        ),
        race_allowed: allows(info.allowable_race, context.race),
        classes_allowed: allowed(
            info.allowable_class,
            PLAYABLE_CLASSES,
            vale_protocol::state::query::class_name,
        ),
        class_allowed: allows(info.allowable_class, context.class),
        spells: info
            .spells
            .iter()
            .filter(|s| s.spell_id != 0)
            .map(|s| (s.trigger, item_spell_text(s.spell_id, context)))
            .collect(),
        description: info.description.clone(),
        readable: info.is_readable(),
        openable: info.says_right_click_to_open(
            carried.is_some_and(vale_protocol::play::items::ItemSlot::unlocked),
            carried.is_some_and(vale_protocol::play::items::ItemSlot::wrapped),
        ),
        starts_quest: info.start_quest != 0,
    }
}

/// The `ChrRaces` ids the 1.12 client will list, in the order it walks them.
///
/// The client iterates the whole table and skips a row whose flag bit 0 is set,
/// which is what keeps the unplayable
/// rows out of a "Races:" line. That set is fixed for 5875 and is the same eight
/// [`vale_protocol::state::query::race_name`] names.
const PLAYABLE_RACES: [u32; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

/// …and the nine playable classes, which is the `ChrClasses` walk with 6 (Death
/// Knight) and 10 absent — see [`vale_protocol::state::query::class_name`], which
/// answers nothing for both.
const PLAYABLE_CLASSES: [u32; 9] = [1, 2, 3, 4, 5, 7, 8, 9, 11];

/// **Which of a mask's members to list, or nothing at all when it covers
/// everyone.**
///
/// The client's first pass over each table is looking for
/// a member whose bit is *clear*, and a mask with no such member draws no line —
/// which is why an ordinary sword has no "Classes:" row despite matching every
/// class. Doing it the other way round (list the members, hide the line when the
/// list is full) gives the same answer for these two tables and stops being the
/// client's rule the moment a row is added.
///
/// The namer is passed in rather than chosen from the ids, because **the two id
/// spaces overlap**: 1 is Human and 1 is Warrior, so a shared lookup that tried
/// races first would list a warrior-only sword as usable by humans.
fn allowed<const N: usize>(
    mask: i32,
    members: [u32; N],
    name_of: fn(u32) -> &'static str,
) -> Vec<&'static str> {
    if members.iter().all(|&id| allows(mask, id)) {
        return Vec::new();
    }
    members
        .iter()
        .filter(|&&id| allows(mask, id))
        .map(|&id| name_of(id))
        .filter(|name| !name.is_empty())
        .collect()
}

/// Does an `AllowableRace`/`AllowableClass` mask admit this id? Bit `id - 1`,
/// as the client tests it.
///
/// **Id 0 is admitted by nothing**, which is what makes a plate composed with no
/// player draw its requirements as unmet rather than as satisfied.
fn allows(mask: i32, id: u32) -> bool {
    if id == 0 || id > 32 {
        return false;
    }
    mask as u32 & (1 << (id - 1)) != 0
}

/// The sentence an item's spell prints — the spell's own description, through
/// the same substitution a spellbook plate uses.
///
/// Falls back to the spell's **name** when the row has no description, and to
/// the empty string when the catalogue is not loaded at all. An "Use:" line
/// with nothing after it would be worse than no line, and the caller drops an
/// empty one.
fn item_spell_text(spell_id: u32, context: &TipContext) -> String {
    let Some(catalog) = context.catalog else {
        return String::new();
    };
    let Some(info) = catalog.info(spell_id) else {
        return String::new();
    };
    let described = vale_assets::tables::spelltext::describe(
        &info,
        context.level,
        context.catalog,
        context.home.as_deref(),
    );
    if described.is_empty() {
        info.name.clone()
    } else {
        described
    }
}

/// `GameTooltip:SetAction`'s answer for a slot, or `None` for one with nothing
/// to say — an empty slot, or an item/unresolved action whose name this client
/// does not know. `None` is what hides the plate, so the degradation is "no
/// tooltip" rather than a composed placeholder.
pub fn action_tooltip(bar: &ActionBar, slot: u8, context: &TipContext) -> Option<SpellTip> {
    Some(spell_tip(action(bar, slot)?.spell.as_ref()?, context))
}

/// The same plate for a spell reached any other way — a spellbook row, a link.
///
/// One copy, so a spell hovered on the bar and the same spell hovered in the
/// book cannot print different numbers. The display conversion on the cost is
/// the reason this is a function and not four field copies: rage arrives in
/// tenths and is shown in points.
pub fn spell_tip(
    info: &vale_assets::tables::spellbook::SpellInfo,
    context: &TipContext,
) -> SpellTip {
    SpellTip {
        name: info.name.clone(),
        rank: info.rank.clone(),
        // Filled by the talent panel alone; see the field.
        talent_rank: None,
        power_type: info.power_type,
        power_cost: power_type::display(info.power_type as u8, info.power_cost),
        range_yards: info.range_yards,
        cast_time_ms: info.cast_time_ms,
        cooldown_ms: info.recovery_ms.max(info.category_recovery_ms),
        description: vale_assets::tables::spelltext::describe(
            info,
            context.level,
            context.catalog,
            context.home.as_deref(),
        ),
        reagents: info
            .reagents
            .iter()
            .filter_map(|(entry, count)| Some(((context.item_names)(*entry)?, *count)))
            .collect(),
    }
}

/// **What the caster brings to a cast** — the half of
/// [`vale_assets::tables::spellbook::CastConditions`] that is about *us*,
/// measured once here for every door a cast can leave by.
///
/// There are two doors and they used to disagree. A spell pressed on the bar
/// went through `cast_known_spell`, which built these conditions inline and
/// ran `check_cast`; an *item* used from a bag or from the bar went through
/// `character::items::use_item`, which had a cooldown refusal of its own and
/// nothing else. Every mount in 1.12 is an item, so the moving refusal landed
/// for Summon Warhorse and not for Brown Horse — which is "pre-cast checks
/// still do not happen for a mount", reported a second time after the rule
/// was in. One function, both callers, and a new condition cannot be added to
/// one door and forgotten at the other.
///
/// `distance` and `target_dead` are the caller's, because only it knows what
/// the cast was aimed at; `None` for either asks nothing, which errs towards
/// sending. `world` is the three facts the caster's own snapshot does not carry
/// — see [`CastWorld`].
pub fn caster_conditions(
    me: &WorldEntity,
    info: &vale_assets::tables::spellbook::SpellInfo,
    distance: Option<f32>,
    target_dead: Option<bool>,
    world: CastWorld<'_>,
) -> vale_assets::tables::spellbook::CastConditions {
    vale_assets::tables::spellbook::CastConditions {
        power: me
            .power_value
            .filter(|(_, _, kind)| u32::from(*kind) == info.power_type)
            .map(|(now, _, _)| now),
        caster_dead: me.dead,
        distance,
        target_dead,
        // **The flags the *client* is running on**, which for the local player
        // are the mover's own rather than the last thing the server said — the
        // press is judged against the movement the player can see. See
        // [`vale_assets::tables::spellbook::SpellInfo::refused_while_moving`].
        move_flags: me.move_flags,
        // …and the caster-state chain's own inputs, every one of them a field
        // the server already sends. See `book::check_cast`, which is the rule.
        unit_flags: me.unit_flags,
        // **A charm by somebody else**, which is what the client compares: the
        // field holding the caster's own guid is a spell they cast on
        // themselves and is not a refusal.
        charmed: me.charmed_by.is_some_and(|who| who != me.guid),
        mounted: me.mounted,
        // Anything but standing — the innkeeper's stool, a chair, the ground.
        sitting: me.stand_state != STAND_STATE_STANDING,
        stealthed: me.vis_flags & VIS_FLAG_CREEP != 0,
        // **Sheath state `0` is "nothing drawn"**; 1 is melee and 2 is ranged.
        sheathed: me.sheath_state == 0,
        aura_state: me.aura_state,
        daytime: world.daytime,
        outdoors: world.outdoors,
        // **Asked only when the spell names a faction**, which on the shipped
        // table is one disabled row — see `book::spell_fields::MIN_FACTION_ID`.
        // The lookup is a scan of the character's own standing list, so it is
        // worth not doing 22,359 times out of 22,360.
        // **The talented cost and range**, which the server will charge and
        // measure and this client must therefore test against — see
        // `crate::world::spellmods`. `None` when the character has no
        // modifier for this spell at all, which is the whole table for a
        // character with no talents and is why the empty case costs nothing.
        cost_override: world.mods.moved(
            vale_protocol::play::spells::spell_mod_op::COST,
            info.spell_family_flags,
            info.power_cost,
        ),
        range_override: world.mods.moved_yards(info.spell_family_flags, info.range_yards),
        reputation_rank: (info.min_faction_id != 0)
            .then(|| {
                world
                    .standing
                    .standings
                    .iter()
                    .find(|(faction, _)| *faction == info.min_faction_id)
                    .map(|(_, state)| state.rank as u8)
            })
            .flatten(),
    }
}

/// `UNIT_FIELD_BYTES_1` byte 0 — the one value that is not sitting on
/// something.
const STAND_STATE_STANDING: u8 = 0;

/// `UNIT_FIELD_BYTES_1` byte 3 bit 1, `UNIT_BYTE1_FLAG_CREEP` — the caster is
/// sneaking. See [`WorldEntity::vis_flags`], where the byte's other two bits are.
const VIS_FLAG_CREEP: u8 = 0x02;

/// **The three facts a cast is judged against that are not on the caster's own
/// snapshot** — the clock, the sky overhead and the character's standing list.
///
/// One bundle rather than three arguments, on [`Friendship`]'s own reasoning:
/// the cast path threads this through two doors, and a run of `&Res` arguments
/// repeated down a chain is the shape that gets one of them dropped. Both
/// `Option`s are "the caller has no answer", which skips the pair of refusals
/// each decides rather than guessing at one of them.
#[derive(Clone, Copy)]
pub struct CastWorld<'a> {
    pub daytime: Option<bool>,
    pub outdoors: Option<bool>,
    pub standing: &'a crate::interface::reputation::PlayerStanding,
    /// …and what the character's talents do to the two numbers a refusal is
    /// decided on — see [`crate::world::spellmods`].
    pub mods: &'a crate::world::spellmods::SpellMods,
}

/// …and the resources it is read off, as one [`SystemParam`].
///
/// **A bundle because both doors are at the parameter ceiling.** Bevy's
/// `SystemParam` tuples stop at sixteen, `run_bindings` was at exactly sixteen
/// and `use_item` at fifteen, so three more `Res` between them was not a thing
/// either could be given. It carries the settings as well as the two world
/// facts because the press door already held `Res<CVars>` for one flag, and
/// replacing that with this keeps the count where it was.
///
/// **Nothing that takes this may take any of the four `ResMut`**, which is the
/// same rule [`Units`] carries about `PlayerStanding` and for the same reason:
/// the systems that write the clock, the place and the settings do not press
/// buttons.
#[derive(SystemParam)]
pub struct Surroundings<'w> {
    clock: Res<'w, crate::render::sky::WorldClock>,
    place: Res<'w, crate::interface::worldmap::WorldMapState>,
    standing: Res<'w, crate::interface::reputation::PlayerStanding>,
    mods: Res<'w, crate::world::spellmods::SpellMods>,
    cvars: Res<'w, crate::settings::cvars::CVars>,
}

impl Surroundings<'_> {
    /// The three as [`caster_conditions`] wants them.
    ///
    /// **Both `Option`s are always `Some` here**, because this client always has
    /// a clock and always knows whether there is a building overhead. The
    /// `Option` is the *rule's*, for a caller — a test, a CLI check — that has
    /// neither.
    pub fn cast_world(&self) -> CastWorld<'_> {
        let (hour, minute) = self.clock.hour_minute();
        CastWorld {
            daytime: Some(vale_assets::tables::spellbook::is_daytime(hour * 60 + minute)),
            outdoors: Some(self.place.outdoors),
            standing: &self.standing,
            mods: &self.mods,
        }
    }

    /// …and the settings, for the one flag the press door reads off them.
    pub fn cvars(&self) -> &crate::settings::cvars::CVars {
        &self.cvars
    }
}

/// `IsCurrentAction` — is this button's action the one currently *running*?
///
/// **Two questions, not one**, and the second is the reference's *first*.
/// The client reads the slot's spell and compares it against its
/// `CURRENT_MELEE_SPELL` before it looks at anything else, so a
/// **next-swing** ability draws its checked border from the moment it is
/// pressed until the weapon lands. That is the whole of Heroic Strike, Cleave,
/// Raptor Strike and Maul, and it was the first symptom in the action-bar
/// report: pressing one did something on the wire and nothing on the button.
///
/// The auto-attack toggle is the same slot seen from the other end — 6603 is
/// what the client files there while a swing is in progress — which is why this
/// takes both and ors them rather than choosing.
///
/// **…and a cast that has not landed yet is the third**, which this used to
/// leave out. The reference's spell branch is two comparisons in a row against
/// the slot's spell id and nothing else stands between them: the next swing,
/// and then the cast being assembled — which is the spell the press assembled,
/// but only while the press's targeting word is non-zero, i.e. only while it
/// still wants a target.
///
/// **That gate is what the earlier attempt was missing.** It was tried once,
/// lit "the right button and several wrong things with it", and was reverted
/// with the targeting word unread — and unread it looks like "is a cast
/// happening", which is a question about the *client* rather than about this
/// slot. Read, it is the pending spell masked to nothing unless the cast is
/// still holding: the word is seeded from the spell's own `Targets` column and
/// the per-effect switch adds to it, a local refusal clears it, and the press
/// tests it for zero to decide whether the cast goes now or waits.
///
/// So the two states the border is drawn for are:
///
/// * **the spell cursor is up for this slot's spell** — pressed, waiting to be
///   pointed at something, which is [`crate::interface::action::SpellTargeting`];
/// * **a cast of this slot's spell is in flight** — the bar filling, which is
///   [`crate::interface::action::Casting`], and it goes out the moment the
///   cast lands, is interrupted or is refused, because that is when `Casting`
///   stops naming a spell.
///
/// Both are asked as *"is this slot's spell that spell"*, never as "is
/// something happening", which is the whole difference from the attempt that
/// was reverted. It is also the same question [`crate::lua::panels::spellbook`]
/// already answers for a book button (`spell_is_current_cast`), and the two
/// disagreeing was a split with no symptom on either side.
///
/// `ActionButton.lua` reads it to decide whether the button draws its checked
/// border, and pairs it with [`is_auto_repeat_action`], which is the same
/// question for a ranged volley. The reference's third branch — a shapeshift
/// button reading "current" for the form you are in — is not here, because
/// there are no stance buttons to answer it.
pub fn is_current_action(
    bar: &ActionBar,
    slot: u8,
    attacking: bool,
    next_swing: Option<u32>,
    // **The two halves of "a press of this is still in the air"** — the cast in
    // flight and the cursor waiting to be pointed. `None` for neither. See the
    // note above, which is where the reference's own gate is.
    in_flight: Option<u32>,
    aiming: Option<u32>,
) -> bool {
    if attacking && is_attack_action(bar, slot) {
        return true;
    }
    // **Asked of the slot's spell and never of its kind**, exactly as
    // [`is_auto_repeat_action`] is: an item whose entry happens to equal Heroic
    // Strike's id is not the queued swing.
    let is = |spell: Option<u32>| {
        spell.is_some_and(|spell| {
            action(bar, slot).is_some_and(|a| {
                a.kind == vale_protocol::play::spells::action_kind::SPELL && a.action == spell
            })
        })
    };
    is(next_swing) || is(in_flight) || is(aiming)
}

/// `IsAutoRepeatAction` — is this button the ranged attack that is currently
/// **repeating**?
///
/// The other half of the pair above, and it is a different state rather than a
/// variation on it: [`is_current_action`] is the melee swing
/// (`SMSG_ATTACKSTART`, the server's answer) and this is the ranged loop, whose
/// start is the client's own decision and whose only statement on the wire is
/// its *end*. See [`super::action::AutoRepeat`].
///
/// It is asked of the slot's **spell** and never of its kind: an item slot
/// whose entry happens to equal Auto Shot's id is not the auto-repeat, and a
/// macro slot is nothing at all. Three readers, all in `ActionButton.lua` —
/// `ActionButton_UpdateState`'s checked border and the two flash arms — and
/// each re-asks per button rather than being handed a spell id, which is why
/// the events that go with it carry nothing.
pub fn is_auto_repeat_action(bar: &ActionBar, repeating: Option<u32>, slot: u8) -> bool {
    let Some(spell) = repeating else { return false };
    action(bar, slot).is_some_and(|a| {
        a.kind == vale_protocol::play::spells::action_kind::SPELL && a.action == spell
    })
}

/// `ActionHasRange` — is *range* a question about this button at all?
///
/// The interface asks it before it asks [`is_action_in_range`], and it decides
/// something the second question cannot: a button with no key bound draws the
/// **range dot** (`RANGE_INDICATOR`) in place of its hotkey text, and only a
/// button that answers yes here ever gets one (`ActionButton_UpdateHotkeys`).
/// A button that answers no keeps its grey hotkey for ever, which is right for
/// Battle Shout and wrong for nothing.
///
/// The answer is the spell's own [`vale_assets::tables::spellbook::SpellInfo::checks_range`],
/// so the indicator and the client's own local refusal are the same predicate —
/// a bar that lit a button red for a range `check_cast` does not enforce would
/// be telling the player something the press then contradicts.
///
/// **An item slot answers no, and that is a stated gap rather than the
/// reference's behaviour.** A thrown weapon or a bandage has a range in the
/// item's own prototype, which this client does not read; answering from the
/// `ON_USE` spell instead would light the indicator off a number nothing here
/// measures against.
pub fn action_has_range(
    bar: &ActionBar,
    slot: u8,
    // The item's own on-use spell, for an item slot — see the branch below.
    item_spell: Option<&vale_assets::tables::spellbook::SpellInfo>,
) -> bool {
    slot_spell(bar, slot, item_spell)
        .is_some_and(vale_assets::tables::spellbook::SpellInfo::checks_range)
}

/// **The spell a slot's range and cooldown are really about**, whichever kind
/// it is.
///
/// A spell slot carries its own; an item slot carries an entry, and what has a
/// range is the item's *on-use spell* — a bandage reaches five yards because
/// First Aid does. Three reads went through `a.spell` and so answered for no
/// item at all: the fade and the swirl were fixed a round ago and this is the
/// third, which is why it is a shared function now rather than a fourth copy of
/// the same two lines.
fn slot_spell<'a>(
    bar: &'a ActionBar,
    slot: u8,
    item_spell: Option<&'a vale_assets::tables::spellbook::SpellInfo>,
) -> Option<&'a vale_assets::tables::spellbook::SpellInfo> {
    let action = action(bar, slot)?;
    if action.kind == vale_protocol::play::spells::action_kind::ITEM {
        return item_spell;
    }
    action.spell.as_ref()
}

/// `IsActionInRange` — **`Some(true)` in range, `Some(false)` out of it, `None`
/// where the question does not apply.**
///
/// The three answers are the game's own `1` / `0` / `nil` and the interface
/// reads all three differently, testing against the numbers rather than for
/// truth: `ActionButton_OnUpdate` colours the hotkey red on an explicit **`0`**
/// and grey otherwise, and hides the range dot on an explicit **`1`**. So `nil`
/// is "say nothing", not "out of range" — which is what a unit the renderer has
/// not placed yet, or no target at all, has to answer.
///
/// The distance is [`Units::reach`], surface to surface, because that is what
/// the server's own `Spell::CheckRange` measures; the threshold is the spell's
/// maximum **without** `check_cast`'s slack term, since the slack exists to stop
/// this client refusing a cast the server would take and an indicator refuses
/// nothing.
///
/// Only the maximum is tested. A spell with a *minimum* range — the hunter
/// shots — is inside it rather than out of it, and 1.12's own indicator says
/// nothing about being too close either.
pub fn is_action_in_range(
    bar: &ActionBar,
    units: &Units,
    slot: u8,
    item_spell: Option<&vale_assets::tables::spellbook::SpellInfo>,
) -> Option<bool> {
    let info = slot_spell(bar, slot, item_spell)?;
    if !info.checks_range() {
        return None;
    }
    let reach = units.reach(UnitId::Player, UnitId::Target)?;
    Some(reach <= info.range_yards)
}

/// `IsUsableAction` — `(usable, not_enough_mana)`, the game's own pair.
///
/// Two booleans rather than one because the interface draws three states, and the
/// middle one is the informative one: `ActionButton_UpdateUsable` tints an
/// unusable button grey and a merely *unaffordable* one **blue**, so a player can
/// tell "I cannot do that" from "I cannot do that yet". Both false means usable.
///
/// What is checked here is the cost against the caster's own power, which is the
/// half that needs no server. Range, line of sight, stance and reagents are the
/// other half of the real answer and are not modelled — so this errs towards
/// usable, which is the direction that leaves the player able to press the button
/// and be told why by the server.
pub fn is_usable_action(
    bar: &ActionBar,
    units: &Units,
    slot: u8,
    // How many of the slot's item the character is carrying, for an item slot.
    // `None` for a slot that is not an item — see the item branch below.
    carrying: Option<u32>,
) -> (bool, bool) {
    let Some(action) = action(bar, slot) else {
        return (false, false);
    };
    // The attack toggle costs nothing and is always usable.
    if action.is_auto_attack() {
        return (true, false);
    }
    // **An item is usable when you have one**, and that is the reference's own
    // rule rather than an approximation. The per-slot recompute behind
    // `IsUsableAction` has an item branch that is two tests before it ever
    // reaches the mana path:
    //
    // ```text
    // if ((action & 0xf0000000) == 0x80000000)   // the top nibble is the kind
    //     if (count[slot] == 0) return 0;        // the per-slot count
    // ```
    //
    // `notEnoughMana` is left at the zero the function opened with, so an item
    // is never the *blue* tint — grey or nothing.
    //
    // Until this branch existed every item on the bar answered `(false, false)`
    // and `ActionButton_UpdateUsable`'s `else` painted it at 0.4 grey: **every
    // potion, bandage and trinket a player owned was drawn as unusable, always**,
    // which is exactly what it looks like and is the report this came from.
    if action.kind == vale_protocol::play::spells::action_kind::ITEM {
        return (carrying.is_some_and(|count| count > 0), false);
    }
    let Some(info) = action.spell.as_ref() else {
        return (false, false);
    };
    spell_is_usable(info, units)
}

/// **Can this spell be cast right now?** -> `(usable, notEnoughMana)`, which is
/// the pair every button in the game is tinted from.
///
/// Split out of [`is_usable_action`] because the stance bar asks the identical
/// question about a spell that is on no action-bar slot — see
/// [`crate::lua::panels::shapeshift`]. The two must not drift: a form drawn
/// castable on one bar and not on the other is a bug with no symptom on either.
pub fn spell_is_usable(
    info: &vale_assets::tables::spellbook::SpellInfo,
    units: &Units,
) -> (bool, bool) {
    // **The conditions the row states, before the cost.** A spell that cannot
    // be cast at all is *grey*, not blue: `ActionButton_UpdateUsable`'s three
    // branches are usable, `notEnoughMana`, and everything else, and "you have
    // no Seal up" is the third. Ordering it after the cost test would paint a
    // Judgement you cannot afford blue and one you have no Seal for blue too,
    // which says the wrong thing about which problem to fix.
    //
    // See [`SpellInfo::castable_now`] for what the five conditions are and
    // which ability each is famous for. The equipped-item test is the one the
    // row states that this does not make — it wants the character's weapons,
    // which is the inventory's business; it is named there.
    let me = units.get(UnitId::Player);
    if !info.castable_now(
        me.map_or(0, |me| me.aura_state),
        units.get(UnitId::Target).map(|t| t.aura_state),
        me.map_or(0, |me| me.combo_points),
        me.map_or(0, |me| me.shapeshift_form),
    ) {
        return (false, false);
    }
    if info.power_cost == 0 {
        return (true, false);
    }
    // A spell paid for in a power the caster does not have at all reads
    // unaffordable rather than unusable — a warrior's Battle Shout with no rage
    // is the ordinary case and the blue tint is what the game shows for it.
    let has = match units.power_type(UnitId::Player) {
        Some(kind) if u32::from(kind) == info.power_type => units.mana(UnitId::Player),
        _ => 0,
    };
    let affordable = has >= power_type::display(info.power_type as u8, info.power_cost);
    (affordable, !affordable)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The Attack button is checked while the *server* says we are
    /// swinging**, and that is the whole of `IsCurrentAction` for it.
    ///
    /// The bug this pins was never in this function — it answered correctly
    /// throughout — it was that nothing re-asked it. See
    /// `crate::interface::action::follow_attack_state`.
    #[test]
    fn the_attack_slot_is_current_exactly_while_attacking() {
        let mut bar = ActionBar::default();
        bar.slots = vec![None; 4];
        bar.slots[0] = Some(crate::interface::action::Slot {
            action: vale_protocol::play::spells::SPELL_ATTACK,
            kind: vale_protocol::play::spells::action_kind::SPELL,
            spell: None,
        });

        assert!(!is_current_action(&bar, 1, false, None, None, None), "not swinging");
        assert!(is_current_action(&bar, 1, true, None, None, None), "…and swinging");

        // **An item whose entry happens to equal Attack's id is not the Attack
        // button**, which is why the test is on the kind and not the number.
        let mut item = ActionBar::default();
        item.slots = vec![None; 4];
        item.slots[0] = Some(crate::interface::action::Slot {
            action: vale_protocol::play::spells::SPELL_ATTACK,
            kind: vale_protocol::play::spells::action_kind::ITEM,
            spell: None,
        });
        assert!(!is_current_action(&item, 1, true, None, None, None));
    }

    use crate::world::session::WorldEntity;
    use vale_protocol::play::group::{GroupList, GroupMember, PartyMemberStats, member_status};

    /// **What one pass of [`Units`] answered**, recorded out of a real system so
    /// the `SystemParam` is exercised rather than reimplemented.
    #[derive(Resource, Default, Debug, PartialEq)]
    struct Answered {
        guid: Option<u64>,
        exists: bool,
        name: Option<String>,
        health: (u32, u32),
        connected: bool,
    }

    /// An app holding the four resources [`Units`] reads, plus whatever
    /// entities a test spawns.
    fn world(members: &[(u64, &str)]) -> App {
        let mut app = App::new();
        app.init_resource::<crate::interface::target::Selection>()
            .init_resource::<crate::interface::target::Hovered>()
            .init_resource::<crate::interface::gossip::NpcUnit>()
            .init_resource::<crate::interface::reputation::PlayerStanding>()
            .init_resource::<crate::interface::party::Party>()
            .init_resource::<Answered>();
        if !members.is_empty() {
            let list = GroupList {
                group_type: 0,
                own_flags: 0,
                members: members
                    .iter()
                    .map(|(guid, name)| GroupMember {
                        guid: *guid,
                        name: (*name).to_string(),
                        status: member_status::ONLINE,
                        flags: 0,
                    })
                    .collect(),
                leader: members[0].0,
                loot: None,
            };
            app.world_mut()
                .resource_mut::<crate::interface::party::Party>()
                .apply_list(&list);
        }
        app
    }

    fn spawn(app: &mut App, unit: WorldEntity) {
        app.world_mut().spawn(unit);
    }

    /// Despawn the units and leave everything else standing — **`World::clear_entities`
    /// would take the resources with it**, since Bevy stores those as entities too.
    fn clear_units(app: &mut App) {
        let world = app.world_mut();
        let doomed: Vec<Entity> = world
            .query_filtered::<Entity, With<WorldEntity>>()
            .iter(world)
            .collect();
        for entity in doomed {
            world.despawn(entity);
        }
    }

    fn stats(app: &mut App, stats: PartyMemberStats) {
        app.world_mut()
            .resource_mut::<crate::interface::party::Party>()
            .apply_stats(stats);
    }

    /// Run one read of `id` and hand back what it answered.
    fn ask(app: &mut App, id: UnitId) -> Answered {
        app.world_mut().insert_resource(Answered::default());
        let mut schedule = Schedule::default();
        schedule.add_systems(move |units: Units, mut out: ResMut<Answered>| {
            *out = Answered {
                guid: units.guid(id),
                exists: units.exists(id),
                name: units.name(id).map(str::to_string),
                health: (units.health(id), units.health_max(id)),
                connected: units.is_connected(id),
            };
        });
        schedule.run(app.world_mut());
        std::mem::take(&mut app.world_mut().resource_mut::<Answered>())
    }

    /// **The `??` rule, both clauses and both gates** — see
    /// [`Units::level_shown`] for the rule.
    ///
    /// Run with no `DisplayTables`, which is the one thing this harness cannot
    /// build. That fixes the reaction at "no opinion", so the ten-level clause
    /// is off and what is pinned here is the boss clause, the raw level either
    /// side of it, and the fact that a `-1` player level does not mask the
    /// world. The hostile half is pinned by
    /// [`Units::level_shown`]'s own comparison and by the arithmetic below.
    #[test]
    fn a_boss_reports_no_level_and_everybody_else_reports_theirs() {
        let mut app = world(&[]);
        spawn(
            &mut app,
            WorldEntity { guid: 1, is_self: true, level: Some(1), ..Default::default() },
        );
        spawn(
            &mut app,
            WorldEntity {
                guid: 2,
                name: "Archbishop Benedictus".to_string(),
                level: Some(63),
                classification: BOSS_CLASSIFICATION,
                ..Default::default()
            },
        );
        spawn(
            &mut app,
            WorldEntity {
                guid: 3,
                name: "Nightbane Worgen".to_string(),
                level: Some(26),
                ..Default::default()
            },
        );

        assert_eq!(target_level(&mut app, 2), -1, "a worldboss says nothing");
        assert_eq!(
            target_level(&mut app, 3),
            26,
            "…and without a faction table nothing else is masked"
        );
        assert_eq!(player_level(&mut app), 1, "the player is never masked");
    }

    /// **Nine levels above is a number and ten is a skull**, which is the whole
    /// of the comparison `playerLevel <= targetLevel - 10`.
    ///
    /// Asserted on the arithmetic rather than through the param, because the
    /// clause it guards needs a `FactionTemplate.dbc` reading to be reached at
    /// all and the harness has no archives. An off-by-one here is the
    /// difference between a level 60 boss's escort showing `??` and showing 70.
    #[test]
    fn the_ten_level_band_includes_ten_and_excludes_nine() {
        let unknown = |player: i32, target: i32| player <= target - 10;
        assert!(!unknown(1, 10), "nine above still says its level");
        assert!(unknown(1, 11), "ten above does not");
        assert!(unknown(1, 26), "the reported Nightbane Worgen, were it hostile");
        assert!(!unknown(60, 63), "…and a level 60 reads a 63 boss's escort");
    }

    /// Select the guid and ask [`Units::level_shown`] about `"target"`,
    /// through the real `SystemParam`.
    fn target_level(app: &mut App, guid: u64) -> i32 {
        let entity = {
            let world = app.world_mut();
            let mut query = world.query::<(Entity, &WorldEntity)>();
            query
                .iter(world)
                .find(|(_, unit)| unit.guid == guid)
                .map(|(entity, _)| entity)
                .expect("the test spawned it")
        };
        app.world_mut()
            .resource_mut::<crate::interface::target::Selection>()
            .set(guid, entity);
        level_read(app, UnitId::Target)
    }

    fn player_level(app: &mut App) -> i32 {
        level_read(app, UnitId::Player)
    }

    fn level_read(app: &mut App, id: UnitId) -> i32 {
        #[derive(Resource, Default)]
        struct Out(i32);
        app.insert_resource(Out::default());
        let mut schedule = Schedule::default();
        schedule.add_systems(move |units: Units, mut out: ResMut<Out>| {
            out.0 = units.level_shown(None, id);
        });
        schedule.run(app.world_mut());
        app.world().resource::<Out>().0
    }

    const OWNER: u64 = 0x0100_0000_0000_0007;
    const LIVE_PET: u64 = 0xF140_0000_0000_0001;
    const CACHED_PET: u64 = 0xF140_0000_0000_0002;

    /// **The live fields beat the cache, and that order is the measured one.**
    ///
    /// The client asks the object manager for the *owner* first and reads their
    /// `UNIT_FIELD_CHARM`/`SUMMON`; only with no owner in the world does it
    /// fall back to the pet guid `SMSG_PARTY_MEMBER_STATS` left on their row.
    /// A cache consulted first would go on showing a dismissed pet for as long
    /// as nothing re-sent the block.
    #[test]
    fn a_party_pet_is_the_owners_live_field_before_the_group_packets_cache() {
        let mut app = world(&[(OWNER, "Bram")]);
        stats(
            &mut app,
            PartyMemberStats {
                guid: OWNER,
                pet: vale_protocol::play::group::PartyPetStats {
                    guid: Some(CACHED_PET),
                    name: Some("Stale".to_string()),
                    health: Some(1),
                    max_health: Some(2),
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        // Out of range: no owner entity, so the cache is the only answer.
        let out = ask(&mut app, UnitId::PartyPet(1));
        assert_eq!(out.guid, Some(CACHED_PET));
        assert!(out.exists, "a pet across the zone still exists");
        assert_eq!(out.name.as_deref(), Some("Stale"));
        assert_eq!(out.health, (1, 2));
        assert!(
            !out.connected,
            "a pet has no roster row, so `UnitIsConnected` finds nothing and the bar greys",
        );

        // …and now the owner walks into range, carrying a *different* pet.
        spawn(
            &mut app,
            WorldEntity { guid: OWNER, pet: Some(LIVE_PET), ..Default::default() },
        );
        let out = ask(&mut app, UnitId::PartyPet(1));
        assert_eq!(out.guid, Some(LIVE_PET), "the live field wins");

        // …and an owner who is present with *no* pet answers nothing rather
        // than falling through to the stale row.
        clear_units(&mut app);
        spawn(&mut app, WorldEntity { guid: OWNER, ..Default::default() });
        let out = ask(&mut app, UnitId::PartyPet(1));
        assert_eq!(out.guid, None);
        assert!(!out.exists, "the pet was dismissed while we watched");
    }

    /// **Our own pet has no cache and needs none** — the client reads the local
    /// player's two fields and stops. There is no case in which we are out of
    /// our own range.
    ///
    /// And it exists *before* its entity does, which is the half that matters:
    /// `UnitExists` falls back to a walk over our charm/summon directly, so `PetFrame` goes up on the frame the summon field arrives
    /// rather than whenever the pet happens to stream in.
    #[test]
    fn our_own_pet_exists_from_the_field_alone() {
        let mut app = world(&[]);
        spawn(
            &mut app,
            WorldEntity { guid: 1, is_self: true, pet: Some(LIVE_PET), ..Default::default() },
        );
        let out = ask(&mut app, UnitId::Pet);
        assert_eq!(out.guid, Some(LIVE_PET));
        assert!(out.exists, "summoned, not yet streamed in");
        assert_eq!(out.name, None, "…and nothing knows its name yet");

        // Then it arrives, and every read answers off the entity.
        spawn(
            &mut app,
            WorldEntity {
                guid: LIVE_PET,
                name: "Snarl".to_string(),
                health_value: Some((300, 400)),
                ..Default::default()
            },
        );
        let out = ask(&mut app, UnitId::Pet);
        assert_eq!(out.name.as_deref(), Some("Snarl"));
        assert_eq!(out.health, (300, 400));
        assert!(out.connected, "it is in the world");

        // …and with no summon at all there is no pet frame.
        clear_units(&mut app);
        spawn(&mut app, WorldEntity { guid: 1, is_self: true, ..Default::default() });
        assert!(!ask(&mut app, UnitId::Pet).exists);
    }

    /// A member with no pet is not a member with an unnamed one: the group
    /// packet writes a bare NUL for the name of a pet that is not there, and a
    /// frame drawn off that would show an empty plate rather than no plate.
    #[test]
    fn an_empty_pet_name_is_not_a_name() {
        let mut app = world(&[(OWNER, "Bram")]);
        stats(
            &mut app,
            PartyMemberStats {
                guid: OWNER,
                pet: vale_protocol::play::group::PartyPetStats {
                    guid: Some(0),
                    name: Some(String::new()),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let out = ask(&mut app, UnitId::PartyPet(1));
        assert!(!out.exists, "a zero guid is the statement that there is no pet");
        assert_eq!(out.name, None);
    }

    /// The tokens are the game's own spelling, and they round-trip — an addon
    /// passing `"targettarget"` has to reach the same place `Bindings.xml` does.
    #[test]
    fn the_tokens_are_the_games_own() {
        for id in [
            UnitId::Player,
            UnitId::Target,
            UnitId::TargetTarget,
            UnitId::Mouseover,
            UnitId::Npc,
        ] {
            assert_eq!(UnitId::parse(id.token()), Some(id));
        }
        assert_eq!(UnitId::parse("player"), Some(UnitId::Player));
        assert_eq!(UnitId::parse("targettarget"), Some(UnitId::TargetTarget));
    }

    /// Case-insensitive, because the game's own is — see [`UnitId::parse`].
    #[test]
    fn a_token_is_case_insensitive() {
        assert_eq!(UnitId::parse("Player"), Some(UnitId::Player));
        assert_eq!(UnitId::parse("TARGET"), Some(UnitId::Target));
    }

    /// **A token this client has no state for is `None`, not a guess.**
    /// `raid7` is a real token and answering it with the player would be a
    /// wrong unit rather than a missing one.
    ///
    /// `party1..4` came off this list the round the roster was read and
    /// `pet`/`partypet1..4` the round the pet frames were; `party5` is still
    /// refused because a 1.12 party is five including the leader, and so is
    /// `partypet5`.
    #[test]
    fn a_token_with_no_state_behind_it_is_refused() {
        assert_eq!(UnitId::parse("party1"), Some(UnitId::Party(1)));
        assert_eq!(UnitId::parse("PARTY4"), Some(UnitId::Party(4)));
        assert_eq!(UnitId::parse("party5"), None);
        assert_eq!(UnitId::parse("party0"), None);
        assert_eq!(UnitId::parse("pet"), Some(UnitId::Pet));
        assert_eq!(UnitId::parse("PartyPet3"), Some(UnitId::PartyPet(3)));
        assert_eq!(UnitId::parse("partypet5"), None);
        assert_eq!(UnitId::parse("partypet0"), None);
        // **`raid<n>` parses now**, and its two neighbours matter as much as it
        // does: 40 is the last slot and `raidpet<n>` is a *different* prefix
        // that must not fall through to it — see [`UnitId::parse`].
        assert_eq!(UnitId::parse("raid7"), Some(UnitId::Raid(7)));
        assert_eq!(UnitId::parse("RAID40"), Some(UnitId::Raid(40)));
        assert_eq!(UnitId::parse("raid41"), None);
        assert_eq!(UnitId::parse("raid0"), None);
        assert_eq!(UnitId::parse("raid"), None);
        assert_eq!(UnitId::parse("raidpet3"), None, "not raid slot 0");
        assert_eq!(UnitId::parse(""), None);
    }

    /// **Every token round-trips through its own name.** The two families are
    /// written out as literals — [`UnitId::token`] answers a `&'static str` for
    /// twenty callers that want one — so a slot added to the enum without a
    /// literal beside it would silently answer `party4`/`partypet4` for it.
    #[test]
    fn a_token_survives_being_named_and_parsed_back() {
        let every = [
            UnitId::Player,
            UnitId::Target,
            UnitId::TargetTarget,
            UnitId::Mouseover,
            UnitId::Npc,
            UnitId::Pet,
            UnitId::Party(1),
            UnitId::Party(2),
            UnitId::Party(3),
            UnitId::Party(4),
            UnitId::PartyPet(1),
            UnitId::PartyPet(2),
            UnitId::PartyPet(3),
            UnitId::PartyPet(4),
        ];
        for id in every {
            // `targettarget` and `mouseover` are the two the resolver does not
            // hold as strings at all, so they are the ones this would miss.
            assert_eq!(UnitId::parse(id.token()), Some(id), "{}", id.token());
        }
    }

    /// **A pet's owner is the token the game raises `UNIT_PET` at**, and it is
    /// the only inverse either frame has: `PetFrame_OnEvent` opens on
    /// `arg1 == "player"`, never on `"pet"`.
    ///
    /// The two directions are checked together because they have to round-trip:
    /// [`UnitId::pet`] is `UNIT_PET`'s own guard, so a token that answers an
    /// owner and is *also* answered as an owner would raise the event at
    /// itself — which is what the first draft of the vitals watcher did, at
    /// `partypet<n>`, where nothing in the ninety files listens.
    #[test]
    fn a_pet_token_names_its_owner_and_nothing_else_does() {
        assert_eq!(UnitId::Pet.owner(), Some(UnitId::Player));
        assert_eq!(UnitId::PartyPet(3).owner(), Some(UnitId::Party(3)));
        assert_eq!(UnitId::Player.owner(), None);
        assert_eq!(UnitId::Party(1).owner(), None);
        assert_eq!(UnitId::Target.owner(), None);

        assert_eq!(UnitId::Player.pet(), Some(UnitId::Pet));
        assert_eq!(UnitId::Party(3).pet(), Some(UnitId::PartyPet(3)));
        assert_eq!(UnitId::Pet.pet(), None, "a pet does not have a pet");
        assert_eq!(UnitId::PartyPet(1).pet(), None);
        assert_eq!(UnitId::Target.pet(), None);

        // …and the two are inverses wherever both are defined.
        for id in [UnitId::Player, UnitId::Party(1), UnitId::Party(4)] {
            assert_eq!(id.pet().and_then(|pet| pet.owner()), Some(id));
        }
    }

    /// **A queued next-swing ability is the current action**, which is the
    /// reference's *first* comparison in `IsCurrentAction` (against
    /// `CURRENT_MELEE_SPELL`) and was missing outright — so pressing Heroic Strike lit
    /// nothing at all.
    ///
    /// The two halves are checked apart because they are different states that
    /// reach the same button: the auto-attack toggle answers off a swing being
    /// in progress, and the queue answers off its own spell id.
    #[test]
    fn a_queued_swing_and_the_attack_toggle_are_both_current() {
        const SPELL: u8 = vale_protocol::play::spells::action_kind::SPELL;
        let mut bar = ActionBar::default();
        bar.slots = vec![
            // 78 Heroic Strike, 6603 Attack, and an *item* whose entry is 78.
            Some(crate::interface::action::Slot { action: 78, kind: SPELL, spell: None }),
            Some(crate::interface::action::Slot { action: 6603, kind: SPELL, spell: None }),
            Some(crate::interface::action::Slot {
                action: 78,
                kind: vale_protocol::play::spells::action_kind::ITEM,
                spell: None,
            }),
        ];
        // Nothing armed and nothing swinging: no button is current.
        for slot in 1..=3 {
            assert!(!is_current_action(&bar, slot, false, None, None, None), "slot {slot}");
        }
        // Armed: the ability lights and the attack toggle does not.
        assert!(is_current_action(&bar, 1, false, Some(78), None, None));
        assert!(!is_current_action(&bar, 2, false, Some(78), None, None));
        // …and an item slot holding the same number is not the queued spell.
        assert!(!is_current_action(&bar, 3, false, Some(78), None, None));
        // Swinging: the toggle lights, on its own state, with nothing armed.
        assert!(is_current_action(&bar, 2, true, None, None, None));
        assert!(!is_current_action(&bar, 1, true, None, None, None));
        // …and both at once, which is the ordinary case mid-fight.
        assert!(is_current_action(&bar, 1, true, Some(78), None, None));
        assert!(is_current_action(&bar, 2, true, Some(78), None, None));

        // **A cast in flight lights its own button and nothing else's**, which
        // is the whole of what the earlier attempt got wrong: the answer is
        // "is this slot's spell that spell", never "is a cast happening".
        assert!(is_current_action(&bar, 1, false, None, Some(78), None));
        assert!(!is_current_action(&bar, 2, false, None, Some(78), None));
        assert!(!is_current_action(&bar, 3, false, None, Some(78), None), "an item slot");
        // …and a cast of something that is on no slot lights nothing at all.
        assert!(!is_current_action(&bar, 1, false, None, Some(999), None));

        // …and the same for the cursor waiting to be pointed, which is the
        // reference's targeting-word gate — see the function's own note.
        assert!(is_current_action(&bar, 1, false, None, None, Some(78)));
        assert!(!is_current_action(&bar, 2, false, None, None, Some(78)));
        // **Both empty is no border**, which is what makes it go out again: the
        // border is not latched anywhere, it is this answer.
        assert!(!is_current_action(&bar, 1, false, None, None, None));
    }

    /// **`GetActionText` is a macro's name and nothing else's.** The reader is
    /// `ActionButton_Update`, which writes the answer straight onto the
    /// button's Name font string — so a spell answering its own name printed a
    /// label over every icon on the bar.
    #[test]
    fn only_a_macro_slot_has_action_text() {
        let mut bar = ActionBar::default();
        bar.slots = vec![
            Some(crate::interface::action::Slot {
                action: 133,
                kind: vale_protocol::play::spells::action_kind::SPELL,
                spell: None,
            }),
            Some(crate::interface::action::Slot {
                action: 7,
                kind: vale_protocol::play::spells::action_kind::MACRO,
                spell: None,
            }),
        ];
        assert_eq!(get_action_text(&bar, 1), None, "a spell has no text");
        assert!(get_action_text(&bar, 2).is_some(), "a macro does");
    }

    /// **An item slot is named by the kind byte, never by the absence of a
    /// `SpellInfo`.** Both are `spell: None` here — one because the catalog does
    /// not carry the id, one because the number is an item entry — and treating
    /// the second test as the first would send `CMSG_USE_ITEM` for a spell
    /// nobody could resolve and search the bags for its id.
    #[test]
    fn only_an_item_slot_names_an_entry() {
        let mut bar = ActionBar::default();
        bar.slots = vec![
            Some(crate::interface::action::Slot {
                action: 133,
                kind: vale_protocol::play::spells::action_kind::SPELL,
                spell: None,
            }),
            Some(crate::interface::action::Slot {
                action: 6948,
                kind: vale_protocol::play::spells::action_kind::ITEM,
                spell: None,
            }),
            Some(crate::interface::action::Slot {
                action: 7,
                kind: vale_protocol::play::spells::action_kind::MACRO,
                spell: None,
            }),
        ];
        assert_eq!(action_item(&bar, 1), None, "an unresolved spell is not one");
        assert_eq!(action_item(&bar, 2), Some(6948), "the hearthstone");
        assert_eq!(action_item(&bar, 3), None, "a macro is not one either");
        assert_eq!(action_item(&bar, 9), None, "an empty slot");
    }

    /// **`ActionHasRange` is the spell's own predicate and not a distance**, so
    /// the button that draws a range indicator and the press that can be refused
    /// locally agree by construction — a bar that lit a button red for a range
    /// [`vale_assets::tables::spellbook::check_cast`] does not enforce would be
    /// telling the player something the press immediately contradicts.
    ///
    /// The melee row is the case worth naming: its stated 5 yards is a
    /// placeholder for a reach the server computes from both units' bulk and how
    /// fast they are moving, so Heroic Strike answers **no** here and keeps its
    /// grey hotkey. That is a stated gap rather than the reference's behaviour.
    #[test]
    fn only_a_spell_with_a_measurable_range_has_one() {
        use vale_assets::tables::spellbook::SpellInfo;
        let with = |range_index: u32, range_yards: f32, attributes: u32| {
            Some(crate::interface::action::Slot {
                action: 1,
                kind: vale_protocol::play::spells::action_kind::SPELL,
                spell: Some(SpellInfo {
                    range_index,
                    range_yards,
                    attributes,
                    ..SpellInfo::default()
                }),
            })
        };
        let mut bar = ActionBar::default();
        bar.slots = vec![
            // Fireball: row 4, 35 yards.
            with(4, 35.0, 0),
            // Self only.
            with(1, 0.0, 0),
            // The melee row, whose 5.0 is not a distance.
            with(2, 5.0, 0),
            // A row this build's table did not carry.
            with(9, 0.0, 0),
            // An item slot, which carries no `SpellInfo` at all.
            Some(crate::interface::action::Slot {
                action: 6948,
                kind: vale_protocol::play::spells::action_kind::ITEM,
                spell: None,
            }),
        ];
        assert!(action_has_range(&bar, 1, None), "a ranged spell");
        assert!(!action_has_range(&bar, 2, None), "self only");
        assert!(!action_has_range(&bar, 3, None), "the melee row is a placeholder");
        assert!(!action_has_range(&bar, 4, None), "no row, no number");
        assert!(!action_has_range(&bar, 5, None), "an item's range is unread");
        assert!(!action_has_range(&bar, 9, None), "an empty slot");
    }

    /// **Slots are one-based**, and slot 0 is not a slot — which is what lets
    /// `ACTIONBAR_SLOT_CHANGED(0)` mean "all of them" without a separate flag.
    #[test]
    fn slot_numbering_is_the_games_one_based() {
        let mut bar = ActionBar::default();
        bar.slots = vec![None; 12];
        assert!(!has_action(&bar, 0), "there is no slot 0");
        assert!(!has_action(&bar, 1));
        assert!(!has_action(&bar, 13), "…and no slot 13");
        // A slot number that would underflow a zero-based index must not panic.
        assert!(action(&bar, 0).is_none());
    }

    /// **The green range is the client's twenty-entry table** — not the
    /// server's arithmetic, which disagrees at the level
    /// that matters most.
    ///
    /// The decade boundaries are the whole of the rule, so they are what this
    /// pins: it steps at 20, 40 and 60 and nowhere in between, because the index
    /// is `level / 10` and the table repeats each value twice up to 80.
    #[test]
    fn the_green_range_steps_by_decade() {
        assert_eq!(quest_green_range(1), 4);
        assert_eq!(quest_green_range(19), 4);
        assert_eq!(quest_green_range(20), 5, "the first step");
        assert_eq!(quest_green_range(39), 5);
        assert_eq!(quest_green_range(40), 6);
        assert_eq!(quest_green_range(59), 6);
        assert_eq!(quest_green_range(60), 7, "the cap, and the common case");

        // **And 7, not the 9 vmangos' `GetGrayLevel` would give at 60.** The
        // number never crosses the wire; only the client's decides the colour.
        assert_ne!(quest_green_range(60), 9, "the server's rule is not this one");
    }

    /// **No player is zero**, which is the client's own early return, and a
    /// level past the table's end reads its last row rather than panicking —
    /// the client clamps the index to 19.
    #[test]
    fn the_green_range_has_no_edge_that_panics() {
        assert_eq!(quest_green_range(0), 0, "no character");
        assert_eq!(quest_green_range(-1), 0, "an unknown level");
        assert_eq!(quest_green_range(199), 12);
        assert_eq!(quest_green_range(i32::MAX), 12, "clamped, not indexed");
    }

    /// **A mask that admits everyone draws no line at all**, which is what keeps
    /// "Classes:" off every sword in the game — and is the client's own first
    /// pass rather than a tidy-up of the list afterwards.
    #[test]
    fn a_requirement_line_appears_only_when_something_is_excluded() {
        use vale_protocol::state::query::{class_name, race_name};

        // `-1` is what almost every row carries.
        assert!(allowed(-1, PLAYABLE_CLASSES, class_name).is_empty());
        assert!(allowed(-1, PLAYABLE_RACES, race_name).is_empty());

        // A warrior-only item: bit 0.
        assert_eq!(allowed(0b1, PLAYABLE_CLASSES, class_name), vec!["Warrior"]);
        // …and a Horde-only one: orc, undead, tauren, troll — bits 1, 4, 5, 7.
        assert_eq!(
            allowed(0b1011_0010, PLAYABLE_RACES, race_name),
            vec!["Orc", "Undead", "Tauren", "Troll"]
        );
    }

    /// **The two id spaces overlap**, and a shared lookup that tried races first
    /// would list a warrior-only sword as usable by humans. `allows` is the bit
    /// test the client makes, bit `id - 1`.
    #[test]
    fn a_mask_admits_by_bit_and_never_admits_nobody() {
        assert!(allows(0b1, 1), "class 1 is the warrior, race 1 is the human");
        assert!(!allows(0b1, 2));
        assert!(allows(-1, 11), "everyone includes the druid");
        // Zero is "there is no player", and it matches nothing — so a plate
        // composed with no character draws its requirements as unmet, which is
        // the safe direction.
        assert!(!allows(-1, 0));
        assert!(!allows(-1, 33), "past the width of the mask");
    }
}
