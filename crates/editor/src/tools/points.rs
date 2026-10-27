//! Points on the map that a tool picks with the pointer and drags: the part
//! the area trigger, graveyard and flight path tools have in common.
//!
//! [`MapPoint`] is a row with an id and a position. [`Points`] is one tool's
//! list of them on the open map and what the pointer is doing with them. A
//! [`PointTool`] is a tool whose subject is one such list, and [`collect`],
//! [`aim`], [`drag`] and [`delete_selected`] are written once over it.
//! [`PointDrag`] is the drag itself; the flight path tool, whose handles are
//! nodes and path points rather than one list, uses it on its own.
//!
//! ## What a drag does
//!
//! A press on a point selects it and holds it. Nothing moves until the
//! pointer has travelled [`DRAG_PIXELS`], so a click that selects does not
//! also nudge. The first moving frame measures the offset from the pointer's
//! hit to the point and writes nothing, so the point does not jump under the
//! pointer. The point then moves across the horizontal plane at its own
//! height or, with Ctrl held, up and down the upright plane through it that
//! faces the camera. A change of plane measures the offset again, so pressing
//! or releasing Ctrl in the middle of a drag does not make the point jump.

use crate::session::EditSession;
use crate::tools::placements::group::DRAG_PIXELS;
use crate::tools::Tool;
use bevy::prelude::*;
use std::collections::HashSet;
use vale_client::assets::GameAssets;
use vale_client::render::axes;
use vale_client::render::focus::WorldFocus;
use vale_client::world::camera::WorldCamera;
use vale_edit::dbc::DbcFile;

/// How wide a point is to the pointer, in logical pixels. A point has no
/// area, so the pointer picks it within a disc of this width around it, which
/// is the same target at every distance.
pub(crate) const HANDLE_PIXELS: f32 = 12.0;

/// The pointer's ray and the numbers that turn a distance off it into pixels.
pub(crate) struct Aim {
    pub(crate) origin: Vec3,
    pub(crate) direction: Vec3,
    /// The projection's `[1][1]`, `1 / tan(fov / 2)`.
    focal: f32,
    /// The window's height in logical pixels.
    height: f32,
}

impl Aim {
    /// How many pixels a distance of `off` yards from the ray is, at `along`
    /// yards down it.
    pub(crate) fn pixels(&self, off: f32, along: f32) -> f32 {
        off / along * self.focal * self.height * 0.5
    }

    /// How far a point is from the ray in pixels, or `None` behind the eye.
    pub(crate) fn to_point(&self, at: Vec3) -> Option<f32> {
        let to = at - self.origin;
        let along = to.dot(self.direction);
        if along <= 0.0 {
            return None;
        }
        Some(self.pixels((to - self.direction * along).length(), along))
    }

    /// How far a segment is from the ray in pixels, at its nearest.
    pub(crate) fn to_segment(&self, a: Vec3, b: Vec3) -> Option<f32> {
        let (along, off) = crate::pick::ray_to_segment(self.origin, self.direction, a, b)?;
        Some(self.pixels(off, along))
    }

    /// Whether any point in the sphere at `centre` of `radius` yards could be
    /// within `pixels` of the ray in front of the eye. False only when no
    /// point is, so a pick that skips what this rules out finds what a pick
    /// over everything finds.
    ///
    /// A point of the sphere is at least `off - radius` yards from the ray and
    /// at most `along + radius` yards down it, and [`Self::pixels`] grows with
    /// the first and shrinks with the second.
    pub(crate) fn may_reach(&self, centre: Vec3, radius: f32, pixels: f32) -> bool {
        let to = centre - self.origin;
        let along = to.dot(self.direction);
        if along + radius <= 0.0 {
            return false;
        }
        let off = (to - self.direction * along).length();
        off <= radius || self.pixels(off - radius, along + radius) <= pixels
    }
}

/// The pointer's [`Aim`], when it is over the window and the camera has a
/// perspective projection.
pub(crate) fn aim_of(
    windows: &Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<Aim> {
    let (origin, direction) = crate::pick::ray(windows, camera)?;
    let window = windows.single().ok()?;
    let (camera, _) = camera.single().ok()?;
    let focal = camera.clip_from_view().to_cols_array()[5];
    (focal.abs() > f32::EPSILON).then_some(Aim {
        origin,
        direction,
        focal,
        height: window.height().max(1.0),
    })
}

/// The eye, in the world's axes.
pub(crate) fn eye(camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>) -> Option<Vec3> {
    let (_, at) = camera.single().ok()?;
    Some(Vec3::from(axes::to_wow(at.translation())))
}

/// Whether Ctrl is held, which makes a drag move its point up and down.
pub(crate) fn vertical(keys: &ButtonInput<KeyCode>) -> bool {
    keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight])
}

