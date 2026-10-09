//! The measuring tool: what is at a point, and how far apart two points are.
//!
//! A click on the terrain reports where it is, and two clicks report the
//! distance between them. It changes nothing.
//!
//! ## What a point reports
//!
//! A click takes [`crate::pick::Cursor::surface`], the nearer of the ground and
//! the collision hull of whatever stands on it, which is the surface the
//! creature and game-object tools place on. [`probe`] then reads the open tile
//! at that point:
//!
//! ```text
//! position     world x, y, z, as `.gps` prints them
//! on           the ground, or a building or model standing on it
//! ground       the terrain height under the point, and the height above it
//! tile, chunk  the ADT and which of its 256 MCNK chunks
//! area         the chunk's AreaTable id, named in the panel
//! slope        the ground's slope in degrees, against the 50-degree limit a
//!              character can climb
//! water        the liquid over the point: kind, surface height, depth, flags
//! flags        a hole in the ground, or the chunk's impassable bit
//! ```
//!
//! The panel shows the same report live for the point under the pointer, so
//! a click is only needed to keep a point or to measure from it.
//!
//! ## What two points report
//!
//! [`span`]: the straight distance, the distance along the ground plane, the
//! height difference, the slope of the line, and the facing from the first
//! point to the second in radians, which is the value a `creature` or
//! `gameobject` row's `orientation` column takes.
//!
//! ## Gestures
//!
//! A click sets the first point, the next the second, and a third starts
//! again from the first, as the grading tool does. `Escape` clears both.
//! `--measure "<x,y>[;<x,y>]"` keeps the points from the command line, on the
//! ground at those positions, once their tiles are open.

use bevy::prelude::*;

use crate::pick::Cursor;
use crate::session::EditSession;
use vale_assets::world::wmo::Liquid;

/// The steepest slope a character can walk up in 1.12, in degrees. The
/// grading tool's panel marks the same limit.
pub const CLIMB_LIMIT: f32 = 50.0;

/// What the measuring tool is holding.
#[derive(Resource, Debug, Clone, Default)]
pub struct Measuring {
    /// The two points, in the order they were clicked.
    pub first: Option<Probe>,
    pub second: Option<Probe>,
}

/// What stands at the point that was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum On {
    Ground,
    /// A building, doodad or game object's collision hull, nearer to the eye
    /// than the ground.
    Model,
}

/// What is at one point of the open map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probe {
    /// World axes: x north, y west, z up.
    pub at: Vec3,
    pub on: On,
    /// The terrain height at `at`'s x and y, or `None` off the open tiles and
    /// over a hole.
    pub ground: Option<f32>,
    pub tile: (u32, u32),
    /// Which of the tile's 256 chunks, when the tile is open.
    pub chunk: Option<usize>,
    /// The chunk's `AreaTable` id.
    pub area: Option<u32>,
    /// The ground's slope, in degrees from level.
    pub slope: Option<f32>,
    /// The liquid over the point: kind, surface height and the cell's flag
    /// nibble.
    pub water: Option<(Liquid, f32, u8)>,
    /// The ground under the point is cut out of the mesh.
    pub hole: bool,
    /// The chunk carries `MCNK`'s impassable flag.
    pub impassable: bool,
}

/// Between two points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    /// Straight-line distance, in yards.
    pub distance: f32,
    /// Distance in the x-y plane, in yards.
    pub horizontal: f32,
    /// The second point's height minus the first's.
    pub rise: f32,
    /// The line's angle from level, in degrees; positive when it climbs.
    pub slope: f32,
    /// The facing from the first point to the second, in radians from +x
    /// toward +y, in `[0, 2π)`: the `orientation` a spawn at the first point
    /// needs to face the second.
    pub facing: f32,
}

impl Measuring {
    /// Take the next point. The third click starts again.
    pub fn pick(&mut self, probe: Probe) {
        match (self.first.is_some(), self.second.is_some()) {
            (false, _) => self.first = Some(probe),
            (true, false) => self.second = Some(probe),
            (true, true) => {
                self.first = Some(probe);
                self.second = None;
            }
        }
    }

    pub fn clear(&mut self) {
        self.first = None;
        self.second = None;
    }

    /// The span between the two points, when both are picked.
    pub fn span(&self) -> Option<Span> {
        Some(span(self.first?.at, self.second?.at))
    }
}

/// The distance and direction from `a` to `b`, both in world axes.
pub fn span(a: Vec3, b: Vec3) -> Span {
    let d = b - a;
    let horizontal = d.truncate().length();
    let facing = d.y.atan2(d.x).rem_euclid(std::f32::consts::TAU);
    Span {
        distance: d.length(),
        horizontal,
        rise: d.z,
        slope: d.z.atan2(horizontal).to_degrees(),
        facing,
    }
}

