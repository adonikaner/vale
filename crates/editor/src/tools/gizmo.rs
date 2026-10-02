//! The handles on a selected placement: three arrows to move it along an axis,
//! three rings to turn it about one.
//!
//! ## What the handles add to the ground drag and the angle keys
//!
//! Dragging across the ground moves two axes at once, neither of them up; `,`
//! and `.` turn one angle in five-degree steps. Neither can move a placement
//! exactly along one axis or turn it about a chosen axis. The handles do both,
//! and they draw each axis at the placement, so the direction of y is shown on
//! screen.
//!
//! ## Ring angles are world angles, not the file's angles
//!
//! `MDDF` stores three angles in degrees whose order and sign differ from the
//! world's: `vale_assets::world::adt::placement_matrix` composes them as
//! `Rz(rot[1]) * Ry(-rot[0]) * Rx(-rot[2])`, so the second one is the turn
//! about the world's up and the other two are turns about y and x with the sign
//! reversed. `placement_euler_to_world` is the one statement of that
//! conversion. This tool, like the inspector beside it, edits the world angles
//! and converts them on the way into the record.
//!
//! The axis each ring is drawn about is a separate question. The three turns
//! compose, so once any of them is non-zero only the first is still about a
//! world axis; each of the others is about its axis carried by the rotations
//! outside it. [`turn_axes`] computes those axes exactly, so a ring stays on its
//! model as the model turns.
//!
//! ## A drag belongs to the handle it started on
//!
//! The doodad and WMO tools follow the same rule. The grab is taken on the
//! press, and while it is held nothing else in this crate acts on the pointer:
//! [`super::doodads::select`] declines a press this took, and
//! [`super::doodads::drag`] declines the whole drag. Without that, grabbing an
//! arrow also picked whatever was behind it and dragged that across the ground.
//!
//! ## Coordinates are WoW's until drawing
//!
//! The pick, the axes and the arithmetic are in WoW coordinates, because the
//! record and the ray are. `render::axes::to_bevy` is applied only on the way
//! into `Gizmos`. It is a rotation, so a direction converts the same way as a
//! position.
//!
//! ## Why the handles are drawn in front of the world
//!
//! [`REACH`] keeps a gizmo the same size in pixels at any camera distance. That
//! is not enough on its own. A handle stands at the placement's origin, and the
//! origin is inside the model: the arrows on a lamp post stick out of the top,
//! and the arrows on a cathedral are a few yards long in the middle of a
//! building a hundred yards across. With the depth test on, the handles were
//! hidden behind their own model until the camera was far enough away for the
//! constant-pixel length to exceed the building. At that distance the handles
//! appeared already at full size, which read as the handles scaling up rather
//! than becoming visible. The scaling was correct; the handles only appeared
//! once they outgrew the geometry.
//!
//! [`EditorHandles`] is a gizmo config group with `depth_bias: -1.0`, which
//! draws in front of everything. Every instrument in this crate that a person
//! aims the pointer at uses it: the handles, the selection boxes, the hole
//! outlines. The two brushes' rings do not: a brush ring shows where the ground
//! is, and a ring drawn through a hill would show the ground somewhere it is
//! not.
//!
//! [`PathMarks`] is the waypoint path's group. It is in front like the handles,
//! because a person aims at a path node by node. See its own doc.
//!
//! [`WorldMarks`] is the group for marks that show where something is, at the
//! default bias. Such a mark is occluded by whatever is in front of it, because
//! a thing behind a tree cannot be seen.

use crate::session::EditSession;
use crate::tools::creatures::Creatures;
use crate::tools::doodads::Selection;
use crate::tools::gameobjects::GameObjects;
use crate::tools::{doodads, spawn, wmos, Tool};
use vale_client::render::axes;
use vale_client::world::camera::WorldCamera;
use bevy::gizmos::config::{GizmoConfig, GizmoConfigGroup, GizmoLineConfig};
use bevy::gizmos::AppGizmoBuilder;
use bevy::prelude::*;

/// Which handles are shown, and what a drag on one does.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Handles {
    /// Three arrows along the world's x, y and z.
    #[default]
    Move,
    /// Three rings, one per angle — see the module comment for what each is
    /// about.
    Turn,
    /// None. For when the handles cover what is behind them, as they do on a
    /// crowded hillside.
    Off,
}

impl Handles {
    pub fn name(self) -> &'static str {
        match self {
            Handles::Move => "Move",
            Handles::Turn => "Turn",
            Handles::Off => "Off",
        }
    }
}

/// The gizmo group for everything the pointer aims at, at `depth_bias: -1.0`,
/// so it draws in front of the world rather than inside it. The module comment
/// gives the reason.
///
/// It is a group rather than a flag on each call because `depth_bias` is a
/// property of the config, and because one group for all aimable instruments
/// sets their visibility in one place instead of five.
#[derive(Default, Reflect, GizmoConfigGroup)]
#[reflect(Default)]
pub struct EditorHandles;

/// The gizmo group for marks that show where something is, at the default
/// depth bias, so they draw inside the world rather than in front of it.
///
/// The two brush rings are drawn this way for the reason the module comment
/// gives: a ring through a hill shows the ground somewhere it is not. The same
/// applies to a mark for something standing on the ground. The creature tool
/// marks every spawn it cannot draw a model for, which in a forest is a
/// thousand rings; drawn in front, they showed through the trees and over the
/// hillsides behind them. A spawn behind a tree cannot be seen, so its mark
/// must be hidden too.
///
/// What a person aims at stays in [`EditorHandles`]. The creature tool draws
/// the selected and hovered marks there and all other marks here, the same
/// split the brushes make.
#[derive(Default, Reflect, GizmoConfigGroup)]
#[reflect(Default)]
pub struct WorldMarks;

