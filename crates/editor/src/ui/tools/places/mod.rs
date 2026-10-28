//! The panels of the place tools: the views of `crate::tools::places`.
//!
//! ```text
//! flightpaths.rs the flight path tool's strip over the viewport (what a
//!               click on the ground is armed to make) and its panel (the
//!               selected node, path or point)
//! triggers.rs   the area trigger tool's panel: the selected trigger's shape
//!               and place, and its teleport, inn and quest rows
//! graveyards.rs the graveyard tool's panel: the selected safe place, its
//!               facing, and the zones it serves
//! lights.rs     the light tool's panel: the selected light's form, its
//!               sphere and place, and the buttons that make a light
//! bands.rs      the form for a light's bands, the one table no other row
//!               references: a light's eighteen colours and six numbers over
//!               the day, each drawn as the day it produces rather than as
//!               sixteen pairs of numbers
//! ```

pub(crate) mod bands;
pub(crate) mod flightpaths;
pub(crate) mod graveyards;
pub(crate) mod lights;
pub(crate) mod triggers;
