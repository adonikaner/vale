//! How the reference builds a perspective, and the opening angle it builds the
//! world with.
//!
//! The 5875 client has **one** perspective builder, and every
//! camera in the game goes through it: the world camera, the login screen's
//! scenes, the paperdoll bake and the portrait bake. It takes
//! `(fov, aspect, near, far)` and writes a 4x4, and the two terms that matter
//! are
//!
//! ```text
//! m00 = 1 / (aspect * tan(half))     m11 = 1 / tan(half)
//! half = (fov / sqrt(aspect² + 1)) / 2
//! ```
//!
//! so **the `fov` it is given is a diagonal angle**, and the *vertical* opening
//! angle Bevy's `PerspectiveProjection::fov` wants is [`vertical_fov`]. The
//! function guards `0 < fov <= π`, which is what says the argument is in
//! radians.
//!
//! The world camera's own value is [`WORLD_FIELD_OF_VIEW`], and past 16:9 this
//! module stops applying the rule — see [`WIDEST_FRAMED_ASPECT`], which is a
//! stated deviation rather than a reading.

/// The world camera's field of view: **`π/2`, a diagonal angle** — the 90° the
/// game is always said to have.
///
/// The camera is constructed with `π/2` as its field of view, and that value
/// goes straight into the perspective build beside the aspect, the near and
/// the far. Resetting the camera restores the same constant, which is what a
/// field of view set from a script (in degrees) is reset *to*.
///
/// **What it means on screen is not 90° of anything Bevy names**, because of
/// the diagonal convention above: at 4:3 it is 54.0° vertical and 68.2°
/// horizontal, at 16:9 44.1° and 71.8°. The vertical figure is the one to
/// compare against a `PerspectiveProjection::fov`, whose Bevy default — 45° —
/// is within a degree of the reference at 16:9 and about nine degrees short of
/// it at 4:3.
pub const WORLD_FIELD_OF_VIEW: f32 = std::f32::consts::FRAC_PI_2;

/// The widest aspect the diagonal rule is applied at, **and it is a stated
/// deviation.**
///
/// Taking the live aspect makes the reference vertical-minus: the wider the
/// window, the narrower the whole view. At 21:9 the rule gives 34.8° vertical
/// against 16:9's 44.1°, so an ultrawide screen would show *less* world than a
/// laptop rather than more, and on the login screen — where this was first hit,
/// at 3440x1387 — a character filled the frame with their head cropped off.
///
/// The reference never met such a screen: its resolution list tops out at 4:3.
/// So faithfulness past this point reproduces an accident rather than a design,
/// and beyond it the vertical framing holds at the 16:9 answer and the extra
/// width widens the view instead of zooming it. Up to 16:9 nothing is changed.
pub const WIDEST_FRAMED_ASPECT: f32 = 16.0 / 9.0;

/// The **vertical** opening angle for a diagonal one, at this aspect.
///
/// At the 4:3 the portrait path bakes to, this is the community's familiar
/// `0.6 · fov`; at a 16:9 window it is `fov / 2.04`, and `UI_MainMenu`'s
/// authored 86° becomes 42°.
///
/// **The formula is the client's** — see this module's
/// own note for its two matrix terms. The picture agrees:
/// taken as vertical, the login screen's portal fills about half the frame
/// against a reference that has it filling the whole of it, and the ratio
/// between the two readings is this factor.
pub fn vertical_fov(diagonal: f32, aspect: f32) -> f32 {
    diagonal / (aspect * aspect + 1.0).sqrt()
}

/// [`vertical_fov`], with the aspect held at [`WIDEST_FRAMED_ASPECT`].
///
/// This is what both cameras that fill the window use — the world's and the
/// login screen's. The two bakes do not: each renders into a target of its own
/// whose aspect it states itself, so there is no window to be wider than. The
/// **portrait bake's extra 3:4 anamorphic squeeze** is no part of
/// any of this either; it comes from rendering into a square target at a fixed
/// 4/3, and the two cameras here fill a window.
pub fn framed_vertical_fov(diagonal: f32, aspect: f32) -> f32 {
    vertical_fov(diagonal, aspect.min(WIDEST_FRAMED_ASPECT))
}

/// The aspect of a window, or the 16:9 to assume before one has been measured.
///
/// Guards the two readings that would take a whole frame's culling with them: a
/// zero-height window during a minimise, and the NaN a zero-by-zero produces.
pub fn window_aspect(width: f32, height: f32) -> f32 {
    let aspect = width / height.max(1.0);
    if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        WIDEST_FRAMED_ASPECT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The record's fov is diagonal**, so the same number is a different
    /// picture on a different window — see [`vertical_fov`], where the source
    /// and what was checked against it are.
    #[test]
    fn the_records_fov_is_diagonal_and_narrows_with_the_aspect() {
        let degrees = |d: f32, a: f32| vertical_fov(d.to_radians(), a).to_degrees();
        // At 4:3 the factor is exactly 3/5 — the "0.6 x fov" the portrait path
        // bakes at.
        assert!((degrees(86.0, 4.0 / 3.0) - 51.6).abs() < 0.05);
        // At 16:9 the same record is a little over half that again.
        assert!((degrees(86.0, 16.0 / 9.0) - 42.16).abs() < 0.05);
        // A square target is `fov / sqrt(2)`, and a wider window is always
        // narrower than a taller one.
        assert!((degrees(90.0, 1.0) - 63.64).abs() < 0.05);
        assert!(degrees(86.0, 21.0 / 9.0) < degrees(86.0, 16.0 / 9.0));
    }

    /// **What `π/2` diagonal is worth vertically**, which is the number that
    /// goes into a `PerspectiveProjection` and the only one comparable with
    /// Bevy's own 45° default.
    #[test]
    fn the_worlds_own_field_of_view_in_the_units_bevy_wants() {
        let degrees = |a: f32| vertical_fov(WORLD_FIELD_OF_VIEW, a).to_degrees();
        assert!((degrees(4.0 / 3.0) - 54.0).abs() < 0.05);
        assert!((degrees(16.0 / 9.0) - 44.13).abs() < 0.05);
    }

    /// **Past 16:9 the view widens instead of zooming**, which is
    /// [`WIDEST_FRAMED_ASPECT`]'s whole point: the vertical answer stops
    /// moving, so the extra pixels are extra world.
    #[test]
    fn an_ultrawide_window_holds_the_sixteen_by_nine_vertical() {
        let held = framed_vertical_fov(WORLD_FIELD_OF_VIEW, 3440.0 / 1440.0);
        let sixteen_by_nine = vertical_fov(WORLD_FIELD_OF_VIEW, 16.0 / 9.0);
        assert!((held - sixteen_by_nine).abs() < 1e-6);
        // …and narrower than 16:9 is untouched, which is the half that is a
        // reading rather than a deviation.
        let four_by_three = framed_vertical_fov(WORLD_FIELD_OF_VIEW, 4.0 / 3.0);
        assert!(four_by_three > sixteen_by_nine);
    }

    /// A window with no height is a divide this must not pass on.
    #[test]
    fn a_degenerate_window_reads_as_sixteen_by_nine() {
        assert_eq!(window_aspect(0.0, 0.0), WIDEST_FRAMED_ASPECT);
        assert_eq!(window_aspect(f32::NAN, 720.0), WIDEST_FRAMED_ASPECT);
        assert!((window_aspect(1920.0, 1080.0) - 16.0 / 9.0).abs() < 1e-6);
    }
}
