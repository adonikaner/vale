//! The stat block, read off the object manager's entity. The decode and the
//! rules it follows are in [`crate::play::stats`].

use crate::play::items::HeldItem;
use crate::play::stats::UnitStats;
use crate::state::objects::Entity;

impl UnitStats {
    /// Decode, or `None` for a unit whose stat block the server never sent —
    /// which is every unit but the local player. See the [`crate::play::stats`]
    /// module comment.
    ///
    /// `weapons` is [`crate::state::objects::ObjectManager::weapons_of`]'s answer,
    /// which is where the three skill lines and the off-hand test come from:
    /// the *fields* say what a skill is worth and the *item* says which skill
    /// to look up.
    pub fn read(entity: &Entity, weapons: &[HeldItem; 3]) -> Option<UnitStats> {
        Self::decode(|index| entity.field(index), weapons)
    }
}
