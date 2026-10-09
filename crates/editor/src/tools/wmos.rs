//! `MODF`: the buildings standing on a tile. Select one, move it, turn it, take
//! it away.
//!
//! ## Why the tool is named WMO rather than Buildings
//!
//! `MWMO` names these placements, `MODF` places them, and `vale wmos` checks
//! them. Half of what a `.wmo` file holds is not a building — a bridge, a
//! gate, an elevator shaft, the canals under Stormwind — so naming the tool
//! after the file format keeps the rail's label searchable against those
//! names.
//!
//! ## Differences from the doodad tool
//!
//! Picking, dragging, nudging, turning and removing work the way
//! [`super::doodads`] does: a drag belongs to where it began, `unique_id`
//! crosses the boundary rather than a list index, and `before` is read off
//! the file. Three things differ:
//!
//! * There is no scale. `MODF` in 1.12 holds a position, three angles, a
//!   world-space box, a doodad set and a name set; the two bytes that later
//!   versions use for a scale are padding here. The reference client does not
//!   read a scale on a `MODF` record, so this tool offers none.
//! * The placement carries its own bounding box, and other code depends on
//!   it holding the right value. See [`refit`].
//! * There is no resident list to keep in step. A doodad's transform is
//!   cached by `ResidentDoodads` so a re-spawn keeps it; a building is spawned
//!   once from `PendingWmos` and read from the file again whenever its tile
//!   is, so moving the entity and the file is the whole of it.
//!
//! ## What a move does not update, and why re-reading the tile is cheap
//!
//! A building's collision is a hull placed by a task at spawn and keyed on
//! the placement's id, its interior lighting is `Interior`'s inverse matrix,
//! and its `MOLT` lamps are resolved into world space once. This module
//! updates the second directly, since it is one matrix and the playtest's
//! indoor test reads it; the other two come back only when the tile is read
//! again, which this module requests once the mouse button comes up.
//! Re-reading a tile used to blank it for about a third of a second; since
//! `super::terrain::swap` keeps the old tile drawn until the new one is
//! ready, the re-read is invisible, which is why waiting for it is
//! acceptable rather than a fallback of last resort.

use crate::pick::Cursor;
use crate::marks::{Look, Marks};
use crate::session::EditSession;
use crate::tools::Tool;
use vale_client::render::axes;
use vale_client::render::doodads::Doodad;
use vale_client::render::wmos::{self as wmo_render, Interior, WmoCache, WmoPart, WmoPlacement};
use vale_client::world::camera::WorldCamera;
use vale_edit::adt::place::Building;
use vale_edit::ops::Edit;
use bevy::camera::primitives::Aabb;
use bevy::prelude::*;

/// Which building the tool is holding: the primary, and the rest of the
/// selected group. Same shape as [`super::doodads::Selection`], for the same
/// reasons.
#[derive(Resource, Debug, Default, Clone)]
pub struct Selection {
    pub at: Option<Selected>,
    pub also: Vec<Selected>,
}

/// One selected building, and everything a panel needs to say about it.
#[derive(Debug, Clone)]
pub struct Selected {
    pub tile: (u32, u32),
    pub index: usize,
    /// The id the renderer knows it by.
    pub unique_id: u32,
    /// Its `MWMO` path.
    pub path: String,
    pub record: Building,
}

impl Selection {
    /// Select one building, or none, and drop the rest of the group.
    pub fn only(&mut self, at: Option<Selected>) {
        self.at = at;
        self.also.clear();
    }

    /// How many buildings are selected.
    pub fn count(&self) -> usize {
        usize::from(self.at.is_some()) + self.also.len()
    }

    /// Every selected building, the primary first.
    pub fn members(&self) -> impl Iterator<Item = &Selected> {
        self.at.iter().chain(self.also.iter())
    }

    /// Whether a building is selected, as the primary or as a member.
    pub fn holds(&self, unique_id: u32) -> bool {
        self.members().any(|at| at.unique_id == unique_id)
    }

    /// Shift+click — see [`super::doodads::Selection::toggle`].
    pub fn toggle(&mut self, found: Selected) {
        if self.at.as_ref().is_some_and(|at| at.unique_id == found.unique_id) {
            self.at = self.also.pop();
            return;
        }
        if let Some(at) = self.also.iter().position(|m| m.unique_id == found.unique_id) {
            self.also.remove(at);
            return;
        }
        if let Some(was) = self.at.replace(found) {
            self.also.push(was);
        }
    }

    /// Make a member the primary, keeping the group.
    pub fn promote(&mut self, unique_id: u32) {
        let Some(at) = self.also.iter().position(|m| m.unique_id == unique_id) else {
            return;
        };
        let member = self.also.remove(at);
        if let Some(was) = self.at.replace(member) {
            self.also.insert(at, was);
        }
    }

    /// The record a panel has just edited, or `None` when nothing moved.
    pub fn edited(&self, was: &Building) -> Option<Building> {
        let at = self.at.as_ref()?;
        (at.record != *was).then_some(at.record)
    }
}

/// Yards a nudge moves a building, and degrees a turn turns it.
///
/// Ten times the doodad tool's step, because buildings are larger than
/// doodads and need a coarser default. Shift multiplies it by ten again.
const STEP: f32 = 5.0;
const TURN: f32 = 5.0;

pub struct WmoToolPlugin;

impl Plugin for WmoToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Selection>()
            .init_resource::<Held>()
            .add_systems(
                Update,
                (
                    select, enclose, drag, nudge, remove, reconcile, publish, settle, resync,
                )
                    .chain()
                    .after(crate::pick::aim)
                    // After the undo key, for the reason
                    // `super::doodads::resync` is: an undo puts the file back
                    // without telling the panel holding a copy of the record.
                    .after(super::shortcuts),
            )
            .add_systems(Update, (draw_marker, draw_locked));
    }
}

