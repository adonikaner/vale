//! The road tool: points clicked on the ground, then one press that grades
//! the ground along them and paints the road's textures.
//!
//! The shape and the paint are `vale_edit::ops::road`'s, tested without a
//! renderer. What is here is the gestures, the preview and the apply.
//!
//! ## Gestures
//!
//! A click on the ground adds a point at the end of the road. A press on a
//! point drags it, and the point takes the ground's height where it is let
//! go. Shift and a click on a point removes it. Backspace removes the last
//! point and Enter applies. The right-click menu inserts a point into the
//! nearest segment and removes the nearest point.
//!
//! The points stay after an apply, so a road is adjusted by undoing it,
//! moving a point or a setting, and applying again.

use bevy::prelude::*;

use crate::pick::Cursor;
use crate::session::EditSession;
use vale_edit::ops::road::{Heights, Painting, Path, Point, Road, Surface, Verge};
use vale_edit::ops::Falloff;

/// Which of the road's two textures the tileset list sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Slot {
    #[default]
    Surface,
    Verge,
}

/// The road tool's points and settings.
#[derive(Resource, Debug, Clone)]
pub struct RoadTool {
    pub points: Vec<Point>,
    /// A curve through the points rather than straight segments.
    pub curved: bool,
    /// The height along the road from the ground under it, averaged over
    /// [`Self::smoothing`] yards; otherwise from the points' own heights.
    pub follow_ground: bool,
    pub smoothing: f32,
    /// Whether the apply moves the ground at all. Off paints only.
    pub shape_ground: bool,
    pub width: f32,
    pub shoulder: f32,
    pub falloff: Falloff,
    pub strength: f32,
    pub crown: f32,
    pub offset: f32,
    pub objects_follow: bool,
    /// Whether the apply paints the surface texture, and how.
    pub paint_surface: bool,
    pub surface: Surface,
    pub paint_verge: bool,
    pub verge: Verge,
    pub ragged: f32,
    pub grain: f32,
    pub reuse_hidden: bool,
    /// Which texture a click in the tileset list sets.
    pub slot: Slot,
    /// What the last apply said.
    pub said: String,
    /// The point being dragged, and the point under the pointer.
    pub dragging: Option<usize>,
    pub hovered: Option<usize>,
}

impl Default for RoadTool {
    fn default() -> Self {
        let road = Road::default();
        let painting = Painting::default();
        RoadTool {
            points: Vec::new(),
            curved: road.curved,
            follow_ground: true,
            smoothing: match road.heights {
                Heights::Ground { smoothing } => smoothing,
                Heights::Points => 30.0,
            },
            shape_ground: true,
            width: road.width,
            shoulder: road.shoulder,
            falloff: road.falloff,
            strength: road.strength,
            crown: road.crown,
            offset: road.offset,
            objects_follow: true,
            paint_surface: true,
            surface: Surface {
                texture: String::new(),
                effect_id: 0,
                width: road.width - 1.0,
                softness: 2.0,
                opacity: 1.0,
                wear: 0.0,
            },
            paint_verge: false,
            verge: Verge {
                texture: String::new(),
                effect_id: 0,
                width: 3.0,
                softness: 2.0,
                opacity: 1.0,
            },
            ragged: painting.ragged,
            grain: painting.grain,
            reuse_hidden: painting.reuse_hidden,
            slot: Slot::Surface,
            said: String::new(),
            dragging: None,
            hovered: None,
        }
    }
}

impl RoadTool {
    /// The shape these points and settings describe.
    pub fn road(&self) -> Road {
        Road {
            points: self.points.clone(),
            curved: self.curved,
            heights: match self.follow_ground {
                true => Heights::Ground {
                    smoothing: self.smoothing,
                },
                false => Heights::Points,
            },
            width: self.width,
            shoulder: self.shoulder,
            falloff: self.falloff,
            strength: self.strength,
            crown: self.crown,
            offset: self.offset,
        }
    }

