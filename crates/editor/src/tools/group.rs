//! A selection that is more than one thing: the rules the four placement
//! tools share, the rectangle a drag on empty ground draws, the arithmetic of
//! turning a group about one point, and the clipboard.
//!
//! ## The rules, stated once
//!
//! The doodad, WMO, creature and game-object tools each hold a **primary**
//! selection — the one the inspector's numbers and the gizmo are on — and a
//! list of further **members** beside it. Every tool follows the same rules:
//!
//! ```text
//! click                selects one thing and drops the rest
//! shift + click        adds the thing to the group, or takes it out; an added
//!                      thing becomes the primary
//! drag on empty ground a rectangle: everything drawn whose origin is inside it
//!                      is selected. With shift it is added to the group
//! click on nothing     drops the whole selection (doodads, WMOs); leaves it
//!                      alone (creatures, objects — see `creatures::press`)
//! drag a member        moves every member by the same step. A press on a
//!                      member that is released without moving selects that
//!                      member alone
//! gizmo, keys, alt     act on every member: a move moves each by the same
//!                      step, a turn turns the group about the primary's origin
//!                      and each member about its own
//! delete               removes every member, as one undo entry
//! ctrl+c, ctrl+v       copies the group and pastes it with the primary's origin
//!                      on the surface under the pointer (doodads and WMOs)
//! ctrl+d               with one selected, arms the placer as before; with
//!                      several, pastes a copy two yards north (doodads and WMOs)
//! ```
//!
//! The primary stays a single record rather than becoming one entry of a
//! list. Every panel, the gizmo, the placer and the playtest's duplicate read a
//! single selected thing, and all of them keep working unchanged: they act on
//! the primary, and the group operations act on the primary and the members
//! together. The inspector edits the primary's own numbers; a group has no
//! single position to type.
//!
//! One gesture is one undo entry, whatever the size of the group. A tile
//! edit opens one entry around every member's write. A spawn's writers each
//! name their own row as the gesture subject, so a group write runs inside
//! `EditSession::as_one`, which gives every write the group's subject.
//!
//! ## What a group does not do
//!
//! * It is per tool. A doodad and a building are different lists with
//!   different rules (a building has no scale and carries its own box), so a
//!   group is doodads or buildings, not both.
//! * Spawns are not copied. A creature duplicate copies every column of
//!   the row, which is read one row at a time when the spawn is selected, so a
//!   group copy would have to read every member's row first. `Ctrl+D` on a
//!   group of spawns says so rather than writing rows with default columns.

use crate::session::EditSession;
use vale_client::render::axes;
use vale_client::world::camera::WorldCamera;
use vale_edit::adt::place::{Building, Doodad as Placement, MINTED_BASE};
use vale_edit::ops::{Edit, Placements};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

use super::Tool;

/// How far the pointer must travel from a press before it is a drag, in
/// pixels. The creature tool's threshold, for its reason: an ordinary click
/// moves the pointer a few pixels between press and release.
pub const DRAG_PIXELS: f32 = 6.0;

/// Yards north a group copy made with `Ctrl+D` is put from the original, so
/// the copy can be seen and clicked. The creature tool's duplicate distance.
pub const APART: f32 = 2.0;

/// A group's ids, sorted, for [`holds`].
pub fn sorted<T: Ord>(ids: impl Iterator<Item = T>) -> Vec<T> {
    let mut ids: Vec<T> = ids.collect();
    ids.sort_unstable();
    ids
}

/// Whether a sorted list of ids holds one. For the marker passes, which ask it
/// of every drawn entity on every frame: an empty group answers without a
/// search, and a full one in a handful of comparisons.
pub fn holds<T: Ord>(sorted: &[T], id: T) -> bool {
    !sorted.is_empty() && sorted.binary_search(&id).is_ok()
}

/// Whether either shift key is down.
pub fn shift(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight)
}

pub struct GroupPlugin;

