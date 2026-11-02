//! What is placed on a continent's or the whole world's picture: a
//! continent's zones, by their `WorldMapArea` rectangles, and the whole
//! world's continents, by their `WorldMapContinent` offsets and scale.
//!
//! In the Zones or Continents view a click chooses one, a drag moves it, and
//! a drag of its bottom-right corner resizes it and keeps its shape. The
//! chosen one's numbers are under the picture.
//!
//! A zone's rectangle is in world yards, and its place on the continent's
//! picture is that rectangle inside the continent's. Moving it therefore also
//! changes the part of the world the zone's own picture covers, and where the
//! player's marker lands on it.

use bevy_egui::egui;

use super::Work;
use crate::import::worldmap::{rows, Entry, Kind, Which, AREAS, CONTINENTS};
use crate::ui::widgets::theme;
use vale_assets::tables::worldmap::WorldMap;
use vale_edit::worldmap as pieces;

/// The whole world's picture's pixels per unit of a `WorldMapContinent`
/// offset.
const UNIT: f32 = 16.0;

/// The size of the corner grip on screen, in points.
const GRIP: f32 = 9.0;

/// One zone or continent on the picture.
pub(super) struct Place {
    /// The zone's `WorldMapArea` row id, or the continent's map id.
    pub(super) id: u32,
    /// The `WorldMapArea` row its own map is drawn from.
    row: u32,
    pub(super) name: String,
    /// Left, top, right and bottom, in the picture's pixels.
    pub(super) rect: [f32; 4],
}

