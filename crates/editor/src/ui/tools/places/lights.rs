//! The light tool's toolbar and panel.
//!
//! ```text
//! toolbar     [Select | New Light]  Show Falloff  Keys
//! inspector   with no light chosen: the lights of the open map as a list,
//!             the default light first and then the nearest to the camera
//!             with one chosen: the light (its place, its reach, its weight
//!             at the camera), its five conditions as a list, and under the
//!             list the `LightParams` row of the chosen condition, edited in
//!             place: who else uses the row, the sky and water values, and
//!             the bands over the day
//! ```
//!
//! The state is [`crate::tools::places::lights`]; a `LightParams` row's bands
//! are [`super::bands`].
//!
//! ## A light and its `LightParams` rows are one page
//!
//! A `Light` row is a sphere and five ids. Each id names the `LightParams`
//! row the client lights by under one condition
//! ([`Condition`](crate::tools::places::lights::Condition)), and that row
//! with its bands holds every colour. The panel shows both on one page: the
//! five conditions are a list under the light, and the chosen condition's
//! row is drawn under the list. The light stays on the page while its colours
//! are edited, and the page says which condition is open, which lights share
//! the row, and whether the viewport draws that condition.
//!
//! The panel is drawn from the tool's selection
//! (`Lights::selected`) and not from the table browser's open row. An
//! earlier version drew the browser's row, so a press on a condition's link
//! replaced the light's form with the `LightParams` row's form, and neither
//! the condition nor the sharing was shown there.
//!
//! ## What the table workspace borrows
//!
//! [`sphere_block`] and [`place_strip`] are drawn by the table workspace's
//! form (`crate::ui::tables::form`), among that form's own sections, when a
//! `Light` row is open there. `sphere_block` is laid out to that form's
//! measure for that reason.

use bevy_egui::egui;

use crate::tables::{self, Modal};
use crate::tools::places::lights::{Condition, Lights, Mark, PanelData, Skybox, Use};
use crate::ui::tables::modals::modals;
use crate::ui::tables::{Workspace, LABEL, VALUE};
use crate::ui::widgets::theme::{self, bar_group, bar_keys, bar_segment, bar_switch, property};

use vale_assets::tables::light::{light_field as lf, YARDS_PER_UNIT};

/// The fields of `LightParams.dbc` the panel edits, by column number.
mod params_field {
    pub(super) const HIGHLIGHT_SKY: usize = 1;
    pub(super) const SKYBOX: usize = 2;
    pub(super) const GLOW: usize = 4;
    pub(super) const WATER_SHALLOW_ALPHA: usize = 5;
    pub(super) const WATER_DEEP_ALPHA: usize = 6;
    pub(super) const OCEAN_SHALLOW_ALPHA: usize = 7;
    pub(super) const OCEAN_DEEP_ALPHA: usize = 8;
}

/// The light tool's toolbar over the viewport:
///
/// ```text
/// [Select | New Light]  Show Falloff  Keys
/// ```
///
/// New Light arms the pointer: while it is chosen a click on the ground makes
/// a light, and Escape returns to Select. Show Falloff is whether the selected
/// light's two spheres are drawn. See [`theme::toolbar`].
pub(crate) fn toolbar(
    ctx: &egui::Context,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    lights: &mut Lights,
) -> Option<egui::Rect> {
    theme::toolbar(ctx, "lights-toolbar", viewport, corner, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            bar_group(ui, |ui| {
                if bar_segment(ui, !lights.armed, "Select")
                    .on_hover_text("Click a light to select it. Drag its centre to move it, or a ring to resize it.")
                    .clicked()
                {
                    lights.armed = false;
                }
                if bar_segment(ui, lights.armed, "New Light")
                    .on_hover_text(
                        "A click on the ground places a light there: full strength to 50 yards, \
                         faded out at 120 yards, with the LightParams of the selected light or \
                         the map's default light.",
                    )
                    .clicked()
                {
                    lights.armed = true;
                }
            });
            if bar_switch(ui, lights.show_falloff, "Show Falloff")
                .on_hover_text(
                    "On: draw the selected light's FalloffStart and FalloffEnd spheres. Off: \
                     draw only its marker. Every light near the camera is marked either way.",
                )
                .clicked()
            {
                lights.show_falloff = !lights.show_falloff;
            }
            bar_keys(ui, match lights.armed {
                false => &SELECT_KEYS,
                true => &NEW_KEYS,
            });
        });
    })
}

/// Select's controls, for the toolbar's Keys button.
const SELECT_KEYS: [(&str, &str); 5] = [
    ("Left button", "Select a light"),
    ("Drag the centre", "Move the light; its height is kept"),
    ("Drag a ring", "Change that falloff radius"),
    ("F", "Move the camera to the selected light"),
    ("Esc", "Deselect the light and list the map's lights"),
];

/// New Light's controls.
const NEW_KEYS: [(&str, &str); 2] = [
    ("Left button", "Place a light on the ground"),
    ("Escape", "Return to Select"),
];

