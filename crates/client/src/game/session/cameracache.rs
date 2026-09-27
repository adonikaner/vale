//! `camera-settings.txt`: the two numbers a character keeps about the camera,
//! its distance behind the character and its pitch.
//!
//! ```text
//! WTF\Account\<A>\<realm>\<character>\camera-settings.txt
//!   cameraDistance 6.919909
//!   cameraPitch 9.049930
//! ```
//!
//! A real 1.12 folder has one beside every character's `layout-cache.txt`.
//! The player sets both numbers with the mouse wheel and the right button and
//! expects them to be kept. The file is read when the character enters the
//! world and written when the character leaves it, like every per-character
//! file here. See [`vale_assets::interface::wtf::CAMERA_SETTINGS_NAME`] for the
//! format and which part of it is inferred.
//!
//! The distance is in yards and is clamped as the mouse wheel clamps it. The
//! pitch is in degrees in the file and in radians on
//! [`crate::world::camera::CameraRig`], clamped to the rig's limit. A missing
//! or empty file leaves the rig where the login placed it.

use bevy::prelude::*;
use std::path::PathBuf;

use vale_assets::interface::wtf;

use crate::world::camera::{CameraRig, CLOSEST, PITCH_LIMIT};
use crate::world::session::{ClientConfig, Session, WorldStatus};

/// The path of this character's file, and which character it was read for.
#[derive(Resource, Default)]
pub struct CameraCache {
    path: Option<PathBuf>,
    /// `(account, realm, character)` the file was read for. The file is read
    /// again when any of the three changes, as for a second character in one
    /// session.
    read_for: Option<(String, String, String)>,
}

pub struct CameraCachePlugin;

impl Plugin for CameraCachePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraCache>()
            .add_systems(Update, load.in_set(super::super::GameSet))
            .add_systems(Update, save_on_logout)
            .add_systems(Last, save);
    }
}

/// Read the file when the character enters the world, and set the two numbers
/// on the rig. Once per character: a second character in the same session
/// reads its own file.
///
/// The account falls back to `Config.wtf`'s `accountName` when the session
/// has none, as for every other file under `WTF\Account\`; see
/// [`vale_config::account_of`].
fn load(
    session: Res<Session>,
    status: Res<WorldStatus>,
    config: Res<ClientConfig>,
    mut cache: ResMut<CameraCache>,
    mut rig: ResMut<CameraRig>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    if !status.in_world || status.character.is_empty() {
        return;
    }
    let key = (
        config.0.account_for([Some(active.account.as_str())]),
        active.realm.clone(),
        status.character.clone(),
    );
    if cache.read_for.as_ref() == Some(&key) {
        return;
    }
    cache.path = wtf::character_file_path(&key.0, &key.1, &key.2, wtf::CAMERA_SETTINGS_NAME)
        .map(|path| config.0.path(path));
    cache.read_for = Some(key);
    let Some(path) = cache.path.as_ref() else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        info!("camera: no {} yet", path.display());
        return;
    };
    let (distance, pitch) = wtf::parse_camera_settings(&text);
    if let Some(distance) = distance {
        rig.distance = distance.clamp(CLOSEST, 50.0);
    }
    if let Some(pitch) = pitch {
        rig.pitch = pitch.to_radians().clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }
    info!(
        "camera: {} -> distance {:.2}, pitch {:.2} deg",
        path.display(),
        rig.distance,
        rig.pitch.to_degrees()
    );
}

/// Write the file when leaving the world, while the rig still belongs to the
/// character.
fn save_on_logout(
    mut leaving: MessageReader<super::super::events::PlayerLeavingWorld>,
    cache: Res<CameraCache>,
    rig: Res<CameraRig>,
) {
    if leaving.read().next().is_none() {
        return;
    }
    write(&cache, &rig);
}

/// Write the file at exit, for the reason [`super::super::cvars::save`] gives.
fn save(mut exits: MessageReader<AppExit>, cache: Res<CameraCache>, rig: Res<CameraRig>) {
    if exits.read().next().is_none() {
        return;
    }
    write(&cache, &rig);
}

fn write(cache: &CameraCache, rig: &CameraRig) {
    let Some(path) = cache.path.as_ref() else {
        return;
    };
    let text = wtf::render_camera_settings(rig.distance, rig.pitch.to_degrees());
    match vale_config::write_file(path, text) {
        Ok(()) => info!("camera settings to {}", path.display()),
        Err(e) => warn!("camera settings not written: {} ({e})", path.display()),
    }
}
