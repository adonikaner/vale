//! The height brush and what every round brush shares: its shape, its
//! falloff, and the noise that roughens a stroke.
//!
//! ```text
//! Raise, Lower     add or subtract strength yards a second
//! Flatten          move toward one height, or a tilted plane through where
//!                  the stroke began
//! Smooth           move toward the mean height under the brush
//! Noise            move each vertex up or down by value noise
//! Blur             move toward a local mean, removing bumps narrower than
//!                  the scale and keeping wider ground
//! Terrace          move toward steps of a set height, measured from where
//!                  the stroke began
//! Erode            water drops wear gullies and settle ground lower down,
//!                  or slopes steeper than a talus angle collapse
//! Mountain         raise with ridges along the noise's crests
//! Crevice          dig a trench in the middle of the brush and pile the
//!                  ground on its rims
//! ```
//!
//! Every mode but Blur and Erode works chunk by chunk from the chunk's own
//! heights. Blur and Erode read each vertex's neighbours, so the caller reads
//! the ground into a [`Lattice`] first and strokes with
//! [`Brush::stroke_over`]; see [`super::lattice`].

use crate::adt::{heights, AdtFile};
use vale_assets::world::adt::{vertex_offset, CHUNK_SIZE};
use super::lattice::{Lattice, STEP};
use super::{Edit, reshade, vertices};

/// A brush's footprint, given as a distance from its centre rather than as a
/// mask.
///
/// A shape is the norm that measures `t`, the distance from the centre as a
/// fraction of the radius. Every [`Falloff`] is a function of `t` alone, so
/// any falloff combines with any shape, and a square brush still has a soft
/// edge. A mask would give a hard-edged square with a round falloff inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// The Euclidean norm. A circle.
    Circle,
    /// The Chebyshev norm, `max(|dx|, |dy|)`. A square aligned to the world
    /// axes, which are also the chunk grid's axes, so a plateau made with it
    /// has sides along the chunk grid.
    Square,
    /// The Manhattan norm, `|dx| + |dy|`. A diamond.
    Diamond,
}

impl Shape {
    /// How far an offset is from the centre, in this shape's own measure.
    pub fn distance(self, dx: f32, dy: f32) -> f32 {
        match self {
            Shape::Circle => (dx * dx + dy * dy).sqrt(),
            Shape::Square => dx.abs().max(dy.abs()),
            Shape::Diamond => dx.abs() + dy.abs(),
        }
    }

    /// The outline of the shape at one radius, as a closed run of points
    /// around the centre.
    ///
    /// Defined here, beside [`Shape::distance`], so the ring a tool draws is
    /// exactly where the brush reaches. The caller drops each point onto the
    /// ground.
    pub fn outline(self, radius: f32, segments: usize) -> Vec<(f32, f32)> {
        (0..segments)
            .map(|i| {
                let t = i as f32 / segments as f32;
                let angle = t * std::f32::consts::TAU;
                let (sin, cos) = angle.sin_cos();
                // The point on the shape's rim along this direction: the unit
                // direction divided by its own distance.
                let scale = radius / self.distance(cos, sin).max(1e-6);
                (cos * scale, sin * scale)
            })
            .collect()
    }

    /// The area inside the shape at radius 1.
    fn unit_area(self) -> f32 {
        match self {
            Shape::Circle => std::f32::consts::PI,
            Shape::Square => 4.0,
            Shape::Diamond => 2.0,
        }
    }
}

/// How a brush's strength falls off toward its edge.
///
/// Five curves over `t`, the distance from the centre as a fraction of the
/// radius, measured in the brush's [`Shape`]. Each is zero at and beyond the
/// rim, so a brush leaves the ground it does not reach unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Falloff {
    /// Full strength everywhere inside the radius. Leaves a visible rim.
    Flat,
    /// Straight from full at the centre to nothing at the rim.
    Linear,
    /// `(1 - t^2)^2`: flat in the middle and tangent to zero at the rim, so
    /// repeated strokes blend instead of leaving steps.
    Smooth,
    /// `(1 - t)^2`: concave, so the strength drops quickly away from the
    /// centre. Makes a peak or a well rather than a hill.
    Sharp,
    /// `sqrt(1 - t^2)`: convex, the profile of a sphere. Nearly full strength
    /// over most of the radius and a steep drop at the rim; a fuller mound
    /// than [`Falloff::Smooth`].
    Dome,
}

impl Falloff {
    /// The weight at `t` with an inner core held at full strength.
    ///
    /// `core` is the fraction of the radius that does not fall off; the curve
    /// runs over the rest. The brush preview draws its inner ring at the core.
    /// A core of zero is [`Falloff::weight`] exactly.
    ///
    /// The core is clamped below 1, because a core of the whole radius is
    /// [`Falloff::Flat`] and the division below would be by zero.
    pub fn over(self, t: f32, core: f32) -> f32 {
        let core = core.clamp(0.0, 0.95);
        match t < core {
            true => 1.0,
            false => self.weight((t - core) / (1.0 - core)),
        }
    }

