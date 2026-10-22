//! The windows of the spawn tools: the views of `crate::tools::spawns`.
//!
//! ```text
//! creatures.rs  the form for the server's creature rows: one spawn, the
//!               creature_template behind it, and which of the two an edit
//!               changes. It is separate from the table browser
//!               (`crate::ui::tables`) because a DBC row is an index
//!               into a fixed-width record and a server row is a keyed map of
//!               text
//! gameobjects.rs the same form for a game object: one spawn, the
//!               gameobject_template behind it, the 24 data columns under the
//!               names the row's type gives them, and a lock resolved through
//!               the client's Lock.dbc
//! waypoints.rs  the window one creature's path is edited in: its points as a
//!               list, beside the same points drawn on the ground. The list
//!               and the world share one selection, so they always agree on
//!               which point is chosen
//! ```

pub mod creatures;
pub mod gameobjects;
pub mod waypoints;
