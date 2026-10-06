//! The perspective rule the 1.12.1 client uses, and the world camera's field
//! of view.
//!
//! Every camera in the 1.12.1 client builds its projection the same way: the
//! world camera, the login screen's scenes, the paperdoll bake and the
//! portrait bake. It takes `(fov, aspect, near, far)`, and the two terms that
//! depend on the angle are
//!
//! ```text
//! m00 = 1 / (aspect * tan(half))     m11 = 1 / tan(half)
//! half = (fov / sqrt(aspect² + 1)) / 2
//! ```
//!
//! So the `fov` it is given is a diagonal angle, and the vertical angle that
//! Bevy's `PerspectiveProjection::fov` takes is [`vertical_fov`]. The angle is
//! in radians and must satisfy `0 < fov <= π`.
//!
//! The world camera's value is [`WORLD_FIELD_OF_VIEW`]. Past 16:9 this module
//! stops applying the rule; see [`WIDEST_FRAMED_ASPECT`], which is a deviation
//! from the client.

/// The world camera's field of view, a diagonal angle in radians.
///
/// 1.925 is the value the widely used `vanilla-tweaks` patch sets; the
/// unmodified 1.12.1 client uses `π/2`. The patched value is used because the
/// reference this client is compared against runs the patch, and its wider
/// view is what players of that install see. Resetting the camera restores
/// this constant.
///
/// The diagonal convention means the number is not a vertical or horizontal
/// angle. At 4:3 it is 66.2° vertical and at 16:9 54.1° (the unmodified `π/2`
/// gives 54.0° and 44.1°). The vertical figure is the one to compare against a
/// `PerspectiveProjection::fov`, whose Bevy default is 45°.
pub const WORLD_FIELD_OF_VIEW: f32 = 1.925;

/// The widest aspect the diagonal rule is applied at. This is a deviation from
/// the client.
///
/// With the live aspect, the rule narrows the vertical view as the window
/// widens: at 21:9 the unmodified `π/2` gives 34.8° vertical against 16:9's
/// 44.1°, so an ultrawide screen shows less of the world than a laptop. On the
/// login screen at 3440x1387 a character filled the frame with the head cut
/// off.
///
/// The 1.12.1 client's resolution list stops at 4:3, so it was never used at
/// such an aspect. Past this aspect the vertical angle holds at the 16:9 value
/// and the extra width widens the view. Up to 16:9 the rule is unchanged.
pub const WIDEST_FRAMED_ASPECT: f32 = 16.0 / 9.0;

/// The vertical angle for a diagonal one, at this aspect.
///
/// At 4:3, the aspect the portrait path bakes at, this is `0.6 · fov`; at
/// 16:9 it is `fov / 2.04`, so `UI_MainMenu`'s authored 86° becomes 42°.
///
/// The formula is the client's; see the module doc for its two matrix terms.
/// The login screen confirms it: read as a vertical angle, the scene's portal
/// fills about half the frame where the reference fills the whole of it, and
/// the ratio between the two is this factor.
pub fn vertical_fov(diagonal: f32, aspect: f32) -> f32 {
    diagonal / (aspect * aspect + 1.0).sqrt()
}

/// [`vertical_fov`], with the aspect held at [`WIDEST_FRAMED_ASPECT`].
///
/// The two cameras that fill the window use this: the world camera and the
/// login screen's. The paperdoll and portrait bakes do not, because each
/// renders into its own target at an aspect it states. The portrait bake's
/// extra 3:4 horizontal squeeze comes from rendering into a square target at a
/// fixed 4/3 and has nothing to do with this function.
pub fn framed_vertical_fov(diagonal: f32, aspect: f32) -> f32 {
    vertical_fov(diagonal, aspect.min(WIDEST_FRAMED_ASPECT))
}

/// The aspect of a window, or 16:9 before one has been measured.
///
/// A zero-height window during a minimise and the NaN of a zero-by-zero window
/// both read as 16:9, because either would break the frame's culling.
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

    /// A record's field of view is diagonal, so the same number gives a
    /// different vertical angle on a different window. See [`vertical_fov`].
    #[test]
    fn the_records_fov_is_diagonal_and_narrows_with_the_aspect() {
        let degrees = |d: f32, a: f32| vertical_fov(d.to_radians(), a).to_degrees();
        // At 4:3 the factor is exactly 3/5, the "0.6 x fov" the portrait path
        // bakes at.
        assert!((degrees(86.0, 4.0 / 3.0) - 51.6).abs() < 0.05);
        // At 16:9 the same record is narrower.
        assert!((degrees(86.0, 16.0 / 9.0) - 42.16).abs() < 0.05);
        // A square target is `fov / sqrt(2)`, and a wider window is always
        // narrower than a taller one.
        assert!((degrees(90.0, 1.0) - 63.64).abs() < 0.05);
        assert!(degrees(86.0, 21.0 / 9.0) < degrees(86.0, 16.0 / 9.0));
    }

    /// The world's field of view as a vertical angle, which is the number a
    /// `PerspectiveProjection` takes.
    #[test]
    fn the_worlds_own_field_of_view_in_the_units_bevy_wants() {
        let degrees = |a: f32| vertical_fov(WORLD_FIELD_OF_VIEW, a).to_degrees();
        assert!((degrees(4.0 / 3.0) - 66.18).abs() < 0.05);
        assert!((degrees(16.0 / 9.0) - 54.08).abs() < 0.05);
    }

    /// Past 16:9 the vertical angle stops changing, so a wider window shows
    /// more of the world instead of a narrower view of it.
    #[test]
    fn an_ultrawide_window_holds_the_sixteen_by_nine_vertical() {
        let held = framed_vertical_fov(WORLD_FIELD_OF_VIEW, 3440.0 / 1440.0);
        let sixteen_by_nine = vertical_fov(WORLD_FIELD_OF_VIEW, 16.0 / 9.0);
        assert!((held - sixteen_by_nine).abs() < 1e-6);
        // Narrower than 16:9, the client's rule applies unchanged.
        let four_by_three = framed_vertical_fov(WORLD_FIELD_OF_VIEW, 4.0 / 3.0);
        assert!(four_by_three > sixteen_by_nine);
    }

    /// A window with no height must not reach the division.
    #[test]
    fn a_degenerate_window_reads_as_sixteen_by_nine() {
        assert_eq!(window_aspect(0.0, 0.0), WIDEST_FRAMED_ASPECT);
        assert_eq!(window_aspect(f32::NAN, 720.0), WIDEST_FRAMED_ASPECT);
        assert!((window_aspect(1920.0, 1080.0) - 16.0 / 9.0).abs() < 1e-6);
    }
}