/// The light tool's inspector panel. See the module comment.
///
/// A press or drag on a band's day asks for the viewport's hour, which is the
/// clock's and is written here once the panel is drawn.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { tool, session, assets, now, .. } = subject;
    let hour = light_inspector(ui, Workspace::new(tool, session, editing, assets, now, true, false));
    // An hour set by hand is applied at once, as the view bar's is.
    if let Some(hour) = hour.filter(|hour| editing.clock.override_half_minutes != Some(*hour)) {
        editing.clock.override_half_minutes = Some(hour);
        editing.clock.half_minutes = hour;
    }
}

/// The panel: the chosen light's page, or with none chosen the list of the
/// map's lights. Answers the hour a band was pressed at, in half-minutes.
fn light_inspector(ui: &mut egui::Ui, mut work: Workspace<'_>) -> Option<u32> {
    if !work.session.open_table(work.assets, "Light") {
        theme::note(ui, "Opening Light.dbc\u{2026}");
        return None;
    }
    let chosen = work.lights.as_deref().and_then(|lights| lights.selected);
    let record = chosen.and_then(|id| work.browser.record_of(work.session, "Light", id));
    let hour = match (chosen, record) {
        (Some(id), Some(record)) => light_page(ui, &mut work, id, record),
        _ => {
            // A chosen light whose row is gone, by an undo or a discard, is
            // no longer chosen.
            if let Some(lights) = work.lights.as_deref_mut() {
                lights.selected = None;
            }
            light_list(ui, &mut work);
            None
        }
    };
    modals(ui, &mut work, None);
    hour
}

/// Select a light: in the tool, and in the table browser, which the tool's
/// pick and the Tables workspace both read.
fn select(work: &mut Workspace<'_>, id: u32) {
    if let Some(lights) = work.lights.as_deref_mut() {
        lights.selected = Some(id);
    }
    work.browser.look_at("Light");
    let session = &*work.session;
    work.browser.follow(session, "Light", id);
}

/// What a light is called: lights have no name column.
fn light_name(mark: Option<&Mark>, id: u32) -> String {
    match mark.is_some_and(|mark| mark.everywhere) {
        true => "Default Light".to_string(),
        false => format!("Light {id}"),
    }
}

/// A weight as a whole percentage.
fn percent(weight: f32) -> String {
    format!("{:.0}%", weight * 100.0)
}

/// The panel with no light chosen: the buttons that find or make one, then
/// the lights of the open map. The default light is first, because it lights
/// most of the map; the others follow by distance from the camera, so the
/// lights that colour what is on screen are at the top.
fn light_list(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    browse_button(ui, work);
    new_default_light(ui, work);

    let Some(lights) = work.lights.as_deref() else { return };
    let eye = lights.eye;
    let mut rows: Vec<(Mark, f32)> = lights
        .marks
        .iter()
        .map(|mark| (*mark, eye.map_or(0.0, |eye| mark.at.distance(eye))))
        .collect();
    rows.sort_by(|a, b| {
        b.0.everywhere
            .cmp(&a.0.everywhere)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.0.id.cmp(&b.0.id))
    });

    theme::heading_about(
        ui,
        &format!("Lights on This Map ({})", rows.len()),
        "Click a light here or its marker in the world to edit it. Double-click one here to \
         move the camera to it. The default light applies wherever no other light does; the \
         others are listed nearest the camera first. A percentage is the light's weight \
         where the camera is; a light with none does not colour what is on screen.",
    );
    if rows.is_empty() {
        theme::note(ui, "This map has no light. The client lights it by the Eastern Kingdoms default light.");
        return;
    }
    let mut pick: Option<(Mark, bool)> = None;
    // Only the rows in view are laid out and worded.
    let height = theme::LIST_ROW - 2.0;
    egui::ScrollArea::vertical().id_salt("light-list").auto_shrink([false, false]).show_rows(ui, height, rows.len(), |ui, range| {
        for (mark, distance) in &rows[range] {
            let title = light_name(Some(mark), mark.id);
            let sub = match mark.everywhere {
                true => "Whole map".to_string(),
                false => {
                    let weight = eye.map_or(0.0, |eye| mark.weight(eye));
                    // The weight leads, because it is the shortest part and
                    // the row is cut at its end where the panel is narrow.
                    let reach = format!("{:.0}..{:.0} yd \u{b7} {distance:.0} yd away", mark.start, mark.end);
                    match weight > 0.0 {
                        true => format!("{} here \u{b7} {reach}", percent(weight)),
                        false => reach,
                    }
                }
            };
            let id = mark.id.to_string();
            let row = theme::ListRow { title: &title, sub: &sub, trailing: &id, tint: theme::INK, picture: false };
            let response = theme::list_row(ui, row, false).response;
            if response.double_clicked() {
                pick = Some((*mark, true));
            } else if response.clicked() {
                pick = Some((*mark, false));
            }
        }
    });
    if let Some((mark, fly)) = pick {
        select(work, mark.id);
        if fly && !mark.everywhere {
            work.fly.write(Lights::fly_to(&mark));
        }
    }
}