    /// The weight at `t`, the distance from the centre as a fraction of the
    /// radius. Zero at and beyond the rim in every mode.
    pub fn weight(self, t: f32) -> f32 {
        if !(0.0..1.0).contains(&t) {
            return 0.0;
        }
        match self {
            Falloff::Flat => 1.0,
            Falloff::Linear => 1.0 - t,
            Falloff::Smooth => {
                let f = 1.0 - t * t;
                f * f
            }
            Falloff::Sharp => {
                let f = 1.0 - t;
                f * f
            }
            Falloff::Dome => (1.0 - t * t).max(0.0).sqrt(),
        }
    }
}

/// A signed value in `-1..1` that depends only on the position and varies
/// over `scale` yards. Used by [`Mode::Noise`], [`Mode::Mountain`], the paint
/// brush's grain and the road's ragged edge.
///
/// Value noise: the plane is divided into a lattice of `scale`-yard cells,
/// each corner is hashed to its own number, and a point takes the smooth blend
/// of the four corners around it. With cells wider than the 4.17 yards between
/// vertices, neighbouring vertices get similar numbers and the ground rolls.
/// Hashing the position directly gave every vertex an unrelated number, which
/// is white noise at the vertex spacing and looks jagged at any strength.
///
/// The blend uses the fifth-order ease. A linear blend creases the surface
/// along every lattice line, and the creases show as a grid.
///
/// The value depends on nothing but the position, so a held stroke deepens the
/// same bumps each frame instead of shaking the ground, and an undo takes back
/// one shape.
pub(crate) fn wobble(x: f32, y: f32, scale: f32) -> f32 {
    // A cell smaller than this is finer than the vertices can show, and zero
    // would divide by nothing.
    let scale = scale.max(0.5);
    let (u, v) = (x / scale, y / scale);
    let (cx, cy) = (u.floor(), v.floor());
    let (fx, fy) = (u - cx, v - cy);
    // Smootherstep: zero slope and zero curvature at both ends, so neither
    // the surface nor its shading shows the lattice.
    let ease = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (sx, sy) = (ease(fx), ease(fy));
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let corner = |i: f32, j: f32| lattice(cx + i, cy + j);
    mix(
        mix(corner(0.0, 0.0), corner(1.0, 0.0), sx),
        mix(corner(0.0, 1.0), corner(1.0, 1.0), sx),
        sy,
    )
}

/// One lattice corner's own number, in `-1..1`. The hash [`wobble`] is built
/// on.
fn lattice(i: f32, j: f32) -> f32 {
    hash_unit(i as i64 as u64, j as i64 as u64) * 2.0 - 1.0
}

/// A number in `0..1` from two integers, the same each time for the same two.
fn hash_unit(a: u64, b: u64) -> f32 {
    let mut hash = a.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    hash ^= b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    hash ^= hash >> 32;
    // The top 24 bits, which the mixing has moved most.
    (hash >> 40) as f32 / (1u32 << 24) as f32
}

/// How high a ridge is at a position, in `0..1`: 1 along the lines where
/// [`wobble`] crosses zero, falling away between them. Two octaves, the
/// second at half the scale and half the weight, so the ridges branch.
fn ridged(x: f32, y: f32, scale: f32) -> f32 {
    let one = 1.0 - wobble(x, y, scale).abs();
    let two = 1.0 - wobble(x, y, scale * 0.5).abs();
    (one * one + 0.5 * two * two) / 1.5
}

/// The share of a [`Mode::Crevice`] brush's radius that is trench; the rest,
/// out to the rim, is where the dug ground is piled.
const TRENCH: f32 = 0.5;

/// How deep [`Mode::Crevice`] digs at `t`: 1 at the centre, easing to 0 at
/// the trench's wall.
fn trench(t: f32) -> f32 {
    match t < TRENCH {
        true => 0.5 + 0.5 * (std::f32::consts::PI * t / TRENCH).cos(),
        false => 0.0,
    }
}

/// How high [`Mode::Crevice`] piles ground at `t`: 0 at the trench's wall and
/// at the rim, 1 half-way between.
fn rim(t: f32) -> f32 {
    match t > TRENCH && t < 1.0 {
        true => (std::f32::consts::PI * (t - TRENCH) / (1.0 - TRENCH)).sin(),
        false => 0.0,
    }
}

/// The factor that makes [`rim`] hold as much ground as [`trench`] takes out.
///
/// In every [`Shape`] the area at distance `t` grows as `t dt`, because a
/// shape is a norm and its outline at `t` is the outline at 1 scaled by `t`.
/// So the ratio of the two volumes is the ratio of the integrals of each
/// profile times `t`, whatever the shape.
fn rim_factor() -> f32 {
    const STEPS: usize = 256;
    let (mut dug, mut piled) = (0.0f32, 0.0f32);
    for k in 0..STEPS {
        let t = (k as f32 + 0.5) / STEPS as f32;
        dug += trench(t) * t;
        piled += rim(t) * t;
    }
    dug / piled
}

