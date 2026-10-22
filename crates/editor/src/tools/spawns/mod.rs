//! The server's creature and game object spawns on the map.
//!
//! ```text
//! creatures.rs   the server's `creature` spawns on the map: each drawn where
//!                it stands, picked, moved, edited together with the
//!                `creature_template` behind it, created and removed. The
//!                first subject here whose document is a database row, and
//!                the first where an edit can remove a row; game objects,
//!                items and quests remove rows the same way
//! gameobjects.rs the same for `gameobject` and `gameobject_template`: a chest,
//!                a door, a vein, a mailbox. It works as the creature tool
//!                does over the next table, with no path and with a facing
//!                that is three columns
//! waypoints.rs   the path a creature walks: `creature_movement`, drawn over
//!                the ground, its points picked, dragged, added and removed. A
//!                mode of the creature tool rather than an entry on the rail,
//!                because a path has no existence apart from its creature
//! spawn.rs       the handles and keys for a selected creature or game-object
//!                spawn, which has a position and a facing and no other angle:
//!                the reader and writer the handles use, and the keys that
//!                move, turn, remove and duplicate it
//! ```

pub mod creatures;
pub mod gameobjects;
pub mod spawn;
pub mod waypoints;
