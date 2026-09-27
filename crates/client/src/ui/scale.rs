//! **How big the interface is drawn**: the `uiScale` in force, in one place
//! that the painter, the pointer and the loader all read.
//!
//! The *rule* is [`crate::lua::widgets::layout`] — `automatic_scale` and
//! `scale_in_force`, both following the client's own rules — and
//! it is there because it decides how many units tall the space is, which is
//! that module's whole subject. What is here is only the answer: one `f64`,
//! recomputed each frame from the window and from the two CVars that control it.
//!
//! ```text
//! useUiScale = 0   (the default)   scale = automatic_scale(window height)
//! useUiScale = 1                   scale = max(0.64, uiScale)
//! ```
//!
//! ## Why a resource rather than each reader working it out
//!
//! Four passes need it and three of them are in other directories:
//! [`super::framexml`] paints through it, [`crate::lua::api::mouse`] hit-tests
//! through it, [`crate::lua::widgets::tooltip`] places a floating plate with
//! it, and [`crate::lua::host`] tells the freshly-loaded interface how big the
//! screen is in it. **The painter and the pointer must agree within a frame or
//! the hit test misses what is on screen** — the same argument that made
//! `Viewport` a type with both directions on it rather than a bare scale — so
//! they read one value rather than each deriving one from the CVars.
//!
//! ## …and why it is `PreUpdate`
//!
//! So that every reader in `Update` sees the same number and none of them needs
//! an `.after()` on this. The cost is that a slider dragged this frame is
//! answered on the next, which is one frame of a change nobody can see at
//! sixty of them a second.

use bevy::prelude::*;

use crate::settings::cvars::CVars;
use crate::lua::widgets::layout;

/// **The `uiScale` in force**, as [`layout::scale_in_force`] answers it.
///
/// Defaults to 1.0 rather than to [`layout::AUTO_MIN_UI_SCALE`], because the
/// value before the first frame belongs to a client that has not been told how
/// big its window is, and 1.0 is the identity — the space is then exactly
/// [`layout::UI_SIZE`], which is what a headless run stands in with.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct InterfaceScale(pub f64);

impl Default for InterfaceScale {
    fn default() -> InterfaceScale {
        InterfaceScale(1.0)
    }
}

impl InterfaceScale {
    /// The number, for a caller that only wants to divide by it.
    pub fn get(self) -> f64 {
        self.0
    }
}

pub struct InterfaceScalePlugin;

impl Plugin for InterfaceScalePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InterfaceScale>()
            .add_systems(PreUpdate, follow);
    }
}

/// Recompute from the window and the two CVars.
///
/// **The window's height in logical pixels**, which is what the reference's own
/// rule takes: it is applied on a resolution change with the back buffer's own
/// width and height. A run with no window at all — the
/// headless `--audit` — keeps [`layout::UI_SIZE`]'s height, so the space stays
/// the stated 1365x768 rather than becoming a shape derived from nothing.
///
/// **The glue draws at scale 1.0 outright — neither the `uiScale` pair nor
/// the automatic floor.** `uiScale` is a game-UI setting the login screens
/// have no slider for, and the automatic 0.9 is `UIParent`'s rule, not the
/// glue's: the reference's `GlueParent` is a fixed
/// 768-tall space stretched to the window, and `Interface\GlueXML\` is
/// authored against exactly that height in **absolute** offsets —
/// `AccountLoginLoginButton` hangs 519 from the top while the two edit
/// boxes stand 270 and 345 from the bottom, which only meet in the right
/// order in a 768-tall space. Drawn at the automatic 0.9 (any window taller
/// than 853 pixels) the space is 853 units tall, the Login button rides up
/// over the password box, and the login screen scrambles — which is exactly
/// what a resize to a large window looked like, on both painters. At 1.0
/// the space is 768 tall at every window size and the authored offsets
/// stand.
fn follow(
    cvars: Res<CVars>,
    loaded: Res<crate::lua::host::LoadWanted>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut scale: ResMut<InterfaceScale>,
) {
    let height = windows
        .single()
        .map_or(layout::UI_SIZE.1, |window| f64::from(window.height()));
    let now = if loaded.world_interface_loaded() {
        InterfaceScale(layout::scale_in_force(
            cvars.flag("useUiScale"),
            f64::from(cvars.number("uiScale")),
            height,
        ))
    } else {
        InterfaceScale(1.0)
    };
    // **Written only when it moves.** `ResMut` marks the resource changed on
    // every deref, and the painter's own re-walk latch is keyed on the space's
    // size — a resource that says "changed" sixty times a second would be a
    // trap for the next thing that reads it with `Res::is_changed`.
    if *scale != now {
        *scale = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The default is 0.9 on anything taller than 853 pixels**, which is the
    /// whole of what this round changed on screen: the interface is drawn
    /// `768/0.9` units tall rather than 768, one ninth smaller.
    #[test]
    fn the_automatic_scale_floors_at_nine_tenths() {
        assert_eq!(layout::automatic_scale(768.0), 1.0, "a 768-pixel window is 1:1");
        assert_eq!(layout::automatic_scale(600.0), 1.0, "and so is a shorter one");
        // 768/900 = 0.853, 768/1080 = 0.711, 768/1440 = 0.533 — all under the
        // floor, so all of them answer it.
        for height in [900.0, 1080.0, 1200.0, 1440.0, 2160.0] {
            assert_eq!(
                layout::automatic_scale(height),
                layout::AUTO_MIN_UI_SCALE,
                "{height} is floored"
            );
        }
        // …and the one band where the divide is what answers: 768 < h < 853⅓.
        let scale = layout::automatic_scale(800.0);
        assert!((scale - 0.96).abs() < 1e-6, "768/800, above the floor: {scale}");
    }

    /// **The switch is the switch**: with `useUiScale` off the slider's number
    /// is ignored outright, which is the client's own rule rather than a
    /// simplification.
    #[test]
    fn the_slider_is_ignored_until_the_box_is_ticked() {
        assert_eq!(layout::scale_in_force(false, 0.64, 1080.0), 0.9);
        assert_eq!(layout::scale_in_force(true, 0.64, 1080.0), 0.64);
        // …and the floor `set_ui_scale` applies before it touches the frame.
        assert_eq!(layout::scale_in_force(true, 0.1, 1080.0), layout::MIN_UI_SCALE);
        assert_eq!(layout::scale_in_force(true, f64::NAN, 1080.0), 0.9);
    }
}
