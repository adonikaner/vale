//! The render tab: how the frame is produced, and what is drawn over it.
//!
//! The tab has two sets of switches. The first three are [`RenderTuning`]:
//! MSAA, vsync and sun shadows. Each trades CPU, GPU and image quality in the
//! finished frame. The seven under them are [`DebugOverlay`], which are not
//! part of the finished frame: they are instruments drawn on top of it.
//!
//! The world's own eighteen switches are a third kind and are on their own
//! tab, because they are subtractions; see [`crate::render::tuning`] for the
//! distinction.
//!
//! ## The keys and the checkboxes write the same resource
//!
//! F5, F9 and F10 write [`RenderTuning`] and so do these checkboxes, so a key
//! and a checkbox cannot disagree. The keys exist so a setting can be compared
//! within one login; a panel that held its own copy would be a third state to
//! reconcile.

use bevy_egui::egui;

use crate::render::overlay::{DebugOverlay, OVERLAYS};
use crate::world::camera::RenderTuning;

use super::{row, Readout, BAD, DIM, GOOD, WARN};

pub fn show(
    ui: &mut egui::Ui,
    tuning: &mut RenderTuning,
    overlay: &mut DebugOverlay,
    params: &Readout,
) {
    ui.strong("Rendering");
    ui.colored_label(DIM, "the same three F5, F9 and F10 toggle, over the same resource");
    // Drawn from `crate::world::camera::SWITCHES` rather than written out, so
    // a setting added to `RenderTuning` gets a checkbox with no change here.
    for (key, label, field) in crate::world::camera::SWITCHES {
        ui.checkbox(field(tuning), format!("{key:<3} {label}"));
    }
    // Vsync masks every measurement on the frame tab. Under it a frame is 16.7
    // or 33.3 ms and nothing in between, so a scene that misses the budget by a
    // millisecond reads as the frame rate halving, and the wait itself is
    // indistinguishable from CPU cost in the frame-against-spans comparison
    // that tab opens with.
    if tuning.vsync {
        ui.colored_label(
            DIM,
            "vsync is on. The frame tab's timings are quantised to the display.",
        );
    }

    ui.add_space(10.0);
    ui.strong("Diagnostic overlays");
    ui.colored_label(
        DIM,
        "drawn over the finished frame; they show what the picture alone does not",
    );
    for (label, help, field) in OVERLAYS {
        // The wireframe is the one switch here that the machine can refuse. When
        // it is unsupported the checkbox is greyed out with the reason beside
        // it, rather than ticking and doing nothing. See
        // `crate::render::overlay::WireframeSupported`.
        let available = label != "wireframe" || params.wireframe_supported.available();
        ui.add_enabled_ui(available, |ui| {
            ui.checkbox(field(overlay), label).on_hover_text(help);
        });
        if !available {
            ui.colored_label(BAD, format!("  {}", params.wireframe_supported.why()));
        }
    }

    // The two sliders, under the switches they belong to and enabled only
    // while those are on, since a slider for an overlay that is not drawn has
    // no visible effect.
    ui.add_space(4.0);
    ui.add_enabled(
        overlay.wireframe && params.wireframe_supported.available(),
        egui::Slider::new(&mut overlay.wireframe_width, 1.0..=4.0)
            .text("wireframe width")
            .fixed_decimals(1),
    );
    ui.add_enabled(
        overlay.collision,
        egui::Slider::new(&mut overlay.collision_radius, 5.0..=120.0)
            .text("collision radius (yards)")
            .fixed_decimals(0),
    );
    if overlay.collision {
        // What the overlay drew and whether its cap dropped anything, because
        // a silently truncated overlay reads as all the collision there is.
        // See `overlay::MAX_COLLISION_TRIANGLES`.
        let drawn = &params.collision_drawn;
        row(
            ui,
            "  drawing",
            format!("{} hulls · {} triangles", drawn.hulls, drawn.triangles),
        );
        if drawn.capped {
            ui.colored_label(WARN, "  Capped. Narrow the radius to see all of it.");
        }
        if drawn.hulls == 0 {
            // Not a warning: nothing solid within the radius is the ordinary
            // case in an open field outdoors. Zero means there is no collision
            // here, which answers a report of falling through the floor.
            ui.colored_label(DIM, "  Nothing solid within the radius.");
        }
    }

    // Which overlays are on, so an overlay left on explains a world that looks
    // wrong later. The world tab's `not drawn` line does the same for
    // subtractions: there something is missing, here something is drawn on
    // top.
    let on: Vec<&str> = OVERLAYS
        .iter()
        .filter(|(_, _, field)| *field(overlay))
        .map(|(label, _, _)| *label)
        .collect();
    ui.add_space(6.0);
    if on.is_empty() {
        ui.colored_label(GOOD, "No overlay is drawn.");
    } else {
        ui.colored_label(WARN, format!("Drawn over the world: {}.", on.join(", ")));
        // The `--overlay` flag that reproduces this state, so a set of
        // overlays chosen by clicking can be used in a scripted screenshot.
        // The world tab prints the same line for its subtractions.
        ui.colored_label(
            DIM,
            format!(
                "--overlay {}",
                on.iter()
                    .map(|label| label.replace(' ', ""))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        );
    }
}