/// The page of one light: its heading, its place, its five conditions, and
/// the `LightParams` row of the chosen condition. Answers the hour a band was
/// pressed at.
fn light_page(ui: &mut egui::Ui, work: &mut Workspace<'_>, id: u32, record: usize) -> Option<u32> {
    let mark = work.lights.as_deref().and_then(|lights| lights.marks.iter().copied().find(|mark| mark.id == id));
    let eye = work.lights.as_deref().and_then(|lights| lights.eye);

    ui.add_space(4.0);
    let mut deselect = false;
    ui.horizontal(|ui| {
        deselect = ui
            .button("All Lights")
            .on_hover_text("Deselect this light and list the lights of this map. Escape does the same.")
            .clicked();
        if let Some(mark) = mark.filter(|mark| !mark.everywhere) {
            if ui
                .button("Fly To")
                .on_hover_text("Move the camera to this light's position, at the row's stored height. F does the same.")
                .clicked()
            {
                work.fly.write(Lights::fly_to(&mark));
            }
        }
    });
    if deselect {
        if let Some(lights) = work.lights.as_deref_mut() {
            lights.selected = None;
        }
        return None;
    }

    theme::heading(ui, &light_name(mark.as_ref(), id));
    ui.horizontal(|ui| {
        theme::note(ui, format!("Light.dbc \u{b7} ID {id}"));
        if work.session.unsaved_tables.contains("Light") {
            ui.label(egui::RichText::new("Unsaved").size(theme::SMALL).color(theme::WARN));
        }
    });

    let mut hour = None;
    egui::ScrollArea::vertical().id_salt("light-form").auto_shrink([false, false]).show(ui, |ui| {
        place(ui, work, record, mark.as_ref(), eye);
        hour = conditions(ui, work, id, record, mark.as_ref());
        ui.add_space(12.0);
    });
    hour
}

/// The light's place and reach, in yards and in the world's axes.
///
/// The default light has neither: its `FalloffEnd` is 0, it applies wherever
/// no other light does, and its position is not read. A light on another map
/// has both, and no weight at this map's camera.
fn place(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize, mark: Option<&Mark>, eye: Option<bevy::math::Vec3>) {
    let Some(was) = Sphere::read(work, record) else { return };
    if was.end <= 0.0 {
        theme::heading(ui, "Place");
        property(ui, "Reach", "", |ui| {
            theme::value(ui, "Whole map");
        });
        theme::note(
            ui,
            "The default light applies wherever no other light does, and under every other \
             light's falloff. Its position is not read.",
        );
        return;
    }
    theme::heading_about(
        ui,
        "Place",
        "The light's centre and its two radii, in yards. In the viewport, drag the centre to \
         move the light and a ring to change that radius; the height changes only here.",
    );
    let mut now = was;
    theme::xyz(ui, &mut now.at);
    property(
        ui,
        "FalloffStart",
        "The radius within which the light applies at full weight: the inner ring in the \
         viewport. Stored in 1/36 of a yard.",
        |ui| {
            theme::field(ui, egui::DragValue::new(&mut now.start).speed(1.0).range(0.0..=f32::MAX).max_decimals(1).suffix(" yd"));
        },
    );
    property(
        ui,
        "FalloffEnd",
        "The radius at which the light has faded out: the outer ring in the viewport. Its \
         weight falls linearly from FalloffStart to here. Stored in 1/36 of a yard.",
        |ui| {
            // Kept above zero: a FalloffEnd of 0 makes the row the map's
            // default light, which is a different thing and not a radius.
            theme::field(ui, egui::DragValue::new(&mut now.end).speed(1.0).range(1.0..=f32::MAX).max_decimals(1).suffix(" yd"));
        },
    );
    if now != was {
        now.write(work, record);
    }
    match (mark, eye) {
        (Some(mark), Some(eye)) => {
            let weight = mark.weight(eye);
            let distance = mark.at.distance(eye);
            let words = match weight > 0.0 {
                true => format!("{} \u{b7} {distance:.0} yd from centre", percent(weight)),
                false => format!("0% \u{b7} {:.0} yd outside", distance - mark.end),
            };
            property(
                ui,
                "At the Camera",
                "How strongly this light applies where the camera is: 100% within FalloffStart, \
                 falling to 0% at FalloffEnd. Beside it, the camera's distance from the light's \
                 centre, or from FalloffEnd when it is outside. At 0% an edit to this light's \
                 colours changes nothing in the viewport. Where two lights overlap, the nearer \
                 centre is laid over the farther.",
                |ui| {
                    let colour = match weight > 0.0 {
                        true => theme::INK,
                        false => theme::WARN,
                    };
                    ui.add(egui::Label::new(egui::RichText::new(words).color(colour)).truncate());
                },
            );
        }
        (None, _) => theme::note(ui, "This light is on another map. Open that map to see it in the world."),
        (Some(_), None) => {}
    }
}

