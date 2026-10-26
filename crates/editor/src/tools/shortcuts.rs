//! The editor's global keys: undo, redo, save, the playtest, and the shell
//! over one. The shell's plugin, `crate::ui::PanelPlugin`, registers them.

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
///
/// The three keys that work during a playtest are read from [`Taken`], which
/// [`take`] fills before the game's bindings run.
#[allow(clippy::too_many_arguments)]
pub(crate) fn shortcuts(
    mut session: Option<ResMut<EditSession>>,
    mut state: ResMut<crate::playtest::Playtest>,
    mut shell: ResMut<crate::playtest::ShellOpen>,
    mut client: ResMut<vale_client::world::session::Session>,
    login: Res<crate::playtest::Login>,
    mut auto: ResMut<vale_client::glue::autologin::AutoLogin>,
    keys: Res<ButtonInput<KeyCode>>,
    mut taken: ResMut<Taken>,
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
    let taken = std::mem::take(&mut *taken);
    let Some(session) = session.as_mut() else {
        return;
    };
    if !taken.ctrl {
        return;
    }
    if taken.p {
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
    if taken.e {
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
    if taken.s {
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

/// The editor's keys taken this frame, by [`take`], for [`shortcuts`].
#[derive(Resource, Debug, Default, Clone, Copy)]
pub(crate) struct Taken {
    /// `Ctrl` is held and egui does not want the keyboard: the editor's other
    /// `Ctrl` keys, undo, redo and duplicate, may act.
    ctrl: bool,
    /// `Ctrl+S`, `Ctrl+P`, and during a playtest `Ctrl+E`, pressed with the
    /// game's keyboard free as well.
    s: bool,
    p: bool,
    e: bool,
}

/// Take the editor's `Ctrl+S`, `Ctrl+P` and `Ctrl+E` before the game's key
/// bindings read the keyboard, and clear each one's press, so the binding the
/// game has for the same key does not also fire. `CTRL-S` is `TOGGLESOUND`
/// in the shipped `WTF\DefaultBindings.wtf`, `CTRL-P`
/// `RESETPERFORMANCEVALUES` and `CTRL-E` `TOGGLECOLLISION`. A save during a
/// playtest also switched the game's sound effects off, and the client wrote
/// `MasterSoundEffects 0` to `WTF\Config.wtf`, which silenced every sound
/// effect the editor plays from then on.
///
/// A system of its own because [`shortcuts`] runs after the pointer's pick,
/// which runs after the game's bindings; this has no other ordering, and
/// runs before `vale_client::input::bindings::BindingSet`.
pub(crate) fn take(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    typing: Res<vale_client::lua::api::keyboard::KeyboardFocus>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    state: Res<crate::playtest::Playtest>,
    mut taken: ResMut<Taken>,
) {
    let ctrl = !wants.wants_keyboard_input() && keys.pressed(KeyCode::ControlLeft);
    let mut press = |key: KeyCode, allowed: bool| {
        let pressed = ctrl && allowed && !typing.active && keys.just_pressed(key);
        if pressed {
            keys.clear_just_pressed(key);
        }
        pressed
    };
    let s = press(KeyCode::KeyS, true);
    let p = press(KeyCode::KeyP, true);
    let e = press(KeyCode::KeyE, state.playing());
    *taken = Taken { ctrl, s, p, e };
}

/// The armed path, for the status line.
fn placing_path(placing: &placements::place::Placing) -> String {
    placing.path.clone().unwrap_or_default()
}