/// A held left button on a point. `H` names what is held: a point's id, or a
/// flight path tool handle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PointDrag<H> {
    pub(crate) handle: H,
    /// Where the pointer was when the button went down, in logical pixels.
    pub(crate) pressed_at: Vec2,
    /// Whether the pointer has travelled [`DRAG_PIXELS`], which makes the
    /// press a drag.
    pub(crate) moving: bool,
    /// The offset from the pointer's hit to the point, and whether it was
    /// measured on the upright plane. See the module comment.
    pub(crate) grab: Option<(Vec3, bool)>,
}

impl<H> PointDrag<H> {
    /// A press on `handle`, with the pointer where it is now.
    pub(crate) fn press(handle: H, windows: &Query<&Window, With<bevy::window::PrimaryWindow>>) -> PointDrag<H> {
        let pressed_at = windows.single().ok().and_then(Window::cursor_position).unwrap_or_default();
        PointDrag { handle, pressed_at, moving: false, grab: None }
    }

    /// Where the held point, now at `from`, is moved to this frame. `None`
    /// while the press is still a click, on the frame that measures the grab
    /// offset, and when the pointer's ray meets neither plane.
    pub(crate) fn follow(
        &mut self,
        from: Vec3,
        vertical: bool,
        windows: &Query<&Window, With<bevy::window::PrimaryWindow>>,
        camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    ) -> Option<Vec3> {
        let pointer = windows.single().ok().and_then(Window::cursor_position)?;
        if !self.moving {
            if pointer.distance(self.pressed_at) < DRAG_PIXELS {
                return None;
            }
            self.moving = true;
        }
        let (origin, direction) = crate::pick::ray(windows, camera)?;
        let hit = match vertical {
            false => crate::pick::meets_plane(origin, direction, from.z),
            true => crate::pick::meets_upright_plane(origin, direction, from),
        }?;
        let grab = match self.grab {
            Some((grab, was_vertical)) if was_vertical == vertical => grab,
            _ => {
                self.grab = Some((from - hit, vertical));
                return None;
            }
        };
        Some(match vertical {
            false => Vec3::new(hit.x + grab.x, hit.y + grab.y, from.z),
            true => Vec3::new(from.x, from.y, hit.z + grab.z),
        })
    }
}

/// A row of a client table that is a point on a map.
pub(crate) trait MapPoint {
    /// The row's id, which a selection names.
    fn id(&self) -> u32;
    /// Where it is, in the world's axes.
    fn position(&self) -> Vec3;
}

/// One tool's points on the open map, and what the pointer is doing with
/// them.
#[derive(Debug)]
pub(crate) struct Points<P> {
    /// The points on the open map, in file order.
    pub(crate) list: Vec<P>,
    /// The ids the game's own copy of the table has, read once, for the
    /// warning a removal of one gives.
    pub(crate) shipped: Option<HashSet<u32>>,
    /// The session's table revision and the map the list was built for.
    built: Option<(u64, u32)>,
    pub(crate) map: u32,
    pub(crate) selected: Option<u32>,
    pub(crate) hovered: Option<u32>,
    pub(crate) drag: Option<PointDrag<u32>>,
    /// A shipped point the panel is asking to remove, until it is confirmed.
    pub(crate) confirm_remove: Option<u32>,
}

impl<P> Default for Points<P> {
    fn default() -> Points<P> {
        Points {
            list: Vec::new(),
            shipped: None,
            built: None,
            map: 0,
            selected: None,
            hovered: None,
            drag: None,
            confirm_remove: None,
        }
    }
}

impl<P: MapPoint> Points<P> {
    pub(crate) fn get(&self, id: u32) -> Option<&P> {
        self.list.iter().find(|point| point.id() == id)
    }

    /// Rebuild the list on the next frame, after an edit this frame.
    pub(crate) fn stale(&mut self) {
        self.built = None;
    }

    /// Whether the list has been built since the last edit.
    pub(crate) fn is_built(&self) -> bool {
        self.built.is_some()
    }

