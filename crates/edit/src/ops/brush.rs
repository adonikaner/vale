//! The height brush and what every round brush shares: its shape, its
//! falloff, and the noise that roughens a stroke.

use crate::adt::{heights, AdtFile};
use vale_assets::world::adt::{vertex_offset, CHUNK_SIZE};
use super::{Edit, reshade, vertices};

/// **What a brush's footprint is**, as a distance rather than as a mask.
///
/// The whole of a shape here is *which norm measures `t`*, and that is what
/// makes it free: every [`Falloff`] curve composes with every shape without
/// either knowing about the other, and a square brush still has a soft edge.
/// A mask would have given a hard-edged square with a circular falloff inside
/// it, which is neither shape.
///
/// The three are the three norms anybody wants: round, a square aligned to the
/// world, and the square turned forty-five degrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// The Euclidean norm. A circle.
    Circle,
    /// The Chebyshev norm, `max(|dx|, |dy|)`. A square, axis-aligned — which for
    /// this game's ground is also **chunk-aligned**, so it is the shape that
    /// squares off a plateau along the grid the file is stored in.
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

    /// …and the outline of the shape at one radius, as a closed run of points
    /// around the centre.
    ///
    /// **Here rather than in the tool that draws it**, because a ring drawn from
    /// a second reading of the shape is a ring that lies about where the brush
    /// is — and the two would drift the first time a norm was corrected. The
    /// caller drops each point onto the ground.
    pub fn outline(self, radius: f32, segments: usize) -> Vec<(f32, f32)> {
        (0..segments)
            .map(|i| {
                let t = i as f32 / segments as f32;
                let angle = t * std::f32::consts::TAU;
                let (sin, cos) = angle.sin_cos();
                // The point on the unit shape along this direction: divide by
                // the direction's own distance, which is exactly the definition
                // of the shape's rim.
                let scale = radius / self.distance(cos, sin).max(1e-6);
                (cos * scale, sin * scale)
            })
            .collect()
    }
}

/// How a brush's strength falls off toward its edge.
///
/// Five curves over one parameter: `t`, how far out a point is as a fraction of
/// the radius, measured in whatever [`Shape`] the brush is. Every one of them is
/// zero at and beyond the rim, which is what lets a brush walk over ground it
/// does not reach without special-casing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Falloff {
    /// Full strength everywhere inside the radius. Leaves a visible rim.
    Flat,
    /// Straight from full at the centre to nothing at the rim.
    Linear,
    /// `1 — t^2` squared: flat in the middle and tangent to zero at the rim,
    /// which is what makes repeated strokes blend instead of terracing.
    Smooth,
    /// `(1 — t)^2` — **concave**, so the strength collapses as soon as it leaves
    /// the centre. A spike rather than a dome, and the one to reach for when
    /// what is wanted is a peak or a well rather than a hill.
    Sharp,
    /// `sqrt(1 — t^2)` — **convex**: a sphere's own profile, full for most of
    /// the radius and falling off a cliff at the rim. What a dome of ground
    /// actually looks like, where [`Falloff::Smooth`] gives a softer mound.
    Dome,
}

