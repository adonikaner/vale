//! The editor's global keys: undo, redo, save, the playtest, and the shell
//! over one.

use crate::session::EditSession;
use bevy::prelude::*;
use super::{Tool, open_on, placements};

/// Undo, redo, save, the playtest, and the shell over one.
///
/// Three of the six work during a playtest, each for its own reason. `Ctrl+P`
/// ends the playtest. `Ctrl+E` opens and closes the editor's panels over it —
/// see [`crate::playtest::ShellOpen`]. `Ctrl+S` writes the project and
/// republishes, which makes an edit made during a playtest reach the running
/// game without leaving it.
///
/// All of them check the game's own keyboard focus as well as egui's:
/// `KeyboardFocus` says whether an edit box or an `enableKeyboard` frame took
/// the keystroke. Without that check, a `Ctrl+P` typed into the chat line
/// would end the session it was typed in.
#[allow(clippy::too_many_arguments)]
pub(crate) fn shortcuts(
    mut session: Option<ResMut<EditSession>>,
    mut state: ResMut<crate::playtest::Playtest>,
    mut shell: ResMut<crate::playtest::ShellOpen>,
    mut client: ResMut<vale_client::world::session::Session>,
    login: Res<crate::playtest::Login>,
    mut auto: ResMut<vale_client::glue::autologin::AutoLogin>,
    keys: Res<ButtonInput<KeyCode>>,
    typing: Res<vale_client::lua::api::keyboard::KeyboardFocus>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut tool: ResMut<Tool>,
    assets: Res<vale_client::assets::GameAssets>,
    doodads: Res<placements::doodads::Selection>,
    wmos: Res<placements::wmos::Selection>,
    mut placing: ResMut<placements::place::Placing>,
    // What a save and a playtest need to reach the server: the reload queue,
    // where the database is, and whether the next session keeps its query
    // answers. See [`crate::server::Reach`], which bundles them into one
    // parameter because this system is at Bevy's limit of sixteen.
    mut reach: crate::server::Reach,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if wants.wants_keyboard_input() || !keys.pressed(KeyCode::ControlLeft) {
        return;
    }
    if keys.just_pressed(KeyCode::KeyP) && !typing.active {
        match state.editing() {
            true => crate::playtest::start(
                &mut state,
                session,
                &assets,
                &login,
                &mut auto,
                &mut reach.queue,
                &reach.settings,
                &mut reach.caches,
            ),
            false => crate::playtest::stop(&mut state, &mut client, &mut auto, session),
        }
        return;
    }
    // The shell over a playtest. This is the only shortcut here that does
    // nothing while editing, because the panels are already drawn then.
    if keys.just_pressed(KeyCode::KeyE) && !typing.active && state.playing() {
        shell.0 = !shell.0;
        if shell.0 {
            *tool = open_on(*tool);
        }
        return;
    }
    // Save works both while editing and during a playtest. During a playtest
    // it performs the whole reload: the write puts the edit in the project
    // folder and [`crate::playtest::republish`] puts it where the running
    // client reads it, so the next cast, the next entity and the next model
    // are the edited ones.
    if keys.just_pressed(KeyCode::KeyS) && !typing.active {
        session.save_all();
        session.save_all_tables();
        // The server half of the same key press: every subject's SQL, and,
        // when Apply on save is on, the statements run and the tables
        // reloaded. See [`crate::server::save`], which holds the only list of
        // subjects.
        crate::server::save(session, &assets, &reach.settings, &mut reach.queue);
        if state.playing() {
            crate::playtest::republish(session, &assets);
            session.status = "saved and republished to the playtest".to_string();
        }
        return;
    }
    if !state.editing() {
        return;
    }
    // Undo and redo mark what they moved, exactly as a stroke does, so the
    // ground and the placements follow them the same way.
    if keys.just_pressed(KeyCode::KeyZ) {
        session.undo(&assets);
    }
    if keys.just_pressed(KeyCode::KeyY) {
        session.redo(&assets);
    }
    // Duplicate arms the placement tool rather than copying — see [`placements::place`]
    // for the reason. It is here rather than in either tool because it is one
    // key doing one thing to whichever list is open.
    //
    // One placement only. A group cannot ride on the cursor as one model,
    // so over several `Ctrl+D` is `placements::group::clipboard`'s, which copies them in
    // place a step to the north.
    if keys.just_pressed(KeyCode::KeyD) {
        let armed = match *tool {
            Tool::Doodads if !doodads.also.is_empty() => None,
            Tool::Wmos if !wmos.also.is_empty() => None,
            Tool::Doodads => doodads.at.as_ref().map(|at| {
                let turns =
                    vale_assets::world::adt::placement_euler_to_world(at.record.rotation);
                (
                    placements::place::Kind::Doodad,
                    at.path.clone(),
                    turns[2],
                    f32::from(at.record.scale) / 1024.0,
                )
            }),
            Tool::Wmos => wmos.at.as_ref().map(|at| {
                let turns =
                    vale_assets::world::adt::placement_euler_to_world(at.record.rotation);
                (placements::place::Kind::Wmo, at.path.clone(), turns[2], 1.0)
            }),
            _ => None,
        };
        if let Some((kind, path, turn, scale)) = armed {
            placing.arm(kind, path, turn, scale);
            session.status = format!("duplicating {}", placements::place::leaf(&placing_path(&placing)));
        }
    }
}

/// The armed path, for the status line.
fn placing_path(placing: &placements::place::Placing) -> String {
    placing.path.clone().unwrap_or_default()
}
