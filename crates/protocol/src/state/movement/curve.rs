//! **Catmull-Rom**, which is what `MoveSplineFlag::Flying` actually means.
//!
//! The flag's own comment in vmangos says both halves of it:
//!
//! ```text
//! Flying = 0x00000200,   // Smooth movement(Catmullrom interpolation mode), flying animation
//! ```
//!
//! and `Mask_CatmullRom = Flying` — one bit, two consequences, and this module
//! is the first of them. A flying spline's points arrive **absolute** rather
//! than as packed offsets precisely because the client is expected to run a
//! curve through them; walked as line segments instead, a taxi flight snaps
//! direction at every waypoint, which is exactly how it was reported.
//!
//! ## Transcribed, not derived
//!
//! `Movement/spline/spline.cpp` is the same code the client runs — mangos took
//! it from the client, comments and all ("we should use catmullrom initializer
//! even for linear mode! (client's internal structure limitation)"). Four
//! things in it are copied rather than reinvented, and each is a place a
//! reasonable independent implementation would differ:
//!
//! * **The basis matrix** (`s_catmullRomCoeffs`), applied as a row vector
//!   `(t³, t², t, 1) × M`. Standard Catmull-Rom with the 0.5 tension.
//! * **The two phantom control points** (`InitCatmullRom`). The one before the
//!   path is `controls[0].lerp(controls[1], -1)` — the second point mirrored
//!   through the first — and the one after is the **last point repeated**. They
//!   are not symmetric, and mirroring both ends makes the arrival overshoot.
//! * **The segment length is sampled, at three steps** (`STEPS_PER_SEGMENT`).
//!   Coarse, and it has to be: those lengths are what the *server* divided the
//!   duration by, so a more accurate measure here would put the client ahead of
//!   or behind the server's own idea of where the unit is.
//! * **The heading is the curve's derivative**, `atan2(hermite.y, hermite.x)`
//!   (`MoveSpline::ComputePosition`), evaluated with the same basis and
//!   `(3t², 2t, 1, 0)`. That is what makes a turn continuous rather than a
//!   corner, and it is why the smoothing and the banking are one change.

/// `s_catmullRomCoeffs`, row-major — see the module note.
const BASIS: [[f32; 4]; 4] = [
    [-0.5, 1.5, -1.5, 0.5],
    [1.0, -2.5, 2.0, -0.5],
    [-0.5, 0.0, 0.5, 0.0],
    [0.0, 1.0, 0.0, 0.0],
];

/// How many samples a segment's length is measured with — `STEPS_PER_SEGMENT`.
const STEPS_PER_SEGMENT: usize = 3;

/// The four weights for a parameter, given a `(t³, t², t, 1)`-shaped row.
fn weights(row: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0f32; 4];
    for (column, weight) in out.iter_mut().enumerate() {
        *weight = (0..4).map(|i| row[i] * BASIS[i][column]).sum();
    }
    out
}

fn combine(window: &[[f32; 3]], w: [f32; 4]) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for (axis, value) in out.iter_mut().enumerate() {
        *value = (0..4).map(|i| window[i][axis] * w[i]).sum();
    }
    out
}

/// **The control points the curve is actually run through** — the path with the
/// two phantoms `InitCatmullRom` adds.
///
/// The returned vector is `path.len() + 2` long, so segment `i` of the path is
/// evaluated over the window starting at `i`.
pub fn controls(path: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(path.len() + 2);
    match path {
        // `controls[0].lerp(controls[1], -1)` is `2a - b`: the second point
        // reflected through the first.
        [a, b, ..] => out.push([2.0 * a[0] - b[0], 2.0 * a[1] - b[1], 2.0 * a[2] - b[2]]),
        [only] => out.push(*only),
        [] => return out,
    }
    out.extend_from_slice(path);
    // …and the tail is the last point **repeated**, not mirrored.
    if let Some(last) = path.last() {
        out.push(*last);
    }
    out
}

/// Where segment `index` of the path is at local parameter `t` in `0..=1`.
///
/// `controls` is [`controls`]' output; `index` is the segment, so the window is
/// `controls[index ..= index + 3]`.
pub fn position(controls: &[[f32; 3]], index: usize, t: f32) -> Option<[f32; 3]> {
    let window = controls.get(index..index + 4)?;
    Some(combine(window, weights([t * t * t, t * t, t, 1.0])))
}

/// …and which way it is going there — the tangent, unnormalised.
pub fn derivative(controls: &[[f32; 3]], index: usize, t: f32) -> Option<[f32; 3]> {
    let window = controls.get(index..index + 4)?;
    Some(combine(window, weights([3.0 * t * t, 2.0 * t, 1.0, 0.0])))
}