/// The gizmo group for a waypoint path. It draws in front of the world, like
/// [`EditorHandles`], and is its own group because line width is a property of
/// a gizmo config and cannot be set per call. At the default two pixels a path
/// over grass was a hairline.
///
/// ## Why the path is drawn in front
///
/// A depth-tested path, on [`WorldMarks`]' reasoning that a mark drawn through
/// a hillside shows the creature walking somewhere it does not, disappeared
/// into every rise between it and the camera, which hid most of a path.
///
/// Drawing each leg twice, solid and depth-tested in one group and dashed and
/// in front in a second, made the path flicker: two coincident translucent
/// lines are both visible, and the blend order of two gizmo groups is not
/// stable from frame to frame.
///
/// Drawing each leg once and choosing the pass by whether the node was under
/// the terrain needed a height lookup per node per frame and still hid most of
/// a path, because a leg is hidden by the rise between it and the camera, not
/// by the ground at its own node.
///
/// Drawn in front, the path matches the selection marks, the handles and the
/// hole outlines, and the nodes a person aims at are visible. [`WorldMarks`]
/// exists to stop a mark showing something where it is not; a path drawn over
/// a hill still meets the ground at every node, which is where its values are.
#[derive(Default, Reflect, GizmoConfigGroup)]
#[reflect(Default)]
pub struct PathMarks;

/// How far in front of the world they are drawn. -1 is "always in front".
///
/// Fully in front rather than slightly: a handle half-buried in a wall can only
/// be used after moving the camera.
const IN_FRONT: f32 = -1.0;

/// The three, as a list a panel can offer.
pub const HANDLES: [(&str, Handles); 3] = [
    ("Move", Handles::Move),
    ("Turn", Handles::Turn),
    ("Off", Handles::Off),
];

/// The placement the handles are on, and how to read and write it.
///
/// Four tools have a selected thing with a position and a rotation. The doodad
/// and WMO tools store three angles in the file's own numbers; the creature and
/// game-object tools store a facing, which is carried here as the same three
/// angles with only the turn about up set. This struct holds what the four have
/// in common, so the gizmo is written once rather than once per tool.
///
/// Reading and writing are a pair of free functions and not a trait, because
/// the selections are Bevy resources and the gizmo has to take them mutably at
/// the same time; a trait object over them would need them behind one
/// parameter, which would be a `SystemParam` shaped for these four tools.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Held {
    /// The world position, which is what a handle stands at.
    position: Vec3,
    /// …and the file's three angles, which is what a ring turns — see
    /// [`turn_axes`] for what each of them turns about.
    rotation: [f32; 3],
    /// Whether only the turn about the world's up exists: true for a creature
    /// or game-object spawn, whose row stores a facing and no tilt. The x and
    /// y rings are then neither drawn nor grabbed.
    upright: bool,
}

/// What the chosen tool has selected, if anything.
///
/// A spawn's facing is carried as the file's three angles with only the turn
/// about up set, so the ring arithmetic below is the same for all four tools.
fn held(
    tool: Tool,
    doodads: &Selection,
    buildings: &wmos::Selection,
    spawn: Option<spawn::Standing>,
) -> Option<Held> {
    let (position, rotation) = match tool {
        Tool::Doodads => {
            let at = doodads.at.as_ref()?;
            (at.record.position, at.record.rotation)
        }
        Tool::Wmos => {
            let at = buildings.at.as_ref()?;
            (at.record.position, at.record.rotation)
        }
        Tool::Creatures | Tool::GameObjects => {
            let spawn = spawn?;
            return Some(Held {
                position: spawn.at,
                rotation: facing_to_rotation(spawn.orientation),
                upright: true,
            });
        }
        _ => return None,
    };
    Some(Held {
        position: Vec3::from(vale_assets::world::adt::placement_to_world(position)),
        rotation,
        upright: false,
    })
}

/// A spawn's `orientation`, in radians, as the file's three angles: a turn
/// about the world's up and nothing else.
fn facing_to_rotation(orientation: f32) -> [f32; 3] {
    vale_assets::world::adt::placement_euler_from_world([0.0, 0.0, orientation.to_degrees()])
}

/// …and back: the turn about the world's up, in radians.
fn rotation_to_facing(rotation: [f32; 3]) -> f32 {
    vale_assets::world::adt::placement_euler_to_world(rotation)[2].to_radians()
}

/// The selected spawn of the creature or game-object tool, as the project's
/// edits leave it.
fn standing(
    tool: Tool,
    creatures: &Creatures,
    objects: &GameObjects,
    session: Option<&EditSession>,
) -> Option<spawn::Standing> {
    spawn::chosen(
        tool,
        creatures,
        objects,
        session.map(|session| &session.server_edits),
    )
}

/// Writes a changed [`Held`] back through each tool's own writer.
///
/// It goes through the writers rather than into the records because each
/// writer does more than set two fields: the doodad writer reads the change's
/// `before` off the file, and the building writer moves the `MODF` box with the
/// position. Writing the records directly here would add a third place where
/// either step could be missed.
fn put(
    tool: Tool,
    session: &mut EditSession,
    doodads: &mut Selection,
    buildings: &mut wmos::Selection,
    spawn: Option<spawn::Standing>,
    was: Held,
    now: Held,
    time: f64,
) {
    match tool {
        Tool::Doodads => {
            let Some(at) = doodads.at.as_mut() else {
                return;
            };
            at.record.position =
                vale_assets::world::adt::placement_from_world(now.position.to_array());
            at.record.rotation = now.rotation;
            let at = at.clone();
            doodads::write_record(session, &at);
        }
        Tool::Wmos => {
            let Some(at) = buildings.at.as_mut() else {
                return;
            };
            let was = at.record;
            wmos::set_world_position(&mut at.record, &was, now.position.to_array());
            at.record.rotation = now.rotation;
            let at = at.clone();
            wmos::write_record(session, &at);
        }
        // A spawn's row is written column by column, each under the gesture
        // its own tool's drag uses, so only what changed is written.
        Tool::Creatures | Tool::GameObjects => {
            let Some(spawn) = spawn else {
                return;
            };
            if now.position != was.position {
                spawn.move_to(session, now.position, time);
            }
            if now.rotation != was.rotation {
                spawn.face(session, rotation_to_facing(now.rotation), time);
            }
        }
        _ => {}
    }
}

