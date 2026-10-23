//! The selected creature or game-object spawn as a thing with a position and
//! a facing: how it is read, how a move or a turn is written, and the keys
//! that move, turn, remove and duplicate it.
//!
//! ## What a spawn has, compared with a doodad
//!
//! A doodad or a WMO placement carries a position, three angles and (for a
//! doodad) a scale. A `creature` row carries `position_x`, `position_y`,
//! `position_z` and `orientation`, and nothing else about where it stands. A
//! `gameobject` row carries the same four plus the `rotation0..3` quaternion.
//! The client draws a server-spawned game object as `T · Rz(facing)` and does
//! not read the quaternion's tilt (see
//! `vale_assets::world::collision::object_matrix`), so a tilt edited here
//! would not be visible in the editor or in a playtest. Both kinds therefore
//! get the doodad tool's controls for position and for the turn about the
//! world's up, and none for the other two axes or for scale.
//!
//! ## Where an edit goes
//!
//! Every write goes through `EditSession::set_server_edit` with a
//! `Gesture`, under the same labels and subjects the two tools' own pointer
//! drags use (`creature {guid} position`, `gameobject {guid} facing`). A move
//! made with the gizmo, the arrow keys and the pointer in quick succession
//! therefore folds into one undo entry. A game object's facing is written by
//! `GameObjectSubject::face`, which also writes `rotation2` and `rotation3` for an
//! upright object so the quaternion the client is sent agrees with the facing.
//!
//! ## The keys
//!
//! The doodad tool's: the arrows move along x and y, `PageUp` and `PageDown`
//! along z, `,` and `.` turn about z, `Shift` multiplies each step by ten,
//! `Delete` removes, `Ctrl+D` duplicates. `Alt` with the mouse turns, and is
//! in `crate::tools::placements::gizmo::spin`, which serves all four tools.
//!
//! While a new spawn is on the cursor in Place, `,` and `.` and `Alt` with
//! the mouse turn that spawn and not the selection, in the doodad placer's
//! steps: [`PLACE_TURN`] degrees a press, three times that with `Shift`, and
//! `Ctrl` with `Alt` snaps to it. See `SpawnTool::turn_ghost`.

use crate::tools::spawns::creatures::Creatures;
use crate::tools::spawns::gameobjects::GameObjects;
use super::{SpawnSubject, SpawnTool, SpawnWrites, Spawned};
use crate::session::EditSession;
use crate::tools::Tool;
use bevy::prelude::*;
use vale_mangos::row::Edits;

/// Yards an arrow key or `PageUp`/`PageDown` moves a spawn. The doodad tool's
/// step, because a creature is about the size of a doodad.
pub(crate) const STEP: f32 = 0.5;

/// Degrees `,` and `.` turn a spawn.
pub(crate) const TURN: f32 = 5.0;

/// Degrees `,` and `.` turn a spawn that is on the cursor, not yet placed,
/// and the step a held `Ctrl` snaps its `Alt` turn to. The doodad placer's
/// step: a new spawn is turned to roughly the right way and corrected after.
pub(crate) const PLACE_TURN: f32 = 15.0;

/// A facing turned by `degrees`, in radians, wrapped into one turn.
///
/// With `snap`, the result is the nearest multiple of [`PLACE_TURN`], which
/// is what a held `Ctrl` asks of the `Alt` turn.
pub(crate) fn turned(facing: f32, degrees: f32, snap: bool) -> f32 {
    let to = facing.to_degrees() + degrees;
    let to = match snap {
        true => (to / PLACE_TURN).round() * PLACE_TURN,
        false => to,
    };
    to.rem_euclid(360.0).to_radians()
}

/// The selected spawn: which row, where it stands and which way it faces, as
/// the project's edits leave it, and the subject that writes it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Standing {
    /// The subject's writes: which table the row is in and how a move and a
    /// turn are written.
    pub(crate) writes: &'static dyn SpawnWrites,
    pub(crate) guid: u64,
    /// World position, in WoW axes: x north, y west, z up.
    pub(crate) at: Vec3,
    /// Radians anticlockwise from north, as the `orientation` column stores it.
    pub(crate) orientation: f32,
    /// Whether the row's turn is more than its facing. A tilted game object's
    /// facing is written without the quaternion; see
    /// `crate::tools::spawns::gameobjects::GameObjectSubject::face`.
    pub(crate) tilted: bool,
}

