//! The New map form: a zone map, a continent or a single map, the zone or
//! map it is for, and its directory. See
//! [`crate::import::worldmap::rows`] for what each adds.

use bevy_egui::egui;

use super::Work;
use crate::import::worldmap::rows::{self, NewMap};
use crate::import::worldmap::{Ask, Kind};
use crate::ui::widgets::theme;

/// Draw the form.
pub(super) fn form(ui: &mut egui::Ui, work: &mut Work<'_>) {
    let Some(mut form) = work.maps.create.clone() else { return };
    ui.label(egui::RichText::new("New map").strong().size(17.0).color(theme::INK));
    ui.allocate_ui(egui::vec2(ui.available_width().min(380.0), 24.0), |ui| {
        let was = form.kind;
        theme::segmented(
            ui,
            &mut form.kind,
            &[("Zone map", Kind::Zone), ("Continent", Kind::Continent), ("Single map", Kind::Single)],
            |a, b| a == b,
        );
        if form.kind != was {
            form = NewMap { kind: form.kind, ..NewMap::default() };
        }
    });
    theme::note(
        ui,
        match form.kind {
            Kind::Zone => "A picture of one zone on a continent. The continent's drop-down lists it, and on the zone the \
                           world map opens on it.",
            Kind::Continent => "A map of its own in the continent drop-down, with its own zones, placed on the whole \
                                world's picture. Its WorldMapContinent row also gives the map a flight map.",
            _ => "One picture for a map with no continent, as a battleground has: on that map the world map opens on \
                  it, with no zones.",
        },
    );
    ui.add_space(4.0);
    match form.kind {
        Kind::Zone => {
            let zones = rows::zones_without_map(work.session);
            let shown = form
                .area
                .and_then(|id| zones.iter().find(|(z, _, _)| *z == id))
                .map_or_else(|| "Choose a zone".to_string(), |(id, name, map)| format!("{name} ({id}), map {map}"));
            theme::property(ui, "Zone", "A zone with no map yet, on a map that has a continent.", |ui| {
                egui::ComboBox::from_id_salt("new-map-zone").selected_text(shown).width(280.0).show_ui(ui, |ui| {
                    for (id, name, map) in &zones {
                        if ui.selectable_label(form.area == Some(*id), format!("{name} ({id}), map {map}")).clicked() {
                            form.area = Some(*id);
                            form.directory = rows::directory_from(name);
                        }
                    }
                });
            });
            if zones.is_empty() {
                theme::note(ui, "Every zone on a continent's map already has a map. Make a zone in Zones first.");
            }
        }
        Kind::Continent | Kind::Single => {
            let single = form.kind == Kind::Single;
            let maps: Vec<(u32, String, bool)> =
                rows::maps_without_continent(work.session).into_iter().filter(|(_, _, has)| !single || !has).collect();
            let shown = form
                .map
                .and_then(|id| maps.iter().find(|(m, _, _)| *m == id))
                .map_or_else(|| "Choose a map".to_string(), |(id, directory, _)| format!("{directory} ({id})"));
            theme::property(ui, "Map", "A map with no continent yet.", |ui| {
                egui::ComboBox::from_id_salt("new-map-map").selected_text(shown).width(280.0).show_ui(ui, |ui| {
                    for (id, directory, _) in &maps {
                        if ui.selectable_label(form.map == Some(*id), format!("{directory} ({id})")).clicked() {
                            form.map = Some(*id);
                            form.area = None;
                            form.directory = rows::directory_from(directory);
                        }
                    }
                });
            });
            if single {
                if let Some(map) = form.map {
                    let zones = rows::zones_of_map(work.session, map);
                    let shown = form
                        .area
                        .and_then(|id| zones.iter().find(|(z, _)| *z == id))
                        .or(zones.first())
                        .map_or_else(|| "The map has no zone".to_string(), |(id, name)| format!("{name} ({id})"));
                    theme::property(
                        ui,
                        "Zone",
                        "The zone the map is of, which names it in the list. A single map's row needs one: a row \
                         with no area is a continent.",
                        |ui| {
                        egui::ComboBox::from_id_salt("new-map-single-zone").selected_text(shown).width(280.0).show_ui(ui, |ui| {
                            for (id, name) in &zones {
                                if ui.selectable_label(form.area == Some(*id), format!("{name} ({id})")).clicked() {
                                    form.area = Some(*id);
                                }
                            }
                        });
                    },
                    );
                    if zones.is_empty() {
                        theme::note(ui, "This map has no zone. Make one in Zones first.");
                    }
                }
            }
        }
        Kind::World => {}
    }
    theme::property(
        ui,
        "Directory",
        "The folder under Interface\\WorldMap\\ its pieces go in, and the start of their names. Letters and digits.",
        |ui| {
            ui.add(egui::TextEdit::singleline(&mut form.directory).desired_width(200.0));
        },
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let ready = !form.directory.is_empty()
            && match form.kind {
                Kind::Zone => form.area.is_some(),
                _ => form.map.is_some(),
            };
        if ui
            .add_enabled(ready, egui::Button::new("Create"))
            .on_hover_text("Add the rows, as one undo entry, and fit the rectangle once the map's tiles are read.")
            .clicked()
        {
            work.maps.ask = Some(Ask::Create);
        }
        if ui.button("Cancel").clicked() {
            work.maps.create = None;
        }
    });
    if let Some(said) = &work.maps.said {
        theme::note(ui, said.clone());
    }
    if work.maps.create.is_some() {
        work.maps.create = Some(form);
    }
}