/// Whether a drag is being held. Counterpart of [`super::doodads::Held`],
/// with the same fields for the same reasons.
#[derive(Resource, Default)]
pub(crate) struct Held {
    armed: bool,
    dragging: bool,
    from: Option<Vec3>,
    was: Option<Building>,
    /// The record as it stood when this building was last put where it is.
    ///
    /// [`publish`] re-fits the `MODF` box from the drawn geometry, and it must
    /// do that only when the placement has actually moved. This tool's box is
    /// the union of what is drawn, while Blizzard's is the whole model, so
    /// the two differ on nearly every placement; re-fitting unconditionally
    /// would make selecting a building register as an edit and mark the tile
    /// unsaved on the first click.
    picked: Option<Building>,
    /// The two sets as of the last time the tile was read again.
    ///
    /// Which dressing a placement wears is decided when the building is
    /// spawned: the `MODD` furniture of the chosen set is folded into world
    /// space and handed to the doodad pass once. Changing this field changes
    /// the record but nothing on screen until the tile is read again. See
    /// [`settle`].
    dressed: Option<(u16, u16)>,
    /// The other members as they stood when the button went down — see
    /// [`super::doodads::Held`], which has the same field.
    group_was: Vec<Selected>,
    /// Whether this press landed on a member of a group.
    collapse: bool,
    /// Frames to wait before the group's members' boxes are re-fitted
    /// ([`publish`]); zero when nothing is waiting.
    ///
    /// A count rather than a flag because the box is taken from the drawn
    /// parts' `GlobalTransform`, which Bevy propagates after `Update`. On the
    /// frame a key moves the group, the parts still stand where they were, and
    /// a box taken then is the old one. Two frames is one for the transform to
    /// be written and one for it to propagate.
    pub(crate) refit_members: u8,
    /// Frames to wait before the primary's box is re-fitted ([`publish`]);
    /// zero when nothing is waiting. One, for [`Self::refit_members`]' reason:
    /// on the frame a key or the panel changes the record, the parts'
    /// `GlobalTransform` still holds the old transform, and a box taken then
    /// is the box from before the move.
    refit_primary: u8,
    /// Whether the group has moved since its members were last put in the
    /// tiles their origins are in ([`settle`]).
    pub(crate) unsettled: bool,
}

impl Held {
    /// Say that the group's members have been moved from outside this tool's
    /// own systems, so [`publish`] re-fits their boxes and [`settle`] puts each
    /// in the tile its origin is in.
    pub(crate) fn members_moved(&mut self) {
        self.refit_members = 2;
        self.unsettled = true;
    }
}

/// Pick a building, or drop the one held.
#[allow(clippy::too_many_arguments)]
pub(crate) fn select(
    mut selection: ResMut<Selection>,
    mut held: ResMut<Held>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    parts: Query<(&ChildOf, &GlobalTransform, &Aabb), With<WmoPart>>,
    placements: Query<&WmoPlacement>,
    // The narrow phase's triangles — see the walk below.
    mut buildings: ResMut<WmoCache>,
    cursor: Res<Cursor>,
    // Tuples, because this system is past Bevy's limit of sixteen parameters.
    (gizmo, placing): (Res<super::gizmo::Gizmo>, Res<super::place::Placing>),
    (keys, mut marquee, mut menu): (
        Res<ButtonInput<KeyCode>>,
        ResMut<super::group::Marquee>,
        ResMut<crate::context::ContextMenu>,
    ),
) {
    if *tool != Tool::Wmos || !state.editing() {
        return;
    }
    // A right click: the building under the pointer, locked or not, for the
    // menu, as the doodad pick does. See `crate::context`.
    if menu.asked() {
        let Some(session) = session else { return };
        let hit = super::doodads::pointer_ray(&windows, &camera).and_then(|(origin, direction)| {
            let boxed = super::doodads::boxes_hit(
                parts.iter().filter_map(|(parent, at, aabb)| {
                    let id = placements.get(parent.parent()).ok()?.unique_id;
                    Some((id, at, aabb))
                }),
                origin,
                direction,
            );
            solid_hit(&boxed, &session, &mut buildings, origin, direction)
                .or_else(|| boxed.first().map(|&(_, unique_id, _)| unique_id))
        });
        menu.object = hit.map(|unique_id| {
            let locked = session.placement_locked(super::place::Kind::Wmo, unique_id);
            if !locked && !selection.holds(unique_id) {
                if let Some(found) = find(&session, unique_id) {
                    selection.only(Some(found));
                    picked_primary(&mut held, &selection);
                }
            }
            crate::context::Target::Wmo { unique_id, locked }
        });
        return;
    }
    // A click that places must not also select — see [`super::place`].
    if placing.armed() {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        held.armed = false;
        return;
    }
    if gizmo.holding() {
        held.armed = false;
        return;
    }
    let Some(session) = session else { return };
    let Ok(window) = windows.single() else { return };
    let Some(at) = window.cursor_position() else {
        return;
    };
    let Ok((camera, eye)) = camera.single() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(eye, at * window.scale_factor()) else {
        return;
    };

    // Against the drawn batches, exactly as the doodad pick is, and up to
    // the placement through the parent. A building's own `MODF` box is the
    // axis-aligned hull of the whole building, so ray-casting against it
    // directly would select on empty air beside a spire, or through an open
    // archway.
    let boxed = super::doodads::boxes_hit(
        parts.iter().filter_map(|(parent, at, aabb)| {
            let id = placements.get(parent.parent()).ok()?.unique_id;
            Some((id, at, aabb))
        }),
        ray.origin,
        *ray.direction,
    );
    // A locked building is not picked; see `crate::session::PlacementLocks`.
    let hit = solid_hit(&boxed, &session, &mut buildings, ray.origin, *ray.direction)
        .or_else(|| boxed.first().map(|&(_, unique_id, _)| unique_id))
        .filter(|unique_id| !session.placement_locked(super::place::Kind::Wmo, *unique_id))
        .and_then(|unique_id| find(&session, unique_id));
    // The same four cases as the doodad pick — see `super::doodads::select`.
    let adding = super::group::shift(&keys);
    held.dragging = false;
    held.collapse = false;
    match hit {
        Some(found) if adding => {
            selection.toggle(found);
            held.armed = false;
            picked_primary(&mut held, &selection);
            return;
        }
        Some(found) if !selection.also.is_empty() && selection.holds(found.unique_id) => {
            selection.promote(found.unique_id);
            held.collapse = true;
        }
        Some(found) => selection.only(Some(found)),
        None => {
            if !adding {
                selection.only(None);
            }
            marquee.begin(Tool::Wmos, at, adding);
            held.armed = false;
            picked_primary(&mut held, &selection);
            return;
        }
    }
    held.armed = true;
    held.from = cursor.ground;
    held.group_was = selection.also.clone();
    picked_primary(&mut held, &selection);
}

/// Take the primary's record as the one [`publish`] and [`settle`] compare
/// against, so selecting a building is not an edit — see [`Held::picked`].
fn picked_primary(held: &mut Held, selection: &Selection) {
    held.was = selection.at.as_ref().map(|at| at.record);
    held.picked = held.was;
    held.dressed = held.was.map(|record| (record.doodad_set, record.name_set));
}

