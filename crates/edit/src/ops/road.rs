//! A road: a run of points, the ground graded along it, and up to two
//! textures painted along it.
//!
//! ## The centre line
//!
//! The points are joined by straight segments, or by a centripetal
//! Catmull-Rom curve through them ([`Road::curved`]). Either way the line is
//! sampled every [`SPACING`] yards into a [`Path`], and everything else is
//! measured from that: how far a vertex or a texel is from the centre, and the
//! road's height beside it. The centripetal form is used because the uniform
//! one loops and overshoots where two points are much closer together than
//! their neighbours.
//!
//! ## The height along it
//!
//! [`Heights::Points`] takes each point's own height and runs the curve
//! through them, which is [`super::grade`] over more than two points.
//! [`Heights::Ground`] follows the ground under the centre line, averaged
//! over a window of yards, so a road over rolling ground keeps the roll and
//! loses the bumps.
//!
//! ## Across it
//!
//! Within [`Road::width`] of the centre the ground is moved to the road's
//! height, plus [`Road::crown`] at the centre falling to nothing at the
//! edge, plus [`Road::offset`]. Across [`Road::shoulder`] past that it blends
//! back to the ground along [`Road::falloff`]. The ends are round: a point
//! past the last one measures its distance to the last one.
//!
//! ## The textures
//!
//! [`Painting`] paints a surface down the middle and, optionally, a verge in
//! a band along both sides of it, verge first. The verge stops where the
//! surface is solid, so a chunk the road crosses wholly does not spend one of
//! its four layers on a texture nobody can see. Both edges can be made
//! irregular ([`Painting::ragged`]) and the surface worn into patches
//! ([`Surface::wear`]).

use super::{reshade, wobble, Edit, Falloff, PaintBrush, Painted, Working};
use crate::adt::{heights, AdtFile};
use vale_assets::world::adt::CHUNK_SIZE;

/// Yards between two samples of the centre line.
///
/// A quarter of the 4.17 yards between vertices would be wasted, and the
/// half-yard texel does not need it either: the distance to a segment is
/// exact between samples, so the spacing decides only how closely the
/// samples follow a curve.
pub const SPACING: f32 = 1.0;

/// One of the points a road runs through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// World x and y.
    pub at: [f32; 2],
    /// The road's height here, for [`Heights::Points`].
    pub height: f32,
}

/// Where the road's height along its length comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Heights {
    /// The points' own heights, joined along the line.
    Points,
    /// The ground under the centre line, averaged over `smoothing` yards.
    Ground { smoothing: f32 },
}

/// The shape a road gives the ground.
#[derive(Debug, Clone, PartialEq)]
pub struct Road {
    pub points: Vec<Point>,
    /// A curve through the points rather than straight segments.
    pub curved: bool,
    pub heights: Heights,
    /// Yards from the centre line to the edge of the part moved fully to the
    /// road's height.
    pub width: f32,
    /// Yards past the width over which the road blends back to the ground.
    /// Zero leaves a vertical face along both sides.
    pub shoulder: f32,
    /// The curve the blend across the shoulder follows.
    pub falloff: Falloff,
    /// How far toward the road the ground moves, 0 to 1.
    pub strength: f32,
    /// Yards the centre stands above the edges, so the surface sheds water.
    /// Negative makes a dished road.
    pub crown: f32,
    /// Yards added to the road's height everywhere: negative sinks the road
    /// into the ground, positive makes a causeway.
    pub offset: f32,
}

impl Default for Road {
    fn default() -> Self {
        Road {
            points: Vec::new(),
            curved: true,
            heights: Heights::Ground { smoothing: 30.0 },
            // About the width of the shipped roads in Elwynn and Westfall.
            width: 4.0,
            shoulder: 6.0,
            falloff: Falloff::Smooth,
            strength: 1.0,
            crown: 0.0,
            offset: 0.0,
        }
    }
}

/// The centre line, sampled, with the road's height at each sample.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    /// World x, y and the road's height, every [`SPACING`] yards or closer.
    pub samples: Vec<[f32; 3]>,
}