/// How long segment `index` is, by the reference's own three-step sampling.
///
/// **Not the straight line between its ends**, which is what makes a curved
/// path longer than its polyline and is why this exists at all.
pub fn segment_length(controls: &[[f32; 3]], index: usize) -> f32 {
    let Some(start) = position(controls, index, 0.0) else {
        return 0.0;
    };
    let mut here = start;
    let mut length = 0.0;
    for step in 1..=STEPS_PER_SEGMENT {
        let Some(next) = position(controls, index, step as f32 / STEPS_PER_SEGMENT as f32) else {
            break;
        };
        length += super::distance(here, next);
        here = next;
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path along the x axis with one corner in it.
    fn path() -> Vec<[f32; 3]> {
        vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [20.0, 10.0, 0.0],
            [30.0, 10.0, 0.0],
        ]
    }

    /// **The curve passes through every control point.** `t = 0` on a segment
    /// is its first point and `t = 1` is its second — which is the property
    /// that makes the phantom points invisible and a wrong basis matrix
    /// obvious: any transposition of it moves the endpoints.
    #[test]
    fn the_curve_interpolates_its_control_points() {
        let path = path();
        let controls = controls(&path);
        assert_eq!(controls.len(), path.len() + 2);
        for (index, point) in path.iter().enumerate().take(path.len() - 1) {
            let at = position(&controls, index, 0.0).expect("a segment");
            for axis in 0..3 {
                assert!((at[axis] - point[axis]).abs() < 1e-4, "{index}: {at:?}");
            }
        }
        let last = position(&controls, path.len() - 2, 1.0).expect("the final segment");
        assert_eq!(last, [30.0, 10.0, 0.0]);
    }

    /// **The phantoms are asymmetric on purpose.** The leading one is a mirror
    /// and the trailing one is a repeat, which is what makes the *departure*
    /// straight and the *arrival* settle rather than overshoot.
    #[test]
    fn the_two_phantom_points_are_not_the_same_rule() {
        let path = path();
        let controls = controls(&path);
        assert_eq!(controls[0], [-10.0, 0.0, 0.0], "the second mirrored through the first");
        assert_eq!(controls[5], [30.0, 10.0, 0.0], "the last, repeated");
    }

    /// **A corner is rounded, and that is the whole point.** Halfway along the
    /// segment that ends at the corner, a straight line would still be on the x
    /// axis; the curve has already begun to turn.
    #[test]
    fn a_corner_is_a_curve_rather_than_a_hinge() {
        let controls = controls(&path());
        let middle = position(&controls, 1, 0.5).expect("the middle segment");
        assert!(middle[1] > 0.5, "the curve leans into the corner: {middle:?}");
    }

    /// **The heading at a corner is a blend of the two legs, and that is the
    /// whole of the smoothing.** Walked as line segments, a unit arriving at
    /// the corner faces 0 rad and the very next reading has it facing 45
    /// degrees — one frame, one snap, once per waypoint. The curve's own
    /// tangent there is Catmull-Rom's `(next - previous) / 2`, which for this
    /// path is `(10, 5)`: strictly between the two, and continuous across the
    /// join, so the turn takes as long as the corner does.
    #[test]
    fn the_heading_through_a_corner_is_continuous() {
        let controls = controls(&path());
        let heading = |index: usize, t: f32| {
            let d = derivative(&controls, index, t).expect("a tangent");
            d[1].atan2(d[0])
        };
        let corner = heading(1, 0.0);
        assert!(
            (corner - 0.4636).abs() < 1e-3,
            "atan2(5, 10) at the corner, got {corner}"
        );
        // The same point reached from the other side is the same heading —
        // which is what "continuous" means and what a polyline cannot do.
        assert!((heading(0, 1.0) - corner).abs() < 1e-4);
        // …and the two legs it sits between are 0 and 45 degrees.
        assert!(heading(0, 0.0).abs() < 0.2);
        assert!((heading(2, 1.0)).abs() < 0.2);
    }

    /// The sampled length is longer than the chord and close to it — three
    /// steps is coarse, and it is the number the durations were computed with.
    #[test]
    fn a_segments_length_is_sampled_rather_than_measured_straight() {
        let controls = controls(&path());
        let chord = super::super::distance([10.0, 0.0, 0.0], [20.0, 10.0, 0.0]);
        let sampled = segment_length(&controls, 1);
        assert!(sampled > chord, "{sampled} vs {chord}");
        assert!(sampled < chord * 1.2, "{sampled} vs {chord}");
        // A straight run has nothing to add: its curve is the line.
        let line = super::controls(&[[0.0, 0.0, 0.0], [3.0, 4.0, 0.0], [6.0, 8.0, 0.0]]);
        assert!((segment_length(&line, 0) - 5.0).abs() < 0.01);
    }

    /// A path too short to have a segment answers nothing rather than reading
    /// past its own control points.
    #[test]
    fn a_path_with_no_segment_answers_nothing() {
        assert!(controls(&[]).is_empty());
        let one = controls(&[[1.0, 2.0, 3.0]]);
        assert_eq!(position(&one, 0, 0.5), None);
        assert_eq!(derivative(&one, 0, 0.5), None);
        assert_eq!(segment_length(&one, 0), 0.0);
    }
}