/// The word for what the chosen tool edits, in an undo label.
fn what(tool: Tool) -> &'static str {
    match tool {
        Tool::Wmos => "WMO",
        Tool::Creatures => "creature",
        Tool::GameObjects => "game object",
        _ => "doodad",
    }
}

/// What is being dragged, if anything.
#[derive(Resource, Debug, Default)]
pub struct Gizmo {
    pub handles: Handles,
    /// The axis the pointer is over, for lighting it before it is grabbed.
    hot: Option<usize>,
    grab: Option<Grab>,
}

impl Gizmo {
    /// Whether the gizmo has the pointer. Asked by the doodad tool, which must
    /// not pick or drag through a handle; see the module comment.
    pub fn holding(&self) -> bool {
        self.grab.is_some()
    }
}

#[derive(Debug)]
struct Grab {
    /// 0, 1 or 2 — the world's x, y or z. A move is along it and a turn is
    /// about it.
    axis: usize,
    /// Where the thing stood when the press landed, so a drag is exact rather
    /// than an accumulation of frames.
    was: Held,
    /// Where the gizmo stood when the press landed, in the world's own axes. It
    /// does not follow the placement during the drag, because an origin that
    /// moved with the placement would move the handle away from the pointer.
    origin: Vec3,
    /// …and the axis direction, frozen for the same reason.
    direction: Vec3,
    /// The value the drag started from: yards along the axis for a move, radians
    /// round the ring for a turn.
    from: f32,
    /// The rest of the selection as it stood when the press landed, so each
    /// member is moved and turned from where it began — see [`Members`].
    members: Members,
    /// Whether this grab's history entry has been opened. See [`drag`].
    opened: bool,
}

/// The members of a group beside the primary, as the handles were taken.
///
/// The handles stand on the primary. A drag on one moves the primary as it
/// always did, and then carries every member by the same move: along the axis
/// by the same distance, or round the primary's origin by the same angle about
/// the same axis, with each member turned by that angle about its own origin.
/// See [`super::group`].
#[derive(Debug, Default)]
enum Members {
    #[default]
    None,
    Doodads(Vec<doodads::Selected>),
    Wmos(Vec<wmos::Selected>),
    Spawns(Vec<spawn::Standing>),
}

impl Members {
    /// The members of whatever the chosen tool has selected.
    fn of(
        tool: Tool,
        doodads: &Selection,
        buildings: &wmos::Selection,
        creatures: &Creatures,
        objects: &GameObjects,
        session: Option<&EditSession>,
    ) -> Members {
        match tool {
            Tool::Doodads if !doodads.also.is_empty() => Members::Doodads(doodads.also.clone()),
            Tool::Wmos if !buildings.also.is_empty() => Members::Wmos(buildings.also.clone()),
            Tool::Creatures | Tool::GameObjects => {
                let members = spawn::members(
                    tool,
                    creatures,
                    objects,
                    session.map(|session| &session.server_edits),
                );
                match members.is_empty() {
                    true => Members::None,
                    false => Members::Spawns(members),
                }
            }
            _ => Members::None,
        }
    }
}

/// The two tools' own drag state, for the flags a group change sets: the
/// doodad members want settling into their tiles, and the building members
/// their boxes re-fitted as well.
#[derive(bevy::ecs::system::SystemParam)]
struct Settling<'w> {
    doodads: ResMut<'w, doodads::Held>,
    wmos: ResMut<'w, wmos::Held>,
}

/// Carry every member by a move of the group: round `pivot` by `turn`, then by
/// `step`, each turned by `turn` about its own origin. `members` are the
/// members as they were before the move, so the move is exact rather than an
/// accumulation of frames.
///
/// The tile placements are written into whatever history entry the caller has
/// open. The spawns are written under one group subject of their own, which
/// folds a drag into one entry, as the primary's own writes do.
#[allow(clippy::too_many_arguments)]
fn carry(
    session: &mut EditSession,
    members: &Members,
    doodad_selection: &mut Selection,
    buildings: &mut wmos::Selection,
    settling: &mut Settling,
    pivot: Vec3,
    step: Vec3,
    turn: Quat,
    now: f64,
) {
    match members {
        Members::None => {}
        Members::Doodads(was) => {
            for member in was {
                let Some(standing) = doodad_selection
                    .also
                    .iter_mut()
                    .find(|at| at.unique_id == member.unique_id)
                else {
                    continue;
                };
                let world = Vec3::from(vale_assets::world::adt::placement_to_world(
                    member.record.position,
                ));
                let to = super::group::orbit(pivot, world, turn) + step;
                standing.record.position =
                    vale_assets::world::adt::placement_from_world(to.to_array());
                standing.record.rotation = super::group::turn_record(member.record.rotation, turn);
                doodads::write_record(session, standing);
            }
            settling.doodads.unsettled = true;
        }
        Members::Wmos(was) => {
            for member in was {
                let Some(standing) = buildings
                    .also
                    .iter_mut()
                    .find(|at| at.unique_id == member.unique_id)
                else {
                    continue;
                };
                let world = Vec3::from(vale_assets::world::adt::placement_to_world(
                    member.record.position,
                ));
                let to = super::group::orbit(pivot, world, turn) + step;
                wmos::set_world_position(&mut standing.record, &member.record, to.to_array());
                standing.record.rotation = super::group::turn_record(member.record.rotation, turn);
                wmos::write_record(session, standing);
            }
            settling.wmos.refit_members = 2;
            settling.wmos.unsettled = true;
        }
        Members::Spawns(was) => {
            // A spawn has a facing and no tilt, so only the turn about up is
            // taken from the group's turn.
            let (axis, angle) = turn.to_axis_angle();
            let about_up = match axis.z < 0.0 {
                true => -angle,
                false => angle,
            };
            for member in was {
                let to = super::group::orbit(pivot, member.at, turn) + step;
                if to != member.at {
                    member.move_to(session, to, now);
                }
                if about_up.abs() > f32::EPSILON {
                    member.face(session, member.orientation + about_up, now);
                }
            }
        }
    }
}