impl Plugin for GroupPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Marquee>()
            .init_resource::<Clipboard>()
            // Before the four tools' presses: a release is turned into a
            // finished rectangle here, and each tool's own `finish` takes it
            // on the same frame.
            .add_systems(
                Update,
                track
                    .after(crate::pick::aim)
                    .before(super::doodads::select)
                    .before(super::wmos::select)
                    .before(super::creatures::press)
                    .before(super::gameobjects::press),
            )
            .add_systems(EguiPrimaryContextPass, paint)
            // After the undo key, for the reason the tools' own chains are: a
            // paste reads the selection, which the undo key's resync corrects.
            .add_systems(Update, clipboard.after(super::shortcuts))
            // Before the tools' own `enclose`, so the rectangle is taken the
            // frame it is released.
            .add_systems(Update, scripted.after(track).before(super::doodads::select));
    }
}

/// `--enclose`: release a rectangle over the world once it has streamed in,
/// and a second later print what it selected. See `crate::Args::enclose`.
#[allow(clippy::too_many_arguments)]
fn scripted(
    mut marquee: ResMut<Marquee>,
    mut done: Local<(bool, bool)>,
    args: Res<crate::Args>,
    shot: Res<crate::shot::Shot>,
    time: Res<Time>,
    tool: Res<Tool>,
    doodads: Res<super::doodads::Selection>,
    buildings: Res<super::wmos::Selection>,
    creatures: Res<super::creatures::Creatures>,
    objects: Res<super::gameobjects::GameObjects>,
) {
    let Some([x0, y0, x1, y1]) = args.enclose else {
        return;
    };
    let at = (shot.after - 7.0).max(1.0);
    let now = time.elapsed_secs();
    if !done.0 && now >= at {
        done.0 = true;
        marquee.done = Some(Done {
            tool: *tool,
            rect: Rect::new(x0, y0, x1, y1),
            adding: false,
        });
        info!("--enclose: released {x0},{y0} to {x1},{y1} with {}", tool.name());
    }
    if done.0 && !done.1 && now >= at + 1.0 {
        done.1 = true;
        let count = match *tool {
            Tool::Doodads => doodads.count(),
            Tool::Wmos => buildings.count(),
            Tool::Creatures => usize::from(creatures.selected.is_some()) + creatures.also.len(),
            Tool::GameObjects => usize::from(objects.selected.is_some()) + objects.also.len(),
            _ => 0,
        };
        info!("--enclose: {count} selected");
    }
}

