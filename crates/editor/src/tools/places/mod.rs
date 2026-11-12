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
//! mounts.rs      the creatures a flight node's two mount columns name, read
//!                from the world database, and the state of the picker one
//!                is chosen in
//! transports.rs  the Transports half of flightpaths.rs, the boats and
//!                zeppelins: their templates and periods on the server, a
//!                route as one leg per map, a leg drawn by clicking, going to
//!                a leg on another map, the schedule the client computes, and
//!                the period written to follow a route edit
//! triggers.rs    AreaTrigger.dbc's spheres and boxes on the map, picked,
//!                dragged, made and removed, and the server rows that say what
//!                each does: a teleport, an inn, a quest objective
//! graveyards.rs  WorldSafeLocs.dbc's places on the map, where a released
//!                spirit appears, and the server rows that link each to the
//!                zones it serves and give its facing
//! ```

pub(crate) mod flightpaths;
pub(crate) mod graveyards;
pub(crate) mod lights;
pub(crate) mod mounts;
pub(crate) mod transports;
pub(crate) mod triggers;
