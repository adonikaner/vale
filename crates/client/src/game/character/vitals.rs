//! **The unit frames' news feed**: the `UNIT_*` vitals events, written on a
//! change and never per frame.
//!
//! The directory does not poll. `UnitFrameHealthBar_Update` runs **only** when
//! `UNIT_HEALTH`/`UNIT_MAXHEALTH` arrive, `UnitFrameManaBar_Update` only on the
//! ten power events, and the colours ride the same path — the health bar's
//! green is `HealthBar_OnValueChanged` downstream of the `SetValue` those
//! handlers make, and the mana bar's blue is `UnitFrame_UpdateManaType` inside
//! the power handler. So before this module raised them, every unit frame sat
//! exactly as it loaded: both bars full-width and untinted, which is the white
//! plate over `UI-StatusBar`'s pale gradient the screenshots showed.
//!
//! `PLAYER_ENTERING_WORLD` is here too, because it is the same news one level
//! up — "there is a player now" — and it is the event half the directory
//! initialises itself on (`PlayerFrame_OnEvent` runs `PlayerFrame_Update` off
//! it, which fills the name, the level and both bars in one call).
//!
//! ## What is watched
//!
//! `player`, `target` and **`party1`..`party4`** — every token the shipped unit
//! frames put a bar on. `targettarget`'s little frame reads through
//! `TargetFrame_Update` on `PLAYER_TARGET_CHANGED`, so it refreshes on
//! selection; what it misses is a *drift* in the target's target's health while
//! the selection holds, which is a stated gap rather than an oversight. The
//! values compared are [`super::super::api::Units`]' own answers — the display units the
//! interface reads — so a diff here is exactly a diff in what a re-read would
//! return.
//!
//! ## The party is on that list because the party frames were dead
//!
//! `PartyFrameTemplates.xml` wires `PartyMemberFrame<n>HealthBar`'s `OnEvent` to
//! `UnitFrameHealthBar_Update(this, arg1)`, and `UnitFrameHealthBar_Initialize`
//! — reached from the template's own `OnLoad`, through
//! `UnitFrame_Initialize("party"..id, …)` — registers `UNIT_HEALTH` and
//! `UNIT_MAXHEALTH` **on the bar itself**. So a party member's bar moves for
//! exactly one reason in 1.12: a `UNIT_HEALTH` whose `arg1` is `party<n>`.
//! Nothing else in the file touches it.
//!
//! This watcher answered `player` and `target` only, so that event never
//! carried a party token and **no party frame ever moved** — a member standing
//! beside us losing half their health redrew nothing at all. The one path that
//! did reach them was accidental: `PARTY_MEMBERS_CHANGED` runs
//! `PartyMemberFrame_UpdateMember`, which re-reads both bars from scratch, and
//! [`super::super::session::party`] raised it on every `SMSG_PARTY_MEMBER_STATS`.
//! That packet is only sent for a member **out of range**
//! (`Player::SendUpdateToOutOfRangeGroupMembers`), which is precisely the case
//! where nobody is looking — the member in the room with us, whose health comes
//! off an ordinary update block, had no path at all.

use bevy::prelude::*;

use super::super::api::{UnitId, Units};
use super::super::events::{
    ActionbarUpdateUsable, PlayerEnteringWorld, PlayerXpUpdate, UnitDisplaypowerChanged,
    UnitFactionChanged, UnitHealthChanged, UnitLevelChanged, UnitMaxHealthChanged, UnitNameUpdate,
    UnitPetChanged, UnitPetExperience, UnitPetTrainingPoints, UnitPowerChanged, UpdateExhaustion,
};