/// The five conditions as a list, and under it the `LightParams` row of the
/// chosen one. Answers the hour a band was pressed at.
fn conditions(ui: &mut egui::Ui, work: &mut Workspace<'_>, id: u32, record: usize, mark: Option<&Mark>) -> Option<u32> {
    theme::heading_about(
        ui,
        "Conditions",
        "A light names one LightParams row for each of five conditions, and the client lights \
         by the row of the condition that holds. Click a condition to edit its row below. The \
         number is the row's id, followed by how many lights use that row.",
    );
    let mut chosen = work.lights.as_deref().map_or(Condition::Clear, |lights| lights.condition);
    let ids: Vec<u32> = {
        let Some(table) = work.session.table("Light") else { return None };
        Condition::ALL.iter().map(|condition| table.u32_at(record, condition.field()).unwrap_or(0)).collect()
    };
    // Who uses each row, and the skybox rows, are read from the tool's
    // cache, which is built again only when a table has changed.
    let data = work.lights.as_deref_mut().map(|lights| lights.panel_data(work.session)).unwrap_or_default();
    for (condition, &params) in Condition::ALL.iter().zip(&ids) {
        let right = match (params, condition.falls_back_to()) {
            (0, Some(to)) => format!("None \u{b7} uses {}", to.name()),
            (0, None) => "None".to_string(),
            (params, _) => match data.users.get(&params).map_or(0, Vec::len) {
                0 | 1 => format!("{params}"),
                many => format!("{params} \u{b7} {many} lights"),
            },
        };
        if condition_row(ui, condition.name(), &right, *condition == chosen)
            .on_hover_text(format!("{}\n\n{}", condition.column(), condition.when()))
            .clicked()
        {
            chosen = *condition;
        }
    }
    if let Some(lights) = work.lights.as_deref_mut() {
        lights.condition = chosen;
    }
    let params = ids[Condition::ALL.iter().position(|condition| *condition == chosen).unwrap_or(0)];
    params_block(ui, work, &data, id, record, mark, chosen, params)
}

/// One row of the condition list: the name on the left, the row it names on
/// the right, and the whole row is the button. The right-hand text is cut
/// with an ellipsis where the two do not fit.
fn condition_row(ui: &mut egui::Ui, name: &str, right: &str, chosen: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    if chosen {
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(3),
            theme::ACCENT_SUNK,
            egui::Stroke::new(1.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(3), theme::RAISED);
    }
    let inner = rect.shrink2(egui::vec2(8.0, 0.0));
    let mut line = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::right_to_left(egui::Align::Center))
            .sense(egui::Sense::hover()),
    );
    line.spacing_mut().item_spacing.x = 8.0;
    // The name is laid out first so that it keeps its width, and the row it
    // names takes what is left.
    let ink = match chosen {
        true => theme::INK,
        false => theme::INK_NAME,
    };
    let name_width = line
        .painter()
        .layout_no_wrap(name.to_string(), egui::TextStyle::Body.resolve(line.style()), ink)
        .size()
        .x;
    let room = (inner.width() - name_width - 8.0).max(0.0);
    line.allocate_ui_with_layout(
        egui::vec2(room, inner.height()),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.add(egui::Label::new(theme::number(right).color(theme::INK_DIM)).truncate().selectable(false));
        },
    );
    line.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.add(egui::Label::new(egui::RichText::new(name).color(ink)).selectable(false));
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The chosen condition's `LightParams` row: which row, the buttons that
/// choose, copy or make one, who else uses it, whether the viewport draws the
/// condition, and then the row's values and bands. Answers the hour a band
/// was pressed at.
#[allow(clippy::too_many_arguments)]
fn params_block(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    data: &PanelData,
    id: u32,
    record: usize,
    mark: Option<&Mark>,
    condition: Condition,
    params: u32,
) -> Option<u32> {
    let title = match params {
        0 => format!("{} \u{b7} None", condition.name()),
        params => format!("{} \u{b7} LightParams {params}", condition.name()),
    };
    theme::heading_about(ui, &title, &format!("{}\n\n{}", condition.column(), condition.when()));

    // Choose a row, and copy or make one.
    let target = Some(("Light", record, condition.field()));
    // The row is shared when any use of it is not this condition of this
    // light: another light's, or another condition of this one.
    let uses: &[Use] = data.users.get(&params).map_or(&[], Vec::as_slice);
    let shared = uses.iter().any(|each| each.light != id || each.without(condition).is_some());
    let mut made: Option<u32> = None;
    ui.columns(2, |columns| {
        if columns[0]
            .add(theme::full_button(&columns[0], "Choose\u{2026}"))
            .on_hover_text("Choose the LightParams row this condition names, from the list of all of them.")
            .clicked()
        {
            work.browser.modal = Some(Modal::Pick {
                table: "Light".to_string(),
                record,
                field: condition.field(),
                points_at: "LightParams",
            });
            work.browser.pick_query.clear();
            work.browser.pick_focus = true;
        }
        match params {
            0 => {
                if columns[1]
                    .add(theme::full_button(&columns[1], "New Row"))
                    .on_hover_text(
                        "Create a LightParams row with its 18 colour bands and 6 float bands, copied \
                         from this light's Clear row (or from the Eastern Kingdoms default light's), \
                         and point this condition at it.",
                    )
                    .clicked()
                {
                    let from = light_params_to_copy(work, "Light", record);
                    made = new_light_params(work, from, target);
                }
            }
            params => {
                let button = theme::full_button(&columns[1], "Copy Row");
                if columns[1]
                    .add_enabled(shared, button)
                    .on_hover_text(
                        "Copy this LightParams row with its 18 colour bands and 6 float bands, and \
                         point this condition at the copy. Every other use of the original keeps it.",
                    )
                    .on_disabled_hover_text("Only this condition of this light uses the row.")
                    .clicked()
                {
                    made = new_light_params(work, params, target);
                }
            }
        }
    });
    if made.is_some() {
        work.browser.forget_buffers();
    }

    if params == 0 {
        theme::note(
            ui,
            match condition.falls_back_to() {
                Some(to) => format!("This light names no row for {}. The client lights by its {} row.", condition.name(), to.name()),
                None => format!("This light names no row for {}.", condition.name()),
            },
        );
        return None;
    }
    shared_line(ui, work, id, params, condition, uses);
    if condition == Condition::Death && mark.is_some_and(|mark| !mark.everywhere) {
        theme::note(ui, "The client reads the Death row of the map's default light only. This light's is not used.");
    } else if !condition.drawn_in_the_editor() {
        theme::note(
            ui,
            match condition {
                Condition::Death => "The editor's viewport does not draw this condition: it has no ghost world.",
                _ => "The editor's viewport does not draw this condition: it has no weather. A playtest draws it while it rains.",
            },
        );
    }

    for table in ["LightParams", "LightSkybox"] {
        if !work.session.open_table(work.assets, table) {
            theme::note(ui, format!("Opening {table}.dbc\u{2026}"));
            return None;
        }
    }
    let Some(row) = work.browser.record_of(work.session, "LightParams", params) else {
        ui.label(egui::RichText::new(format!("No LightParams row has the id {params}.")).color(theme::BAD));
        return None;
    };
    params_fields(ui, work, row, &data.skyboxes);
    let hour = work.hour;
    super::bands::blocks(ui, work, params, hour)
}

