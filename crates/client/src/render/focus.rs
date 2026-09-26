//! Where the world being drawn is, and which map it is.
//!
//! The streaming passes in this directory need four facts: whether there is a
//! world at all, which map, where the centre of interest is, and the map's name
//! for the file paths. They used to take those from
//! [`crate::world::session::Session`] and
//! [`crate::world::session::WorldStatus`] directly, which made every one of them
//! a pass that cannot run without a network socket — against this directory's
//! own rule, which is that a pass still making sense with the network unplugged
//! belongs here.
//!
//! [`WorldFocus`] is that projection and nothing more. It is written once a
//! frame by [`crate::world::session::follow_the_session`] while there is a
//! session, and by whatever else is driving the view while there is not.
//!
//! ## Two writers, and which one wins
//!
//! The client's own writer returns without touching this while
//! `Session::active` is `None`, so a host drawing a map with no server behind
//! it can write the focus itself and keep it. When such a host logs in the
//! session takes the field back, which is the behaviour it wants: the character
//! decides what streams while there is a character.
//!
//! `position` is in the world's own axes, like every field it comes from.
//! [`crate::render::axes`] is still the one place that becomes Bevy's.

use bevy::prelude::*;

/// The centre of what is being drawn.
#[derive(Resource, Debug, Clone, Default)]
pub struct WorldFocus {
    /// Whether there is a world to stream at all. False before the first login,
    /// at the character screen, and while a host has no map open.
    pub present: bool,
    /// The map id, which is what `CollisionWorld` and the height field are keyed
    /// by.
    pub map_id: u32,
    /// …and its name, which is what the file paths are keyed by.
    pub map_name: String,
    /// The centre of the 3x3, in the world's own axes.
    pub position: Vec3,
}

impl WorldFocus {
    /// A focus on one point of one map.
    pub fn at(map_id: u32, map_name: impl Into<String>, position: Vec3) -> WorldFocus {
        WorldFocus {
            present: true,
            map_id,
            map_name: map_name.into(),
            position,
        }
    }

    /// Whether this focus names the same place as another, to within `step`
    /// yards. The streamers compare tiles rather than positions, so this is for
    /// hosts deciding whether to write at all.
    pub fn same_place(&self, other: &WorldFocus, step: f32) -> bool {
        self.present == other.present
            && self.map_id == other.map_id
            && self.position.distance_squared(other.position) < step * step
    }
}
