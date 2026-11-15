//! The building editor's panel in Doodads mode, under the editor's heading
//! (`super::wmoview::heading`). The state is
//! [`crate::tools::placements::wmodoodads`].
//!
//! ```text
//! (notice)      before the first edit of a file the game ships: Copy for
//!               This Placement or Edit the Shared File
//! Doodad Sets   every set with its count; the chosen one is drawn and an
//!               added doodad goes into it; a name field with Rename and
//!               New Set
//! Doodad        the chosen doodad: its model, set, place, turn and scale,
//!               Duplicate and Remove
//! Add           the model list the Place mode uses; a chosen model is added
//!               where the next click lands
//! ```

use bevy::prelude::*;
use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::placements::place::{Kind, Placing};
use crate::tools::placements::wmodoodads::{self, Placed, Spawns};
use crate::tools::placements::wmoview::WmoView;
use crate::ui::widgets::theme::{self, property};
use vale_client::assets::GameAssets;

use super::place::{browse, number_row};

/// The panel.
#[allow(clippy::too_many_arguments)]
pub(in crate::ui::tools) fn panel(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    assets: &GameAssets,
    spawns: &mut Spawns,
    view: &mut WmoView,
    placing: &mut Placing,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
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
    shared(ui, session, assets, spawns, view, &placed);
    let editable = spawns.editable(session);
    egui::ScrollArea::vertical()
        .id_salt("wmo-doodads")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            sets(ui, session, assets, view, &placed, editable, now);
            ui.add_enabled_ui(editable, |ui| {
                chosen(ui, session, assets, spawns, &placed, now);
                add(ui, spawns, placing, portraits, favourites);
            });
            ui.add_space(12.0);
        });
}

/// Before the first edit of a file the game ships: the choice between a copy
/// for this placement and the shared file. See the state's module comment.
pub(super) fn shared(ui: &mut egui::Ui, session: &mut EditSession, assets: &GameAssets, spawns: &mut Spawns, view: &mut WmoView, placed: &Placed) {
    if spawns.editable(session) {
        return;
    }
    let Some((from, tile)) = view.open.as_ref().map(|open| (open.from, open.tile)) else { return };
    theme::note(
        ui,
        format!(
            "{} is the game's file. Every placement of it, on every map, draws an edit to it.",
            placed.leaf()
        ),
    );
    ui.horizontal_wrapped(|ui| {
        let copy = theme::primary(ui, "Copy for This Placement").on_hover_text(
            "Copies the building's root and group files into the project under a new name, with every \
             edit made to them so far, and points the placement the editor was opened from at the \
             copy. Other placements keep the game's file.",
        );
        if copy.clicked() {
            session.status = match wmodoodads::copy_for_placement(session, assets, &placed.path, from, tile) {
                Ok(path) => {
                    if let Some(open) = view.open.as_mut() {
                        open.path = path.clone();
                    }
                    spawns.chosen = None;
                    format!("copied {} to {path} for this placement", placed.leaf())
                }
                Err(why) => why,
            };
        }
        let shared = ui.button("Edit the Shared File").on_hover_text(
            "Edits the game's file in the project. Every placement of it draws the edit once the project \
             is saved and published.",
        );
        if shared.clicked() {
            spawns.shared.insert(crate::session::buildings::building_key(&placed.path));
        }
    });
}

/// Every set with its count, the chosen one being drawn and where an added
/// doodad goes, and the name field.
fn sets(ui: &mut egui::Ui, session: &mut EditSession, assets: &GameAssets, view: &mut WmoView, placed: &Placed, editable: bool, now: f64) {
    let Some(open) = session.building(&placed.path) else { return };
    let sets = open.building.root.doodad_sets();
    theme::heading_about(
        ui,
        &format!("Doodad Sets ({})", sets.len()),
        "The set chosen here is drawn, and a doodad added goes into it. A placement draws the one set \
         its record names (Select mode's Doodad Set).",
    );
    let Some(viewing) = view.open.as_mut() else { return };
    for (at, set) in sets.iter().enumerate() {
        let label = format!("{at} \u{b7} {} \u{b7} {}", set.name, counted(set.count as usize));
        if ui.selectable_label(usize::from(viewing.set) == at, label).clicked() {
            viewing.set = at as u16;
        }
    }
    let chosen = usize::from(viewing.set);
    ui.add_enabled_ui(editable, |ui| {
        let id = ui.id().with("set-name");
        let mut name: String = ui.data_mut(|data| data.get_temp(id)).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut name)
                    .hint_text("Set name")
                    .char_limit(vale_edit::wmo::SET_NAME)
                    .desired_width(150.0),
            );
            let typed = name.trim().to_string();
            let rename = ui
                .add_enabled(!typed.is_empty(), egui::Button::new("Rename"))
                .on_hover_text("Gives the chosen set this name. At most 19 characters.");
            let new = ui
                .add_enabled(!typed.is_empty(), egui::Button::new("New Set"))
                .on_hover_text("Adds an empty set with this name after the last, and draws it.");
            let asked = match (rename.clicked(), new.clicked()) {
                (true, _) => Some(Some(chosen)),
                (_, true) => Some(None),
                _ => None,
            };
            if let Some(rename) = asked {
                match wmodoodads::name_set(session, assets, &placed.path, rename, &typed, now) {
                    Ok(()) => {
                        if rename.is_none() {
                            viewing.set = sets.len() as u16;
                        }
                        name.clear();
                    }
                    Err(why) => session.status = why,
                }
            }
        });
        ui.data_mut(|data| data.insert_temp(id, name));
    });
}

