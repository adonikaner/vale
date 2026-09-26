//! **The loading screen, on the screen** — the one thing this client draws that
//! is over the game's own interface rather than under it.
//!
//! Three quads and no widgets. 1.12's loading screen is not a `FrameXML` frame,
//! not a `GlueXML` one and not an addon: it is the client itself drawing a
//! full-viewport picture with two more quads over it, which is why nothing in
//! `Interface\` mentions it and why there was nothing to *load* here — only
//! something to write.
//!
//! ```text
//! vale_assets::tables::loading   which picture, and where the bar goes  <- measured
//! crate::game::place::loading      whether it is up, and how full         <- this client's
//! this file                 three quads                            <- egui
//! ```
//!
//! ## It is drawn on `Order::Foreground`, and that is the point
//!
//! Everything else this crate paints is ordered *under* something: the
//! interface goes on egui's background layer so the diagnostics sit over it, and
//! `ui/mod.rs` says why. This one covers all of it, including the F4 window,
//! because that is what a loading screen is — the reference draws it instead of
//! the frame rather than on top of one, and a HUD readable through a loading
//! screen would be a HUD reporting a world the player cannot see. What the
//! numbers were doing is in the log instead: [`crate::game::place::loading`] says what
//! it was waiting for when it gives up.
//!
//! ## The picture is a centred 4:3 box, and the bar's fill is squashed
//!
//! Both are the reference's, and both are the kind of thing a renderer gets
//! plausibly wrong.
//!
//! **The 4:3 box is [`vale_assets::tables::loading::fit`]** and it is the whole of
//! this round's second report. The client narrows the viewport to a centred
//! 4:3 box before drawing anything into it — so a 16:9 window gets black bars
//! either side and the picture looks the way it looked on the monitors it was
//! drawn for. This client stretched it across the window instead, which is
//! exactly the most dangerous failure mode: a complete,
//! plausible picture that is wrong. **The bar goes through the same fit**,
//! because the reference sets one viewport and draws both.
//!
//! And the **texture coordinates are a single shared constant** —
//! `(0,1) (1,1) (0,0) (1,0)`, used unchanged by the background draw *and* the
//! bar draw. So a half-full bar draws the whole of `Loading-BarFill` into half
//! the width. Clipping the texture instead — which is what a status bar
//! normally does, and what `lua::statusbar` does — would look right at full and
//! wrong everywhere else.

use crate::assets::GameAssets;
use crate::game::place::loading::LoadingScreen;
use vale_assets::tables::loading::{fit, BAR};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use std::collections::HashMap;

/// The three textures, decoded once each and held while a screen is up.
///
/// Its own tiny cache rather than [`super::framexml`]'s `Art`, which is private
/// to that pass and keyed for a different job — and because these three are the
/// one set of textures whose whole lifetime is "a screen is showing". See
/// [`forget`].
#[derive(Resource, Default)]
struct LoadingArt {
    loaded: HashMap<String, Option<egui::TextureHandle>>,
}

impl LoadingArt {
    fn texture(
        &mut self,
        ctx: &egui::Context,
        assets: &GameAssets,
        path: &str,
    ) -> Option<egui::TextureHandle> {
        if let Some(found) = self.loaded.get(path) {
            return found.clone();
        }
        let handle = super::framexml::decode_rgba(assets, path).map(|(width, height, mut rgba)| {
            let image = super::framexml::image([width as usize, height as usize], &mut rgba);
            // **Clamped, not repeated.** The fill is squashed into a narrowing
            // rectangle rather than tiled along it — see the module comment —
            // and a wrapping sampler would bleed the far edge in at every
            // width.
            ctx.load_texture(path, image, egui::TextureOptions::LINEAR)
        });
        if handle.is_none() {
            warn!("loading screen: {path} would not decode");
        }
        self.loaded.insert(path.to_string(), handle.clone());
        handle
    }
}

pub struct LoadingScreenPlugin;

