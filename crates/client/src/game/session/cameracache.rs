//! **`camera-settings.txt` — the two numbers a character keeps about the
//! camera**: how far back it sits and how steep it looks.
//!
//! ```text
//! WTF\Account\<A>\<realm>\<character>\camera-settings.txt
//!   cameraDistance 6.919909
//!   cameraPitch 9.049930
//! ```
//!
//! A real 5875 folder has one beside every character's `layout-cache.txt`,
//! and the two numbers are the ones a player sets with the wheel and the
//! right button and expects to find where they left them. Read once when the
//! character lands in the world, written on the way out of it — the same two
//! ends every per-character file here has. See
//! [`vale_assets::interface::wtf::CAMERA_SETTINGS_NAME`] for the format
//! and what about it is a reading.
//!
//! The distance is yards and clamped the way the wheel clamps it; the pitch
//! is degrees in the file and radians on [`crate::world::camera::CameraRig`],
//! clamped to the rig's own limit. A file that is missing or says nothing
//! leaves the rig where the login put it.

use bevy::prelude::*;
use std::path::PathBuf;

use vale_assets::interface::wtf;

use crate::world::camera::{CameraRig, CLOSEST, PITCH_LIMIT};
use crate::world::session::{Session, WorldStatus};

/// Where this character's file is, and which character it was read for.
#[derive(Resource, Default)]
pub struct CameraCache {
    path: Option<PathBuf>,
    /// `(account, realm, character)` the file was read for — re-read when any
    /// of the three changes, which is a second character in one session.
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

/// **Read the file when the character lands**, and put the two numbers on the
/// rig. Once per character: a second character of the same session reads its
/// own.
fn load(
    session: Res<Session>,
    status: Res<WorldStatus>,
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
        active.account.clone(),
        active.realm.clone(),
        status.character.clone(),
    );
    if cache.read_for.as_ref() == Some(&key) {
        return;
    }
    cache.path = wtf::character_file_path(&key.0, &key.1, &key.2, wtf::CAMERA_SETTINGS_NAME)
        .map(PathBuf::from);
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

/// **Write it on the way out of the world**, while the rig is still the
/// character's.
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

/// …and at exit, for the reason [`super::super::cvars::save`] gives.
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
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            warn!("camera settings not written: {} ({e})", dir.display());
            return;
        }
    }
    let text = wtf::render_camera_settings(rig.distance, rig.pitch.to_degrees());
    match std::fs::write(path, text) {
        Ok(()) => info!("camera settings to {}", path.display()),
        Err(e) => warn!("camera settings not written: {} ({e})", path.display()),
    }
}
