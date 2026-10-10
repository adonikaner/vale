//! How much of the player's own model shows as the camera zooms in to first
//! person.
//!
//! The 1.12.1 client lets the camera zoom to a distance of zero from the
//! point it orbits, which puts the eye inside the character's head. On the
//! way in it fades the player's own model: fully drawn while the camera is at
//! least [`FADE_WINDOW`] yards beyond its near plane, fading on a cosine ease
//! through that window, and not drawn at all within [`HIDDEN_WITHIN`] of it.
//! The opacity is kept as a byte, 0 to 255.
//!
//! The client widens the window while the camera looks steeply down; that
//! widening is not ported.

/// The distance beyond the near plane over which the model fades, in yards.
pub const FADE_WINDOW: f32 = 1.8315;

/// Within this distance beyond the near plane the model is not drawn, in
/// yards (1/360 of a yard). The camera is then in first person.
pub const HIDDEN_WITHIN: f32 = 0.002_777_78;

/// The player model's opacity, as the client's byte, for a camera
/// `distance` yards from the point it orbits with its near plane `near` yards
/// in front of it.
pub fn opacity_byte(distance: f32, near: f32) -> u8 {
    let beyond = distance - near;
    if beyond >= FADE_WINDOW {
        return 255;
    }
    if beyond <= HIDDEN_WITHIN {
        return 0;
    }
    let t = beyond / FADE_WINDOW;
    let eased = (1.0 - (std::f32::consts::PI * t).cos()) * 0.5;
    (eased * 255.0) as u8
}

/// The same as a fraction, 0 to 1.
pub fn opacity(distance: f32, near: f32) -> f32 {
    f32::from(opacity_byte(distance, near)) / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_is_whole_at_a_distance_and_gone_in_first_person() {
        assert_eq!(opacity_byte(15.0, 0.1), 255);
        assert_eq!(opacity_byte(0.1 + FADE_WINDOW, 0.1), 255);
        assert_eq!(opacity_byte(0.0, 0.1), 0, "inside the near plane");
        assert_eq!(opacity_byte(0.1, 0.1), 0, "at the near plane");
        assert_eq!(opacity_byte(0.1 + HIDDEN_WITHIN, 0.1), 0);
    }

    #[test]
    fn the_fade_is_a_cosine_ease_through_the_window() {
        let half = opacity_byte(0.1 + FADE_WINDOW / 2.0, 0.1);
        assert!((126..=128).contains(&half), "{half}");
        let early = opacity_byte(0.1 + FADE_WINDOW / 4.0, 0.1);
        assert!(early < 64, "a cosine ease starts slowly: {early}");
        let mut last = 0;
        for step in 0..=100 {
            let byte = opacity_byte(0.1 + FADE_WINDOW * step as f32 / 100.0, 0.1);
            assert!(byte >= last);
            last = byte;
        }
    }
}
