//! The building editor's panel in Lights mode, under the editor's heading
//! (`super::wmoview::heading`). The state is
//! [`crate::tools::placements::wmolights`].
//!
//! ```text
//! (notice)   before the first edit of a file the game ships: Copy for This
//!            Placement or Edit the Shared File
//! Lights     every light with its type and colour; a click chooses one;
//!            Add Light
//! Light      the chosen light: type, colour, intensity, attenuation, place,
//!            Duplicate and Remove
//! ```

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::placements::wmodoodads::Spawns;
use crate::tools::placements::wmolights::{self, Lamps};
use crate::tools::placements::wmoview::WmoView;
use crate::ui::widgets::theme::{self, property};
use vale_client::assets::GameAssets;
use vale_edit::wmo::Light;

use super::place::number_row;

/// What a light is, on the heading's info mark.
const ABOUT: &str = "A building's lights light the characters and creatures inside it, and the lamps at night. \
                     The walls' light is baked into the group files and does not change with them. A light \
                     added is listed in the group whose box holds it.";

/// The four types, in the file's numbering.
const TYPES: [&str; 4] = ["Omni", "Spot", "Directional", "Ambient"];

/// The panel.
#[allow(clippy::too_many_arguments)]
pub(in crate::ui::tools) fn panel(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    assets: &GameAssets,
    spawns: &mut Spawns,
    lamps: &mut Lamps,
    view: &mut WmoView,
    now: f64,
) {
    let Some(placed) = spawns.building.clone() else {
        theme::note(ui, "Drawing the building\u{2026}");
        return;
    };
    if let Err(why) = session.open_building(assets, &placed.path) {
        theme::note(ui, why);
        return;
    }
    super::wmodoodads::shared(ui, session, assets, spawns, view, &placed);
    let editable = spawns.editable(session);
    let Some(lights) = session.building(&placed.path).map(|open| open.building.root.lights()) else { return };
    egui::ScrollArea::vertical()
        .id_salt("wmo-lights")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            theme::heading_about(ui, &format!("Lights ({})", lights.len()), ABOUT);
            for (at, light) in lights.iter().enumerate() {
                let [r, g, b] = light.colour();
                let chosen = lamps.chosen == Some(at as u16);
                let clicked = ui
                    .horizontal(|ui| {
                        let (swatch, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                        ui.painter().rect_filled(swatch, 2.0, egui::Color32::from_rgb(r, g, b));
                        let kind = TYPES.get(usize::from(light.kind())).copied().unwrap_or("Unknown");
                        ui.selectable_label(chosen, format!("{at} \u{b7} {kind} \u{b7} {:.2}", light.intensity())).clicked()
                    })
                    .inner;
                if clicked {
                    lamps.chosen = Some(at as u16);
                    lamps.adding = false;
                }
            }
            ui.add_enabled_ui(editable, |ui| {
                let label = match lamps.adding {
                    true => "Click Inside the Building\u{2026}",
                    false => "Add Light",
                };
                if ui.button(label).on_hover_text("Then click where it goes: it stands a yard and a half above the click. Escape stops.").clicked() {
                    lamps.adding = !lamps.adding;
                }
                chosen(ui, session, assets, lamps, &placed, &lights, now);
            });
            ui.add_space(12.0);
        });
}

/// The chosen light's values, with Duplicate and Remove.
fn chosen(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    assets: &GameAssets,
    lamps: &mut Lamps,
    placed: &crate::tools::placements::wmodoodads::Placed,
    lights: &[Light],
    now: f64,
) {
    let Some(index) = lamps.chosen else {
        theme::heading(ui, "Light");
        theme::note(ui, "Click a light, a ball of its own colour, to choose it.");
        return;
    };
    let Some(&was) = lights.get(usize::from(index)) else {
        lamps.chosen = None;
        return;
    };
    let mut light = was;
    theme::heading_about(ui, "Light", "Drag it, or use the handles; the Keys button lists the rest. The place is in the building's own axes.");
    property(ui, "Type", "", |ui| {
        let mut kind = usize::from(light.kind()).min(TYPES.len() - 1);
        egui::ComboBox::from_id_salt("wmo-light-type").selected_text(TYPES[kind]).show_ui(ui, |ui| {
            for (at, name) in TYPES.iter().enumerate() {
                ui.selectable_value(&mut kind, at, *name);
            }
        });
        light.set_kind(kind as u8);
    });
    property(ui, "Colour", "", |ui| {
        let mut colour = light.colour();
        ui.color_edit_button_srgb(&mut colour);
        light.set_colour(colour);
    });
    let mut intensity = light.intensity();
    number_row(ui, "Intensity", "", egui::DragValue::new(&mut intensity).speed(0.01).range(0.0..=10.0).fixed_decimals(2));
    light.set_intensity(intensity);
    property(ui, "Fades", "Whether it fades out between the two distances below.", |ui| {
        let mut on = light.attenuated();
        ui.checkbox(&mut on, "");
        light.set_attenuated(on);
    });
    let [mut start, mut end] = light.attenuation();
    number_row(ui, "Fade From", "Yards.", egui::DragValue::new(&mut start).speed(0.1).range(0.0..=200.0).fixed_decimals(1));
    number_row(ui, "Fade To", "Yards: the ring drawn round the light.", egui::DragValue::new(&mut end).speed(0.1).range(0.0..=200.0).fixed_decimals(1));
    light.set_attenuation([start, end.max(start)]);
    let mut at = light.position();
    for (name, axis) in [("X", 0), ("Y", 1), ("Z", 2)] {
        number_row(ui, name, "", egui::DragValue::new(&mut at[axis]).speed(0.1).fixed_decimals(2));
    }
    light.set_position(at);
    if light != was {
        let label = match light.position() == was.position() {
            true => "Edit building light",
            false => "Move building light",
        };
        if let Err(why) = wmolights::change_light(session, assets, &placed.path, index, label, now, |edited| *edited = light) {
            session.status = why;
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Duplicate").on_hover_text("Ctrl+D. Adds a copy a yard north, and chooses it.").clicked() {
            session.status = match wmolights::duplicate(session, assets, placed, was, now) {
                Ok(added) => {
                    lamps.chosen = Some(added);
                    format!("added light {added}, a copy of light {index}")
                }
                Err(why) => why,
            };
        }
        if ui.button("Remove").on_hover_text("Delete. Takes the light out, and out of every group's list.").clicked() {
            session.status = match wmolights::remove_light(session, assets, &placed.path, index, now) {
                Ok(()) => format!("removed light {index}"),
                Err(why) => why,
            };
            lamps.chosen = None;
        }
    });
}