/// The places on `entry`'s picture: a continent's zones, or the whole
/// world's continents. Other maps have none.
pub(super) fn of(work: &mut Work<'_>, entry: &Entry) -> Vec<Place> {
    let (w, h) = (pieces::WIDTH as f32, pieces::HEIGHT as f32);
    match entry.kind {
        Kind::Continent => {
            let Some(whole) = &entry.area else { return Vec::new() };
            let (span_u, span_v) = (whole.left - whole.right, whole.top - whole.bottom);
            if span_u == 0.0 || span_v == 0.0 {
                return Vec::new();
            }
            let entries: Vec<Entry> = work.maps.entries(work.session, work.assets).to_vec();
            entries
                .iter()
                .filter(|zone| zone.continent == Some(whole.id))
                .filter_map(|zone| {
                    let area = zone.area.as_ref()?;
                    Some(Place {
                        id: area.id,
                        row: area.id,
                        name: zone.name.clone(),
                        rect: [
                            (whole.left - area.left) / span_u * w,
                            (whole.top - area.top) / span_v * h,
                            (whole.left - area.right) / span_u * w,
                            (whole.top - area.bottom) / span_v * h,
                        ],
                    })
                })
                .collect()
        }
        Kind::World => {
            let (Some(areas), Some(continents)) = (work.session.table(AREAS), work.session.table(CONTINENTS)) else {
                return Vec::new();
            };
            let Some(built) = WorldMap::parse(&areas.write(), &continents.write(), &[], None) else { return Vec::new() };
            built
                .continents()
                .iter()
                .map(|continent| {
                    let (l, t, r, b) = continent.rect_on_cosmic();
                    let name = work.maps.entry_of(Which::Row(continent.whole)).map_or_else(String::new, |e| e.name.clone());
                    Place { id: continent.map, row: continent.whole, name, rect: [l * w, t * h, r * w, b * h] }
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The note over the picture in the Zones or Continents view.
pub(super) fn actions(ui: &mut egui::Ui, work: &mut Work<'_>, entry: &Entry) {
    theme::note(
        ui,
        match entry.kind {
            Kind::Continent => {
                "Click a zone to choose it, drag it to move it, and drag its corner to resize it. This sets the \
                 zone's WorldMapArea rectangle, which is also the part of the world the zone's own picture covers."
            }
            _ => {
                "Click a continent to choose it, drag it to move it, and drag its corner to resize it. This sets \
                 the continent's WorldMapContinent offsets and scale."
            }
        },
    );
    if let Some(said) = &work.maps.said {
        theme::note(ui, said.clone());
    }
}

/// Which part of the chosen place a drag holds.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Grip {
    Move,
    Corner,
}

/// What a drag started from: the place, the grip, the pointer in the
/// picture's pixels, the place's numbers and its size in pixels.
#[derive(Debug, Clone, Copy)]
struct Held {
    id: u32,
    grip: Grip,
    from: (f32, f32),
    /// A zone's left, right, top and bottom, or a continent's offset x,
    /// offset y and scale.
    numbers: [f32; 4],
    size: (f32, f32),
}

/// Draw the places over the picture, the chosen one filled and with its
/// corner grip, and answer the clicks and drags.
#[allow(clippy::too_many_arguments)]
pub(super) fn picture(
    ui: &egui::Ui,
    painter: &egui::Painter,
    work: &mut Work<'_>,
    entry: &Entry,
    response: &egui::Response,
    at: Option<(f32, f32)>,
    scale: f32,
    on_screen: &dyn Fn(f32, f32, f32, f32) -> egui::Rect,
) {
    let places = of(work, entry);
    let font = egui::FontId::proportional(12.0);
    let plain = egui::Color32::from_rgba_unmultiplied(255, 220, 120, 210);
    for place in &places {
        let [l, t, r, b] = place.rect;
        let rect = on_screen(l, t, r, b);
        let chosen = work.maps.placed == Some(place.id);
        let colour = if chosen { theme::ACCENT } else { plain };
        if chosen {
            painter.rect_filled(rect, 0.0, theme::ACCENT.gamma_multiply(0.15));
        }
        painter.rect_stroke(rect, 0.0, egui::Stroke::new(if chosen { 2.0 } else { 1.0 }, colour), egui::StrokeKind::Inside);
        label(painter, rect, &place.name, font.clone(), colour);
        if chosen {
            painter.rect_filled(egui::Rect::from_center_size(rect.right_bottom(), egui::vec2(GRIP, GRIP)), 1.0, theme::ACCENT);
        }
    }
    let Some((x, y)) = at else { return };
    let reach = GRIP / scale;
    let on_grip = |place: &Place| (x - place.rect[2]).abs() <= reach && (y - place.rect[3]).abs() <= reach;
    let inside = |place: &Place| (place.rect[0]..=place.rect[2]).contains(&x) && (place.rect[1]..=place.rect[3]).contains(&y);
    let chosen = |id: Option<u32>| id.and_then(|id| places.iter().find(|place| place.id == id));
    if response.drag_started() || response.clicked() {
        // Keep the chosen place when the press is on it, so a drag of a zone
        // that overlaps another moves the one chosen; otherwise choose the
        // smallest place under the pointer.
        let keep = chosen(work.maps.placed).is_some_and(|place| inside(place) || on_grip(place));
        if !keep {
            let size = |place: &&Place| (place.rect[2] - place.rect[0]) * (place.rect[3] - place.rect[1]);
            work.maps.placed = places.iter().filter(|place| inside(place)).min_by(|a, b| size(a).total_cmp(&size(b))).map(|place| place.id);
        }
    }
    if response.hovered() && chosen(work.maps.placed).is_some_and(on_grip) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    }
    // A drag is measured from where it started, so a slow drag of less than a
    // pixel a frame still moves.
    let key = egui::Id::new("world-map-place-drag");
    if response.drag_started() {
        let held = chosen(work.maps.placed).and_then(|place| {
            Some(Held {
                id: place.id,
                grip: if on_grip(place) { Grip::Corner } else { Grip::Move },
                from: (x, y),
                numbers: numbers(work, entry, place.id)?,
                size: (place.rect[2] - place.rect[0], place.rect[3] - place.rect[1]),
            })
        });
        ui.data_mut(|data| match held {
            Some(held) => {
                data.insert_temp(key, held);
            }
            None => data.remove::<Held>(key),
        });
    }
    if !response.dragged() {
        return;
    }
    let Some(held) = ui.data(|data| data.get_temp::<Held>(key)) else { return };
    if work.maps.placed != Some(held.id) {
        return;
    }
    let (dx, dy) = (x - held.from.0, y - held.from.1);
    // A corner drag scales by the drag's reach along the place's diagonal.
    let (w0, h0) = held.size;
    let k = match w0 * w0 + h0 * h0 {
        diagonal if diagonal > 0.0 => (((w0 + dx) * w0 + (h0 + dy) * h0) / diagonal).max(0.05),
        _ => 1.0,
    };
    match entry.kind {
        Kind::Continent => {
            let Some(whole) = &entry.area else { return };
            let (span_u, span_v) = (whole.left - whole.right, whole.top - whole.bottom);
            let [left, right, top, bottom] = held.numbers;
            let rect = match held.grip {
                Grip::Move => {
                    let (du, dv) = (dx / pieces::WIDTH as f32 * span_u, dy / pieces::HEIGHT as f32 * span_v);
                    [left - du, right - du, top - dv, bottom - dv]
                }
                Grip::Corner => [left, left - (left - right) * k, top, top - (top - bottom) * k],
            };
            rows::set_rect(work.session, held.id, rect, Some(work.now));
        }
        Kind::World => {
            let Some((_, range)) = rows::placement(work.session, held.id) else { return };
            let [x0, y0, s0, _] = held.numbers;
            let place = match held.grip {
                Grip::Move => [x0 + dx / UNIT, y0 + dy / UNIT, s0],
                Grip::Corner => {
                    // The left edge is at `(first column - 32) * scale +
                    // offset x` units from a fixed point, and the top edge
                    // likewise, so the offsets change with the scale to hold
                    // the top-left corner where it is.
                    let s = (s0 * k).clamp(0.001, 4.0);
                    [x0 + (range[0] as f32 - 32.0) * (s0 - s), y0 + (range[2] as f32 - 32.0) * (s0 - s), s]
                }
            };
            rows::set_placement(work.session, held.id, place, work.now);
        }
        _ => {}
    }
}

/// A place's name in the top-left corner of its outline, on a dark backing
/// so it reads over the picture.
pub(super) fn label(painter: &egui::Painter, rect: egui::Rect, name: &str, font: egui::FontId, colour: egui::Color32) {
    let galley = painter.layout_no_wrap(name.to_string(), font, colour);
    let at = rect.left_top() + egui::vec2(3.0, 2.0);
    painter.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand(2.0), 2.0, egui::Color32::from_black_alpha(150));
    painter.galley(at, galley, colour);
}

/// The numbers a drag of place `id` starts from: a zone's rectangle, or a
/// continent's offsets and scale.
fn numbers(work: &Work<'_>, entry: &Entry, id: u32) -> Option<[f32; 4]> {
    match entry.kind {
        Kind::Continent => {
            let area = work.maps.entry_of(Which::Row(id))?.area.as_ref()?;
            Some([area.left, area.right, area.top, area.bottom])
        }
        Kind::World => {
            let (place, _) = rows::placement(work.session, id)?;
            Some([place[0], place[1], place[2], 0.0])
        }
        _ => None,
    }
}

/// The chosen zone's rectangle or continent's placement, under the picture.
pub(super) fn form(ui: &mut egui::Ui, work: &mut Work<'_>, entry: &Entry) {
    let places = of(work, entry);
    let Some(place) = work.maps.placed.and_then(|id| places.into_iter().find(|place| place.id == id)) else {
        theme::note(
            ui,
            match entry.kind {
                Kind::Continent => "Click a zone on the picture to choose it.",
                _ => "Click a continent on the picture to choose it.",
            },
        );
        return;
    };
    theme::heading(ui, &place.name);
    match entry.kind {
        Kind::Continent => {
            if let Some(area) = work.maps.entry_of(Which::Row(place.id)).and_then(|zone| zone.area.clone()) {
                super::rows::rectangle(ui, work, &area);
            }
        }
        _ => super::rows::placement_numbers(ui, work, place.id),
    }
    if ui.button("Open its map").on_hover_text("Choose this map in the list, to change its picture.").clicked() {
        work.maps.choose(Which::Row(place.row));
    }
}