/// Who else uses the open `LightParams` row, because an edit to it changes
/// every one of them. Each light is a link that selects it. `uses` is every
/// use of the row, this light's included.
fn shared_line(ui: &mut egui::Ui, work: &mut Workspace<'_>, id: u32, params: u32, condition: Condition, uses: &[Use]) {
    let others = uses.iter().filter(|each| each.light != id).count();
    // The other conditions this light names the same row under.
    let own: Vec<&str> = uses
        .iter()
        .filter(|each| each.light == id)
        .filter_map(|each| each.without(condition))
        .flat_map(|each| each.conditions().map(|other| other.name()))
        .collect();
    if others == 0 && own.is_empty() {
        theme::note(ui, "Only this condition of this light uses the row.");
        return;
    }
    let mut follow: Option<u32> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut words = format!("An edit to LightParams {params} also changes");
        if !own.is_empty() {
            words.push_str(&format!(" this light's {}", own.join(" and ")));
        }
        match (own.is_empty(), others) {
            (_, 0) => words.push('.'),
            (true, 1) => words.push_str(" 1 other light:"),
            (true, n) => words.push_str(&format!(" {n} other lights:")),
            (false, 1) => words.push_str(" and 1 other light:"),
            (false, n) => words.push_str(&format!(" and {n} other lights:")),
        }
        ui.label(egui::RichText::new(words).size(theme::SMALL).color(theme::WARN));
        // The first eight, which is two lines of a docked inspector.
        const SHOWN: usize = 8;
        for each in uses.iter().filter(|each| each.light != id).take(SHOWN) {
            let name = format!("Light {}", each.light);
            let under: Vec<&str> = each.conditions().map(|other| other.name()).collect();
            let hover = format!("Uses the row for {}. Click to select that light.", under.join(", "));
            if crate::ui::widgets::reference::link_text(ui, egui::RichText::new(name).size(theme::SMALL), theme::ACCENT, &hover) {
                follow = Some(each.light);
            }
        }
        if others > SHOWN {
            ui.label(egui::RichText::new("\u{2026}").size(theme::SMALL).color(theme::INK_FAINT));
        }
    });
    if let Some(light) = follow {
        select(work, light);
    }
}

