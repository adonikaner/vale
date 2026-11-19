//! The building editor's Lights mode: the lights a building carries, its
//! root's `MOLT`, and the groups' `MOLR` lists of them, edited in the
//! building's files. The editor is [`super::wmoview`].
//!
//! ## What a light does
//!
//! A building's lights light the characters and creatures inside it
//! (`vale_client::render::building_lights`) and the night's lamps
//! (`vale_client::render::lamps`). The walls' own light is baked into the
//! group files' vertex colours and does not change with them. A group's
//! `MOLR` lists the lights that light it; a light added here is listed in the
//! group whose box holds it, and a light removed is taken out of every list,
//! the indexes after it moving down. A light moved stays in the lists it was
//! in.
//!
//! The building, its file and the copy or shared choice are the Doodads
//! mode's ([`super::wmodoodads::Spawns`]).
//!
//! ## In the viewport
//!
//! ```text
//! click near a light        choose it; every light is a ball of its colour
//! drag                      move it across its level; Ctrl onto the surface
//! the Move handles          move along an axis (`super::gizmo`)
//! arrows, Page Up/Down      nudge half a yard; Shift ten times
//! Delete                    remove
//! Ctrl + D                  a copy a yard north, chosen
//! Add Light, then a click   a light a yard and a half above where the click
//!                           lands
//! Escape                    stop adding, let go of the light, then leave
//! ```

use bevy::prelude::*;

use super::place::{Placing, Switch};
use super::wmodoodads::{Placed, Spawns};
use crate::marks::{Look, Marks};
use crate::session::EditSession;
use crate::tools::Tool;
use vale_client::assets::GameAssets;
use vale_client::render::axes;
use vale_client::world::camera::WorldCamera;
use vale_edit::wmo::{Light, Renumber};

/// The mode's state.
#[derive(Resource, Default)]
pub(crate) struct Lamps {
    /// Whether the mode is on: the building editor is open in Lights mode.
    pub(crate) active: bool,
    /// The chosen light, by `MOLT` index.
    pub(crate) chosen: Option<u16>,
    /// Whether the next click adds a light.
    pub(crate) adding: bool,
    hovered: Option<u16>,
    drag: Option<Drag>,
}

/// A drag of the chosen light: where the pointer met its level when the
/// button went down, and where the light stood, in the world.
#[derive(Debug, Clone, Copy)]
struct Drag {
    from: Vec3,
    was: Vec3,
    moved: bool,
}

/// Change one light; `change` is given it.
pub(crate) fn change_light(
    session: &mut EditSession,
    assets: &GameAssets,
    path: &str,
    index: u16,
    label: &str,
    now: f64,
    change: impl FnOnce(&mut Light),
) -> Result<(), String> {
    let subject = format!("building {path} light {index}");
    session.edit_building(assets, path, label, &subject, now, false, |building| {
        let mut lights = building.root.lights();
        let light = lights.get_mut(usize::from(index)).ok_or("no such light")?;
        change(light);
        building.root.set_lights(&lights);
        Ok(())
    })
}

/// Add a light after the last, listed in the group whose box holds it.
/// Answers its index.
pub(crate) fn add_light(session: &mut EditSession, assets: &GameAssets, path: &str, light: Light, now: f64) -> Result<u16, String> {
    let mut made = None;
    let subject = format!("building {path} add light");
    session.edit_building(assets, path, "Add building light", &subject, now, true, |building| {
        let mut lights = building.root.lights();
        let at = u16::try_from(lights.len()).map_err(|_| "a group's light list cannot index more lights")?;
        lights.push(light);
        building.root.set_lights(&lights);
        if let Some(home) = building.group_holding(light.position()) {
            if let Some(Some(group)) = building.groups.get_mut(home) {
                let mut refs = group.light_refs();
                refs.push(at);
                group.set_light_refs(&refs).map_err(|e| e.to_string())?;
            }
        }
        made = Some(at);
        Ok(())
    })?;
    made.ok_or_else(|| "nothing was added".to_string())
}