impl Falloff {
    /// **The weight at `t` with an inner core held at full strength.**
    ///
    /// `core` is the fraction of the radius that does not fall off at all, and
    /// the curve runs over what is left. It is what the brush preview's inner
    /// ring is drawn at, and what a person means by *where the pressure is*: a
    /// curve alone says how the strength decays but not how much of the brush is
    /// under it, and those are two different wants. A core of zero is
    /// [`Falloff::weight`] exactly, which is what every existing brush had.
    ///
    /// Clamped below 1 because a core of the whole radius is [`Falloff::Flat`]
    /// and would divide by nothing to get there.
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

/// **A signed value in `-1..1` that depends only on where a point is, and
/// varies over `scale` yards**, for [`Mode::Noise`].
///
/// Value noise: the plane is divided into a lattice of `scale`-yard cells, each
/// corner is hashed to its own number, and a point takes the smooth blend of the
/// four corners around it.
///
/// **The scale is the whole of what makes it usable.** The first version hashed
/// the position directly at a tenth of a yard, which is finer than the 4.17
/// yards between vertices — so every vertex got an *unrelated* number and the
/// result was white noise at the vertex spacing. That is the roughest thing the
/// format can hold, it is the same roughness at every strength, and it was
/// reported from the window as *"too jagged"*. With a lattice coarser than the
/// vertex spacing, neighbouring vertices sit inside one cell and get *similar*
/// numbers, which is what a rolling surface is. The scale is how many yards a
/// bump is across, and it is the brush's to choose.
///
/// The blend uses the fifth-order ease rather than a straight line, because a
/// linear blend is flat-sided inside each cell and creases at every lattice
/// line: the creases are a grid, and a grid is more obviously wrong than the
/// noise it replaced.
///
/// **Deterministic is still the point.** A brush that drew a fresh random number
/// each frame would shake the ground while it was held and settle nowhere; with
/// the value fixed by where the vertex is, holding it deepens the *same* bumps,
/// so roughness grows the way every other mode's effect grows and an undo takes
/// back one coherent shape. It is the same argument `ChunkFoliage`'s scatter
/// makes for hashing a chunk's world position rather than keeping a seed.
pub(crate) fn wobble(x: f32, y: f32, scale: f32) -> f32 {
    // A cell smaller than this is finer than the vertices can express, and zero
    // would divide by nothing.
    let scale = scale.max(0.5);
    let (u, v) = (x / scale, y / scale);
    let (cx, cy) = (u.floor(), v.floor());
    let (fx, fy) = (u - cx, v - cy);
    // Smootherstep: zero slope *and* zero curvature at both ends, so neither the
    // surface nor its shading shows the lattice.
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

/// One lattice corner's own number, in `-1..1`. The hash [`wobble`] is built on.
fn lattice(i: f32, j: f32) -> f32 {
    let mut hash = i as i64 as u64;
    hash = hash.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    hash ^= (j as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    hash ^= hash >> 32;
    // The top 24 bits, which are the ones the mixing has moved most.
    ((hash >> 40) as f32 / (1u32 << 23) as f32) - 1.0
}

/// What a stroke does to the height under it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Add `strength` yards a second.
    Raise,
    /// Subtract them.
    Lower,
    /// Move toward one height, at `strength` of the remaining distance a second.
    Flatten { to: f32 },
    /// Move toward the mean height of the vertices the brush covers, which
    /// pulls a ridge down and a ditch up at once.
    Smooth,
    /// **Roughen**: move each vertex by its own signed share of `strength`, at
    /// `strength` yards a second. See [`wobble`] for why the sign is a function
    /// of *where the vertex is* rather than of the frame — holding the brush has
    /// to deepen the same bumps rather than shake the ground.
    ///
    /// The one mode that is not a statement about a target. Raise, lower,
    /// flatten and smooth all converge on something; this is the only one whose
    /// whole purpose is that neighbouring vertices disagree.
    Noise,
}

/// A terrain brush.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brush {
    /// Yards.
    pub radius: f32,
    /// Yards a second for [`Mode::Raise`], [`Mode::Lower`] and [`Mode::Noise`];
    /// a fraction of the remaining distance a second for the other two.
    pub strength: f32,
    pub falloff: Falloff,
    /// How much of the radius is held at **full strength** before the falloff
    /// starts, as a fraction — see [`Falloff::over`]. Zero is the plain curve.
    pub core: f32,
    /// The footprint — see [`Shape`], which is a distance and not a mask.
    pub shape: Shape,
    /// How many yards a bump is across, for [`Mode::Noise`] — see [`wobble`].
    /// Ignored by every other mode.
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
}

/// Which way a flatten may move the ground.
///
/// A flatten toward a height does two things at once: it fills what is under
/// the height and cuts what is over it. Each alone is a tool of its own. Fill
/// makes a causeway across a hollow and leaves the hills beside it; cut
/// makes a shelf in a hillside and leaves the valley.
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
            // Six times the 4.17 yards between vertices, so the default reads as
            // rolling ground rather than as the vertex grid.
            scale: 25.0,
            mode: Mode::Raise,
            only: Only::Both,
            tilt: [0.0; 2],
            pivot: [0.0; 2],
        }
    }
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
    pub fn stroke_masked(
        &self,
        tile: &mut AdtFile,
        at: [f32; 2],
        seconds: f32,
        level: Option<f32>,
        mask: Option<(&vertices::Selected, vertices::Mask)>,
    ) -> Vec<Edit> {
        let touched = self.chunks_under(tile, at);
        let mut edits = Vec::new();
        // **What [`Mode::Smooth`] converges on, decided once for the whole
        // stroke.** See [`Brush::weighed`]: a mean taken per chunk is a different
        // height in every chunk, which is a step at every boundary. The caller
        // supplies it when the brush can reach more than one tile, which is the
        // only way the two sides of a border can agree.
        let level = match (self.mode, level) {
            (Mode::Smooth, None) => {
                let (sum, weight) = self.weighed(tile, at);
                (weight > 0.0).then(|| (sum / weight) as f32)
            }
            _ => level,
        };

        for &index in &touched {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            let before = heights::heights(chunk);
            if before.is_empty() {
                continue;
            }
            let mut after = self.moved(&before, origin, at, seconds, level);
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

    /// The heights this brush would leave, without touching the tile.
    ///
    /// `level` is what [`Mode::Smooth`] and [`Mode::Flatten`] converge on, and it
    /// comes from outside because it is a property of the whole stroke rather
    /// than of this chunk.
    fn moved(
        &self,
        before: &[f32],
        origin: [f32; 3],
        at: [f32; 2],
        seconds: f32,
        level: Option<f32>,
    ) -> Vec<f32> {
        // Where a vertex is, which both the falloff and the noise need.
        let world = |i: usize| {
            let (dx, dy) = vertex_offset(i);
            (origin[0] - dx, origin[1] - dy)
        };
        // …and how far out it is, **in the brush's own shape** — see [`Shape`],
        // where the whole of a shape being a distance is argued.
        let distance = |i: usize| {
            let (x, y) = world(i);
            self.shape.distance(x - at[0], y - at[1])
        };
        // **The height the two converging modes converge on**, and it is handed in
        // rather than worked out here. This used to take the mean of `before` —
        // *this chunk's own 145 heights* — so each chunk under one brush smoothed
        // toward a different height and left a step at every 33-yard boundary,
        // worst where that boundary was also a tile seam and the two sides were
        // in different files. Reported from the window as *"smooth is broken
        // across tiles"*; it was broken across chunks too.
        let target = match self.mode {
            Mode::Smooth => level,
            Mode::Flatten { to } => Some(to),
            _ => None,
        };
        // A flatten's target at one vertex: the plane through the pivot at
        // the mode's height, which is that height everywhere while the tilt
        // is zero. A smooth has no plane.
        let flattening = matches!(self.mode, Mode::Flatten { .. });
        let target_at = |i: usize, to: f32| match flattening {
            true => {
                let (x, y) = world(i);
                to + self.tilt[0] * (x - self.pivot[0]) + self.tilt[1] * (y - self.pivot[1])
            }
            false => to,
        };

        before
            .iter()
            .enumerate()
            .map(|(i, &h)| {
                let w = self.falloff.over(distance(i) / self.radius, self.core);
                if w == 0.0 {
                    return h;
                }
                match self.mode {
                    Mode::Raise => h + self.strength * w * seconds,
                    Mode::Lower => h - self.strength * w * seconds,
                    // **Its own direction per vertex**, fixed by where the
                    // vertex is — see [`wobble`].
                    Mode::Noise => {
                        let (x, y) = world(i);
                        h + self.strength * w * seconds * wobble(x, y, self.scale)
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
                                // Clamped so a long frame cannot overshoot the
                                // target and oscillate around it.
                                true => h + (to - h) * (self.strength * w * seconds).min(1.0),
                                false => h,
                            }
                        }
                        None => h,
                    },
                }
            })
            .collect()
    }

    /// **The two halves of the mean height under this brush, in one tile**: the
    /// weighted sum, and the total weight.
    ///
    /// Kept apart rather than divided because the caller adds up several tiles
    /// before dividing, which is the whole point. A brush 58 yards across spans
    /// four chunks and can span two files, and [`Mode::Smooth`] has to pull all
    /// of them toward *one* height or it leaves a step exactly where the boundary
    /// is, with nothing about either file wrong.
    ///
    /// Weighted by the same curve the move is, so a vertex that barely moves
    /// barely counts toward where everything is moving to.
    ///
    /// `f64` because this sums a few thousand terms and the heights are hundreds
    /// of yards; in `f32` the tail of the sum stops landing.
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

    /// The chunks whose square the brush's circle reaches.
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

    /// Every falloff is one at the centre and nothing at the rim, which is what
    /// stops a stroke from leaving a step where the brush ended.
    ///
    /// **The list is every variant and not a sample.** A curve added without
    /// this property is a brush that terraces, and the terracing is a thin step
    /// exactly at the radius — which reads as a tool that cannot blend rather
    /// than as one curve being wrong.
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
            // …and it never climbs on the way out, which is what "falloff"
            // means and what a mistyped exponent quietly breaks.
            let mut last = 1.0;
            for step in 0..=20 {
                let w = falloff.weight(step as f32 / 20.0);
                assert!(w <= last + 1e-6, "{falloff:?} rises at {step}");
                last = w;
            }
        }
        assert_eq!(Falloff::Linear.weight(0.5), 0.5);
        assert!(Falloff::Smooth.weight(0.5) > 0.5, "smooth is flatter in the middle");
        // **The two new ones are each other's opposite**, which is the whole of
        // why both exist: at the half-way point a dome still has most of its
        // strength and a sharp has almost none.
        assert!(Falloff::Dome.weight(0.5) > Falloff::Smooth.weight(0.5), "dome is fuller");
        assert!(Falloff::Sharp.weight(0.5) < Falloff::Linear.weight(0.5), "sharp collapses");
    }

    /// **A shape is a distance, and its outline is the rim that distance
    /// describes.**
    ///
    /// The property that makes shapes free: the falloff never learns about them,
    /// so every point the outline draws has to be exactly at `t == 1`. A ring
    /// drawn from a second reading of the shape would say the brush reaches
    /// somewhere it does not, which is the one thing a preview must not do.
    #[test]
    fn a_shapes_outline_is_its_own_rim() {
        for shape in [Shape::Circle, Shape::Square, Shape::Diamond] {
            for (x, y) in shape.outline(17.0, 64) {
                let t = shape.distance(x, y) / 17.0;
                assert!((t - 1.0).abs() < 1e-3, "{shape:?} at ({x}, {y}) is {t} out");
            }
        }
        // …and the three really are different shapes: along a diagonal, a square
        // reaches furthest and a diamond least.
        let diagonal = |shape: Shape| shape.distance(1.0, 1.0);
        assert!(diagonal(Shape::Square) < diagonal(Shape::Circle));
        assert!(diagonal(Shape::Circle) < diagonal(Shape::Diamond));
        // …and along an axis all three agree, which is what says they are norms
        // on one radius rather than three unrelated sizes.
        for shape in [Shape::Circle, Shape::Square, Shape::Diamond] {
            assert!((shape.distance(3.0, 0.0) - 3.0).abs() < 1e-6, "{shape:?}");
        }
    }

    /// **Noise depends on where a vertex is and on nothing else.**
    ///
    /// The property a held stroke rests on: asked twice for the same place it
    /// answers the same, so the brush deepens the same bumps instead of shaking
    /// the ground.
    #[test]
    fn noise_is_a_function_of_the_place_and_not_of_the_call() {
        assert_eq!(wobble(-9450.0, -50.0, 25.0), wobble(-9450.0, -50.0, 25.0));
        assert!(wobble(-9450.0, -50.0, 25.0) != wobble(-9445.8, -50.0, 25.0));
        assert!(wobble(-9450.0, -50.0, 25.0) != wobble(-9450.0, -45.8, 25.0));
        // A different scale is a different field, or the setting does nothing.
        assert!(wobble(-9450.0, -50.0, 25.0) != wobble(-9450.0, -50.0, 60.0));
        // In range, and not stuck to one side of it: a hundred samples a cell
        // apart have to straddle zero, or "roughen" is "raise".
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

    /// **The noise is smooth at the scale it is given**, which is what the scale
    /// is for.
    ///
    /// The reported fault was *"too jagged"*, and the measurement behind it is
    /// this: hashing the position directly gives neighbouring vertices
    /// *unrelated* numbers, so the step between them is as large as the range
    /// itself. Value noise on a lattice coarser than the vertex spacing gives
    /// them nearly the same number. The check is the mean step between vertices
    /// 4.17 yards apart, and it has to fall as the scale rises — otherwise the
    /// panel's control is a number that changes nothing.
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
        // …and it keeps falling, rather than bottoming out at some middle value.
        assert!(roughness(200.0) < coarse, "200 yd is not smoother than 50");
    }

    /// **A core holds the middle of the brush at full strength**, and a core of
    /// nothing is the curve it was before.
    ///
    /// The second half is what makes it safe to add: every brush in the crate
    /// defaults to zero, so nothing that was tuned against the old behaviour
    /// moved.
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
            // Everything inside the core is full strength, the rim is still
            // nothing, and it never climbs on the way out.
            assert_eq!(falloff.over(0.49, 0.5), 1.0, "{falloff:?} inside the core");
            assert_eq!(falloff.over(1.0, 0.5), 0.0, "{falloff:?} at the rim");
            let mut last = 1.0;
            for step in 0..=20 {
                let w = falloff.over(step as f32 / 20.0, 0.5);
                assert!(w <= last + 1e-6, "{falloff:?} rises at {step}");
                last = w;
            }
            // …and a bigger core is never weaker anywhere, which is the whole
            // of what the control promises.
            for step in 0..=20 {
                let t = step as f32 / 20.0;
                assert!(
                    falloff.over(t, 0.6) >= falloff.over(t, 0.3) - 1e-6,
                    "{falloff:?} at {t} got weaker with a larger core"
                );
            }
        }
    }
}