impl Plugin for LoadingScreenPlugin {
    fn build(&self, app: &mut App) {
        // **No ordering against the other painters, and that is deliberate
        // rather than an omission.** What puts this over the interface and the
        // diagnostics is egui's own layer order — `Order::Foreground` sorts
        // above the background layer `framexml` paints on and above the
        // `Order::Middle` the F4 window is — and that holds whichever system
        // ran first. Writing an `.after()` here would state a dependency the
        // pixels do not have, which is the opposite failure to an unstated
        // ordering.
        app.init_resource::<LoadingArt>()
            .add_systems(EguiPrimaryContextPass, paint);
    }
}

/// Put the picture and the bar on the screen, if there is one to put.
fn paint(
    mut contexts: EguiContexts,
    assets: Res<GameAssets>,
    screen: Res<LoadingScreen>,
    mut art: ResMut<LoadingArt>,
) -> Result {
    // The mesh painter's twin draws these instead when it is on — see
    // [`crate::ui::mesh::loading`], which states its one layering divergence.
    if crate::ui::mesh::active() {
        return Ok(());
    }
    let Some(picture) = screen.picture() else {
        // **Dropped as soon as the screen comes down.** A 512x512 parchment and
        // two bar strips are ~1.3 MiB of upload that nothing will look at again
        // until the next teleport, and re-reading them then costs three
        // archive reads — which is a thing to do while a loading screen is
        // already up rather than a thing to hold a megabyte for.
        art.loaded.clear();
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?.clone();
    // `viewport_rect` rather than `content_rect`, on [`super::framexml`]'s own
    // terms: the picture fills the window and 1.12 keeps out of nothing.
    let screen_rect = ctx.viewport_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("loading-screen"),
    ));
    // The whole texture, every time — the reference's own shared constant. See
    // the module comment.
    let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

    // **Black over the whole window, always** — and it is not only the frame
    // before the picture has decoded. `fit` leaves bars on two of the four
    // edges at every aspect but 4:3, and those bars are the reference's own
    // clear (the full viewport is cleared before it is narrowed). Without this the world shows through them, half torn down,
    // which is the one thing the screen exists to hide.
    painter.rect_filled(screen_rect, 0.0, egui::Color32::BLACK);

    // …and everything else is drawn inside the box, the bar included — see
    // [`vale_assets::tables::loading::fit`], which is the one door. Computed once:
    // it is the same answer for the picture and for both bar pieces, and three
    // calls is three chances for them to disagree.
    let into = fit(aspect_of(screen_rect));
    if let Some(texture) = art.texture(&ctx, &assets, picture) {
        let box_ = to_screen(screen_rect, into, [0.0, 0.0, 1.0, 1.0]);
        painter.image(texture.id(), box_, whole, egui::Color32::WHITE);
    }

    for piece in BAR {
        let Some(texture) = art.texture(&ctx, &assets, piece.path) else {
            continue;
        };
        let rect = piece.rect(screen.progress());
        // An empty bar is a zero-width quad, which egui would still submit.
        if rect[2] <= rect[0] {
            continue;
        }
        painter.image(
            texture.id(),
            to_screen(screen_rect, into, rect),
            whole,
            egui::Color32::WHITE,
        );
    }
    Ok(())
}

/// `width / height`, which is what [`fit`] is a function of. Zero-height windows
/// answer zero rather than infinity; `fit` handles it.
fn aspect_of(screen: egui::Rect) -> f32 {
    if screen.height() > 0.0 {
        screen.width() / screen.height()
    } else {
        0.0
    }
}

