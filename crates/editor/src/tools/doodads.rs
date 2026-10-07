//! What is standing on the ground: select it, move it, turn it, scale it,
//! take it away.
//!
//! ## Picking is against what is drawn, not against the file
//!
//! An `MDDF` record is a path, a position, three angles and a scale. It says
//! nothing about how big the model is, so a ray marched against the file
//! alone can only hit a point — and a point is not clickable. The sizes are in
//! the M2s, which the renderer has already read and already placed: every batch
//! it spawned carries a [`Doodad`] naming the placement's unique id, a world
//! transform, and the `Aabb` Bevy computed from its mesh.
//!
//! So [`select`] rays against those. Two properties come out of that and
//! both are the ones wanted: a doodad too far away to be drawn cannot be
//! clicked, and what is picked is what the pointer is actually over rather than
//! what a sphere around an origin says. What it costs is a walk of the spawned
//! batches, which is a few thousand entities on a click and nothing at all on
//! the frames between.
//!
//! ## Why a box alone cannot decide which placement was hit
//!
//! A box is a broad phase and a bad answer on its own, because a batch's box is
//! the box of that mesh and some meshes are mostly air. A tree canopy is a few
//! crossed quads eighty yards across; its box encloses the trunk, the rock, the
//! fence and the cart. Testing the box alone made every click near one of these
//! select the canopy, including a click on something behind the camera's
//! shoulder that was not visible at all.
//!
//! So the box decides which placements to consider and the model's own drawn
//! triangles decide which of them was hit. `vale_assets::look::pick` is that
//! walk and it is the game's own: `ModelAssets::pick` is the same CPU copy of
//! the drawn triangles the client's targeting pick uses, kept per model path.
//! See [`nearest`].
//!
//! Two things about the narrow phase are deliberate:
//!
//! * A triangle hit anywhere beats every box. The nearest box is not always
//!   the nearest model, so every candidate is tested and the nearest triangle
//!   wins. Only when nothing is hit does the nearest box answer, which keeps a
//!   model whose mesh has not arrived selectable.
//! * A posed placement is tested in its bind pose. The batches of one carry
//!   the identity and the placement lives in the joints, so the transform is
//!   right; what is not is the sway. Six of a tile's seventy-five models have a
//!   skeleton at all and a branch moves by inches, which is well inside where a
//!   person is aiming.
//!
//! The unique id is then looked up in the open [`AdtFile`]'s own `MDDF` list to
//! get the index the edit is written at. The id and not the index is what
//! crosses that boundary, because it is the only thing both sides agree on: the
//! renderer never sees a list position, and the list position changes whenever
//! anything is added or removed.
//!
//! ## Moving it moves three things
//!
//! ```text
//! the file       Edit::Doodad, through the history, into the AdtFile
//! the residents  ResidentDoodads::reposition, so a re-spawn keeps it
//! the entities   the batch transforms, so the frame shows it
//! ```
//!
//! The middle one is easy to overlook: a placement walked out of range and
//! back returns to where the resident says it stands, not to where the
//! entities were last written. An editor that wrote only the entities would
//! show doodads snapping back into place after a walk. See
//! `vale_client::render::doodads::ResidentDoodads::reposition`.
//!
//! ## Removing one is a different kind of edit
//!
//! Moving a placement changes 36 bytes and nothing else. Removing one
//! renumbers every reference above it in all 256 chunks' `MCRF` — see
//! `vale_edit::adt::place`, which is where that is maintained — so it is
//! recorded as the placement lists whole and everything holding an index into
//! them has to let go. There is nothing left to reconcile entry by entry, so the
//! tile is read again instead.
//!
//! ## Everything a person sees is in the world's axes, and the file's are not
//!
//! `MDDF` stores a position measured from the map's corner in an axis order
//! nobody has in hand, and three angles whose order and signs are neither the
//! world's nor anything else:
//! `vale_assets::world::adt::placement_matrix` composes them as
//! `Rz(rot[1]) * Ry(-rot[0]) * Rx(-rot[2])`, so the second of the three is the
//! turn about the world's up.
//!
//! Both are converted at the edge and nowhere else. `placement_to_world` and
//! `placement_euler_to_world` are the two statements of it, both of them
//! `vale_assets`', and this tool and the panel beside it deal in x, y and z
//! meaning the world's x, y and z — the axes the status line, `.gps` and every
//! check in this project print. A second reading of either conversion is a
//! placement written a hundred yards from where it was dragged, or turned about
//! an axis other than the one the handle was drawn on.

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::Tool;
use vale_client::render::axes;
use vale_client::render::doodads::{Doodad, PosedDoodad, ResidentDoodads};
use vale_client::render::models::{Lookup, ModelCache};
use vale_client::render::terrain::TerrainTile;
use vale_client::world::camera::WorldCamera;
use vale_edit::adt::place::Doodad as Placement;
use vale_edit::ops::Edit;
use bevy::camera::primitives::Aabb;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

/// Which placement the tool is holding.
///
/// `at` is the primary: the one the inspector's numbers and the gizmo are
/// on. `also` is the rest of the group, in the order they were added. See
/// [`super::group`] for the rules and why the primary stays a single record.
#[derive(Resource, Debug, Default, Clone)]
pub struct Selection {
    pub at: Option<Selected>,
    /// The members beside the primary. Empty for a selection of one, which is
    /// every selection a plain click makes.
    pub also: Vec<Selected>,
    /// Whether a ctrl-drag also leans the placement onto the slope it is
    /// dropped on. Off, it drops the origin onto the ground and keeps the
    /// record's turns; on, the two lean angles are rewritten from the ground's
    /// normal under the origin so the model stands square to a hillside. The
    /// turn about up is kept either way — see
    /// `vale_assets::world::adt::lean_to_normal`.
    pub align: bool,
}

/// One selected placement, and everything a panel needs to say about it.
#[derive(Debug, Clone)]
pub struct Selected {
    /// Which tile's list it is in, and where in that list.
    pub tile: (u32, u32),
    pub index: usize,
    /// The id the renderer knows it by — see the module comment.
    pub unique_id: u32,
    /// Its `MMDX` path, for the panel to name it.
    pub path: String,
    /// The record as it stands. Written by the tool and read by the panel; the
    /// panel writes it back through [`Selection::edited`].
    pub record: Placement,
}

impl Selection {
    /// Select one placement, or none, and drop the rest of the group.
    pub fn only(&mut self, at: Option<Selected>) {
        self.at = at;
        self.also.clear();
    }

    /// How many placements are selected.
    pub fn count(&self) -> usize {
        usize::from(self.at.is_some()) + self.also.len()
    }

    /// Every selected placement, the primary first.
    pub fn members(&self) -> impl Iterator<Item = &Selected> {
        self.at.iter().chain(self.also.iter())
    }

    /// Whether a placement is selected, as the primary or as a member.
    pub fn holds(&self, unique_id: u32) -> bool {
        self.members().any(|at| at.unique_id == unique_id)
    }

    /// Shift+click: take a selected placement out of the group, or add one and
    /// make it the primary. Taking the primary out promotes the member added
    /// last.
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

    /// The placement a panel has just edited, as a change to record.
    ///
    /// `None` when nothing is selected or nothing moved, which is what a panel
    /// redrawn on a frame nobody touched it answers.
    pub fn edited(&self, was: &Placement) -> Option<(usize, Placement)> {
        let at = self.at.as_ref()?;
        (at.record != *was).then_some((at.index, at.record))
    }
}