    pub(crate) fn is_shipped(&self, id: u32) -> bool {
        self.shipped.as_ref().is_some_and(|ids| ids.contains(&id))
    }

    /// Whether a point is drawn and picked: the selected one always, the rest
    /// within `range` yards of the eye.
    pub(crate) fn near(&self, point: &P, eye: Vec3, range: f32) -> bool {
        self.selected == Some(point.id()) || (point.position() - eye).length() <= range
    }

    /// The colour a selected point, or the point under the pointer, is drawn
    /// in. `None` for any other, which the tool colours by what it is.
    pub(crate) fn highlight(&self, id: u32) -> Option<Color> {
        match (self.selected == Some(id), self.hovered == Some(id)) {
            (true, _) => Some(Color::WHITE),
            (false, true) => Some(Color::srgb(1.0, 1.0, 0.8)),
            (false, false) => None,
        }
    }

    /// A press on a point: select it and hold it for a drag.
    pub(crate) fn grab(&mut self, id: u32, windows: &Query<&Window, With<bevy::window::PrimaryWindow>>) {
        self.selected = Some(id);
        self.confirm_remove = None;
        self.drag = Some(PointDrag::press(id, windows));
    }
}

/// A tool whose subject is a list of [`MapPoint`]s read from one client table.
pub(crate) trait PointTool: Resource<Mutability = bevy::ecs::component::Mutable> {
    type Point: MapPoint + Clone;
    /// The tool this is.
    const TOOL: Tool;
    /// The client table the points are rows of.
    const TABLE: &'static str;
    /// How far from the eye a point is drawn and picked, in yards.
    const RANGE: f32;
    /// How near the pointer must be to a point's centre to pick it, in
    /// logical pixels.
    const PICK_PIXELS: f32;
    /// What a point is called in a status line: "trigger", "graveyard".
    const NOUN: &'static str;

    fn points(&self) -> &Points<Self::Point>;
    fn points_mut(&mut self) -> &mut Points<Self::Point>;

    /// Fill [`Points::list`] with the open map's rows of `table`.
    fn rebuild(&mut self, table: &DbcFile, map: u32);

    /// Follow what else the tool reads from the session, every frame the
    /// tool is open, before the list is checked.
    fn follow(&mut self, _session: &EditSession) {}

    /// Write a point's new position as one gesture.
    fn set_position(session: &mut EditSession, point: &Self::Point, to: Vec3, now: f64);

    /// The point the pointer is over when no centre is within
    /// [`Self::PICK_PIXELS`]: an area trigger's volume.
    fn aim_inside(&self, _aim: &Aim, _eye: Vec3) -> Option<u32> {
        None
    }

    /// Remove a point and what names it. Answers the status line.
    fn remove(&mut self, session: &mut EditSession, id: u32, now: f64) -> String;
}

fn active<T: PointTool>(tool: &Tool, state: &crate::playtest::Playtest) -> bool {
    *tool == T::TOOL && state.editing()
}

/// The ids of the game's own copy of a table, read past the overlay and past
/// this project's own archives. Empty when it cannot be read.
pub(crate) fn shipped_ids(session: &EditSession, assets: &GameAssets, table: &str) -> HashSet<u32> {
    let own: Vec<String> = session.project.published().into_iter().map(|archive| archive.name).collect();
    let path = vale_assets::tables::dbc::dbc_path(table);
    assets
        .read_past_overlay_skipping(&path, &own)
        .ok()
        .and_then(|bytes| DbcFile::parse(&bytes).ok())
        .map(|file| (0..file.record_count()).filter_map(|record| file.u32_at(record, 0)).collect())
        .unwrap_or_default()
}

/// Read the table into the list when the table or the map changed, and drop
/// a selection the new list does not hold.
pub(crate) fn collect<T: PointTool>(
    mut subject: ResMut<T>,
    session: Option<Res<EditSession>>,
    assets: Res<GameAssets>,
    focus: Res<WorldFocus>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !active::<T>(&tool, &state) {
        return;
    }
    let Some(session) = session else { return };
    subject.follow(&session);
    let key = (session.table_revision, focus.map_id);
    if subject.points().built == Some(key) {
        return;
    }
    let Some(table) = session.table(T::TABLE) else {
        return;
    };
    if subject.points().shipped.is_none() {
        subject.points_mut().shipped = Some(shipped_ids(&session, &assets, T::TABLE));
    }
    subject.points_mut().built = Some(key);
    subject.points_mut().map = focus.map_id;
    subject.rebuild(table, focus.map_id);
    let points = subject.points_mut();
    if points.selected.is_some_and(|id| points.get(id).is_none()) {
        points.selected = None;
    }
}

