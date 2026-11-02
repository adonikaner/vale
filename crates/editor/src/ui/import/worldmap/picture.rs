//! The picture, drawn as large as the panel allows up to its own size.
//!
//! Over the picture, by view:
//!
//! ```text
//! Picture         the twelve sectors' lines, the chosen sectors outlined, and
//!                 a click toggles the sector under the pointer. On a
//!                 continent the zones' rectangles, and on the whole world the
//!                 continents', are outlined and named
//! Explored areas  every overlay drawn over the picture and outlined, the
//!                 chosen one's label rectangle, a click chooses the overlay
//!                 under the pointer and a drag moves the chosen one
//! Zones,          a continent's zones or the whole world's continents
//! Continents      outlined and named; a click chooses one and a drag moves
//!                 or resizes it (see `places`)
//! ```

use bevy_egui::egui;

use super::{places, View, Work};
use crate::import::worldmap::overlays::{self as rows_of, Overlays, Target};
use crate::import::worldmap::Entry;
use crate::ui::widgets::theme;
use vale_edit::worldmap as pieces;

/// Draw the picture and answer its clicks and drags.
pub(super) fn picture(ui: &mut egui::Ui, work: &mut Work<'_>, entry: &Entry, view: View) {
    let current = work.maps.picture.current(entry, work.session, work.assets);
    let (current_picture, current_stamp) = (current.picture.clone(), current.stamp);
    let pending = work.maps.picture.pending_of(entry.which).filter(|_| !work.maps.picture.showing_current);
    let (base, base_stamp, base_key) = match pending {
        Some(pending) => (pending.picture.clone(), pending.stamp, "new"),
        None => (current_picture, current_stamp, "current"),
    };
    let overlays = match (view, &entry.area) {
        (View::Explored, Some(area)) => Overlays::rows(work.session, area.id),
        _ => Vec::new(),
    };
    let handle = match view {
        View::Picture | View::Places => texture(ui, &format!("world-map-{base_key}"), base_stamp, || base),
        View::Explored => {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (base_key, base_stamp, work.maps.overlays.stamp(), work.session.table_revision, work.session.files_revision)
                .hash(&mut hasher);
            let (maps, session, assets) = (&mut *work.maps, &*work.session, work.assets);
            texture(ui, "world-map-explored", hasher.finish(), || {
                maps.overlays.composite(&base, &overlays, &entry.directory, session, assets)
            })
        }
    };
    let width = ui.available_width().min(pieces::WIDTH as f32);
    let scale = width / pieces::WIDTH as f32;
    let size = egui::vec2(width, pieces::HEIGHT as f32 * scale);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::SUNK);
    painter.image(handle.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    // A point of the picture, in its own pixels, and back.
    let to_screen = |x: f32, y: f32| rect.min + egui::vec2(x * scale, y * scale);
    let box_on_screen = |l: f32, t: f32, r: f32, b: f32| egui::Rect::from_min_max(to_screen(l, t), to_screen(r, b));
    let at = response.hover_pos().or(response.interact_pointer_pos()).map(|pos| {
        let local = (pos - rect.min) / scale;
        (local.x.clamp(0.0, pieces::WIDTH as f32 - 1.0), local.y.clamp(0.0, pieces::HEIGHT as f32 - 1.0))
    });
    match view {
        View::Picture => {
            outlines(&painter, work, entry, &box_on_screen);
            let selected = work.maps.picture.selected;
            for index in 0..pieces::PIECES {
                let [l, t, r, b] = pieces::sector_rect(index);
                let on_screen = box_on_screen(l as f32, t as f32, r as f32, b as f32);
                let chosen = selected & (1 << index) != 0;
                let stroke = match chosen {
                    true => egui::Stroke::new(2.0, theme::ACCENT),
                    false => egui::Stroke::new(1.0, egui::Color32::from_white_alpha(40)),
                };
                if chosen {
                    painter.rect_filled(on_screen, 0.0, theme::ACCENT.gamma_multiply(0.12));
                }
                painter.rect_stroke(on_screen, 0.0, stroke, egui::StrokeKind::Inside);
            }
            if let Some((x, y)) = at.filter(|_| response.hovered()) {
                let index = (y as u32 / pieces::PIECE) * pieces::ACROSS + x as u32 / pieces::PIECE;
                let [l, t, r, b] = pieces::sector_rect(index);
                painter.rect_stroke(
                    box_on_screen(l as f32, t as f32, r as f32, b as f32),
                    0.0,
                    egui::Stroke::new(1.0, theme::INK),
                    egui::StrokeKind::Inside,
                );
                if response.clicked() {
                    work.maps.picture.selected ^= 1 << index;
                }
            }
            let response = response.on_hover_text("Click a sector to choose it; Import and Draw then change only the chosen sectors.");
            let _ = response;
        }
        View::Explored => {
            let selected = work.maps.overlays.selected;
            for row in &overlays {
                let (l, t) = (row.offset.0 as f32, row.offset.1 as f32);
                let bounds = box_on_screen(l, t, l + row.width as f32, t + row.height as f32);
                let chosen = selected == Some(row.id);
                let stroke = match chosen {
                    true => egui::Stroke::new(2.0, theme::ACCENT),
                    false => egui::Stroke::new(1.0, egui::Color32::from_white_alpha(50)),
                };
                painter.rect_stroke(bounds, 0.0, stroke, egui::StrokeKind::Inside);
                if chosen {
                    let [top, left, bottom, right] = row.hit;
                    let hit = box_on_screen(left as f32, top as f32, right as f32, bottom as f32);
                    painter.rect_stroke(hit, 0.0, egui::Stroke::new(1.0, theme::WARN), egui::StrokeKind::Inside);
                }
            }
            if let Some(art) = work.maps.overlays.art.as_ref().filter(|art| art.target == Target::New) {
                let (l, t) = (art.offset.0 as f32, art.offset.1 as f32);
                let bounds = box_on_screen(l, t, l + art.width as f32, t + art.height as f32);
                painter.rect_stroke(bounds, 0.0, egui::Stroke::new(2.0, theme::WARN), egui::StrokeKind::Inside);
            }
            explored_input(ui, work, &overlays, &response, at, scale);
        }
        View::Places => places::picture(ui, &painter, work, entry, &response, at, scale, &box_on_screen),
    }
    painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Outside);
    theme::note(
        ui,
        match (view, work.maps.picture.pending_of(entry.which).is_some() && !work.maps.picture.showing_current) {
            (View::Explored, _) => "Every overlay drawn, as a character who has explored the whole zone sees it. Click one to choose it; drag it to move it.",
            (View::Places, _) => "Where the world map places each one, and where it lights it under the pointer.",
            (_, true) => "The new picture, as the world map will show it. Nothing is written until Write to map.",
            _ => "The picture the world map shows now.",
        },
    );
}