/// Which way [`Mode::Erode`] wears the ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Erosion {
    /// Drops of water run downhill, picking ground up where they speed up
    /// and putting it down where they slow. See [`Lattice::washed`].
    #[default]
    Water,
    /// Slopes steeper than [`Brush::talus`] collapse until they are not. See
    /// [`Lattice::slumped`].
    Thermal,
}

/// What a stroke does to the height under it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Add `strength` yards a second.
    Raise,
    /// Subtract them.
    Lower,
    /// Move toward one height, at `strength` of the remaining distance a
    /// second.
    Flatten { to: f32 },
    /// Move toward the mean height of the vertices the brush covers, which
    /// pulls a ridge down and a ditch up at once.
    Smooth,
    /// Move each vertex up or down by its own share of `strength` yards a
    /// second. The share is [`wobble`] at the vertex, which depends only on
    /// where the vertex is.
    Noise,
    /// Move toward the mean of the ground within [`Brush::scale`] yards, at
    /// `strength` of the remaining distance a second. Bumps narrower than the
    /// scale go; wider ground keeps its shape. Reads neighbours.
    Blur,
    /// Move toward steps [`Brush::step`] yards apart, at `strength` of the
    /// remaining distance a second. The steps are measured from `from`, the
    /// height where the stroke began, so a step is level with that point. See
    /// [`terraced`].
    Terrace { from: f32 },
    /// Wear the ground by [`Brush::erosion`]: drops of water, or collapsing
    /// slopes. Reads neighbours.
    Erode,
    /// Raise `strength` yards a second, more along the ridges of the noise
    /// at [`Brush::scale`] and less between them, by [`Brush::ridges`].
    Mountain,
    /// Dig a trench down the middle of the brush, `strength` yards a second at
    /// its centre, and pile [`Brush::lip`] of the dug ground on its rims. The
    /// profile is fixed, so the falloff and core do not apply.
    Crevice,
}

impl Mode {
    /// Whether [`Brush::strength`] is in yards a second for this mode. The
    /// other modes take a rate: a fraction of the remaining distance a second,
    /// passes a second, or drops a second.
    pub fn climbs(self) -> bool {
        matches!(self, Mode::Raise | Mode::Lower | Mode::Noise | Mode::Mountain | Mode::Crevice)
    }

    /// Whether this mode reads each vertex's neighbours, and so is stroked
    /// with [`Brush::stroke_over`] over a [`Lattice`].
    pub fn reads_neighbours(self) -> bool {
        matches!(self, Mode::Blur | Mode::Erode)
    }
}

/// A terrain brush.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brush {
    /// Yards.
    pub radius: f32,
    /// Yards a second for the modes [`Mode::climbs`] names. For Flatten,
    /// Smooth, Blur and Terrace, a fraction of the remaining distance a
    /// second. For Erode, drops a second for each vertex under the brush, or
    /// collapse passes a second.
    pub strength: f32,
    pub falloff: Falloff,
    /// How much of the radius is held at full strength before the falloff
    /// starts, as a fraction. See [`Falloff::over`]. Zero is the plain curve.
    pub core: f32,
    /// The footprint. See [`Shape`].
    pub shape: Shape,
    /// The size of a feature in yards: a bump's width for [`Mode::Noise`]
    /// and [`Mode::Mountain`] (see [`wobble`]), and the width of the bumps
    /// [`Mode::Blur`] removes.
    pub scale: f32,
    pub mode: Mode,
    /// Which way [`Mode::Flatten`] may move a vertex. See [`Only`].
    pub only: Only,
    /// The slope of the plane [`Mode::Flatten`] flattens toward, as the rise
    /// in yards per yard along world x and along world y. Zero is level, and
    /// then the target is the mode's own height everywhere. See
    /// [`Brush::tilted`].
    pub tilt: [f32; 2],
    /// The point the tilted plane passes through at the mode's own height.
    /// A caller sets it where the stroke begins. Unused while [`Self::tilt`]
    /// is zero.
    pub pivot: [f32; 2],
    /// The height of one step of [`Mode::Terrace`], in yards.
    pub step: f32,
    /// How much of each step of [`Mode::Terrace`] is level, 0 to 1. The rest
    /// is the riser. At 1 the steps are level with vertical risers; at 0 the
    /// ground is a smooth staircase with no level part.
    pub sharpness: f32,
    /// How much of [`Mode::Mountain`]'s height follows the ridges, 0 to 1. At
    /// 0 it raises like Raise; at 1 the ground between ridges does not rise.
    pub ridges: f32,
    /// The share of the ground [`Mode::Crevice`] digs out that it piles on
    /// the trench's rims, 0 to 1.
    pub lip: f32,
    /// How [`Mode::Erode`] wears the ground.
    pub erosion: Erosion,
    /// The steepest slope [`Erosion::Thermal`] leaves, in degrees.
    pub talus: f32,
}

/// Which way a flatten may move the ground.
///
/// A flatten toward a height fills what is under the height and cuts what is
/// over it. Each half alone is useful: fill makes a causeway across a hollow
/// and leaves the hills beside it; cut makes a shelf in a hillside and leaves
/// the valley.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Only {
    /// Up and down, toward the target.
    #[default]
    Both,
    /// Raise what is under the target and leave what is over it.
    Fill,
    /// Lower what is over the target and leave what is under it.
    Cut,
}

