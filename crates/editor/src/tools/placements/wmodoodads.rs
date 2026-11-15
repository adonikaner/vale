//! The building editor's Doodads mode: the building's own doodads (its
//! `MODD` spawns) and the sets they are in, edited in the building's files.
//! The editor is [`super::wmoview`].
//!
//! ## What is edited
//!
//! A building carries its furniture in its root file: sets of spawns, each
//! spawn a model, a place, a turn and a scale in the building's own axes, and
//! each group lists the spawns that stand in it. A placement draws the one
//! set its `MODF` record names. The editor draws one set, chosen in the
//! panel, and a spawn added goes into it. The mode moves, turns, scales, adds
//! and removes spawns, and adds and renames sets. The file layer is
//! `vale_edit::wmo`, and an edit is one undo entry
//! (`crate::session::buildings`).
//!
//! ## An edit reaches every placement of the file
//!
//! An edit changes the building's file, and every placement of that file on
//! every map draws it. A building the game ships is often placed many times
//! (one inn model is placed as a dozen inns), so before the first edit the panel offers
//! Copy for This Placement ([`copy_for_placement`]): the root and its groups
//! copied into the project under a new name, and this placement pointed at
//! the copy. Editing the shared file is the other choice, for a building
//! placed once or a change meant for all of them. A building the project
//! holds already is edited without asking.
//!
//! ## Viewport controls
//!
//! The controls are the doodad tool's (`super::doodads`), on the building's
//! spawns:
//!
//! ```text
//! click a spawn             choose it; the spawn under the pointer is boxed
//! drag a spawn              move it across the level it stands at
//! Ctrl + drag               stand it on the surface under the pointer
//! the handles               move along an axis or turn about one
//!                           (`super::gizmo`, which reads and writes the spawn
//!                           through [`gizmo_held`] and [`gizmo_put`])
//! Alt + mouse               turn about up
//! arrows, Page Up/Down      nudge half a yard; Shift ten times
//! , and .                   turn five degrees; Shift ten times
//! Ctrl + wheel              scale
//! Delete                    remove
//! Ctrl + D                  put a copy on the cursor
//! a model on the cursor     chosen in the panel's Add list; a click adds it
//!                           where it lands, into the drawn set, and it stays
//!                           on the cursor; `,` `.` and Ctrl + wheel turn and
//!                           scale it
//! Escape                    put the model down, let go of the spawn, then
//!                           leave the editor
//! ```
//!
//! The pick is the doodad tool's narrow phase: the spawn whose own triangles
//! the pointer's ray meets first (`vale_assets::look::pick::hit_mesh`). The
//! renderer gives every spawn of a building the building's id, so the pick
//! is against the file's spawns and the model cache's triangles, not against
//! the drawn entities.
//!
//! ## Spawn rotations and `MDDF` angles
//!
//! A spawn's turn is a quaternion; a tile doodad's is `MDDF`'s three angles,
//! which the gizmo turns. `placement_matrix` builds `Rz * Ry * Rx * Rz(180°)`
//! from the angles, so [`angles_of`] takes the 180° off the quaternion and
//! reads the three turns, and [`rotation_of`] puts them back: the matrix the
//! angles give is the spawn's own.
//!
//! A spawn moved into another group's box is taken out of the lists of the
//! groups that held it and put in that group's, which reads every group of
//! the building once.

use std::collections::HashMap;

use bevy::prelude::*;

use super::place::{Placing, Switch};
use crate::marks::{Look, Marks};
use crate::session::EditSession;
use crate::tools::Tool;
use vale_client::assets::GameAssets;
use vale_client::render::axes;
use vale_client::render::models::{Lookup, Materials, ModelCache};
use vale_client::render::wmos::{Interior, WmoPlacement};
use vale_client::world::camera::WorldCamera;
use vale_edit::wmo::{insert_doodad, remove_doodad, Building, Doodad, DoodadSet};

/// How far from a spawn's origin the point the pointer meets may be and still
/// choose it, in yards.
pub(crate) const REACH: f32 = 6.0;

/// The `MODF` id of a WMO-only map's building. See `super::wmoareas`.
const GLOBAL_BUILDING: u32 = u32::MAX;

/// The building the mode is on: a placement of a root file.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Placed {
    pub(crate) unique_id: u32,
    pub(crate) path: String,
    /// The building's own axes to the world's, and back.
    pub(crate) to_world: Mat4,
    pub(crate) to_local: Mat4,
    /// The set this placement draws.
    pub(crate) set: u16,
}

impl Placed {
    pub(crate) fn leaf(&self) -> &str {
        self.path.rsplit(['\\', '/']).next().unwrap_or(&self.path)
    }

    /// Whether a set is drawn by this placement.
    pub(crate) fn draws(&self, set: usize) -> bool {
        set == usize::from(self.set)
    }
}

/// The mode's state.
#[derive(Resource)]
pub(crate) struct Spawns {
    pub(crate) building: Option<Placed>,
    /// Whether the mode is on: the building editor is open in Doodads mode.
    pub(crate) active: bool,
    /// The chosen spawn, by `MODD` index.
    pub(crate) chosen: Option<u16>,
    /// The model on the cursor, chosen in the asset drawer, with its turn
    /// about up in degrees and its scale.
    pub(crate) adding: Option<String>,
    pub(crate) turn: f32,
    pub(crate) scale: f32,
    /// Buildings the person chose to edit in place, by their roots' keys.
    pub(crate) shared: std::collections::HashSet<String>,
    hovered: Option<u16>,
    drag: Option<Drag>,
    /// The model on the cursor as drawn, and its path.
    ghost: Option<(Entity, String)>,
    /// Each model's pick shape, by path; `None` for one with no triangles.
    shapes: HashMap<String, Option<Shape>>,
}