/// Where a point is relative to the centre line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Near {
    /// Yards from the centre line.
    pub across: f32,
    /// The road's height at the nearest point of the centre line.
    pub height: f32,
}

impl Road {
    /// Whether the points make a road: at least two, not all in one place.
    pub fn is_ready(&self) -> bool {
        self.points.windows(2).any(|pair| distance(pair[0].at, pair[1].at) > 1.0e-3)
    }

    /// **The centre line through the points**, sampled, with the points'
    /// heights joined along it. Empty for fewer than two points.
    pub fn centre(&self) -> Vec<[f32; 3]> {
        // Two points in one place are one point: the curve would divide by
        // the distance between them.
        let mut points: Vec<[f32; 3]> = Vec::new();
        for point in &self.points {
            let here = [point.at[0], point.at[1], point.height];
            if points.last().is_none_or(|last| distance([last[0], last[1]], point.at) > 1.0e-3) {
                points.push(here);
            }
        }
        if points.len() < 2 {
            return Vec::new();
        }
        let mut samples = vec![points[0]];
        for i in 0..points.len() - 1 {
            let (b, c) = (points[i], points[i + 1]);
            let steps = (distance([b[0], b[1]], [c[0], c[1]]) / SPACING).ceil().max(1.0) as usize;
            if !self.curved || points.len() == 2 {
                for step in 1..=steps {
                    samples.push(lerp3(b, c, step as f32 / steps as f32));
                }
                continue;
            }
            // The neighbours either side, mirrored past the ends so the curve
            // leaves the first point and reaches the last one heading
            // straight for its neighbour.
            let a = match i {
                0 => mirror(b, c),
                _ => points[i - 1],
            };
            let d = match points.get(i + 2) {
                Some(&d) => d,
                None => mirror(c, b),
            };
            for step in 1..=steps {
                samples.push(centripetal(a, b, c, d, step as f32 / steps as f32));
            }
        }
        samples
    }

    /// **The path the road follows**, with its height from [`Road::heights`].
    ///
    /// `ground` is the ground's height at a point, or `None` where no tile is
    /// open; a sample with no ground under it keeps the points' height.
    /// `None` when the points do not make a road.
    pub fn path(&self, ground: impl Fn(f32, f32) -> Option<f32>) -> Option<Path> {
        let mut samples = self.centre();
        if samples.len() < 2 {
            return None;
        }
        if let Heights::Ground { smoothing } = self.heights {
            let under: Vec<f32> = samples
                .iter()
                .map(|s| ground(s[0], s[1]).unwrap_or(s[2]))
                .collect();
            // A running mean over the window, twice, which is a triangle
            // filter: one pass of a box leaves corners where a bump entered
            // and left the window.
            let half = ((smoothing / SPACING) / 2.0).round().max(0.0) as usize;
            let smoothed = mean_over(&mean_over(&under, half), half);
            for (sample, height) in samples.iter_mut().zip(smoothed) {
                sample[2] = height;
            }
        }
        Some(Path { samples })
    }

    /// How far from the centre line the road moves the ground.
    pub fn reach(&self) -> f32 {
        self.width.max(0.0) + self.shoulder.max(0.0)
    }

    /// **What a vertex at `(x, y)` currently at `height` becomes.** The
    /// identity beyond the shoulder.
    pub fn applied(&self, path: &Path, segments: &[usize], x: f32, y: f32, height: f32) -> f32 {
        let Some(near) = path.near_among(segments, x, y) else {
            return height;
        };
        let width = self.width.max(0.0);
        let weight = match near.across <= width {
            true => 1.0,
            false => match self.shoulder > 1.0e-6 {
                true => self.falloff.weight((near.across - width) / self.shoulder),
                false => 0.0,
            },
        };
        if weight == 0.0 {
            return height;
        }
        // The crown is a parabola across the width: its full height at the
        // centre and nothing at the edge, where the shoulder takes over.
        let crown = match width > 1.0e-6 && near.across < width {
            true => {
                let t = near.across / width;
                self.crown * (1.0 - t * t)
            }
            false => 0.0,
        };
        let wanted = near.height + self.offset + crown;
        height + (wanted - height) * weight * self.strength.clamp(0.0, 1.0)
    }

