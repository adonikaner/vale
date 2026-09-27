//! Settings: the files under `WTF\` that the 1.12.1 client reads and writes,
//! read at start or at login and written when the session ends.
//!
//! ```text
//! cvars.rs        WTF\Config.wtf: the CVars, mirrored out of the interpreter
//! savedvars.rs    SavedVariables.lua: the interface's `RegisterForSave`
//!                 globals, and each addon's saved variables
//! keybindings.rs  the two bindings-cache.wtf files, per account and per
//!                 character
//! cameracache.rs  camera-settings.txt: the camera's distance and pitch, per
//!                 character
//! addons.rs       AddOns.txt: which addons each character has enabled, and
//!                 the addon list read from `Interface\AddOns\`
//! ```
//!
//! The formats are in `vale_assets::interface` and the install folder's paths
//! and file writes in `vale_config`. This directory decides when each file is
//! read and written.

pub mod addons;
pub mod cameracache;
pub mod cvars;
pub mod keybindings;
pub mod savedvars;

use bevy::prelude::*;

/// Every plugin in this directory, as one group. See
/// [`crate::render::RenderPlugins`] for why each directory registers its own.
pub struct SettingsPlugins;

impl Plugin for SettingsPlugins {
    fn build(&self, app: &mut App) {
        // `CVarsPlugin` and `SavedVariablesPlugin` read `ClientConfig` in
        // `build`, which `crate::app::core` inserts before adding this group.
        app.add_plugins((
            cvars::CVarsPlugin,
            savedvars::SavedVariablesPlugin,
            keybindings::KeybindingsPlugin,
            cameracache::CameraCachePlugin,
            addons::AddonsPlugin,
        ));
    }
}
