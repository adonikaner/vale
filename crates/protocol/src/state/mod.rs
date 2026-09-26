//! **What the world *is*** — keyed by guid, or by where the character stands.
//!
//! ```text
//! objects/    the object manager: every entity the server has described, and
//!             UNIT_FIELD_AURA's 48 slots packed three ways
//! update.rs   …and what fills it: SMSG_UPDATE_OBJECT, parsed
//! fields.rs   the update-field indices, one module per type
//! query.rs    …and the three questions this client asks about an id
//! movement/   …and where any of it *is*: MovementInfo, MSG_MOVE_*, the local
//!             simulation, and the spline a flying one is walked along
//! ```
//!
//! **This is the `world/` module, under a different name.** The
//! crate already has a `world.rs` — the mangosd socket — and a `protocol::world`
//! that meant one thing yesterday and another thing today is worse than a name
//! nobody chose.

pub mod fields;
pub mod movement;
pub mod objects;
pub mod query;
pub mod update;