/// `Ctrl+C`, `Ctrl+V`, and `Ctrl+D` over a group, for doodads and WMOs.
///
/// `Ctrl+D` over one placement is `super::shortcuts`', which arms the placer
/// with it; this answers only when more than one is selected, so the two never
/// act on the same press.
#[allow(clippy::too_many_arguments)]
fn clipboard(
    mut board: ResMut<Clipboard>,
    mut session: Option<ResMut<EditSession>>,
    mut doodads: ResMut<super::doodads::Selection>,
    mut buildings: ResMut<super::wmos::Selection>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    typing: Res<vale_client::lua::api::keyboard::KeyboardFocus>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    cursor: Res<crate::pick::Cursor>,
    drawn: Query<(
        &vale_client::render::doodads::Doodad,
        &GlobalTransform,
        &bevy::camera::primitives::Aabb,
    )>,
) {
    if !state.editing() || wants.wants_keyboard_input() || typing.active {
        return;
    }
    if !keys.pressed(KeyCode::ControlLeft) && !keys.pressed(KeyCode::ControlRight) {
        return;
    }
    let (copy, paste, duplicate) = (
        keys.just_pressed(KeyCode::KeyC),
        keys.just_pressed(KeyCode::KeyV),
        keys.just_pressed(KeyCode::KeyD),
    );
    if !copy && !paste && !duplicate {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    match *tool {
        Tool::Doodads => {
            if copy {
                board.doodads = copy_doodads(&doodads, &drawn);
                session.status = format!("copied {} doodads", board.doodads.len());
            }
            if duplicate && doodads.count() > 1 {
                let clip = copy_doodads(&doodads, &drawn);
                let anchor = doodads.at.as_ref().map(|at| {
                    Vec3::from(vale_assets::world::adt::placement_to_world(at.record.position))
                        + Vec3::X * APART
                });
                if let Some(anchor) = anchor {
                    let pasted = paste_doodads(session, &clip, anchor, "Duplicate doodads");
                    select_pasted_doodads(session, &mut doodads, &clip, pasted);
                }
            }
            if paste {
                let Some(anchor) = cursor.surface else {
                    session.status = "nothing under the pointer to paste onto".to_string();
                    return;
                };
                if board.doodads.is_empty() {
                    session.status = "no doodads have been copied".to_string();
                    return;
                }
                let clip = board.doodads.clone();
                let pasted = paste_doodads(session, &clip, anchor, "Paste doodads");
                select_pasted_doodads(session, &mut doodads, &clip, pasted);
            }
        }
        Tool::Wmos => {
            if copy {
                board.buildings = copy_buildings(&buildings);
                session.status = format!("copied {} WMOs", board.buildings.len());
            }
            if duplicate && buildings.count() > 1 {
                let clip = copy_buildings(&buildings);
                let anchor = buildings.at.as_ref().map(|at| {
                    Vec3::from(vale_assets::world::adt::placement_to_world(at.record.position))
                        + Vec3::X * APART
                });
                if let Some(anchor) = anchor {
                    let pasted = paste_buildings(session, &clip, anchor, "Duplicate WMOs");
                    select_pasted_buildings(session, &mut buildings, &clip, pasted);
                }
            }
            if paste {
                let Some(anchor) = cursor.surface else {
                    session.status = "nothing under the pointer to paste onto".to_string();
                    return;
                };
                if board.buildings.is_empty() {
                    session.status = "no WMOs have been copied".to_string();
                    return;
                }
                let clip = board.buildings.clone();
                let pasted = paste_buildings(session, &clip, anchor, "Paste WMOs");
                select_pasted_buildings(session, &mut buildings, &clip, pasted);
            }
        }
        _ => {}
    }
}

/// The doodad selection as a clip, the primary first.
fn copy_doodads(
    selection: &super::doodads::Selection,
    drawn: &Query<(
        &vale_client::render::doodads::Doodad,
        &GlobalTransform,
        &bevy::camera::primitives::Aabb,
    )>,
) -> Vec<ClipDoodad> {
    let Some(primary) = selection.at.as_ref() else {
        return Vec::new();
    };
    let pivot = Vec3::from(vale_assets::world::adt::placement_to_world(
        primary.record.position,
    ));
    selection
        .members()
        .map(|at| ClipDoodad {
            path: at.path.clone(),
            offset: Vec3::from(vale_assets::world::adt::placement_to_world(at.record.position))
                - pivot,
            record: at.record,
            radius: super::doodads::footprint(drawn, at.unique_id),
        })
        .collect()
}

/// The building selection as a clip, the primary first.
fn copy_buildings(selection: &super::wmos::Selection) -> Vec<ClipBuilding> {
    let Some(primary) = selection.at.as_ref() else {
        return Vec::new();
    };
    let pivot = Vec3::from(vale_assets::world::adt::placement_to_world(
        primary.record.position,
    ));
    selection
        .members()
        .map(|at| ClipBuilding {
            path: at.path.clone(),
            offset: Vec3::from(vale_assets::world::adt::placement_to_world(at.record.position))
                - pivot,
            record: at.record,
        })
        .collect()
}

/// Select what a paste wrote, the first row written as the primary, and say
/// what happened on the status line.
fn select_pasted_doodads(
    session: &mut EditSession,
    selection: &mut super::doodads::Selection,
    clip: &[ClipDoodad],
    pasted: Pasted,
) {
    selection.only(None);
    // The rows are in clip order with the skipped ones left out. The indices
    // are the ones `add_doodad` answered, which appends, so a later row in the
    // same tile does not move an earlier one.
    for (coord, index, unique_id) in &pasted.rows {
        let Some(record) = session.tiles.get(coord).and_then(|t| t.doodad_at(*index)) else {
            continue;
        };
        let path = session
            .tiles
            .get(coord)
            .and_then(|t| t.model_names().get(record.name_id as usize).cloned())
            .map(|name| vale_assets::world::m2::model_path(&name))
            .unwrap_or_default();
        let at = super::doodads::Selected {
            tile: *coord,
            index: *index,
            unique_id: *unique_id,
            path,
            record,
        };
        match selection.at.is_none() {
            true => selection.at = Some(at),
            false => selection.also.push(at),
        }
    }
    session.status = pasted_status(pasted.rows.len(), clip.len(), pasted.skipped, "doodads");
}

/// …and the same for buildings.
fn select_pasted_buildings(
    session: &mut EditSession,
    selection: &mut super::wmos::Selection,
    clip: &[ClipBuilding],
    pasted: Pasted,
) {
    selection.only(None);
    for (coord, index, unique_id) in &pasted.rows {
        let Some(record) = session.tiles.get(coord).and_then(|t| t.building_at(*index)) else {
            continue;
        };
        let path = session
            .tiles
            .get(coord)
            .and_then(|t| t.building_names().get(record.name_id as usize).cloned())
            .unwrap_or_default();
        let at = super::wmos::Selected {
            tile: *coord,
            index: *index,
            unique_id: *unique_id,
            path,
            record,
        };
        match selection.at.is_none() {
            true => selection.at = Some(at),
            false => selection.also.push(at),
        }
    }
    session.status = pasted_status(pasted.rows.len(), clip.len(), pasted.skipped, "WMOs");
}

/// What a paste did, in words.
fn pasted_status(written: usize, asked: usize, skipped: usize, noun: &str) -> String {
    match skipped {
        0 => format!("pasted {written} {noun}"),
        _ => format!(
            "pasted {written} of {asked} {noun}; {skipped} would land on a tile that is not open"
        ),
    }
}

/// The rectangle a drag on empty ground is drawing.
#[derive(Resource, Debug, Default)]
pub struct Marquee {
    /// Which tool began it, where the press was and where the pointer is, in
    /// logical window pixels. `None` when no press on empty ground is held.
    held: Option<Held>,
    /// A finished rectangle, waiting for the tool that began it.
    done: Option<Done>,
}

#[derive(Debug, Clone, Copy)]
struct Held {
    tool: Tool,
    from: Vec2,
    to: Vec2,
    /// Whether shift was down at the press, which makes the rectangle add to
    /// the group rather than replace it.
    adding: bool,
    /// Whether the pointer has travelled [`DRAG_PIXELS`] yet. Until it has,
    /// the press is a click on nothing and draws no rectangle.
    moving: bool,
}

/// A rectangle that has been released, as `(corner, corner, adding)`.
#[derive(Debug, Clone, Copy)]
pub struct Done {
    pub tool: Tool,
    pub rect: Rect,
    pub adding: bool,
}

impl Marquee {
    /// Start a rectangle at `at`. Called by a tool's press when it hit
    /// nothing.
    pub fn begin(&mut self, tool: Tool, at: Vec2, adding: bool) {
        self.held = Some(Held {
            tool,
            from: at,
            to: at,
            adding,
            moving: false,
        });
        self.done = None;
    }

    /// The rectangle `tool` began and the button released, if there is one.
    /// Taken, so it is acted on once.
    pub fn finished(&mut self, tool: Tool) -> Option<Done> {
        match self.done {
            Some(done) if done.tool == tool => self.done.take(),
            _ => None,
        }
    }

    /// Whether a rectangle is being drawn, for anything that would otherwise
    /// act on the same drag.
    pub fn drawing(&self) -> bool {
        self.held.is_some_and(|held| held.moving)
    }

    /// Whether a press on empty ground is held at all, drawn or not yet.
    pub fn pressed(&self) -> bool {
        self.held.is_some()
    }
}

/// Follow the pointer while a rectangle is held, and finish it on release.
fn track(
    mut marquee: ResMut<Marquee>,
    buttons: Res<ButtonInput<MouseButton>>,
    tool: Res<Tool>,
    windows: Query<&Window>,
) {
    let Some(mut held) = marquee.held else {
        return;
    };
    // A tool switched mid-drag drops it: the rectangle would select in a tool
    // that did not begin it.
    if held.tool != *tool {
        marquee.held = None;
        return;
    }
    if let Some(at) = windows.single().ok().and_then(|w| w.cursor_position()) {
        held.to = at;
        if held.from.distance(at) >= DRAG_PIXELS {
            held.moving = true;
        }
    }
    if buttons.pressed(MouseButton::Left) {
        marquee.held = Some(held);
        return;
    }
    marquee.held = None;
    if held.moving {
        marquee.done = Some(Done {
            tool: held.tool,
            rect: Rect::from_corners(held.from, held.to),
            adding: held.adding,
        });
    }
}

/// Draw the rectangle over the world.
fn paint(mut contexts: EguiContexts, marquee: Res<Marquee>) -> Result {
    let Some(held) = marquee.held.filter(|held| held.moving) else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("group-marquee"),
    ));
    // Held in the window's logical pixels, which the tools compare with
    // `on_screen`; painted in egui's points, which are larger by the zoom.
    let zoom = ctx.zoom_factor().max(0.01);
    let rect = egui::Rect::from_two_pos(
        egui::pos2(held.from.x / zoom, held.from.y / zoom),
        egui::pos2(held.to.x / zoom, held.to.y / zoom),
    );
    let accent = crate::ui::theme::ACCENT;
    painter.rect_filled(rect, 0.0, accent.gamma_multiply(0.12));
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, accent),
        egui::StrokeKind::Inside,
    );
    Ok(())
}