impl Default for Brush {
    fn default() -> Brush {
        Brush {
            radius: 20.0,
            strength: 8.0,
            falloff: Falloff::Smooth,
            core: 0.0,
            shape: Shape::Circle,
            // Six times the 4.17 yards between vertices, so the default reads
            // as rolling ground rather than as the vertex grid.
            scale: 25.0,
            mode: Mode::Raise,
            only: Only::Both,
            tilt: [0.0; 2],
            pivot: [0.0; 2],
            step: 8.0,
            sharpness: 0.6,
            ridges: 0.6,
            lip: 0.5,
            erosion: Erosion::Water,
            talus: 35.0,
        }
    }
}

/// The height [`Mode::Terrace`] moves `h` toward: steps `step` yards apart
/// measured from `from`, each level for `sharpness` of its height with a
/// riser between them.
///
/// The level part of a step is centred on the step's height and the riser on
/// the height half-way to the next, so no vertex moves more than half a step,
/// and ground already on a step stays there.
pub fn terraced(h: f32, from: f32, step: f32, sharpness: f32) -> f32 {
    if step <= 0.0 {
        return h;
    }
    let p = (h - from) / step;
    let below = p.floor();
    let f = p - below;
    // The riser's share of a step, centred half-way up it.
    let riser = 1.0 - sharpness.clamp(0.0, 1.0);
    let start = 0.5 - riser / 2.0;
    let rise = match riser <= 0.0 {
        true => match f < 0.5 {
            true => 0.0,
            false => 1.0,
        },
        false => {
            let s = ((f - start) / riser).clamp(0.0, 1.0);
            s * s * (3.0 - 2.0 * s)
        }
    };
    from + (below + rise) * step
}

impl Brush {
    /// Apply one step of the stroke at a world position, and return the edits it
    /// made.
    ///
    /// `seconds` is how long the stroke has been held since the last step, so a
    /// brush moves the ground at the same rate whatever the frame rate is.
    ///
    /// The normals of every chunk the heights changed are recomputed, and so are
    /// those of the chunks around them: a vertex on a chunk edge takes its
    /// gradient from the chunk next door, so raising ground next to a boundary
    /// changes the shading on both sides of it.
    pub fn stroke(
        &self,
        tile: &mut AdtFile,
        at: [f32; 2],
        seconds: f32,
        level: Option<f32>,
    ) -> Vec<Edit> {
        self.stroke_masked(tile, at, seconds, level, None)
    }

    /// The slope [`Self::tilt`] holds for a plane that rises `angle` degrees
    /// toward the compass bearing `toward`: 0 is north, which is world +x, and
    /// 90 is east, which is world -y.
    pub fn tilted(angle: f32, toward: f32) -> [f32; 2] {
        let rise = angle.clamp(0.0, 89.0).to_radians().tan();
        let bearing = toward.to_radians();
        [rise * bearing.cos(), -rise * bearing.sin()]
    }

    /// [`Self::stroke`] through a selection of this tile's vertices: with
    /// [`vertices::Mask::Protect`] the stroke leaves them where they are, and
    /// with [`vertices::Mask::Confine`] it moves only them. The shading is
    /// recomputed for every chunk the stroke reached either way, since a
    /// vertex's normal depends on its neighbours.
    ///
    /// `level` is what [`Mode::Smooth`] moves toward. A caller whose brush can
    /// reach more than one tile computes it over all of them with
    /// [`Brush::weighed`]; `None` takes it from this tile alone.
    pub fn stroke_masked(
        &self,
        tile: &mut AdtFile,
        at: [f32; 2],
        seconds: f32,
        level: Option<f32>,
        mask: Option<(&vertices::Selected, vertices::Mask)>,
    ) -> Vec<Edit> {
        // The mean Smooth moves toward is one height for the whole stroke. A
        // mean taken per chunk is a different height in every chunk and
        // leaves a step at each chunk boundary.
        let level = match (self.mode, level) {
            (Mode::Smooth, None) => {
                let (sum, weight) = self.weighed(tile, at);
                (weight > 0.0).then(|| (sum / weight) as f32)
            }
            _ => level,
        };
        self.apply(tile, at, seconds, level, None, mask)
    }

    /// One step of a mode that [`Mode::reads_neighbours`], toward `goal`,
    /// which [`Brush::goal`] made from the ground under the brush across
    /// every tile it reaches.
    ///
    /// The caller computes the goal once a step and strokes each tile with it,
    /// so a vertex on a tile's edge moves the same way in both files.
    pub fn stroke_over(
        &self,
        tile: &mut AdtFile,
        at: [f32; 2],
        seconds: f32,
        goal: &Lattice,
        mask: Option<(&vertices::Selected, vertices::Mask)>,
    ) -> Vec<Edit> {
        self.apply(tile, at, seconds, None, Some(goal), mask)
    }

