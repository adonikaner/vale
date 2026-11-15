//! The building editor's panel in Areas mode: the building's groups and the
//! chosen group's `WMOAreaTable` row, under the editor's heading
//! (`super::wmoview::heading`). The state is
//! [`crate::tools::placements::wmoareas`].
//!
//! ```text
//! Area Keys  the two keys every row of the building shares
//! Groups     the whole building, then each group, with what its row says;
//!            a click chooses one, as a click in the viewport does, and Ctrl
//!            and a click chooses more, which the chosen group's row is
//!            given to together (`wmoareas`' *An area is made of groups*)
//! the row    the chosen group's row, drawn with the table workspace's form,
//!            with Remove Row; or, for a group with no row, what it takes
//!            from the building's row and Add Row
//! ```
//!
//! The row is drawn with the table form (`crate::ui::tables::form::fields`),
//! so its references have the same pickers and links as in the table
//! workspace. The form edits the table browser's open row, so choosing a
//! group opens its row in the browser, as picking a light opens the light's.
//! A reference followed from the form opens that row here, with a button
//! back to the group's row.

use bevy_egui::egui;

use crate::tables;
use crate::tools::placements::wmoareas::{self, Areas, Building, Chosen, TABLE, WHOLE_BUILDING};
use crate::ui::tables::form::fields;
use crate::ui::tables::modals::modals;
use crate::ui::tables::Workspace;
use crate::ui::widgets::theme::{self, property};

use vale_assets::tables::schema;

/// How tall the group list may grow before it scrolls, in points.
const LIST_HEIGHT: f32 = 220.0;

/// What a reader needs once about Areas mode, on the first heading's info
/// mark.
const ABOUT: &str = "A building's places are rows of WMOAreaTable. A row is found by the building's WMO id, \
                     the placement's name set and the group's id; the whole building's row (group -1) is \
                     used by every group with no row of its own. A row names the place, the zone it counts \
                     as, and its music and ambience. The client reads the name and sounds, and the server \
                     reads the zone.";

/// The panel. `areas` is the mode's state, taken out of the shell for the
/// time the workspace holds the rest.
pub(in crate::ui::tools) fn panel(ui: &mut egui::Ui, mut work: Workspace<'_>, areas: &mut Areas) {
    let Some(building) = areas.building.clone() else {
        theme::note(ui, "Drawing the building\u{2026}");
        modals(ui, &mut work, None);
        return;
    };
    if !work.session.open_table(work.assets, TABLE) {
        theme::note(ui, format!("Opening {TABLE}.dbc\u{2026}"));
        return;
    }
    // Names an area row by, and does nothing more when it is not open yet.
    work.session.open_table(work.assets, "AreaTable");

    property(ui, "Area Keys", ABOUT, |ui| {
        theme::value(ui, format!("WMO {} \u{b7} name set {}", building.wmo_id, building.name_set));
    });

    theme::heading_about(
        ui,
        &format!("Groups ({})", building.groups.len()),
        "The whole building, then each group in file order. A group in the brighter ink has a row \
         of its own; the others take the whole building's. Click a group here or inside the \
         building in the viewport.\n\nAn area is the groups whose rows say the same: to make one, \
         or to make one larger, choose groups with Ctrl and a click and give them the first group's \
         row. A group's shape is its geometry and is not edited here.",
    );
    if let Some((chosen, ctrl)) = group_list(ui, &work, &building, areas.chosen, &areas.also) {
        match (ctrl, chosen) {
            (true, Chosen::Group(at)) if matches!(areas.chosen, Some(Chosen::Group(_))) => wmoareas::toggle(areas, at),
            _ => {
                areas.chosen = Some(chosen);
                areas.also.clear();
                areas.listed = Some(chosen);
            }
        }
    }
    if !areas.also.is_empty() {
        several(ui, &mut work, &building, areas);
    }

    let Some(chosen) = areas.chosen else {
        modals(ui, &mut work, None);
        return;
    };
    let Some(key) = building.key_of(chosen) else { return };
    let title = match chosen {
        Chosen::Whole => "Whole Building".to_string(),
        Chosen::Group(at) => building.group_label(at),
    };
    let row = work
        .session
        .table(TABLE)
        .and_then(|table| wmoareas::row_of(table, building.wmo_id, building.name_set, key));
    // The row's form is a page of fields, longer than the inspector, so it
    // scrolls under the building and the list, which stay in view.
    egui::ScrollArea::vertical()
        .id_salt("wmo-area-row")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            match row {
                Some(record) => row_form(ui, &mut work, &building, &title, record),
                None => no_row(ui, &mut work, &building, &title, key),
            }
            ui.add_space(12.0);
        });
    modals(ui, &mut work, None);
}

