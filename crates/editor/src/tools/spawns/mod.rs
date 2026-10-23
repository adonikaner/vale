//! The server's creature and game object spawns on the map.
//!
//! ```text
//! spawn/         the mechanism the two spawn tools share, over SpawnSubject:
//!                the spawns of the open map drawn where they stand, picked,
//!                moved, created and removed, the template behind each edited,
//!                created, copied and renumbered, the command-line flags, and
//!                the keys and handles for a selected spawn
//! creatures.rs   the creature subject: `creature` and `creature_template`,
//!                a facing that is one column, a drag that stands on the
//!                ground, a dome for a marker, and the service icons over the
//!                creatures that offer one. The first subject here whose
//!                document is a database row, and the first where an edit can
//!                remove a row; game objects, items and quests remove rows the
//!                same way
//! gameobjects.rs the game object subject: `gameobject` and
//!                `gameobject_template`, a chest, a door, a vein, a mailbox,
//!                with no path, a facing that is three columns, a drag that
//!                keeps its height above the ground, and a box for a marker
//! waypoints.rs   the path a creature walks: `creature_movement`, drawn over
//!                the ground, its points picked, dragged, added and removed. A
//!                mode of the creature tool rather than an entry on the rail,
//!                because a path has no existence apart from its creature
//! ```

pub(crate) mod creatures;
pub(crate) mod gameobjects;
pub(crate) mod spawn;
pub(crate) mod waypoints;