/// How long an arrow is, and how wide a ring, as a fraction of the distance from
/// the camera.
///
/// It is a fraction rather than a size in yards. A size in yards is too small to
/// see on a lamp post and larger than the hillside on a tree, and the camera is
/// at every distance during a session. A fraction keeps the gizmo about the same
/// number of pixels at any distance, as other 3D editors do.
const REACH: f32 = 0.16;

/// Degrees a held `Alt` turns a placement per pixel of mouse travel.
///
/// It is a rate rather than a step, because this gesture turns a placement by
/// eye rather than to a number: a fence post is straightened by eye, and a tree
/// is turned until it differs from the one beside it. At half a degree a pixel,
/// a full turn is about the width of the viewport.
const SPIN_RATE: f32 = 0.5;

/// Degrees a held `Ctrl` snaps the `Alt` turn to. It is the step `,` and `.`
/// move in.
const SPIN_SNAP: f32 = 15.0;

/// How near the pointer has to be to a handle to take it, as a fraction of the
/// handle's own length. It is generous because an arrow is a line with no
/// thickness, and a person aims at the drawn arrow rather than at its exact
/// line.
const GRIP: f32 = 0.12;

pub struct GizmoToolPlugin;

impl Plugin for GizmoToolPlugin {
    fn build(&self, app: &mut App) {
        app.insert_gizmo_config(
            EditorHandles,
            GizmoConfig {
                depth_bias: IN_FRONT,
                ..default()
            },
        );
        // The marks drawn inside the world, at the default bias, so geometry in
        // front of a mark hides it. See [`WorldMarks`].
        app.insert_gizmo_config(WorldMarks, GizmoConfig::default());
        // The waypoint path; see [`PathMarks`]. It has its own group for the
        // width, which is a property of the config and cannot be set per call,
        // and it is in front for the reason given there.
        //
        // Two and a half pixels rather than four: at four a path was too heavy
        // across the viewport, and drawn in front of the world it does not need
        // the extra width to be found.
        app.insert_gizmo_config(
            PathMarks,
            GizmoConfig {
                depth_bias: IN_FRONT,
                line: GizmoLineConfig {
                    width: 2.5,
                    ..default()
                },
                ..default()
            },
        );
        app.init_resource::<Gizmo>().add_systems(
            Update,
            // Before each tool's own pick, so a press on a handle does not also
            // select what is behind it. The ordering is stated because nothing
            // reports its absence: without it the systems run in registration
            // order, and the selection changes whenever a handle is grabbed.
            (aim, drag)
                .chain()
                .after(crate::pick::aim)
                .before(crate::tools::doodads::select)
                .before(crate::tools::wmos::select)
                .before(crate::tools::creatures::press)
                .before(crate::tools::gameobjects::press),
        );
        app.add_systems(Update, draw);
        // The whole group is hidden during a playtest by
        // [`hide_while_playing`], one system rather than a guard in each of
        // the seven that draw into it.
        app.add_systems(Update, hide_while_playing);
        // After the drag, so a held handle takes precedence: a grab and a held
        // `Alt` both turn the same placement, and only one may act in a frame.
        app.add_systems(Update, spin.after(drag));
    }
}

/// Disables the [`EditorHandles`] group while a playtest is running.
///
/// A selection box was drawn in the world during a playtest with a character
/// walking under it. The seven systems that draw into [`EditorHandles`] each
/// checked the tool and none checked the playtest, and a new one would likely
/// repeat that.
///
/// So the guard is on the group and not on the systems. The group's buffer is
/// drawn only while `GizmoConfig::enabled` is set, so switching it off leaves
/// every producer running and writing lines that nothing renders. Writing the
/// lines is the cheap part of a gizmo, and no state has to be rebuilt when the
/// playtest ends.
///
/// It does not affect the two brushes' rings, which are in the default group
/// for the reason the module comment gives; those are already off during a
/// playtest because the tools that draw them stop.
fn hide_while_playing(
    mut config: ResMut<bevy::gizmos::config::GizmoConfigStore>,
    state: Res<crate::playtest::Playtest>,
) {
    let (config, _) = config.config_mut::<EditorHandles>();
    let wanted = state.editing();
    if config.enabled != wanted {
        config.enabled = wanted;
    }
}

/// Take a handle, or notice the pointer is over one.
#[allow(clippy::too_many_arguments)]
fn aim(
    mut gizmo: ResMut<Gizmo>,
    selection: Res<Selection>,
    buildings: Res<wmos::Selection>,
    creatures: Res<Creatures>,
    objects: Res<GameObjects>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    gizmo.hot = None;
    if !state.editing() || gizmo.handles == Handles::Off {
        return;
    }
    let spawn = standing(*tool, &creatures, &objects, session.as_deref());
    let Some(at) = held(*tool, &selection, &buildings, spawn) else {
        return;
    };
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    let Some((origin, direction)) = ray(&windows, &camera) else {
        return;
    };

    let centre = at.position;
    let reach = (centre - origin).length() * REACH;
    let axes = match gizmo.handles {
        Handles::Turn => turn_axes(at.rotation),
        _ => [Vec3::X, Vec3::Y, Vec3::Z],
    };

    let hit = match gizmo.handles {
        Handles::Turn => nearest_ring(origin, direction, centre, &axes, reach, at.upright),
        _ => nearest_arrow(origin, direction, centre, &axes, reach),
    };
    gizmo.hot = hit.map(|(axis, _)| axis);
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if let Some((axis, from)) = hit {
        gizmo.grab = Some(Grab {
            axis,
            was: at,
            origin: centre,
            direction: axes[axis],
            from,
            members: Members::of(
                *tool,
                &selection,
                &buildings,
                &creatures,
                &objects,
                session.as_deref(),
            ),
            opened: false,
        });
    }
}