    /// **Write the road's shape into one tile**, returning what it changed:
    /// one [`Edit::Heights`] per chunk that moved, and the shading recomputed
    /// beside them, as [`super::grade::Grade::write_into`] does.
    pub fn write_heights(&self, path: &Path, tile: &mut AdtFile) -> Vec<Edit> {
        let reach = self.reach();
        let mut edits = Vec::new();
        for index in 0..tile.chunks.len() {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            let segments = path.segments_near(chunk_box(origin), reach);
            if segments.is_empty() {
                continue;
            }
            let before = heights::heights(chunk);
            if before.is_empty() {
                continue;
            }
            let mut after = before.clone();
            for (i, height) in after.iter_mut().enumerate() {
                let (dx, dy) = heights::vertex_offset(i);
                *height = self.applied(path, &segments, origin[0] - dx, origin[1] - dy, *height);
            }
            if after == before {
                continue;
            }
            let Some(open) = tile.chunk_mut(index) else {
                continue;
            };
            heights::set_heights(open, &after);
            edits.push(Edit::Heights {
                chunk: index,
                before,
                after,
            });
        }
        if !edits.is_empty() {
            reshade(tile, &mut edits);
        }
        edits
    }
}

impl Path {
    /// How long the centre line is, in yards.
    pub fn length(&self) -> f32 {
        self.samples
            .windows(2)
            .map(|pair| distance([pair[0][0], pair[0][1]], [pair[1][0], pair[1][1]]))
            .sum()
    }

    /// The road's height at its two ends.
    pub fn ends(&self) -> Option<(f32, f32)> {
        Some((self.samples.first()?[2], self.samples.last()?[2]))
    }

    /// **The steepest the road gets**, as rise over run, measured over runs
    /// of at least `over` yards so one sample's rounding does not count as a
    /// slope.
    pub fn steepest(&self, over: f32) -> f32 {
        let mut steepest = 0.0f32;
        let mut start = 0;
        let mut run = 0.0;
        for i in 1..self.samples.len() {
            let (a, b) = (self.samples[i - 1], self.samples[i]);
            run += distance([a[0], a[1]], [b[0], b[1]]);
            if run >= over {
                let rise = (self.samples[i][2] - self.samples[start][2]).abs();
                steepest = steepest.max(rise / run);
                start = i;
                run = 0.0;
            }
        }
        // A road shorter than the window is measured whole.
        if steepest == 0.0 && self.samples.len() >= 2 {
            let length = self.length();
            if length > 1.0e-3 {
                let (first, last) = self.ends().unwrap_or_default();
                steepest = (last - first).abs() / length;
            }
        }
        steepest
    }

    /// The segments, by the index of their first sample, whose box comes
    /// within `reach` of the box `[min x, min y, max x, max y]`.
    pub fn segments_near(&self, area: [f32; 4], reach: f32) -> Vec<usize> {
        (0..self.samples.len().saturating_sub(1))
            .filter(|&i| {
                let (a, b) = (self.samples[i], self.samples[i + 1]);
                a[0].min(b[0]) - reach <= area[2]
                    && a[0].max(b[0]) + reach >= area[0]
                    && a[1].min(b[1]) - reach <= area[3]
                    && a[1].max(b[1]) + reach >= area[1]
            })
            .collect()
    }

    /// Where a point is relative to the whole centre line.
    pub fn near(&self, x: f32, y: f32) -> Option<Near> {
        let all: Vec<usize> = (0..self.samples.len().saturating_sub(1)).collect();
        self.near_among(&all, x, y)
    }

