//! A light's colours over the day, drawn as a strip of colour rather than as
//! rows of numbers.
//!
//! `LightIntBand` is 7,668 rows of 34 columns and `LightFloatBand` is 2,556
//! more. A row holds a count, sixteen times and sixteen values, and says
//! nothing about which of the eighteen bands it is or which light it belongs
//! to — that is its position: band `b` of `LightParams` row `p` is row
//! `(p - 1) * 18 + b + 1`. So the tables cannot be browsed usefully and no
//! reference column points into them; the only way to reach a band is to
//! compute it from the params row, which is what this does.
//!
//! The panel draws the band itself: a strip of the colour at every minute of
//! the day, with the light's keys marked on it. That colour, moving from a
//! cool night through a warm dawn, is what is being edited. A list of
//! `(720, 0x00FF8800)` pairs holds the same information and is hard to read.
//!
//! ## The strip uses the renderer's interpolation
//!
//! A strip drawn on its own curve would show something the game does not do.
//! [`vale_assets::tables::light`]'s `straddle` is how the client samples a
//! band: linear between the two keys that straddle the time, wrapping across
//! midnight, because a band is a cycle and a time past the last key belongs
//! between it and the first. [`sample`] is that rule, and it is the only copy
//! of it here.
//!
//! ## What is edited and what is not
//!
//! A key's time and its value are edited. The count is written with them, so
//! adding or removing a key is the same edit as moving one. The sixteen slots
//! past the count keep whatever they held: `0xCCCCCCCC` in 101,116 of the
//! shipped ones, which is the authoring tool's uninitialised memory. Cleaning
//! it would rewrite rows nobody edited.

use super::data::Workspace;
use super::theme;
use crate::tools::tables;
use vale_assets::tables::light::{
    float_band_row, int_band_row, BAND_KEYS, BAND_TIME_FIELD, BAND_VALUE_FIELD, DAY,
    FLOAT_BAND_NAMES, FLOAT_BAND_NOTES, INT_BAND_NAMES, INT_BAND_NOTES,
};
use bevy_egui::egui;

/// How tall a band's day strip is drawn.
const STRIP_HEIGHT: f32 = 22.0;

/// How many samples a strip is painted with. One per six minutes of game time,
/// which is finer than the strip has pixels at any width this panel reaches.
const STRIP_STEPS: usize = 120;

/// A band's keys, as they are in the file.
///
/// `times` and `values` are the live ones — the first `count` of the sixteen —
/// so `times.len()` is the count and the slots past it are not represented
/// here at all. See the module note on why they are left alone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Band {
    pub row: usize,
    pub times: Vec<u32>,
    pub values: Vec<u32>,
}

impl Band {
    /// The value at a time, by the rule the renderer samples with.
    ///
    /// `None` for a band with no keys, which draws as nothing rather than as
    /// black: a band with no keys states no value, which is not the same as a
    /// dark one.
    pub fn at(&self, time: u32) -> Option<u32> {
        let (i, j, t) = straddle(&self.times, time)?;
        Some(mix(self.values[i], self.values[j], t))
    }

    /// The value at a time as an `f32`, for the float table, whose sixteen
    /// values are the bits of an `f32` rather than a packed colour.
    pub fn value_at(&self, time: u32) -> Option<f32> {
        let (i, j, t) = straddle(&self.times, time)?;
        let (a, b) = (
            f32::from_bits(self.values[i]),
            f32::from_bits(self.values[j]),
        );
        Some(a + (b - a) * t)
    }
}

/// The two keys that straddle `time`, and how far between them it is.
///
/// A copy of `vale_assets::tables::light`'s `straddle`, which is private to
/// that module and is the renderer's own. It is kept identical, wrap included:
/// a time before the first key belongs between the last key and the first,
/// across midnight. Every one of these bands has a night that crosses
/// midnight, so this case always applies.
fn straddle(times: &[u32], time: u32) -> Option<(usize, usize, f32)> {
    if times.is_empty() {
        return None;
    }
    if times.len() == 1 {
        return Some((0, 0, 0.0));
    }
    let time = time % DAY;
    let (i, j, from, span) = match times.iter().rposition(|&t| t <= time) {
        Some(i) if i + 1 < times.len() => (i, i + 1, times[i], times[i + 1] - times[i]),
        Some(i) => (i, 0, times[i], DAY - times[i] + times[0]),
        None => {
            let last = times.len() - 1;
            (last, 0, times[last], DAY - times[last] + times[0])
        }
    };
    let elapsed = match time >= from {
        true => time - from,
        false => DAY - from + time,
    };
    let t = match span {
        0 => 0.0,
        span => (elapsed as f32 / span as f32).clamp(0.0, 1.0),
    };
    Some((i, j, t))
}

