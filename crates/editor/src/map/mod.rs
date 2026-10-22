//! The map as a whole, rather than a place on it: a new map, a new tile, a
//! find-and-replace over every tile, and the tiles opened for editing around
//! the focus. The map window drives the first three.
//!
//! ```text
//! maps.rs        a new map: its Map.dbc row, a WDT with no tiles in the
//!                project, and its map_template row with a dungeon's parent
//!                and ghost entrance
//! tiles.rs       the tile itself: making ground where there was none, and the
//!                shadow bake and minimap picture that no other edit keeps in
//!                step
//! sweep.rs       a find-and-replace over every tile of the map: the paths in
//!                MTEX, MMDX and MWMO, most of which are not open
//! open.rs        the tiles around the focus read and parsed for the session
//!                on the task pool, and the tiles it has left closed
//! ```

pub mod maps;
pub(crate) mod open;
pub mod sweep;
pub mod tiles;