/// Select every building drawn whose origin is inside a finished rectangle —
/// [`super::doodads::enclose`]'s counterpart, over the `WmoPlacement`s.
#[allow(clippy::too_many_arguments)]
fn enclose(
    mut selection: ResMut<Selection>,
    mut held: ResMut<Held>,
    mut marquee: ResMut<super::group::Marquee>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    placements: Query<&WmoPlacement>,
) {
    if *tool != Tool::Wmos {
        return;
    }
    let Some(done) = marquee.finished(Tool::Wmos) else {
        return;
    };
    let Some(session) = session else { return };
    let Some((camera, eye, window)) = super::group::view(&camera, &windows) else {
        return;
    };
    let shown: std::collections::HashSet<u32> = placements.iter().map(|p| p.unique_id).collect();
    let mut found: Vec<(f32, Selected)> = Vec::new();
    for (coord, tile) in &session.tiles {
        let names = tile.building_names();
        for (index, record) in tile.building_list().iter().enumerate() {
            if !shown.contains(&record.unique_id)
                || session.placement_locked(super::place::Kind::Wmo, record.unique_id)
            {
                continue;
            }
            let world = vale_assets::world::adt::placement_to_world(record.position);
            if vale_assets::tile_for_position(world[0], world[1]) != *coord {
                continue;
            }
            let Some(point) = super::group::on_screen(camera, eye, window, Vec3::from(world))
            else {
                continue;
            };
            if !done.rect.contains(point) {
                continue;
            }
            found.push((
                point.distance(done.rect.center()),
                Selected {
                    tile: *coord,
                    index,
                    unique_id: record.unique_id,
                    path: names
                        .get(record.name_id as usize)
                        .cloned()
                        .unwrap_or_default(),
                    record: *record,
                },
            ));
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    if !done.adding {
        selection.only(None);
    }
    for (_, at) in found {
        if selection.holds(at.unique_id) {
            continue;
        }
        match selection.at.is_none() {
            true => selection.at = Some(at),
            false => selection.also.push(at),
        }
    }
    picked_primary(&mut held, &selection);
}

/// The narrow phase: the nearest of the candidate buildings whose own solid
/// triangles the ray crosses, or `None` when it crosses none.
///
/// The same two-phase pick the doodads use, for the reason given in
/// [`super::doodads::nearest`] about the fault in using a box alone. A
/// building's batches are per group, so their boxes are already tighter than
/// a doodad's; they are still wrong for large flat shapes, where a city
/// wall's box is a slab of empty air in front of it.
///
/// This tests against the collision hull rather than the drawn triangles,
/// which is a compromise: the WMO cache keeps a `CollisionMesh` on the CPU
/// and not the drawn geometry, so a batch the file marks as not solid — a
/// banner, a window frame, a rail — has nothing here to hit. Those batches
/// fall back to their box, which is what they had before this function
/// existed, so the fallback does not make anything worse while it fixes the
/// pick for the buildings people actually click.
fn solid_hit(
    boxed: &[(f32, u32, &GlobalTransform)],
    session: &EditSession,
    buildings: &mut WmoCache,
    origin: Vec3,
    direction: Vec3,
) -> Option<u32> {
    let mut best: Option<(f32, u32)> = None;
    let mut tested: Vec<u32> = Vec::new();
    for (_, unique_id, at) in boxed {
        if tested.contains(unique_id) {
            continue;
        }
        tested.push(*unique_id);
        let Some(path) = find(session, *unique_id).map(|found| found.path) else {
            continue;
        };
        let wmo_render::Lookup::Ready(ready) = buildings.lookup(&path) else {
            continue;
        };
        let hull = ready.collision();
        let placed = at.affine();
        // The hull's vertices are the file's own, in the world's axes and in the
        // building's model space; the part's transform is the placement's,
        // because a part is a child of the placement and carries the identity.
        let corner = |index: u32| {
            hull.positions.get(index as usize).map(|&p| {
                placed
                    .transform_point3(Vec3::from(axes::to_bevy(p)))
                    .to_array()
            })
        };
        for triangle in hull.indices.chunks_exact(3) {
            let (Some(a), Some(b), Some(c)) = (
                corner(triangle[0]),
                corner(triangle[1]),
                corner(triangle[2]),
            ) else {
                continue;
            };
            let Some(distance) = vale_assets::look::pick::ray_triangle(
                origin.to_array(),
                direction.to_array(),
                a,
                b,
                c,
            ) else {
                continue;
            };
            if best.is_none_or(|(had, _)| distance < had) {
                best = Some((distance, *unique_id));
            }
        }
    }
    best.map(|(_, unique_id)| unique_id)
}

/// Find the open tile and list position a unique id belongs to.
///
/// Returns the copy the renderer is drawing, not the first one found. A
/// model touching two tiles is listed in both with one `unique_id` — a city
/// is listed in many — and `render::terrain` draws the copy whose origin is
/// in the tile that holds it. Taking the first match instead would edit a
/// row nobody draws: the thing on screen would not move, and the row that
/// does move would never be selected.
///
/// This applies the renderer's own claim rule for that reason. A placement
/// that no tile claims — which is what a half-finished edit leaves — falls
/// back to the first match, because refusing to select it would leave no way
/// to put it right.
fn find(session: &EditSession, unique_id: u32) -> Option<Selected> {
    let mut fallback: Option<Selected> = None;
    for (coord, tile) in &session.tiles {
        let names = tile.building_names();
        for (index, record) in tile.building_list().iter().enumerate() {
            if record.unique_id != unique_id {
                continue;
            }
            let found = Selected {
                tile: *coord,
                index,
                unique_id,
                path: names
                    .get(record.name_id as usize)
                    .cloned()
                    .unwrap_or_default(),
                record: *record,
            };
            // The claim rule, asked of this row: `render::terrain` draws the copy
            // whose origin is in the tile that lists it.
            let world = vale_assets::world::adt::placement_to_world(record.position);
            if vale_assets::tile_for_position(world[0], world[1]) == *coord {
                return Some(found);
            }
            fallback.get_or_insert(found);
        }
    }
    fallback
}

/// Every placement of one building in the open tiles, each the copy the
/// renderer draws — [`super::doodads::all_of`]'s counterpart.
pub(crate) fn all_of(session: &EditSession, path: &str) -> Vec<Selected> {
    let mut found = Vec::new();
    for (coord, tile) in &session.tiles {
        let names = tile.building_names();
        for (index, record) in tile.building_list().iter().enumerate() {
            let named = names.get(record.name_id as usize).cloned().unwrap_or_default();
            if !named.eq_ignore_ascii_case(path)
                || session.placement_locked(super::place::Kind::Wmo, record.unique_id)
            {
                continue;
            }
            let world = vale_assets::world::adt::placement_to_world(record.position);
            if vale_assets::tile_for_position(world[0], world[1]) != *coord {
                continue;
            }
            found.push(Selected {
                tile: *coord,
                index,
                unique_id: record.unique_id,
                path: named,
                record: *record,
            });
        }
    }
    found
}

/// Drag the held building across the ground.
#[allow(clippy::too_many_arguments)]
fn drag(
    mut selection: ResMut<Selection>,
    mut held: ResMut<Held>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Res<Cursor>,
    gizmo: Res<super::gizmo::Gizmo>,
) {
    if *tool != Tool::Wmos || !state.editing() || gizmo.holding() {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    if buttons.just_released(MouseButton::Left) {
        session.history.end();
        // A press on a member that never moved selects that member alone.
        if held.collapse && !held.dragging {
            let primary = selection.at.take();
            selection.only(primary);
        }
        held.dragging = false;
        held.armed = false;
        held.collapse = false;
        return;
    }
    if !buttons.pressed(MouseButton::Left) || !held.armed {
        return;
    }
    let (Some(at), Some(from), Some(was)) = (selection.at.as_mut(), held.from, held.was) else {
        return;
    };
    let Some(now) = cursor.ground else { return };
    let step = now - from;
    if step.length_squared() < 1e-6 && !held.dragging {
        return;
    }
    if !held.dragging {
        held.dragging = true;
        session.history.begin(match held.group_was.len() {
            0 => "Move WMO".to_string(),
            more => format!("Move {} WMOs", more + 1),
        });
    }

    let drop = keys.pressed(KeyCode::ControlLeft);
    let world = vale_assets::world::adt::placement_to_world(was.position);
    let mut moved = Vec3::from(world) + step;
    if drop {
        moved.z = now.z;
    }
    set_world_position(&mut at.record, &was, [moved.x, moved.y, moved.z]);
    write_record(session, at);

    // The rest of the group by the same step, each from where it began, and
    // with a ctrl-drag each onto the ground under its own origin.
    let group = std::mem::take(&mut held.group_was);
    for member in &group {
        let Some(standing) = selection
            .also
            .iter_mut()
            .find(|at| at.unique_id == member.unique_id)
        else {
            continue;
        };
        let world = vale_assets::world::adt::placement_to_world(member.record.position);
        let mut moved = Vec3::from(world) + step;
        if drop {
            if let Some(z) = super::doodads::ground_height(session, moved.x, moved.y) {
                moved.z = z;
            }
        }
        set_world_position(&mut standing.record, &member.record, moved.to_array());
        write_record(session, standing);
    }
    if !group.is_empty() {
        held.refit_members = 2;
        held.unsettled = true;
    }
    held.group_was = group;
}

/// Carry the rest of the group by the change the primary has just had from
/// `was`: the same step and the same turn about the primary's old origin.
/// Written into whatever history entry is open. See
/// [`super::doodads::carry_members`], which this is the counterpart of; a
/// building has no scale, and its doodad and name sets are its own.
pub(crate) fn carry_members(session: &mut EditSession, selection: &mut Selection, was: &Building) {
    let Some(now) = selection.at.as_ref().map(|at| at.record) else {
        return;
    };
    if now.position == was.position && now.rotation == was.rotation {
        return;
    }
    let pivot = Vec3::from(vale_assets::world::adt::placement_to_world(was.position));
    let step = Vec3::from(vale_assets::world::adt::placement_to_world(now.position)) - pivot;
    let turned = now.rotation != was.rotation;
    let turn = match turned {
        true => super::group::turn_between(was.rotation, now.rotation),
        false => Quat::IDENTITY,
    };
    for member in selection.also.iter_mut() {
        let before = member.record;
        let world = Vec3::from(vale_assets::world::adt::placement_to_world(before.position));
        let moved = super::group::orbit(pivot, world, turn) + step;
        set_world_position(&mut member.record, &before, moved.to_array());
        if turned {
            member.record.rotation = super::group::turn_record(before.rotation, turn);
        }
        write_record(session, member);
    }
}

/// Move a building to a world position, taking its box with it.
///
/// See [`refit`]. `pub(crate)` because the panel moves a building too, and a
/// second copy of this would be a second place the box could be forgotten.
/// The record is moved from `was` rather than from itself so that
/// a drag is exact rather than an accumulation of frames, which is the same
/// reason the doodad tool holds a `was`.
pub(crate) fn set_world_position(record: &mut Building, was: &Building, to: [f32; 3]) {
    let position = vale_assets::world::adt::placement_from_world(to);
    let step = [
        position[0] - was.position[0],
        position[1] - was.position[1],
        position[2] - was.position[2],
    ];
    record.position = position;
    for axis in 0..3 {
        record.bounds_lower[axis] = was.bounds_lower[axis] + step[axis];
        record.bounds_upper[axis] = was.bounds_upper[axis] + step[axis];
    }
}

/// Turn, nudge and the rest, from the keyboard.
#[allow(clippy::too_many_arguments)]
fn nudge(
    mut selection: ResMut<Selection>,
    mut held: ResMut<Held>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    time: Res<Time>,
) {
    if *tool != Tool::Wmos || !state.editing() || wants.wants_keyboard_input() {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    let Some(at) = selection.at.as_mut() else {
        return;
    };
    let was = at.record;
    let far = if keys.pressed(KeyCode::ShiftLeft) {
        10.0
    } else {
        1.0
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
        let world = Vec3::from(vale_assets::world::adt::placement_to_world(was.position));
        let moved = world + step * STEP * far;
        set_world_position(&mut at.record, &was, [moved.x, moved.y, moved.z]);
    }

    let mut turn = 0.0;
    if keys.just_pressed(KeyCode::Comma) {
        turn -= TURN * far;
    }
    if keys.just_pressed(KeyCode::Period) {
        turn += TURN * far;
    }
    if turn != 0.0 {
        let mut world = vale_assets::world::adt::placement_euler_to_world(was.rotation);
        world[2] = (world[2] + turn).rem_euclid(360.0);
        at.record.rotation = vale_assets::world::adt::placement_euler_from_world(world);
    }

    if at.record == was {
        return;
    }
    let (label, kind) = what_changed(&was, &at.record);
    let group = !selection.also.is_empty();
    let Some(at) = selection.at.as_ref() else {
        return;
    };
    let subject = match group {
        false => format!("wmo {} {kind}", at.unique_id),
        true => format!("wmo group {} {kind}", at.unique_id),
    };
    session
        .history
        .begin_gesture(label, subject, time.elapsed_secs_f64());
    write_record(session, at);

    // The rest of the group: the same step, and a turn about the primary's
    // origin — see `super::doodads::nudge`.
    let pivot = Vec3::from(vale_assets::world::adt::placement_to_world(was.position));
    let spin = super::group::turn_about(Vec3::Z, turn);
    for member in selection.also.iter_mut() {
        let before = member.record;
        let world = Vec3::from(vale_assets::world::adt::placement_to_world(before.position));
        let moved = super::group::orbit(pivot, world, spin) + step * STEP * far;
        set_world_position(&mut member.record, &before, moved.to_array());
        if turn != 0.0 {
            member.record.rotation = super::group::turn_record(before.rotation, spin);
        }
        write_record(session, member);
    }
    if group {
        held.refit_members = 2;
        held.unsettled = true;
        session.status = format!("{label} · {} selected", selection.count());
    }
    session.history.end();
}

/// What changed between two records: a name for the history, and the subject a
/// gesture folds by — [`super::doodads::what_changed`]'s counterpart.
pub(crate) fn what_changed(was: &Building, now: &Building) -> (&'static str, &'static str) {
    if was.position != now.position {
        return ("Move WMO", "position");
    }
    if was.rotation != now.rotation {
        return ("Turn WMO", "rotation");
    }
    if was.doodad_set != now.doodad_set {
        return ("Set WMO doodad set", "doodadset");
    }
    if was.name_set != now.name_set {
        return ("Set WMO name set", "nameset");
    }
    ("Edit WMO", "record")
}

/// Take the held building out of the tile.
///
/// The same shape as a doodad removal and for the same reason: it renumbers
/// every `MODF` reference above it in all 256 chunks' `MCRF`, so it is recorded
/// as the placement lists whole and the tile is read again.
fn remove(
    mut selection: ResMut<Selection>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut menu: ResMut<crate::context::ContextMenu>,
) {
    if *tool != Tool::Wmos || !state.editing() {
        return;
    }
    // The Delete key, or the right-click menu's Delete.
    let asked = menu
        .take_request(|r| *r == crate::context::Request::Delete)
        .is_some();
    if !asked && (wants.wants_keyboard_input() || !keys.just_pressed(KeyCode::Delete)) {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    let Some(at) = selection.at.take() else {
        return;
    };
    // Every member, as one entry — see [`super::group::remove_rows`].
    let rows: Vec<((u32, u32), usize)> = std::iter::once(&at)
        .chain(selection.also.iter())
        .map(|member| (member.tile, member.index))
        .collect();
    let label = match rows.len() {
        1 => "Remove WMO".to_string(),
        more => format!("Remove {more} WMOs"),
    };
    selection.also.clear();
    let removed = super::group::remove_rows(session, &rows, true, &label);
    session.status = match removed {
        1 => format!("removed {}", super::textures::leaf(&at.path)),
        more => format!("removed {more} WMOs"),
    };
}

/// Write one building's new record into the tile and onto the history.
///
/// `before` is read off the file by `Edit::move_building`, which is why this
/// function takes no `was` parameter — see `vale_edit::ops::Edit::move_doodad`
/// for the invariant.
pub(crate) fn write_record(session: &mut EditSession, at: &Selected) {
    let key = session.key(at.tile);
    let Some(tile) = session.tiles.get_mut(&at.tile) else {
        return;
    };
    let Some(edit) = Edit::move_building(tile, at.index, at.record) else {
        return;
    };
    edit.apply(tile);
    session.history.record(&key, [edit]);
    session.moved_building(at.tile, at.index);
}

/// Put the drawn buildings where the file now says they are.
///
/// ## What this function updates: transform, interior lighting, and furniture
///
/// The entity's `Transform` is updated directly. [`Interior::inverse`] must
/// also be updated: the playtest's indoor test uses it to bring a character
/// into the building's own frame, so a stale inverse lights whoever walks in
/// as though they were outdoors, and lights the empty ground where the
/// building stood as though someone were still inside.
///
/// The building's furniture must move too. A building's `MODD` spawns are
/// folded into world space when it is spawned (`mul4(&placement.matrix,
/// &d.matrix)`) and handed to the doodad pass as placements of their own, so
/// they are not children of the building and do not move with it on their
/// own. Without this update, moving a building's shell leaves its beds,
/// barrels and bookshelves standing where the building used to be.
///
/// The furniture can be moved here because each `MODD` spawn carries its
/// building's `unique_id` (`wmos::interior_placement`), so the delta between
/// where the placement was and where it is going applies to every one of
/// them. The delta is taken from the entity's current transform rather than
/// from a remembered matrix, which makes this idempotent: a building just
/// spawned from the file is already where it is going, so the delta is the
/// identity and the furniture is left alone.
///
/// The collision hulls move by the same delta, through
/// `CollisionWorld::move_placement`: the building's own hull and its
/// furniture's both carry the building's `unique_id`. The `MOLT` lamps still
/// cannot move live; they come back only when the tile is read again.
fn reconcile(
    mut session: Option<ResMut<EditSession>>,
    mut placements: Query<(&WmoPlacement, &mut Transform, Option<&mut Interior>)>,
    mut furniture: Query<(&Doodad, &mut Transform), Without<WmoPlacement>>,
    solids: Res<vale_client::world::session::Solids>,
    focus: Res<vale_client::render::focus::WorldFocus>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if session.moved_buildings.is_empty() {
        return;
    }
    let mut wanted: Vec<((u32, u32), u32, Mat4)> = Vec::new();
    for (coord, indices) in std::mem::take(&mut session.moved_buildings) {
        let Some(tile) = session.tiles.get(&coord) else {
            continue;
        };
        for index in indices {
            let Some(record) = tile.building_at(index) else {
                continue;
            };
            wanted.push((coord, record.unique_id, matrix_of(&record)));
        }
    }

    let mut moved: Vec<(u32, Mat4)> = Vec::new();
    for (placement, mut transform, interior) in &mut placements {
        let Some((_, _, matrix)) = wanted.iter().find(|(_, id, _)| *id == placement.unique_id)
        else {
            continue;
        };
        let to = axes::to_bevy_affine(*matrix);
        // The delta, in the axes the entities are drawn in, before the placement
        // is written — see the note above on why it comes from the entity.
        let delta = to * transform.to_matrix().inverse();
        *transform = Transform::from_matrix(to);
        // In the world's own axes, which is what `Interior` is documented to
        // hold: the entities it tests arrive from the object manager in those.
        if let Some(mut interior) = interior {
            interior.inverse = matrix.inverse();
        }
        // The hulls, by the same delta in the world's own axes. A building
        // just spawned from the file is already where it is going, and a
        // rebuild of its hull for an identity move would cost milliseconds
        // for nothing.
        if !delta.abs_diff_eq(Mat4::IDENTITY, 1.0e-5) {
            let in_world = axes::from_bevy_affine(delta).to_cols_array();
            solids.0.move_placement(focus.map_id, placement.unique_id, &in_world);
        }
        moved.push((placement.unique_id, delta));
    }

    for (doodad, mut transform) in &mut furniture {
        let Some((_, delta)) = moved.iter().find(|(id, _)| *id == doodad.unique_id) else {
            continue;
        };
        *transform = Transform::from_matrix(*delta * transform.to_matrix());
    }

    // A placement with no matching entity is marked stale rather than
    // dropped. This happens while a tile is being replaced: the outgoing
    // copy is gone and the incoming one has not spawned yet, so a write that
    // would have corrected the placement's position is lost, and the
    // building stays wherever the in-flight read left it. Re-reading the
    // tile converges on the correct state, since the file is already right
    // and `remesh` starts a new replacement once the in-flight one lands.
    for (coord, unique_id, _) in wanted {
        if !placements.iter().any(|(p, _, _)| p.unique_id == unique_id) {
            session.stale.insert(coord);
        }
    }
}

/// One `MODF` record as the matrix the renderer places it with.
fn matrix_of(record: &Building) -> Mat4 {
    let world = vale_assets::world::adt::placement_to_world(record.position);
    Mat4::from_cols_array(&vale_assets::world::adt::placement_matrix(
        world,
        record.rotation,
        1.0,
    ))
}

/// Take the selected building's `MODF` box from what is actually drawn.
///
/// ## Why the `MODF` box must track the building's position
///
/// `MODF` states each placement's own world-space box, and the character
/// mover reads it twice before the `.wmo` file has been loaded at all:
/// `awaiting_building` decides whether to wait rather than use the terrain
/// height (the ground under a city is far below its streets, so using it
/// drops the character through the world), and `inside_building` decides
/// whether terrain above the character's head may be stood on, which is what
/// keeps a character from being placed on top of Ironforge's mountain.
///
/// A building whose box has not been updated breaks both checks: the old
/// position still says to wait, and the new position says nothing.
///
/// A translation moves the box exactly, and [`set_position`] does that as
/// part of the move. A rotation does not: there is no arithmetic on the
/// stored box that recovers its correct extents after a turn, since rotating
/// a box and re-fitting it inflates the box, and inflates it again on the
/// next turn. What recovers it is the union of the placement's own drawn
/// batches, each an `Aabb` the renderer computed from the real geometry.
/// This function takes that union once, when the mouse button comes up.
///
/// The result is not identical to Blizzard's box: theirs covers the whole
/// model, while this covers only what is drawn, so a part that draws nothing
/// is excluded. It has the right shape and the right position, which is what
/// both readers of the box need.
fn refit(
    unique_id: u32,
    parts: &Query<(&ChildOf, &GlobalTransform, &Aabb), With<WmoPart>>,
    placements: &Query<&WmoPlacement>,
) -> Option<([f32; 3], [f32; 3])> {
    let mut lower = Vec3::splat(f32::INFINITY);
    let mut upper = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;
    for (parent, transform, aabb) in parts.iter() {
        if placements.get(parent.parent()).ok()?.unique_id != unique_id {
            continue;
        }
        let centre = Vec3::from(aabb.center);
        let half = Vec3::from(aabb.half_extents);
        for corner in 0..8 {
            let offset = Vec3::new(
                if corner & 1 == 0 { -half.x } else { half.x },
                if corner & 2 == 0 { -half.y } else { half.y },
                if corner & 4 == 0 { -half.z } else { half.z },
            );
            let point = transform.affine().transform_point3(centre + offset);
            lower = lower.min(point);
            upper = upper.max(point);
            found = true;
        }
    }
    if !found {
        return None;
    }
    // Back into the file's own frame, and re-sorted: the conversion swaps axes
    // and negates two of them, so the corner that was lowest is not any more.
    let a = vale_assets::world::adt::placement_from_world(axes::to_wow(lower));
    let b = vale_assets::world::adt::placement_from_world(axes::to_wow(upper));
    let mut low = [0.0f32; 3];
    let mut high = [0.0f32; 3];
    for axis in 0..3 {
        low[axis] = a[axis].min(b[axis]);
        high[axis] = a[axis].max(b[axis]);
    }
    Some((low, high))
}

/// Whether two corners of a box are far enough apart to be worth writing.
///
/// Uses a tolerance rather than `!=`. This tool's box is the union of what
/// is drawn; Blizzard's is the whole model. `vale wmos` measures the two as
/// agreeing to 0.00 yards, but that agreement is a physical claim, and `!=`
/// on an `f32` is not one — an exact comparison would write a change for a
/// difference nothing can see. Two centimetres is far below what either
/// reader of this box cares about, since both only ask whether a point is
/// inside it.
fn moved_by(was: [f32; 3], now: [f32; 3]) -> bool {
    (0..3).any(|axis| (was[axis] - now[axis]).abs() > 0.02)
}

/// Publish the edited bytes, re-fit the box, and ask for the tile again.
///
/// All three run once the placement has settled, and none of them run while
/// it is still being moved: writing a tile out is two megabytes, the box
/// needs a transform that has stopped changing, and re-reading the tile is
/// what brings the collision hull and the `MOLT` lamps to the building's new
/// position.
///
/// This checks settled state rather than a release event, because a drag is
/// only one of four ways to change a building's rotation, and the only one
/// with a release to trigger on: `,` and `.` step it, `Alt` plus the mouse
/// sweeps it, and the panel's own field drags it. Each of these changes
/// `MODF`'s three angles and leaves the box behind, and a stale box is not
/// visible on screen — both of its readers are the character mover, asking
/// about a building it has not loaded yet. See [`refit`].
///
/// The trigger is therefore a state rather than an edge: no mouse button
/// down, no `Alt` held, and the record different from the one the selection
/// was picked with. This fires exactly once per settle, because the last
/// thing it does is take a fresh copy of the record.
#[allow(clippy::too_many_arguments)]
fn publish(
    mut session: Option<ResMut<EditSession>>,
    mut selection: ResMut<Selection>,
    mut held: ResMut<Held>,
    tool: Res<Tool>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    parts: Query<(&ChildOf, &GlobalTransform, &Aabb), With<WmoPart>>,
    placements: Query<&WmoPlacement>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if *tool != Tool::Wmos {
        return;
    }
    // Still being moved by one of the four: the box would be fitted to a
    // transform that is about to change again, and the tile would be read once
    // a frame.
    if buttons.pressed(MouseButton::Left)
        || keys.pressed(KeyCode::AltLeft)
        || keys.pressed(KeyCode::AltRight)
    {
        return;
    }
    // Only when the placement actually moved. A click that only selects must
    // not register as an edit — see [`Held::picked`].
    let changed = match (selection.at.as_ref(), held.picked) {
        (Some(at), Some(picked)) => {
            at.record.position != picked.position || at.record.rotation != picked.rotation
        }
        _ => false,
    };
    // The primary is re-fitted one frame after the change is seen, when the
    // parts' transforms have caught up — see [`Held::refit_primary`].
    let moved = match (changed, held.refit_primary) {
        (false, _) => {
            held.refit_primary = 0;
            false
        }
        (true, 0) => {
            held.refit_primary = 1;
            false
        }
        (true, _) => {
            held.refit_primary = 0;
            true
        }
    };
    // …and the rest of a group, which is re-fitted whenever the group has
    // moved: a group change moves every member, so there is no member that
    // was merely selected.
    // Counted down to the frame the parts' transforms have caught up with the
    // move — see [`Held::refit_members`].
    let members = match held.refit_members {
        0 => false,
        1 => {
            held.refit_members = 0;
            true
        }
        _ => {
            held.refit_members -= 1;
            false
        }
    };
    if !moved && !members {
        return;
    }

    // The re-fit is folded into the move's own undo entry rather than added
    // as a separate one, since it is a consequence of the move rather than an
    // edit anyone asked for. As its own entry, the first `Ctrl+Z` after a
    // drag would undo only the re-fit, leaving the building at its new
    // position until a second undo. See `vale_edit::undo::History::amend`,
    // including the case where there is nothing to fold into.
    let mut opened = false;
    let mut fit = |session: &mut EditSession, at: &mut Selected| {
        let Some((lower, upper)) = refit(at.unique_id, &parts, &placements) else {
            return;
        };
        if !moved_by(at.record.bounds_lower, lower) && !moved_by(at.record.bounds_upper, upper) {
            return;
        }
        at.record.bounds_lower = lower;
        at.record.bounds_upper = upper;
        if !opened {
            opened = true;
            if !session.history.amend() {
                session.history.begin("Fit WMO bounds");
            }
        }
        write_record(session, at);
    };
    if moved {
        if let Some(at) = selection.at.as_mut() {
            fit(session, at);
        }
    }
    if members {
        for member in selection.also.iter_mut() {
            fit(session, member);
        }
    }
    if opened {
        session.history.end();
    }
    held.picked = selection.at.as_ref().map(|at| at.record);

    let unsaved: Vec<(u32, u32)> = session.unsaved.iter().copied().collect();
    for coord in unsaved {
        session.publish(coord);
    }
    // The hull and the lamps only move on a re-read. The re-read is invisible
    // because `super::terrain::swap` keeps the old tile drawn until the new
    // one replaces it.
    let tiles: Vec<(u32, u32)> = match members {
        true => selection.members().map(|at| at.tile).collect(),
        false => selection.at.iter().map(|at| at.tile).collect(),
    };
    session.stale.extend(tiles);
}

/// Handles two things that are true only once a change has settled: a record
/// that has moved to a different tile, and a dressing (doodad or name set)
/// that nothing has re-read.
///
/// Runs only while no mouse button is down, which debounces it: a
/// `DragValue` held for a second writes the record sixty times, and
/// re-reading the tile on each of those would mean sixty rebuilds. Waiting
/// for the button to come up turns one change into one re-read. A value
/// typed and entered has no button down at all, so this fires on the next
/// frame.
fn settle(
    mut session: Option<ResMut<EditSession>>,
    mut selection: ResMut<Selection>,
    mut held: ResMut<Held>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if *tool != Tool::Wmos || !state.editing() || buttons.pressed(MouseButton::Left) {
        return;
    }
    let Some(at) = selection.at.as_ref() else {
        return;
    };
    let (tile, index) = (at.tile, at.index);

    // The dressing, which only a re-read applies: the `MODD` spawns of the
    // chosen set are folded into world space when the building is spawned and
    // never again.
    let sets = (at.record.doodad_set, at.record.name_set);
    if held.dressed.is_some_and(|had| had != sets) {
        held.dressed = Some(sets);
        session.publish(tile);
        session.stale.insert(tile);
        session.status = format!("doodad set {} · name set {}", sets.0, sets.1);
    }

    // …and keeps the file's own invariant: one row, in the tile that contains
    // this placement's origin. See [`super::rehome`] for why this is stated
    // as an invariant rather than a move, and why a city listed in a dozen
    // tiles makes the difference between one copy and fifteen.
    let _ = index;
    // Not while the primary's box waits to be re-fitted: the settle takes a
    // fresh copy of the record, which would end the wait with the old box.
    let settled = match held.refit_primary {
        0 => super::rehome::settle_building(session, at.unique_id),
        _ => None,
    };
    if let Some((coord, index)) = settled {
        if let Some(at) = selection.at.as_mut() {
            at.tile = coord;
            at.index = index;
            // And the record, because `add_building` gave it this tile's own
            // `name_id`. Without it `resync` sees a mismatch, searches, and the
            // next frame starts again from whatever it found.
            if let Some(row) = session.tiles.get(&coord).and_then(|t| t.building_at(index)) {
                at.record = row;
            }
        }
        held.picked = selection.at.as_ref().map(|at| at.record);
        held.dressed = selection
            .at
            .as_ref()
            .map(|at| (at.record.doodad_set, at.record.name_set));
    }

    // The rest of the group, once after a change to it, and after its boxes
    // have been re-fitted, so a member is moved into its new tile with the box
    // it will keep. See `super::doodads::settle` for why not every frame.
    if held.refit_members > 0 || !std::mem::take(&mut held.unsettled) {
        return;
    }
    let members: Vec<u32> = selection.also.iter().map(|m| m.unique_id).collect();
    for unique_id in members {
        let Some((coord, index)) = super::rehome::settle_building(session, unique_id) else {
            continue;
        };
        let row = session.tiles.get(&coord).and_then(|t| t.building_at(index));
        if let (Some(member), Some(row)) = (
            selection.also.iter_mut().find(|m| m.unique_id == unique_id),
            row,
        ) {
            member.tile = coord;
            member.index = index;
            member.record = row;
        }
    }
}

/// Put the selection back in step with the file — [`super::doodads::resync`]'s
/// counterpart, by `unique_id` for the same reason, members included.
fn resync(mut selection: ResMut<Selection>, session: Option<Res<EditSession>>) {
    let Some(session) = session else { return };
    let standing = |at: &Selected| {
        session
            .tiles
            .get(&at.tile)
            .and_then(|tile| tile.building_at(at.index))
            .is_some_and(|row| row.unique_id == at.unique_id && row == at.record)
    };
    if selection.members().all(|at| standing(at)) {
        return;
    }
    if let Some(at) = selection.at.as_ref() {
        if !standing(at) {
            selection.at = find(&session, at.unique_id);
        }
    }
    selection.also.retain_mut(|member| {
        if standing(member) {
            return true;
        }
        match find(&session, member.unique_id) {
            Some(found) => {
                *member = found;
                true
            }
            None => false,
        }
    });
    if selection.at.is_none() {
        selection.at = selection.also.pop();
    }
}

/// One box per building: the hull of its drawn parts' boxes, in the
/// building's own frame, so it turns with the building.
///
/// One box per part drew a selected house as a nest of thirty boxes, one for
/// each group and batch. The hull says which building is selected, which is
/// what the box is for. The `MODF` box is not drawn: it is the axis-aligned
/// hull in the world's frame, larger than the building whenever the building
/// is turned.
fn building_boxes<'a>(
    parts: impl Iterator<Item = (&'a ChildOf, &'a GlobalTransform, &'a Aabb)>,
    placements: &Query<(&WmoPlacement, &GlobalTransform)>,
    wanted: impl Fn(u32) -> bool,
) -> Vec<(u32, Transform)> {
    let mut hulls: Vec<(Entity, u32, GlobalTransform, Vec3, Vec3)> = Vec::new();
    for (parent, transform, aabb) in parts {
        let Ok((placement, frame)) = placements.get(parent.parent()) else {
            continue;
        };
        if !wanted(placement.unique_id) {
            continue;
        }
        let into_frame = frame.affine().inverse() * transform.affine();
        let (centre, half) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
        let at = match hulls.iter().position(|hull| hull.0 == parent.parent()) {
            Some(at) => at,
            None => {
                hulls.push((parent.parent(), placement.unique_id, *frame, Vec3::MAX, Vec3::MIN));
                hulls.len() - 1
            }
        };
        let hull = &mut hulls[at];
        for corner in 0..8 {
            let sign = Vec3::new(
                if corner & 1 == 0 { -1.0 } else { 1.0 },
                if corner & 2 == 0 { -1.0 } else { 1.0 },
                if corner & 4 == 0 { -1.0 } else { 1.0 },
            );
            let point = into_frame.transform_point3(centre + half * sign);
            hull.3 = hull.3.min(point);
            hull.4 = hull.4.max(point);
        }
    }
    hulls
        .into_iter()
        .map(|(_, unique_id, frame, lower, upper)| {
            let (scale, rotation, _) = frame.to_scale_rotation_translation();
            let centre = frame.affine().transform_point3((lower + upper) * 0.5);
            let pose = Transform::from_translation(centre)
                .with_rotation(rotation)
                .with_scale((upper - lower) * scale);
            (unique_id, pose)
        })
        .collect()
}

/// Box every locked WMO in red while the tool is chosen. See
/// `crate::session::PlacementLocks`.
fn draw_locked(
    mut marks: ResMut<Marks>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    parts: Query<(&ChildOf, &GlobalTransform, &Aabb), With<WmoPart>>,
    placements: Query<(&WmoPlacement, &GlobalTransform)>,
) {
    if *tool != Tool::Wmos {
        return;
    }
    let Some(session) = session else { return };
    let locked = &session.placement_locks().wmos;
    if locked.is_empty() {
        return;
    }
    for (_, pose) in building_boxes(parts.iter(), &placements, |id| locked.contains(&id)) {
        marks.box_edges(pose, super::doodads::LOCKED.with_alpha(0.5), Look::Ghosted);
    }
}

/// A box around each selected building; see [`building_boxes`]. Ghosted, so a
/// box inside a cathedral is still seen, faintly, through its walls. See
/// `crate::marks`.
fn draw_marker(
    mut marks: ResMut<Marks>,
    selection: Res<Selection>,
    tool: Res<Tool>,
    parts: Query<(&ChildOf, &GlobalTransform, &Aabb), With<WmoPart>>,
    placements: Query<(&WmoPlacement, &GlobalTransform)>,
) {
    if *tool != Tool::Wmos {
        return;
    }
    let Some(at) = selection.at.as_ref() else {
        return;
    };
    let colour = Color::srgb(0.4, 0.85, 1.0).with_alpha(0.5);
    // The rest of a group paler, so the primary is still the one that stands out.
    let member = Color::srgb(0.55, 0.75, 0.9).with_alpha(0.3);
    // Sorted and searched rather than hashed — see `super::doodads::draw_marker`.
    let others = super::group::sorted(selection.also.iter().map(|m| m.unique_id));
    let wanted = |id: u32| id == at.unique_id || super::group::holds(&others, id);
    for (unique_id, pose) in building_boxes(parts.iter(), &placements, wanted) {
        let colour = match unique_id == at.unique_id {
            true => colour,
            false => member,
        };
        marks.box_edges(pose, colour, Look::Ghosted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn building(position: [f32; 3]) -> Building {
        Building {
            name_id: 0,
            unique_id: 7,
            position,
            rotation: [0.0; 3],
            bounds_lower: [position[0] - 10.0, position[1] - 5.0, position[2] - 20.0],
            bounds_upper: [position[0] + 10.0, position[1] + 5.0, position[2] + 20.0],
            flags: 0,
            doodad_set: 0,
            name_set: 0,
            padding: 0,
        }
    }

    /// Moving a building moves its box with it. The box is what the mover
    /// reads before the `.wmo` has loaded — to decide whether to wait rather
    /// than fall through the world, and whether terrain overhead may be stood
    /// on — so one left behind is wrong in two places at once.
    #[test]
    fn a_moved_building_takes_its_box_with_it() {
        let was = building([100.0, 50.0, 200.0]);
        let mut record = was;
        let world = vale_assets::world::adt::placement_to_world(was.position);
        set_world_position(
            &mut record,
            &was,
            [world[0] + 30.0, world[1] - 12.0, world[2] + 4.0],
        );

        // The box moved by exactly what the position did, on every axis.
        for axis in 0..3 {
            let step = record.position[axis] - was.position[axis];
            assert!(
                (record.bounds_lower[axis] - (was.bounds_lower[axis] + step)).abs() < 1e-3,
                "axis {axis}"
            );
            assert!(
                (record.bounds_upper[axis] - (was.bounds_upper[axis] + step)).abs() < 1e-3,
                "axis {axis}"
            );
        }
        // …and it is still the same size, which a re-fit from the wrong frame
        // would not be.
        for axis in 0..3 {
            let was_span = was.bounds_upper[axis] - was.bounds_lower[axis];
            let now_span = record.bounds_upper[axis] - record.bounds_lower[axis];
            assert!((was_span - now_span).abs() < 1e-3, "axis {axis}");
        }
    }

    /// A drag is measured from where it started, so the same pointer position
    /// always produces the same record however many frames it took to get there.
    #[test]
    fn a_drag_is_measured_from_where_it_began() {
        let was = building([100.0, 50.0, 200.0]);
        let world = vale_assets::world::adt::placement_to_world(was.position);
        let to = [world[0] + 30.0, world[1], world[2]];

        let mut once = was;
        set_world_position(&mut once, &was, to);
        // …and the same again from a record that has already been moved, still
        // measured against `was`.
        let mut twice = once;
        set_world_position(&mut twice, &was, to);
        assert_eq!(once, twice);
    }

    /// The four things a `MODF` edit can be, named apart so that a gesture folds
    /// only with its own kind.
    #[test]
    fn each_kind_of_change_has_its_own_subject() {
        let was = building([0.0; 3]);
        let mut turned = was;
        turned.rotation[1] = 90.0;
        assert_eq!(what_changed(&was, &turned).1, "rotation");
        let mut dressed = was;
        dressed.doodad_set = 2;
        assert_eq!(what_changed(&was, &dressed).1, "doodadset");
        let mut named = was;
        named.name_set = 1;
        assert_eq!(what_changed(&was, &named).1, "nameset");
        let mut moved = was;
        moved.position[0] = 5.0;
        assert_eq!(what_changed(&was, &moved).1, "position");
    }
}