/// The selected spawn of whichever of the two tools is chosen, or `None` for
/// any other tool or when nothing is selected.
pub(crate) fn chosen(tool: Tool, creatures: &Creatures, objects: &GameObjects, edits: Option<&Edits>) -> Option<Standing> {
    match tool {
        Tool::Creatures => creatures.standing(edits),
        Tool::GameObjects => objects.standing(edits),
        _ => None,
    }
}

/// The rest of the chosen tool's selection, beside the primary [`chosen`]
/// answers, as the project's edits leave each. A member no longer on the map is
/// left out.
pub(crate) fn members(tool: Tool, creatures: &Creatures, objects: &GameObjects, edits: Option<&Edits>) -> Vec<Standing> {
    match tool {
        Tool::Creatures => creatures.member_standings(edits),
        Tool::GameObjects => objects.member_standings(edits),
        _ => Vec::new(),
    }
}

/// Move and turn a group of spawns by one step and one turn, as one undo entry.
///
/// Each member is carried round `pivot` — the primary's position — by `turn`
/// radians about up, then moved by `step`, and turned by the same `turn` about
/// its own position. `what` is the undo entry's own word for it ("position",
/// "facing"), which keeps a run of arrow presses one entry and a turn after it
/// a second.
pub(crate) fn move_group(
    session: &mut EditSession,
    members: &[Standing],
    pivot: Vec3,
    step: Vec3,
    turn: f32,
    what: &str,
    now: f64,
) {
    let Some(first) = members.first() else {
        return;
    };
    let noun = first.writes.nouns();
    let label = match what {
        "facing" => format!("Turn {} {noun}", members.len()),
        _ => format!("Move {} {noun}", members.len()),
    };
    let subject = format!("{} group {} {what}", first.writes.table(), first.guid);
    let spin = crate::tools::placements::group::turn_about(Vec3::Z, turn.to_degrees());
    session.as_one(&label, &subject, |session| {
        for member in members {
            let to = crate::tools::placements::group::orbit(pivot, member.at, spin) + step;
            if to != member.at {
                member.move_to(session, to, now);
            }
            if turn != 0.0 {
                member.face(session, member.orientation + turn, now);
            }
        }
    });
}

impl Standing {
    /// A spawn of subject `S`, as it is drawn.
    pub(crate) fn of<S: SpawnSubject>(spawn: &S::Spawn) -> Standing {
        Standing {
            writes: S::writes(),
            guid: spawn.guid(),
            at: spawn.at(),
            orientation: spawn.orientation(),
            tilted: spawn.tilted(),
        }
    }

    /// The word for it in an undo label and a status line.
    pub(crate) fn noun(&self) -> &'static str {
        self.writes.noun()
    }

    /// Write the three position columns, under the gesture subject the tool's
    /// own drag uses for the same row.
    pub(crate) fn move_to(&self, session: &mut EditSession, to: Vec3, now: f64) {
        self.writes.move_to(session, self.guid, to, now);
    }

    /// Write the facing, in radians, brought into `0..2π`.
    pub(crate) fn face(&self, session: &mut EditSession, orientation: f32, now: f64) {
        let orientation = orientation.rem_euclid(std::f32::consts::TAU);
        self.writes.face(session, self.guid, self.tilted, orientation, now);
    }
}

/// Registers [`keys`] for both tools.
pub(crate) struct SpawnKeysPlugin;

impl Plugin for SpawnKeysPlugin {
    fn build(&self, app: &mut App) {
        use crate::tools::spawns::creatures::CreatureSubject;
        use crate::tools::spawns::gameobjects::GameObjectSubject;
        app.add_systems(Update, (keys::<CreatureSubject>, keys::<GameObjectSubject>));
    }
}