/// Move or turn the held placement while the button is down.
#[allow(clippy::too_many_arguments)]
fn drag(
    mut gizmo: ResMut<Gizmo>,
    mut selection: ResMut<Selection>,
    mut buildings: ResMut<wmos::Selection>,
    mut session: Option<ResMut<EditSession>>,
    creatures: Res<Creatures>,
    objects: Res<GameObjects>,
    tool: Res<Tool>,
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    mut settling: Settling,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    // The release is handled wherever the pointer is: a drag that began on a
    // handle and ended over a panel must still end, or the history entry is
    // left open and the next change folds into it.
    if buttons.just_released(MouseButton::Left) {
        if gizmo.grab.is_some() {
            session.history.end();
            session.history.release();
            gizmo.grab = None;
        }
        return;
    }
    let Some(grab) = gizmo.grab.as_ref() else {
        return;
    };
    if !buttons.pressed(MouseButton::Left) {
        return;
    }
    let spawn = standing(*tool, &creatures, &objects, Some(&**session));
    let Some(was) = held(*tool, &selection, &buildings, spawn) else {
        gizmo.grab = None;
        return;
    };
    let Some((origin, direction)) = ray(&windows, &camera) else {
        return;
    };

    let mut now = grab.was;
    // …and the same move as the group's: the step along the axis, or the turn
    // about it. See [`carry`].
    let mut turn = Quat::IDENTITY;
    match gizmo.handles {
        Handles::Turn => {
            let Some(angle) = ring_angle(origin, direction, grab.origin, grab.direction) else {
                return;
            };
            let turned = (angle - grab.from).to_degrees();
            let mut world = vale_assets::world::adt::placement_euler_to_world(grab.was.rotation);
            world[grab.axis] = wrap(world[grab.axis] + turned);
            now.rotation = vale_assets::world::adt::placement_euler_from_world(world);
            turn = super::group::turn_about(grab.direction, turned);
        }
        _ => {
            let Some(along) = along_axis(origin, direction, grab.origin, grab.direction) else {
                return;
            };
            now.position = grab.was.position + grab.direction * (along - grab.from);
        }
    }
    if now == was {
        return;
    }
    let step = now.position - grab.was.position;
    let size = match &grab.members {
        Members::None => 1,
        Members::Doodads(m) => m.len() + 1,
        Members::Wmos(m) => m.len() + 1,
        Members::Spawns(m) => m.len() + 1,
    };
    let verb = match gizmo.handles {
        Handles::Turn => "Turn",
        _ => "Move",
    };
    let label = match size {
        1 => format!("{verb} {}", what(*tool)),
        more => format!("{verb} {more} {}s", what(*tool)),
    };
    // A tile placement's drag is one history entry from the first frame that
    // moves to the release, opened here on that frame. It is opened by this
    // grab and not taken over from whatever is open: `spin` leaves its gesture
    // open, and a drag that wrote into it was folded into the turn before it.
    //
    // A spawn's writes each open and close an entry of their own under a
    // gesture subject, which folds a drag into one; an entry held open here
    // would be closed by the first of them. The hold keeps a pause in the drag
    // from ending that entry — see `vale_edit::undo::History::hold`.
    let secs = time.elapsed_secs_f64();
    let opened = grab.opened;
    match was.upright {
        false if !opened => session.history.begin(label.clone()),
        false => {}
        true => session.history.hold(secs),
    }
    let mut write = |session: &mut EditSession| {
        put(
            *tool,
            session,
            &mut selection,
            &mut buildings,
            spawn,
            was,
            now,
            secs,
        );
        carry(
            session,
            &grab.members,
            &mut selection,
            &mut buildings,
            &mut settling,
            grab.origin,
            step,
            turn,
            secs,
        );
    };
    match (&grab.members, spawn) {
        // A group of spawns is one entry under one subject for the whole drag,
        // the primary's writes included — see `EditSession::as_one`.
        (Members::Spawns(_), Some(spawn)) => {
            let subject = format!("{} group {} gizmo", what(*tool), spawn.guid);
            session.as_one(&label, &subject, write);
        }
        _ => write(&mut **session),
    }
    if let Some(grab) = gizmo.grab.as_mut() {
        grab.opened = true;
    }
}