/// What one unit's frames are drawn from, read in one pass so the diff and the
/// re-read cannot disagree. The name and the level ride with the bars because
/// they are the two vitals that routinely resolve *late* — a query round-trip
/// after the unit exists — so a plate filled only on `PLAYER_TARGET_CHANGED`
/// shows the fallback for ever.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Vitals {
    /// **Who this reading is of**, which a token does not say.
    ///
    /// `party1` is a slot: somebody leaving shifts everybody below them up one
    /// and the token then names a different character. Comparing the numbers
    /// alone would announce nothing where two members' health happened to
    /// agree, so a slot whose occupant changed is treated as a first reading
    /// and announces everything. `None` only for a unit the roster and the
    /// world both fail to identify, which cannot happen for a token
    /// [`Vitals::read`] answered at all.
    guid: Option<u64>,
    health: u32,
    health_max: u32,
    power: u32,
    power_max: u32,
    kind: Option<u8>,
    name: Option<String>,
    level: i32,
    /// `PLAYER_XP` and `PLAYER_NEXT_LEVEL_XP`, and the rested pool beside them.
    ///
    /// **On the player's row only** — the fields are `PRIVATE` and the server
    /// sends nobody else's, so the target's copy is `(0, 0)`/`None` for ever and
    /// its two events are raised once, on the frame it resolves. Cheap, and the
    /// alternative is a second watcher for two numbers.
    experience: (u32, u32),
    rested: Option<u32>,
    /// **Who this unit's `pet` token names**, and the whole of `UNIT_PET`.
    ///
    /// The guid rather than a bool, because the event is "your pet is
    /// different" and not "you have one": a warlock swapping an imp for a
    /// voidwalker keeps a pet throughout, and a frame that only heard about
    /// the *presence* would keep drawing the imp's portrait and health.
    ///
    /// Read for every watched token, not only for owners — a unit with no pet
    /// answers `None` for ever and costs one comparison. See
    /// [`super::super::api::Units::pet_guid_for`].
    pet: Option<u64>,
    /// `UNIT_FIELD_FACTIONTEMPLATE` and the PvP flag, which are the two things
    /// `UNIT_FACTION` is about — see the event.
    faction: Option<(u32, bool)>,
    /// **The five numbers of the pet panel**, read for the `pet` token alone.
    ///
    /// The happiness value moves every few seconds on a live pet and the mood
    /// face redraws only on `UNIT_HAPPINESS` (`PetFrame_OnEvent`), which the
    /// power diff above cannot raise: the pet's *display* power is focus, and
    /// happiness lives in `UNIT_FIELD_POWER5` beside it. Before this was
    /// watched, feeding a pet changed nothing on screen until a re-log rebuilt
    /// the frames. `None` for every other token, which costs one branch.
    pet_stats: Option<vale_protocol::state::objects::PetStats>,
}

impl Vitals {
    /// The pet of the unit a token names, for the tokens that can own one.
    ///
    /// `player` and `party<n>` are the only two the game raises `UNIT_PET` at,
    /// and they are the only two [`UnitId::owner`] has an inverse for — so this
    /// maps back the other way and answers `None` for everything else, which
    /// includes a pet's own token. A pet does not have a pet.
    fn pet_of(units: &Units, id: UnitId) -> Option<u64> {
        units.pet_guid_for(id.pet()?)
    }

    /// **Gated on [`Units::exists`], not on an entity.** A party member across
    /// the zone has no `WorldEntity` at all — what is known about them is a
    /// roster row and whatever `SMSG_PARTY_MEMBER_STATS` last said — and every
    /// read below falls through to it. Gating on the entity would drop that
    /// member out of the watch list the instant they walked out of range,
    /// which is the half of the party the stats packet exists to cover.
    fn read(units: &Units, id: UnitId) -> Option<Self> {
        if !units.exists(id) {
            return None;
        }
        Some(Self {
            guid: units.guid(id),
            health: units.health(id),
            health_max: units.health_max(id),
            power: units.mana(id),
            power_max: units.mana_max(id),
            kind: units.power_type(id),
            name: units.name(id).map(str::to_string),
            level: units.level(id),
            experience: units.experience(id),
            rested: units.rested_experience(id),
            // **`pet_guid_for` answers only for a pet token**, so this is the
            // *owner*'s reading: `Units::pet_guid_for(Player)` is `None` by
            // construction. What is wanted here is the pet of the unit this
            // row is about, which is the pet token whose owner it is.
            pet: Self::pet_of(units, id),
            faction: units.get(id).map(|unit| {
                (
                    unit.faction.unwrap_or(0),
                    unit.unit_flags & super::super::api::UNIT_FLAG_PVP != 0,
                )
            }),
            pet_stats: match id {
                UnitId::Pet => units.get(id).and_then(|unit| unit.pet_stats),
                _ => None,
            },
        })
    }
}

