//! The skill block, read off the object manager's entity. The slots and their
//! decode are in [`crate::play::skills`].

use crate::play::skills::Skills;
use crate::state::objects::Entity;

impl Skills {
    /// Read it off a unit, or `None` for anybody but the local player.
    ///
    /// `PRIVATE`, like the stat block and the exploration mask, so nobody else's
    /// ever arrives — see [`crate::play::stats::UnitStats::read`], which answers
    /// `None` for the same reason and would be the place to look if this ever
    /// started answering for a target.
    ///
    /// **`None` and "no skills" are different**, and both are possible: a
    /// character always has at least a language, so an empty list here means the
    /// update block has not landed yet rather than a character with nothing.
    pub fn read(entity: &Entity) -> Option<Skills> {
        if entity.object_type != Some(crate::state::update::ObjectType::Player) {
            return None;
        }
        Self::decode(|index| entity.field(index))
    }
}
