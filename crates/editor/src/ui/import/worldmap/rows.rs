//! The chosen map's settings, under its picture: the directory, the
//! rectangle and Fit to map, a continent's place on the whole world's
//! picture, Write zone grid, Draw highlight, and Remove map.

use bevy_egui::egui;

use super::Work;
use crate::import::worldmap::{rows, AfterScan, Ask, Entry, Kind, Which};
use crate::ui::widgets::theme;
use vale_assets::tables::worldmap::MapArea;

/// The settings section.
pub(super) fn settings(ui: &mut egui::Ui, work: &mut Work<'_>, entry: &Entry) {
    let Some(row) = entry.area.clone() else {
        theme::heading(ui, "The whole world");
        theme::note(
            ui,
            "The whole world has no row. Each continent is placed on its picture by its own WorldMapContinent \
             row; the Continents view above shows and drags them.",
        );
        return;
    };
    theme::heading(ui, "Map");
    // The directory, applied on Enter or when the field loses focus.
    let key = egui::Id::new(("world-map-directory", row.id));
    let mut text = ui.data(|data| data.get_temp::<String>(key)).unwrap_or_else(|| row.directory.clone());
    theme::property(
        ui,
        "Directory",
        "The folder under Interface\\WorldMap\\ and the start of the pieces' names. A new name finds the \
         pieces under that name; the old pieces are not moved.",
        |ui| {
            let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(200.0));
            if edit.lost_focus() && text != row.directory {
                match rows::set_directory(work.session, row.id, &text) {
                    Ok(()) => work.maps.said = Some(format!("the map's directory is now {text}")),
                    Err(why) => {
                        work.maps.said = Some(why);
                        text = row.directory.clone();
                    }
                }
            }
        },
    );
    ui.data_mut(|data| data.insert_temp(key, text));
    rectangle(ui, work, &row);
    let scanning = work.maps.scans.running.as_ref().filter(|running| running.map == row.map).map(|running| running.progress.clone());
    ui.horizontal_wrapped(|ui| {
        let fit = ui.add_enabled(scanning.is_none(), egui::Button::new("Fit to map")).on_hover_text(match entry.kind {
            Kind::Zone => "Set the rectangle round the chunks painted with this zone or its sub-areas, with a margin, \
                           in the picture's shape. Reads the map's saved tiles.",
            _ => "Set the rectangle round every chunk of the map, with a margin, in the picture's shape. Reads the \
                  map's saved tiles.",
        });
        if fit.clicked() {
            scan(work, AfterScan::FitRect);
        }
        if entry.kind == Kind::Continent {
            let grid = ui.add_enabled(scanning.is_none(), egui::Button::new("Write zone grid")).on_hover_text(format!(
                "Write Interface\\WorldMap\\{}.zmp from the areas painted on the map's saved tiles. It names the zone \
                 under the pointer on the continent's picture, and a click there opens that zone's map.",
                entry.directory
            ));
            if grid.clicked() {
                scan(work, AfterScan::WriteGrid);
            }
        }
        if matches!(entry.kind, Kind::Zone | Kind::Continent) {
            let about = match entry.kind {
                Kind::Zone => "Draw the sheet that lights this zone on its continent's picture under the pointer, from \
                               the areas painted on the map's saved tiles.",
                _ => "Draw the sheet that lights this continent on the whole world's picture under the pointer, from \
                      the tiles the map has.",
            };
            if ui.add_enabled(scanning.is_none(), egui::Button::new("Draw highlight")).on_hover_text(about).clicked() {
                scan(work, AfterScan::WriteHighlight);
            }
        }
        if ui
            .button("Remove map")
            .on_hover_text(
                "Remove this WorldMapArea row and its overlays' rows. Ctrl+Z puts them back; the pictures stay in \
                 the project.",
            )
            .clicked()
        {
            let said = rows::remove(work.session, entry);
            work.maps.said = Some(said);
        }
    });
    if let Some(progress) = scanning {
        let label = format!("tile {} of {}", progress.done(), progress.total());
        theme::progress(ui, progress.fraction(), "tiles", &label, ui.available_width());
        ui.ctx().request_repaint();
    }
    if entry.kind == Kind::Continent {
        placement(ui, work, row.map);
    }
}

/// A row's rectangle, as four numbers.
pub(super) fn rectangle(ui: &mut egui::Ui, work: &mut Work<'_>, row: &MapArea) {
    theme::property(
        ui,
        "Rectangle",
        "The part of the world the picture covers, in world yards: x of the top and bottom edges, y of the \
         left and right edges. The top and left are the larger numbers. The player's marker is placed by it, \
         and a zone's place on its continent's picture; the continent's Zones view shows and drags it there.",
        |ui| {
            let mut rect = [row.left, row.right, row.top, row.bottom];
            let mut changed = false;
            for (index, prefix) in [(2, "top x "), (3, "bottom x "), (0, "left y "), (1, "right y ")] {
                changed |= ui.add(egui::DragValue::new(&mut rect[index]).speed(2.0).max_decimals(1).prefix(prefix)).changed();
            }
            if changed {
                rows::set_rect(work.session, row.id, rect, Some(work.now));
            }
        },
    );
}

/// Read the map's tiles, then do `then`.
fn scan(work: &mut Work<'_>, then: AfterScan) {
    work.maps.after_scan = Some(then);
    work.maps.ask = Some(Ask::Scan);
}

/// A continent's place on the whole world's picture.
fn placement(ui: &mut egui::Ui, work: &mut Work<'_>, map: u32) {
    theme::heading(ui, "On the whole world");
    placement_numbers(ui, work, map);
    if rows::placement(work.session, map).is_some()
        && ui
            .button("Show on the whole world")
            .on_hover_text("Choose World with its Continents view and this continent chosen, to see it and drag it.")
            .clicked()
    {
        work.maps.choose(Which::World);
        work.maps.placing = true;
        work.maps.placed = Some(map);
    }
}

/// A continent's offsets, scale and tile range.
pub(super) fn placement_numbers(ui: &mut egui::Ui, work: &mut Work<'_>, map: u32) {
    let Some((place, range)) = rows::placement(work.session, map) else {
        theme::note(ui, "This map has no WorldMapContinent row, so the continent is not on the whole world's picture.");
        return;
    };
    theme::property(
        ui,
        "Placement",
        "WorldMapContinent's offsets and scale. One tile is 16 times the scale in the picture's pixels; an \
         offset moves the continent 16 pixels a unit.",
        |ui| {
            let mut place = place;
            let mut changed = false;
            changed |= ui.add(egui::DragValue::new(&mut place[0]).speed(0.05).max_decimals(3).prefix("x ")).changed();
            changed |= ui.add(egui::DragValue::new(&mut place[1]).speed(0.05).max_decimals(3).prefix("y ")).changed();
            changed |= ui
                .add(egui::DragValue::new(&mut place[2]).speed(0.005).range(0.0..=4.0).max_decimals(3).prefix("scale "))
                .changed();
            if changed {
                rows::set_placement(work.session, map, place, work.now);
            }
            if ui.small_button("Centre").on_hover_text("Put it in the middle of the picture, a third of it across or down.").clicked() {
                rows::set_placement(work.session, map, rows::default_placement(range), work.now);
            }
        },
    );
    theme::property(ui, "Tiles", "The tile range the continent's picture on the whole world covers.", |ui| {
        ui.label(theme::number(format!("columns {}-{}, rows {}-{}", range[0], range[1], range[2], range[3])));
    });
}
