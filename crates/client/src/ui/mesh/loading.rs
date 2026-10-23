//! The loading screen: the one thing this client draws over the game's own
//! interface rather than under it.
//!
//! Three quads and no widgets. 1.12's loading screen is not a `FrameXML`
//! frame, not a `GlueXML` one and not an addon: the client itself draws a
//! full-viewport picture with two more quads over it, which is why nothing in
//! `Interface\` mentions it.
//!
//! ```text
//! vale_assets::tables::loading   which picture, and where the bar goes
//! crate::glue::loading           whether it is up, and how full
//! this file                      three quads
//! ```
//!
//! ## The picture is a centred 4:3 box, and the bar's fill is squashed
//!
//! The client narrows the viewport to a centred 4:3 box before drawing
//! anything into it ([`vale_assets::tables::loading::fit`]), so a 16:9 window
//! gets black bars either side and the picture looks the way it looked on
//! the monitors it was drawn for. The bar goes through the same fit, because
//! the client sets one viewport and draws both. The bars are the client's own
//! clear of the full viewport, so the whole window is cleared to black first:
//! without that the world shows through them, half torn down.
//!
//! The texture coordinates are one shared constant, `(0,1) (1,1) (0,0)
//! (1,0)`, used unchanged by the background draw and the bar draw. So a
//! half-full bar draws the whole of `Loading-BarFill` into half the width.
//! Clipping the texture instead, which is what a status bar does, would look
//! right at full and wrong everywhere else.
//!
//! ## Layering
//!
//! The batches draw over the whole interface ([`LOADING_Z`] is above every
//! band the painter uses) and under egui, so the F4 window and the HUD float
//! over the loading screen. A load is when the frame tab is worth reading.

use bevy::prelude::*;
use std::collections::HashMap;

use vale_assets::tables::loading::{fit, BAR};

use super::material::{Blend, InterfaceMaterial};
use super::textures::UiTextures;

/// Over the whole interface (1.0+) and the carried icon (900); under nothing
/// but egui's own pass.
const LOADING_Z: f32 = 950.0;

#[derive(Default)]
pub(super) struct LoadState {
    slots: Vec<super::Slot>,
    materials: HashMap<(AssetId<Image>, Blend), Handle<InterfaceMaterial>>,
}

/// The picture and the bar, if there is a screen to put up. Rebuilt every
/// frame while up: the bar moves, and the whole thing is three quads.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    mut commands: Commands,
    screen: Res<crate::glue::loading::LoadingScreen>,
    assets: Res<crate::assets::GameAssets>,
    mut textures: ResMut<UiTextures>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<InterfaceMaterial>>,
    viewport: Res<crate::ui::scale::InterfaceViewport>,
    mut state: Local<LoadState>,
) {
    let state = &mut *state;
    let Some(size) = viewport.window else { return };
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }

    let mut emit = super::build::Emitter::new();
    if let Some(picture) = screen.picture() {
        // The black clear first — the bars `fit` leaves are the reference's
        // own, and the world must not show through them.
        let white = textures.white(&mut images);
        emit.quad(
            &white,
            Blend::Opaque,
            [0.0, 0.0, size.x, size.y],
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
            None,
        );
        let into = fit(if size.y > 0.0 { size.x / size.y } else { 0.0 });
        emit.barrier();
        if let Some(image) = textures.texture(&mut images, &assets, picture) {
            emit.quad(
                &image,
                Blend::Alpha,
                to_screen(size, into, [0.0, 0.0, 1.0, 1.0]),
                [0.0, 0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
                None,
            );
        }
        for piece in BAR {
            let Some(image) = textures.texture(&mut images, &assets, piece.path) else {
                continue;
            };
            let rect = piece.rect(screen.progress());
            if rect[2] <= rect[0] {
                continue;
            }
            emit.barrier();
            emit.quad(
                &image,
                Blend::Alpha,
                to_screen(size, into, rect),
                [0.0, 0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
                None,
            );
        }
    } else if state.slots.is_empty() {
        return;
    }
    let mut batches = emit.into_batches();
    for (index, batch) in batches.iter_mut().enumerate() {
        batch.z = LOADING_Z + index as f32 * 0.001;
    }
    super::apply(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut state.materials,
        &mut state.slots,
        batches,
        size,
    );
}

/// One `[left, bottom, right, top]` in the game's unit square (y up) as
/// `[left, top, right, bottom]` window pixels (y down).
///
/// Two mappings in one, and both would draw a plausible picture on their own:
/// `into` is the part of the window the unit square covers ([`fit`]'s answer),
/// and the flip is that the game measures `y` up while the window measures it
/// down. Getting the second wrong puts the bar 7.5% from the top of a picture
/// that is otherwise right, which reads as art rather than as a bug.
fn to_screen(size: Vec2, into: [f32; 4], [left, bottom, right, top]: [f32; 4]) -> [f32; 4] {
    let (l, b, r, t) = (into[0], into[1], into[2], into[3]);
    let x = |f: f32| size.x * (l + (r - l) * f);
    let y = |f: f32| size.y * (1.0 - (b + (t - b) * f));
    [x(left), y(top), x(right), y(bottom)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bar is near the bottom of the window, not the top, and the
    /// picture is a centred 4:3 box with the bar inside it. Both mistakes
    /// draw a complete, plausible picture in the wrong place.
    #[test]
    fn the_bar_is_near_the_bottom_and_inside_the_centred_box() {
        // 4:3, where `fit` is the identity: the flip alone.
        let size = Vec2::new(1200.0, 900.0);
        let [_, border] = BAR;
        let [l, t, r, b] = to_screen(size, fit(size.x / size.y), border.rect(1.0));
        assert!(((l + r) / 2.0 - 600.0).abs() < 0.01);
        assert!((r - l - 720.0).abs() < 0.01);
        assert!(t < b, "top above bottom in y-down pixels");
        assert!(b > 810.0, "the bottom eighth of the window");

        // 16:9: the picture is a centred 1200-wide box in a 1600 window, and
        // the bar is 60% of the box, not of the window.
        let size = Vec2::new(1600.0, 900.0);
        let into = fit(size.x / size.y);
        let [pl, pt, pr, pb] = to_screen(size, into, [0.0, 0.0, 1.0, 1.0]);
        assert!((pr - pl - 1200.0).abs() < 0.5);
        assert!((pb - pt - 900.0).abs() < 0.5);
        assert!(((pl + pr) / 2.0 - 800.0).abs() < 0.5);
        let [bl, bt, br, bb] = to_screen(size, into, border.rect(1.0));
        assert!((br - bl - 720.0).abs() < 0.5);
        assert!(bl >= pl && br <= pr && bt >= pt && bb <= pb, "the bar stays on its art");
    }

    /// A window taller than 4:3 letterboxes instead, on the same rule.
    #[test]
    fn a_tall_window_gets_its_bars_on_the_other_two_edges() {
        let size = Vec2::new(600.0, 1200.0);
        let [l, t, r, b] = to_screen(size, fit(size.x / size.y), [0.0, 0.0, 1.0, 1.0]);
        assert!((r - l - 600.0).abs() < 0.5);
        assert!((b - t - 450.0).abs() < 0.5);
        assert!(((t + b) / 2.0 - 600.0).abs() < 0.5);
    }

}
