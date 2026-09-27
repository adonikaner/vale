//! The screens that are not the world: the login and character screens, the
//! character creation screen, and the loading screen between worlds.
//!
//! ```text
//! glue.rs        the client's side of Interface\GlueXML\: the login and
//!                character screens, driven by the game's own Lua
//! charcreate.rs  the character creation screen: its one packet and the model
//!                on the plinth
//! autologin.rs   entering the world without either screen, for a run that
//!                names its character
//! loading.rs     the loading screen: when it is up and what is on it
//! ```
//!
//! The 3D scenes behind these screens are drawn by `crate::render::glue`.

pub mod autologin;
pub mod charcreate;
pub mod glue;
pub mod loading;

use bevy::prelude::*;

/// Every plugin in this directory, as one group. See
/// [`crate::render::RenderPlugins`] for why each directory registers its own.
pub struct GluePlugins;

impl Plugin for GluePlugins {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            glue::GluePlugin,
            autologin::AutoLoginPlugin,
            charcreate::CharCreatePlugin,
            loading::LoadingPlugin,
        ));
    }
}
