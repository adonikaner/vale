//! **The loading screen as meshes** — the mesh painter's form of
//! [`crate::ui::loading`], same three quads through the same
//! [`vale_assets::tables::loading::fit`] door: the black clear over the
//! whole window, the picture in its centred 4:3 box, and the bar whose fill
//! is squashed rather than clipped (the module comment over the egui twin
//! carries the argument for all three).
//!
//! **One stated divergence from the egui twin's layering.** egui's
//! `Order::Foreground` put the loading screen over the F4 window; a mesh
//! draws under egui by construction, so with the mesh painter on, the
//! diagnostics float over the loading screen. That is the diagnostic surface
//! doing its job — a load is exactly when the frame tab is worth reading —
//! and the game's own interface is still fully covered ([`LOADING_Z`] is
//! above every band the painter uses).

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
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

/// The picture and the bar, if there is a screen to put up — the mesh twin of
/// `loading::paint`. Rebuilt every frame while up: the bar moves, and the
/// whole thing is three quads.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    mut commands: Commands,
    screen: Res<crate::game::place::loading::LoadingScreen>,
    assets: Res<crate::assets::GameAssets>,
    mut textures: ResMut<UiTextures>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<InterfaceMaterial>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut state: Local<LoadState>,
) {
    let state = &mut *state;
    let Ok(window) = windows.single() else { return };
    let size = Vec2::new(window.width(), window.height());
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
/// `[left, top, right, bottom]` window pixels (y down) — the egui twin's
/// `to_screen`, whose tests pin the flip and the box; this is the same
/// arithmetic over bare floats.
fn to_screen(size: Vec2, into: [f32; 4], [left, bottom, right, top]: [f32; 4]) -> [f32; 4] {
    let (l, b, r, t) = (into[0], into[1], into[2], into[3]);
    let x = |f: f32| size.x * (l + (r - l) * f);
    let y = |f: f32| size.y * (1.0 - (b + (t - b) * f));
    [x(left), y(top), x(right), y(bottom)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two assertions the egui twin's tests make, over this port's own
    /// arithmetic — the same wrong-place failure modes, pinned on both sides
    /// of the painter switch.
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
}