/// Yards a nudge moves a placement, and degrees a turn turns it.
///
/// Both are round numbers rather than measured ones: what a person doing this
/// by keyboard wants is a step they can count, and the wheel is there for
/// anything finer. Shift is ten times each — see [`nudge`].
const STEP: f32 = 0.5;
const TURN: f32 = 5.0;

/// The smallest and largest a placement may be scaled to, in `MDDF`'s own
/// units, where 1024 is unit scale.
///
/// This one is the file's limit and not a choice. `MDDF`'s scale field is a
/// `u16`, so the range is `1..=65535` — about a thousandth to sixty-four times —
/// and 0 is excluded because it is a placement with no size rather than a small
/// one.
///
/// It used to be `128..=8192`, an eighth to eight times, taken from what the
/// shipped tiles happen to use (measured over `Azeroth_32_48`: 512 to 2048).
/// That range described the art in the shipped tiles, not a property of the
/// format, so the editor now accepts any value the field can hold.
const SCALE: std::ops::RangeInclusive<u16> = 1..=u16::MAX;

/// …and the same as a multiplier, which is what a person means and what the
/// panel edits.
pub const SCALE_RANGE: std::ops::RangeInclusive<f32> = (1.0 / 1024.0)..=(u16::MAX as f32 / 1024.0);

pub struct DoodadToolPlugin;

impl Plugin for DoodadToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Selection>()
            .init_resource::<Held>()
            .add_systems(
                Update,
                (
                    select, enclose, drag, nudge, remove, reconcile, publish, settle, resync,
                )
                    .chain()
                    // After the undo key, which is `super::shortcuts` and
                    // lives in the terrain tool's chain. Undo puts the file back
                    // without telling anything that is holding a copy of a
                    // record, and this is what puts the selection back in step —
                    // so running before it would leave the panel a frame behind
                    // every press of Ctrl+Z.
                    .after(super::shortcuts)
                    // After the pick, for the reason the brush is: the ray
                    // this rays against is built from the camera the pick has
                    // already placed, and a frame earlier is a frame of lag on
                    // a drag.
                    .after(crate::pick::aim),
            )
            .add_systems(Update, (draw_marker, draw_locked));
    }
}

/// Whether a drag is being held, and where the pointer was when it started.
#[derive(Resource, Default)]
pub(crate) struct Held {
    /// Whether the press that began this belonged to the world.
    ///
    /// A drag belongs to wherever it started. The gate here used to be asked on
    /// every frame, so a press on the inspector's own number fields was read as
    /// a drag in the viewport and teleported the selected placement to wherever
    /// the pointer's ray met the ground. Set by [`select`] on a press it
    /// accepted and cleared by one it declined, so it tracks only whether the
    /// press that opened this drag was in the world.
    ///
    /// This also gives the other half of the rule without extra code: a drag
    /// that begins in the world and wanders over a panel keeps going, which is
    /// what a person dragging something to the edge of the screen expects.
    armed: bool,
    dragging: bool,
    /// The ground under the pointer when the button went down, so a drag moves
    /// the placement by how far the pointer has travelled rather than to
    /// where it is. Without it a click on the edge of a tree jumps the tree's
    /// origin to that edge.
    from: Option<Vec3>,
    /// …and the record it started from, which is what the whole drag is
    /// measured against.
    was: Option<Placement>,
    /// The other members as they stood when the button went down, so each
    /// is moved by the drag's step from where it began, as the primary is.
    group_was: Vec<Selected>,
    /// Whether this press landed on a member of a group. Released without
    /// moving, it selects that member alone — see [`super::group`].
    collapse: bool,
    /// Whether a member other than the primary has been moved since it was
    /// last put in the tile its origin is in. See [`settle`].
    pub(crate) unsettled: bool,
}

impl Held {
    /// Say that the group's members have been moved from outside this tool's
    /// own systems, so [`settle`] puts each in the tile its origin is in.
    pub(crate) fn members_moved(&mut self) {
        self.unsettled = true;
    }
}

/// Pick a placement, or drop the one held.
///
/// `pub(crate)` for its ordering and nothing else: [`super::gizmo`] runs
/// before it, so a press that took a handle is not also a press that picks
/// whatever is behind the handle.
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
    drawn: Query<(&Doodad, &GlobalTransform, &Aabb)>,
    // The narrow phase's meshes — see [`nearest`]. `ResMut` because a
    // lookup that misses files a load; every path here is one the renderer has
    // already drawn, so it never does.
    mut models: ResMut<ModelCache>,
    cursor: Res<Cursor>,
    gizmo: Res<super::gizmo::Gizmo>,
    placing: Res<super::place::Placing>,
    // A tuple, because this system is at Bevy's limit of sixteen parameters.
    (keys, mut marquee, mut menu): (
        Res<ButtonInput<KeyCode>>,
        ResMut<super::group::Marquee>,
        ResMut<crate::context::ContextMenu>,
    ),
) {
    if *tool != Tool::Doodads || !state.editing() {
        return;
    }
    // A right click: what is under the pointer, locked or not, for the menu.
    // An unlocked placement outside the selection becomes the selection, so
    // the menu's actions apply to it. See `crate::context`.
    if menu.asked() {
        let Some(session) = session else { return };
        let hit = pointer_ray(&windows, &camera)
            .and_then(|(origin, direction)| nearest(&drawn, &session, &mut models, origin, direction));
        menu.object = hit.map(|unique_id| {
            let locked = session.placement_locked(super::place::Kind::Doodad, unique_id);
            if !locked && !selection.holds(unique_id) {
                if let Some(found) = find(&session, unique_id) {
                    selection.only(Some(found));
                }
            }
            crate::context::Target::Doodad { unique_id, locked }
        });
        return;
    }
    // A click that places must not also select. `place::commit` runs first
    // and has already taken it — see that module, where the ordering is stated.
    if placing.armed() {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    // A press the gizmo took is the gizmo's. It runs first — see the plugin,
    // where the ordering is stated — so by now it has decided. Without this,
    // grabbing an arrow also picked whatever was drawn behind it.
    if gizmo.holding() {
        held.armed = false;
        return;
    }
    // A press on the chrome is the chrome's, and it disarms the drag as well
    // as declining the pick — see [`Held::armed`]. Without clearing it, the
    // previous press's arming would stay set, and the next press in the world
    // would be read as a continued drag.
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
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
    // Physical pixels, for the reason [`crate::pick::aim`] gives beside its own
    // copy of this line.
    let Ok(ray) = camera.viewport_to_world(eye, at * window.scale_factor()) else {
        return;
    };

    // A locked placement is not picked: the click is treated as landing on
    // nothing. See `crate::session::PlacementLocks`.
    let hit = nearest(&drawn, &session, &mut models, ray.origin, *ray.direction)
        .filter(|unique_id| !session.placement_locked(super::place::Kind::Doodad, *unique_id))
        .and_then(|unique_id| find(&session, unique_id));
    let adding = super::group::shift(&keys);
    held.dragging = false;
    held.collapse = false;
    match hit {
        // Shift+click adds or takes away, and does not drag: the press is
        // about which things are selected, not where they are.
        Some(found) if adding => {
            selection.toggle(found);
            held.armed = false;
            return;
        }
        // A press on a member keeps the group, so the drag that follows moves
        // all of it. Released without moving, it becomes a selection of one.
        Some(found) if !selection.also.is_empty() && selection.holds(found.unique_id) => {
            selection.promote(found.unique_id);
            held.collapse = true;
        }
        Some(found) => selection.only(Some(found)),
        // Nothing under the pointer: a rectangle begins. Without shift the
        // selection is dropped now, which is also what a click on nothing is.
        None => {
            if !adding {
                selection.only(None);
            }
            marquee.begin(Tool::Doodads, at, adding);
            held.armed = false;
            return;
        }
    }
    held.armed = true;
    held.from = cursor.ground;
    held.was = selection.at.as_ref().map(|at| at.record);
    held.group_was = selection.also.clone();
}

/// The pointer's ray through the world camera, in Bevy's axes. Physical
/// pixels, for the reason `crate::pick::aim` gives beside its own copy.
pub(crate) fn pointer_ray(
    windows: &Query<&Window>,
    camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<(Vec3, Vec3)> {
    let window = windows.single().ok()?;
    let at = window.cursor_position()?;
    let (camera, eye) = camera.single().ok()?;
    let ray = camera.viewport_to_world(eye, at * window.scale_factor()).ok()?;
    Some((ray.origin, *ray.direction))
}

/// Select everything drawn whose origin is inside a finished rectangle.
///
/// Only drawn placements are considered, because only drawn ones can be
/// picked: a placement too far away to be on
/// screen cannot be clicked, so it cannot be enclosed either. The `Doodad`
/// markers say which ids are drawn; the file says where each one's origin is,
/// and the claim rule says which of a model's copies is the one drawn — see
/// [`find`].
#[allow(clippy::too_many_arguments)]
fn enclose(
    mut selection: ResMut<Selection>,
    mut marquee: ResMut<super::group::Marquee>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    drawn: Query<&Doodad>,
) {
    if *tool != Tool::Doodads {
        return;
    }
    let Some(done) = marquee.finished(Tool::Doodads) else {
        return;
    };
    let Some(session) = session else { return };
    let Some((camera, eye, window)) = super::group::view(&camera, &windows) else {
        return;
    };
    let shown: std::collections::HashSet<u32> = drawn.iter().map(|d| d.unique_id).collect();
    let mut found: Vec<(f32, Selected)> = Vec::new();
    for (coord, tile) in &session.tiles {
        let names = tile.model_names();
        for (index, record) in tile.doodad_list().iter().enumerate() {
            if !shown.contains(&record.unique_id)
                || session.placement_locked(super::place::Kind::Doodad, record.unique_id)
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
                        .map(|name| vale_assets::world::m2::model_path(name))
                        .unwrap_or_default(),
                    record: *record,
                },
            ));
        }
    }
    // The one nearest the middle of the rectangle is the primary, so the
    // handles stand in the middle of what was enclosed rather than at a corner.
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
}