    /// How far around the stroke's centre [`Brush::goal`] needs the ground,
    /// in yards: the radius, and the ground a vertex at the rim reads.
    pub fn reach(&self) -> f32 {
        match self.mode {
            // Two rounds of a box `scale` wide reach a scale to each side.
            Mode::Blur => self.radius + 2.0 * self.scale,
            // A drop runs out of the brush before it stops; ground past the
            // rim does not move, but the drop needs it to run on.
            Mode::Erode => self.radius + 8.0 * STEP,
            _ => self.radius,
        }
    }

    /// The heights a mode that [`Mode::reads_neighbours`] moves toward this
    /// step, from `ground`, the heights [`Brush::reach`] yards around `at`.
    ///
    /// `seed` picks where this step's drops of water start, and the caller
    /// changes it every step. Any other mode answers `ground` unchanged.
    pub fn goal(&self, ground: &Lattice, at: [f32; 2], seconds: f32, seed: u32) -> Lattice {
        match (self.mode, self.erosion) {
            (Mode::Blur, _) => ground.blurred(self.scale),
            (Mode::Erode, Erosion::Thermal) => ground.slumped(self.talus, self.strength * seconds),
            (Mode::Erode, Erosion::Water) => ground.washed(&self.drops(at, seconds, seed)),
            _ => ground.clone(),
        }
    }

    /// Where this step's drops of water start: about `strength` drops a
    /// second for every vertex under the brush, placed at random and more
    /// densely where the falloff is stronger.
    fn drops(&self, at: [f32; 2], seconds: f32, seed: u32) -> Vec<[f32; 2]> {
        /// The most drops one step starts, which bounds a step's cost on a
        /// large brush.
        const MOST: f32 = 20_000.0;
        let vertices = self.shape.unit_area() * self.radius * self.radius / (STEP * STEP);
        let wanted = (self.strength * seconds * vertices).min(MOST);
        let seed = seed as u64;
        let mut count = wanted.floor() as usize;
        if hash_unit(seed, u64::MAX) < wanted.fract() {
            count += 1;
        }
        let mut starts = Vec::with_capacity(count);
        // Each try is accepted with the falloff's weight at that point, so
        // tries run out before drops do only where the weights are tiny.
        for k in 0..(count as u64 * 8) {
            if starts.len() == count {
                break;
            }
            let dx = (hash_unit(seed, 3 * k) * 2.0 - 1.0) * self.radius;
            let dy = (hash_unit(seed, 3 * k + 1) * 2.0 - 1.0) * self.radius;
            let w = self.falloff.over(self.shape.distance(dx, dy) / self.radius, self.core);
            if hash_unit(seed, 3 * k + 2) < w {
                starts.push([at[0] + dx, at[1] + dy]);
            }
        }
        starts
    }

    /// The stroke for every mode: each chunk the brush reaches is moved, the
    /// mask applied, the edits recorded and the shading recomputed.
    fn apply(
        &self,
        tile: &mut AdtFile,
        at: [f32; 2],
        seconds: f32,
        level: Option<f32>,
        goal: Option<&Lattice>,
        mask: Option<(&vertices::Selected, vertices::Mask)>,
    ) -> Vec<Edit> {
        let touched = self.chunks_under(tile, at);
        let mut edits = Vec::new();
        for &index in &touched {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            let before = heights::heights(chunk);
            if before.is_empty() {
                continue;
            }
            let mut after = self.moved(&before, origin, at, seconds, level, goal);
            if let Some((selected, mask)) = mask {
                for (vertex, height) in after.iter_mut().enumerate() {
                    if !mask.lets(selected, index, vertex) {
                        *height = before[vertex];
                    }
                }
            }
            if after == before {
                continue;
            }
            edits.push(Edit::Heights {
                chunk: index,
                before,
                after: after.clone(),
            });
            heights::set_heights(tile.chunk_mut(index).unwrap(), &after);
        }
        if edits.is_empty() {
            return edits;
        }

        // The shading, on the chunks whose heights moved and on their four
        // neighbours.
        reshade(tile, &mut edits);
        edits
    }