/// The groups chosen with Ctrl beside the first: how many, and the two
/// things done to them together.
fn several(ui: &mut egui::Ui, work: &mut Workspace<'_>, building: &Building, areas: &mut Areas) {
    let Some(Chosen::Group(first)) = areas.chosen else { return };
    let from = building
        .key_of(Chosen::Group(first))
        .and_then(|key| work.session.table(TABLE).and_then(|table| wmoareas::row_of(table, building.wmo_id, building.name_set, key)));
    let label = building.group_label(first);
    theme::note(ui, format!("{} more groups chosen with {label}", areas.also.len()));
    ui.horizontal_wrapped(|ui| {
        let give = ui
            .add_enabled(from.is_some(), egui::Button::new(format!("Give Them {label}'s Row")))
            .on_hover_text(
                "Gives every other chosen group the first group's name, area and sounds: a row where it \
                 has none, and its own row changed where it has one. One undo entry.",
            )
            .on_disabled_hover_text("The first group has no row of its own. Add one, or choose a group that has one first.");
        if give.clicked() {
            if let Some(from) = from {
                let done = wmoareas::give_rows(work.session, building, from, &areas.also);
                work.session.status = format!("gave {done} groups {label}'s area");
            }
        }
        let take = ui.button("Remove Their Rows").on_hover_text(
            "Removes the rows of every chosen group, the first included, so each takes the whole \
             building's row. One undo entry.",
        );
        if take.clicked() {
            let groups: Vec<usize> = std::iter::once(first).chain(areas.also.iter().copied()).collect();
            let done = wmoareas::take_rows(work.session, building, &groups);
            work.browser.close_row();
            work.session.status = format!("removed the rows of {done} groups");
        }
        if ui.button("Clear").on_hover_text("Keeps the first group and forgets the others.").clicked() {
            areas.also.clear();
        }
    });
}

/// The whole building and each group, each with what its row says. Answers
/// the one clicked, and whether Ctrl was held. The groups chosen beside the
/// first are drawn chosen as well.
fn group_list(
    ui: &mut egui::Ui,
    work: &Workspace<'_>,
    building: &Building,
    chosen: Option<Chosen>,
    also: &[usize],
) -> Option<(Chosen, bool)> {
    let table = work.session.table(TABLE)?;
    let names = work.session.table("AreaTable");
    let says = |group: u32| {
        wmoareas::row_of(table, building.wmo_id, building.name_set, group)
            .map(|record| wmoareas::row_words(table, record, names))
    };
    let whole = says(WHOLE_BUILDING);
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("wmo-area-groups")
        .max_height(LIST_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let mut line = |ui: &mut egui::Ui, choice: Chosen, label: String, row: Option<String>| {
                let on = chosen == Some(choice) || matches!(choice, Chosen::Group(at) if also.contains(&at));
                let (says, ink) = match &row {
                    Some(words) if words.is_empty() => ("row, no name".to_string(), theme::INK),
                    Some(words) => (words.clone(), theme::INK),
                    None => ("\u{2014}".to_string(), theme::INK_DIM),
                };
                let text = egui::RichText::new(format!("{label}  \u{b7}  {says}")).color(ink);
                let response = ui.selectable_label(on, text);
                if response.clicked() {
                    clicked = Some((choice, ui.input(|i| i.modifiers.ctrl || i.modifiers.command)));
                }
            };
            line(ui, Chosen::Whole, "Whole building".to_string(), whole.clone());
            for (at, group) in building.groups.iter().enumerate() {
                line(ui, Chosen::Group(at), building.group_label(at), says(group.id));
            }
        });
    clicked
}