/// The values of a `LightParams` row that are not bands: whether the sky is
/// highlighted, the skybox model, the glow, and the four water opacities.
/// `CloudType` is not drawn: it is 0 on all 426 shipped rows and the client
/// does not read it. Every column is in the Tables workspace.
fn params_fields(ui: &mut egui::Ui, work: &mut Workspace<'_>, row: usize, skyboxes: &[Skybox]) {
    let Some(table) = work.session.table("LightParams") else { return };
    let raw = |field: usize| table.u32_at(row, field).unwrap_or(0);
    let float = |field: usize| f32::from_bits(raw(field));
    let mut writes: Vec<(usize, u32)> = Vec::new();

    theme::heading(ui, "Sky");
    let mut highlight = raw(params_field::HIGHLIGHT_SKY) != 0;
    property(ui, "HighlightSky", "0 or 1 on all 426 shipped rows.", |ui| {
        if ui.checkbox(&mut highlight, "").changed() {
            writes.push((params_field::HIGHLIGHT_SKY, u32::from(highlight)));
        }
    });

    // The skybox is chosen from the rows of `LightSkybox.dbc` as the project
    // has the table now (`Lights::panel_data`, built again when a table
    // changes). A row the project adds, here or in the Tables workspace, is
    // in the list; nothing about which skyboxes exist is written in this
    // file.
    let skybox = raw(params_field::SKYBOX);
    let missing = format!("LightSkybox {skybox}, which has no row");
    let mut options: Vec<(&str, u32)> = vec![("None", 0)];
    options.extend(skyboxes.iter().map(|each| (each.label.as_str(), each.id)));
    if !options.iter().any(|(_, id)| *id == skybox) {
        options.push((&missing, skybox));
    }
    let model = skyboxes.iter().find(|each| each.id == skybox && each.id != 0);
    let mut picked = skybox;
    property(
        ui,
        "Skybox",
        "A model drawn around the camera in place of the sky's own dome, as a row of \
         LightSkybox.dbc. The list is that table's rows as this project has them, so a row \
         added for an imported model is in it. Four of the five shipped rows that set a \
         skybox are Death rows, and all five name DeathClouds.",
        |ui| {
            theme::choice(ui, "light-skybox", &mut picked, &options, |a, b| a == b);
        },
    );
    if picked != skybox {
        writes.push((params_field::SKYBOX, picked));
    }

    // The chosen row's model path, edited here, so a skybox of the project's
    // own is named without leaving the light. The text is kept beside the
    // box while it has the keyboard and written when it loses it: a write
    // per key press would add a string to the table's string block each time.
    let mut model_write: Option<(usize, String)> = None;
    if let Some(chosen) = model.filter(|_| picked == skybox) {
        let (record, declared) = (chosen.record, &chosen.model);
        let id = ui.make_persistent_id(("light-skybox-model", skybox));
        let mut text = ui.data_mut(|data| data.get_temp::<String>(id)).unwrap_or_else(|| declared.clone());
        property(
            ui,
            "Skybox Model",
            "LightSkybox.Model: the model's path with the .mdx extension, which the client reads \
             as the .m2 of the same name. The shipped ones are under Environments\\Stars\\. This \
             row is shared: every LightParams row that names it draws the same model.",
            |ui| {
                let response = theme::text_field(ui, &mut text);
                let focused = response.has_focus();
                ui.data_mut(|data| match focused {
                    true => {
                        data.insert_temp(id, text.clone());
                    }
                    false => data.remove::<String>(id),
                });
                if response.lost_focus() && text != *declared {
                    model_write = Some((record, text.clone()));
                }
            },
        );
        // Asked on each frame the row is shown, because a model imported
        // after the row was made moves no table revision. It is one lookup
        // in the archive chain's index.
        let path = &chosen.path;
        let present = !path.is_empty() && work.assets.with_archive(|chain| Ok(chain.exists(path))).unwrap_or(false);
        if !present {
            ui.label(
                egui::RichText::new(match declared.is_empty() {
                    true => "This skybox row names no model. Nothing is drawn for it.".to_string(),
                    false => format!("No file {path} is in the archives or the project. Nothing is drawn for it."),
                })
                .size(theme::SMALL)
                .color(theme::WARN),
            );
        }
    }
    let new_box = ui
        .button("New Skybox Row")
        .on_hover_text(
            "Add a blank row to LightSkybox.dbc and point this LightParams row at it, as one \
             undo entry. Then type the model's path under Skybox Model. Import the model in \
             the Assets workspace first.",
        )
        .clicked();

    let mut unit = |ui: &mut egui::Ui, name: &str, about: &str, field: usize| {
        let mut value = float(field);
        property(ui, name, about, |ui| {
            if theme::field(ui, egui::DragValue::new(&mut value).speed(0.01).range(0.0..=1.0).max_decimals(2)).changed() {
                writes.push((field, value.to_bits()));
            }
        });
    };
    unit(
        ui,
        "Glow",
        "The strength of the full-screen effect drawn while the player is a ghost under this \
         row. The ordinary glow does not read it. 0.0 to 1.0, over 13 distinct values in the \
         shipped rows.",
        params_field::GLOW,
    );

    theme::heading_about(
        ui,
        "Water Opacity",
        "How opaque the water is drawn while this row lights it, 0.0 to 1.0. These four columns \
         affect water only.",
    );
    unit(ui, "Water Shallow", "WaterShallowAlpha: how opaque a river is near the camera.", params_field::WATER_SHALLOW_ALPHA);
    unit(ui, "Water Deep", "WaterDeepAlpha.", params_field::WATER_DEEP_ALPHA);
    unit(ui, "Ocean Shallow", "OceanShallowAlpha.", params_field::OCEAN_SHALLOW_ALPHA);
    unit(ui, "Ocean Deep", "OceanDeepAlpha.", params_field::OCEAN_DEEP_ALPHA);

    if !writes.is_empty() {
        let subject = format!("LightParams {row} field {}", writes[0].0);
        tables::fields::set_fields(work.session, "LightParams", row, &writes, "Edit light params", &subject, work.now);
    }
    if let Some((record, text)) = model_write {
        tables::fields::set_text(work.session, "LightSkybox", record, SKYBOX_MODEL, &text, "Edit skybox model", work.now);
    }
    if new_box {
        work.session.status = match tables::rows::link_new(work.session, "LightParams", row, params_field::SKYBOX, "LightSkybox") {
            Some(id) => format!("Created LightSkybox {id}; this LightParams row now names it"),
            None => "Could not create a LightSkybox row".to_string(),
        };
    }
}

/// `LightSkybox.dbc`'s model column.
const SKYBOX_MODEL: usize = 1;