/// Move, turn, remove and duplicate the selected spawn from the keyboard.
/// The keys are listed in the module comment and on both tools' panels.
#[allow(clippy::too_many_arguments)]
pub(crate) fn keys<S: SpawnSubject>(
    mut spawns: ResMut<SpawnTool<S>>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    time: Res<Time>,
    mut menu: ResMut<crate::context::ContextMenu>,
) {
    if *tool != S::TOOL || !state.editing() {
        return;
    }
    // The right-click menu's Delete and Duplicate, carried out here so the
    // item and the key are one operation. A menu item keeps egui's focus
    // after it is pressed, so a request is answered whatever has the
    // keyboard.
    use crate::context::Request;
    let request = menu.take_request(|request| matches!(request, Request::Delete | Request::Duplicate));
    if request.is_none() && wants.wants_keyboard_input() {
        return;
    }
    // While a spawn is on the cursor the keys are the placer's: `,` and `.`
    // turn it, and nothing reaches the selection, whose form the panel is not
    // showing. `Escape` is the tool's own `ghost`.
    if spawns.placing() {
        let far = match keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
            true => 3.0,
            false => 1.0,
        };
        let mut turn = 0.0;
        if keys.just_pressed(KeyCode::Comma) {
            turn -= PLACE_TURN * far;
        }
        if keys.just_pressed(KeyCode::Period) {
            turn += PLACE_TURN * far;
        }
        if turn != 0.0 {
            spawns.turn_ghost(turn, false);
        }
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    let Some(was) = spawns.standing(Some(&session.rows.edits)) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let control = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let delete = keys.just_pressed(KeyCode::Delete) || request == Some(Request::Delete);
    let duplicate = (control && keys.just_pressed(KeyCode::KeyD)) || request == Some(Request::Duplicate);
    // The group is looked up only on a frame a key is pressed: finding a member
    // is a walk of the map's spawns, and most frames press nothing.
    if !spawns.also.is_empty() {
        if request.is_none() && !KEYS.iter().any(|&key| keys.just_pressed(key)) {
            return;
        }
        let others = spawns.member_standings(Some(&session.rows.edits));
        group_keys(&mut spawns, session, &keys, was, others, Asked { control, delete, duplicate }, now);
        return;
    }

    // Ctrl+D. `tools::shortcuts` handles it for doodads and WMOs and does not
    // name these two tools, so the two cannot both act on one press.
    if duplicate {
        session.status = match spawns.duplicate(session, now) {
            Some(guid) => format!("duplicated as guid {guid}"),
            None => "not duplicated: the spawn row has not loaded yet".to_string(),
        };
        return;
    }
    // Ctrl+Z, Ctrl+Y and Ctrl+S are `tools::shortcuts`; a step key pressed
    // with Ctrl held is not a nudge.
    if control {
        return;
    }

    if delete {
        let created = spawns.chosen().is_some_and(|spawn| spawn.is_new());
        SpawnTool::<S>::remove(session, was.guid, now);
        // A spawn this project created has no row once its claim is dropped,
        // so the selection is cleared, as the panels' Remove buttons do.
        session.status = match created {
            true => format!("{} {} discarded", was.noun(), was.guid),
            false => format!("{} {} is marked for removal", was.noun(), was.guid),
        };
        if created {
            spawns.select_only(None);
        }
        return;
    }

    let far = match keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        true => 10.0,
        false => 1.0,
    };
    let step = step_of(&keys);
    if step != Vec3::ZERO {
        was.move_to(session, was.at + step * STEP * far, now);
    }
    let turn = turn_of(&keys) * far;
    if turn != 0.0 {
        was.face(session, was.orientation + turn.to_radians(), now);
    }
}

/// The step the arrow keys and `PageUp`/`PageDown` pressed this frame ask for,
/// one yard per key.
fn step_of(keys: &ButtonInput<KeyCode>) -> Vec3 {
    let mut step = Vec3::ZERO;
    for (key, direction) in [
        (KeyCode::ArrowUp, Vec3::X),
        (KeyCode::ArrowDown, Vec3::NEG_X),
        (KeyCode::ArrowLeft, Vec3::Y),
        (KeyCode::ArrowRight, Vec3::NEG_Y),
        (KeyCode::PageUp, Vec3::Z),
        (KeyCode::PageDown, Vec3::NEG_Z),
    ] {
        if keys.just_pressed(key) {
            step += direction;
        }
    }
    step
}

/// The turn `,` and `.` pressed this frame ask for, in degrees.
fn turn_of(keys: &ButtonInput<KeyCode>) -> f32 {
    let mut turn = 0.0;
    if keys.just_pressed(KeyCode::Comma) {
        turn -= TURN;
    }
    if keys.just_pressed(KeyCode::Period) {
        turn += TURN;
    }
    turn
}