/// The chosen group's row: the table form, opened in the browser, and
/// Remove Row.
fn row_form(ui: &mut egui::Ui, work: &mut Workspace<'_>, building: &Building, title: &str, record: usize) {
    let on_the_row = work.browser.table == TABLE && work.browser.open == Some(record);
    theme::heading(ui, title);
    // The form edits the browser's open row. A row followed from it, such as
    // a ZoneMusic row, is drawn instead, with the way back.
    if !on_the_row && work.browser.table != TABLE {
        if let (Some(open), Some(schema)) = (work.browser.open, schema::for_table(&work.browser.table)) {
            let table = work.browser.table.clone();
            ui.horizontal(|ui| {
                if ui.button("Back to the Group's Row").clicked() {
                    follow(work, record);
                }
                theme::note(ui, format!("In {table}"));
            });
            fields(ui, work, open, Some(schema));
            return;
        }
    }
    if !on_the_row {
        follow(work, record);
    }
    let id = work.session.table(TABLE).and_then(|table| table.u32_at(record, 0)).unwrap_or(0);
    ui.horizontal(|ui| {
        theme::note(ui, format!("{TABLE}.dbc \u{b7} row {record} \u{b7} ID {id}"));
        if work.session.unsaved_tables.contains(TABLE) {
            ui.label(egui::RichText::new("Unsaved").size(theme::SMALL).color(theme::WARN));
        }
    });
    fields(ui, work, record, schema::for_table(TABLE));
    ui.add_space(6.0);
    let whole = building.key_of(Chosen::Whole) == key_of_record(work, record);
    let about = match whole {
        true => "Removes the whole building's row. Groups with no row of their own then take the area of \
                 the ground under the building, and no name or music.",
        false => "Removes this group's row. The group then takes the whole building's row.",
    };
    if ui.button("Remove Row").on_hover_text(about).clicked() && tables::rows::delete_row(work.session, TABLE, record) {
        work.browser.close_row();
        work.session.status = format!("removed {TABLE} row {id}");
    }
}

/// The group key a record holds.
fn key_of_record(work: &Workspace<'_>, record: usize) -> Option<u32> {
    work.session.table(TABLE).and_then(|table| table.u32_at(record, 3))
}

/// A choice with no row: what it takes instead, and Add Row.
fn no_row(ui: &mut egui::Ui, work: &mut Workspace<'_>, building: &Building, title: &str, key: u32) {
    theme::heading(ui, title);
    let fallback = work.session.table(TABLE).and_then(|table| {
        let record = wmoareas::row_of(table, building.wmo_id, building.name_set, WHOLE_BUILDING)?;
        Some(wmoareas::row_words(table, record, work.session.table("AreaTable")))
    });
    let note = match (key == WHOLE_BUILDING, fallback) {
        (true, _) => "The building has no row of its own. Groups with no row take the area of the ground \
                      under the building, and no name or music."
            .to_string(),
        (false, Some(words)) if !words.is_empty() => format!("No row of its own: the group takes the whole building's, {words}."),
        (false, Some(_)) => "No row of its own: the group takes the whole building's row.".to_string(),
        (false, None) => "No row of its own, and the building has none either: the group takes the area of \
                          the ground under the building."
            .to_string(),
    };
    theme::note(ui, note);
    let about = match key == WHOLE_BUILDING {
        true => "Adds the whole building's row, used by every group with no row of its own.",
        false => "Adds a row for this group, starting with the whole building's name, area and sounds, so \
                  the group sounds and reads as it does now until the row is changed.",
    };
    if theme::primary(ui, "Add Row").on_hover_text(about).clicked() {
        let Some(table) = work.session.table(TABLE) else { return };
        let (numbers, texts) = wmoareas::new_row_values(table, building, key);
        work.session.history.begin("Add building area");
        let made = tables::rows::push_row(work.session, TABLE, &numbers, &texts);
        work.session.history.end();
        match made {
            Some(record) => {
                follow(work, record);
                work.session.status = format!("added a {TABLE} row for {title}");
            }
            None => work.session.status = format!("{TABLE}: no row could be added"),
        }
    }
}

/// Open a row of the table in the browser, which the form edits.
fn follow(work: &mut Workspace<'_>, record: usize) {
    let Some(id) = work.session.table(TABLE).and_then(|table| table.u32_at(record, 0)) else { return };
    work.browser.index(work.session, TABLE);
    work.browser.follow(work.session, TABLE, id);
}