    /// …relative to the given segments only, which a caller has already
    /// narrowed to the ones that can matter. `None` for no segments.
    pub fn near_among(&self, segments: &[usize], x: f32, y: f32) -> Option<Near> {
        let mut best: Option<Near> = None;
        for &i in segments {
            let (a, b) = (self.samples[i], self.samples[i + 1]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let run = dx * dx + dy * dy;
            let t = match run > 1.0e-9 {
                true => (((x - a[0]) * dx + (y - a[1]) * dy) / run).clamp(0.0, 1.0),
                false => 0.0,
            };
            let (px, py) = (a[0] + dx * t, a[1] + dy * t);
            let across = ((x - px) * (x - px) + (y - py) * (y - py)).sqrt();
            if best.is_none_or(|best| across < best.across) {
                best = Some(Near {
                    across,
                    height: a[2] + (b[2] - a[2]) * t,
                });
            }
        }
        best
    }

    /// The box the road and `reach` either side of it cover, as
    /// `[min x, min y, max x, max y]`.
    pub fn bounds(&self, reach: f32) -> [f32; 4] {
        let mut out = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for s in &self.samples {
            out[0] = out[0].min(s[0] - reach);
            out[1] = out[1].min(s[1] - reach);
            out[2] = out[2].max(s[0] + reach);
            out[3] = out[3].max(s[1] + reach);
        }
        out
    }
}

/// The texture down the middle of a road.
#[derive(Debug, Clone, PartialEq)]
pub struct Surface {
    /// The tileset's archive path.
    pub texture: String,
    /// `MCLY` `effectId` for a layer this adds. Zero grows nothing, which is
    /// what a road usually wants.
    pub effect_id: u32,
    /// Yards from the centre line to where the texture starts to fade.
    pub width: f32,
    /// Yards over which it fades out past that.
    pub softness: f32,
    /// How visible it is where it is solid, 0 to 1.
    pub opacity: f32,
    /// How much of it is worn away in patches, 0 to 1. Zero is solid.
    pub wear: f32,
}

/// The texture in a band along both sides of the surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Verge {
    pub texture: String,
    pub effect_id: u32,
    /// Yards the band reaches past the surface's width.
    pub width: f32,
    /// Yards over which its outer edge fades.
    pub softness: f32,
    pub opacity: f32,
}

/// The textures a road paints.
#[derive(Debug, Clone, PartialEq)]
pub struct Painting {
    pub surface: Option<Surface>,
    pub verge: Option<Verge>,
    /// Yards by which the edges wander in and out of a straight line.
    pub ragged: f32,
    /// Yards across one wander of the edges, and one patch of wear.
    pub grain: f32,
    /// When a chunk already has four textures, give the road's texture the
    /// layer that shows least, if it shows almost nothing. See
    /// [`PaintBrush::reuse_hidden`].
    pub reuse_hidden: bool,
}

impl Default for Painting {
    fn default() -> Self {
        Painting {
            surface: None,
            verge: None,
            ragged: 0.6,
            grain: 6.0,
            reuse_hidden: true,
        }
    }
}

impl Painting {
    /// How far from the centre line anything is painted.
    pub fn reach(&self) -> f32 {
        let surface = self
            .surface
            .as_ref()
            .map(|s| s.width.max(0.0) + s.softness.max(0.0))
            .unwrap_or(0.0);
        let verge = self
            .verge
            .as_ref()
            .map(|v| self.surface_width() + v.width.max(0.0) + v.softness.max(0.0))
            .unwrap_or(0.0);
        surface.max(verge) + self.ragged.abs()
    }

    /// The surface's width, or nothing without a surface: a verge alone is
    /// painted from the centre line.
    fn surface_width(&self) -> f32 {
        self.surface.as_ref().map(|s| s.width.max(0.0)).unwrap_or(0.0)
    }

    /// The distance from the centre line the edges are measured with: the
    /// true distance, moved in or out by the raggedness at this place.
    fn wandered(&self, near: Near, x: f32, y: f32) -> f32 {
        match self.ragged.abs() > 1.0e-6 {
            true => near.across + self.ragged * wobble(x, y, self.grain.max(0.5)),
            false => near.across,
        }
    }

    /// **The surface's share of a point**, 0 to 1.
    pub fn surface_weight(&self, path: &Path, segments: &[usize], x: f32, y: f32) -> f32 {
        let Some(surface) = &self.surface else {
            return 0.0;
        };
        let Some(near) = path.near_among(segments, x, y) else {
            return 0.0;
        };
        edge(self.wandered(near, x, y), surface.width, surface.softness)
    }

