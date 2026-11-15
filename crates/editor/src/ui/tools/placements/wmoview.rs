//! The building editor's toolbar, and the heading of its panel. The state is
//! [`crate::tools::placements::wmoview`].
//!
//! ```text
//! toolbar     Back to the World  name.wmo  [Areas | Doodads | Lights]  Keys
//!             Handles [Move | Turn | Off]             in Doodads and Lights
//! inspector   Building     the model and the placement it was opened from
//!             then the mode's own panel: super::wmoareas or super::wmodoodads
//! ```

use bevy_egui::egui;

use crate::tools::placements::gizmo::{Handles, HANDLES};
use crate::tools::placements::place::{Placing, Switch, BUILDING_SWITCHES};
use crate::tools::placements::wmoview::WmoView;
use crate::ui::widgets::theme::{self, bar_group, bar_keys, bar_segment, bar_setting, property};

/// Areas mode's mouse and keyboard controls, for the Keys button.
const AREAS_KEYS: [(&str, &str); 5] = [
    ("Left button", "Choose the group under the pointer"),
    ("Ctrl + left button", "Choose another group beside it, or let it go"),
    ("Pointer over the building", "Outline the group a click chooses"),
    ("1, 2, 3", "Areas, Doodads, Lights"),
    ("Escape", "Back to the world"),
];

/// Doodads mode's mouse and keyboard controls, for the Keys button. The
/// doodad tool's, on the building's own doodads.
const DOODADS_KEYS: [(&str, &str); 15] = [
    ("Left button", "Choose the doodad under the pointer"),
    ("Drag", "Move it across the level it stands at"),
    ("Ctrl + drag", "Stand it on the surface under the pointer"),
    ("Arrows", "Nudge 0.5 yd"),
    ("Page Up, Page Down", "Raise, lower 0.5 yd"),
    (", and .", "Turn about up 5\u{b0}"),
    ("Shift + any of those keys", "Ten times as far"),
    ("Alt + mouse", "Turn"),
    ("Ctrl + Alt + mouse", "Snap the turn"),
    ("Ctrl + wheel", "Scale"),
    ("Delete", "Remove"),
    ("Ctrl + D", "Put a copy on the cursor"),
    ("Left button, a model on the cursor", "Add it there; it stays on the cursor"),
    ("1, 2, 3", "Areas, Doodads, Lights"),
    ("Escape", "Put the model down, let go of the doodad, then back to the world"),
];

/// Lights mode's controls, for the Keys button.
const LIGHTS_KEYS: [(&str, &str); 9] = [
    ("Left button near a light", "Choose it"),
    ("Drag", "Move it across its level"),
    ("Ctrl + drag", "Stand it over the surface under the pointer"),
    ("Arrows", "Nudge 0.5 yd"),
    ("Page Up, Page Down", "Raise, lower 0.5 yd"),
    ("Delete", "Remove"),
    ("Ctrl + D", "A copy a yard north"),
    ("1, 2, 3", "Areas, Doodads, Lights"),
    ("Escape", "Stop adding, let go of the light, then back to the world"),
];

/// The building editor's toolbar over the viewport.
pub(crate) fn toolbar(
    ctx: &egui::Context,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    placing: &mut Placing,
    handles: &mut Handles,
    view: &mut WmoView,
) -> Option<egui::Rect> {
    let leaf = view.open.as_ref().map_or(String::new(), |open| open.leaf().to_string());
    theme::toolbar(ctx, "building-editor-toolbar", viewport, corner, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let back = ui
                .add(egui::Button::new("Back to the World").min_size(egui::vec2(0.0, theme::BAR_HEIGHT)))
                .on_hover_text("Escape. Closes the building editor and puts the camera back where it was.");
            if back.clicked() {
                view.leave = true;
            }
            ui.label(egui::RichText::new(leaf).color(theme::INK));
            let was = placing.switch();
            bar_group(ui, |ui| {
                for (name, to) in BUILDING_SWITCHES {
                    let hover = match to {
                        Switch::Areas => {
                            "Choose a group of the building, and edit what it is called, which zone it \
                             counts as and what plays there."
                        }
                        Switch::Lights => "Choose, move, add and remove the building's lights, and set their colour and reach.",
                        _ => "Choose, move, add and remove the doodads the building's file carries, and its doodad sets.",
                    };
                    if bar_segment(ui, was == to, name).on_hover_text(hover).clicked() && was != to {
                        placing.set_switch(to);
                    }
                }
            });
            bar_keys(ui, match was {
                Switch::Areas => &AREAS_KEYS,
                Switch::Lights => &LIGHTS_KEYS,
                _ => &DOODADS_KEYS,
            });
        });
        if matches!(placing.switch(), Switch::Doodads | Switch::Lights) {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                bar_setting(ui, "Handles", |ui| {
                    bar_group(ui, |ui| {
                        for (name, to) in HANDLES {
                            let hover = match to {
                                Handles::Move => "Drag an arrow to move along one axis.",
                                Handles::Turn => "Drag a ring to turn about one axis.",
                                Handles::Off => "No handles: a click chooses and a drag moves across the level.",
                            };
                            if bar_segment(ui, *handles == to, name).on_hover_text(hover).clicked() {
                                *handles = to;
                            }
                        }
                    });
                });
            });
        }
    })
}

/// The heading of the building editor's panel: the model, and the placement
/// it was opened from.
pub(in crate::ui::tools) fn heading(ui: &mut egui::Ui, view: &WmoView) {
    let Some(open) = &view.open else { return };
    theme::heading_about(
        ui,
        "Building",
        "The building alone, at its file's own origin: the numbers shown are the file's. Edits to its \
         doodads change the file, which every placement of it draws.",
    );
    property(ui, "Model", "", |ui| {
        theme::value(ui, open.leaf()).on_hover_text(open.path.clone());
    });
    let from = match open.opened_on_the_map() {
        true => "the map's building".to_string(),
        false => format!("WMO {}", open.from),
    };
    property(ui, "Opened From", "Its name set is the second key of the building's area rows.", |ui| {
        theme::value(ui, format!("{from}, name set {}", open.name_set));
    });
}
