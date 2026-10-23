//! The panels of the doodad and WMO tools, one file per stem of their state
//! in `crate::tools::placements`.
//!
//! ```text
//! doodads.rs    the doodad tool: the chosen doodad's numbers and the picker
//! wmos.rs       the WMO tool: the chosen WMO's numbers and the picker
//! place.rs      the placement picker both use: the model browser and the
//!               preview of the chosen model
//! group.rs      a selection of more than one placement, and the placements
//!               locked against editing
//! ```

pub(crate) mod doodads;
pub(crate) mod group;
pub(crate) mod place;
pub(crate) mod wmos;
