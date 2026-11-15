//! The placed models and buildings, and the machinery the placement tools
//! share.
//!
//! ```text
//! doodads.rs     the placed models (MDDF): select, move, turn, scale, remove
//! wmos.rs        the placed buildings, which are MODF and a different list
//! wmoview.rs     the building editor: one building drawn on its own, opened
//!                from the WMO tool, with the two modes below
//! wmoareas.rs    the building editor's Areas mode: a group chosen in the
//!                viewport, and its WMOAreaTable row
//! wmodoodads.rs  the building editor's Doodads mode: the building's own
//!                doodads and sets, edited in its files
//! wmolights.rs   the building editor's Lights mode: the building's lights
//!                and the groups' lists of them
//! place.rs       putting a new doodad or building in the world
//! cluster.rs     several doodads placed with one click, spread round the pointer
//! swap.rs        replacing the model of the selected doodads or buildings
//! group.rs       a selection of more than one placement: the rules the four
//!                placement tools share, the rectangle a drag on empty ground
//!                draws, turning a group about one point, and the clipboard
//! gizmo.rs       the handles a selected placement is moved and turned by
//! rehome.rs      what moving a doodad or a building past a tile border means
//! ```

pub(crate) mod cluster;
pub(crate) mod doodads;
pub(crate) mod gizmo;
pub(crate) mod group;
pub(crate) mod place;
pub(crate) mod rehome;
pub(crate) mod swap;
pub(crate) mod wmoareas;
pub(crate) mod wmodoodads;
pub(crate) mod wmolights;
pub(crate) mod wmoview;
pub(crate) mod wmos;