    /// The heights this brush would leave in one chunk, without touching the
    /// tile.
    ///
    /// `level` is what [`Mode::Smooth`] moves toward and `goal` what the modes
    /// that read neighbours move toward. Both come from the caller because
    /// they belong to the whole stroke, not to this chunk: a level taken from
    /// this chunk's own heights is a different level in each chunk, which
    /// leaves a step at every chunk boundary.
    fn moved(
        &self,
        before: &[f32],
        origin: [f32; 3],
        at: [f32; 2],
        seconds: f32,
        level: Option<f32>,
        goal: Option<&Lattice>,
    ) -> Vec<f32> {
        // Where a vertex is, which the falloff, the noise and the goal need.
        let world = |i: usize| {
            let (dx, dy) = vertex_offset(i);
            (origin[0] - dx, origin[1] - dy)
        };
        // How far out it is, in the brush's own shape. See [`Shape`].
        let distance = |i: usize| {
            let (x, y) = world(i);
            self.shape.distance(x - at[0], y - at[1])
        };
        // The height Flatten and Smooth move toward. Flatten's is a plane
        // through the pivot at the mode's height, which is that height
        // everywhere while the tilt is zero.
        let target = match self.mode {
            Mode::Smooth => level,
            Mode::Flatten { to } => Some(to),
            _ => None,
        };
        let flattening = matches!(self.mode, Mode::Flatten { .. });
        let target_at = |i: usize, to: f32| match flattening {
            true => {
                let (x, y) = world(i);
                to + self.tilt[0] * (x - self.pivot[0]) + self.tilt[1] * (y - self.pivot[1])
            }
            false => to,
        };
        // A step toward a height, at `strength` of the distance a second,
        // clamped so a long frame cannot overshoot and oscillate.
        let toward = |h: f32, to: f32, w: f32| h + (to - h) * (self.strength * w * seconds).min(1.0);
        let crevice = match self.mode {
            Mode::Crevice => self.lip.clamp(0.0, 1.0) * rim_factor(),
            _ => 0.0,
        };

        before
            .iter()
            .enumerate()
            .map(|(i, &h)| {
                let t = distance(i) / self.radius;
                if let Mode::Crevice = self.mode {
                    return match t < 1.0 {
                        true => h + self.strength * seconds * (crevice * rim(t) - trench(t)),
                        false => h,
                    };
                }
                let w = self.falloff.over(t, self.core);
                if w == 0.0 {
                    return h;
                }
                match self.mode {
                    Mode::Raise => h + self.strength * w * seconds,
                    Mode::Lower => h - self.strength * w * seconds,
                    Mode::Noise => {
                        let (x, y) = world(i);
                        h + self.strength * w * seconds * wobble(x, y, self.scale)
                    }
                    Mode::Mountain => {
                        let (x, y) = world(i);
                        let ridges = self.ridges.clamp(0.0, 1.0);
                        let share = 1.0 - ridges + ridges * ridged(x, y, self.scale);
                        h + self.strength * w * seconds * share
                    }
                    Mode::Terrace { from } => {
                        toward(h, terraced(h, from, self.step, self.sharpness), w)
                    }
                    Mode::Blur | Mode::Erode => {
                        let (x, y) = world(i);
                        let Some(to) = goal.and_then(|goal| goal.at(x, y)) else {
                            return h;
                        };
                        match self.mode {
                            Mode::Blur => toward(h, to, w),
                            // The goal already holds this step's erosion at
                            // full strength; the falloff scales it.
                            _ => h + (to - h) * w,
                        }
                    }
                    Mode::Flatten { .. } | Mode::Smooth => match target {
                        Some(to) => {
                            let to = target_at(i, to);
                            // A fill leaves what is over the target and a cut
                            // what is under it. A smooth moves both ways.
                            let allowed = match (flattening, self.only) {
                                (true, Only::Fill) => to > h,
                                (true, Only::Cut) => to < h,
                                _ => true,
                            };
                            match allowed {
                                true => toward(h, to, w),
                                false => h,
                            }
                        }
                        None => h,
                    },
                    Mode::Crevice => h,
                }
            })
            .collect()
    }

    /// The two halves of the mean height under this brush in one tile: the
    /// weighted sum, and the total weight.
    ///
    /// They are returned apart so a caller can add up several tiles before
    /// dividing. A brush can span two files, and [`Mode::Smooth`] has to move
    /// all of them toward one height or it leaves a step at the border.
    ///
    /// Each vertex is weighted by the same curve the move is, so a vertex that
    /// barely moves barely counts toward the mean. The sum is `f64` because it
    /// adds a few thousand heights of hundreds of yards, and in `f32` the last
    /// terms are lost to rounding.
    pub fn weighed(&self, tile: &AdtFile, at: [f32; 2]) -> (f64, f64) {
        let mut sum = 0.0;
        let mut weight = 0.0;
        for index in self.chunks_under(tile, at) {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            for (i, h) in heights::heights(chunk).iter().enumerate() {
                let (dx, dy) = vertex_offset(i);
                let (x, y) = (origin[0] - dx, origin[1] - dy);
                let t = self.shape.distance(x - at[0], y - at[1]) / self.radius;
                let w = self.falloff.over(t, self.core) as f64;
                sum += *h as f64 * w;
                weight += w;
            }
        }
        (sum, weight)
    }

