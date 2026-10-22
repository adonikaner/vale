//! The placed models and buildings, and the machinery the placement tools
//! share.
//!
//! ```text
//! doodads.rs     the placed models (MDDF): select, move, turn, scale, remove
//! wmos.rs        the placed buildings, which are MODF and a different list
//! place.rs       putting a new doodad or building in the world
//! group.rs       a selection of more than one placement: the rules the four
//!                placement tools share, the rectangle a drag on empty ground
//!                draws, turning a group about one point, and the clipboard
//! gizmo.rs       the handles a selected placement is moved and turned by
//! rehome.rs      what moving a doodad or a building past a tile border means
//! ```

pub mod doodads;
pub mod gizmo;
pub mod group;
pub mod place;
pub mod rehome;
pub mod wmos;