    /// **The verge's share of a point**, 0 to 1: out to its own outer edge,
    /// and nothing where the surface over it is solid.
    pub fn verge_weight(&self, path: &Path, segments: &[usize], x: f32, y: f32) -> f32 {
        let Some(verge) = &self.verge else {
            return 0.0;
        };
        let Some(near) = path.near_among(segments, x, y) else {
            return 0.0;
        };
        let d = self.wandered(near, x, y);
        let width = self.surface_width();
        // Inside the surface's width the surface is solid and covers it. From
        // there out the verge is solid under the surface's fade, so the two
        // meet without the old ground showing between them.
        if self.surface.is_some() && d <= width {
            return 0.0;
        }
        edge(d, width + verge.width.max(0.0), verge.softness)
    }

    /// **Paint the road's textures into one tile**: the verge, then the
    /// surface over it. The edits are [`Edit::Paint`], one per chunk per
    /// texture that changed.
    pub fn write_into(&self, path: &Path, tile: &mut AdtFile) -> Painted {
        let mut painted = Painted::default();
        let reach = self.reach();
        let chunks: Vec<(usize, Vec<usize>)> = (0..tile.chunks.len())
            .filter_map(|index| {
                let origin = tile.chunk(index)?.head().position();
                let segments = path.segments_near(chunk_box(origin), reach);
                (!segments.is_empty()).then_some((index, segments))
            })
            .collect();
        if chunks.is_empty() {
            return painted;
        }
        let brush = |texture: &str, effect_id: u32, opacity: f32, wear: f32| PaintBrush {
            texture: texture.to_string(),
            effect_id,
            opacity: opacity.clamp(0.0, 1.0),
            // One call moves every texel all the way: see
            // [`PaintBrush::paint_where`].
            strength: 1.0,
            density: (1.0 - wear).clamp(0.0, 1.0),
            grain: self.grain.max(0.5),
            reuse_hidden: self.reuse_hidden,
            ..PaintBrush::default()
        };
        if let Some(verge) = &self.verge {
            let brush = brush(&verge.texture, verge.effect_id, verge.opacity, 0.0);
            let mut working = Working::default();
            for (index, segments) in &chunks {
                let step = brush.paint_where(tile, &mut working, &[*index], 1.0, |x, y| {
                    self.verge_weight(path, segments, x, y)
                });
                merge(&mut painted, step);
            }
        }
        if let Some(surface) = &self.surface {
            let brush = brush(&surface.texture, surface.effect_id, surface.opacity, surface.wear);
            let mut working = Working::default();
            for (index, segments) in &chunks {
                let step = brush.paint_where(tile, &mut working, &[*index], 1.0, |x, y| {
                    self.surface_weight(path, segments, x, y)
                });
                merge(&mut painted, step);
            }
        }
        painted.full.sort_unstable();
        painted.full.dedup();
        painted
    }
}

/// Add one paint step's results to the road's.
fn merge(into: &mut Painted, step: Painted) {
    into.edits.extend(step.edits);
    into.full.extend(step.full);
    into.based.extend(step.based);
    into.base.extend(step.base);
    into.absent.extend(step.absent);
    into.reused.extend(step.reused);
}

