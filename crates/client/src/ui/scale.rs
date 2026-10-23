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
//! ## The viewport beside it
//!
//! [`InterfaceViewport`] is the other half of the same answer: where the
//! space sits in the window this frame. The draw walk, the hit test, the
//! world tooltip, the paper doll and the mesh painter all read it, so a window
//! size or scale factor is taken from one place and the pointer and the
//! picture cannot be fitted differently.
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
use crate::lua::widgets::layout::Viewport;

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

/// Where the interface's space sits in the window this frame, computed with
/// [`InterfaceScale`] from the same window.
///
/// Every system that converts between interface units and window pixels
/// reads this resource rather than building a [`Viewport`] from the window
/// itself: the draw walk, the hit test, the world tooltip, the paper doll and
/// the mesh painter.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct InterfaceViewport {
    /// The primary window's size in logical pixels, or `None` in a run with no
    /// window.
    pub window: Option<Vec2>,
    /// Physical pixels per logical pixel: the window's scale factor, or 1.0
    /// with no window.
    pub dpi: f32,
    /// Interface units to logical pixels and back, at the `uiScale` in force.
    /// With no window it fits the space into [`layout::UI_SIZE`].
    pub view: Viewport,
    /// The interface's space in its own units, `(wide, tall)`:
    /// [`layout::units_wide`] by [`layout::ui_height`]. The draw walk solves
    /// against it and walks again when it changes.
    pub space: (f32, f32),
}

impl InterfaceViewport {
    /// The viewport for a window of `size` logical pixels (or none) at `dpi`
    /// and `ui_scale`.
    pub fn of(size: Option<Vec2>, dpi: f32, ui_scale: f64) -> InterfaceViewport {
        let (width, height) = size.map_or(layout::UI_SIZE, |size| {
            (f64::from(size.x), f64::from(size.y))
        });
        InterfaceViewport {
            window: size,
            dpi,
            view: Viewport::of(width, height, ui_scale),
            space: (
                layout::units_wide(width, height, ui_scale) as f32,
                layout::ui_height(ui_scale) as f32,
            ),
        }
    }
}

impl Default for InterfaceViewport {
    fn default() -> InterfaceViewport {
        InterfaceViewport::of(None, 1.0, InterfaceScale::default().get())
    }
}

pub struct InterfaceScalePlugin;

impl Plugin for InterfaceScalePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InterfaceScale>()
            .init_resource::<InterfaceViewport>()
            .add_systems(PreUpdate, follow);
    }
}

/// Recompute the scale from the window and the two CVars, and the viewport
/// from the window and that scale.
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
    loaded: Res<crate::lua::load::LoadWanted>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut scale: ResMut<InterfaceScale>,
    mut viewport: ResMut<InterfaceViewport>,
) {
    let window = windows.single().ok();
    let height = window.map_or(layout::UI_SIZE.1, |window| f64::from(window.height()));
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
    let fitted = InterfaceViewport::of(
        window.map(|window| Vec2::new(window.width(), window.height())),
        window.map_or(1.0, Window::scale_factor),
        now.get(),
    );
    if *viewport != fitted {
        *viewport = fitted;
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
