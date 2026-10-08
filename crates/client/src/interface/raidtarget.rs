//! Raid target icons, the client's side: the eight slots, the event, and
//! `SetRaidTarget`.
//!
//! The wire is [`vale_protocol::play::raidtarget`]. The 1.12.1 client keeps
//! one guid per icon and does the following:
//!
//! * A change stores one guid in one slot; a guid of 0 empties it. The whole
//!   list empties every slot first and then stores each entry. An icon of 8 or
//!   more is ignored. Either form raises `RAID_TARGET_UPDATE`, with no
//!   arguments. The client does not itself keep a unit to one icon; the server
//!   sends the clearing changes for that.
//! * A group list for a party, rather than a raid, empties every slot and
//!   raises `RAID_TARGET_UPDATE`. vmangos follows each party group list with
//!   the whole icon list, so the icons come straight back.
//! * The first raid group list (the raid roster was empty before it) sends the
//!   request for the whole list. This is how a raid member gets the icons:
//!   vmangos does not send a raid the list unasked.
//! * `GetRaidTargetIndex(unit)` answers the lowest slot holding the unit's
//!   guid, plus one, or nil.
//! * `SetRaidTarget(unit, index)` with `index` 1 to 8 sends that icon, less
//!   one, with the unit's guid. Any other index clears: the unit's current icon
//!   is sent with guid 0, and nothing is sent when the unit has none. The
//!   client makes no leader, assistant or group check; the server ignores a
//!   request from anyone else.
//! * The one local refusal is `ERR_INVALID_RAID_TARGET`: a player who is not in
//!   the group, is not the character, and is in a different faction group.
//!   Creatures always pass. The client also refuses a player that is charmed,
//!   which this client does not test.

use bevy::prelude::*;

use vale_protocol::play::raidtarget::{RaidTargetUpdate, ICON_COUNT};
use vale_protocol::play::spells::PlayerEvent;

use crate::input::bindings::{Binding, BindingPressed};
use crate::interface::api::{UnitId, Units};
use crate::interface::events::{PlayerLeavingWorld, RaidTargetUpdated};
use crate::interface::messages::Announce;
use crate::interface::party::{Party, PartyAnswer};
use crate::world::session::Session;

/// What the session thread said. Written by [`crate::world::incoming`].
#[derive(Message, Debug, Clone)]
pub struct RaidTargetAnswer(pub RaidTargetUpdate);

/// The packet this module answers, for `incoming::drain_events`.
pub fn answer_of(event: &PlayerEvent) -> Option<RaidTargetAnswer> {
    match event {
        PlayerEvent::RaidTargets(update) => Some(RaidTargetAnswer(update.clone())),
        _ => None,
    }
}

/// The eight icons: the guid each is on, 0 for none.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct RaidTargets {
    slots: [u64; ICON_COUNT as usize],
}

impl RaidTargets {
    /// Fold one packet in.
    pub fn apply(&mut self, update: &RaidTargetUpdate) {
        match update {
            RaidTargetUpdate::Change { icon, guid } => self.store(*icon, *guid),
            RaidTargetUpdate::List(list) => {
                self.clear();
                for &(icon, guid) in list {
                    self.store(icon, guid);
                }
            }
        }
    }

    fn store(&mut self, icon: u8, guid: u64) {
        if let Some(slot) = self.slots.get_mut(usize::from(icon)) {
            *slot = guid;
        }
    }

    pub fn clear(&mut self) {
        self.slots = Default::default();
    }

    /// The wire icon (0..8) on `guid`: the lowest slot holding it.
    pub fn icon_of(&self, guid: u64) -> Option<u8> {
        if guid == 0 {
            return None;
        }
        self.slots.iter().position(|&held| held == guid).map(|slot| slot as u8)
    }

    /// `GetRaidTargetIndex`: the icon on `guid`, 1 to 8.
    pub fn index_of(&self, guid: u64) -> Option<u8> {
        self.icon_of(guid).map(|icon| icon + 1)
    }

    /// What `SetRaidTarget(unit, index)` sends for the unit `guid`: the wire
    /// icon and the guid to send, or `None` when nothing is sent.
    pub fn request(&self, guid: u64, index: i64) -> Option<(u8, u64)> {
        if guid == 0 {
            return None;
        }
        // The client takes the index as a byte and subtracts one.
        let slot = (index as u8).wrapping_sub(1);
        if slot < ICON_COUNT {
            Some((slot, guid))
        } else {
            self.icon_of(guid).map(|icon| (icon, 0))
        }
    }
}

pub struct RaidTargetPlugin;

impl Plugin for RaidTargetPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RaidTargetAnswer>()
            .init_resource::<RaidTargets>()
            .add_systems(
                Update,
                (answers, press)
                    .chain()
                    .after(crate::input::bindings::BindingSet)
                    .in_set(super::GameSet),
            );
    }
}