/// Turns the selection about the world's up while `Alt` is held and the mouse
/// moves.
///
/// This is the third way to turn a placement, beside `,` and `.` and the ring.
/// It needs no handle to hit and no number field. The turn about z is usually
/// the only one scenery needs. `,` and `.` step it in fifteens, and the ring is
/// exact but has to be grabbed; this turns it continuously.
///
/// No mouse button is involved, only the modifier and mouse movement, and
/// holding `Alt` moves nothing else: the camera turns on a right-drag and the
/// tools act on a left one.
///
/// It folds into one history entry through `begin_gesture`, as the wheel and
/// the arrow keys do: a run of changes with no release to close it would
/// otherwise be one undo step per frame. The gesture subject names the
/// placement and the axis, so a spin and a nudge in the same tenth of a second
/// stay two entries.
#[allow(clippy::too_many_arguments)]
fn spin(
    mut selection: ResMut<Selection>,
    mut buildings: ResMut<wmos::Selection>,
    mut session: Option<ResMut<EditSession>>,
    mut placing: ResMut<super::place::Placing>,
    mut creatures: ResMut<Creatures>,
    mut objects: ResMut<GameObjects>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    time: Res<Time>,
    mut settling: Settling,
) {
    if !state.editing()
        || !matches!(
            *tool,
            Tool::Doodads | Tool::Wmos | Tool::Creatures | Tool::GameObjects
        )
    {
        return;
    }
    if wants.wants_keyboard_input() {
        return;
    }
    if !keys.pressed(KeyCode::AltLeft) && !keys.pressed(KeyCode::AltRight) {
        return;
    }
    let by = motion.delta.x * SPIN_RATE;
    if by == 0.0 {
        return;
    }
    let snap = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);

    // While the placer is armed, the spin turns the model on the cursor rather
    // than the selection, because that model is what the next click affects.
    // The placer holds doodads and WMOs only, so a spin in the creature or
    // game-object tool never turns it.
    if placing.armed() && matches!(*tool, Tool::Doodads | Tool::Wmos) {
        let turned = placing.turn + by;
        placing.turn = wrap(match snap {
            true => (turned / SPIN_SNAP).round() * SPIN_SNAP,
            false => turned,
        });
        return;
    }
    // The same for a spawn on the cursor, which each spawn tool holds itself.
    if *tool == Tool::Creatures && creatures.placing() {
        creatures.turn_ghost(by, snap);
        return;
    }
    if *tool == Tool::GameObjects && objects.placing() {
        objects.turn_ghost(by, snap);
        return;
    }

    let Some(session) = session.as_mut() else {
        return;
    };
    let spawn = standing(*tool, &creatures, &objects, Some(&**session));
    let Some(was) = held(*tool, &selection, &buildings, spawn) else {
        return;
    };
    let mut world = vale_assets::world::adt::placement_euler_to_world(was.rotation);
    world[2] = wrap(match snap {
        true => ((world[2] + by) / SPIN_SNAP).round() * SPIN_SNAP,
        false => world[2] + by,
    });
    let now = Held {
        rotation: vale_assets::world::adt::placement_euler_from_world(world),
        ..was
    };
    if now == was {
        return;
    }
    let now_secs = time.elapsed_secs_f64();
    // The rest of the group, as they stand now: the spin is applied a frame at
    // a time, so each frame turns the group by that frame's angle about the
    // primary's origin. The angle is what the primary actually turned, which
    // with `Ctrl` is the snapped one.
    let members = Members::of(
        *tool,
        &selection,
        &buildings,
        &creatures,
        &objects,
        Some(&**session),
    );
    let before = vale_assets::world::adt::placement_euler_to_world(was.rotation)[2];
    let turned = (world[2] - before + 180.0).rem_euclid(360.0) - 180.0;
    let turn = super::group::turn_about(Vec3::Z, turned);
    let what = what(*tool);
    // A spawn's write makes its own gesture entry; see [`drag`].
    if !was.upright {
        let subject = match members {
            Members::None => format!("{what} spin"),
            _ => format!("{what} group spin"),
        };
        session
            .history
            .begin_gesture(format!("Turn {what}"), subject, now_secs);
    }
    let mut write = |session: &mut EditSession| {
        put(
            *tool,
            session,
            &mut selection,
            &mut buildings,
            spawn,
            was,
            now,
            now_secs,
        );
        carry(
            session,
            &members,
            &mut selection,
            &mut buildings,
            &mut settling,
            was.position,
            Vec3::ZERO,
            turn,
            now_secs,
        );
    };
    match (&members, spawn) {
        (Members::Spawns(_), Some(spawn)) => {
            let subject = format!("{what} group {} spin", spawn.guid);
            session.as_one(&format!("Turn {what}s"), &subject, write);
        }
        _ => write(&mut **session),
    }
    // Closed on every frame, as the arrow keys close theirs: the next frame's
    // `begin_gesture` continues it, and an entry left open would take in
    // whatever is written next, a gizmo drag included.
    if !was.upright {
        session.history.end();
    }
}

/// Degrees, brought back into `0..360`.
fn wrap(degrees: f32) -> f32 {
    degrees.rem_euclid(360.0)
}