/// Make the map's default light, offered while the map has none. It copies
/// its weather from the chosen light; see
/// `crate::tools::places::lights::new_light`. A light placed by a click is
/// the toolbar's New Light, [`toolbar`].
fn new_default_light(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let Some(lights) = work.lights.as_deref_mut() else {
        return;
    };
    if lights.default_light().is_none() {
        let map = work.session.map_id;
        let button = theme::full_button(ui, "New Default Light");
        if ui
            .add(button)
            .on_hover_text(
                "This map has no default light (the light that applies wherever no \
                 other light covers). Creates one with the LightParams of the selected \
                 light, another light on this map, or light 1 (the Eastern Kingdoms \
                 default).",
            )
            .clicked()
        {
            let line = crate::tools::places::lights::new_light(work.session, lights, work.browser, map, None);
            work.session.status = line;
        }
    }
}

/// A button that opens the list of every light of every map as a dialog, for
/// a light that is not on the open map or is hard to find in the panel's
/// list. The camera moves to the chosen light.
fn browse_button(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let count = work
        .session
        .table("Light")
        .map(|table| table.record_count())
        .unwrap_or(0);
    if ui
        .add_sized(
            egui::vec2(ui.available_width(), 24.0),
            egui::Button::new(format!("Browse All {count} Lights\u{2026}")),
        )
        .on_hover_text("Choose a light of any map from a searched list. The camera moves to the chosen light.")
        .clicked()
    {
        work.browser.modal = Some(Modal::Rows { table: "Light" });
        work.browser.pick_query.clear();
        work.browser.pick_page = 0;
        work.browser.pick_focus = true;
    }
}

/// A `Light` row's centre and two radii, in yards and in the world's axes.
///
/// The five columns are
/// [`light_field`](vale_assets::tables::light::light_field)'s: three
/// coordinates in the internal representation every placement in the game
/// uses, scaled by 36, and two radii in the same unit. The axes swap: the
/// world's x is `InternalZ` in the file.
/// [`vale_assets::world::adt::placement_from_world`] converts back; loading
/// the game's tiles never uses that direction, so it is called here and not
/// written again.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sphere {
    at: [f32; 3],
    start: f32,
    end: f32,
}

impl Sphere {
    fn read(work: &Workspace<'_>, record: usize) -> Option<Sphere> {
        let table = work.session.table("Light")?;
        let raw = |field: usize| table.f32_at(record, field).unwrap_or(0.0);
        Some(Sphere {
            at: vale_assets::world::adt::placement_to_world([
                raw(lf::INTERNAL_X) * YARDS_PER_UNIT,
                raw(lf::INTERNAL_Y) * YARDS_PER_UNIT,
                raw(lf::INTERNAL_Z) * YARDS_PER_UNIT,
            ]),
            start: raw(lf::FALLOFF_START) * YARDS_PER_UNIT,
            end: raw(lf::FALLOFF_END) * YARDS_PER_UNIT,
        })
    }

    /// Write all five columns under one gesture, so a drag on one box is one
    /// entry on the undo stack and not one per pixel.
    fn write(&self, work: &mut Workspace<'_>, record: usize) {
        let placement = vale_assets::world::adt::placement_from_world(self.at);
        let store = |yards: f32| (yards / YARDS_PER_UNIT).to_bits();
        tables::fields::set_fields(
            work.session,
            "Light",
            record,
            &[
                (lf::INTERNAL_X, store(placement[0])),
                (lf::INTERNAL_Y, store(placement[1])),
                (lf::INTERNAL_Z, store(placement[2])),
                (lf::FALLOFF_START, store(self.start)),
                (lf::FALLOFF_END, store(self.end)),
            ],
            "Edit light sphere",
            "Light sphere",
            work.now,
        );
    }
}

/// A light's position and falloff in yards, as a section of the table
/// workspace's form: the `Sphere` section, drawn in the world's units in
/// place of the file's. The inspector's own rows are [`place`].
///
/// The unit changes and the radius names do not: `FalloffStart` and
/// `FalloffEnd` keep the columns' names. The three coordinates are labelled as
/// the world's `X`, `Y` and `Z`, because after conversion they no longer match
/// the columns they come from. Each field's tooltip names the column it
/// writes and its unit.
///
/// The heading and the rows are the table form's and not the inspector's
/// property rows, because this section is drawn among that form's own
/// sections and matches them.
pub(in crate::ui) fn sphere_block(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    let Some(was) = Sphere::read(work, record) else {
        return;
    };
    let mut now = was;

    egui::CollapsingHeader::new(egui::RichText::new("Sphere").size(14.0).strong())
        .default_open(true)
        .show(ui, |ui| {
            let row = |ui: &mut egui::Ui, name: &str, value: &mut f32, tip: &str| {
                ui.horizontal(|ui| {
                    let label_width = LABEL.min(ui.available_width() * 0.45);
                    ui.add_sized(
                        egui::vec2(label_width, 22.0),
                        egui::Label::new(egui::RichText::new(name).color(theme::INK_NAME))
                            .truncate(),
                    )
                    .on_hover_text(tip);
                    ui.add_sized(
                        egui::vec2(VALUE, 22.0),
                        egui::DragValue::new(value).speed(1.0).suffix(" yd"),
                    );
                });
            };
            row(
                ui,
                "X",
                &mut now.at[0],
                "The world's x. Stored as InternalZ, which is northward from \
                 the corner of the map in 1/36 of a yard.",
            );
            row(
                ui,
                "Y",
                &mut now.at[1],
                "The world's y. Stored as InternalX, which is westward from \
                 the corner of the map in 1/36 of a yard.",
            );
            row(
                ui,
                "Z",
                &mut now.at[2],
                "The world's z. Stored as InternalY, in 1/36 of a yard. A \
                 drag in the world moves x and y only, so z changes only \
                 here.",
            );
            ui.add_space(4.0);
            row(
                ui,
                "FalloffStart",
                &mut now.start,
                "The radius within which the light applies at full strength. \
                 Drawn as the inner sphere in the world, where it can be \
                 dragged. Stored in 1/36 of a yard.",
            );
            row(
                ui,
                "FalloffEnd",
                &mut now.end,
                "The radius at which the light has faded out completely. Zero makes this \
                 the map's default light, which applies everywhere and whose \
                 position is not read. Stored in 1/36 of a yard.",
            );
        });

    if now != was {
        now.write(work, record);
    }
}

