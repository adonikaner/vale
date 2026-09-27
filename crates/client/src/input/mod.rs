//! Input: what a key or a mouse button means.
//!
//! ```text
//! bindings.rs  the key table: a key -> a binding name -> the Lua body or verb
//!              it runs, and the `BindingSet` every reader orders itself after
//! controls.rs  the character's movement, assembled from bindings rather than
//!              from raw keys
//! ```
//!
//! A key is not an action in the 1.12.1 client: `Bindings.xml` names what a
//! key runs, and the player's binding files choose which key. Mouse input over
//! the interface is in `crate::lua::api::mouse`, and the world pick is in
//! `crate::interface::target`.

pub mod bindings;
pub mod controls;

use bevy::prelude::*;

/// Every plugin in this directory, as one group. See
/// [`crate::render::RenderPlugins`] for why each directory registers its own.
pub struct InputPlugins;

impl Plugin for InputPlugins {
    fn build(&self, app: &mut App) {
        // `controls` states its own ordering against `bindings::BindingSet`.
        app.add_plugins((bindings::BindingsPlugin, controls::ControlsPlugin));
    }
}
