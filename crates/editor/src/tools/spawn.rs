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
//! `GameObjects::turn`, which also writes `rotation2` and `rotation3` for an
//! upright object so the quaternion the client is sent agrees with the facing.
//!
//! ## The keys
//!
//! The doodad tool's: the arrows move along x and y, `PageUp` and `PageDown`
//! along z, `,` and `.` turn about z, `Shift` multiplies each step by ten,
//! `Delete` removes, `Ctrl+D` duplicates. `Alt` with the mouse turns, and is
//! in `super::gizmo::spin`, which serves all four tools.

use super::creatures::Creatures;
use super::gameobjects::GameObjects;
use super::Tool;
use crate::session::{EditSession, Gesture};
use vale_mangos::row::{Edits, Key};
use vale_mangos::{creature, gameobject};
use bevy::prelude::*;

/// Yards an arrow key or `PageUp`/`PageDown` moves a spawn. The doodad tool's
/// step, because a creature is about the size of a doodad.
pub const STEP: f32 = 0.5;

/// Degrees `,` and `.` turn a spawn.
pub const TURN: f32 = 5.0;

/// Which table the selected spawn is a row of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Creature,
    /// `tilted` is whether `rotation0` or `rotation1` is non-zero. A tilted
    /// object's facing is written without the quaternion; see
    /// `GameObjects::turn`.
    Object { tilted: bool },
}

/// The selected spawn: which row, where it stands and which way it faces, as
/// the project's edits leave it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Standing {
    pub kind: Kind,
    pub guid: u64,
    /// World position, in WoW axes: x north, y west, z up.
    pub at: Vec3,
    /// Radians anticlockwise from north, as the `orientation` column stores it.
    pub orientation: f32,
}

/// The selected spawn of whichever of the two tools is chosen, or `None` for
/// any other tool or when nothing is selected.
pub fn chosen(
    tool: Tool,
    creatures: &Creatures,
    objects: &GameObjects,
    edits: Option<&Edits>,
) -> Option<Standing> {
    match tool {
        Tool::Creatures => creatures.chosen_edited(edits).map(|spawn| Standing {
            kind: Kind::Creature,
            guid: spawn.guid,
            at: spawn.at,
            orientation: spawn.orientation,
        }),
        Tool::GameObjects => objects.chosen_edited(edits).map(|spawn| Standing {
            kind: Kind::Object {
                tilted: spawn.tilted,
            },
            guid: spawn.guid,
            at: spawn.at,
            orientation: spawn.orientation,
        }),
        _ => None,
    }
}