/// **Every token a shipped unit frame puts a bar on**, and the whole of what
/// this watcher polls.
///
/// The order is the slot order in [`announce`]'s `Local` and nothing else reads
/// it, so it may be extended freely.
///
/// **`pet` and `partypet<n>` are on it now**, and they are here for exactly the
/// reason the party tokens were: `PetFrame.xml`'s `OnLoad` is
/// `UnitFrame_Initialize("pet", …)` and `PartyFrameTemplates.xml`'s is
/// `UnitFrame_Initialize("partypet"..id, …)`, so each pet frame's bars register
/// `UNIT_HEALTH` and its nine siblings **on themselves** and move for a single
/// reason — an event whose `arg1` is their own token. Nothing raised one, and
/// every pet bar in the game sat at its loaded width.
///
/// **…and `raid1`..`raid40` are on it now**, for the same reason once more:
/// `RaidGroupFrame_OnEvent` matches `arg1` against `raid([0-9]+)` on
/// `UNIT_HEALTH` and `UNIT_LEVEL` and redraws that one button, so a client that
/// raises those names only at `party<n>` leaves every cell of the raid grid at
/// whatever it was filled with when the panel was built. A slot nobody occupies
/// — which is all forty of them for a solo character or a party — resolves to
/// nothing and costs one branch, because
/// [`crate::game::session::party::Party::raid_slot`] answers `None` before it
/// looks at the world.
const WATCHED: [UnitId; 11 + vale_protocol::play::group::MAX_RAID_MEMBERS] = {
    let head = [
        UnitId::Player,
        UnitId::Target,
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
    let mut all = [UnitId::Player; 11 + vale_protocol::play::group::MAX_RAID_MEMBERS];
    let mut index = 0;
    while index < head.len() {
        all[index] = head[index];
        index += 1;
    }
    while index < all.len() {
        all[index] = UnitId::Raid(index - head.len() + 1);
        index += 1;
    }
    all
};

pub struct VitalsPlugin;

impl Plugin for VitalsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            // After the targeting chain, so the frame a selection changes in
            // announces the *new* unit's vitals rather than last frame's.
            announce
                .after(super::super::combat::target::TargetSet)
                .in_set(super::super::GameSet),
        );
    }
}

