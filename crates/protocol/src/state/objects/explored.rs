//! The exploration mask, read off the object manager's entity. The mask and
//! its rules are in [`crate::play::explored`].

use crate::play::explored::Explored;
use crate::state::objects::Entity;

impl Explored {
    /// Read the mask off a unit's fields, or `None` for a unit that carries none
    /// — which is everyone but us. See the [`crate::play::explored`] module
    /// comment.
    ///
    /// **A single set bit is enough to count as carrying one**, and the reason
    /// is the update protocol rather than taste: a *values* block sends only the
    /// dwords that changed, so a character who has just discovered one area
    /// arrives with 1 of the 64 present. Testing for all 64 would answer `None`
    /// for every session after the create block.
    pub fn read(entity: &Entity) -> Option<Explored> {
        Self::decode(|index| entity.field(index))
    }
}