/// The unique id of the placement the ray actually hits: every box it enters,
/// then those placements' own triangles.
///
/// The batches are walked whole rather than culled first. A tile draws a
/// few thousand of them and this runs on a press, not on a frame; a broad-phase
/// would be more code than the walk it replaces and would have to be kept in
/// step with a population that streams.
///
/// The narrow phase is one `hit_mesh` per placement and not per batch: the
/// pick mesh is the whole model's triangles, so testing it once per batch would
/// be the same work three or four times over. A model whose mesh has not
/// arrived, or which carries none, keeps its box — see the module comment.
fn nearest(
    drawn: &Query<(&Doodad, &GlobalTransform, &Aabb)>,
    session: &EditSession,
    models: &mut ModelCache,
    origin: Vec3,
    direction: Vec3,
) -> Option<u32> {
    let boxed = boxes_hit(
        drawn
            .iter()
            .map(|(doodad, at, aabb)| (doodad.unique_id, at, aabb)),
        origin,
        direction,
    );
    let fallback = boxed.first().map(|&(_, unique_id, _)| unique_id);

    let mut best: Option<(f32, u32)> = None;
    let mut tested: Vec<u32> = Vec::new();
    for (_, unique_id, at) in &boxed {
        if tested.contains(unique_id) {
            continue;
        }
        tested.push(*unique_id);
        let Some(path) = find(session, *unique_id).map(|found| found.path) else {
            continue;
        };
        let Lookup::Ready(model) = models.lookup(&path) else {
            continue;
        };
        if model.pick.is_empty() {
            continue;
        }
        let placed = at.affine();
        let hit = vale_assets::look::pick::hit_mesh(
            &model.pick,
            |vertex| {
                placed
                    .transform_point3(Vec3::from(axes::to_bevy(model.pick.positions[vertex])))
                    .to_array()
            },
            origin.to_array(),
            direction.to_array(),
        );
        if let Some(distance) = hit {
            if best.is_none_or(|(had, _)| distance < had) {
                best = Some((distance, *unique_id));
            }
        }
    }
    best.map(|(_, unique_id)| unique_id).or(fallback)
}

