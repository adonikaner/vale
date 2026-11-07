//! The text drawn over the world at a projected point. The labels and the
//! passes that write them are [`crate::labels`].
//!
//! Painted on the background layer and clipped to the viewport, as the
//! service marks are ([`super::shell::servicemarks`]), so every panel, window
//! and toolbar is drawn over a label and a label takes no press.
//!
//! Each label is its text in its colour on a dark, rounded, semi-opaque plate
//! a few points larger than the text. The ground and the lines behind a label
//! can be any colour, including the label's own: the measure tool's distance
//! is drawn in the colour of the line it sits on. The plate keeps the text
//! readable on all of them, and its transparency leaves the ground under it
//! visible. A [`Weight::Strong`] label is larger and is painted twice half a
//! point apart, which thickens its strokes; egui's default fonts have no bold
//! face.

use bevy::color::ColorToPacked;
use bevy_egui::egui;

use crate::labels::{Labels, Weight};

/// The text size of a [`Weight::Normal`] and a [`Weight::Strong`] label, in
/// points.
const NORMAL_SIZE: f32 = 16.0;
const STRONG_SIZE: f32 = 24.0;

/// How far the plate reaches past the text on each side, in points, as
/// (across, up and down).
const PLATE_MARGIN: egui::Vec2 = egui::vec2(5.0, 2.0);

/// The plate's corner radius, in points, and its opacity out of 255.
const PLATE_ROUNDING: f32 = 4.0;
const PLATE_ALPHA: u8 = 185;

/// Paint every label inside `viewport`.
///
/// A label's position is in physical pixels and the painter works in points,
/// so each is divided by egui's pixels per point, which is the window's scale
/// factor times egui's zoom.
pub(crate) fn draw(ctx: &egui::Context, viewport: egui::Rect, labels: &Labels) {
    if !viewport.is_positive() {
        return;
    }
    let painter = ctx
        .layer_painter(egui::LayerId::background())
        .with_clip_rect(viewport);
    let per_point = ctx.pixels_per_point().max(0.01);
    let plate = egui::Color32::from_black_alpha(PLATE_ALPHA);
    for label in labels.iter() {
        let at = egui::pos2(label.at.x / per_point, label.at.y / per_point);
        let size = match label.weight {
            Weight::Normal => NORMAL_SIZE,
            Weight::Strong => STRONG_SIZE,
        };
        // A label wholly outside the viewport is not laid out.
        let reach = egui::Rect::from_center_size(at, egui::vec2(size * 8.0, size * 2.0));
        if !viewport.intersects(reach) {
            continue;
        }
        let [r, g, b, a] = label.colour.to_srgba().to_u8_array();
        let colour = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
        let galley = painter.layout_no_wrap(
            label.text.clone(),
            egui::FontId::proportional(size),
            colour,
        );
        let corner = egui::Align2::CENTER_CENTER.anchor_size(at, galley.size()).min;
        let behind = egui::Rect::from_min_size(corner, galley.size()).expand2(PLATE_MARGIN);
        painter.rect_filled(behind, PLATE_ROUNDING, plate);
        painter.galley(corner, galley.clone(), colour);
        if label.weight == Weight::Strong {
            painter.galley(corner + egui::vec2(0.5, 0.0), galley, colour);
        }
    }
}