/// The chosen doodad's model, set, place, turn and scale, with Duplicate and
/// Remove.
fn chosen(ui: &mut egui::Ui, session: &mut EditSession, assets: &GameAssets, spawns: &mut Spawns, placed: &Placed, now: f64) {
    let Some(index) = spawns.chosen else {
        theme::heading(ui, "Doodad");
        theme::note(ui, "Click one of the building's doodads to choose it.");
        return;
    };
    let Some(open) = session.building(&placed.path) else { return };
    let (Some(doodad), sets) = (open.building.root.doodads().get(index as usize).cloned(), open.building.root.doodad_sets()) else {
        spawns.chosen = None;
        return;
    };
    let set = sets.iter().position(|run| u32::from(index) >= run.start && u32::from(index) < run.start + run.count).unwrap_or(0);
    theme::heading_about(
        ui,
        "Doodad",
        "Drag it, or use the handles; the Keys button lists the rest. The numbers are in the building's own axes.",
    );
    property(ui, "Model", "", |ui| {
        let leaf = doodad.name.rsplit(['\\', '/']).next().unwrap_or(&doodad.name).to_string();
        theme::value(ui, leaf).on_hover_text(doodad.name.clone());
    });
    property(ui, "Set", "", |ui| {
        theme::value(ui, format!("{set} \u{b7} {}", sets.get(set).map_or("", |run| run.name.as_str())));
    });
    // The building's own axes, which are the file's: the editor draws it at
    // its file's origin with no turn.
    let mut moved = Vec3::from(doodad.position);
    let was = moved;
    for (name, axis) in [("X", 0), ("Y", 1), ("Z", 2)] {
        number_row(ui, name, "In the building's own axes, as the file stores it.", egui::DragValue::new(&mut moved[axis]).speed(0.1).fixed_decimals(2));
    }
    let quat = Quat::from_array(doodad.rotation);
    let mut degrees = vale_assets::world::adt::placement_euler_to_world(wmodoodads::angles_of(doodad.rotation))[2];
    let was_degrees = degrees;
    number_row(
        ui,
        "Turn",
        "About the building's up, in degrees.",
        egui::DragValue::new(&mut degrees).speed(1.0).fixed_decimals(1).suffix("\u{b0}"),
    );
    let mut scale = doodad.scale;
    number_row(ui, "Scale", "", egui::DragValue::new(&mut scale).speed(0.01).range(0.01..=50.0).fixed_decimals(2));
    let path = placed.path.clone();
    let outcome = if moved != was {
        let local = moved.to_array();
        Some(wmodoodads::change_spawn(session, assets, &path, index, "Move building doodad", now, |doodad| doodad.position = local))
    } else if degrees != was_degrees {
        let turned = Quat::from_rotation_z((degrees - was_degrees).to_radians()) * quat;
        Some(wmodoodads::change_spawn(session, assets, &path, index, "Turn building doodad", now, |doodad| {
            doodad.rotation = turned.to_array()
        }))
    } else if scale != doodad.scale {
        Some(wmodoodads::change_spawn(session, assets, &path, index, "Scale building doodad", now, |doodad| doodad.scale = scale))
    } else {
        None
    };
    if let Some(Err(why)) = outcome {
        session.status = why;
    }
    ui.horizontal(|ui| {
        if ui.button("Duplicate").on_hover_text("Ctrl+D. Puts a copy on the cursor at the same turn and scale. Click to add it.").clicked() {
            spawns.adding = Some(vale_assets::world::m2::model_path(&doodad.name));
            spawns.turn = degrees.rem_euclid(360.0);
            spawns.scale = doodad.scale;
        }
        if ui.button("Remove").on_hover_text("Delete. Takes the doodad out of the building and out of every group's list.").clicked() {
            session.status = match wmodoodads::remove_spawn(session, assets, &path, index, now) {
                Ok(()) => "removed the doodad".to_string(),
                Err(why) => why,
            };
            spawns.chosen = None;
        }
    });
}

/// The model list; a model chosen is added where the next click lands.
fn add(
    ui: &mut egui::Ui,
    spawns: &mut Spawns,
    placing: &mut Placing,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
) {
    theme::heading_about(
        ui,
        "Add",
        "Choose a model and it follows the pointer; click inside the building where it goes. It goes \
         into the drawn set and into the list of the group whose box holds it, and stays on the cursor \
         for the next. , and . turn it, Ctrl and the wheel scale it, Escape puts it down.",
    );
    match spawns.adding.clone() {
        Some(path) => {
            ui.horizontal(|ui| {
                let leaf = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
                theme::value(ui, format!("{leaf} \u{b7} {:.0}\u{b0} \u{b7} x{:.2}", spawns.turn, spawns.scale)).on_hover_text(path.clone());
                if ui.small_button("Put Down").clicked() {
                    spawns.adding = None;
                }
            });
        }
        None => theme::note(ui, "No model on the cursor"),
    }
    ui.add(egui::TextEdit::singleline(&mut spawns.search).hint_text("Search the models").desired_width(f32::INFINITY));
    let chosen: Vec<String> = spawns.adding.iter().cloned().collect();
    let search = spawns.search.clone();
    if let Some(picked) = browse(ui, placing, Kind::Doodad, &search, &chosen, portraits, favourites) {
        spawns.adding = Some(picked);
    }
}

/// `1 doodad`, `12 doodads`.
fn counted(count: usize) -> String {
    match count {
        1 => "1 doodad".to_string(),
        n => format!("{n} doodads"),
    }
}
