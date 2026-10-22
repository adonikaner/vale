//! The client-table rows that are places on the map.
//!
//! ```text
//! lights.rs      Light.dbc's spheres, drawn on the map, picked with the
//!                pointer, flown to and made. A client table whose row is also a
//!                position, so, like flightpaths.rs, it is a DBC tool that
//!                keeps the viewport
//! flightpaths.rs the flight paths: TaxiNodes, TaxiPath and TaxiPathNode,
//!                drawn on the map, their nodes and points picked, dragged,
//!                added and removed. Client tables whose rows are places, so
//!                the tool keeps the viewport, as lights.rs does
//! triggers.rs    AreaTrigger.dbc's spheres and boxes on the map, picked,
//!                dragged, made and removed, and the server rows that say what
//!                each does: a teleport, an inn, a quest objective
//! graveyards.rs  WorldSafeLocs.dbc's places on the map, where a released
//!                spirit appears, and the server rows that link each to the
//!                zones it serves and give its facing
//! ```

pub mod flightpaths;
pub mod graveyards;
pub mod lights;
pub mod triggers;
