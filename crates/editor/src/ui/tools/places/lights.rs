//! The light tool's panel: the selected light's form, its sphere and place,
//! and the buttons that make a light or its parameters. A light is a row of
//! `Light.dbc` picked in the viewport, so its form is drawn with the table
//! workspace's fields. The state is [`crate::tools::places::lights`]; a light's
//! bands are [`super::bands`].

use bevy_egui::egui;

use crate::tables::Modal;
use crate::tables;
use crate::ui::tables::form::{fields, section_block};
use crate::ui::tables::modals::modals;
use crate::ui::tables::{LABEL, VALUE, Workspace};
use crate::ui::widgets::theme;

use vale_assets::tables::schema;

/// A light's whole form, drawn in the inspector: the light picked in the world
/// and the numbers behind it.
///
/// This is the lights tool's only panel. A light is chosen by picking it in
/// the viewport rather than from a list, and the shell's rule is that the
/// inspector shows the selection, so the form is drawn here. The list of all
/// 374 lights is a dialog behind Browse…, for finding one that is not on
/// screen.
///
/// Its contents, in the order they are used: which light this is, a button to
/// fly the camera to it, its position and radius in yards rather than the
/// file's 1/36 of a yard, and then the five `LightParams` rows it uses under
/// the five conditions, which hold the colours.
pub fn light_inspector(ui: &mut egui::Ui, mut work: Workspace<'_>) {
    if !work.session.open_table(work.assets, "Light") {
        theme::note(ui, "opening Light.dbc…");
        return;
    }
    browse_button(ui, &mut work);
    new_light_buttons(ui, &mut work);
    ui.add_space(6.0);

    // Draw whatever table the browser has open, which is not always `Light`.
    // Pressing a `ParamsClear` link opens `LightParams`, and the bands belong
    // to that row. If this panel always drew the light's own form, those five
    // links would have no visible effect.
    //
    // The browser is not re-pointed here. Re-pointing it at the selected light
    // on every frame undid a followed reference on the frame after the press.
    // The two places that change the selection, the pick in the viewport and
    // the browse dialog, each point the browser at the row themselves, which
    // is the only time it should move. `tables::open::open_tables` has a note on the
    // same rule for the rail's table.
    let table_name = work.browser.table.clone();
    let on_the_light = table_name == "Light";
    let Some(record) = work.browser.open else {
        theme::note(
            ui,
            "Click a light in the world to edit it. Every light near the camera \
             is marked; the selected light has its falloff spheres drawn in the \
             world and its fields listed here.",
        );
        modals(ui, &mut work, None);
        return;
    };

    // A `< back` button to return from a followed reference. The middle
    // workspace has one in its heading; without this one the inspector had no
    // way back after following a reference to another table.
    if !work.browser.back.is_empty() {
        ui.horizontal(|ui| {
            if ui.button("< back").clicked() {
                work.browser.go_back();
            }
            if !on_the_light {
                ui.label(
                    egui::RichText::new(format!("in {table_name}"))
                        .size(12.0)
                        .color(theme::INK_FAINT),
                );
            }
        });
        ui.add_space(4.0);
    }

    let label = work.browser.describe(work.session, &table_name, record);
    let title = match label.title.is_empty() {
        true => format!("{table_name} row"),
        false => label.title.clone(),
    };
    theme::heading(ui, &title);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{table_name}.dbc  row {record}  ·  id {}",
                label.id
            ))
            .size(12.0)
            .color(theme::INK_FAINT),
        );
        if work.session.unsaved_tables.contains(&table_name) {
            ui.label(egui::RichText::new("unsaved").color(theme::WARN));
        }
    });
    ui.add_space(4.0);
    if on_the_light {
        place_strip(ui, &mut work, record);
        ui.add_space(6.0);
    }

    let schema = schema::for_table(&table_name);
    egui::ScrollArea::vertical()
        .id_salt("light-form")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            match (on_the_light, schema) {
                // The light's own form: the sphere in the world's units, then
                // the five `LightParams` rows it uses under the five
                // conditions, which hold the colours. The conditions are drawn
                // by the ordinary `section_block`, so the picker, the resolved
                // name and the link behave as they do elsewhere.
                (true, Some(schema)) => {
                    sphere_block(ui, &mut work, record);
                    if let Some(section) = schema
                        .sections
                        .iter()
                        .find(|section| section.name == "Conditions")
                    {
                        section_block(ui, &mut work, record, schema, section);
                    }
                }
                // A row reached by following a reference from the light, such
                // as a `LightParams` or `LightSkybox` row, gets the ordinary
                // form for its table.
                _ => {
                    fields(ui, &mut work, record, schema);
                    // A `LightParams` row also shows its bands, which hold
                    // every colour the world is drawn in. No column points at
                    // the two tables that hold the bands, so this is the only
                    // way to reach them.
                    if table_name == "LightParams" {
                        if let Some(id) = work
                            .session
                            .table("LightParams")
                            .and_then(|table| table.u32_at(record, 0))
                        {
                            let hour = work.hour;
                            crate::ui::tools::places::bands::blocks(ui, &mut work, id, hour);
                        }
                    }
                }
            }
            ui.add_space(12.0);
        });
    modals(ui, &mut work, None);
}