/// Where the world camera draws a world point, in logical window pixels, or
/// `None` behind the camera.
///
/// The projection answers in physical pixels, because the world camera draws
/// into a target sized in physical pixels; the pointer and the rectangle are
/// logical. The same division `vale_client::ui::boxes` makes.
pub fn on_screen(
    camera: &Camera,
    eye: &GlobalTransform,
    window: &Window,
    world: Vec3,
) -> Option<Vec2> {
    let drawn = Vec3::from(axes::to_bevy(world.to_array()));
    // In front of the camera only: a point behind it projects to a mirrored
    // place on the screen.
    let forward = eye.forward();
    if (drawn - eye.translation()).dot(*forward) <= 0.0 {
        return None;
    }
    let pixels = camera.world_to_viewport(eye, drawn).ok()?;
    Some(pixels / window.scale_factor())
}

/// The camera and window [`on_screen`] needs, from the queries a tool holds.
pub fn view<'a>(
    camera: &'a Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    windows: &'a Query<&Window>,
) -> Option<(&'a Camera, &'a GlobalTransform, &'a Window)> {
    let (camera, eye) = camera.single().ok()?;
    let window = windows.single().ok()?;
    Some((camera, eye, window))
}

/// Where a spawn moved from `from` towards `to` stands: at `to` across the
/// ground, and at the height above the terrain it had at `from`.
///
/// For the other members of a group of spawns, whose primary follows the
/// surface under the pointer and who have no pointer of their own. Keeping the
/// height above the terrain is what keeps a creature on an inn's upper floor on
/// the upper floor, and one on a hillside on the hillside. Where either point
/// has no open terrain the height is `to`'s, which is the primary's change.
pub fn carried(session: &EditSession, from: Vec3, to: Vec3) -> Vec3 {
    let terrain = |at: Vec3| super::doodads::ground_height(session, at.x, at.y);
    match (terrain(from), terrain(to)) {
        (Some(under_from), Some(under_to)) => Vec3::new(to.x, to.y, under_to + (from.z - under_from)),
        _ => to,
    }
}