/// A click chooses the overlay under the pointer, the last drawn first, and a
/// drag moves the chosen overlay, or the new overlay's art.
fn explored_input(
    ui: &egui::Ui,
    work: &mut Work<'_>,
    overlays: &[rows_of::OverlayRow],
    response: &egui::Response,
    at: Option<(f32, f32)>,
    scale: f32,
) {
    let Some((x, y)) = at else { return };
    let inside = |row: &rows_of::OverlayRow| {
        (row.offset.0 as f32..(row.offset.0 + row.width) as f32).contains(&x)
            && (row.offset.1 as f32..(row.offset.1 + row.height) as f32).contains(&y)
    };
    if response.drag_started() || response.clicked() {
        let new_art = work.maps.overlays.art.as_ref().filter(|art| art.target == Target::New).is_some_and(|art| {
            (art.offset.0 as f32..(art.offset.0 + art.width) as f32).contains(&x)
                && (art.offset.1 as f32..(art.offset.1 + art.height) as f32).contains(&y)
        });
        if !new_art {
            // Keep the chosen overlay when the press is on it, so a drag of an
            // overlay under another moves the one chosen.
            let keep = work.maps.overlays.selected.and_then(|id| overlays.iter().find(|row| row.id == id)).is_some_and(inside);
            if !keep {
                work.maps.overlays.selected = overlays.iter().rev().find(|row| inside(row)).map(|row| row.id);
            }
        }
    }
    // A drag is measured from where it started, so a slow drag of less than a
    // pixel a frame still moves.
    let held = egui::Id::new("world-map-drag");
    let new_art = work.maps.overlays.art.as_ref().filter(|art| art.target == Target::New).map(|art| art.offset);
    let chosen = work.maps.overlays.selected.and_then(|id| overlays.iter().find(|row| row.id == id));
    if response.drag_started() {
        let from = new_art.or(chosen.map(|row| row.offset));
        if let Some(from) = from {
            ui.data_mut(|data| data.insert_temp(held, (from, (x, y))));
        }
    }
    if !response.dragged() {
        return;
    }
    let Some(((ox, oy), (sx, sy))) = ui.data(|data| data.get_temp::<((u32, u32), (f32, f32))>(held)) else { return };
    let offset = (
        (ox as f32 + x - sx).round().clamp(0.0, pieces::WIDTH as f32 - 1.0) as u32,
        (oy as f32 + y - sy).round().clamp(0.0, pieces::HEIGHT as f32 - 1.0) as u32,
    );
    let _ = scale;
    if new_art.is_some() {
        work.maps.overlays.place_new(offset);
        return;
    }
    let Some(row) = chosen else { return };
    if offset != row.offset {
        rows_of::set_offset(work.session, row, offset, work.now);
    }
}

/// On a continent, its zones' rectangles; on the whole world, the
/// continents'. Each outlined faintly and named.
fn outlines(painter: &egui::Painter, work: &mut Work<'_>, entry: &Entry, on_screen: &dyn Fn(f32, f32, f32, f32) -> egui::Rect) {
    let font = egui::FontId::proportional(11.0);
    let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 220, 120, 140));
    for place in places::of(work, entry) {
        let [l, t, r, b] = place.rect;
        let rect = on_screen(l, t, r, b);
        painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
        places::label(painter, rect, &place.name, font.clone(), stroke.color);
    }
}

/// The texture for a 1002x668 picture under `key`, made by `pixels` and
/// uploaded again only when its stamp changes. One texture is kept per key.
fn texture(ui: &egui::Ui, key: &str, stamp: u64, pixels: impl FnOnce() -> Vec<u8>) -> egui::TextureHandle {
    let id = egui::Id::new(key);
    if let Some((held, handle)) = ui.data(|data| data.get_temp::<(u64, egui::TextureHandle)>(id)) {
        if held == stamp {
            return handle;
        }
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([pieces::WIDTH as usize, pieces::HEIGHT as usize], &pixels());
    let handle = ui.ctx().load_texture(key, image, egui::TextureOptions::LINEAR);
    ui.data_mut(|data| data.insert_temp(id, (stamp, handle.clone())));
    handle
}
