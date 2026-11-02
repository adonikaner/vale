//! The explored areas of a zone or a single map: New overlay, the new art's
//! Write art and Discard, the list of overlays, and the chosen overlay's
//! numbers, areas, Replace art and Remove.

use bevy_egui::egui;

use super::Work;
use crate::import::worldmap::overlays::{self as rows_of, OverlayRow, Overlays, Target};
use crate::import::worldmap::{area_name, Ask, Entry, Kind};
use crate::tables::zones::area;
use crate::ui::widgets::theme;
use vale_edit::worldmap as pieces;

/// What goes above the picture: New overlay, and the new art's choices.
pub(super) fn actions(ui: &mut egui::Ui, work: &mut Work<'_>, entry: &Entry) {
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("New overlay\u{2026}")
            .on_hover_text(
                "Choose a PNG, TGA or BLP picture with transparency as the art of a new explored area. It \
                 is drawn on the picture to be dragged into place, then Write art adds the row.",
            )
            .clicked()
        {
            work.maps.ask = Some(Ask::NewOverlay);
        }
    });
    if let Some(said) = &work.maps.said {
        theme::note(ui, said.clone());
    }
    let Some(art) = &work.maps.overlays.art else { return };
    let name = art.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().to_string());
    theme::heading(ui, "New art");
    theme::note(
        ui,
        match art.target {
            Target::New => format!(
                "{name}, {}x{}, for a new overlay. Drag it into place on the picture; it is outlined in yellow.",
                art.width, art.height
            ),
            Target::Row(_) => format!("{name}, {}x{}, in place of the chosen overlay's art.", art.width, art.height),
        },
    );
    ui.horizontal(|ui| {
        let write = ui
            .add_sized(
                [120.0, 26.0],
                egui::Button::new(egui::RichText::new("Write art").color(egui::Color32::from_rgb(0x0C, 0x16, 0x1E)))
                    .fill(theme::ACCENT),
            )
            .on_hover_text(format!(
                "Write the art's files in Interface\\WorldMap\\{}\\, DXT3 with mipmaps as the shipped ones are, \
                 and for a new overlay add its WorldMapOverlay row.",
                entry.directory
            ));
        if write.clicked() {
            work.maps.ask = Some(Ask::WriteOverlay);
        }
        if ui.button("Discard").on_hover_text("Drop the new art; nothing is written.").clicked() {
            work.maps.overlays.art = None;
        }
    });
}