/// One write per changed number, once per change.
///
/// The same comparison shape as `announce_target_change`: the previous reading
/// is a `Local`, the first frame a unit resolves announces everything (a bar
/// that has never heard is a bar at its loaded state), and a unit that stops
/// resolving is simply forgotten — the frames hide themselves off
/// `PLAYER_TARGET_CHANGED`, not off a vitals event.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn announce(
    units: Units,
    // **A `Vec` rather than `[Option<Vitals>; WATCHED.len()]`**, which is what
    // it was until the forty raid slots took the array past Rust's `Default`
    // bound of 32. Sized once on the first frame and never again.
    mut last: Local<Vec<Option<Vitals>>>,
    mut entering: MessageWriter<PlayerEnteringWorld>,
    mut health: MessageWriter<UnitHealthChanged>,
    mut health_max: MessageWriter<UnitMaxHealthChanged>,
    mut power: MessageWriter<UnitPowerChanged>,
    mut displaypower: MessageWriter<UnitDisplaypowerChanged>,
    mut name: MessageWriter<UnitNameUpdate>,
    mut level: MessageWriter<UnitLevelChanged>,
    mut experience: MessageWriter<PlayerXpUpdate>,
    mut exhaustion: MessageWriter<UpdateExhaustion>,
    mut usable: MessageWriter<ActionbarUpdateUsable>,
    mut pet: MessageWriter<UnitPetChanged>,
    mut faction: MessageWriter<UnitFactionChanged>,
    mut pet_experience: MessageWriter<UnitPetExperience>,
    mut pet_training: MessageWriter<UnitPetTrainingPoints>,
) {
    if last.len() != WATCHED.len() {
        last.resize(WATCHED.len(), None);
    }
    for (slot, id) in WATCHED.iter().copied().enumerate() {
        let now = Vitals::read(&units, id);
        let previous = std::mem::replace(&mut last[slot], now.clone());
        let Some(now) = now else { continue };

        // **A slot whose occupant changed has no history**, so drop the old
        // reading rather than diff against somebody else's numbers — see
        // [`Vitals::guid`]. For `player` and `target` this is the ordinary
        // "selection moved" case and the target frame is rebuilt off
        // `PLAYER_TARGET_CHANGED` anyway; for `party<n>` it is a member leaving
        // and everybody below them shuffling up one.
        let previous = previous.filter(|p| p.guid == now.guid);

        // The player existing where there was none is the login (or the far
        // side of a teleport, where the world was torn down and rebuilt).
        if id == UnitId::Player && previous.is_none() {
            entering.write(PlayerEnteringWorld);
        }

        let kind = now.kind.unwrap_or(0);
        let changed = |field: fn(&Vitals) -> u32| {
            previous.as_ref().is_none_or(|p| field(p) != field(&now))
        };
        if changed(|v| v.health) {
            health.write(UnitHealthChanged(id));
        }
        if changed(|v| v.health_max) {
            health_max.write(UnitMaxHealthChanged(id));
        }
        if previous.as_ref().is_none_or(|p| p.kind != now.kind) {
            displaypower.write(UnitDisplaypowerChanged(id));
        }
        if changed(|v| v.power) {
            power.write(UnitPowerChanged { unit: id, power: kind, max: false });
        }
        if changed(|v| v.power_max) {
            power.write(UnitPowerChanged { unit: id, power: kind, max: true });
        }
        // **…and the bar, on our own power only.** `ACTIONBAR_UPDATE_USABLE` is
        // the one thing that re-runs `ActionButton_UpdateUsable` while a session
        // is going, and what that function reads —
        // [`super::super::api::is_usable_action`] — is the spell's cost against *this*
        // character's own power and nothing else. So the target's mana moving is
        // not news to a button, and raising it there would be a bar's worth of
        // work every tick of every enemy caster in sight.
        //
        // The kind counts as well as the amount: a druid shifting into cat form
        // swaps mana for energy, and every button costing the old one becomes
        // unaffordable in the same instant.
        if id == UnitId::Player
            && (changed(|v| v.power)
                || changed(|v| v.power_max)
                || previous.as_ref().is_none_or(|p| p.kind != now.kind))
        {
            usable.write(ActionbarUpdateUsable);
        }
        if previous.as_ref().is_none_or(|p| p.name != now.name) {
            name.write(UnitNameUpdate(id));
        }
        if previous.as_ref().is_none_or(|p| p.level != now.level) {
            level.write(UnitLevelChanged(id));
        }
        // **`arg1` is the owner's token** — see [`UnitPetChanged`] — and the
        // gate is [`UnitId::pet`] rather than the reading, because `None` here
        // is two different things: a token that *cannot* own a pet and one that
        // owns none at the moment. Diffing on the reading alone raised
        // `UNIT_PET("partypet1")` on the first frame a party pet resolved,
        // which no body in the ninety files listens for.
        if id.pet().is_some() && previous.as_ref().is_none_or(|p| p.pet != now.pet) {
            pet.write(UnitPetChanged(id));
        }
        if previous.as_ref().is_none_or(|p| p.faction != now.faction) {
            faction.write(UnitFactionChanged(id));
        }
        // **The pet panel's three**, on the `pet` token alone — see
        // [`Vitals::pet_stats`]. Happiness fires as `UNIT_HAPPINESS`, which is
        // power type 4 under [`UnitPowerChanged`]'s naming; the experience and
        // training-point pair have names of their own, and a loyalty-level
        // change rides the training event because the paper doll's only
        // loyalty redraw is the full-update branch that event reaches.
        if id == UnitId::Pet {
            let prev = previous.as_ref().and_then(|p| p.pet_stats);
            if let Some(stats) = now.pet_stats {
                if prev.is_none_or(|p| p.happiness != stats.happiness) {
                    power.write(UnitPowerChanged {
                        unit: id,
                        power: vale_protocol::state::objects::power_type::HAPPINESS as u8,
                        max: false,
                    });
                }
                if prev.is_none_or(|p| {
                    (p.experience, p.next_level_experience)
                        != (stats.experience, stats.next_level_experience)
                }) {
                    pet_experience.write(UnitPetExperience);
                }
                if prev.is_none_or(|p| {
                    (p.training_total, p.training_spent, p.loyalty_level)
                        != (stats.training_total, stats.training_spent, stats.loyalty_level)
                }) {
                    pet_training.write(UnitPetTrainingPoints);
                }
            }
        }
        // **The XP bar's two, player only.** `MainMenuExpBar_Update` runs off
        // `PLAYER_XP_UPDATE` and off nothing else, and the bar it feeds is
        // *hidden* while its maximum is zero — so the first raise, on the frame
        // the player resolves, is what puts the bar on the screen at all.
        if id == UnitId::Player {
            if previous.as_ref().is_none_or(|p| p.experience != now.experience) {
                experience.write(PlayerXpUpdate);
            }
            if previous.as_ref().is_none_or(|p| p.rested != now.rested) {
                exhaustion.write(UpdateExhaustion);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::session::party::Party;
    use vale_protocol::play::group::{GroupList, GroupMember, PartyMemberStats, member_status};

    /// One app holding just the five resources [`Units`] reads and the ten
    /// messages [`announce`] writes. No renderer, no session, no entities — the
    /// roster path is the whole point, because it is the one a party member
    /// across the zone has.
    fn harness() -> App {
        let mut app = App::new();
        app.init_resource::<crate::game::combat::target::Selection>()
            .init_resource::<crate::game::combat::target::Hovered>()
            .init_resource::<crate::game::npc::gossip::NpcUnit>()
            .init_resource::<crate::game::character::reputation::PlayerStanding>()
            .init_resource::<Party>()
            .add_message::<PlayerEnteringWorld>()
            .add_message::<UnitHealthChanged>()
            .add_message::<UnitMaxHealthChanged>()
            .add_message::<UnitPowerChanged>()
            .add_message::<UnitDisplaypowerChanged>()
            .add_message::<UnitNameUpdate>()
            .add_message::<UnitLevelChanged>()
            .add_message::<PlayerXpUpdate>()
            .add_message::<UpdateExhaustion>()
            .add_message::<ActionbarUpdateUsable>()
            .add_message::<UnitPetChanged>()
            .add_message::<UnitFactionChanged>()
            .add_message::<UnitPetExperience>()
            .add_message::<UnitPetTrainingPoints>()
            .add_systems(Update, announce);
        app
    }

    /// The tokens `UNIT_HEALTH` carried on the last run of [`announce`].
    fn health_tokens(app: &mut App) -> Vec<&'static str> {
        app.world_mut()
            .resource_mut::<Messages<UnitHealthChanged>>()
            .drain()
            .map(|event| event.0.token())
            .collect()
    }

    fn roster(app: &mut App, members: &[(u64, &str)]) {
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
            leader: members.first().map_or(0, |(guid, _)| *guid),
            loot: None,
        };
        app.world_mut().resource_mut::<Party>().apply_list(&list);
    }

    fn stats(app: &mut App, guid: u64, health: u16, max: u16) {
        app.world_mut()
            .resource_mut::<Party>()
            .apply_stats(PartyMemberStats {
                guid,
                health: Some(health),
                max_health: Some(max),
                ..Default::default()
            });
    }

    /// **The bug this module's watch list was widened for.**
    ///
    /// `PartyMemberFrame<n>HealthBar` moves for exactly one reason in 1.12 — a
    /// `UNIT_HEALTH` whose `arg1` is its own token — so a party member taking
    /// damage has to come out of here as `party1` or the frame never redraws.
    #[test]
    fn a_party_members_health_is_announced_at_their_own_token() {
        let mut app = harness();
        roster(&mut app, &[(7, "Bram")]);
        stats(&mut app, 7, 900, 900);
        app.update();
        assert_eq!(
            health_tokens(&mut app),
            ["party1"],
            "the first reading of a member is news"
        );

        // A frame in which nothing moved says nothing — the directory does not
        // poll and neither does this.
        app.update();
        assert!(health_tokens(&mut app).is_empty(), "a bar redrawn for nothing");

        // …and then they take a hit.
        stats(&mut app, 7, 400, 900);
        app.update();
        assert_eq!(health_tokens(&mut app), ["party1"]);
    }

    /// **A slot is not a person.** `party1` leaving shifts `party2` up into it,
    /// and if the two happened to be on the same health a watcher comparing
    /// only the numbers would announce nothing — leaving the frame drawn at the
    /// departed member's bar. See [`Vitals::guid`].
    #[test]
    fn a_slot_whose_occupant_changed_announces_everything() {
        let mut app = harness();
        roster(&mut app, &[(7, "Bram"), (8, "Merrick")]);
        stats(&mut app, 7, 500, 900);
        stats(&mut app, 8, 500, 900);
        app.update();
        assert_eq!(health_tokens(&mut app), ["party1", "party2"]);

        // Bram leaves. Merrick is `party1` now, on exactly the same numbers.
        roster(&mut app, &[(8, "Merrick")]);
        app.update();
        assert_eq!(
            health_tokens(&mut app),
            ["party1"],
            "the slot changed hands with no event"
        );
    }

    /// A party member who is *not* there is not news either way — the four
    /// tokens cost nothing when the character is soloing, which is the ordinary
    /// case for this system.
    #[test]
    fn an_empty_party_announces_nothing() {
        let mut app = harness();
        app.update();
        assert!(health_tokens(&mut app).is_empty());
    }

    /// The tokens `UNIT_PET` carried on the last run of [`announce`].
    fn pet_tokens(app: &mut App) -> Vec<&'static str> {
        app.world_mut()
            .resource_mut::<Messages<UnitPetChanged>>()
            .drain()
            .map(|event| event.0.token())
            .collect()
    }

    /// Give a party member a pet, over the group packet's own pet block.
    fn pet_stats(app: &mut App, guid: u64, pet: u64, health: u16, max: u16) {
        app.world_mut().resource_mut::<Party>().apply_stats(PartyMemberStats {
            guid,
            pet: vale_protocol::play::group::PartyPetStats {
                guid: Some(pet),
                name: Some("Snarl".to_string()),
                health: Some(health),
                max_health: Some(max),
                ..Default::default()
            },
            ..Default::default()
        });
    }

    /// **The pet frames' own bug, and it is the party frames' one unit further
    /// out.** `PartyMemberFrame<n>PetFrame`'s bars are initialised with
    /// `UnitFrame_Initialize("partypet"..id, …)`, so they move for a
    /// `UNIT_HEALTH` whose `arg1` is `partypet<n>` and for nothing else — and
    /// the watch list had five tokens on it, none of them a pet's.
    #[test]
    fn a_party_pets_health_is_announced_at_its_own_token() {
        let mut app = harness();
        roster(&mut app, &[(7, "Bram")]);
        pet_stats(&mut app, 7, 0xF140_0000_0000_0001, 500, 500);
        app.update();
        let tokens = health_tokens(&mut app);
        assert!(tokens.contains(&"party1"), "the member: {tokens:?}");
        assert!(tokens.contains(&"partypet1"), "…and their pet: {tokens:?}");

        app.update();
        assert!(health_tokens(&mut app).is_empty(), "a bar redrawn for nothing");

        pet_stats(&mut app, 7, 0xF140_0000_0000_0001, 120, 500);
        app.update();
        assert_eq!(health_tokens(&mut app), ["partypet1"]);
    }

    /// **`UNIT_PET`'s `arg1` is the *owner*'s token**, which is what tells
    /// `PetFrame_OnEvent` (`arg1 == "player"`) from `PartyMemberFrame_OnEvent`
    /// (`arg1 == "party<n>"`). Announcing it at the pet's own token would reach
    /// neither body.
    #[test]
    fn a_pet_appearing_is_announced_at_its_owners_token() {
        let mut app = harness();
        roster(&mut app, &[(7, "Bram")]);
        app.update();
        // The member resolved, with no pet — one reading, and it is news
        // because a frame that has never heard is a frame at its loaded state.
        assert_eq!(pet_tokens(&mut app), ["party1"]);

        // …and now they call one, which has to reach
        // `PartyMemberFrame_UpdatePet` — the function that both shows the pet
        // frame and moves the member's own down sixteen pixels.
        pet_stats(&mut app, 7, 0xF140_0000_0000_0001, 500, 500);
        app.update();
        assert_eq!(pet_tokens(&mut app), ["party1"]);

        app.update();
        assert!(pet_tokens(&mut app).is_empty(), "a frame rebuilt for nothing");
    }

    /// **A pet that is dismissed is the same event again**, and it is the half
    /// a watcher gated on "is there a pet" would miss: the frame comes down
    /// only because something raised `UNIT_PET` with no pet behind it.
    #[test]
    fn a_pet_being_dismissed_is_announced_too() {
        let mut app = harness();
        roster(&mut app, &[(7, "Bram")]);
        pet_stats(&mut app, 7, 0xF140_0000_0000_0001, 500, 500);
        app.update();
        let _ = pet_tokens(&mut app);

        // `GROUP_UPDATE_FLAG_PET_GUID` carrying **zero** is the statement that
        // the pet is gone — an absent field would mean "unchanged".
        app.world_mut().resource_mut::<Party>().apply_stats(PartyMemberStats {
            guid: 7,
            pet: vale_protocol::play::group::PartyPetStats {
                guid: Some(0),
                ..Default::default()
            },
            ..Default::default()
        });
        app.update();
        assert_eq!(pet_tokens(&mut app), ["party1"]);
    }
}
