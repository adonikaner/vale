//! The editor's global keys: undo, redo, save, the playtest, the shell over
//! one, and `F`, which moves the camera to the selection. The shell's plugin,
//! `crate::ui::PanelPlugin`, registers them.

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
    // In a shared session only the host saves: a joined editor's save
    // submits to the host. A joined editor's playtest applies to its server
    // only when the session allows it. See `crate::collab`.
    mut collab: ResMut<crate::collab::Collab>,
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
                &collab.server_gate(),
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
    // A full save a panel or the frame's work asked for: a table's Save
    // button, the end of a sweep, another author's Submit. See
    // `EditSession::save_asked`.
    let asked = std::mem::take(&mut session.save_asked);
    if collab.is_joined() && (asked || taken.s) {
        collab.ask = Some(crate::collab::Ask::Submit);
        return;
    }
    if asked {
        crate::server::save_everything(session, &assets, &reach.settings, &mut reach.queue, state.playing());
    }
    if taken.s {
        crate::server::save_everything(session, &assets, &reach.settings, &mut reach.queue, state.playing());
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

/// The camera request that shows a selection: one place is looked at from the
/// camera's current distance, and several are framed from a distance that
/// fits the box they span. `None` for no places.
pub(crate) fn framing(places: &[Vec3]) -> Option<crate::camera::FlyTo> {
    let first = *places.first()?;
    if places.len() == 1 {
        return Some(crate::camera::FlyTo::at(first));
    }
    let (low, high) = places.iter().fold((first, first), |(low, high), at| (low.min(*at), high.max(*at)));
    Some(crate::camera::FlyTo {
        at: (low + high) * 0.5,
        distance: Some(((high - low).length() * 1.2).clamp(GROUP_NEAREST, GROUP_FARTHEST)),
    })
}

/// The nearest and the farthest the camera stands to frame a group, in yards.
/// The farthest is about what the editor streams.
const GROUP_NEAREST: f32 = 20.0;
const GROUP_FARTHEST: f32 = 1500.0;

/// `F`: move the camera to the chosen tool's selection, as the Fly To button
/// of that tool's panel does.
///
/// ```text
/// doodads, WMOs               the selected placement, or the group framed
/// creatures, game objects     the selected spawn, or the group framed
/// lights, area triggers,      the selected row's place
/// graveyards
/// flight paths                the selected point, else the selected path
///                             framed, else the selected node
/// ```
///
/// Nothing happens for a tool with no such selection, while a text field or
/// the game has the keyboard, with `Ctrl` or `Alt` held, or during a
/// playtest, where `F` is the game's.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fly_to_selection(
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    typing: Res<vale_client::lua::api::keyboard::KeyboardFocus>,
    state: Res<crate::playtest::Playtest>,
    tool: Res<Tool>,
    session: Option<Res<EditSession>>,
    doodads: Res<placements::doodads::Selection>,
    wmos: Res<placements::wmos::Selection>,
    creatures: Res<super::spawns::creatures::Creatures>,
    objects: Res<super::spawns::gameobjects::GameObjects>,
    lights: Res<super::places::lights::Lights>,
    flights: Res<super::places::flightpaths::Flightpaths>,
    triggers: Res<super::places::triggers::Triggers>,
    graveyards: Res<super::places::graveyards::Graveyards>,
    mut fly: MessageWriter<crate::camera::FlyTo>,
) {
    use crate::tools::points::MapPoint;
    let held = |key: KeyCode| keys.pressed(key);
    let modified = held(KeyCode::ControlLeft) || held(KeyCode::ControlRight) || held(KeyCode::AltLeft) || held(KeyCode::AltRight);
    if !keys.just_pressed(KeyCode::KeyF) || modified || !state.editing() || wants.wants_keyboard_input() || typing.active {
        return;
    }
    let world = |position: [f32; 3]| Vec3::from(vale_assets::world::adt::placement_to_world(position));
    let flight = match *tool {
        Tool::Doodads => {
            let places: Vec<Vec3> = doodads.at.iter().chain(&doodads.also).map(|at| world(at.record.position)).collect();
            framing(&places)
        }
        Tool::Wmos => {
            let places: Vec<Vec3> = wmos.at.iter().chain(&wmos.also).map(|at| world(at.record.position)).collect();
            framing(&places)
        }
        Tool::Creatures | Tool::GameObjects => {
            let edits = session.as_ref().map(|session| &session.rows.edits);
            let places: Vec<Vec3> = super::spawns::spawn::chosen(*tool, &creatures, &objects, edits)
                .into_iter()
                .chain(super::spawns::spawn::members(*tool, &creatures, &objects, edits))
                .map(|standing| standing.at)
                .collect();
            framing(&places)
        }
        Tool::Lights => lights.chosen().map(super::places::lights::Lights::fly_to),
        Tool::Flightpaths => match (flights.point.and_then(|id| flights.point(id)), flights.path, flights.node) {
            (Some((_, point)), _, _) => Some(crate::camera::FlyTo::at(point.at.into())),
            (None, Some(path), _) => flights.path_flight(path),
            (None, None, Some(node)) => flights.node(node).map(|node| crate::camera::FlyTo::at(node.at.into())),
            _ => None,
        },
        Tool::Triggers => triggers
            .points
            .selected
            .and_then(|id| triggers.points.get(id))
            .map(|trigger| crate::camera::FlyTo::at(trigger.position())),
        Tool::Graveyards => graveyards
            .points
            .selected
            .and_then(|id| graveyards.points.get(id))
            .map(|place| crate::camera::FlyTo::at(place.position())),
        _ => None,
    };
    if let Some(flight) = flight {
        fly.write(flight);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One place is looked at from where the camera stands; several are
    /// framed from a distance that fits them, within the two limits.
    #[test]
    fn a_selection_is_looked_at_or_framed() {
        assert_eq!(framing(&[]), None);
        let one = Vec3::new(10.0, 20.0, 30.0);
        assert_eq!(framing(&[one]), Some(crate::camera::FlyTo::at(one)));
        let group = framing(&[Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0)]).expect("a request");
        assert_eq!(group.at, Vec3::new(50.0, 0.0, 0.0));
        assert!(group.distance.is_some_and(|distance| (distance - 120.0).abs() < 1e-3), "{:?}", group.distance);
        let close = framing(&[Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)]).expect("a request");
        assert_eq!(close.distance, Some(GROUP_NEAREST));
        let far = framing(&[Vec3::ZERO, Vec3::new(9000.0, 0.0, 0.0)]).expect("a request");
        assert_eq!(far.distance, Some(GROUP_FARTHEST));
    }
}
