//! The watched faction, read off the object manager's entity. The reputation
//! packets are in [`crate::play::reputation`].

use crate::state::objects::Entity;

/// `PLAYER_FIELD_WATCHED_FACTION_INDEX` off a unit's fields, or `None` for
/// anybody but the local player — the field is `PRIVATE`, like the stat block.
///
/// The field is update index 1261: entry 1073 of the player block, whose base
/// is `PLAYER_FIELD_DUEL_ARBITER` at index 188. [`crate::play::stats`]
/// documents the same conversion.
pub fn watched_faction(entity: &Entity) -> Option<i32> {
    entity
        .field(crate::state::fields::player::WATCHED_FACTION_INDEX)
        .map(|raw| raw as i32)
}