/// Blend two packed `0x00RRGGBB` colours.
///
/// The top byte is taken from the nearer key rather than blended. It is
/// zero on all but one shipped row and means nothing in between; blending it
/// would invent values for a byte whose meaning is unknown.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        ((x + (y - x) * t).round().clamp(0.0, 255.0) as u32) << shift
    };
    (a & 0xff00_0000) | channel(16) | channel(8) | channel(0)
}

/// A packed colour as egui's.
fn swatch(packed: u32) -> egui::Color32 {
    egui::Color32::from_rgb(
        ((packed >> 16) & 0xff) as u8,
        ((packed >> 8) & 0xff) as u8,
        (packed & 0xff) as u8,
    )
}

/// `14:30` — a band time, which is half-minutes past midnight.
pub fn clock(time: u32) -> String {
    let minutes = (time % DAY) / 2;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// A typed time, as minutes past midnight: the inverse of [`clock`].
///
/// `9:00`, `09:00` and `21:30` are hours and minutes. A bare number is minutes
/// past midnight, which is what the value behind the box already is, so a
/// person who drags the field and then types what they saw gets the same
/// number back either way.
///
/// `None` for anything else, which leaves the box on the value it had. That is
/// egui's own behaviour for a parse it cannot make, and it is safer than
/// guessing a time from a word.
pub fn minutes_of_day(text: &str) -> Option<u32> {
    let text = text.trim();
    let minutes = match text.split_once(':') {
        Some((hours, minutes)) => {
            let hours: u32 = hours.trim().parse().ok()?;
            let minutes: u32 = minutes.trim().parse().ok()?;
            // 24:00 is midnight the next day and 10:75 is not a time. Both are
            // refused rather than folded, so a typo does not silently become a
            // different hour.
            if hours > 23 || minutes > 59 {
                return None;
            }
            hours * 60 + minutes
        }
        None => text.parse().ok()?,
    };
    (minutes < DAY / 2).then_some(minutes)
}

/// Read one band's live keys out of the open table.
fn read(work: &Workspace<'_>, table: &str, row: u32) -> Option<Band> {
    let open = work.session.table(table)?;
    let record = open.row_of(row)?;
    let count = open.u32_at(record, 1).unwrap_or(0).min(BAND_KEYS as u32) as usize;
    Some(Band {
        row: record,
        times: (0..count)
            .filter_map(|n| open.u32_at(record, BAND_TIME_FIELD + n))
            .collect(),
        values: (0..count)
            .filter_map(|n| open.u32_at(record, BAND_VALUE_FIELD + n))
            .collect(),
    })
}

/// Write a band's keys back: the count, then the sixteen slots it fills.
///
/// One gesture per band, so adding a key, dragging its time and recolouring it
/// inside the undo window are one entry — the same rule every other drag in
/// this editor follows.
fn write(work: &mut Workspace<'_>, table: &'static str, band: &Band, label: &str) {
    let mut fields: Vec<(usize, u32)> = vec![(1, band.times.len() as u32)];
    for n in 0..band.times.len().min(BAND_KEYS) {
        fields.push((BAND_TIME_FIELD + n, band.times[n]));
        fields.push((BAND_VALUE_FIELD + n, band.values[n]));
    }
    tables::set_fields(
        work.session,
        table,
        band.row,
        &fields,
        label,
        &format!("{table} {} keys", band.row),
        work.now,
    );
}

/// Draw every band a `LightParams` row owns, each as a day of colour.
///
/// This is the only route into the two band tables: they are reached by
/// arithmetic on this row's id, and nothing points at them.
pub fn blocks(ui: &mut egui::Ui, work: &mut Workspace<'_>, params_id: u32, hour: u32) {
    for (table, count, names, notes) in [
        (
            "LightIntBand",
            INT_BAND_NAMES.len(),
            &INT_BAND_NAMES[..],
            &INT_BAND_NOTES[..],
        ),
        (
            "LightFloatBand",
            FLOAT_BAND_NAMES.len(),
            &FLOAT_BAND_NAMES[..],
            &FLOAT_BAND_NOTES[..],
        ),
    ] {
        if !work.session.open_table(work.assets, table) {
            theme::note(ui, format!("opening {table}.dbc…"));
            continue;
        }
        let colours = table == "LightIntBand";
        let title = match colours {
            true => format!("Colours ({count} bands)"),
            false => format!("Float values ({count} bands)"),
        };
        egui::CollapsingHeader::new(egui::RichText::new(title).size(14.0).strong())
            .default_open(colours)
            .id_salt((table, params_id))
            .show(ui, |ui| {
                for index in 0..count {
                    let row = match colours {
                        true => int_band_row(params_id, index as u32),
                        false => float_band_row(params_id, index as u32),
                    };
                    let Some(row) = row else { continue };
                    let Some(band) = read(work, table, row) else {
                        continue;
                    };
                    one(
                        ui,
                        work,
                        table,
                        index,
                        names[index],
                        notes[index],
                        band,
                        hour,
                    );
                }
            });
    }
}

/// One band: its name, its day, and its keys when it is opened.
#[allow(clippy::too_many_arguments)]
fn one(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table: &'static str,
    index: usize,
    name: &str,
    note: &str,
    band: Band,
    hour: u32,
) {
    let colours = table == "LightIntBand";
    let id = ui.make_persistent_id((table, band.row));
    let mut open =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);

    ui.horizontal(|ui| {
        // egui's own painted triangle, not a text arrow. The interface's four
        // typefaces are the game's (`Fonts\*.TTF`) and none of them has `▸`
        // or `▾`, so a button with one in it draws the missing-glyph box. The
        // section headers beside this one draw correctly because
        // `CollapsingHeader` paints its marker rather than using a glyph.
        open.show_toggle_button(ui, egui::collapsing_header::paint_default_icon);
        let label = ui.add_sized(
            egui::vec2(120.0, 18.0),
            egui::Label::new(egui::RichText::new(name).size(12.5).color(theme::INK_DIM)).truncate(),
        );
        let mut tip = format!("band {index}");
        if !note.is_empty() {
            tip.push_str("\n\n");
            tip.push_str(note);
        }
        label.on_hover_text(tip);
        match colours {
            true => strip(ui, &band, hour),
            false => numbers(ui, &band, hour),
        }
    });

    // Indented by hand rather than through `show_body_indented`, which needs
    // the header's own `Response`. There is no header widget here: the heading
    // row is a strip and a name, laid out above.
    if open.is_open() {
        ui.indent((table, band.row), |ui| {
            keys(ui, work, table, band, colours);
        });
    }
    open.store(ui.ctx());
}