/// The icon packets, and the two group-list rules; see the module comment.
fn answers(
    session: Res<Session>,
    mut incoming: MessageReader<RaidTargetAnswer>,
    mut roster: MessageReader<PartyAnswer>,
    mut leaving: MessageReader<PlayerLeavingWorld>,
    mut targets: ResMut<RaidTargets>,
    mut in_raid: Local<bool>,
    mut raise: MessageWriter<RaidTargetUpdated>,
) {
    let mut changed = false;
    if leaving.read().next().is_some() {
        targets.clear();
        *in_raid = false;
    }
    for answer in roster.read() {
        match answer {
            PartyAnswer::List(list) if list.is_raid() => {
                if !*in_raid {
                    if let Some(active) = session.active.as_ref() {
                        active.live.raid_target_list();
                    }
                }
                *in_raid = true;
            }
            PartyAnswer::List(_) => {
                *in_raid = false;
                targets.clear();
                changed = true;
            }
            PartyAnswer::Destroyed => *in_raid = false,
            _ => {}
        }
    }
    for RaidTargetAnswer(update) in incoming.read() {
        targets.apply(update);
        changed = true;
    }
    if changed {
        raise.write(RaidTargetUpdated);
    }
}

/// `SetRaidTarget(unit, index)` on the wire.
fn press(
    mut pressed: MessageReader<BindingPressed>,
    targets: Res<RaidTargets>,
    units: Units,
    party: Res<Party>,
    assets: Res<crate::assets::GameAssets>,
    session: Res<Session>,
    mut say: Announce,
) {
    for BindingPressed(binding) in pressed.read() {
        let Binding::SetRaidTarget { unit, index } = binding else {
            continue;
        };
        let Some(guid) = units.guid(*unit) else {
            continue;
        };
        if refused(&units, &party, &assets, *unit, guid) {
            say.key("ERR_INVALID_RAID_TARGET");
            continue;
        }
        let Some((icon, guid)) = targets.request(guid, *index) else {
            continue;
        };
        if let Some(active) = session.active.as_ref() {
            active.live.raid_target_set(icon, guid);
        }
    }
}

/// `ERR_INVALID_RAID_TARGET`: a player outside the group, other than the
/// character, in another faction group. A unit that is not in view, or a
/// character whose own faction is unknown, is refused too, as the client
/// refuses a unit it cannot resolve.
fn refused(units: &Units, party: &Party, assets: &crate::assets::GameAssets, unit: UnitId, guid: u64) -> bool {
    if party.holds(guid) {
        return false;
    }
    let Some(target) = units.get(unit) else {
        return true;
    };
    if target.kind != vale_protocol::state::update::ObjectType::Player || target.is_self {
        return false;
    }
    let Some(me) = units.get(UnitId::Player) else {
        return true;
    };
    let Ok(tables) = assets.display_tables() else {
        return false;
    };
    let group = |template: Option<u32>| template.and_then(|template| tables.faction_group(template));
    group(target.faction) != group(me.faction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_replaces_and_a_change_stores_one() {
        let mut targets = RaidTargets::default();
        targets.apply(&RaidTargetUpdate::Change { icon: 7, guid: 40 });
        targets.apply(&RaidTargetUpdate::List(vec![(0, 10), (3, 20)]));
        assert_eq!(targets.index_of(40), None, "the list replaced the skull");
        assert_eq!(targets.index_of(10), Some(1));
        assert_eq!(targets.index_of(20), Some(4));
        targets.apply(&RaidTargetUpdate::Change { icon: 3, guid: 0 });
        assert_eq!(targets.index_of(20), None);
        targets.apply(&RaidTargetUpdate::Change { icon: 8, guid: 50 });
        assert_eq!(targets.index_of(50), None, "an icon past the eighth is ignored");
    }

    #[test]
    fn the_lowest_slot_answers_when_a_unit_holds_two() {
        let mut targets = RaidTargets::default();
        targets.apply(&RaidTargetUpdate::Change { icon: 5, guid: 9 });
        targets.apply(&RaidTargetUpdate::Change { icon: 2, guid: 9 });
        assert_eq!(targets.index_of(9), Some(3));
    }

    /// Index 0 clears by sending the unit's current icon with guid 0, and
    /// sends nothing for a unit with no icon.
    #[test]
    fn set_sends_the_icon_less_one_and_clear_sends_the_held_icon() {
        let mut targets = RaidTargets::default();
        assert_eq!(targets.request(9, 8), Some((7, 9)));
        assert_eq!(targets.request(9, 1), Some((0, 9)));
        assert_eq!(targets.request(9, 0), None);
        targets.apply(&RaidTargetUpdate::Change { icon: 4, guid: 9 });
        assert_eq!(targets.request(9, 0), Some((4, 0)));
        assert_eq!(targets.request(9, 9), Some((4, 0)));
        assert_eq!(targets.request(0, 1), None);
    }
}