impl Default for Spawns {
    fn default() -> Spawns {
        Spawns {
            building: None,
            active: false,
            chosen: None,
            adding: None,
            turn: 0.0,
            scale: 1.0,
            shared: Default::default(),
            hovered: None,
            drag: None,
            ghost: None,
            shapes: HashMap::new(),
        }
    }
}

impl Spawns {
    /// Whether the chosen building may be edited: the project holds its file,
    /// or the person chose to edit the shared file. See the module comment.
    pub(crate) fn editable(&self, session: &EditSession) -> bool {
        self.building.as_ref().is_some_and(|placed| {
            session.project.has(&placed.path) || self.shared.contains(&crate::session::buildings::building_key(&placed.path))
        })
    }
}

/// The building a placed WMO is, with the set it draws.
fn placed_of(placement: &WmoPlacement, interior: &Interior) -> Placed {
    Placed {
        unique_id: placement.unique_id,
        path: placement.path.clone(),
        to_world: interior.inverse.inverse(),
        to_local: interior.inverse,
        set: placement.doodad_set,
    }
}

/// A spawn's origin in the world.
pub(crate) fn world_of(placed: &Placed, doodad: &Doodad) -> Vec3 {
    placed.to_world.transform_point3(Vec3::from(doodad.position))
}

/// The drawn spawn nearest a world point, within [`REACH`].
fn spawn_near(building: &Building, placed: &Placed, at: Vec3) -> Option<u16> {
    let sets = building.root.doodad_sets();
    let local = placed.to_local.transform_point3(at);
    let doodads = building.root.doodads();
    sets.iter()
        .enumerate()
        .filter(|(set, _)| placed.draws(*set))
        .flat_map(|(_, run)| run.start..run.start + run.count)
        .filter_map(|index| {
            let doodad = doodads.get(index as usize)?;
            let distance = Vec3::from(doodad.position).distance(local);
            (distance <= REACH).then_some((index as u16, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// Put a spawn in the list of the group whose box holds it, and out of the
/// others' lists. Needs every group read.
fn list_where_it_stands(building: &mut Building, index: u16) {
    let Some(doodad) = building.root.doodads().get(index as usize).cloned() else { return };
    let Some(home) = building.group_holding(doodad.position) else { return };
    for (at, group) in building.groups.iter_mut().enumerate() {
        let Some(group) = group.as_mut() else { continue };
        let mut refs = group.doodad_refs();
        let listed = refs.contains(&index);
        match (at == home, listed) {
            (true, false) => {
                refs.push(index);
                refs.sort_unstable();
            }
            (false, true) => refs.retain(|&had| had != index),
            _ => continue,
        }
        let _ = group.set_doodad_refs(&refs);
    }
}

/// Whether moving a spawn to `local` takes it out of the group whose box
/// held it.
fn changes_group(building: &Building, index: u16, local: [f32; 3]) -> bool {
    let Some(doodad) = building.root.doodads().get(index as usize).cloned() else { return false };
    building.group_holding(doodad.position) != building.group_holding(local)
}

/// Move, turn or scale one spawn: `change` is given the spawn. A move into
/// another group's box lists it there. Answers why it was not made.
pub(crate) fn change_spawn(
    session: &mut EditSession,
    assets: &GameAssets,
    path: &str,
    index: u16,
    label: &str,
    now: f64,
    change: impl FnOnce(&mut Doodad),
) -> Result<(), String> {
    session.open_building(assets, path)?;
    let Some(open) = session.building(path) else { return Ok(()) };
    let mut doodad = open.building.root.doodads().get(index as usize).cloned().ok_or("no such spawn")?;
    change(&mut doodad);
    let regroup = changes_group(&open.building, index, doodad.position);
    let subject = format!("building {path} spawn {index}");
    session.edit_building(assets, path, label, &subject, now, regroup, |building| {
        let sets = building.root.doodad_sets();
        let mut doodads = building.root.doodads();
        doodads[index as usize] = doodad;
        building.root.set_doodads(&sets, &doodads).map_err(|e| e.to_string())?;
        if regroup {
            list_where_it_stands(building, index);
        }
        Ok(())
    })
}

/// Add a spawn to set `set`, listed in the group whose box holds it. Answers
/// its index.
pub(crate) fn add_spawn(
    session: &mut EditSession,
    assets: &GameAssets,
    path: &str,
    set: usize,
    doodad: Doodad,
    now: f64,
) -> Result<u16, String> {
    let mut made = None;
    let subject = format!("building {path} add");
    session.edit_building(assets, path, "Add building doodad", &subject, now, true, |building| {
        let mut sets = building.root.doodad_sets();
        let mut doodads = building.root.doodads();
        let (at, moved) = insert_doodad(&mut sets, &mut doodads, set, doodad).map_err(|e| e.to_string())?;
        building.root.set_doodads(&sets, &doodads).map_err(|e| e.to_string())?;
        for group in building.groups.iter_mut().flatten() {
            group.renumber(moved).map_err(|e| e.to_string())?;
        }
        list_where_it_stands(building, at);
        made = Some(at);
        Ok(())
    })?;
    made.ok_or_else(|| "nothing was added".to_string())
}

/// Remove a spawn from the building and from every group's list.
pub(crate) fn remove_spawn(session: &mut EditSession, assets: &GameAssets, path: &str, index: u16, now: f64) -> Result<(), String> {
    let subject = format!("building {path} remove");
    session.edit_building(assets, path, "Remove building doodad", &subject, now, true, |building| {
        let mut sets = building.root.doodad_sets();
        let mut doodads = building.root.doodads();
        let moved = remove_doodad(&mut sets, &mut doodads, index).map_err(|e| e.to_string())?;
        building.root.set_doodads(&sets, &doodads).map_err(|e| e.to_string())?;
        for group in building.groups.iter_mut().flatten() {
            group.renumber(moved).map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

/// Add an empty set at the end, or rename set `rename`.
pub(crate) fn name_set(
    session: &mut EditSession,
    assets: &GameAssets,
    path: &str,
    rename: Option<usize>,
    name: &str,
    now: f64,
) -> Result<(), String> {
    let label = match rename {
        Some(_) => "Rename doodad set",
        None => "Add doodad set",
    };
    let subject = format!("building {path} sets");
    session.edit_building(assets, path, label, &subject, now, false, |building| {
        let mut sets = building.root.doodad_sets();
        let doodads = building.root.doodads();
        match rename.and_then(|at| sets.get_mut(at)) {
            Some(set) => set.name = name.to_string(),
            None => sets.push(DoodadSet { name: name.to_string(), start: doodads.len() as u32, count: 0, pad: 0 }),
        }
        building.root.set_doodads(&sets, &doodads).map_err(|e| e.to_string())
    })
}

/// A new name for a copy of `path` that neither the archives nor the project
/// has: `Name_copy1.wmo`, `Name_copy2.wmo` and so on.
fn copy_path(session: &EditSession, assets: &GameAssets, path: &str) -> String {
    let stem = match path.len().checked_sub(4) {
        Some(cut) if path[cut..].eq_ignore_ascii_case(".wmo") => &path[..cut],
        _ => path,
    };
    (1..)
        .map(|n| format!("{stem}_copy{n}.wmo"))
        .find(|candidate| {
            !session.project.has(candidate) && assets.with_archive(|chain| Ok(chain.read(candidate).ok())).ok().flatten().is_none()
        })
        .expect("an unused name")
}

/// Copy the building's root and groups into the project under a new name,
/// as this editor holds them, and point placement `from` at the copy: the
/// `MODF` record of `tile`, or a WMO-only map's WDT. Answers the copy's path.
///
/// The tile is opened first: the building editor stands off the map's grid,
/// and the tiles near the placement close while the camera is there. Nothing
/// is written when the placement is not found.
pub(crate) fn copy_for_placement(
    session: &mut EditSession,
    assets: &GameAssets,
    building: &str,
    from: u32,
    tile: Option<(u32, u32)>,
) -> Result<String, String> {
    if from != GLOBAL_BUILDING {
        if let Some(coord) = tile {
            session.open(assets, coord);
        }
        if super::wmos::find(session, from).is_none() {
            return Err(format!("WMO {from} is in no open tile, so no copy was made"));
        }
    }
    session.open_building(assets, building)?;
    session.read_building_groups(assets, building)?;
    let open = session.building(building).ok_or("the building is not open")?.clone();
    let path = copy_path(session, assets, building);
    session.project.write(&path, &open.building.root.write()).map_err(|e| format!("{path}: {e}"))?;
    for (at, group) in open.building.groups.iter().enumerate() {
        let Some(group) = group else { continue };
        let target = vale_edit::wmo::group_path(&path, at);
        session.project.write(&target, &group.write()).map_err(|e| format!("{target}: {e}"))?;
    }
    session.files_revision += 1;
    match from {
        GLOBAL_BUILDING if session.tiles.is_empty() => {
            crate::map::tiles::set_layout(session, assets, Some(&path))?;
            session.republished.push(vale_edit::wdt::wdt_path(&session.map).to_ascii_lowercase());
        }
        id => {
            let bounds = crate::map::tiles::wmo_bounds(assets, &path)?;
            let sets = open.building.root.doodad_sets().len();
            if super::swap::replace_buildings(session, &[id], &path, bounds, sets) == 0 {
                return Err("no open tile places this building, so it was not pointed at the copy".to_string());
            }
        }
    }
    Ok(path)
}

/// Take the building the editor draws, when it is drawn, and again when the
/// editor shows another file or set.
fn follow_the_mode(
    view: Res<super::wmoview::WmoView>,
    placing: Res<Placing>,
    mut spawns: ResMut<Spawns>,
    placed: Query<(&WmoPlacement, &Interior)>,
) {
    spawns.active = view.open.is_some() && placing.switch() == Switch::Doodads;
    if view.open.is_none() {
        spawns.building = None;
        spawns.chosen = None;
        spawns.hovered = None;
        spawns.adding = None;
        return;
    }
    // The building is followed in every mode of the editor, because Lights
    // mode (`super::wmolights`) uses the same building and the same choice
    // between a copy and the shared file.
    let Some((placement, interior)) = placed.iter().find(|(placement, _)| placement.unique_id == super::wmoview::VIEW_ID) else {
        return;
    };
    let drawn = placed_of(placement, interior);
    if spawns.building.as_ref() != Some(&drawn) {
        if spawns.building.as_ref().is_some_and(|had| had.path != drawn.path) {
            spawns.chosen = None;
        }
        spawns.building = Some(drawn);
    }
}

/// `--wmo-doodads [<index>]` and `--wmo-copy`: open the building
/// editor in Doodads mode on a WMO-only map's building once the WMO tool is
/// chosen, then copy the building and choose the doodad once it is drawn,
/// with the camera on it.
/// See `crate::args::tools::Tools::wmo_doodads`.
#[allow(clippy::too_many_arguments)]
fn scripted(
    args: Res<crate::args::Args>,
    tool: Res<Tool>,
    assets: Res<GameAssets>,
    mut view: ResMut<super::wmoview::WmoView>,
    mut spawns: ResMut<Spawns>,
    session: Option<ResMut<EditSession>>,
    mut fly: MessageWriter<crate::camera::FlyTo>,
    mut done: Local<u8>,
) {
    let Some(index) = args.tools.wmo_doodads else { return };
    match *done {
        0 if *tool == Tool::Wmos && view.global.is_some() => {
            view.tab = Switch::Doodads;
            view.asked = Some(GLOBAL_BUILDING);
            *done = 1;
        }
        1 => {
            let (Some(building), Some(mut session)) = (spawns.building.clone(), session) else { return };
            if args.tools.wmo_copy && !spawns.editable(&session) {
                let (from, tile) = view.open.as_ref().map_or((GLOBAL_BUILDING, None), |open| (open.from, open.tile));
                match copy_for_placement(&mut session, &assets, &building.path, from, tile) {
                    Ok(path) => {
                        info!("--wmo-copy: copied {} to {path}", building.path);
                        if let Some(open) = view.open.as_mut() {
                            open.path = path;
                        }
                    }
                    Err(why) => warn!("--wmo-copy: {why}"),
                }
            }
            *done = 2;
        }
        // Runs once the drawn building is the file the editor shows. After a
        // copy, that is a few frames later.
        2 => {
            let path = view.open.as_ref().map(|open| open.path.clone());
            let (Some(path), Some(mut session)) = (path, session) else { return };
            if spawns.building.as_ref().is_none_or(|building| building.path != path) {
                return;
            }
            spawns.chosen = index;
            // Fly the camera to the chosen doodad, so a screenshot shows it.
            if let Some(at) = chosen_place(&spawns, Some(&session)) {
                fly.write(crate::camera::FlyTo { at, distance: Some(14.0) });
            }
            if session.open_building(&assets, &path).is_ok() {
                if let Some(open) = session.building(&path) {
                    info!(
                        "--wmo-doodads: {path}, {} sets, {} doodads, chosen {index:?}",
                        open.building.root.doodad_sets().len(),
                        open.building.root.doodads().len()
                    );
                }
            }
            *done = 3;
        }
        _ => {}
    }
}

/// A building spawn's rotation as `MDDF`'s three angles, the form the gizmo
/// and the doodad tool turn: the matrix `placement_matrix` builds from the
/// angles is the spawn's own. See the module comment.
pub(crate) fn angles_of(rotation: [f32; 4]) -> [f32; 3] {
    let flipped = Quat::from_array(rotation).normalize() * Quat::from_rotation_z(std::f32::consts::PI);
    let (z, y, x) = flipped.to_euler(EulerRot::ZYX);
    vale_assets::world::adt::placement_euler_from_world([x.to_degrees(), y.to_degrees(), z.to_degrees()])
}

/// The inverse of [`angles_of`]: `MDDF`'s three angles as a spawn's rotation.
pub(crate) fn rotation_of(angles: [f32; 3]) -> [f32; 4] {
    let [x, y, z] = vale_assets::world::adt::placement_euler_to_world(angles).map(f32::to_radians);
    (Quat::from_euler(EulerRot::ZYX, z, y, x) * Quat::from_rotation_z(std::f32::consts::PI)).normalize().to_array()
}

/// A spawn's turn about the building's up, in degrees: the turn the doodad
/// tool's `,` and `.` change and the model on the cursor is given.
fn turn_of(rotation: [f32; 4]) -> f32 {
    vale_assets::world::adt::placement_euler_to_world(angles_of(rotation))[2]
}

/// The matrix from a spawn's model axes to the world.
fn matrix_of(placed: &Placed, doodad: &Doodad) -> Mat4 {
    placed.to_world * Mat4::from_cols_array(&vale_assets::world::wmo::doodad_matrix(doodad.position, doodad.rotation, doodad.scale))
}

/// Where the chosen spawn stands in the world, for F.
pub(crate) fn chosen_place(spawns: &Spawns, session: Option<&EditSession>) -> Option<Vec3> {
    let (placed, index) = (spawns.building.as_ref()?, spawns.chosen?);
    let open = session?.building(&placed.path)?;
    let doodad = open.building.root.doodads().get(usize::from(index))?.clone();
    Some(world_of(placed, &doodad))
}

/// What the gizmo stands on in Doodads mode: the chosen spawn's place in the
/// world and its rotation as `MDDF`'s angles, while the building may be
/// edited. See `super::gizmo`.
pub(crate) fn gizmo_held(spawns: &Spawns, session: Option<&EditSession>) -> Option<(Vec3, [f32; 3])> {
    let session = session?;
    if !spawns.active || spawns.adding.is_some() || !spawns.editable(session) {
        return None;
    }
    let (placed, index) = (spawns.building.as_ref()?, spawns.chosen?);
    let open = session.building(&placed.path)?;
    let doodad = open.building.root.doodads().get(usize::from(index))?.clone();
    Some((world_of(placed, &doodad), angles_of(doodad.rotation)))
}

/// The write-back for [`gizmo_held`]: a handle or the `Alt` turn moves the
/// spawn to a world point and turns it to `MDDF`'s angles. One gesture is one
/// undo entry.
pub(crate) fn gizmo_put(spawns: &Spawns, session: &mut EditSession, assets: &GameAssets, position: Vec3, angles: [f32; 3], now: f64) {
    let (Some(placed), Some(index)) = (spawns.building.as_ref(), spawns.chosen) else { return };
    let local = placed.to_local.transform_point3(position).to_array();
    let rotation = rotation_of(angles);
    if let Err(why) = change_spawn(session, assets, &placed.path, index, "Move building doodad", now, |doodad| {
        doodad.position = local;
        doodad.rotation = rotation;
    }) {
        session.status = why;
    }
}

/// The pointer's ray, in the world's own axes, when the pointer is over the
/// world.
fn ray(
    windows: &Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<(Vec3, Vec3)> {
    let (origin, direction) = super::doodads::pointer_ray(windows, camera)?;
    let origin = Vec3::from(axes::to_wow(origin));
    let direction = Vec3::from(axes::to_wow(direction)).normalize_or_zero();
    (direction != Vec3::ZERO).then_some((origin, direction))
}

/// A model's pick mesh, and the radius about its origin that holds it, read
/// once per path.
fn shape<'a>(shapes: &'a mut HashMap<String, Option<Shape>>, models: &mut ModelCache, path: &str) -> Option<&'a Shape> {
    if !shapes.contains_key(path) {
        let Lookup::Ready(model) = models.lookup(path) else { return None };
        let positions = &model.pick.positions;
        let found = (!positions.is_empty()).then(|| {
            let radius = positions.iter().map(|&at| Vec3::from(at).length()).fold(0.0, f32::max);
            Shape { pick: model.pick.clone(), radius }
        });
        shapes.insert(path.to_string(), found);
    }
    shapes.get(path).and_then(Option::as_ref)
}

/// A model's triangles for the pick, and how far the furthest is from the
/// origin, in the model's own axes.
#[derive(Clone)]
pub(crate) struct Shape {
    pick: std::sync::Arc<vale_assets::look::pick::PickMesh>,
    radius: f32,
}

/// The drawn spawn whose triangles the ray meets first. A spawn whose model
/// has no triangles to test is found by its origin, within [`REACH`] of the
/// point the pointer meets.
fn pick(
    building: &Building,
    placed: &Placed,
    models: &mut ModelCache,
    shapes: &mut HashMap<String, Option<Shape>>,
    (origin, direction): (Vec3, Vec3),
    surface: Option<Vec3>,
) -> Option<u16> {
    let sets = building.root.doodad_sets();
    let doodads = building.root.doodads();
    let mut best: Option<(f32, u16)> = None;
    for index in sets.iter().enumerate().filter(|(set, _)| placed.draws(*set)).flat_map(|(_, run)| run.start..run.start + run.count) {
        let Some(doodad) = doodads.get(index as usize) else { continue };
        let path = vale_assets::world::m2::model_path(&doodad.name);
        let Some(shape) = shape(shapes, models, &path) else { continue };
        let matrix = matrix_of(placed, doodad);
        // The sphere about the origin that holds the model, first.
        let centre = matrix.transform_point3(Vec3::ZERO);
        let radius = shape.radius * doodad.scale.abs();
        let along = (centre - origin).dot(direction);
        if (origin + direction * along - centre).length() > radius || along < -radius {
            continue;
        }
        let hit = vale_assets::look::pick::hit_mesh(
            &shape.pick,
            |vertex| matrix.transform_point3(Vec3::from(shape.pick.positions[vertex])).to_array(),
            origin.to_array(),
            direction.to_array(),
        );
        if let Some(distance) = hit {
            if best.is_none_or(|(had, _)| distance < had) {
                best = Some((distance, index as u16));
            }
        }
    }
    best.map(|(_, index)| index).or_else(|| surface.and_then(|at| spawn_near(building, placed, at)))
}

/// A drag of the chosen spawn with the pointer: across the level it stood
/// at, or with Ctrl onto the surface under the pointer.
#[derive(Debug, Clone, Copy)]
struct Drag {
    /// Where the pointer met that level when the button went down, and where
    /// the spawn stood, both in the world.
    from: Vec3,
    was: Vec3,
    /// Whether the pointer has moved the spawn yet. A press that does not
    /// move it changes nothing.
    moved: bool,
}

/// Where a ray meets the level plane at `height`.
fn level_hit(origin: Vec3, direction: Vec3, height: f32) -> Option<Vec3> {
    if direction.z.abs() < 1e-5 {
        return None;
    }
    let t = (height - origin.z) / direction.z;
    (t > 0.0).then(|| origin + direction * t)
}

/// Choose the spawn under the pointer, drag it, or add the model on the
/// cursor where a click lands. See the module comment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn aim(
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    cursor: Res<crate::pick::Cursor>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    (assets, time, gizmo): (Res<GameAssets>, Res<Time>, Res<super::gizmo::Gizmo>),
    mut models: ResMut<ModelCache>,
    mut spawns: ResMut<Spawns>,
    session: Option<ResMut<EditSession>>,
) {
    if *tool != Tool::Wmos || !state.editing() || !spawns.active {
        spawns.hovered = None;
        spawns.drag = None;
        return;
    }
    let Some(mut session) = session else { return };
    let Some(placed) = spawns.building.clone() else { return };
    if session.open_building(&assets, &placed.path).is_err() {
        return;
    }
    let now = time.elapsed_secs_f64();
    if buttons.just_released(MouseButton::Left) {
        if spawns.drag.take().is_some_and(|drag| drag.moved) {
            session.history.release();
        }
        return;
    }
    let over = crate::ui::over_the_world(&viewport, &wants, &windows);
    let pointer = ray(&windows, &camera);

    // A drag in progress, wherever the pointer is.
    if let (Some(drag), Some(index), true) = (spawns.drag, spawns.chosen, buttons.pressed(MouseButton::Left)) {
        let Some((origin, direction)) = pointer else { return };
        let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
        let to = match ctrl {
            true => cursor.surface,
            false => level_hit(origin, direction, drag.was.z).map(|at| drag.was + (at - drag.from)),
        };
        let Some(to) = to else { return };
        let local = placed.to_local.transform_point3(to).to_array();
        let standing = session.building(&placed.path).and_then(|open| open.building.root.doodads().get(usize::from(index)).map(|d| d.position));
        if standing == Some(local) {
            return;
        }
        session.history.hold(now);
        if let Err(why) = change_spawn(&mut session, &assets, &placed.path, index, "Move building doodad", now, |doodad| doodad.position = local) {
            session.status = why;
        }
        if let Some(drag) = spawns.drag.as_mut() {
            drag.moved = true;
        }
        return;
    }

    let Some(open) = session.building(&placed.path) else { return };
    let hovered = match (over, pointer) {
        (true, Some(pointer)) if spawns.adding.is_none() && !gizmo.holding() => {
            let Spawns { shapes, .. } = &mut *spawns;
            pick(&open.building, &placed, &mut models, shapes, pointer, cursor.surface)
        }
        _ => None,
    };
    spawns.hovered = hovered;
    if !over || !buttons.just_pressed(MouseButton::Left) || gizmo.holding() {
        return;
    }
    let editable = spawns.editable(&session);

    // A model on the cursor is added where the click lands, and stays on the
    // cursor for the next.
    if let Some(model) = spawns.adding.clone() {
        let Some(at) = cursor.surface else { return };
        if !editable {
            session.status = "choose Copy for This Placement or Edit the Shared File first".to_string();
            return;
        }
        let doodad = Doodad {
            name: model,
            name_at: None,
            flags: 0,
            position: placed.to_local.transform_point3(at).to_array(),
            rotation: rotation_of(vale_assets::world::adt::placement_euler_from_world([0.0, 0.0, spawns.turn])),
            scale: spawns.scale,
            colour: 0xFF7F_7F7F,
        };
        let set = usize::from(placed.set);
        session.status = match add_spawn(&mut session, &assets, &placed.path, set, doodad, now) {
            Ok(index) => {
                spawns.chosen = Some(index);
                format!("added a doodad to set {set} of {}", placed.leaf())
            }
            Err(why) => why,
        };
        return;
    }

    spawns.chosen = hovered;
    spawns.drag = None;
    let (Some(index), true, Some((origin, direction))) = (hovered, editable, pointer) else { return };
    let Some(doodad) = open.building.root.doodads().get(usize::from(index)).cloned() else { return };
    let was = world_of(&placed, &doodad);
    if let Some(from) = level_hit(origin, direction, was.z) {
        spawns.drag = Some(Drag { from, was, moved: false });
    }
}

/// Yards an arrow key or Page Up and Page Down moves the chosen spawn, and
/// degrees `,` and `.` turn it; Shift is ten times each. The doodad tool's
/// own steps.
const STEP: f32 = 0.5;
const TURN: f32 = 5.0;

/// The keys and the wheel: Delete, Ctrl+D, the nudges, the turn and the scale,
/// for the chosen spawn or the model on the cursor; and Escape.
#[allow(clippy::too_many_arguments)]
fn keys(
    tool: Res<Tool>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    typing: Res<vale_client::lua::api::keyboard::KeyboardFocus>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    (assets, time): (Res<GameAssets>, Res<Time>),
    mut spawns: ResMut<Spawns>,
    mut view: ResMut<super::wmoview::WmoView>,
    session: Option<ResMut<EditSession>>,
) {
    if *tool != Tool::Wmos || !spawns.active || wants.wants_keyboard_input() || typing.active {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let wheel = match ctrl && crate::ui::over_the_world(&viewport, &wants, &windows) {
        true => scroll.delta.y,
        false => 0.0,
    };
    if keys.just_pressed(KeyCode::Escape) {
        match (spawns.adding.is_some(), spawns.chosen.is_some()) {
            (true, _) => spawns.adding = None,
            (false, true) => spawns.chosen = None,
            (false, false) => view.leave = true,
        }
        return;
    }
    // The model on the cursor.
    if spawns.adding.is_some() {
        let far = if shift { 3.0 } else { 1.0 };
        if keys.just_pressed(KeyCode::Comma) {
            spawns.turn = (spawns.turn - TURN * far).rem_euclid(360.0);
        }
        if keys.just_pressed(KeyCode::Period) {
            spawns.turn = (spawns.turn + TURN * far).rem_euclid(360.0);
        }
        if wheel != 0.0 {
            spawns.scale = (spawns.scale * (1.0 + wheel * 0.05)).clamp(0.01, 50.0);
        }
        return;
    }
    let Some(mut session) = session else { return };
    let (Some(placed), Some(index)) = (spawns.building.clone(), spawns.chosen) else { return };
    if !spawns.editable(&session) {
        return;
    }
    let Some(doodad) = session.building(&placed.path).and_then(|open| open.building.root.doodads().get(usize::from(index)).cloned()) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    if keys.just_pressed(KeyCode::Delete) {
        session.status = match remove_spawn(&mut session, &assets, &placed.path, index, now) {
            Ok(()) => format!("removed a doodad from {}", placed.leaf()),
            Err(why) => why,
        };
        spawns.chosen = None;
        return;
    }
    // Ctrl+D puts a copy on the cursor at the same turn and scale, as the
    // doodad tool's Duplicate does.
    if ctrl && keys.just_pressed(KeyCode::KeyD) {
        spawns.adding = Some(vale_assets::world::m2::model_path(&doodad.name));
        spawns.turn = turn_of(doodad.rotation).rem_euclid(360.0);
        spawns.scale = doodad.scale;
        return;
    }
    let step = if shift { STEP * 10.0 } else { STEP };
    let turn = if shift { TURN * 10.0 } else { TURN };
    let mut moved = Vec3::ZERO;
    for (key, by) in [
        (KeyCode::ArrowUp, Vec3::X),
        (KeyCode::ArrowDown, Vec3::NEG_X),
        (KeyCode::ArrowLeft, Vec3::Y),
        (KeyCode::ArrowRight, Vec3::NEG_Y),
        (KeyCode::PageUp, Vec3::Z),
        (KeyCode::PageDown, Vec3::NEG_Z),
    ] {
        if keys.just_pressed(key) {
            moved += by * step;
        }
    }
    let mut turned = 0.0;
    if keys.just_pressed(KeyCode::Comma) {
        turned -= turn;
    }
    if keys.just_pressed(KeyCode::Period) {
        turned += turn;
    }
    let scaled = match wheel {
        0.0 => doodad.scale,
        wheel => (doodad.scale * (1.0 + wheel * 0.05)).clamp(0.01, 50.0),
    };
    if moved == Vec3::ZERO && turned == 0.0 && scaled == doodad.scale {
        return;
    }
    let local = (Vec3::from(doodad.position) + placed.to_local.transform_vector3(moved)).to_array();
    let rotation = match turned {
        0.0 => doodad.rotation,
        _ => (Quat::from_rotation_z(turned.to_radians()) * Quat::from_array(doodad.rotation)).normalize().to_array(),
    };
    if let Err(why) = change_spawn(&mut session, &assets, &placed.path, index, "Move building doodad", now, |doodad| {
        doodad.position = local;
        doodad.rotation = rotation;
        doodad.scale = scaled;
    }) {
        session.status = why;
    }
}

/// Marks the model on the cursor, on its root and every batch, so nothing
/// else takes it for a placed model.
#[derive(Component, Clone)]
struct Ghost;

/// Keep the model on the cursor where the pointer meets the building,
/// building it when the model changes. The placer's own preview, drawn
/// translucent; see `super::place`.
fn ghost(
    tool: Res<Tool>,
    cursor: Res<crate::pick::Cursor>,
    mut spawns: ResMut<Spawns>,
    mut models: ResMut<ModelCache>,
    mut materials: Materials,
    mut commands: Commands,
    mut ghosts: Query<&mut Transform, With<Ghost>>,
) {
    let wanted = spawns.adding.clone().filter(|_| *tool == Tool::Wmos && spawns.active);
    let (Some(path), Some(at)) = (wanted, cursor.surface) else {
        if let Some((ghost, _)) = spawns.ghost.take() {
            commands.entity(ghost).despawn();
        }
        return;
    };
    if spawns.ghost.as_ref().is_none_or(|(_, showing)| *showing != path) {
        if let Some((ghost, _)) = spawns.ghost.take() {
            commands.entity(ghost).despawn();
        }
        let Lookup::Ready(model) = models.lookup(&path) else { return };
        let ghost = super::place::spawn_ghost(&mut commands, &mut materials, &model.draws, Ghost, Transform::default());
        spawns.ghost = Some((ghost, path));
    }
    let Some((ghost, _)) = &spawns.ghost else { return };
    if let Ok(mut transform) = ghosts.get_mut(*ghost) {
        *transform = super::place::transform_of(super::place::Kind::Doodad, at, spawns.turn, [0.0; 2], spawns.scale);
    }
}

/// The colour of the chosen spawn's mark, and of the one under the pointer.
const CHOSEN: Color = Color::srgb(0.31, 0.64, 0.89);
const HOVERED: Color = Color::srgba(0.89, 0.90, 0.93, 0.8);

/// Mark the chosen spawn and the one under the pointer: a ball at the
/// spawn's origin and a stem up from it. The handles stand on the chosen one
/// as well.
///
/// The mark is not a box round the model, as the doodad tool draws. A box of
/// marks, of lines or of tubes, drawn round a building's doodad stopped that
/// doodad being drawn; the same box eight yards to the side did not. The
/// cause is not known; see the editor's notes on the building editor.
fn draw(spawns: Res<Spawns>, session: Option<Res<EditSession>>, mut marks: ResMut<Marks>) {
    if !spawns.active {
        return;
    }
    let (Some(placed), Some(session)) = (&spawns.building, session) else { return };
    let Some(open) = session.building(&placed.path) else { return };
    let doodads = open.building.root.doodads();
    let hovered = spawns.hovered.filter(|index| Some(*index) != spawns.chosen);
    for (index, colour) in [(hovered, HOVERED), (spawns.chosen, CHOSEN)] {
        let Some(doodad) = index.and_then(|index| doodads.get(usize::from(index))) else { continue };
        let at = world_of(placed, doodad);
        let bevy = axes::to_bevy(at.to_array());
        let radius = marks.visible_radius(bevy, 0.3);
        marks.sphere(bevy, radius, colour, Look::Ghosted);
        let top = axes::to_bevy((at + Vec3::Z * 2.5).to_array());
        let width = marks.line_radius((bevy + top) * 0.5);
        marks.tube(bevy, top, width, colour, Look::Ghosted);
    }
}

/// The Doodads mode's systems; Lights mode runs after them, since it reads
/// the building they follow.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct WmoDoodadsSet;

pub(crate) struct WmoDoodadsPlugin;

impl Plugin for WmoDoodadsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Spawns>().add_systems(
            Update,
            (scripted, follow_the_mode, aim, keys, ghost, draw)
                .chain()
                .in_set(WmoDoodadsSet)
                .after(super::wmoview::follow)
                .after(super::gizmo::GizmoSet),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The nearest drawn spawn within reach is chosen; a spawn in a set the
    /// placement does not draw, or out of reach, is not.
    #[test]
    fn the_nearest_drawn_spawn_is_chosen() {
        let mut root = Vec::new();
        for (magic, data) in [(*b"MVER", 17u32.to_le_bytes().to_vec()), (*b"MOHD", vec![0u8; 64])] {
            root.extend([magic[3], magic[2], magic[1], magic[0]]);
            root.extend((data.len() as u32).to_le_bytes());
            root.extend(data);
        }
        let mut file = vale_edit::wmo::RootFile::parse(&root).unwrap();
        let doodad = |x: f32| Doodad { name: "a.mdx".into(), name_at: None, flags: 0, position: [x, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: 1.0, colour: 0 };
        let sets = vec![
            DoodadSet { name: "Global".into(), start: 0, count: 2, pad: 0 },
            DoodadSet { name: "Night".into(), start: 2, count: 1, pad: 0 },
        ];
        file.set_doodads(&sets, &[doodad(0.0), doodad(4.0), doodad(1.0)]).unwrap();
        let building = Building::new(file);
        let placed = Placed { unique_id: 1, path: "a.wmo".into(), to_world: Mat4::IDENTITY, to_local: Mat4::IDENTITY, set: 0 };
        assert_eq!(spawn_near(&building, &placed, Vec3::new(0.8, 0.0, 0.0)), Some(0), "set 1 is not drawn");
        assert_eq!(spawn_near(&building, &Placed { set: 1, ..placed.clone() }, Vec3::new(0.8, 0.0, 0.0)), Some(2));
        assert_eq!(spawn_near(&building, &placed, Vec3::new(30.0, 0.0, 0.0)), None);
        assert!(placed.draws(0) && !placed.draws(1));
    }

    /// A spawn's rotation and `MDDF`'s three angles give the same matrix: the
    /// angles' `placement_matrix` is the spawn's own, and the round trip
    /// keeps the rotation.
    #[test]
    fn a_rotation_and_its_angles_place_the_model_alike() {
        for rotation in [
            Quat::IDENTITY,
            Quat::from_rotation_z(0.7),
            Quat::from_euler(EulerRot::ZYX, 2.1, 0.3, -0.4),
            Quat::from_euler(EulerRot::ZYX, -1.0, -0.6, 1.2),
        ] {
            let rotation = rotation.to_array();
            let angles = angles_of(rotation);
            let from_angles = Mat4::from_cols_array(&vale_assets::world::adt::placement_matrix([0.0; 3], angles, 1.0));
            let own = Mat4::from_cols_array(&vale_assets::world::wmo::doodad_matrix([0.0; 3], rotation, 1.0));
            for probe in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1.0, 2.0, 3.0)] {
                let (a, b) = (from_angles.transform_point3(probe), own.transform_point3(probe));
                assert!((a - b).length() < 1e-4, "{rotation:?}: {a} against {b}");
            }
            let back = Quat::from_array(rotation_of(angles));
            assert!(back.dot(Quat::from_array(rotation)).abs() > 0.9999, "{rotation:?} came back as {back:?}");
        }
    }

    /// A ray meets a level plane ahead of it and not behind it.
    #[test]
    fn a_ray_meets_the_level_ahead_of_it() {
        let hit = level_hit(Vec3::new(0.0, 0.0, 10.0), Vec3::new(1.0, 0.0, -1.0).normalize(), 2.0).unwrap();
        assert!((hit - Vec3::new(8.0, 0.0, 2.0)).length() < 1e-4);
        assert_eq!(level_hit(Vec3::new(0.0, 0.0, 10.0), Vec3::Z, 2.0), None);
    }
}