/// 1 out to `inner`, falling smoothly to 0 across `soft` past it.
fn edge(d: f32, inner: f32, soft: f32) -> f32 {
    if d <= inner {
        return 1.0;
    }
    if soft <= 1.0e-6 {
        return 0.0;
    }
    let t = ((d - inner) / soft).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// A chunk's square as `[min x, min y, max x, max y]`. A chunk runs back from
/// its origin along both axes; see [`heights::vertex_offset`].
fn chunk_box(origin: [f32; 3]) -> [f32; 4] {
    [origin[0] - CHUNK_SIZE, origin[1] - CHUNK_SIZE, origin[0], origin[1]]
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// `about` reflected through `at`.
fn mirror(at: [f32; 3], about: [f32; 3]) -> [f32; 3] {
    [2.0 * at[0] - about[0], 2.0 * at[1] - about[1], 2.0 * at[2] - about[2]]
}

/// The centripetal Catmull-Rom curve between `b` and `c`, at `u` from 0 at
/// `b` to 1 at `c`, with `a` and `d` the points either side. Parameterised by
/// the square root of the distance in x and y (Barry and Goldman's form).
fn centripetal(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], u: f32) -> [f32; 3] {
    let knot = |p: [f32; 3], q: [f32; 3]| distance([p[0], p[1]], [q[0], q[1]]).sqrt().max(1.0e-4);
    let t0 = 0.0;
    let t1 = t0 + knot(a, b);
    let t2 = t1 + knot(b, c);
    let t3 = t2 + knot(c, d);
    let t = t1 + (t2 - t1) * u;
    let mix = |p: [f32; 3], q: [f32; 3], from: f32, to: f32| lerp3(p, q, (t - from) / (to - from));
    let a1 = mix(a, b, t0, t1);
    let a2 = mix(b, c, t1, t2);
    let a3 = mix(c, d, t2, t3);
    let b1 = mix(a1, a2, t0, t2);
    let b2 = mix(a2, a3, t1, t3);
    mix(b1, b2, t1, t2)
}

/// The mean of each value and `half` values either side, with the window cut
/// short at the ends.
fn mean_over(values: &[f32], half: usize) -> Vec<f32> {
    if half == 0 {
        return values.to_vec();
    }
    (0..values.len())
        .map(|i| {
            let from = i.saturating_sub(half);
            let to = (i + half).min(values.len() - 1);
            values[from..=to].iter().sum::<f32>() / (to - from + 1) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, y: f32, height: f32) -> Point {
        Point { at: [x, y], height }
    }

    fn straight() -> Road {
        Road {
            points: vec![point(0.0, 0.0, 0.0), point(100.0, 0.0, 50.0)],
            curved: false,
            heights: Heights::Points,
            width: 5.0,
            shoulder: 0.0,
            ..Road::default()
        }
    }

    #[test]
    fn a_straight_road_runs_linearly_between_its_points() {
        let road = straight();
        let path = road.path(|_, _| None).expect("two points");
        assert!((path.length() - 100.0).abs() < 1e-3);
        for (x, want) in [(0.0, 0.0), (25.0, 12.5), (50.0, 25.0), (100.0, 50.0)] {
            let near = path.near(x, 3.0).expect("a path");
            assert!((near.height - want).abs() < 1e-3, "at {x}: {} is not {want}", near.height);
            assert!((near.across - 3.0).abs() < 1e-3);
        }
        assert!((path.steepest(4.0) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn the_ground_is_moved_inside_the_width_and_left_outside_it() {
        let road = straight();
        let path = road.path(|_, _| None).unwrap();
        let all: Vec<usize> = (0..path.samples.len() - 1).collect();
        assert!((road.applied(&path, &all, 50.0, 4.0, 0.0) - 25.0).abs() < 1e-3);
        assert_eq!(road.applied(&path, &all, 50.0, 6.0, 0.0), 0.0);
    }

    /// The ends are round: a point beyond the last one, within the width of
    /// it, takes the last one's height.
    #[test]
    fn the_ends_are_round() {
        let road = straight();
        let path = road.path(|_, _| None).unwrap();
        let near = path.near(103.0, 0.0).unwrap();
        assert!((near.across - 3.0).abs() < 1e-3);
        assert!((near.height - 50.0).abs() < 1e-3);
    }

    #[test]
    fn the_shoulder_blends_back_to_the_ground() {
        let road = Road {
            shoulder: 10.0,
            falloff: Falloff::Linear,
            ..straight()
        };
        let path = road.path(|_, _| None).unwrap();
        let all: Vec<usize> = (0..path.samples.len() - 1).collect();
        // Half way across the shoulder, half way from the ground to the road.
        let half = road.applied(&path, &all, 50.0, 10.0, 0.0);
        assert!((half - 12.5).abs() < 1e-2, "{half}");
        assert_eq!(road.applied(&path, &all, 50.0, 15.5, 0.0), 0.0);
    }

    #[test]
    fn the_crown_is_highest_at_the_centre_and_gone_at_the_edge() {
        let road = Road {
            crown: 1.0,
            ..straight()
        };
        let path = road.path(|_, _| None).unwrap();
        let all: Vec<usize> = (0..path.samples.len() - 1).collect();
        assert!((road.applied(&path, &all, 50.0, 0.0, 0.0) - 26.0).abs() < 1e-3);
        assert!((road.applied(&path, &all, 50.0, 4.999, 0.0) - 25.0).abs() < 1e-2);
    }

    /// A curve passes through every point, with its height.
    #[test]
    fn a_curve_passes_through_its_points() {
        let road = Road {
            points: vec![
                point(0.0, 0.0, 0.0),
                point(40.0, 30.0, 10.0),
                point(80.0, 0.0, 0.0),
                point(120.0, 40.0, 5.0),
            ],
            curved: true,
            heights: Heights::Points,
            ..Road::default()
        };
        let path = road.path(|_, _| None).unwrap();
        for p in &road.points {
            let near = path.near(p.at[0], p.at[1]).unwrap();
            assert!(near.across < 1e-2, "{:?} is {} off the curve", p.at, near.across);
            assert!((near.height - p.height).abs() < 1e-2);
        }
        // …and it is longer than the straight segments, since it bends.
        let straight: f32 = road.points.windows(2).map(|w| distance(w[0].at, w[1].at)).sum();
        assert!(path.length() > straight);
    }

    /// Following the ground keeps the slope and takes the bumps out.
    #[test]
    fn following_the_ground_smooths_it() {
        let road = Road {
            heights: Heights::Ground { smoothing: 20.0 },
            ..straight()
        };
        // A slope with a two-yard bump every ten yards.
        let ground = |x: f32, _: f32| Some(x * 0.2 + if (x as i32) % 10 < 5 { 2.0 } else { 0.0 });
        let path = road.path(ground).unwrap();
        let middle = path.near(50.0, 0.0).unwrap().height;
        // The slope's own height there is 10, and the bump averages to 1.
        assert!((middle - 11.0).abs() < 0.6, "{middle}");
    }

    #[test]
    fn the_verge_is_beside_the_surface_and_not_under_it() {
        let painting = Painting {
            surface: Some(Surface {
                texture: "a".into(),
                effect_id: 0,
                width: 4.0,
                softness: 2.0,
                opacity: 1.0,
                wear: 0.0,
            }),
            verge: Some(Verge {
                texture: "b".into(),
                effect_id: 0,
                width: 3.0,
                softness: 1.0,
                opacity: 1.0,
            }),
            ragged: 0.0,
            ..Painting::default()
        };
        let path = straight().path(|_, _| None).unwrap();
        let all: Vec<usize> = (0..path.samples.len() - 1).collect();
        let at = |y: f32| {
            (
                painting.surface_weight(&path, &all, 50.0, y),
                painting.verge_weight(&path, &all, 50.0, y),
            )
        };
        assert_eq!(at(0.0), (1.0, 0.0), "the centre is the surface alone");
        let (surface, verge) = at(5.0);
        assert!(surface > 0.0 && surface < 1.0, "the surface fades: {surface}");
        assert_eq!(verge, 1.0, "under the fade, the verge is solid");
        assert_eq!(at(3.5).1, 0.0, "under the solid surface, no verge");
        assert_eq!(at(6.5), (0.0, 1.0), "past the surface, the verge");
        assert_eq!(at(8.5), (0.0, 0.0), "past the verge, nothing");
        assert!((painting.reach() - 8.0).abs() < 1e-3);
    }

    #[test]
    fn one_point_is_not_a_road() {
        let road = Road {
            points: vec![point(0.0, 0.0, 0.0), point(0.0, 0.0, 5.0)],
            ..Road::default()
        };
        assert!(!road.is_ready());
        assert!(road.path(|_, _| None).is_none());
    }
}