/// Every placement whose box the ray enters, nearest first.
///
/// The broad phase [`nearest`] narrows. It answers a transform beside each id
/// because the narrow phase needs one to put the model's own vertices in the
/// world, and the batch's is the placement's whichever kind of placement it is —
/// a still one carries it, and a posed one's batches are children of a rig root
/// that does.
///
/// `pub(crate)` because [`super::wmos`] picks buildings through the same walk.
pub(crate) fn boxes_hit<'a>(
    drawn: impl Iterator<Item = (u32, &'a GlobalTransform, &'a Aabb)>,
    origin: Vec3,
    direction: Vec3,
) -> Vec<(f32, u32, &'a GlobalTransform)> {
    let mut found: Vec<(f32, u32, &GlobalTransform)> = Vec::new();
    for (unique_id, at, aabb) in drawn {
        // The box is the mesh's, in the mesh's own space; the transform is the
        // placement's. Testing in model space costs one inverse and keeps the
        // box axis-aligned, which is what makes the slab test four lines rather
        // than a rotated-box intersection.
        let inverse = at.affine().inverse();
        let from = inverse.transform_point3(origin);
        let along = inverse.transform_vector3(direction);
        let Some(distance) = slab(
            from,
            along,
            Vec3::from(aabb.center),
            Vec3::from(aabb.half_extents),
        ) else {
            continue;
        };
        // …and back into world terms, so two placements at different scales are
        // compared on the same axis. A non-uniform scale would make this
        // approximate; `MDDF` carries one number, so it cannot be one.
        found.push((distance * at.scale().x.abs().max(1e-6), unique_id, at));
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found
}

/// Where a ray enters an axis-aligned box, or `None` if it misses.
///
/// `pub(crate)` for the same reason [`boxes_hit`] is.
///
/// The standard slab test. A ray that starts inside the box answers zero
/// rather than nothing, which is what a click from a camera parked inside a
/// tree should do.
pub(crate) fn slab(origin: Vec3, direction: Vec3, centre: Vec3, half: Vec3) -> Option<f32> {
    let (lower, upper) = (centre - half, centre + half);
    let mut near = f32::NEG_INFINITY;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        let d = direction[axis];
        if d.abs() < 1e-6 {
            // Parallel to this pair of planes: a miss unless it is already
            // between them.
            if origin[axis] < lower[axis] || origin[axis] > upper[axis] {
                return None;
            }
            continue;
        }
        let a = (lower[axis] - origin[axis]) / d;
        let b = (upper[axis] - origin[axis]) / d;
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (far >= near.max(0.0)).then(|| near.max(0.0))
}

/// Find the open tile and list position a unique id belongs to.
///
/// The copy the renderer is drawing, not the first one found. A model
/// touching two tiles is listed in both with one `unique_id` — a city is listed
/// in many — and `render::terrain` draws the copy whose origin is in the tile
/// that holds it. Taking the first match instead means editing a row nobody
/// draws: the thing on screen does not move, and the row that does move is one
/// the claim rule will never look at.
///
/// So the claim rule is asked here too, and it is the renderer's own line. A
/// placement that no tile claims — which is what a half-finished edit leaves —
/// falls back to the first match, because refusing to select it would leave no
/// way to put it right.
fn find(session: &EditSession, unique_id: u32) -> Option<Selected> {
    let mut fallback: Option<Selected> = None;
    for (coord, tile) in &session.tiles {
        let names = tile.model_names();
        for (index, record) in tile.doodad_list().iter().enumerate() {
            if record.unique_id != unique_id {
                continue;
            }
            let found = Selected {
                tile: *coord,
                index,
                unique_id,
                // Resolved, because `MMDX` is `.mdx` and no archive holds
                // one — `vale_assets::world::m2::model_path` is the one
                // statement of that swap, and everything downstream of this
                // opens the path rather than only printing it: the placer arms
                // it, and the pick asks the model cache for its triangles. See
                // [`super::place`], where what an unresolved one cost is.
                path: names
                    .get(record.name_id as usize)
                    .map(|name| vale_assets::world::m2::model_path(name))
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

/// Every placement of one model in the open tiles, each the copy the renderer
/// draws (see [`find`]'s claim rule). What the inspector's Select all of
/// this model selects.
pub(crate) fn all_of(session: &EditSession, path: &str) -> Vec<Selected> {
    let mut found = Vec::new();
    for (coord, tile) in &session.tiles {
        let names = tile.model_names();
        for (index, record) in tile.doodad_list().iter().enumerate() {
            let named = names
                .get(record.name_id as usize)
                .map(|name| vale_assets::world::m2::model_path(name))
                .unwrap_or_default();
            if !named.eq_ignore_ascii_case(path)
                || session.placement_locked(super::place::Kind::Doodad, record.unique_id)
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

/// Drag the held placement across the ground.
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
    if *tool != Tool::Doodads || !state.editing() {
        return;
    }
    // …and a drag the gizmo is holding is the gizmo's for the whole of it, not
    // only for the press: this one moves along the ground and that one along an
    // axis, and both running would do both.
    if gizmo.holding() {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    // The release is answered wherever the pointer is, and always: a drag
    // that began in the viewport and ended over a panel still has to end, or the
    // history is left open and the next change folds into it.
    if buttons.just_released(MouseButton::Left) {
        session.history.end();
        // A press on a member that never moved was a click on it: it becomes
        // the whole selection. See [`super::group`].
        if held.collapse && !held.dragging {
            let primary = selection.at.take();
            selection.only(primary);
        }
        held.dragging = false;
        held.armed = false;
        held.collapse = false;
        return;
    }
    // …and everything else needs a press that was the world's — see
    // [`Held::armed`], which is where the rule is.
    if !buttons.pressed(MouseButton::Left) || !held.armed {
        return;
    }
    let align = selection.align;
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
            0 => "Move doodad".to_string(),
            more => format!("Move {} doodads", more + 1),
        });
    }

    let drop = keys.pressed(KeyCode::ControlLeft);
    // The primary drops onto the ground under the pointer, which is the height
    // the pick already found; each other member onto the ground under itself.
    at.record = dragged(session, &was, step, drop.then_some(now.z), drop && align);
    write_record(session, at);
    let group = std::mem::take(&mut held.group_was);
    for member in &group {
        let Some(standing) = selection
            .also
            .iter_mut()
            .find(|at| at.unique_id == member.unique_id)
        else {
            continue;
        };
        let ground = drop
            .then(|| {
                let world = vale_assets::world::adt::placement_to_world(member.record.position);
                ground_height(session, world[0] + step.x, world[1] + step.y)
            })
            .flatten();
        standing.record = dragged(session, &member.record, step, ground, drop && align);
        write_record(session, standing);
    }
    if !group.is_empty() {
        held.unsettled = true;
    }
    held.group_was = group;
}

/// A record moved by a drag's step from where it began.
///
/// `MDDF` is not in world axes — see
/// `vale_assets::world::adt::placement_position`, which is the one statement
/// of the conversion. Moving the record therefore means going out to the
/// world, adding the step, and coming back.
///
/// `ground` is the height to drop it onto, which a ctrl-drag asks for: what a
/// placement wants nine times in ten and the one thing a drag across a hillside
/// cannot do on its own. With `lean` it is also leaned onto the slope there —
/// see [`Selection::align`]. The normal is the edited ground's, off the tile
/// the origin now stands on, which is not always the tile the record is in.
fn dragged(
    session: &EditSession,
    was: &Placement,
    step: Vec3,
    ground: Option<f32>,
    lean: bool,
) -> Placement {
    let mut record = *was;
    let world = vale_assets::world::adt::placement_to_world(was.position);
    let mut moved = Vec3::from(world) + step;
    if let Some(z) = ground {
        moved.z = z;
        if lean {
            if let Some(normal) = ground_normal(session, moved.x, moved.y) {
                let turns = vale_assets::world::adt::placement_euler_to_world(was.rotation);
                record.rotation = vale_assets::world::adt::placement_euler_from_world(
                    vale_assets::world::adt::lean_to_normal(normal, turns[2]),
                );
            }
        }
    }
    record.position = vale_assets::world::adt::placement_from_world([moved.x, moved.y, moved.z]);
    record
}

/// The edited ground's height under a world position, off whichever open tile
/// it is on.
pub(crate) fn ground_height(session: &EditSession, x: f32, y: f32) -> Option<f32> {
    let tile = session
        .tiles
        .get(&vale_assets::tile_for_position(x, y))?;
    vale_edit::adt::heights::height_at(tile, x, y)
}

/// The edited ground's slope under a world position, off whichever open tile
/// it is on.
pub(crate) fn ground_normal(session: &EditSession, x: f32, y: f32) -> Option<[f32; 3]> {
    let tile = session
        .tiles
        .get(&vale_assets::tile_for_position(x, y))?;
    vale_edit::adt::heights::normal_at(tile, x, y)
}

/// Lean the selected placement onto the ground under its origin, keeping
/// its turn about up. The panel's button, and what a ctrl-drag does with
/// [`Selection::align`] on. `false` when the origin is over no open ground.
pub(crate) fn lean_onto_ground(session: &EditSession, at: &mut Selected) -> bool {
    let world = vale_assets::world::adt::placement_to_world(at.record.position);
    let Some(normal) = ground_normal(session, world[0], world[1]) else {
        return false;
    };
    let turns = vale_assets::world::adt::placement_euler_to_world(at.record.rotation);
    at.record.rotation = vale_assets::world::adt::placement_euler_from_world(
        vale_assets::world::adt::lean_to_normal(normal, turns[2]),
    );
    true
}

/// …and the inverse: stand it upright, keeping its turn.
/// Align every selected doodad to the slope under its own origin, or stand
/// every one upright, as one undo entry: the doodad panel's two slope buttons
/// over the whole selection. Returns how many were written.
pub(crate) fn lean_selection(
    session: &mut EditSession,
    selection: &mut Selection,
    held: &mut Held,
    upright: bool,
) -> usize {
    let label = match upright {
        true => "Stand doodads upright",
        false => "Align doodads to slope",
    };
    session.history.begin(label);
    let mut written = 0;
    let members = selection.at.iter_mut().chain(selection.also.iter_mut());
    for member in members {
        match upright {
            true => stand_upright(member),
            false => {
                if !lean_onto_ground(session, member) {
                    continue;
                }
            }
        }
        write_record(session, member);
        session.publish(member.tile);
        written += 1;
    }
    held.members_moved();
    session.history.end();
    written
}

pub(crate) fn stand_upright(at: &mut Selected) {
    let turns = vale_assets::world::adt::placement_euler_to_world(at.record.rotation);
    at.record.rotation =
        vale_assets::world::adt::placement_euler_from_world([0.0, 0.0, turns[2]]);
}

/// Turn, scale, nudge and remove, from the keyboard.
///
/// The keys are the ones every editor of this kind uses and they are listed in
/// the panel rather than only here: the arrows nudge, `<` and `>` turn, the
/// wheel scales, `Delete` removes.
#[allow(clippy::too_many_arguments)]
fn nudge(
    mut selection: ResMut<Selection>,
    mut held: ResMut<Held>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    time: Res<Time>,
) {
    if *tool != Tool::Doodads || !state.editing() || wants.wants_keyboard_input() {
        return;
    }
    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);
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
        at.record.position =
            vale_assets::world::adt::placement_from_world([moved.x, moved.y, moved.z]);
    }

    // About the world's up, which is what a bare turn key means: the other
    // two axes are the gizmo's rings and the panel's other two rows, where
    // somebody asking for them has said so. `rot[1]` is the file's field for it
    // — see the module comment, and `placement_euler_to_world`, which is the one
    // statement of which field is which axis.
    let mut turn = 0.0;
    if keys.just_pressed(KeyCode::Comma) {
        turn -= TURN * far;
    }
    if keys.just_pressed(KeyCode::Period) {
        turn += TURN * far;
    }
    if turn != 0.0 {
        let mut world = vale_assets::world::adt::placement_euler_to_world(was.rotation);
        world[2] = wrap(world[2] + turn);
        at.record.rotation = vale_assets::world::adt::placement_euler_from_world(world);
    }

    // The wheel over a panel is the panel's — a scroll bar, or a drag value.
    let grow = match in_world && scroll.delta.y != 0.0 && keys.pressed(KeyCode::ControlLeft) {
        true => 1.0 + scroll.delta.y * 0.05,
        false => 1.0,
    };
    if grow != 1.0 {
        at.record.scale = scaled(at.record.scale, grow);
    }

    if at.record == was {
        return;
    }
    // One entry per run of the same key or wheel, not one per event. Turning
    // the wheel twenty notches is twenty of these in a third of a second, and
    // each used to be its own press of undo. `begin_gesture` continues the last
    // entry when it was about the same thing and recent — see
    // `vale_edit::undo::History::begin_gesture`, where the window and the
    // subject rule are.
    let (label, kind) = what_changed(&was, &at.record);
    let group = !selection.also.is_empty();
    let Some(at) = selection.at.as_ref() else {
        return;
    };
    let subject = match group {
        false => format!("doodad {} {kind}", at.unique_id),
        true => format!("doodad group {} {kind}", at.unique_id),
    };
    session
        .history
        .begin_gesture(label, subject, time.elapsed_secs_f64());
    write_record(session, at);

    // The rest of the group, by the same step, turn and size. A turn is
    // about the primary's origin, so the group turns as one piece rather than
    // each member on its own spot.
    let pivot = Vec3::from(vale_assets::world::adt::placement_to_world(was.position));
    let spin = super::group::turn_about(Vec3::Z, turn);
    for member in selection.also.iter_mut() {
        let world = Vec3::from(vale_assets::world::adt::placement_to_world(
            member.record.position,
        ));
        let moved = super::group::orbit(pivot, world, spin) + step * STEP * far;
        member.record.position = vale_assets::world::adt::placement_from_world(moved.to_array());
        if turn != 0.0 {
            member.record.rotation = super::group::turn_record(member.record.rotation, spin);
        }
        if grow != 1.0 {
            member.record.scale = scaled(member.record.scale, grow);
        }
        write_record(session, member);
    }
    if group {
        held.unsettled = true;
        session.status = format!("{label} · {} selected", selection.count());
    }
    session.history.end();
}

/// Carry the rest of the group by the change the primary has just had from
/// `was`: the same step, the same turn about the primary's old origin, and the
/// same factor of scale. Written into whatever history entry is open.
///
/// For the inspector's fields, which edit the primary's own numbers. The keys
/// and the gizmo carry the group by the step they made; this works the step out
/// from the two records, so a typed position moves the group as a drag would.
pub(crate) fn carry_members(session: &mut EditSession, selection: &mut Selection, was: &Placement) {
    let Some(now) = selection.at.as_ref().map(|at| at.record) else {
        return;
    };
    let pivot = Vec3::from(vale_assets::world::adt::placement_to_world(was.position));
    let step = Vec3::from(vale_assets::world::adt::placement_to_world(now.position)) - pivot;
    let turned = now.rotation != was.rotation;
    let turn = match turned {
        true => super::group::turn_between(was.rotation, now.rotation),
        false => Quat::IDENTITY,
    };
    let grow = f32::from(now.scale) / f32::from(was.scale.max(1));
    for member in selection.also.iter_mut() {
        let world = Vec3::from(vale_assets::world::adt::placement_to_world(member.record.position));
        let moved = super::group::orbit(pivot, world, turn) + step;
        member.record.position = vale_assets::world::adt::placement_from_world(moved.to_array());
        if turned {
            member.record.rotation = super::group::turn_record(member.record.rotation, turn);
        }
        if now.scale != was.scale {
            member.record.scale = scaled(member.record.scale, grow);
        }
        write_record(session, member);
    }
}

/// A scale in `MDDF`'s own units, multiplied and kept inside what the field
/// can hold.
fn scaled(scale: u16, by: f32) -> u16 {
    ((f32::from(scale) * by).round() as i32).clamp(i32::from(*SCALE.start()), i32::from(*SCALE.end()))
        as u16
}

/// What changed between two records: a name for the history, and the subject a
/// gesture folds by.
///
/// The subject is the kind and not just the placement. Nudging a tree and
/// then scaling it within half a second are two actions, and folding them
/// together would make one press of undo take back both — so the two produce
/// different subjects and never continue each other.
///
/// Position first when several changed at once, which is what a ctrl-drag onto
/// the ground does: it is the one a person would name.
pub(crate) fn what_changed(was: &Placement, now: &Placement) -> (&'static str, &'static str) {
    if was.position != now.position {
        return ("Move doodad", "position");
    }
    if was.rotation != now.rotation {
        return ("Turn doodad", "rotation");
    }
    if was.scale != now.scale {
        return ("Scale doodad", "scale");
    }
    ("Edit doodad", "record")
}

/// Degrees, brought back into `0..360`.
fn wrap(degrees: f32) -> f32 {
    degrees.rem_euclid(360.0)
}

/// Take the held placement out of the tile.
///
/// Its own system rather than an arm of [`nudge`], because it is the one
/// thing this tool does that is not a change to a record: removing an entry
/// renumbers every reference above it in all 256 chunks, so it is an
/// [`Edit::Placements`] — the lists whole — and everything holding an index into
/// them has to let go. That is the selection, and it is why this drops it.
///
/// The tile is marked stale rather than reconciled. There is no index that
/// survives the renumbering, so there is nothing to write a transform onto;
/// `super::terrain::remesh` reads the tile again, which is the fallback that
/// exists for exactly this.
fn remove(
    mut selection: ResMut<Selection>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut menu: ResMut<crate::context::ContextMenu>,
) {
    if *tool != Tool::Doodads || !state.editing() {
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
    // Every member, as one entry: see [`super::group::remove_rows`], which
    // takes a tile's rows from the top down so no index moves under another.
    let rows: Vec<((u32, u32), usize)> = std::iter::once(&at)
        .chain(selection.also.iter())
        .map(|member| (member.tile, member.index))
        .collect();
    let label = match rows.len() {
        1 => "Remove doodad".to_string(),
        more => format!("Remove {more} doodads"),
    };
    selection.also.clear();
    let removed = super::group::remove_rows(session, &rows, false, &label);
    session.status = match removed {
        1 => format!("removed {}", leaf(&at.path)),
        more => format!("removed {more} doodads"),
    };
}

/// The last part of an archive path, which is the only part that names
/// anything.
fn leaf(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Write one placement's new record into the tile and onto the history.
///
/// The one place a placement is written, and `pub(crate)` because
/// [`super::gizmo`] and [`crate::ui::inspector`] move the same records by the
/// same three steps and must not grow copies of them.
///
/// It deliberately takes no `before`: the change's `before` is read off the
/// tile here, so it is the state the file was actually in, not a caller's idea
/// of that state — see [`Edit::move_doodad`], which is where the argument is.
/// Four things in this crate hold a cached copy of a record, and each can be a
/// frame behind the file; a stack built from those copies would drift out of
/// step with the file.
pub(crate) fn write_record(session: &mut EditSession, at: &Selected) {
    let key = session.key(at.tile);
    let Some(tile) = session.tiles.get_mut(&at.tile) else {
        return;
    };
    let Some(edit) = Edit::move_doodad(tile, at.index, at.record) else {
        return;
    };
    edit.apply(tile);
    session.history.record(&key, [edit]);
    session.moved(at.tile, at.index);
}

/// Move the selected placement's record to the tile its origin has moved into.
///
/// The same fault the WMO tool had and the same fix — see [`super::rehome`]. It
/// is latent here rather than reported, because nothing re-reads a tile when a
/// doodad moves: the tree keeps drawing from the entity whose transform was
/// written, and the record in the wrong tile only costs anything the next time
/// that tile is read, at which point the tree is gone. A playtest is the usual
/// way to find out.
///
/// Only while no button is down, so that a drag across a border is one move
/// of the record and not one per frame.
///
/// The other members are settled once, after a change to the group, not on
/// every frame as the primary is. Settling walks every open tile's list, and
/// doing that for each of a hundred members on every frame is the cost a
/// selection should not have. [`Held::unsettled`] says a group change has
/// happened; [`resync`] then picks up the indices the moves renumbered.
fn settle(
    mut session: Option<ResMut<EditSession>>,
    mut selection: ResMut<Selection>,
    mut held: ResMut<Held>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    drawn: Query<(&Doodad, &GlobalTransform, &Aabb)>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if *tool != Tool::Doodads || !state.editing() || buttons.pressed(MouseButton::Left) {
        return;
    }
    let Some(at) = selection.at.as_ref() else {
        return;
    };
    let unique_id = at.unique_id;
    if let Some((coord, index)) =
        super::rehome::settle_doodad(session, unique_id, footprint(&drawn, unique_id))
    {
        if let Some(at) = selection.at.as_mut() {
            at.tile = coord;
            at.index = index;
            // …and the record, which `add_doodad` gave this tile's own `name_id`.
            if let Some(row) = session.tiles.get(&coord).and_then(|t| t.doodad_at(index)) {
                at.record = row;
            }
        }
    }
    if !std::mem::take(&mut held.unsettled) {
        return;
    }
    let members: Vec<u32> = selection.also.iter().map(|m| m.unique_id).collect();
    for unique_id in members {
        let Some((coord, index)) =
            super::rehome::settle_doodad(session, unique_id, footprint(&drawn, unique_id))
        else {
            continue;
        };
        let row = session.tiles.get(&coord).and_then(|t| t.doodad_at(index));
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

/// The model's own footprint, taken from what is drawn, because that is
/// what `add_doodad` wants and the file does not carry it: an `MDDF` row says
/// nothing about how big the model is. A placement that is not on screen falls
/// back to a stand-in, which over-references a few chunks rather than
/// under-referencing any — the direction that costs nothing, since this client
/// never reads `MCRF` and the reference client culls by it.
pub(crate) fn footprint(drawn: &Query<(&Doodad, &GlobalTransform, &Aabb)>, unique_id: u32) -> f32 {
    drawn
        .iter()
        .find(|(doodad, _, _)| doodad.unique_id == unique_id)
        .map(|(_, transform, aabb)| {
            let half = Vec3::from(aabb.half_extents) * transform.scale();
            half.x.abs().max(half.z.abs())
        })
        .unwrap_or(20.0)
}

/// Put the selection back in step with the file.
///
/// ## What goes stale, and why no single writer can fix it
///
/// [`Selected::record`] is a copy of an `MDDF` row, and three things change that
/// row without going through the panel holding it: an undo, a redo, and a tile
/// read again. An index goes stale too — removing a placement renumbers
/// every entry above it — so the identity that survives is the `unique_id` and
/// nothing else, which is the same thing the pick already relies on.
///
/// So rather than every writer remembering to refresh, the selection is compared
/// against the file once a frame and re-read when the two disagree. The common
/// case is one list lookup and one comparison; the full search only runs when
/// something really has moved underneath.
///
/// A placement that is no longer in any open tile drops the selection, which is
/// what an undone removal leaves behind.
///
/// The group's members are kept in step the same way, one comparison each, and
/// one that has gone leaves the group. A primary that has gone is replaced by
/// the member added last, so undoing a paste does not leave the handles on
/// nothing while the rest of the group is still selected.
fn resync(mut selection: ResMut<Selection>, session: Option<Res<EditSession>>) {
    let Some(session) = session else { return };
    let standing = |at: &Selected| {
        session
            .tiles
            .get(&at.tile)
            .and_then(|tile| tile.doodad_at(at.index))
            .is_some_and(|row| row.unique_id == at.unique_id && row == at.record)
    };
    // The cheap case, and it is nearly every frame: every row is where it was
    // and says what the panel says.
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

/// Put the drawn placements where the file now says they are.
///
/// Runs whenever anything is in [`EditSession::moved`], which is every frame of
/// a drag and once for an undo. Three things are written and the module comment
/// says why each: the resident, a posed placement's rig root, and a still one's
/// batches.
fn reconcile(
    mut session: Option<ResMut<EditSession>>,
    mut residents: Query<(&TerrainTile, &mut ResidentDoodads)>,
    mut batches: Query<(&Doodad, &ChildOf, &mut Transform), Without<PosedDoodad>>,
    mut posed: Query<&mut Transform, With<PosedDoodad>>,
    solids: Res<vale_client::world::session::Solids>,
    focus: Res<vale_client::render::focus::WorldFocus>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if session.moved.is_empty() {
        return;
    }
    // What each moved placement's id and transform now are, taken from the file
    // in one pass so the entity walk below is a comparison and not a lookup.
    let mut wanted: Vec<((u32, u32), u32, Transform)> = Vec::new();
    for (coord, indices) in std::mem::take(&mut session.moved) {
        let Some(tile) = session.tiles.get(&coord) else {
            continue;
        };
        let list = tile.doodad_list();
        for index in indices {
            let Some(record) = list.get(index) else {
                continue;
            };
            wanted.push((coord, record.unique_id, transform_of(record)));
        }
    }
    if wanted.is_empty() {
        return;
    }
    // A placement with no entity is not a request to drop, which is what
    // taking the set above used to be. It happens while a tile is part-way
    // through being replaced — the outgoing copy is gone and the incoming one has
    // not spawned — and the write that would have corrected it is lost, so the
    // placement stays wherever the in-flight read put it. Asking for the tile
    // again is the answer that converges: the file is already right, and
    // `remesh` starts a replacement once the one in flight has landed.
    for (coord, unique_id, _) in &wanted {
        if !batches
            .iter()
            .any(|(drawn, _, _)| drawn.unique_id == *unique_id)
        {
            session.stale.insert(*coord);
        }
    }
    for (_, mut resident) in &mut residents {
        for (_, unique_id, transform) in &wanted {
            resident.reposition(*unique_id, *transform);
        }
    }
    // Which placements' hulls have been moved, so a placement drawn as
    // several batches moves its hull once.
    let mut hulls_moved: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for (doodad, parent, mut transform) in &mut batches {
        let Some((_, _, wanted)) = wanted.iter().find(|(_, id, _)| *id == doodad.unique_id) else {
            continue;
        };
        // A posed placement's batches carry the identity: the placement is
        // in the rig root, because a skinned mesh's own transform is never
        // read. Writing the batch would move nothing and writing both would
        // move it twice.
        let was = match posed.get_mut(parent.parent()) {
            Ok(mut root) => {
                let was = root.to_matrix();
                *root = *wanted;
                was
            }
            Err(_) => {
                let was = transform.to_matrix();
                *transform = *wanted;
                was
            }
        };
        // The hull, by the delta between where the entity was and where the
        // file puts it, in the world's own axes. A placement just spawned
        // from the file has the identity delta and its hull is left alone.
        if hulls_moved.insert(doodad.unique_id) {
            let delta = wanted.to_matrix() * was.inverse();
            if !delta.abs_diff_eq(Mat4::IDENTITY, 1.0e-5) {
                let in_world = axes::from_bevy_affine(delta).to_cols_array();
                solids.0.move_placement(focus.map_id, doodad.unique_id, &in_world);
            }
        }
    }
}

/// One `MDDF` record as the transform the renderer places it with.
///
/// `placement_matrix` is the one statement of the rotation order and the axis
/// change; this only takes what it returns apart into the three fields a
/// `Transform` has, exactly as `Adt::placed_doodads` does one layer down.
fn transform_of(record: &Placement) -> Transform {
    let world = vale_assets::world::adt::placement_to_world(record.position);
    let matrix = vale_assets::world::adt::placement_matrix(
        world,
        record.rotation,
        f32::from(record.scale) / 1024.0,
    );
    // Through `to_bevy_affine` and not `from_cols_array` alone. The matrix
    // is composed in the file's own frame; `render::doodads` makes exactly this
    // call when it first places the thing, and a second reading of the basis
    // change here would move a doodad the moment it was selected.
    Transform::from_matrix(axes::to_bevy_affine(Mat4::from_cols_array(&matrix)))
}

/// Put the edited bytes where the archives will answer with them, once the drag
/// that made them is over.
///
/// The same argument [`super::terrain::publish`] makes: laying out a whole tile
/// is two megabytes of memcpy and nothing reads those bytes until something asks
/// the archives for the tile.
fn publish(mut session: Option<ResMut<EditSession>>, buttons: Res<ButtonInput<MouseButton>>) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let unsaved: Vec<(u32, u32)> = session.unsaved.iter().copied().collect();
    for coord in unsaved {
        session.publish(coord);
    }
}

/// A box around whatever is selected, so a click is visibly a selection.
///
/// The model's own box rather than a ring on the ground: what was picked is
/// a volume, and a marker that did not enclose it would leave "did I click the
/// tree or the rock behind it" unanswered.
fn draw_marker(
    // In front of the world — see [`super::gizmo::EditorHandles`]. A box
    // around a tree is around a thing with leaves in the way of it, and one
    // around a building is inside the building.
    mut gizmos: Gizmos<super::gizmo::EditorHandles>,
    selection: Res<Selection>,
    tool: Res<Tool>,
    drawn: Query<(&Doodad, &GlobalTransform, &Aabb)>,
) {
    if *tool != Tool::Doodads {
        return;
    }
    let Some(at) = selection.at.as_ref() else {
        return;
    };
    let colour = Color::srgb(1.0, 0.85, 0.3);
    // The rest of a group in a paler colour, so which one the handles and the
    // inspector's numbers are on is still visible.
    let member = Color::srgb(0.95, 0.75, 0.45).with_alpha(0.7);
    // A sorted list and a binary search, not a hash set. This is asked of
    // every drawn batch in the block — tens of thousands — on every frame
    // anything is selected, and this crate is built unoptimised in a debug
    // build, where hashing each id cost more than the rest of the frame's
    // selection work together.
    let others = super::group::sorted(selection.also.iter().map(|m| m.unique_id));
    for (doodad, transform, aabb) in &drawn {
        let colour = match doodad.unique_id == at.unique_id {
            true => colour,
            false if super::group::holds(&others, doodad.unique_id) => member,
            false => continue,
        };
        let centre = transform.affine().transform_point3(Vec3::from(aabb.center));
        let half = Vec3::from(aabb.half_extents) * transform.scale();
        gizmos.cube(
            Transform::from_translation(centre)
                .with_rotation(transform.rotation())
                .with_scale(half * 2.0),
            colour,
        );
    }
    // …and a stem to the ground under it, which is what says where it stands
    // when the box is in the air — a hanging lantern, a bird, a treetop.
    let world = vale_assets::world::adt::placement_to_world(at.record.position);
    let foot = axes::to_bevy(world);
    gizmos.line(foot, foot + Vec3::Y * 2.0, colour);
}

/// Box every locked doodad in red while the tool is chosen, so a click that
/// selects nothing is explained. See `crate::session::PlacementLocks`.
fn draw_locked(
    mut gizmos: Gizmos<super::gizmo::EditorHandles>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    drawn: Query<(&Doodad, &GlobalTransform, &Aabb)>,
) {
    if *tool != Tool::Doodads {
        return;
    }
    let Some(session) = session else { return };
    let locked = &session.placement_locks().doodads;
    if locked.is_empty() {
        return;
    }
    for (doodad, transform, aabb) in &drawn {
        if !locked.contains(&doodad.unique_id) {
            continue;
        }
        let centre = transform.affine().transform_point3(Vec3::from(aabb.center));
        let half = Vec3::from(aabb.half_extents) * transform.scale();
        gizmos.cube(
            Transform::from_translation(centre)
                .with_rotation(transform.rotation())
                .with_scale(half * 2.0),
            LOCKED,
        );
    }
}

/// The colour a locked placement is outlined in.
pub(crate) const LOCKED: Color = Color::srgba(1.0, 0.35, 0.3, 0.8);

#[cfg(test)]
mod tests {
    use super::*;

    /// A ray that misses answers nothing and one that starts inside answers
    /// zero. Both are the cases a click actually makes: the pointer is usually
    /// over empty ground, and the camera is sometimes inside the tree it is
    /// looking at.
    #[test]
    fn the_slab_test_answers_where_a_ray_enters() {
        let centre = Vec3::ZERO;
        let half = Vec3::splat(1.0);
        let hit = slab(Vec3::new(0.0, 0.0, -5.0), Vec3::Z, centre, half);
        assert!(hit.is_some_and(|d| (d - 4.0).abs() < 1e-4), "{hit:?}");
        assert_eq!(slab(Vec3::new(5.0, 5.0, -5.0), Vec3::Z, centre, half), None);
        assert_eq!(slab(Vec3::ZERO, Vec3::Z, centre, half), Some(0.0));
        // Parallel and outside is a miss; parallel and inside is not.
        assert_eq!(slab(Vec3::new(9.0, 0.0, 0.0), Vec3::Z, centre, half), None);
    }

    /// The nearer of two boxes on one ray is the one picked: clicking selects
    /// whatever is in front.
    #[test]
    fn the_nearer_box_is_the_one_the_ray_finds() {
        let near = slab(Vec3::new(0.0, 0.0, -10.0), Vec3::Z, Vec3::ZERO, Vec3::ONE);
        let far = slab(
            Vec3::new(0.0, 0.0, -10.0),
            Vec3::Z,
            Vec3::new(0.0, 0.0, 20.0),
            Vec3::ONE,
        );
        assert!(near < far);
    }

    /// The broad phase answers every box the ray enters, nearest first; the
    /// narrow phase depends on that order.
    ///
    /// A pick that stopped at the nearest box is the failure this two-phase
    /// arrangement exists to avoid: a tree canopy's box is eighty yards of
    /// mostly air, so it is nearest to everything standing under it, and the
    /// triangles that would say otherwise are never tested. Answering the whole
    /// list is what lets the model behind the empty box win.
    #[test]
    fn the_broad_phase_keeps_every_candidate_in_order() {
        let near = GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 0.0));
        let far = GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 20.0));
        let aside = GlobalTransform::from_translation(Vec3::new(50.0, 0.0, 0.0));
        let unit = Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0));

        let found = boxes_hit(
            // Deliberately not in ray order: the walk is over spawned entities
            // and nothing about the ECS promises one.
            [(2u32, &far, &unit), (1, &near, &unit), (3, &aside, &unit)].into_iter(),
            Vec3::new(0.0, 0.0, -10.0),
            Vec3::Z,
        );
        assert_eq!(
            found.iter().map(|&(_, id, _)| id).collect::<Vec<_>>(),
            vec![1, 2],
            "both boxes on the ray, nearest first, and the one beside it not at all"
        );
        assert!(found[0].0 < found[1].0);
    }

    /// Degrees stay in `0..360` however far a turn key is held, because the
    /// panel prints them and `-355` reads as a different angle from `5`.
    #[test]
    fn a_turn_wraps_rather_than_running_away() {
        assert!((wrap(365.0) - 5.0).abs() < 1e-4);
        assert!((wrap(-5.0) - 355.0).abs() < 1e-4);
        assert!((wrap(0.0)).abs() < 1e-4);
    }

    fn selected(unique_id: u32) -> Selected {
        Selected {
            tile: (32, 48),
            index: unique_id as usize,
            unique_id,
            path: "World\\Tree.m2".into(),
            record: Placement {
                name_id: 0,
                unique_id,
                position: [1.0, 2.0, 3.0],
                rotation: [0.0; 3],
                scale: 1024,
                flags: 0,
            },
        }
    }

    fn ids(selection: &Selection) -> Vec<u32> {
        selection.members().map(|at| at.unique_id).collect()
    }

    /// Shift+click adds a placement and makes it the primary; a second
    /// shift+click takes it out again and the one added before it leads. The
    /// primary is the one the handles and the inspector's numbers are on, so it
    /// has to be the one just clicked.
    #[test]
    fn a_shift_click_adds_and_takes_away() {
        let mut selection = Selection::default();
        selection.only(Some(selected(1)));
        selection.toggle(selected(2));
        selection.toggle(selected(3));
        assert_eq!(ids(&selection), vec![3, 1, 2]);
        assert_eq!(selection.count(), 3);

        // Taking a member out leaves the primary where it is.
        selection.toggle(selected(1));
        assert_eq!(ids(&selection), vec![3, 2]);
        // Taking the primary out promotes the member added last.
        selection.toggle(selected(3));
        assert_eq!(ids(&selection), vec![2]);
        selection.toggle(selected(2));
        assert_eq!(selection.count(), 0);
    }

    /// A press on a member makes it the primary and keeps the group, which
    /// is what lets a drag start from any member and move all of them.
    #[test]
    fn a_member_promoted_keeps_the_group() {
        let mut selection = Selection::default();
        selection.only(Some(selected(1)));
        selection.toggle(selected(2));
        selection.toggle(selected(3));
        selection.promote(1);
        assert_eq!(selection.at.as_ref().map(|at| at.unique_id), Some(1));
        let mut all = ids(&selection);
        all.sort_unstable();
        assert_eq!(all, vec![1, 2, 3]);
        // Promoting the primary, or something not selected, changes nothing.
        selection.promote(1);
        selection.promote(9);
        assert_eq!(selection.count(), 3);
        assert!(selection.holds(2) && !selection.holds(9));
        // And `only` is what a plain click does: one, and the rest dropped.
        selection.only(Some(selected(2)));
        assert_eq!(ids(&selection), vec![2]);
    }

    /// A selection is only edited when something actually changed. The panel
    /// redraws every frame and writes its fields back every frame; without this
    /// the history would take an entry a frame for as long as the panel is open.
    #[test]
    fn an_untouched_selection_is_not_an_edit() {
        let record = Placement {
            name_id: 0,
            unique_id: 7,
            position: [1.0, 2.0, 3.0],
            rotation: [0.0, 90.0, 0.0],
            scale: 1024,
            flags: 0,
        };
        let mut selection = Selection {
            align: false,
            also: Vec::new(),
            at: Some(Selected {
                tile: (32, 48),
                index: 3,
                unique_id: 7,
                path: "World\\Tree.m2".into(),
                record,
            }),
        };
        assert_eq!(selection.edited(&record), None);
        if let Some(at) = selection.at.as_mut() {
            at.record.scale = 2048;
        }
        assert!(selection.edited(&record).is_some());
    }
}
