//! The panels of the doodad and WMO tools, one file per stem of their state
//! in `crate::tools::placements`.
//!
//! ```text
//! doodads.rs    the doodad tool: the chosen doodad's numbers and the picker
//! wmos.rs       the WMO tool: the chosen WMO's numbers and the picker
//! wmoview.rs    the building editor's toolbar and the heading of its panel
//! wmoareas.rs   the building editor's Areas mode: the building's groups and
//!               the chosen group's WMOAreaTable row
//! wmodoodads.rs the building editor's Doodads mode: the building's own
//!               doodads, its sets, and the copy made before the first edit
//! wmolights.rs  the building editor's Lights mode: the building's lights
//! place.rs      the placement picker both use: the model browser and the
//!               preview of the chosen model
//! group.rs      a selection of more than one placement, and the placements
//!               locked against editing
//! ```

pub(crate) mod doodads;
pub(crate) mod group;
pub(crate) mod place;
pub(crate) mod wmoareas;
pub(crate) mod wmodoodads;
pub(crate) mod wmolights;
pub(crate) mod wmoview;
pub(crate) mod wmos;