/// Slope in degrees from level, for a unit normal in world axes.
pub fn slope_of(normal: [f32; 3]) -> f32 {
    normal[2].clamp(-1.0, 1.0).acos().to_degrees()
}

/// Read what is at `at` from the open tiles. `ground_hit` is where the
/// pointer's ray met the ground, if it did; `at` is taken to be on the ground
/// when the two are the same point.
pub fn probe(session: &EditSession, at: Vec3, ground_hit: Option<Vec3>) -> Probe {
    use vale_edit::adt::{area, heights, impass};

    let tile = vale_assets::tile_for_position(at.x, at.y);
    let open = session.tiles.get(&tile);
    let on = match ground_hit {
        Some(ground) if ground.distance(at) < 0.01 => On::Ground,
        _ => On::Model,
    };
    let ground = open.and_then(|t| heights::height_at(t, at.x, at.y));
    let chunk = open.and_then(|t| heights::chunk_at(t, at.x, at.y));
    let hole = ground.is_none() && open.and_then(|t| heights::solid_height_at(t, at.x, at.y)).is_some();
    Probe {
        at,
        on,
        ground,
        tile,
        chunk,
        area: open.zip(chunk).and_then(|(t, c)| area::area(t, c)),
        slope: open.and_then(|t| heights::normal_at(t, at.x, at.y)).map(slope_of),
        water: chunk.and_then(|c| super::water::surface_at(session, tile, c, at.x, at.y)),
        hole,
        impassable: open.zip(chunk).is_some_and(|(t, c)| impass::impassable(t, c)),
    }
}

/// The liquid's name, as the water panel uses it.
pub fn liquid_name(kind: Liquid) -> &'static str {
    match kind {
        Liquid::Water => "water",
        Liquid::Ocean => "ocean",
        Liquid::Magma => "magma",
        Liquid::Slime => "slime",
    }
}

/// The text the Copy buttons put on the clipboard.
pub fn position_text(at: Vec3) -> String {
    format!("{:.3} {:.3} {:.3}", at.x, at.y, at.z)
}

/// vmangos' teleport command for a point: `.go xyz x y z [mapid]`
/// (`HandleGoXYZCommand`, `src/game/Commands/TeleportCommands.cpp:859`).
pub fn go_command(at: Vec3, map: u32) -> String {
    format!(".go xyz {} {map}", position_text(at))
}

/// One line for the status bar about the span, set on the second click.
pub fn said(span: &Span) -> String {
    format!(
        "distance {:.1} yd ({:.1} yd horizontal, {:+.1} yd vertical)",
        span.distance, span.horizontal, span.rise
    )
}