/// The rows under a `Light` row's heading in the table workspace's form:
/// where the light is and how far it reaches, and a button to fly the camera
/// there.
///
/// The row's three coordinates are stored in a form that cannot be read at a
/// glance (`612096, 0, 998400`, in 1/36 of a yard from the corner of the map),
/// so the form alone does not show where the light is. These rows give the
/// position in world coordinates, keep the viewport's ring on the open row,
/// and fly the camera there.
///
/// They are drawn only for `Light`. The other four tables of the chain have
/// no position: a `LightParams` row is a look, and where it applies is
/// whichever lights point at it.
///
/// The position and the radii are property rows and the button is under
/// them. On one line they were wider than the inspector.
pub(in crate::ui) fn place_strip(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    let Some(id) = work
        .session
        .table("Light")
        .and_then(|table| table.u32_at(record, 0))
    else {
        return;
    };
    let Some(lights) = work.lights.as_deref_mut() else {
        return;
    };
    // Opening a row selects its light. Picking a light already opens its row;
    // without this direction, a row reached from the list or by following a
    // reference left the viewport highlighting the previous light, which is
    // the ring a person would then drag.
    if lights.selected != Some(id) {
        lights.selected = Some(id);
    }
    let Some(&mark) = lights.marks.iter().find(|mark| mark.id == id) else {
        // A light on another map. The row can still be edited, because a
        // `Light` row names its own map, but there is nothing to fly to here.
        theme::note(
            ui,
            "This light is on another map. Open that map to see it in the world.",
        );
        return;
    };

    match mark.everywhere {
        // The default light applies everywhere and its coordinates are not
        // read, so there is no position to fly to.
        true => {
            property(ui, "Position", "", |ui| {
                theme::value(ui, "Default light, whole map");
            });
        }
        false => {
            property(ui, "Position", "", |ui| {
                theme::value(ui, format!("{:.0}, {:.0}, {:.0}", mark.at.x, mark.at.y, mark.at.z));
            });
            property(ui, "Falloff", "", |ui| {
                theme::value(ui, format!("{:.0}..{:.0} yd", mark.start, mark.end));
            });
        }
    }
    if !mark.everywhere
        && ui
            .button("Fly To")
            .on_hover_text("Move the camera to this light's position, at the row's stored height.")
            .clicked()
    {
        work.fly.write(Lights::fly_to(&mark));
    }
}

/// Make a `LightParams` row copied from `from`, with its bands, through
/// [`tables::rows::copy_light_params`], opening the two band tables first. Says what
/// it did on the status line. Returns the new row's id.
pub(in crate::ui) fn new_light_params(
    work: &mut Workspace<'_>,
    from: u32,
    assign: Option<(&str, usize, usize)>,
) -> Option<u32> {
    for table in ["LightParams", "LightIntBand", "LightFloatBand"] {
        if !work.session.open_table(work.assets, table) {
            work.session.status = format!("{table}.dbc is not open yet; try again");
            return None;
        }
    }
    let made = tables::rows::copy_light_params(work.session, from, assign);
    work.session.status = match made {
        Some(id) => format!("Created LightParams {id} with its bands, copied from LightParams {from}"),
        None => "Could not create a LightParams row".to_string(),
    };
    made
}

/// What a new `LightParams` row on a light's form copies: the light's own
/// clear `LightParams`, or, when that is empty too, the Eastern Kingdoms
/// default light's (`Light` row 1), which every map without a light of its own
/// is lit by.
pub(in crate::ui) fn light_params_to_copy(work: &Workspace<'_>, table_name: &str, record: usize) -> u32 {
    let Some(lights) = work.session.table("Light") else {
        return 0;
    };
    let own = (table_name == "Light")
        .then(|| lights.u32_at(record, lf::PARAMS_CLEAR))
        .flatten()
        .filter(|&id| id != 0);
    own.or_else(|| lights.row_of(1).and_then(|row| lights.u32_at(row, lf::PARAMS_CLEAR)))
        .unwrap_or(0)
}