/// One `[left, bottom, right, top]` in the game's own unit square — origin
/// bottom-left, `y` up — as an egui rectangle, whose `y` runs down from the top.
///
/// Two mappings in one, and both would draw a plausible picture on their own:
/// `into` is the part of the window the unit square covers ([`fit`]'s answer),
/// and the flip is that the game measures `y` up while egui measures it down.
/// Getting the second wrong puts the bar 7.5% from the *top* of a picture that
/// is otherwise perfect, which reads as art rather than as a bug.
fn to_screen(
    screen: egui::Rect,
    into: [f32; 4],
    [left, bottom, right, top]: [f32; 4],
) -> egui::Rect {
    let (l, b, r, t) = (into[0], into[1], into[2], into[3]);
    let x = |f: f32| screen.left() + screen.width() * (l + (r - l) * f);
    let y = |f: f32| screen.top() + screen.height() * (1.0 - (b + (t - b) * f));
    egui::Rect::from_min_max(egui::pos2(x(left), y(top)), egui::pos2(x(right), y(bottom)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4:3 window, where [`fit`] is the identity and the only thing left is
    /// the flip.
    fn four_by_three(height: f32) -> (egui::Rect, [f32; 4]) {
        let screen = egui::Rect::from_min_max(
            egui::pos2(0.0, 0.0),
            egui::pos2(height * 4.0 / 3.0, height),
        );
        (screen, fit(aspect_of(screen)))
    }

    /// **The bar is near the bottom of the window, not the top.** The one
    /// mistake in this file that draws a complete, plausible picture in the
    /// wrong place — see [`to_screen`].
    #[test]
    fn the_unit_square_is_flipped_onto_the_window() {
        let (screen, into) = four_by_three(900.0);
        let [_, border] = BAR;
        let rect = to_screen(screen, into, border.rect(1.0));

        // Centred horizontally, 0.6 of the width.
        assert!((rect.center().x - 600.0).abs() < 0.01, "{rect:?}");
        assert!((rect.width() - 720.0).abs() < 0.01, "{rect:?}");
        // …and in the bottom eighth of the window, with `top` above `bottom` in
        // egui's downward `y`.
        assert!(rect.top() < rect.bottom());
        assert!(rect.bottom() > 810.0, "{rect:?}");
        assert!((rect.height() - 45.0).abs() < 0.01, "{rect:?}");
    }

    /// **The picture is a centred 4:3 box and the bar is inside it**, which is
    /// the whole of the second report this round: the art stretched across a
    /// wide window is a complete, plausible, wrong picture.
    #[test]
    fn the_picture_and_its_bar_are_one_centred_box() {
        // 16:9, so the box is 1200 wide inside a 1600-wide window.
        let screen = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1600.0, 900.0));
        let into = fit(aspect_of(screen));
        let picture = to_screen(screen, into, [0.0, 0.0, 1.0, 1.0]);
        assert!((picture.width() - 1200.0).abs() < 0.5, "{picture:?}");
        assert!((picture.height() - 900.0).abs() < 0.5, "{picture:?}");
        assert!((picture.center().x - 800.0).abs() < 0.5, "{picture:?}");

        // …and the bar's frame is 60% of *that*, not of the window — a bar
        // measured against the window slides out from under its own art as the
        // window widens.
        let [_, border] = BAR;
        let bar = to_screen(screen, into, border.rect(1.0));
        assert!((bar.width() - 1200.0 * 0.6).abs() < 0.5, "{bar:?}");
        assert!(picture.contains_rect(bar), "{bar:?} outside {picture:?}");
    }

    /// A window taller than 4:3 letterboxes instead, on the same rule — and the
    /// aspect a zero-height window has is not infinity.
    #[test]
    fn a_tall_window_gets_its_bars_on_the_other_two_edges() {
        let screen = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(600.0, 1200.0));
        let picture = to_screen(screen, fit(aspect_of(screen)), [0.0, 0.0, 1.0, 1.0]);
        assert!((picture.width() - 600.0).abs() < 0.5, "{picture:?}");
        assert!((picture.height() - 450.0).abs() < 0.5, "{picture:?}");
        assert!((picture.center().y - 600.0).abs() < 0.5, "{picture:?}");

        let flat = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(600.0, 0.0));
        assert_eq!(aspect_of(flat), 0.0);
    }
}