/// Take the point under the pointer on a click, and clear on `Escape`.
#[allow(clippy::too_many_arguments)]
fn aim(
    mut measuring: ResMut<Measuring>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<super::Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Res<Cursor>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
) {
    if *tool != super::Tool::Measure || !state.editing() {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    if keys.just_pressed(KeyCode::Escape) && !wants.wants_keyboard_input() {
        measuring.clear();
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) || !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    let Some(at) = cursor.surface else {
        return;
    };
    measuring.pick(probe(session, at, cursor.ground));
    if let Some(span) = measuring.span() {
        session.status = said(&span);
    }
}

/// The two points and the line between them; while only the first is picked,
/// the line runs to the pointer.
///
/// Ghosted, as every other instrument the pointer aims at is: the line
/// between two ground points passes under any ground that bulges between
/// them, and the part under the ground is drawn faint rather than hidden. The
/// numbers in the panel say whether the straight line is above or below the
/// ground. See `crate::marks`.
fn draw(
    mut marks: ResMut<crate::marks::Marks>,
    measuring: Res<Measuring>,
    tool: Res<super::Tool>,
    cursor: Res<Cursor>,
) {
    if *tool != super::Tool::Measure {
        return;
    }
    let bevy = |p: Vec3| vale_client::render::axes::to_bevy(p.to_array());
    let first_colour = Color::srgb(0.35, 0.9, 0.45);
    let second_colour = Color::srgb(0.95, 0.65, 0.25);
    for (probe, colour) in [(measuring.first, first_colour), (measuring.second, second_colour)] {
        let Some(probe) = probe else { continue };
        // A sphere and a two-yard pole, so the point shows from above and
        // from the side.
        marks.sphere(bevy(probe.at), MARK, colour, crate::marks::Look::Ghosted);
        marks.line(bevy(probe.at), bevy(probe.at + Vec3::Z * POLE), colour, crate::marks::Look::Ghosted);
        // A point on a model also has a line down to the ground under it.
        if let (On::Model, Some(ground)) = (probe.on, probe.ground) {
            let below = Vec3::new(probe.at.x, probe.at.y, ground);
            marks.line(bevy(probe.at), bevy(below), colour.with_alpha(0.6), crate::marks::Look::Ghosted);
        }
    }
    let Some(first) = measuring.first else { return };
    let end = measuring.second.map(|p| p.at).or(cursor.surface);
    if let Some(end) = end {
        marks.line(bevy(first.at), bevy(end), Color::srgb(0.95, 0.85, 0.35), crate::marks::Look::Ghosted);
    }
}

/// The radius of a point's sphere, and the height of its pole, in yards.
const MARK: f32 = 0.8;
const POLE: f32 = 2.0;

/// `--measure`: keep the points the command line names, once every one of
/// them is over an open tile with ground under it. Runs until it has.
fn scripted(
    mut measuring: ResMut<Measuring>,
    mut session: Option<ResMut<EditSession>>,
    args: Res<crate::Args>,
    mut done: Local<bool>,
) {
    let (Some(points), Some(session)) = (args.measure.as_ref(), session.as_mut()) else {
        return;
    };
    if *done {
        return;
    }
    let grounded: Option<Vec<Vec3>> = points
        .iter()
        .take(2)
        .map(|&(x, y)| {
            let tile = session.tiles.get(&vale_assets::tile_for_position(x, y))?;
            let z = vale_edit::adt::heights::height_at(tile, x, y)?;
            Some(Vec3::new(x, y, z))
        })
        .collect();
    let Some(grounded) = grounded else {
        return;
    };
    *done = true;
    measuring.clear();
    for at in grounded {
        measuring.pick(probe(session, at, Some(at)));
    }
    if let Some(span) = measuring.span() {
        session.status = said(&span);
        info!("--measure: {}; facing {:.4}", said(&span), span.facing);
    }
}

pub struct MeasureToolPlugin;

impl Plugin for MeasureToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Measuring>()
            .add_systems(Update, (scripted, aim, draw).chain().after(crate::pick::aim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    fn point(x: f32, y: f32, z: f32) -> Probe {
        Probe {
            at: Vec3::new(x, y, z),
            on: On::Ground,
            ground: Some(z),
            tile: (32, 48),
            chunk: None,
            area: None,
            slope: None,
            water: None,
            hole: false,
            impassable: false,
        }
    }

    #[test]
    fn picking_fills_two_points_then_starts_again() {
        let mut measuring = Measuring::default();
        measuring.pick(point(0.0, 0.0, 0.0));
        assert!(measuring.span().is_none());
        measuring.pick(point(3.0, 4.0, 0.0));
        assert_eq!(measuring.span().unwrap().distance, 5.0);
        measuring.pick(point(9.0, 9.0, 9.0));
        assert_eq!(measuring.first.unwrap().at, Vec3::splat(9.0));
        assert!(measuring.second.is_none());
        measuring.clear();
        assert!(measuring.first.is_none());
    }

    #[test]
    fn a_span_splits_into_across_and_up() {
        let s = span(Vec3::new(0.0, 0.0, 10.0), Vec3::new(3.0, 4.0, 22.0));
        assert!((s.distance - 13.0).abs() < 1e-4);
        assert!((s.horizontal - 5.0).abs() < 1e-4);
        assert_eq!(s.rise, 12.0);
        assert!((s.slope - 12f32.atan2(5.0).to_degrees()).abs() < 1e-3);
        // Down is a negative slope.
        assert!(span(Vec3::new(0.0, 0.0, 5.0), Vec3::new(5.0, 0.0, 0.0)).slope < 0.0);
    }

    /// The facing is the `orientation` convention: 0 faces +x (north) and a
    /// quarter turn faces +y (west).
    #[test]
    fn the_facing_is_the_orientation_a_spawn_would_need() {
        let facing = |x: f32, y: f32| span(Vec3::ZERO, Vec3::new(x, y, 0.0)).facing;
        assert!(facing(1.0, 0.0).abs() < 1e-5);
        assert!((facing(0.0, 1.0) - FRAC_PI_2).abs() < 1e-5);
        assert!((facing(-1.0, 0.0) - PI).abs() < 1e-5);
        // South-east of the first point is a facing past π, never negative.
        assert!((facing(0.0, -1.0) - 3.0 * FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn a_normal_gives_the_slope_from_level() {
        assert!(slope_of([0.0, 0.0, 1.0]).abs() < 1e-4);
        let (s, c) = 30f32.to_radians().sin_cos();
        assert!((slope_of([s, 0.0, c]) - 30.0).abs() < 1e-3);
    }

    #[test]
    fn the_go_command_is_the_one_vmangos_parses() {
        assert_eq!(
            go_command(Vec3::new(-9450.0, -50.25, 59.875), 0),
            ".go xyz -9450.000 -50.250 59.875 0"
        );
    }
}