/// Draw the band across a whole day, with its keys marked and the world's
/// current hour shown on it.
///
/// The strip shows the colour at every time, in order, so a sunrise looks like
/// a sunrise. The keys are the ticks under it. The hour is the line across it
/// and comes from the world, so the colour under that line is the one on
/// screen behind the panel.
fn strip(ui: &mut egui::Ui, band: &Band, hour: u32) {
    let width = ui.available_width().max(60.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, STRIP_HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    if band.times.is_empty() {
        painter.rect_filled(rect, 2.0, theme::PANEL);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "no keys",
            egui::FontId::proportional(theme::SMALL),
            theme::INK_FAINT,
        );
        return;
    }
    let step = rect.width() / STRIP_STEPS as f32;
    for n in 0..STRIP_STEPS {
        let time = (n as u32 * DAY) / STRIP_STEPS as u32;
        let Some(packed) = band.at(time) else {
            continue;
        };
        let x = rect.left() + n as f32 * step;
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(x, rect.top()),
                // A hair over a step, so rounding does not leave gaps between
                // the samples.
                egui::vec2(step + 1.0, rect.height()),
            ),
            0.0,
            swatch(packed),
        );
    }
    marks(&painter, rect, band, hour);
    if let Some(at) = response.hover_pos() {
        let t = ((at.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let time = (t * DAY as f32) as u32;
        if let Some(packed) = band.at(time) {
            response.on_hover_text(format!(
                "{}  {} {} {}",
                clock(time),
                (packed >> 16) & 0xff,
                (packed >> 8) & 0xff,
                packed & 0xff
            ));
        }
    }
}

/// Draw a float band: a line across the day rather than a colour wash.
fn numbers(ui: &mut egui::Ui, band: &Band, hour: u32) {
    let width = ui.available_width().max(60.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, STRIP_HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::PANEL);
    if band.times.is_empty() {
        return;
    }
    // Scaled to the band's own range, because the six of them run from a
    // fraction to eighteen thousand and one scale for all would flatten five.
    let sampled: Vec<f32> = (0..STRIP_STEPS)
        .filter_map(|n| band.value_at((n as u32 * DAY) / STRIP_STEPS as u32))
        .collect();
    let (low, high) = sampled
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = (high - low).abs().max(f32::EPSILON);
    let mut last: Option<egui::Pos2> = None;
    for (n, &value) in sampled.iter().enumerate() {
        let x = rect.left() + n as f32 / sampled.len() as f32 * rect.width();
        let y = rect.bottom() - ((value - low) / span) * (rect.height() - 4.0) - 2.0;
        let here = egui::pos2(x, y);
        if let Some(from) = last {
            painter.line_segment([from, here], egui::Stroke::new(1.5, theme::ACCENT));
        }
        last = Some(here);
    }
    marks(&painter, rect, band, hour);
    if let Some(value) = band.value_at(hour) {
        response.on_hover_text(format!("{} now {value:.2}", clock(hour)));
    }
}

/// The keys as ticks, and the world's hour as a line.
fn marks(painter: &egui::Painter, rect: egui::Rect, band: &Band, hour: u32) {
    for &time in &band.times {
        let x = rect.left() + (time % DAY) as f32 / DAY as f32 * rect.width();
        painter.line_segment(
            [
                egui::pos2(x, rect.bottom() - 4.0),
                egui::pos2(x, rect.bottom()),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
        );
    }
    let x = rect.left() + (hour % DAY) as f32 / DAY as f32 * rect.width();
    painter.line_segment(
        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
        egui::Stroke::new(1.0, egui::Color32::WHITE),
    );
}

/// The band's keys, opened: a time and a value each, and the two buttons.
fn keys(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table: &'static str,
    band: Band,
    colours: bool,
) {
    let mut edited = band.clone();
    let mut remove: Option<usize> = None;
    for n in 0..edited.times.len() {
        ui.horizontal(|ui| {
            // Minutes of the day rather than half-minutes. The file counts in
            // half-minutes; the clock beside the box shows what the number
            // means.
            let mut minutes = (edited.times[n] % DAY) / 2;
            if ui
                .add(
                    egui::DragValue::new(&mut minutes)
                        .range(0..=(DAY / 2 - 1))
                        .speed(5.0)
                        .custom_formatter(|v, _| clock((v as u32) * 2))
                        // Parse the text the way the formatter writes it.
                        // Without this the box formats `09:00` and parses it
                        // with egui's own numeric reader, which reads the `09`
                        // and stops at the colon, so a typed time was stored
                        // as nine minutes past midnight.
                        .custom_parser(|text| minutes_of_day(text).map(f64::from)),
                )
                .changed()
            {
                edited.times[n] = minutes * 2;
            }
            match colours {
                true => {
                    let packed = edited.values[n];
                    let mut rgb = [
                        ((packed >> 16) & 0xff) as u8,
                        ((packed >> 8) & 0xff) as u8,
                        (packed & 0xff) as u8,
                    ];
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        // The top byte rides through untouched — see
                        // `schema::Kind::Colour`.
                        edited.values[n] = (packed & 0xff00_0000)
                            | (u32::from(rgb[0]) << 16)
                            | (u32::from(rgb[1]) << 8)
                            | u32::from(rgb[2]);
                    }
                    ui.label(
                        theme::number(format!("{} {} {}", rgb[0], rgb[1], rgb[2]))
                            .color(theme::INK_DIM),
                    );
                }
                false => {
                    let mut value = f32::from_bits(edited.values[n]);
                    if ui
                        .add(egui::DragValue::new(&mut value).speed(1.0))
                        .changed()
                    {
                        edited.values[n] = value.to_bits();
                    }
                }
            }
            if ui
                .small_button("×")
                .on_hover_text("Remove this key from the band")
                .clicked()
            {
                remove = Some(n);
            }
        });
    }

    ui.horizontal(|ui| {
        let full = edited.times.len() >= BAND_KEYS;
        if ui
            .add_enabled(!full, egui::Button::new("+ key").small())
            .on_hover_text(match full {
                true => format!("a band holds at most {BAND_KEYS} keys"),
                false => "Add a key in the middle of the longest gap between keys".to_string(),
            })
            .clicked()
        {
            add(&mut edited);
        }
        if edited.times.is_empty() {
            theme::note(ui, "This band has no keys and defines no value.");
        }
    });

    if let Some(n) = remove {
        edited.times.remove(n);
        edited.values.remove(n);
    }
    if edited == band {
        return;
    }
    // Sorted before it is written. The renderer's `straddle` walks the times
    // expecting them to ascend, and a key dragged past its neighbour would
    // otherwise make the band read backwards from that point. 35 of the
    // shipped rows are already out of order, so the reader tolerates it, but
    // the writer does not add more.
    let mut pairs: Vec<(u32, u32)> = edited
        .times
        .iter()
        .copied()
        .zip(edited.values.iter().copied())
        .collect();
    pairs.sort_by_key(|&(time, _)| time);
    edited.times = pairs.iter().map(|&(time, _)| time).collect();
    edited.values = pairs.iter().map(|&(_, value)| value).collect();
    let what = match colours {
        true => "Edit light colour",
        false => "Edit light band",
    };
    write(work, table, &edited, what);
}

/// Put a key in the middle of the longest gap.
///
/// The end of the list is the wrong place for a new key: a band is a cycle,
/// so the widest gap is the one a person most likely wants to shape. The new
/// key's value is what the band already reads there, so adding a key changes
/// nothing until it is moved. That makes it safe to add one to see what
/// happens.
fn add(band: &mut Band) {
    let Some(at) = widest_gap(&band.times) else {
        // An empty band gets a key at midnight; a band of one gets its
        // opposite, since a single key is a constant and needs a second before
        // anything can change over the day.
        let time = band.times.first().map_or(0, |&t| (t + DAY / 2) % DAY);
        let value = band.values.first().copied().unwrap_or(0);
        band.times.push(time);
        band.values.push(value);
        return;
    };
    let value = band.at(at).unwrap_or(0);
    band.times.push(at);
    band.values.push(value);
}

/// The middle of the widest gap between consecutive keys, wrapping across
/// midnight. `None` for fewer than two keys, which have no gap.
fn widest_gap(times: &[u32]) -> Option<u32> {
    if times.len() < 2 {
        return None;
    }
    let mut best: Option<(u32, u32)> = None;
    for n in 0..times.len() {
        let from = times[n];
        let to = times[(n + 1) % times.len()];
        let span = match to > from {
            true => to - from,
            false => DAY - from + to,
        };
        if best.is_none_or(|(widest, _)| span > widest) {
            best = Some((span, (from + span / 2) % DAY));
        }
    }
    best.map(|(_, at)| at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band(times: &[u32], values: &[u32]) -> Band {
        Band {
            row: 0,
            times: times.to_vec(),
            values: values.to_vec(),
        }
    }

    /// The strip is sampled the way the renderer samples a band, wrap
    /// included; otherwise it would show something the game does not do.
    #[test]
    fn a_band_is_read_the_way_the_renderer_reads_it() {
        // Two keys: black at 00:00, white at 12:00.
        let b = band(&[0, 1440], &[0x000000, 0xFFFFFF]);
        assert_eq!(b.at(0), Some(0x000000));
        assert_eq!(b.at(1440), Some(0xFFFFFF));
        // Halfway between them.
        assert_eq!(b.at(720), Some(0x808080));
        // Past the last key it wraps back to the first. Every one of these
        // bands needs this case, because the night runs across midnight.
        assert_eq!(b.at(2160), Some(0x808080));
        // A single key is a constant all day.
        let flat = band(&[600], &[0x123456]);
        assert_eq!(flat.at(0), Some(0x123456));
        assert_eq!(flat.at(2879), Some(0x123456));
        // A band with no keys says nothing rather than saying black.
        assert_eq!(band(&[], &[]).at(0), None);
    }

    /// The top byte is carried, not blended.
    ///
    /// It is zero on all but one shipped row and means nothing in between, so
    /// interpolating it would invent values for a byte whose meaning is not
    /// known.
    #[test]
    fn a_blend_carries_the_top_byte_rather_than_mixing_it() {
        assert_eq!(mix(0xFF00_0000, 0x0000_0000, 0.5) >> 24, 0xFF);
        assert_eq!(mix(0x0000_0000, 0xFF00_0000, 0.5) >> 24, 0x00);
        // The three colour channels are blended.
        assert_eq!(
            mix(0x00_00_00_00, 0x00_FF_FF_FF, 0.5) & 0xFF_FF_FF,
            0x808080
        );
    }

    /// A new key lands in the widest gap and leaves the band where it was, to
    /// within the one unit a byte can express.
    ///
    /// Adding a key is how a person finds out what a band does, so it has to
    /// be safe: its value is what the band already reads at that time, so the
    /// day looks the same until the key is moved.
    ///
    /// The result is not bit-identical and cannot be. The sampled value is
    /// rounded to a byte per channel before it is stored, so reading through
    /// the new key differs from reading straight across by at most that
    /// rounding: one unit in 255, which is below what a screen shows. Asserting
    /// equality would assert that a `u8` can hold the midpoint of two `u8`s.
    #[test]
    fn a_new_key_lands_in_the_widest_gap_and_leaves_the_band_where_it_was() {
        let mut b = band(&[0, 1440], &[0x000000, 0xFFFFFF]);
        let before: Vec<Option<u32>> = (0..24).map(|h| b.at(h * 120)).collect();
        add(&mut b);
        assert_eq!(b.times.len(), 3);
        // Two equal gaps of 1440; the first found wins, at 720.
        assert!(b.times.contains(&720));
        let mut sorted = b.clone();
        let mut pairs: Vec<(u32, u32)> = sorted
            .times
            .iter()
            .copied()
            .zip(sorted.values.iter().copied())
            .collect();
        pairs.sort_by_key(|&(t, _)| t);
        sorted.times = pairs.iter().map(|&(t, _)| t).collect();
        sorted.values = pairs.iter().map(|&(_, v)| v).collect();
        let after: Vec<Option<u32>> = (0..24).map(|h| sorted.at(h * 120)).collect();
        for (n, (was, now)) in before.iter().zip(after.iter()).enumerate() {
            let (was, now) = (was.expect("two keys"), now.expect("three keys"));
            for shift in [16, 8, 0] {
                let a = ((was >> shift) & 0xff) as i32;
                let b = ((now >> shift) & 0xff) as i32;
                assert!(
                    (a - b).abs() <= 1,
                    "hour {n}: {was:06X} became {now:06X}, which is more than                      the rounding to a byte"
                );
            }
        }
    }

    /// The widest gap wraps across midnight, because a band is a cycle.
    #[test]
    fn the_widest_gap_wraps_across_midnight() {
        // Keys bunched in the morning: the wide gap is 09:00 round to 06:00,
        // whose middle is in the small hours.
        let at = widest_gap(&[600, 700, 1080]).expect("three keys have gaps");
        assert!(
            at > 1080 || at < 600,
            "the gap across midnight was missed: {at}"
        );
        // Fewer than two keys have no gap at all.
        assert_eq!(widest_gap(&[]), None);
        assert_eq!(widest_gap(&[500]), None);
    }

    /// An empty band gets a first key, and a band of one gets its opposite.
    ///
    /// A single key is a constant: without a second one there is nothing for a
    /// day to be, so the second goes twelve hours away rather than beside it.
    #[test]
    fn a_band_with_nothing_in_it_can_be_started() {
        let mut empty = band(&[], &[]);
        add(&mut empty);
        assert_eq!(empty.times, vec![0]);
        let mut one = band(&[600], &[0x112233]);
        add(&mut one);
        assert_eq!(one.times, vec![600, 600 + DAY / 2]);
        assert_eq!(one.values[1], 0x112233, "the new key reads as the old one");
    }

    /// A time is shown as a clock, which is what a person is thinking in.
    #[test]
    fn a_time_reads_as_the_hour_it_is() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(1440), "12:00");
        assert_eq!(clock(2879), "23:59");
        // Past a day it wraps, like every other read of one.
        assert_eq!(clock(DAY), "00:00");
    }

    /// A time reads back the way it was written. The box used to write `09:00`
    /// and read it back as nine.
    #[test]
    fn a_typed_time_is_read_as_a_clock() {
        assert_eq!(minutes_of_day("09:00"), Some(540));
        assert_eq!(minutes_of_day("9:00"), Some(540));
        assert_eq!(minutes_of_day("21:30"), Some(1290));
        assert_eq!(minutes_of_day("00:00"), Some(0));
        assert_eq!(minutes_of_day(" 6:05 "), Some(365));
        // A bare number is minutes past midnight, which is the value behind
        // the box, so dragging and typing agree.
        assert_eq!(minutes_of_day("540"), Some(540));
        // It also round trips against the formatter, for every hour of the day.
        for minutes in (0..DAY / 2).step_by(7) {
            let shown = clock(minutes * 2);
            assert_eq!(minutes_of_day(&shown), Some(minutes), "{shown}");
        }
        // Refused rather than folded: a typo must not become a different hour.
        assert_eq!(minutes_of_day("24:00"), None);
        assert_eq!(minutes_of_day("10:75"), None);
        assert_eq!(minutes_of_day("1440"), None);
        assert_eq!(minutes_of_day("noon"), None);
        assert_eq!(minutes_of_day(""), None);
    }
}