/// Remove a light, and take it out of every group's list.
pub(crate) fn remove_light(session: &mut EditSession, assets: &GameAssets, path: &str, index: u16, now: f64) -> Result<(), String> {
    let subject = format!("building {path} remove light");
    session.edit_building(assets, path, "Remove building light", &subject, now, true, |building| {
        let mut lights = building.root.lights();
        if usize::from(index) >= lights.len() {
            return Err(format!("there is no light {index}"));
        }
        lights.remove(usize::from(index));
        building.root.set_lights(&lights);
        for group in building.groups.iter_mut().flatten() {
            group.renumber_lights(Renumber::Removed(index)).map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

/// A light's place in the world.
pub(crate) fn world_of(placed: &Placed, light: &Light) -> Vec3 {
    placed.to_world.transform_point3(Vec3::from(light.position()))
}

/// What the gizmo stands on in Lights mode: the chosen light, with no turn.
/// See `super::gizmo`.
pub(crate) fn gizmo_held(lamps: &Lamps, spawns: &Spawns, session: Option<&EditSession>) -> Option<(Vec3, [f32; 3])> {
    let session = session?;
    if !lamps.active || lamps.adding || !spawns.editable(session) {
        return None;
    }
    let (placed, index) = (spawns.building.as_ref()?, lamps.chosen?);
    let light = *session.building(&placed.path)?.building.root.lights().get(usize::from(index))?;
    Some((world_of(placed, &light), [0.0; 3]))
}

/// …and what a handle writes back: the light moved to a world point. A light
/// has no turn, so the angles are not read.
pub(crate) fn gizmo_put(lamps: &Lamps, spawns: &Spawns, session: &mut EditSession, assets: &GameAssets, position: Vec3, now: f64) {
    let (Some(placed), Some(index)) = (spawns.building.as_ref(), lamps.chosen) else { return };
    let local = placed.to_local.transform_point3(position).to_array();
    if let Err(why) = change_light(session, assets, &placed.path, index, "Move building light", now, |light| light.set_position(local)) {
        session.status = why;
    }
}

/// How far off the pointer's ray a light may be and still be chosen, as a
/// fraction of its distance (about a degree), and never less than 0.3 yards.
const GRIP: f32 = 0.02;

/// The light nearest the pointer's ray, within [`GRIP`] of it.
fn pick(lights: &[Light], placed: &Placed, origin: Vec3, direction: Vec3) -> Option<u16> {
    lights
        .iter()
        .enumerate()
        .filter_map(|(index, light)| {
            let at = world_of(placed, light) - origin;
            let along = at.dot(direction);
            let off = (at - direction * along).length();
            (along > 0.0 && off <= (along * GRIP).max(0.3)).then_some((off / along, index as u16))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, index)| index)
}

/// `--wmo-lights [<index>]` and `--wmo-copy`: open the building editor in
/// Lights mode once the WMO tool is chosen, on a WMO-only map's building or
/// else on the drawn WMO nearest the camera,
/// copy the building, and choose the light once the copy is drawn, with the
/// camera on it. See `crate::args::tools::Tools::wmo_lights`.
#[allow(clippy::too_many_arguments)]
fn scripted(
    args: Res<crate::args::Args>,
    tool: Res<Tool>,
    assets: Res<GameAssets>,
    mut view: ResMut<super::wmoview::WmoView>,
    spawns: Res<Spawns>,
    mut lamps: ResMut<Lamps>,
    session: Option<ResMut<EditSession>>,
    mut fly: MessageWriter<crate::camera::FlyTo>,
    (placed, camera): (Query<(&vale_client::render::wmos::WmoPlacement, &vale_client::render::wmos::Interior)>, Res<crate::camera::EditorCamera>),
    mut done: Local<(u8, u32)>,
) {
    let Some(index) = args.tools.wmo_lights else { return };
    let (step, frames) = &mut *done;
    match *step {
        0 if *tool == Tool::Wmos && view.global.is_some() => {
            view.tab = Switch::Lights;
            view.asked = Some(u32::MAX);
            *step = 1;
        }
        // On a map with ground, the drawn WMO nearest the camera, once the
        // tiles round it have had a few seconds to stream.
        0 if *tool == Tool::Wmos => {
            *frames += 1;
            if *frames < 240 {
                return;
            }
            let nearest = placed
                .iter()
                .filter(|(placement, _)| placement.unique_id < super::wmoview::VIEW_ID)
                .min_by(|(_, a), (_, b)| {
                    let distance = |interior: &vale_client::render::wmos::Interior| {
                        interior.inverse.inverse().transform_point3(Vec3::ZERO).distance(camera.target)
                    };
                    distance(a).total_cmp(&distance(b))
                })
                .map(|(placement, _)| placement.unique_id);
            if let Some(id) = nearest {
                view.tab = Switch::Lights;
                view.asked = Some(id);
                *step = 1;
            }
        }
        1 => {
            let (Some(building), Some(mut session)) = (spawns.building.clone(), session) else { return };
            if args.tools.wmo_copy && !spawns.editable(&session) {
                let (from, tile) = view.open.as_ref().map_or((u32::MAX, None), |open| (open.from, open.tile));
                match super::wmodoodads::copy_for_placement(&mut session, &assets, &building.path, from, tile) {
                    Ok(path) => {
                        if let Some(open) = view.open.as_mut() {
                            open.path = path;
                        }
                    }
                    Err(why) => warn!("--wmo-copy: {why}"),
                }
            }
            *step = 2;
        }
        2 => {
            let path = view.open.as_ref().map(|open| open.path.clone());
            let (Some(path), Some(mut session)) = (path, session) else { return };
            let Some(building) = spawns.building.clone().filter(|building| building.path == path) else { return };
            lamps.chosen = index;
            if let Err(why) = session.open_building(&assets, &path) {
                warn!("--wmo-lights: {why}");
            }
            let lights = session.building(&path).map(|open| open.building.root.lights()).unwrap_or_default();
            info!("--wmo-lights: {path}, {} lights, chosen {index:?}", lights.len());
            if let Some(light) = index.and_then(|index| lights.get(usize::from(index))) {
                fly.write(crate::camera::FlyTo { at: world_of(&building, light), distance: Some(16.0) });
            }
            *step = 3;
        }
        _ => {}
    }
}

/// Keep the mode's flag in step.
fn follow_the_mode(view: Res<super::wmoview::WmoView>, placing: Res<Placing>, mut lamps: ResMut<Lamps>) {
    lamps.active = view.open.is_some() && placing.switch() == Switch::Lights;
    if view.open.is_none() && (lamps.chosen.is_some() || lamps.adding) {
        *lamps = Lamps::default();
    }
}

/// Choose, drag, or add a light. See the module comment.
#[allow(clippy::too_many_arguments)]
fn aim(
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
    spawns: Res<Spawns>,
    mut lamps: ResMut<Lamps>,
    session: Option<ResMut<EditSession>>,
) {
    if *tool != Tool::Wmos || !state.editing() || !lamps.active {
        lamps.hovered = None;
        lamps.drag = None;
        return;
    }
    let (Some(mut session), Some(placed)) = (session, spawns.building.clone()) else { return };
    let now = time.elapsed_secs_f64();
    if buttons.just_released(MouseButton::Left) {
        if lamps.drag.take().is_some_and(|drag| drag.moved) {
            session.history.release();
        }
        return;
    }
    let pointer = super::doodads::pointer_ray(&windows, &camera).map(|(origin, direction)| {
        (Vec3::from(axes::to_wow(origin)), Vec3::from(axes::to_wow(direction)).normalize_or_zero())
    });
    let Some(lights) = session.building(&placed.path).map(|open| open.building.root.lights()) else { return };

    if let (Some(drag), Some(index), true, Some((origin, direction))) = (lamps.drag, lamps.chosen, buttons.pressed(MouseButton::Left), pointer) {
        let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
        let to = match ctrl {
            true => cursor.surface.map(|at| at + Vec3::Z * 1.5),
            false => (direction.z.abs() > 1e-5)
                .then(|| (drag.was.z - origin.z) / direction.z)
                .filter(|t| *t > 0.0)
                .map(|t| drag.was + (origin + direction * t - drag.from)),
        };
        let Some(to) = to else { return };
        let local = placed.to_local.transform_point3(to).to_array();
        if lights.get(usize::from(index)).is_some_and(|light| light.position() == local) {
            return;
        }
        session.history.hold(now);
        if let Err(why) = change_light(&mut session, &assets, &placed.path, index, "Move building light", now, |light| light.set_position(local)) {
            session.status = why;
        }
        if let Some(drag) = lamps.drag.as_mut() {
            drag.moved = true;
        }
        return;
    }

    let over = crate::ui::over_the_world(&viewport, &wants, &windows);
    let hovered = match (over && !lamps.adding && !gizmo.holding(), pointer) {
        (true, Some((origin, direction))) => pick(&lights, &placed, origin, direction),
        _ => None,
    };
    lamps.hovered = hovered;
    if !over || !buttons.just_pressed(MouseButton::Left) || gizmo.holding() {
        return;
    }
    let editable = spawns.editable(&session);
    if lamps.adding {
        let Some(at) = cursor.surface else { return };
        if !editable {
            session.status = "choose Copy for This Placement or Edit the Shared File first".to_string();
            return;
        }
        let local = placed.to_local.transform_point3(at + Vec3::Z * 1.5).to_array();
        session.status = match add_light(&mut session, &assets, &placed.path, Light::new(local), now) {
            Ok(index) => {
                lamps.chosen = Some(index);
                lamps.adding = false;
                format!("added light {index} to {}", placed.leaf())
            }
            Err(why) => why,
        };
        return;
    }
    lamps.chosen = hovered;
    lamps.drag = None;
    let (Some(index), true, Some((origin, direction))) = (hovered, editable, pointer) else { return };
    let Some(light) = lights.get(usize::from(index)) else { return };
    let was = world_of(&placed, light);
    if direction.z.abs() > 1e-5 {
        let t = (was.z - origin.z) / direction.z;
        if t > 0.0 {
            lamps.drag = Some(Drag { from: origin + direction * t, was, moved: false });
        }
    }
}

/// Delete, Ctrl+D, the nudges, and Escape.
#[allow(clippy::too_many_arguments)]
fn keys(
    tool: Res<Tool>,
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    typing: Res<vale_client::lua::api::keyboard::KeyboardFocus>,
    (assets, time): (Res<GameAssets>, Res<Time>),
    spawns: Res<Spawns>,
    mut lamps: ResMut<Lamps>,
    mut view: ResMut<super::wmoview::WmoView>,
    session: Option<ResMut<EditSession>>,
) {
    if *tool != Tool::Wmos || !lamps.active || wants.wants_keyboard_input() || typing.active {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        match (lamps.adding, lamps.chosen.is_some()) {
            (true, _) => lamps.adding = false,
            (false, true) => lamps.chosen = None,
            (false, false) => view.leave = true,
        }
        return;
    }
    let (Some(mut session), Some(placed), Some(index)) = (session, spawns.building.clone(), lamps.chosen) else { return };
    if !spawns.editable(&session) {
        return;
    }
    let Some(light) = session.building(&placed.path).and_then(|open| open.building.root.lights().get(usize::from(index)).copied()) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    if keys.just_pressed(KeyCode::Delete) {
        session.status = match remove_light(&mut session, &assets, &placed.path, index, now) {
            Ok(()) => format!("removed light {index} from {}", placed.leaf()),
            Err(why) => why,
        };
        lamps.chosen = None;
        return;
    }
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    if ctrl && keys.just_pressed(KeyCode::KeyD) {
        session.status = match duplicate(&mut session, &assets, &placed, light, now) {
            Ok(added) => {
                lamps.chosen = Some(added);
                format!("added light {added}, a copy of light {index}")
            }
            Err(why) => why,
        };
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let step = if shift { 5.0 } else { 0.5 };
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
    if moved == Vec3::ZERO {
        return;
    }
    let local = (Vec3::from(light.position()) + placed.to_local.transform_vector3(moved)).to_array();
    if let Err(why) = change_light(&mut session, &assets, &placed.path, index, "Move building light", now, |light| light.set_position(local)) {
        session.status = why;
    }
}

/// Add a copy of a light a yard north of it. Answers its index.
pub(crate) fn duplicate(session: &mut EditSession, assets: &GameAssets, placed: &Placed, light: Light, now: f64) -> Result<u16, String> {
    let mut copy = light;
    let at = Vec3::from(light.position()) + placed.to_local.transform_vector3(Vec3::X);
    copy.set_position(at.to_array());
    add_light(session, assets, &placed.path, copy, now)
}

/// The colour of the chosen light's mark, and of the one under the pointer.
const CHOSEN: Color = Color::srgb(0.31, 0.64, 0.89);
const HOVERED: Color = Color::srgba(0.89, 0.90, 0.93, 0.9);

/// Every light a ball of its own colour; the chosen one a larger ball with a
/// ring at its far attenuation distance, and the one under the pointer a
/// larger pale ball.
fn draw(lamps: Res<Lamps>, spawns: Res<Spawns>, session: Option<Res<EditSession>>, mut marks: ResMut<Marks>) {
    if !lamps.active {
        return;
    }
    let (Some(placed), Some(session)) = (&spawns.building, session) else { return };
    let Some(open) = session.building(&placed.path) else { return };
    for (index, light) in open.building.root.lights().iter().enumerate() {
        let at = axes::to_bevy(world_of(placed, light).to_array());
        let [r, g, b] = light.colour();
        let mut colour = Color::srgb_u8(r, g, b);
        let mut size = 0.25;
        if lamps.hovered == Some(index as u16) {
            colour = HOVERED;
            size = 0.4;
        }
        if lamps.chosen == Some(index as u16) {
            colour = CHOSEN;
            size = 0.45;
            let far = light.attenuation()[1];
            if far > 0.0 {
                let width = marks.line_radius(at);
                marks.ring(at, Vec3::Y, far, width, CHOSEN.with_alpha(0.6), Look::Ghosted);
            }
        }
        let radius = marks.visible_radius(at, size);
        marks.sphere(at, radius, colour, Look::Ghosted);
    }
}

pub(crate) struct WmoLightsPlugin;

impl Plugin for WmoLightsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Lamps>().add_systems(
            Update,
            (scripted, follow_the_mode, aim, keys.run_if(crate::tools::tool_is(Tool::Wmos)), draw)
                .chain()
                .after(super::wmoview::follow)
                .after(super::wmodoodads::WmoDoodadsSet)
                .after(super::gizmo::GizmoSet),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The light nearest the ray is chosen, and one well off it is not.
    #[test]
    fn the_light_on_the_ray_is_chosen() {
        let placed = Placed { unique_id: 1, path: "a.wmo".into(), to_world: Mat4::IDENTITY, to_local: Mat4::IDENTITY, set: 0 };
        let lights = [Light::new([10.0, 0.0, 0.0]), Light::new([10.0, 3.0, 0.0])];
        assert_eq!(pick(&lights, &placed, Vec3::ZERO, Vec3::X), Some(0));
        assert_eq!(pick(&lights, &placed, Vec3::ZERO, Vec3::new(10.0, 3.0, 0.0).normalize()), Some(1));
        assert_eq!(pick(&lights, &placed, Vec3::ZERO, Vec3::Z), None);
    }
}