/// The pointer's ray, in the world's own axes.
///
/// Physical pixels, for the reason [`crate::pick::aim`] gives beside its own
/// copy of this line: the world camera draws into a target sized in physical
/// pixels and `cursor_position` is logical, so on a 125% display the ray misses
/// by a quarter of the distance from the screen's centre.
fn ray(
    windows: &Query<&Window>,
    camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<(Vec3, Vec3)> {
    let window = windows.single().ok()?;
    let at = window.cursor_position()?;
    let (camera, eye) = camera.single().ok()?;
    let ray = camera
        .viewport_to_world(eye, at * window.scale_factor())
        .ok()?;
    let origin = Vec3::from(axes::to_wow(ray.origin));
    let direction = Vec3::from(axes::to_wow(*ray.direction)).normalize_or_zero();
    (direction != Vec3::ZERO).then_some((origin, direction))
}

/// The world axis each of a placement's three angles turns it about, at the
/// placement's current rotation, indexed by world axis: x, y, then z.
///
/// The composition is `Rz(rot[1]) * Ry(-rot[0]) * Rx(-rot[2]) * Rz(180°)`, so
/// changing an angle inserts a turn about its own basis axis inside the
/// rotations outside it. The world axis of that turn is therefore the outer
/// rotations applied to that basis vector:
///
/// ```text
/// about z   rot[1]    z                        always a world axis
/// about y   rot[0]    Rz(rot[1]) * y
/// about x   rot[2]    Rz(rot[1]) * Ry(-rot[0]) * x
/// ```
///
/// The axes are exact, which keeps each ring on the model it is drawn on. Three
/// fixed world rings look right on an unturned tree and wrong on a turned one,
/// and dragging them would turn the model about a different axis than the ring
/// shows.
fn turn_axes(rotation_deg: [f32; 3]) -> [Vec3; 3] {
    let ry = rotation_deg[1].to_radians();
    let rx = rotation_deg[0].to_radians();
    let about_z = Quat::from_rotation_z(ry);
    let about_y = about_z * Quat::from_rotation_y(-rx);
    [about_y * Vec3::X, about_z * Vec3::Y, Vec3::Z]
}

/// The nearest arrow the ray comes within [`GRIP`] of, and how far along it the
/// closest point is.
fn nearest_arrow(
    origin: Vec3,
    direction: Vec3,
    centre: Vec3,
    axes: &[Vec3; 3],
    reach: f32,
) -> Option<(usize, f32)> {
    let mut best: Option<(f32, usize, f32)> = None;
    for (axis, &along) in axes.iter().enumerate() {
        let Some((s, t)) = closest(centre, along, origin, direction) else {
            continue;
        };
        // Behind the camera, or past the end of the arrow. A small overhang at
        // the tip so the head is grabbable.
        if t <= 0.0 || s < -reach * GRIP || s > reach * 1.15 {
            continue;
        }
        let gap = ((centre + along * s) - (origin + direction * t)).length();
        if gap > reach * GRIP {
            continue;
        }
        if best.is_none_or(|(had, _, _)| gap < had) {
            best = Some((gap, axis, s));
        }
    }
    best.map(|(_, axis, s)| (axis, s))
}

/// …and the nearest ring, with the angle round it the ray meets.
fn nearest_ring(
    origin: Vec3,
    direction: Vec3,
    centre: Vec3,
    axes: &[Vec3; 3],
    reach: f32,
    upright: bool,
) -> Option<(usize, f32)> {
    let mut best: Option<(f32, usize, f32)> = None;
    for (axis, &normal) in axes.iter().enumerate() {
        // Only the z ring exists on an upright subject; see [`Held::upright`].
        if upright && axis != 2 {
            continue;
        }
        let Some(hit) = plane_hit(origin, direction, centre, normal) else {
            continue;
        };
        let gap = ((hit - centre).length() - reach).abs();
        if gap > reach * GRIP * 2.0 {
            continue;
        }
        let angle = angle_in_plane(hit - centre, normal);
        if best.is_none_or(|(had, _, _)| gap < had) {
            best = Some((gap, axis, angle));
        }
    }
    best.map(|(_, axis, angle)| (axis, angle))
}

/// Where along an axis the pointer is now, for a move already in progress.
fn along_axis(origin: Vec3, direction: Vec3, centre: Vec3, along: Vec3) -> Option<f32> {
    closest(centre, along, origin, direction).map(|(s, _)| s)
}

/// …and the angle round a ring, for a turn.
fn ring_angle(origin: Vec3, direction: Vec3, centre: Vec3, normal: Vec3) -> Option<f32> {
    let hit = plane_hit(origin, direction, centre, normal)?;
    Some(angle_in_plane(hit - centre, normal))
}

/// The closest points of a line and a ray, as `(along the line, along the ray)`.
///
/// `None` when the two are parallel, which for a gizmo means the camera is
/// looking straight down the axis. In that view the arrow cannot be dragged,
/// and it is drawn as a single pixel.
fn closest(p: Vec3, u: Vec3, o: Vec3, v: Vec3) -> Option<(f32, f32)> {
    let b = u.dot(v);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-5 {
        return None;
    }
    let r = p - o;
    let c = u.dot(r);
    let f = v.dot(r);
    Some(((b * f - c) / denom, (f - b * c) / denom))
}

/// Where a ray meets the plane through `centre` with `normal`.
fn plane_hit(origin: Vec3, direction: Vec3, centre: Vec3, normal: Vec3) -> Option<Vec3> {
    let facing = direction.dot(normal);
    if facing.abs() < 1e-5 {
        return None;
    }
    let t = (centre - origin).dot(normal) / facing;
    (t > 0.0).then(|| origin + direction * t)
}

/// A direction in a plane, as an angle about that plane's normal.
///
/// The two basis vectors are `Vec3::any_orthonormal_pair`'s, which is a fixed
/// function of the normal, so the angle returned has an arbitrary zero that
/// does not change between calls. A drag only uses the difference between two
/// such angles.
fn angle_in_plane(offset: Vec3, normal: Vec3) -> f32 {
    let (right, up) = normal.any_orthonormal_pair();
    offset.dot(up).atan2(offset.dot(right))
}

/// Draw whichever handles are on.
fn draw(
    mut gizmos: Gizmos<EditorHandles>,
    gizmo: Res<Gizmo>,
    selection: Res<Selection>,
    buildings: Res<wmos::Selection>,
    creatures: Res<Creatures>,
    objects: Res<GameObjects>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
) {
    if !state.editing() || gizmo.handles == Handles::Off {
        return;
    }
    let spawn = standing(*tool, &creatures, &objects, session.as_deref());
    let Some(at) = held(*tool, &selection, &buildings, spawn) else {
        return;
    };
    let Ok(eye) = camera.single() else { return };

    let drawn = axes::to_bevy(at.position.to_array());
    let reach = (drawn - eye.translation()).length() * REACH;
    // The axis being dragged takes precedence over the one under the pointer:
    // a drag that has moved off its handle still belongs to that handle.
    let lit = gizmo.grab.as_ref().map(|grab| grab.axis).or(gizmo.hot);

    // x red, y green, z blue: the common convention, and the colours
    // `render::axes`' own reference cross is drawn in.
    let colours = [
        Color::srgb(0.95, 0.35, 0.35),
        Color::srgb(0.40, 0.90, 0.45),
        Color::srgb(0.40, 0.60, 1.00),
    ];
    let axes_now = match gizmo.handles {
        Handles::Turn => turn_axes(at.rotation),
        _ => [Vec3::X, Vec3::Y, Vec3::Z],
    };

    for (axis, &along) in axes_now.iter().enumerate() {
        let colour = match lit == Some(axis) {
            true => Color::srgb(1.0, 0.95, 0.55),
            false => colours[axis],
        };
        let along = axes::to_bevy(along.to_array());
        match gizmo.handles {
            Handles::Turn if at.upright && axis != 2 => {}
            Handles::Turn => {
                gizmos
                    .circle(
                        Isometry3d::new(drawn, Quat::from_rotation_arc(Vec3::Z, along)),
                        reach,
                        colour,
                    )
                    .resolution(48);
            }
            _ => {
                let tip = drawn + along * reach;
                gizmos.line(drawn, tip, colour);
                // A head, so which end is the tip is legible from any angle: a
                // short cross at the tip, in the plane facing the camera.
                let (a, b) = along.any_orthonormal_pair();
                let head = reach * 0.08;
                gizmos.line(tip - along * head * 2.0 + a * head, tip, colour);
                gizmos.line(tip - along * head * 2.0 - a * head, tip, colour);
                gizmos.line(tip - along * head * 2.0 + b * head, tip, colour);
                gizmos.line(tip - along * head * 2.0 - b * head, tip, colour);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ray at one arrow finds that arrow and no other. This is the arithmetic
    /// that decides which handle a press takes. A gizmo that grabbed y on a
    /// click on x would look like a drag going the wrong way rather than a
    /// missed pick.
    #[test]
    fn a_ray_at_an_arrow_finds_that_arrow() {
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        let centre = Vec3::ZERO;
        // Looking down from above at a point five yards along x.
        let hit = nearest_arrow(Vec3::new(5.0, 0.0, 20.0), Vec3::NEG_Z, centre, &axes, 10.0);
        assert_eq!(hit.map(|(axis, _)| axis), Some(0));
        assert!(hit.is_some_and(|(_, s)| (s - 5.0).abs() < 1e-3), "{hit:?}");

        // …and five yards along y is the other one.
        let hit = nearest_arrow(Vec3::new(0.0, 5.0, 20.0), Vec3::NEG_Z, centre, &axes, 10.0);
        assert_eq!(hit.map(|(axis, _)| axis), Some(1));

        // Well away from all three is nothing at all, rather than the nearest.
        assert_eq!(
            nearest_arrow(Vec3::new(8.0, 8.0, 20.0), Vec3::NEG_Z, centre, &axes, 10.0),
            None
        );
    }

    /// A ray at the rim of a ring finds it, and one through the middle does not.
    #[test]
    fn a_ray_at_a_ring_finds_its_rim_and_not_its_middle() {
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        // Straight down at the rim of the z ring, which lies flat.
        let hit = nearest_ring(
            Vec3::new(10.0, 0.0, 20.0),
            Vec3::NEG_Z,
            Vec3::ZERO,
            &axes,
            10.0,
            false,
        );
        assert_eq!(hit.map(|(axis, _)| axis), Some(2));
        assert_eq!(
            nearest_ring(
                Vec3::new(0.0, 0.0, 20.0),
                Vec3::NEG_Z,
                Vec3::ZERO,
                &axes,
                10.0,
                false
            ),
            None,
            "the middle of a ring is not the ring"
        );
    }

    /// The z ring is about the world's up at any rotation, and the other two
    /// turn with the placement.
    ///
    /// This keeps a ring on its model. An error here is invisible on an
    /// unturned tree, which is most of them, and visible on a turned one.
    #[test]
    fn the_turn_axes_follow_the_placement() {
        let flat = turn_axes([0.0; 3]);
        assert!((flat[0] - Vec3::X).length() < 1e-5);
        assert!((flat[1] - Vec3::Y).length() < 1e-5);
        assert!((flat[2] - Vec3::Z).length() < 1e-5);

        // A quarter turn about the world's up: the z ring is unmoved and the
        // other two have come round with the model.
        let turned = turn_axes([0.0, 90.0, 0.0]);
        assert!((turned[2] - Vec3::Z).length() < 1e-5, "{turned:?}");
        assert!((turned[1] - Vec3::NEG_X).length() < 1e-4, "{turned:?}");
        assert!((turned[0] - Vec3::Y).length() < 1e-4, "{turned:?}");
        // …and all three stay a right-handed orthonormal set, which is what
        // says the composition was applied and not merely permuted.
        for pair in [(0, 1), (1, 2), (2, 0)] {
            assert!(
                turned[pair.0].dot(turned[pair.1]).abs() < 1e-4,
                "{pair:?} of {turned:?}"
            );
        }
    }

    /// A spawn's `orientation` survives the trip through the file's three
    /// angles that the rings turn, and a ring turn of 90° about z adds a
    /// quarter turn to it.
    #[test]
    fn a_spawn_facing_round_trips_through_the_ring_angles() {
        for orientation in [0.0f32, 0.5, 1.519546, 3.0, 6.0] {
            let back = rotation_to_facing(facing_to_rotation(orientation));
            assert!((back - orientation).abs() < 1e-4, "{orientation} came back as {back}");
        }
        let mut world =
            vale_assets::world::adt::placement_euler_to_world(facing_to_rotation(1.0));
        world[2] = wrap(world[2] + 90.0);
        let turned = rotation_to_facing(vale_assets::world::adt::placement_euler_from_world(world));
        assert!((turned - (1.0 + std::f32::consts::FRAC_PI_2)).abs() < 1e-4, "{turned}");
    }

    /// On an upright subject the x and y rings are not offered: a ray at the
    /// rim of the x ring finds nothing, and the z ring is still found.
    #[test]
    fn an_upright_subject_offers_only_the_z_ring() {
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        // Looking along x at the top of the x ring, which stands in the y-z
        // plane. The ray is parallel to the other two rings' planes.
        let at_x_ring = (Vec3::new(-20.0, 0.0, 10.0), Vec3::X);
        assert_eq!(
            nearest_ring(at_x_ring.0, at_x_ring.1, Vec3::ZERO, &axes, 10.0, false)
                .map(|(axis, _)| axis),
            Some(0)
        );
        assert_eq!(
            nearest_ring(at_x_ring.0, at_x_ring.1, Vec3::ZERO, &axes, 10.0, true),
            None
        );
        let at_z_ring = (Vec3::new(10.0, 0.0, 20.0), Vec3::NEG_Z);
        assert_eq!(
            nearest_ring(at_z_ring.0, at_z_ring.1, Vec3::ZERO, &axes, 10.0, true)
                .map(|(axis, _)| axis),
            Some(2)
        );
    }

    /// The closest-point solve answers the parameter along the line, and
    /// declines a ray parallel to it rather than dividing by nothing.
    #[test]
    fn the_closest_point_solve_answers_along_the_line() {
        let hit = closest(Vec3::ZERO, Vec3::X, Vec3::new(3.0, 0.0, 5.0), Vec3::NEG_Z);
        assert!(hit.is_some_and(|(s, t)| (s - 3.0).abs() < 1e-4 && (t - 5.0).abs() < 1e-4));
        assert_eq!(
            closest(Vec3::ZERO, Vec3::X, Vec3::new(0.0, 1.0, 0.0), Vec3::X),
            None
        );
    }
}
