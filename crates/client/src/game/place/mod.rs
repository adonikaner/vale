//! **Where the character is, and what happens when that changes.**
//!
//! ```text
//! controls.rs    what the character is being told to do — the eight held
//!                controls, assembled from bindings rather than from keys
//! worldmap.rs    the parchment: where the character is on it, and which one
//! minimap.rs     …and the little round one: the four facts the widget cannot know
//! areatrigger.rs walking into a dungeon portal — of which the client half is
//!                only the refusal, since the success is an ordinary teleport
//! loading.rs     …and what is on the screen while the far side arrives
//! ```

pub mod areatrigger;
pub mod controls;
pub mod loading;
pub mod minimap;
pub mod worldmap;