/// A turn of `degrees` about `axis`, a unit vector in the world's own axes.
pub fn turn_about(axis: Vec3, degrees: f32) -> Quat {
    Quat::from_axis_angle(axis.normalize_or(Vec3::Z), degrees.to_radians())
}

/// A point carried round `pivot` by `turn`.
pub fn orbit(pivot: Vec3, point: Vec3, turn: Quat) -> Vec3 {
    pivot + turn * (point - pivot)
}

/// A placement's three angles (the file's, in degrees) after `turn` is applied
/// to the placement in the world.
///
/// `placement_euler_to_world` gives the turns as `M = Rz(z) · Ry(y) · Rx(x)`,
/// which is glam's `EulerRot::ZYX`. The group's turn goes on the outside,
/// because it is about a world axis, and the product is taken apart again the
/// same way.
pub fn turn_record(rotation: [f32; 3], turn: Quat) -> [f32; 3] {
    let [x, y, z] =
        vale_assets::world::adt::placement_euler_to_world(rotation).map(f32::to_radians);
    let was = Quat::from_euler(EulerRot::ZYX, z, y, x);
    let (z, y, x) = (turn * was).normalize().to_euler(EulerRot::ZYX);
    vale_assets::world::adt::placement_euler_from_world(
        [x, y, z].map(|radians| radians.to_degrees().rem_euclid(360.0)),
    )
}

/// What `Ctrl+C` took: the placements, each relative to the primary's origin.
#[derive(Resource, Debug, Default)]
pub struct Clipboard {
    pub doodads: Vec<ClipDoodad>,
    pub buildings: Vec<ClipBuilding>,
}