/// Make a light: one placed by a click on the ground, or the map's default
/// light when the map has none. Either copies its weather from the chosen
/// light or the map's default; see `crate::tools::places::lights::new_light`.
fn new_light_buttons(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let Some(lights) = work.lights.as_deref_mut() else {
        return;
    };
    ui.horizontal(|ui| {
        if ui
            .selectable_label(lights.armed, "New light")
            .on_hover_text(
                "While on, a click on the ground places a light there: full strength to \
                 50 yards, faded out at 120 yards, with the LightParams of the selected \
                 light or the map's default light. Escape or a second click turns it off.",
            )
            .clicked()
        {
            lights.armed = !lights.armed;
        }
        if lights.default_light().is_none() {
            let map = work.session.map_id;
            if ui
                .button("New default light")
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
    });
    if lights.armed {
        theme::note(ui, "Click the ground where the light should stand. Escape cancels.");
    }
}

/// A button that opens the list of all lights as a dialog, for finding a light
/// that is not on screen.
///
/// The tool is built around picking a light in the viewport, which cannot
/// reach a light that is off screen. The list is a dialog behind a button
/// rather than a permanent panel, because a permanent panel took space the
/// form needed.
fn browse_button(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let count = work
        .session
        .table("Light")
        .map(|table| table.record_count())
        .unwrap_or(0);
    if ui
        .add_sized(
            egui::vec2(ui.available_width(), 24.0),
            egui::Button::new(format!("Browse all {count} lights…")),
        )
        .on_hover_text("Choose a light from a list, including lights not on screen. The camera moves to the chosen light.")
        .clicked()
    {
        work.browser.modal = Some(Modal::Rows { table: "Light" });
        work.browser.pick_query.clear();
        work.browser.pick_page = 0;
        work.browser.pick_focus = true;
    }
}

/// A light's position and falloff in yards: the `Sphere` section, drawn in the
/// world's units instead of the file's.
///
/// The unit changes and the radius names do not: `FalloffStart` and
/// `FalloffEnd` keep the columns' names. The three coordinates are labelled as
/// the world's `x`, `y` and `z`, because after conversion they no longer match
/// the columns they come from. The axes swap, so the field holding the world's
/// x is `InternalZ` in the file. Each field's tooltip names the column it
/// writes and its unit.
///
/// The five columns are
/// [`light_field`](vale_assets::tables::light::light_field)'s: three
/// coordinates in the internal representation used by every placement in the
/// game, scaled by 36, and two radii in the same unit. All five are converted
/// here rather than shown raw, and typing into one converts back.
/// [`vale_assets::world::adt::placement_from_world`] does the conversion back;
/// loading the game's own tiles never uses that direction, so it is called
/// here rather than reimplemented.
///
/// The raw columns are still available, folded away underneath, because they
/// are what is written to the file and a person checking a diff needs them.
pub(in crate::ui) fn sphere_block(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    use vale_assets::tables::light::{light_field as lf, YARDS_PER_UNIT};
    use vale_assets::world::adt::{placement_from_world, placement_to_world};

    let Some(table) = work.session.table("Light") else {
        return;
    };
    let raw = |field: usize| table.f32_at(record, field).unwrap_or(0.0);
    let at = placement_to_world([
        raw(lf::INTERNAL_X) * YARDS_PER_UNIT,
        raw(lf::INTERNAL_Y) * YARDS_PER_UNIT,
        raw(lf::INTERNAL_Z) * YARDS_PER_UNIT,
    ]);
    let (mut x, mut y, mut z) = (at[0], at[1], at[2]);
    let mut start = raw(lf::FALLOFF_START) * YARDS_PER_UNIT;
    let mut end = raw(lf::FALLOFF_END) * YARDS_PER_UNIT;
    let (was_x, was_y, was_z, was_start, was_end) = (x, y, z, start, end);

    egui::CollapsingHeader::new(egui::RichText::new("Sphere").size(14.0).strong())
        .default_open(true)
        .show(ui, |ui| {
            let row = |ui: &mut egui::Ui, name: &str, value: &mut f32, tip: &str| {
                ui.horizontal(|ui| {
                    let label_width = LABEL.min(ui.available_width() * 0.45);
                    ui.add_sized(
                        egui::vec2(label_width, 22.0),
                        egui::Label::new(egui::RichText::new(name).color(theme::INK_DIM))
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
                "x",
                &mut x,
                "The world's x. Stored as InternalZ, which is northward from \
                 the corner of the map in 1/36 of a yard.",
            );
            row(
                ui,
                "y",
                &mut y,
                "The world's y. Stored as InternalX, which is westward from \
                 the corner of the map in 1/36 of a yard.",
            );
            row(
                ui,
                "z",
                &mut z,
                "The world's z. Stored as InternalY, in 1/36 of a yard. A \
                 drag in the world moves x and y only, so z changes only \
                 here.",
            );
            ui.add_space(4.0);
            row(
                ui,
                "FalloffStart",
                &mut start,
                "The radius within which the light applies at full strength. \
                 Drawn as the inner sphere in the world, where it can be \
                 dragged. Stored in 1/36 of a yard.",
            );
            row(
                ui,
                "FalloffEnd",
                &mut end,
                "The radius at which the light has faded out completely. Zero makes this \
                 the map's default light, which applies everywhere and whose \
                 position is not read. Stored in 1/36 of a yard.",
            );
        });

    // Write only when a value changed, and write all five under one gesture, so
    // a drag on the x box is one entry on the undo stack rather than one per
    // pixel.
    if (x, y, z, start, end) == (was_x, was_y, was_z, was_start, was_end) {
        return;
    }
    let placement = placement_from_world([x, y, z]);
    let store = |yards: f32| (yards / YARDS_PER_UNIT).to_bits();
    tables::fields::set_fields(
        work.session,
        "Light",
        record,
        &[
            (lf::INTERNAL_X, store(placement[0])),
            (lf::INTERNAL_Y, store(placement[1])),
            (lf::INTERNAL_Z, store(placement[2])),
            (lf::FALLOFF_START, store(start)),
            (lf::FALLOFF_END, store(end)),
        ],
        "Edit light sphere",
        "Light sphere",
        work.now,
    );
}

/// The strip under a `Light` row's heading: where the light is, and a button
/// to fly the camera there.
///
/// The row's three coordinates are stored in a form that cannot be read at a
/// glance (`612096, 0, 998400`, in 1/36 of a yard from the corner of the map),
/// so the form alone does not show where the light is. This strip gives the
/// position in world coordinates, keeps the viewport's ring on the open row,
/// and flies the camera there.
///
/// It is drawn only for `Light`. The other four tables of the chain have no
/// position: a `LightParams` row is a look, and where it applies is whichever
/// lights point at it.
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

    ui.horizontal(|ui| {
        if mark.everywhere {
            // The default light applies everywhere and its coordinates are not
            // read, so there is no position to fly to.
            ui.label(
                egui::RichText::new("default light (whole map)")
                    .size(13.0)
                    .color(theme::INK_DIM),
            );
        } else {
            if ui
                .button("Fly to")
                .on_hover_text("Move the camera to this light's position, at the row's stored height.")
                .clicked()
            {
                work.fly.write(crate::tools::places::lights::Lights::fly_to(&mark));
            }
            ui.label(
                theme::number(format!(
                    "{:.0}, {:.0}, {:.0}",
                    mark.at.x, mark.at.y, mark.at.z
                ))
                .color(theme::INK_DIM),
            );
            ui.label(
                egui::RichText::new(format!("{:.0}..{:.0} yd", mark.start, mark.end))
                    .size(13.0)
                    .color(theme::INK_DIM),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut lights.show_falloff, "falloff")
                .on_hover_text(
                    "On: draw this light's FalloffStart and FalloffEnd spheres. \
                     Off: draw only its marker. Every light near the camera is \
                     marked either way.",
                );
        });
    });
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
        Some(id) => format!("created LightParams {id} with its bands, copied from LightParams {from}"),
        None => "could not create a LightParams row".to_string(),
    };
    made
}

/// What a new `LightParams` row on a light's form copies: the light's own
/// clear `LightParams`, or, when that is empty too, the Eastern Kingdoms
/// default light's (`Light` row 1), which every map without a light of its own
/// is lit by.
pub(in crate::ui) fn light_params_to_copy(work: &Workspace<'_>, table_name: &str, record: usize) -> u32 {
    use vale_assets::tables::light::light_field;
    let Some(lights) = work.session.table("Light") else {
        return 0;
    };
    let own = (table_name == "Light")
        .then(|| lights.u32_at(record, light_field::PARAMS_CLEAR))
        .flatten()
        .filter(|&id| id != 0);
    own.or_else(|| lights.row_of(1).and_then(|row| lights.u32_at(row, light_field::PARAMS_CLEAR)))
        .unwrap_or(0)
}

/// The light tool's inspector panel: a light's whole form, because a light is
/// picked in the viewport and the inspector is where the shell shows the
/// selection, as it does a doodad's position and turn. The list of all 374
/// lights is a dialog behind a button, for finding one that is not in view.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { tool, session, assets, now, .. } = subject;
    light_inspector(ui, Workspace::new(tool, session, editing, assets, now, true, false));
}