/// What goes under the picture: the list, and the chosen overlay's form.
pub(super) fn form(ui: &mut egui::Ui, work: &mut Work<'_>, entry: &Entry) {
    let Some(map_area) = entry.area.clone() else { return };
    let rows = Overlays::rows(work.session, map_area.id);
    theme::heading(ui, &format!("Explored areas ({})", rows.len()));
    if rows.is_empty() {
        theme::note(
            ui,
            "This map has no overlays, so its picture is drawn as it is whatever has been explored. Add one \
             with New overlay.",
        );
        return;
    }
    let mut selected = work.maps.overlays.selected;
    ui.horizontal_wrapped(|ui| {
        for row in &rows {
            let name = names(work, row).first().cloned().unwrap_or_else(|| row.texture.clone());
            if ui.selectable_label(selected == Some(row.id), name).on_hover_text(&row.texture).clicked() {
                selected = Some(row.id);
            }
        }
    });
    work.maps.overlays.selected = selected;
    let Some(row) = selected.and_then(|id| rows.iter().find(|row| row.id == id)).cloned() else {
        theme::note(ui, "Choose an overlay here or on the picture.");
        return;
    };
    ui.add_space(4.0);
    theme::property(ui, "Art", "The start of its files' names, and its size in the picture's pixels.", |ui| {
        ui.label(theme::number(format!(
            "{} ({} files), {}x{}",
            row.texture,
            {
                let (wide, tall) = pieces::overlay_grid(row.width, row.height);
                wide * tall
            },
            row.width,
            row.height
        )));
    });
    theme::property(ui, "Offset", "Where its top-left corner is on the picture. Drag it on the picture, or set it here.", |ui| {
        let (mut x, mut y) = row.offset;
        let a = ui.add(egui::DragValue::new(&mut x).range(0..=pieces::WIDTH - 1).prefix("x "));
        let b = ui.add(egui::DragValue::new(&mut y).range(0..=pieces::HEIGHT - 1).prefix("y "));
        if a.changed() || b.changed() {
            rows_of::set_offset(work.session, &row, (x, y), work.now);
        }
    });
    let choices = area_choices(work, entry);
    for slot in 0..4 {
        let label = format!("Area {}", slot + 1);
        theme::property(
            ui,
            &label,
            "Any one of the four areas explored shows the overlay. The first one with a name is the label \
             shown under the pointer.",
            |ui| {
                let current = row.areas[slot];
                let shown = match current {
                    0 => "None".to_string(),
                    id => area_name(work.session, id).map_or_else(|| id.to_string(), |name| format!("{name} ({id})")),
                };
                let mut picked = None;
                egui::ComboBox::from_id_salt(("overlay-area", row.id, slot)).selected_text(shown).width(220.0).show_ui(ui, |ui| {
                    if ui.selectable_label(current == 0, "None").clicked() {
                        picked = Some(0);
                    }
                    for (id, name) in &choices {
                        if ui.selectable_label(current == *id, format!("{name} ({id})")).clicked() {
                            picked = Some(*id);
                        }
                    }
                });
                if let Some(id) = picked.filter(|id| *id != current) {
                    rows_of::set_area(work.session, &row, slot, id);
                }
            },
        );
    }
    theme::property(
        ui,
        "Label",
        "The rectangle, in the picture's pixels, where the area's name shows under the pointer: top, left, \
         bottom, right. Drawn in yellow on the picture.",
        |ui| {
            let mut hit = row.hit;
            let mut changed = false;
            for (value, prefix) in hit.iter_mut().zip(["t ", "l ", "b ", "r "]) {
                changed |= ui.add(egui::DragValue::new(value).range(0..=pieces::WIDTH).prefix(prefix)).changed();
            }
            if changed {
                rows_of::set_hit(work.session, &row, hit, work.now);
            }
        },
    );
    ui.horizontal(|ui| {
        if ui
            .button("Replace art\u{2026}")
            .on_hover_text("Choose a picture as this overlay's art. It is shown in place before it is written.")
            .clicked()
        {
            work.maps.ask = Some(Ask::OverlayArt(row.id));
        }
        if ui
            .button("Remove overlay")
            .on_hover_text("Remove the WorldMapOverlay row. Ctrl+Z puts it back; its files stay in the project.")
            .clicked()
        {
            rows_of::remove(work.session, &row);
            work.maps.overlays.selected = None;
        }
    });
}

/// The names of an overlay's areas, in its order.
fn names(work: &Work<'_>, row: &OverlayRow) -> Vec<String> {
    row.areas.iter().filter(|id| **id != 0).filter_map(|id| area_name(work.session, *id)).collect()
}

/// The areas an overlay of this map can name: for a zone, the zone and its
/// sub-areas; for a single map, every area on its map. As `(id, name)` in
/// name order.
fn area_choices(work: &Work<'_>, entry: &Entry) -> Vec<(u32, String)> {
    let (Some(table), Some(row)) = (work.session.table(area::TABLE), &entry.area) else { return Vec::new() };
    let mut out: Vec<(u32, String)> = (0..table.record_count())
        .filter_map(|record| {
            let id = table.u32_at(record, 0)?;
            let wanted = match entry.kind {
                Kind::Zone => id == row.area || table.u32_at(record, area::PARENT)? == row.area,
                _ => table.u32_at(record, area::MAP)? == row.map,
            };
            wanted.then(|| (id, table.string_at(record, area::NAME).unwrap_or_default()))
        })
        .collect();
    out.sort_by_key(|(_, name)| name.to_lowercase());
    out
}