/// Which point the pointer is over: the nearest centre within
/// [`PointTool::PICK_PIXELS`], else [`PointTool::aim_inside`].
pub(crate) fn aim<T: PointTool>(
    mut subject: ResMut<T>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !active::<T>(&tool, &state) {
        subject.points_mut().hovered = None;
        return;
    }
    // A held drag keeps what it grabbed.
    if subject.points().drag.is_some_and(|drag| drag.moving) {
        return;
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        subject.points_mut().hovered = None;
        return;
    }
    let (Some(aim), Some(eye)) = (aim_of(&windows, &camera), eye(&camera)) else {
        subject.points_mut().hovered = None;
        return;
    };
    let points = subject.points();
    let centre = points
        .list
        .iter()
        .filter(|point| points.near(point, eye, T::RANGE))
        .filter_map(|point| Some((aim.to_point(point.position())?, point.id())))
        .filter(|(pixels, _)| *pixels <= T::PICK_PIXELS)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id);
    let hovered = centre.or_else(|| subject.aim_inside(&aim, eye));
    subject.points_mut().hovered = hovered;
}

/// Move the held point. See the module comment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drag<T: PointTool>(
    mut subject: ResMut<T>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    time: Res<Time>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    mut session: Option<ResMut<EditSession>>,
) {
    if !active::<T>(&tool, &state) || !buttons.pressed(MouseButton::Left) {
        subject.points_mut().drag = None;
        return;
    }
    let Some(mut held) = subject.points().drag else { return };
    let Some(point) = subject.points().get(held.handle).cloned() else {
        subject.points_mut().drag = None;
        return;
    };
    let Some(session) = session.as_mut() else { return };
    let to = held.follow(point.position(), vertical(&keys), &windows, &camera);
    subject.points_mut().drag = Some(held);
    let Some(to) = to else { return };
    T::set_position(session, &point, to, time.elapsed_secs_f64());
    subject.points_mut().stale();
}

/// Delete: remove the selected point. A shipped one is asked about in the
/// panel first.
pub(crate) fn delete_selected<T: PointTool>(subject: &mut T, session: &mut EditSession, now: f64) {
    let Some(id) = subject.points().selected else { return };
    let points = subject.points_mut();
    if points.is_shipped(id) && points.confirm_remove != Some(id) {
        points.confirm_remove = Some(id);
        session.status = format!("{noun} {id} is a shipped {noun}: confirm the removal in the panel", noun = T::NOUN);
        return;
    }
    let line = subject.remove(session, id, now);
    session.status = line;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `may_reach` never rules out a segment that `to_segment` puts within
    /// the pixels, for segments all around and behind a ray.
    #[test]
    fn a_sphere_test_never_rules_out_a_segment_within_reach() {
        let aim = Aim {
            origin: Vec3::new(5.0, -3.0, 40.0),
            direction: Vec3::new(1.0, 0.4, -0.3).normalize(),
            focal: 1.7,
            height: 900.0,
        };
        let pixels = HANDLE_PIXELS;
        let mut within = 0;
        // Segments along the ray, beside it and behind the eye.
        for i in -4i32..=40 {
            for j in -8i32..=8 {
                for k in -8i32..=8 {
                    let beside = Vec3::new(j as f32 * 0.7, k as f32 * 0.9, (j - k) as f32 * 0.4);
                    let a = aim.origin + aim.direction * (i as f32 * 10.0) + beside;
                    let b = a + Vec3::new(4.0, -2.5, 1.5) * (1 + (i + j).rem_euclid(4)) as f32;
                    let centre = (a + b) * 0.5;
                    let radius = (b - a).length() * 0.5;
                    if aim.to_segment(a, b).is_some_and(|found| found <= pixels) {
                        within += 1;
                        assert!(aim.may_reach(centre, radius, pixels), "{a} {b}");
                    }
                }
            }
        }
        assert!(within > 0, "some segments are within reach");
        // Far off the ray, and wholly behind the eye, are ruled out.
        let side = aim.origin + aim.direction * 100.0 + Vec3::Z * 500.0;
        assert!(!aim.may_reach(side, 5.0, pixels));
        assert!(!aim.may_reach(aim.origin - aim.direction * 50.0, 10.0, pixels));
    }
}
