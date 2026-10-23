//! Whose effect an entity at the world root is: the one owner component every
//! effect pass stamps on what it spawns.

use bevy::prelude::*;

/// The entity a spawned effect belongs to.
///
/// The effect passes spawn at the world root, so walking an owner's children
/// never reaches what it emitted:
///
/// ```text
/// particles.rs     each emitter, owned by the attachment or tile carrying it
/// ribbons.rs       each trail, owned the same way
/// decals.rs        each ground quad, owned by its attachment root
/// weapon_trail.rs  each strip, owned by the weapon's attachment root
/// missiles.rs      each projectile, owned by the unit that cast it
/// lightning.rs     each bolt, owned by the unit that cast it
/// ```
///
/// Every one of them carries this, so a host finds all the effects of a set of
/// entities with one query on it, whatever pass spawned them. An effect hung on
/// another effect's model (the emitters on a missile) names an entity under
/// that effect, so the whole tree is found by repeating the join from what the
/// first one found.
///
/// The passes keep their own owner fields for their own liveness rules; this
/// is the read side and nothing in the client acts on it.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectOf(pub(crate) Entity);

impl EffectOf {
    /// The entity this effect belongs to.
    pub fn owner(&self) -> Entity {
        self.0
    }
}