/// Every key [`keys`] answers. A group is looked up only on a frame one of
/// these is pressed.
const KEYS: [KeyCode; 9] = [
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Comma,
    KeyCode::Period,
    KeyCode::Delete,
];

/// What [`keys`] read from the keyboard and the right-click menu, for
/// [`group_keys`].
struct Asked {
    /// `Ctrl` is held.
    control: bool,
    /// `Delete`, or the menu's Delete.
    delete: bool,
    /// `Ctrl+D`, or the menu's Duplicate.
    duplicate: bool,
}

/// The keys over a group of spawns: the primary `was` and the `others`.
///
/// The arrows and `,`/`.` move and turn every member as one undo entry — a
/// turn carries the group round the primary. `Delete` removes every member as
/// one entry; a spawn this project created is discarded rather than marked, and
/// leaves the selection, as the single-spawn `Delete` does.
///
/// `Ctrl+D` does not copy a group. A spawn's copy carries every column of its
/// row, and the whole row is read for the primary only, so copying the others
/// would write rows with the table's defaults in place of their own columns.
fn group_keys<S: SpawnSubject>(
    spawns: &mut SpawnTool<S>,
    session: &mut EditSession,
    keys: &ButtonInput<KeyCode>,
    was: Standing,
    others: Vec<Standing>,
    asked: Asked,
    now: f64,
) {
    let mut all = vec![was];
    all.extend(others);
    let noun = S::WORDS.nouns;
    if asked.duplicate {
        session.status = format!("{} {noun} selected; Ctrl+D duplicates one spawn at a time", all.len());
        return;
    }
    if asked.control {
        return;
    }

    if asked.delete {
        // Which of them this project created, read before the claims change:
        // those have no row once removed, so they leave the selection.
        let created: Vec<u64> = all
            .iter()
            .filter(|member| {
                session
                    .rows
                    .edits
                    .row(S::SPAWN, &S::spawn_key(member.guid))
                    .is_some_and(|row| row.life == vale_mangos::row::Life::Insert)
            })
            .map(|member| member.guid)
            .collect();
        let label = format!("Remove {} {noun}", all.len());
        let subject = format!("{} group {} removal", S::SPAWN, was.guid);
        session.as_one(&label, &subject, |session| {
            for member in &all {
                SpawnTool::<S>::remove(session, member.guid, now);
            }
        });
        let kept = all.len() - created.len();
        session.status = match (kept, created.len()) {
            (_, 0) => format!("{kept} {noun} are marked for removal"),
            (0, gone) => format!("{gone} {noun} discarded"),
            (kept, gone) => format!("{kept} {noun} marked for removal, {gone} discarded"),
        };
        // What is left selected is every member that still has a row, the
        // primary first when it is one of them.
        let survivors: Vec<u64> = all.iter().map(|member| member.guid).filter(|guid| !created.contains(guid)).collect();
        spawns.select_only(survivors.first().copied());
        spawns.also = survivors.iter().skip(1).copied().collect();
        return;
    }

    let far = match keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        true => 10.0,
        false => 1.0,
    };
    let step = step_of(keys);
    if step != Vec3::ZERO {
        move_group(session, &all, was.at, step * STEP * far, 0.0, "position", now);
    }
    let turn = turn_of(keys) * far;
    if turn != 0.0 {
        move_group(session, &all, was.at, Vec3::ZERO, turn.to_radians(), "facing", now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spawn on the cursor turns by the step asked, wraps at a full turn,
    /// and snaps to the placer's step when asked.
    #[test]
    fn a_ghost_turns_wraps_and_snaps() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        assert!(close(turned(0.0, PLACE_TURN, false), 15f32.to_radians()));
        assert!(close(turned(0.0, -PLACE_TURN, false), 345f32.to_radians()));
        assert!(close(turned(350f32.to_radians(), 20.0, false), 10f32.to_radians()));
        assert!(close(turned(40f32.to_radians(), 4.0, true), 45f32.to_radians()));
        assert!(close(turned(40f32.to_radians(), -4.0, true), 30f32.to_radians()));
    }
}