/// One copied doodad.
#[derive(Debug, Clone)]
pub struct ClipDoodad {
    /// The model, resolved (`.m2`) as the selection holds it.
    pub path: String,
    /// Where it stood relative to the primary, in the world's own axes.
    pub offset: Vec3,
    pub record: Placement,
    /// How far its drawn footprint reaches, for the `MCRF` references a new
    /// row needs. See `vale_edit::adt::place::AdtFile::add_doodad`.
    pub radius: f32,
}

/// One copied building.
#[derive(Debug, Clone)]
pub struct ClipBuilding {
    pub path: String,
    pub offset: Vec3,
    pub record: Building,
}

/// The id a new placement gets: the next one in the minted range, above every
/// id the open tiles hold. The placer's own rule — see `place::commit`.
fn next_minted(session: &EditSession) -> u32 {
    session
        .tiles
        .values()
        .filter_map(|tile| tile.highest_minted_id())
        .max()
        .map(|had| had.saturating_add(1))
        .unwrap_or(MINTED_BASE)
}

/// What a paste wrote: where each new row went, and how many were left out
/// because the tile under them was not open.
#[derive(Debug, Default)]
pub struct Pasted {
    pub rows: Vec<((u32, u32), usize, u32)>,
    pub skipped: usize,
}

/// Put copies of `clip` into the tiles, with the primary's origin at `anchor`.
///
/// Every member keeps its offset from the primary, its turn and its size. The
/// height is the anchor's plus the offset it had, so a group copied off a
/// hillside keeps its own shape rather than being flattened onto a slope that
/// is not there.
///
/// One undo entry across every tile written. Each tile is published and asked
/// for again, so what is drawn comes from the file, collision hulls and lamps
/// included; since `terrain::swap` a re-read is not visible.
pub fn paste_doodads(
    session: &mut EditSession,
    clip: &[ClipDoodad],
    anchor: Vec3,
    label: &str,
) -> Pasted {
    let mut pasted = Pasted::default();
    let mut unique_id = next_minted(session);
    let mut before: std::collections::HashMap<(u32, u32), Placements> = Default::default();
    for item in clip {
        let at = anchor + item.offset;
        let coord = vale_assets::tile_for_position(at.x, at.y);
        let Some(tile) = session.tiles.get_mut(&coord) else {
            pasted.skipped += 1;
            continue;
        };
        before
            .entry(coord)
            .or_insert_with(|| Placements::capture(tile));
        let record = Placement {
            name_id: tile.name_model(&item.path),
            unique_id,
            position: vale_assets::world::adt::placement_from_world(at.to_array()),
            ..item.record
        };
        let index = tile.add_doodad(record, item.radius);
        pasted.rows.push((coord, index, unique_id));
        unique_id = unique_id.saturating_add(1);
    }
    record_placements(session, before, label);
    pasted
}

/// …and the same for buildings. The box moves with the building, as a drag
/// moves it — see `wmos::set_world_position`.
pub fn paste_buildings(
    session: &mut EditSession,
    clip: &[ClipBuilding],
    anchor: Vec3,
    label: &str,
) -> Pasted {
    let mut pasted = Pasted::default();
    let mut unique_id = next_minted(session);
    let mut before: std::collections::HashMap<(u32, u32), Placements> = Default::default();
    for item in clip {
        let at = anchor + item.offset;
        let coord = vale_assets::tile_for_position(at.x, at.y);
        let Some(tile) = session.tiles.get_mut(&coord) else {
            pasted.skipped += 1;
            continue;
        };
        before
            .entry(coord)
            .or_insert_with(|| Placements::capture(tile));
        let mut record = Building {
            name_id: tile.name_building(&item.path),
            unique_id,
            ..item.record
        };
        super::wmos::set_world_position(&mut record, &item.record, at.to_array());
        let index = tile.add_building(record);
        pasted.rows.push((coord, index, unique_id));
        unique_id = unique_id.saturating_add(1);
    }
    record_placements(session, before, label);
    pasted
}

