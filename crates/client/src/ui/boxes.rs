//! **What a diagnostic box is standing in for**, written over it.
//!
//! `world::entities::fallback` draws a grey `Cuboid` for anything the server
//! described and this client could not draw, and its own doc calls that a
//! diagnostic. It was not one. A box says *something is wrong here* and nothing
//! else — not which display id, not which model, not why — so the only way to
//! get from a box on the screen to a cause was to stand in front of it and
//! subtract render passes one at a time.
//!
//! That is exactly what happened: a report of "campfires and braziers have grey
//! boxes in them" took six rounds of eliminating passes, and every one of them
//! was avoidable. The box knew the answer the whole time.
//!
//! So this draws it. Behind `F4` → **render** → *error boxes*, off by default
//! like every other overlay, and it costs nothing at all while it is off — the
//! reason is a `String` already on the entity, and with the switch down the
//! system returns before it queries anything.
//!
//! ## Why it is here and not in `render::overlay`
//!
//! The other six visualisations are `Gizmos`, and gizmos have no text. This is
//! egui at a projected world position, which is `worldtext`'s arrangement one
//! module over — the same camera, the same `world_to_viewport`, the same
//! physical-to-logical division. It shares the switch table with the other six
//! because a person looking for it will look on the render tab.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::world::entities::fallback::FallbackReason;

/// How far above the box's own origin the line sits, in yards.
///
/// The cube is `Cuboid::new(1.0, 2.0, 1.0)` centred on the entity, so its top
/// is one yard up; this clears it rather than sitting inside it.
const OVER: f32 = 1.4;

/// Past this, in yards, the line is not drawn.
///
/// A field of forty broken creatures at three hundred yards is unreadable and
/// costs a projection each. Close enough to be pointing at something is the
/// whole use.
const REACH: f32 = 80.0;

pub struct ErrorBoxPlugin;

impl Plugin for ErrorBoxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(bevy_egui::EguiPrimaryContextPass, label);
    }
}

/// Write each box's reason over it.
fn label(
    mut contexts: EguiContexts,
    overlay: Res<crate::render::overlay::DebugOverlay>,
    boxes: Query<(&FallbackReason, &GlobalTransform)>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<(&Camera, &Transform), With<crate::world::camera::WorldCamera>>,
) -> Result {
    // **The whole cost with the switch down**, which is its ordinary state.
    if !overlay.boxes || boxes.is_empty() {
        return Ok(());
    }
    let Ok(window) = windows.single() else {
        return Ok(());
    };
    // **By its marker, not the first active camera** — `render::portraits`
    // keeps one per unit frame with an image target, and projecting through one
    // of those puts the text in a 64-pixel portrait. `worldtext` makes the same
    // point at greater length.
    let Ok((camera, placed)) = camera.single() else {
        return Ok(());
    };
    let eye = GlobalTransform::from(*placed);
    let ctx = contexts.ctx_mut()?.clone();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("error boxes"),
    ));
    let from = eye.translation();
    for (reason, at) in &boxes {
        // **Only the failures**, which is the population that has a box under
        // it. Most things in the world with no model are meant to have none —
        // see `world::entities::fallback::deserves_a_box` — and labelling those
        // would write a line over empty air on every campfire in the game.
        if !reason.fault {
            continue;
        }
        let at = at.translation();
        if at.distance_squared(from) > REACH * REACH {
            continue;
        }
        let Ok(pixels) = camera.world_to_viewport(&eye, at + Vec3::Y * OVER) else {
            continue;
        };
        // Physical pixels out of the projection, logical into egui — the toll
        // `worldtext` pays too, and the reason text drawn without it walks away
        // from the thing it names on a scaled display.
        let pixels = pixels / window.scale_factor();
        let point = egui::pos2(pixels.x, pixels.y);
        let font = egui::FontId::monospace(12.0);
        // A shadow first, because this lands over a lit world as often as a
        // dark one and a single colour is unreadable in one of them.
        painter.text(
            point + egui::vec2(1.0, 1.0),
            egui::Align2::CENTER_BOTTOM,
            &reason.why,
            font.clone(),
            egui::Color32::from_black_alpha(200),
        );
        painter.text(
            point,
            egui::Align2::CENTER_BOTTOM,
            &reason.why,
            font,
            egui::Color32::from_rgb(255, 200, 120),
        );
    }
    Ok(())
}