    /// The textures these settings paint.
    pub fn painting(&self) -> Painting {
        Painting {
            surface: self.paint_surface.then(|| self.surface.clone()),
            verge: self.paint_verge.then(|| self.verge.clone()),
            ragged: self.ragged,
            grain: self.grain,
            reuse_hidden: self.reuse_hidden,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.road().is_ready()
    }

    /// How far a press may be from a point and still take it.
    fn grab(&self) -> f32 {
        (self.width * 0.75).clamp(2.0, 12.0)
    }

    /// The point within reach of `(x, y)`, nearest first.
    pub fn point_near(&self, x: f32, y: f32) -> Option<usize> {
        let grab = self.grab();
        self.points
            .iter()
            .enumerate()
            .map(|(i, p)| (i, (p.at[0] - x).hypot(p.at[1] - y)))
            .filter(|&(_, d)| d <= grab)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Add a point at the end of the road.
    pub fn push(&mut self, at: [f32; 2], height: f32) {
        self.points.push(Point { at, height });
    }

    /// Put a point into the straight segment between two points that passes
    /// nearest `(x, y)`. With fewer than two points, add it at the end.
    pub fn insert(&mut self, at: [f32; 2], height: f32) {
        let nearest = self
            .points
            .windows(2)
            .enumerate()
            .map(|(i, pair)| (i, to_segment(at, pair[0].at, pair[1].at)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match nearest {
            Some((i, _)) => self.points.insert(i + 1, Point { at, height }),
            None => self.push(at, height),
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.points.len() {
            self.points.remove(index);
        }
        self.dragging = None;
        self.hovered = None;
    }

    pub fn clear(&mut self) {
        self.points.clear();
        self.dragging = None;
        self.hovered = None;
    }

    /// Set the chosen slot's texture, from the tileset list.
    pub fn choose(&mut self, path: String, effect_id: u32) {
        match self.slot {
            Slot::Surface => {
                self.surface.texture = path;
                self.surface.effect_id = effect_id;
                self.paint_surface = true;
            }
            Slot::Verge => {
                self.verge.texture = path;
                self.verge.effect_id = effect_id;
                self.paint_verge = true;
            }
        }
    }
}

/// The distance from `p` to the segment from `a` to `b`.
fn to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let run = dx * dx + dy * dy;
    let t = match run > 1.0e-9 {
        true => (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / run).clamp(0.0, 1.0),
        false => 0.0,
    };
    (p[0] - a[0] - dx * t).hypot(p[1] - a[1] - dy * t)
}

/// The ground's height at a point, from the open tiles.
pub fn ground(session: &EditSession) -> impl Fn(f32, f32) -> Option<f32> + '_ {
    move |x, y| {
        let coord = vale_assets::tile_for_position(x, y);
        session
            .tiles
            .get(&coord)
            .and_then(|tile| vale_edit::adt::heights::height_at(tile, x, y))
    }
}

/// The road's path over the ground as it is now.
pub fn path(session: &EditSession, tool: &RoadTool) -> Option<Path> {
    tool.road().path(ground(session))
}

/// What an apply did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    /// Chunks whose heights moved.
    pub shaped: usize,
    /// Chunks whose textures changed.
    pub painted: usize,
    /// Chunks that already carry four textures, none of them the road's.
    pub full: usize,
    /// Tiles the road reaches that are not open, and were not changed.
    pub closed: usize,
}

impl Applied {
    /// One line for the status bar and the panel.
    pub fn said(&self) -> String {
        let mut parts = vec![match (self.shaped, self.painted) {
            (0, 0) => "nothing changed".to_string(),
            (s, 0) => format!("road: {} shaped", chunks(s)),
            (0, p) => format!("road: {} painted", chunks(p)),
            (s, p) => format!("road: {} shaped, {} painted", chunks(s), chunks(p)),
        }];
        if self.full > 0 {
            parts.push(format!(
                "{} already {} four textures and {} not painted",
                chunks(self.full),
                if self.full == 1 { "has" } else { "have" },
                if self.full == 1 { "was" } else { "were" },
            ));
        }
        if self.closed > 0 {
            parts.push(match self.closed {
                1 => "1 tile is not open and was not changed".to_string(),
                n => format!("{n} tiles are not open and were not changed"),
            });
        }
        parts.join("; ")
    }
}

/// "1 chunk" or "n chunks".
fn chunks(n: usize) -> String {
    match n {
        1 => "1 chunk".to_string(),
        n => format!("{n} chunks"),
    }
}

/// **Grade the ground along the road and paint it**, as one undo entry,
/// over every open tile it reaches.
pub fn apply(session: &mut EditSession, tool: &RoadTool) -> Result<Applied, String> {
    let road = tool.road();
    let painting = tool.painting();
    if painting.surface.as_ref().is_some_and(|s| s.texture.is_empty()) {
        return Err("choose a surface texture, or turn off \"Paint a surface\"".into());
    }
    if painting.verge.as_ref().is_some_and(|v| v.texture.is_empty()) {
        return Err("choose a verge texture, or turn off \"Paint a verge\"".into());
    }
    if !tool.shape_ground && painting.surface.is_none() && painting.verge.is_none() {
        return Err("nothing to do: shaping and both textures are off".into());
    }
    let Some(path) = path(session, tool) else {
        return Err("place at least two points".into());
    };

    let reach = match tool.shape_ground {
        true => road.reach().max(painting.reach()),
        false => painting.reach(),
    };
    let [x0, y0, x1, y1] = path.bounds(reach);
    let (tx0, ty0) = vale_assets::tile_for_position(x1, y1);
    let (tx1, ty1) = vale_assets::tile_for_position(x0, y0);
    let tiles: Vec<(u32, u32)> = (tx0.min(tx1)..=tx0.max(tx1))
        .flat_map(|x| (ty0.min(ty1)..=ty0.max(ty1)).map(move |y| (x, y)))
        .collect();

    let mut applied = Applied {
        closed: tiles.iter().filter(|c| !session.tiles.contains_key(*c)).count(),
        ..Applied::default()
    };
    session.history.begin("Road".to_string());

    if tool.shape_ground {
        // What stands on each tile, read before the ground moves, so it is
        // carried by as much as the ground under it. See
        // `vale_edit::ops::follow`.
        let mut standing = Vec::new();
        for &coord in &tiles {
            let key = session.key(coord);
            let Some(tile) = session.tiles.get_mut(&coord) else {
                continue;
            };
            if tool.objects_follow {
                standing.push((coord, vale_edit::ops::follow::standing(tile)));
            }
            let mut edits = road.write_heights(&path, tile);
            session.hold_locked(coord, &mut edits);
            if edits.is_empty() {
                continue;
            }
            let chunks: Vec<usize> = edits.iter().filter_map(vale_edit::ops::Edit::chunk).collect();
            applied.shaped += edits
                .iter()
                .filter(|e| matches!(e, vale_edit::ops::Edit::Heights { .. }))
                .count();
            session.history.record(&key, edits);
            for chunk in chunks {
                session.touched(coord, chunk);
            }
        }
        super::terrain::carry(session, standing);
    }

    if painting.surface.is_some() || painting.verge.is_some() {
        for &coord in &tiles {
            let key = session.key(coord);
            let Some(tile) = session.tiles.get_mut(&coord) else {
                continue;
            };
            let painted = painting.write_into(&path, tile);
            applied.full += painted.full.len();
            if painted.edits.is_empty() {
                continue;
            }
            // The same route to the screen as a texture stroke's: a chunk
            // given a new texture rebuilds the tile, and one whose blends
            // moved is patched in place.
            let mut rebuild = false;
            let mut repainted: Vec<usize> = Vec::new();
            for edit in &painted.edits {
                match edit.changes_the_texture_set() {
                    true => rebuild = true,
                    false => repainted.extend(edit.painted()),
                }
            }
            let mut chunks: Vec<usize> = painted.edits.iter().filter_map(vale_edit::ops::Edit::chunk).collect();
            chunks.sort_unstable();
            chunks.dedup();
            applied.painted += chunks.len();
            session.history.record(&key, painted.edits);
            match rebuild {
                true => {
                    session.publish(coord);
                    session.stale.insert(coord);
                    session.unsaved.insert(coord);
                }
                false => {
                    for chunk in repainted {
                        session.repainted(coord, chunk);
                    }
                }
            }
        }
    }

    for &coord in &tiles {
        if session.tiles.contains_key(&coord) {
            session.publish(coord);
        }
    }
    session.history.end();
    Ok(applied)
}

/// Apply, and say what happened on the status line and in the panel.
pub fn apply_and_report(session: &mut EditSession, tool: &mut RoadTool) {
    tool.said = match apply(session, tool) {
        Ok(applied) => applied.said(),
        Err(why) => why,
    };
    session.status = tool.said.clone();
}

/// The gestures: add, drag and remove points, and the two keys.
#[allow(clippy::too_many_arguments)]
fn aim(
    mut tool: ResMut<RoadTool>,
    current: Res<super::Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Res<Cursor>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
    mut session: Option<ResMut<EditSession>>,
) {
    if *current != super::Tool::Road || !state.editing() {
        tool.dragging = None;
        tool.hovered = None;
        return;
    }
    let ground = cursor.ground;
    if buttons.just_released(MouseButton::Left) {
        tool.dragging = None;
    }
    if let (Some(i), Some(at)) = (tool.dragging, ground) {
        if buttons.pressed(MouseButton::Left) {
            if let Some(point) = tool.points.get_mut(i) {
                point.at = [at.x, at.y];
                point.height = at.z;
            }
            return;
        }
    }

    if !wants.wants_keyboard_input() {
        if keys.just_pressed(KeyCode::Backspace) {
            tool.points.pop();
            tool.hovered = None;
        }
        if keys.just_pressed(KeyCode::Enter) && tool.is_ready() {
            if let Some(session) = session.as_mut() {
                apply_and_report(session, &mut tool);
            }
        }
    }

    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);
    tool.hovered = match (in_world, ground) {
        (true, Some(at)) => tool.point_near(at.x, at.y),
        _ => None,
    };
    if !buttons.just_pressed(MouseButton::Left) || !in_world {
        return;
    }
    let Some(at) = ground else { return };
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    match (tool.hovered, shift) {
        (Some(i), true) => tool.remove(i),
        (Some(i), false) => tool.dragging = Some(i),
        (None, _) => tool.push([at.x, at.y], at.z),
    }
}

/// How far above the ground the preview's lines are drawn, so the ground
/// does not hide them.
const LIFT: f32 = 0.3;

/// Draw the points, the centre line at the road's height, and the edges of
/// the road and its shoulder on the ground. Ghosted, so a line a rise hides is
/// faint rather than gone; see `crate::marks`.
fn draw(
    mut marks: ResMut<crate::marks::Marks>,
    tool: Res<RoadTool>,
    current: Res<super::Tool>,
    cursor: Res<Cursor>,
    session: Option<Res<EditSession>>,
) {
    if *current != super::Tool::Road {
        return;
    }
    let Some(session) = session else { return };
    let to_bevy = |x: f32, y: f32, z: f32| vale_client::render::axes::to_bevy([x, y, z]);

    for (i, point) in tool.points.iter().enumerate() {
        let colour = match (tool.dragging == Some(i), tool.hovered == Some(i), i) {
            (true, ..) | (_, true, _) => Color::srgb(1.0, 0.85, 0.3),
            (.., 0) => Color::srgb(0.35, 0.9, 0.45),
            _ => Color::srgb(0.9, 0.9, 0.9),
        };
        marks.sphere(to_bevy(point.at[0], point.at[1], point.height), 1.2, colour, crate::marks::Look::Ghosted);
    }

    // The next point, from the last one to the pointer.
    if tool.dragging.is_none() && tool.hovered.is_none() {
        if let (Some(last), Some(at)) = (tool.points.last(), cursor.ground) {
            marks.line(
                to_bevy(last.at[0], last.at[1], last.height + LIFT),
                to_bevy(at.x, at.y, at.z + LIFT),
                Color::srgba(0.9, 0.9, 0.9, 0.5),
                crate::marks::Look::Ghosted,
            );
        }
    }

    let Some(path) = path(&session, &tool) else { return };
    let height = ground(&session);
    let centre: Vec<Vec3> = path
        .samples
        .iter()
        .map(|s| to_bevy(s[0], s[1], s[2] + tool.offset + tool.crown.max(0.0) + LIFT))
        .collect();
    marks.line_strip(centre, Color::srgb(0.95, 0.85, 0.35), crate::marks::Look::Ghosted);

    // The edges, offset along each sample's normal and dropped onto the
    // ground: the road's width solid, its shoulder and its paint fainter.
    let mut edges: Vec<(f32, Color)> = Vec::new();
    if tool.shape_ground {
        edges.push((tool.width, Color::srgb(0.95, 0.65, 0.25)));
        if tool.shoulder > 0.0 {
            edges.push((tool.width + tool.shoulder, Color::srgba(0.95, 0.65, 0.25, 0.4)));
        }
    }
    if tool.paint_surface {
        edges.push((tool.surface.width, Color::srgba(0.45, 0.75, 1.0, 0.8)));
    }
    if tool.paint_verge {
        let inner = match tool.paint_surface {
            true => tool.surface.width,
            false => 0.0,
        };
        edges.push((inner + tool.verge.width, Color::srgba(0.45, 1.0, 0.6, 0.6)));
    }
    let n = path.samples.len();
    for (offset, colour) in edges {
        for side in [-1.0f32, 1.0] {
            let line: Vec<Vec3> = (0..n)
                .filter_map(|i| {
                    let (a, b) = (path.samples[i.saturating_sub(1)], path.samples[(i + 1).min(n - 1)]);
                    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                    let len = dx.hypot(dy).max(1.0e-6);
                    let (nx, ny) = (-dy / len, dx / len);
                    let s = path.samples[i];
                    let (x, y) = (s[0] + nx * offset * side, s[1] + ny * offset * side);
                    let z = height(x, y)?;
                    Some(to_bevy(x, y, z + LIFT))
                })
                .collect();
            marks.line_strip(line, colour, crate::marks::Look::Ghosted);
        }
    }
}

pub struct RoadToolPlugin;

impl Plugin for RoadToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RoadTool>()
            .add_systems(Update, (aim, draw).chain().after(crate::pick::aim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_near_a_point_finds_it() {
        let mut tool = RoadTool::default();
        tool.push([0.0, 0.0], 0.0);
        tool.push([50.0, 0.0], 0.0);
        assert_eq!(tool.point_near(1.0, 1.0), Some(0));
        assert_eq!(tool.point_near(49.0, 0.0), Some(1));
        assert_eq!(tool.point_near(25.0, 0.0), None);
    }

    #[test]
    fn an_inserted_point_goes_into_the_nearest_segment() {
        let mut tool = RoadTool::default();
        tool.push([0.0, 0.0], 0.0);
        tool.push([50.0, 0.0], 0.0);
        tool.push([50.0, 50.0], 0.0);
        tool.insert([52.0, 25.0], 1.0);
        assert_eq!(tool.points.len(), 4);
        assert_eq!(tool.points[2].at, [52.0, 25.0]);
        tool.insert([10.0, -1.0], 1.0);
        assert_eq!(tool.points[1].at, [10.0, -1.0]);
    }

    #[test]
    fn the_list_sets_the_chosen_slot() {
        let mut tool = RoadTool {
            paint_verge: false,
            ..RoadTool::default()
        };
        tool.slot = Slot::Verge;
        tool.choose("Tileset\\A\\Grass.blp".into(), 7);
        assert_eq!(tool.verge.texture, "Tileset\\A\\Grass.blp");
        assert_eq!(tool.verge.effect_id, 7);
        assert!(tool.paint_verge, "choosing a verge texture turns the verge on");
        assert!(tool.surface.texture.is_empty());
    }

    #[test]
    fn the_settings_reach_the_road() {
        let tool = RoadTool {
            follow_ground: false,
            width: 7.0,
            crown: 0.5,
            paint_surface: false,
            ..RoadTool::default()
        };
        let road = tool.road();
        assert_eq!(road.heights, Heights::Points);
        assert_eq!(road.width, 7.0);
        assert_eq!(road.crown, 0.5);
        assert!(tool.painting().surface.is_none());
    }

    #[test]
    fn the_report_names_what_was_not_done() {
        let applied = Applied {
            shaped: 4,
            painted: 6,
            full: 2,
            closed: 1,
        };
        assert_eq!(
            applied.said(),
            "road: 4 chunks shaped, 6 chunks painted; 2 chunks already have four textures \
             and were not painted; 1 tile is not open and was not changed"
        );
    }
}