/// One undo entry holding every tile's placement lists before and after, and
/// each tile published and asked for again.
fn record_placements(
    session: &mut EditSession,
    before: std::collections::HashMap<(u32, u32), Placements>,
    label: &str,
) {
    if before.is_empty() {
        return;
    }
    session.history.begin(label);
    for (coord, before) in before {
        let key = session.key(coord);
        let Some(tile) = session.tiles.get(&coord) else {
            continue;
        };
        let edit = Edit::Placements {
            before: Box::new(before),
            after: Box::new(Placements::capture(tile)),
        };
        session.history.record(&key, [edit]);
        session.publish(coord);
        session.stale.insert(coord);
    }
    session.history.end();
}

/// Take rows out of their tiles, as one undo entry.
///
/// `rows` is `(tile, index)`. Within a tile they are removed from the highest
/// index down, because removing a row renumbers every row above it.
pub fn remove_rows(
    session: &mut EditSession,
    rows: &[((u32, u32), usize)],
    buildings: bool,
    label: &str,
) -> usize {
    let mut by_tile: std::collections::BTreeMap<(u32, u32), Vec<usize>> = Default::default();
    for &(coord, index) in rows {
        by_tile.entry(coord).or_default().push(index);
    }
    let mut removed = 0;
    let mut before: std::collections::HashMap<(u32, u32), Placements> = Default::default();
    for (coord, mut indices) in by_tile {
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        before.insert(coord, Placements::capture(tile));
        indices.sort_unstable();
        indices.dedup();
        for index in indices.into_iter().rev() {
            let gone = match buildings {
                true => tile.remove_building(index).is_some(),
                false => tile.remove_doodad(index).is_some(),
            };
            removed += usize::from(gone);
        }
    }
    record_placements(session, before, label);
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A group turn of zero changes nothing, and a quarter turn about up adds
    /// ninety degrees to the turn about up and leaves the two leans alone.
    #[test]
    fn a_turn_about_up_adds_to_the_turn_about_up() {
        let rotation = vale_assets::world::adt::placement_euler_from_world([0.0, 0.0, 30.0]);
        let same = turn_record(rotation, Quat::IDENTITY);
        let world = vale_assets::world::adt::placement_euler_to_world(same);
        assert!((world[2] - 30.0).abs() < 1e-3, "{world:?}");

        let turned = turn_record(rotation, turn_about(Vec3::Z, 90.0));
        let world = vale_assets::world::adt::placement_euler_to_world(turned);
        assert!((world[2] - 120.0).abs() < 1e-3, "{world:?}");
        assert!(world[0].abs() < 1e-3 || (world[0] - 360.0).abs() < 1e-3, "{world:?}");
        assert!(world[1].abs() < 1e-3 || (world[1] - 360.0).abs() < 1e-3, "{world:?}");
    }

    /// The turn applied to a record is the turn applied to the model. The
    /// placement matrix of the turned record equals the group's turn times the
    /// original matrix, for a record that already leans. This is what keeps a
    /// tilted rock in a group upright relative to its neighbours when the group
    /// is turned about another axis.
    #[test]
    fn a_turned_record_draws_as_the_turn_times_the_model() {
        let rotation =
            vale_assets::world::adt::placement_euler_from_world([12.0, -20.0, 75.0]);
        let turn = turn_about(Vec3::new(0.3, 0.8, 0.5).normalize(), 40.0);
        let turned = turn_record(rotation, turn);

        let matrix = |rotation| {
            Mat4::from_cols_array(&vale_assets::world::adt::placement_matrix(
                [0.0; 3], rotation, 1.0,
            ))
        };
        let want = Mat4::from_quat(turn) * matrix(rotation);
        let got = matrix(turned);
        for (a, b) in want.to_cols_array().iter().zip(got.to_cols_array()) {
            assert!((a - b).abs() < 1e-4, "{want:?}\n{got:?}");
        }
    }

    /// A member carried round the pivot by a quarter turn about up ends a
    /// quarter of the way round, at the same distance and height.
    #[test]
    fn an_orbit_keeps_distance_and_height() {
        let pivot = Vec3::new(100.0, 50.0, 10.0);
        let point = pivot + Vec3::new(5.0, 0.0, 2.0);
        let moved = orbit(pivot, point, turn_about(Vec3::Z, 90.0));
        assert!((moved - (pivot + Vec3::new(0.0, 5.0, 2.0))).length() < 1e-4, "{moved:?}");
    }
}