/// The rest of the chosen tool's selection, beside the primary [`chosen`]
/// answers, as the project's edits leave each. A member no longer on the map is
/// left out.
pub fn members(
    tool: Tool,
    creatures: &Creatures,
    objects: &GameObjects,
    edits: Option<&Edits>,
) -> Vec<Standing> {
    match tool {
        Tool::Creatures => creatures
            .also
            .iter()
            .filter_map(|&guid| creatures.edited(guid, edits))
            .map(|spawn| Standing {
                kind: Kind::Creature,
                guid: spawn.guid,
                at: spawn.at,
                orientation: spawn.orientation,
            })
            .collect(),
        Tool::GameObjects => objects
            .also
            .iter()
            .filter_map(|&guid| objects.edited(guid, edits))
            .map(|spawn| Standing {
                kind: Kind::Object {
                    tilted: spawn.tilted,
                },
                guid: spawn.guid,
                at: spawn.at,
                orientation: spawn.orientation,
            })
            .collect(),
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
pub fn move_group(
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
    let noun = match first.kind {
        Kind::Creature => "creatures",
        Kind::Object { .. } => "game objects",
    };
    let label = match what {
        "facing" => format!("Turn {} {noun}", members.len()),
        _ => format!("Move {} {noun}", members.len()),
    };
    let subject = format!("{} group {} {what}", first.row().2, first.guid);
    let spin = super::group::turn_about(Vec3::Z, turn.to_degrees());
    session.as_one(&label, &subject, |session| {
        for member in members {
            let to = super::group::orbit(pivot, member.at, spin) + step;
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
    /// The word for it in an undo label and a status line.
    pub fn noun(&self) -> &'static str {
        match self.kind {
            Kind::Creature => "creature",
            Kind::Object { .. } => "game object",
        }
    }

    /// The table and key its spawn row is written under, and the prefix of
    /// the gesture subjects the tool's own drag uses for the same row.
    fn row(&self) -> (&'static str, Key, &'static str) {
        match self.kind {
            Kind::Creature => (creature::SPAWN, creature::spawn_key(self.guid), "creature"),
            Kind::Object { .. } => (gameobject::SPAWN, gameobject::spawn_key(self.guid), "gameobject"),
        }
    }

    /// Write the three position columns.
    pub fn move_to(&self, session: &mut EditSession, to: Vec3, now: f64) {
        let (table, key, prefix) = self.row();
        let subject = format!("{prefix} {} position", self.guid);
        let label = match self.kind {
            Kind::Creature => "Move creature",
            Kind::Object { .. } => "Move game object",
        };
        for (column, value) in [("position_x", to.x), ("position_y", to.y), ("position_z", to.z)] {
            session.set_server_edit(
                table,
                &key,
                column,
                Some(vale_mangos::sql::float(value)),
                Some(Gesture {
                    label,
                    subject: &subject,
                    now,
                }),
            );
        }
    }

    /// Write the facing, in radians, brought into `0..2π`.
    pub fn face(&self, session: &mut EditSession, orientation: f32, now: f64) {
        let orientation = orientation.rem_euclid(std::f32::consts::TAU);
        match self.kind {
            Kind::Creature => {
                let (table, key, prefix) = self.row();
                let subject = format!("{prefix} {} facing", self.guid);
                session.set_server_edit(
                    table,
                    &key,
                    "orientation",
                    Some(vale_mangos::sql::float(orientation)),
                    Some(Gesture {
                        label: "Turn creature",
                        subject: &subject,
                        now,
                    }),
                );
            }
            Kind::Object { tilted } => {
                GameObjects::turn(session, self.guid, tilted, orientation, now);
            }
        }
    }
}

/// Registers [`keys`].
pub struct SpawnKeysPlugin;

impl Plugin for SpawnKeysPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, keys);
    }
}

/// Move, turn, remove and duplicate the selected spawn from the keyboard.
/// The keys are listed in the module comment and on both tools' panels.
#[allow(clippy::too_many_arguments)]
pub fn keys(
    mut creatures: ResMut<Creatures>,
    mut objects: ResMut<GameObjects>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    time: Res<Time>,
) {
    if !matches!(*tool, Tool::Creatures | Tool::GameObjects) || !state.editing() {
        return;
    }
    if wants.wants_keyboard_input() {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    let Some(was) = chosen(*tool, &creatures, &objects, Some(&session.server_edits)) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let control = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    // The group is looked up only on a frame a key is pressed: finding a member
    // is a walk of the map's spawns, and most frames press nothing.
    let grouped = match *tool {
        Tool::Creatures => !creatures.also.is_empty(),
        _ => !objects.also.is_empty(),
    };
    if grouped {
        if !KEYS.iter().any(|&key| keys.just_pressed(key)) {
            return;
        }
        let others = members(*tool, &creatures, &objects, Some(&session.server_edits));
        group_keys(
            &mut creatures,
            &mut objects,
            session,
            &keys,
            was,
            others,
            control,
            now,
        );
        return;
    }

    // Ctrl+D. `tools::shortcuts` handles it for doodads and WMOs and does not
    // name these two tools, so the two cannot both act on one press.
    if control && keys.just_pressed(KeyCode::KeyD) {
        let made = match was.kind {
            Kind::Creature => creatures.duplicate(session, now),
            Kind::Object { .. } => objects.duplicate(session, now),
        };
        session.status = match made {
            Some(guid) => format!("duplicated as guid {guid}"),
            None => "the spawn's row is still being read".to_string(),
        };
        return;
    }
    // Ctrl+Z, Ctrl+Y and Ctrl+S are `tools::shortcuts`; a step key pressed
    // with Ctrl held is not a nudge.
    if control {
        return;
    }

    if keys.just_pressed(KeyCode::Delete) {
        let created = match was.kind {
            Kind::Creature => creatures.chosen().is_some_and(|spawn| spawn.is_new()),
            Kind::Object { .. } => objects.chosen().is_some_and(|spawn| spawn.is_new()),
        };
        match was.kind {
            Kind::Creature => Creatures::remove(session, was.guid, now),
            Kind::Object { .. } => GameObjects::remove(session, was.guid, now),
        }
        // A spawn this project created has no row once its claim is dropped,
        // so the selection is cleared, as the panels' Remove buttons do.
        session.status = match created {
            true => format!("{} {} discarded", was.noun(), was.guid),
            false => format!("{} {} is marked for removal", was.noun(), was.guid),
        };
        if created {
            match was.kind {
                Kind::Creature => creatures.select_only(None),
                Kind::Object { .. } => objects.select(None),
            }
        }
        return;
    }

    let far = match keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        true => 10.0,
        false => 1.0,
    };
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
    if step != Vec3::ZERO {
        was.move_to(session, was.at + step * STEP * far, now);
    }

    let mut turn = 0.0;
    if keys.just_pressed(KeyCode::Comma) {
        turn -= TURN * far;
    }
    if keys.just_pressed(KeyCode::Period) {
        turn += TURN * far;
    }
    if turn != 0.0 {
        was.face(session, was.orientation + turn.to_radians(), now);
    }
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
#[allow(clippy::too_many_arguments)]
fn group_keys(
    creatures: &mut Creatures,
    objects: &mut GameObjects,
    session: &mut EditSession,
    keys: &ButtonInput<KeyCode>,
    was: Standing,
    others: Vec<Standing>,
    control: bool,
    now: f64,
) {
    let mut all = vec![was];
    all.extend(others);
    let noun = match was.kind {
        Kind::Creature => "creatures",
        Kind::Object { .. } => "game objects",
    };
    if control {
        if keys.just_pressed(KeyCode::KeyD) {
            session.status = format!(
                "{} {noun} are selected; Ctrl+D copies one spawn at a time",
                all.len()
            );
        }
        return;
    }

    if keys.just_pressed(KeyCode::Delete) {
        let (table, prefix) = match was.kind {
            Kind::Creature => (creature::SPAWN, "creature"),
            Kind::Object { .. } => (gameobject::SPAWN, "gameobject"),
        };
        // Which of them this project created, read before the claims change:
        // those have no row once removed, so they leave the selection.
        let created: Vec<u64> = all
            .iter()
            .filter(|member| {
                let key = match member.kind {
                    Kind::Creature => creature::spawn_key(member.guid),
                    Kind::Object { .. } => gameobject::spawn_key(member.guid),
                };
                session
                    .server_edits
                    .row(table, &key)
                    .is_some_and(|row| row.life == vale_mangos::row::Life::Insert)
            })
            .map(|member| member.guid)
            .collect();
        let label = format!("Remove {} {noun}", all.len());
        let subject = format!("{prefix} group {} removal", was.guid);
        session.as_one(&label, &subject, |session| {
            for member in &all {
                match member.kind {
                    Kind::Creature => Creatures::remove(session, member.guid, now),
                    Kind::Object { .. } => GameObjects::remove(session, member.guid, now),
                }
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
        let survivors: Vec<u64> = all
            .iter()
            .map(|member| member.guid)
            .filter(|guid| !created.contains(guid))
            .collect();
        let primary = survivors.first().copied();
        let rest: Vec<u64> = survivors.iter().skip(1).copied().collect();
        match was.kind {
            Kind::Creature => {
                creatures.select_only(primary);
                creatures.also = rest;
            }
            Kind::Object { .. } => {
                objects.select(primary);
                objects.also = rest;
            }
        }
        return;
    }

    let far = match keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        true => 10.0,
        false => 1.0,
    };
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
    if step != Vec3::ZERO {
        move_group(session, &all, was.at, step * STEP * far, 0.0, "position", now);
    }
    let mut turn = 0.0;
    if keys.just_pressed(KeyCode::Comma) {
        turn -= TURN * far;
    }
    if keys.just_pressed(KeyCode::Period) {
        turn += TURN * far;
    }
    if turn != 0.0 {
        move_group(session, &all, was.at, Vec3::ZERO, turn.to_radians(), "facing", now);
    }
}