    /// The chunks whose square the brush's bounding square reaches.
    fn chunks_under(&self, tile: &AdtFile, at: [f32; 2]) -> Vec<usize> {
        (0..tile.chunks.len())
            .filter(|&i| {
                let Some(chunk) = tile.chunk(i) else {
                    return false;
                };
                let origin = chunk.head().position();
                let near = |o: f32, p: f32| p >= o - CHUNK_SIZE - self.radius && p <= o + self.radius;
                near(origin[0], at[0]) && near(origin[1], at[1])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every falloff is one at the centre and nothing at the rim, so a stroke
    /// leaves no step where the brush ended.
    ///
    /// The list is every variant. A curve without this property leaves a thin
    /// step at the radius.
    #[test]
    fn a_falloff_runs_from_one_to_nothing() {
        for falloff in [
            Falloff::Flat,
            Falloff::Linear,
            Falloff::Smooth,
            Falloff::Sharp,
            Falloff::Dome,
        ] {
            assert_eq!(falloff.weight(0.0), 1.0, "{falloff:?} at the centre");
            assert_eq!(falloff.weight(1.0), 0.0, "{falloff:?} at the rim");
            assert_eq!(falloff.weight(2.0), 0.0, "{falloff:?} outside");
            // It never rises on the way out.
            let mut last = 1.0;
            for step in 0..=20 {
                let w = falloff.weight(step as f32 / 20.0);
                assert!(w <= last + 1e-6, "{falloff:?} rises at {step}");
                last = w;
            }
        }
        assert_eq!(Falloff::Linear.weight(0.5), 0.5);
        assert!(Falloff::Smooth.weight(0.5) > 0.5, "smooth is flatter in the middle");
        // At half the radius a dome keeps most of its strength and a sharp
        // falloff has little left.
        assert!(Falloff::Dome.weight(0.5) > Falloff::Smooth.weight(0.5), "dome is fuller");
        assert!(Falloff::Sharp.weight(0.5) < Falloff::Linear.weight(0.5), "sharp collapses");
    }

    /// A shape's outline lies exactly at `t == 1` of its own distance.
    #[test]
    fn a_shapes_outline_is_its_own_rim() {
        for shape in [Shape::Circle, Shape::Square, Shape::Diamond] {
            for (x, y) in shape.outline(17.0, 64) {
                let t = shape.distance(x, y) / 17.0;
                assert!((t - 1.0).abs() < 1e-3, "{shape:?} at ({x}, {y}) is {t} out");
            }
        }
        // Along a diagonal a square reaches furthest and a diamond least.
        let diagonal = |shape: Shape| shape.distance(1.0, 1.0);
        assert!(diagonal(Shape::Square) < diagonal(Shape::Circle));
        assert!(diagonal(Shape::Circle) < diagonal(Shape::Diamond));
        // Along an axis all three agree: they are norms on one radius.
        for shape in [Shape::Circle, Shape::Square, Shape::Diamond] {
            assert!((shape.distance(3.0, 0.0) - 3.0).abs() < 1e-6, "{shape:?}");
        }
    }

    /// Noise depends on the position and nothing else, so a held stroke
    /// deepens the same bumps.
    #[test]
    fn noise_is_a_function_of_the_place_and_not_of_the_call() {
        assert_eq!(wobble(-9450.0, -50.0, 25.0), wobble(-9450.0, -50.0, 25.0));
        assert!(wobble(-9450.0, -50.0, 25.0) != wobble(-9445.8, -50.0, 25.0));
        assert!(wobble(-9450.0, -50.0, 25.0) != wobble(-9450.0, -45.8, 25.0));
        // A different scale is a different field.
        assert!(wobble(-9450.0, -50.0, 25.0) != wobble(-9450.0, -50.0, 60.0));
        // In range, and on both sides of zero: a hundred samples a cell apart
        // must straddle it, or Noise only raises.
        let mut low = 0;
        for step in 0..100 {
            let w = wobble(step as f32 * 4.17, 0.0, 25.0);
            assert!((-1.0..=1.0).contains(&w), "{w} out of range");
            if w < 0.0 {
                low += 1;
            }
        }
        assert!((20..80).contains(&low), "{low} of 100 below zero");
    }

    /// The noise is smoother at a coarser scale: the mean step between
    /// vertices 4.17 yards apart falls as the scale rises.
    #[test]
    fn a_coarser_noise_is_a_smoother_one() {
        let roughness = |scale: f32| {
            let mut total = 0.0;
            let mut steps = 0;
            for i in 0..200 {
                let x = i as f32 * 4.17;
                total += (wobble(x + 4.17, 0.0, scale) - wobble(x, 0.0, scale)).abs();
                steps += 1;
            }
            total / steps as f32
        };
        let fine = roughness(4.17);
        let coarse = roughness(50.0);
        assert!(
            coarse < fine * 0.5,
            "a 50 yd scale steps {coarse} between vertices against {fine} at 4.17"
        );
        assert!(roughness(200.0) < coarse, "200 yd is not smoother than 50");
    }

    /// A core holds the middle of the brush at full strength, and a core of
    /// zero is the plain curve.
    #[test]
    fn a_core_is_the_part_that_does_not_fall_off() {
        for falloff in [
            Falloff::Flat,
            Falloff::Linear,
            Falloff::Smooth,
            Falloff::Sharp,
            Falloff::Dome,
        ] {
            for step in 0..=20 {
                let t = step as f32 / 20.0;
                assert_eq!(falloff.over(t, 0.0), falloff.weight(t), "{falloff:?} at {t}");
            }
            assert_eq!(falloff.over(0.49, 0.5), 1.0, "{falloff:?} inside the core");
            assert_eq!(falloff.over(1.0, 0.5), 0.0, "{falloff:?} at the rim");
            let mut last = 1.0;
            for step in 0..=20 {
                let w = falloff.over(step as f32 / 20.0, 0.5);
                assert!(w <= last + 1e-6, "{falloff:?} rises at {step}");
                last = w;
            }
            // A bigger core is never weaker anywhere.
            for step in 0..=20 {
                let t = step as f32 / 20.0;
                assert!(
                    falloff.over(t, 0.6) >= falloff.over(t, 0.3) - 1e-6,
                    "{falloff:?} at {t} got weaker with a larger core"
                );
            }
        }
    }

    /// Terrace moves no vertex more than half a step, leaves ground on a step
    /// where it is, and at full sharpness answers only step heights.
    #[test]
    fn terraces_are_steps_from_where_the_stroke_began() {
        let (from, step) = (12.0, 8.0);
        for k in 0..400 {
            let h = -20.0 + k as f32 * 0.17;
            for sharpness in [0.0, 0.4, 1.0] {
                let to = terraced(h, from, step, sharpness);
                assert!((to - h).abs() <= step / 2.0 + 1e-3, "{h} went to {to} at {sharpness}");
            }
            let hard = terraced(h, from, step, 1.0);
            let n = (hard - from) / step;
            assert!((n - n.round()).abs() < 1e-4, "{h} went to {hard}, between steps");
        }
        assert_eq!(terraced(from + 2.0 * step, from, step, 0.6), from + 2.0 * step);
        assert_eq!(terraced(from + 0.1, from, step, 0.6), from);
        // Never falls as the height rises, or a slope would get a dip.
        let mut last = f32::MIN;
        for k in 0..400 {
            let to = terraced(k as f32 * 0.1, from, step, 0.5);
            assert!(to >= last - 1e-4, "terrace falls at {}", k as f32 * 0.1);
            last = to;
        }
    }

    /// The crevice's rims hold the ground its trench takes out when the lip is
    /// 1: on level ground a stroke changes the total height by almost nothing.
    #[test]
    fn a_crevice_piles_what_it_digs() {
        let tile = crate::adt::blank::blank_tile(32, 32, "x.blp", 50.0, 0);
        let mut tile = tile;
        let origin = tile.chunk(17).unwrap().head().position();
        let at = [origin[0] - CHUNK_SIZE / 2.0, origin[1] - CHUNK_SIZE / 2.0];
        let total = |tile: &AdtFile| -> f64 {
            (0..256)
                .filter_map(|i| tile.chunk(i))
                .flat_map(|chunk| heights::heights(chunk))
                .map(|h| h as f64)
                .sum()
        };
        let before = total(&tile);
        let brush = Brush { mode: Mode::Crevice, radius: 30.0, strength: 4.0, lip: 1.0, ..Brush::default() };
        let edits = brush.stroke(&mut tile, at, 1.0, None);
        assert!(!edits.is_empty());
        let centre = heights::height_at(&tile, at[0], at[1]).unwrap();
        assert!(centre < 47.0, "the trench is {centre}, not dug");
        // The vertices are 2.1 yd apart, so the sum is a coarse integral of
        // the profile; a few percent of the dug volume is what it can promise.
        let dug: f64 = 4.0 * 145.0;
        assert!((total(&tile) - before).abs() < dug * 0.1, "{} yd of height made or lost", total(&tile) - before);
    }

    /// Blur and Erode leave both copies of a vertex on a chunk edge at one
    /// height, because both read the same cell of the goal.
    #[test]
    fn modes_that_read_neighbours_keep_chunk_edges_together() {
        let mut tile = crate::adt::blank::blank_tile(32, 32, "x.blp", 50.0, 0);
        // Rough ground to blur and erode.
        let rough = Brush { mode: Mode::Noise, radius: 60.0, strength: 30.0, scale: 8.0, ..Brush::default() };
        let origin = tile.chunk(34).unwrap().head().position();
        let at = [origin[0], origin[1]];
        rough.stroke(&mut tile, at, 1.0, None);
        for (mode, erosion) in [(Mode::Blur, Erosion::Water), (Mode::Erode, Erosion::Water), (Mode::Erode, Erosion::Thermal)] {
            let brush = Brush { mode, erosion, radius: 40.0, strength: 5.0, scale: 12.0, talus: 20.0, ..Brush::default() };
            for seed in 0..5 {
                let mut ground = Lattice::around(at, brush.reach());
                ground.gather(&tile);
                let goal = brush.goal(&ground, at, 0.2, seed);
                brush.stroke_over(&mut tile, at, 0.2, &goal, None);
            }
            // Chunk 34's corner at its origin is also a corner of chunks 33,
            // 18 and 17.
            let corner = |index: usize, vertex: usize| heights::heights(tile.chunk(index).unwrap())[vertex];
            let here = corner(34, 0);
            for (index, vertex) in [(33, 8), (18, 136), (17, 144)] {
                assert_eq!(corner(index, vertex), here, "{mode:?} {erosion:?}: chunk {index} disagrees");
            }
        }
    }
}
