//! The graveyard tool's panel: the selected safe place, its name and position,
//! the way a spirit faces there, and the zones it serves.
//!
//! The place writes through `crate::tools::tables`; the facing and the links
//! are rows of the project's store, written through `crate::tools::graveyards`
//! and drawn only once the two server tables have been read. See
//! `crate::tools::graveyards` for the pointer.

use bevy_egui::egui;

use super::theme;
use crate::session::EditSession;
use crate::tools::graveyards::{self, Armed, Graveyards};
use crate::tools::tables;
use vale_assets::tables::safeloc::fields as sf;
use vale_edit::dbc::places::{self, SafeLoc};
use vale_mangos::graveyard::FACTIONS;
use vale_mangos::row::Life;

/// What the panel is drawn from.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub graveyards: &'a mut Graveyards,
    pub assets: &'a vale_client::assets::GameAssets,
    pub now: f64,
    /// The top bar's Server… popover, which holds Apply and Put back.
    pub server_panel: &'a mut super::popover::Popover,
}

pub fn draw(ui: &mut egui::Ui, subject: Subject<'_>) {
    let Subject {
        session,
        graveyards,
        assets,
        now,
        server_panel,
    } = subject;
    for table in [places::SAFE_LOCS, tables::area::TABLE] {
        if !session.open_table(assets, table) {
            theme::note(ui, format!("opening {table}.dbc\u{2026}"));
            return;
        }
    }

    theme::note(ui, format!("{} safe places on this map", graveyards.list.len()));
    ui.add_space(4.0);
    controls(ui, graveyards);

    egui::ScrollArea::vertical()
        .id_salt("graveyards")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            match graveyards.selected.and_then(|id| graveyards.place(id).cloned()) {
                Some(place) => {
                    ui.add_space(4.0);
                    theme::heading(ui, &format!("Safe place {}", place.id));
                    place_block(ui, session, graveyards, &place, now);
                    ui.add_space(6.0);
                    theme::heading(ui, "On the server");
                    server_half(ui, session, graveyards, &place, now);
                }
                None => theme::note(
                    ui,
                    "Click a safe place in the world. Grey ones are linked to no zone, so \
                     the server never sends a spirit there.",
                ),
            }
            ui.add_space(6.0);
            theme::heading(ui, "Server");
            if ui
                .button("Server\u{2026}")
                .on_hover_text(
                    "Opens the Server panel. Client tables copies WorldSafeLocs.dbc into \
                     DataDir\\5875\\dbc; Graveyards writes the links and \
                     facings.",
                )
                .clicked()
            {
                server_panel.show();
            }
            theme::note(
                ui,
                "The server reads WorldSafeLocs.dbc and world_safe_locs_facing at startup \
                 only. Links are live on `.reload game_graveyard_zone`.",
            );
            ui.add_space(12.0);
        });
}

/// What a click on empty ground does.
fn controls(ui: &mut egui::Ui, graveyards: &mut Graveyards) {
    ui.horizontal_wrapped(|ui| {
        let armed = graveyards.armed == Armed::NewPlace;
        if ui
            .selectable_label(armed, "New place")
            .on_hover_text("Armed, a click on the ground makes a safe place there.")
            .clicked()
        {
            graveyards.armed = match armed {
                true => Armed::Nothing,
                false => Armed::NewPlace,
            };
        }
        let linking = graveyards.armed == Armed::LinkZone;
        let link = ui.add_enabled(
            graveyards.selected.is_some() && graveyards.held.is_some(),
            egui::Button::selectable(linking, "Link zone"),
        );
        if link
            .on_hover_text(
                "Armed, a click on the ground links the selected place to the zone the \
                 click lands in, for both sides.",
            )
            .on_disabled_hover_text("Select a place, with the server's rows read.")
            .clicked()
        {
            graveyards.armed = match linking {
                true => Armed::Nothing,
                false => Armed::LinkZone,
            };
        }
    });
    if graveyards.armed == Armed::LinkZone {
        let under = match graveyards.pointed_zone {
            Some(zone) => format!("Under the pointer: {} ({zone}).", graveyards.zone_name(zone)),
            None => "The pointer is over no open ground.".to_string(),
        };
        theme::note(ui, under);
    }
    if graveyards.armed != Armed::Nothing {
        theme::note(ui, "Escape disarms.");
    }
}

/// The place's name and position.
fn place_block(ui: &mut egui::Ui, session: &mut EditSession, graveyards: &mut Graveyards, place: &SafeLoc, now: f64) {
    let mut name = place.name.clone();
    theme::row(ui, "name", |ui| {
        if ui
            .text_edit_singleline(&mut name)
            .on_hover_text("The name the client's files carry. The game does not show it.")
            .changed()
        {
            tables::set_text(session, places::SAFE_LOCS, place.record, sf::NAME, &name, "Rename graveyard", now);
            graveyards.stale();
        }
    });
    let mut at = place.at;
    let mut moved = false;
    for (axis, label) in ["x", "y", "z"].iter().enumerate() {
        theme::row(ui, label, |ui| {
            moved |= ui.add(egui::DragValue::new(&mut at[axis]).speed(0.25).suffix(" yd")).changed();
        });
    }
    if moved {
        graveyards::move_place(session, place, bevy::math::Vec3::from(at), now);
        graveyards.stale();
    }
    ui.horizontal(|ui| {
        if ui.button("Fly to").clicked() {
            graveyards.fly_to(place.at);
        }
        if ui
            .button("Drop to the ground")
            .on_hover_text("Set z to the ground under the place, where the ground is open.")
            .clicked()
        {
            if let Some(ground) = crate::tools::doodads::ground_height(session, at[0], at[1]) {
                graveyards::move_place(session, place, bevy::math::Vec3::new(at[0], at[1], ground), now);
                graveyards.stale();
            }
        }
        let shipped = graveyards.is_shipped(place.id);
        let remove = ui.button(match shipped {
            true => "Remove\u{2026}",
            false => "Remove",
        });
        if remove
            .on_hover_text(
                "Removes the row from WorldSafeLocs.dbc, and the links and the facing that \
                 name it from the project's server rows.",
            )
            .clicked()
        {
            match shipped {
                true => graveyards.confirm_remove = Some(place.id),
                false => {
                    let line = graveyards::remove_place(session, graveyards, place.id, now);
                    session.status = line;
                }
            }
        }
    });
    if graveyards.confirm_remove == Some(place.id) {
        ui.label(
            egui::RichText::new(
                "This is one of the game's own safe places. Every zone it serves sends its \
                 spirits to the next nearest place instead, or nowhere.",
            )
            .size(theme::SMALL)
            .color(theme::WARN),
        );
        ui.horizontal(|ui| {
            if ui.button("Remove it").clicked() {
                let line = graveyards::remove_place(session, graveyards, place.id, now);
                session.status = line;
            }
            if ui.button("Keep it").clicked() {
                graveyards.confirm_remove = None;
            }
        });
    }
}

/// The facing and the links.
fn server_half(ui: &mut egui::Ui, session: &mut EditSession, graveyards: &mut Graveyards, place: &SafeLoc, now: f64) {
    if let Some(trouble) = &graveyards.trouble {
        theme::note(ui, format!("The server's rows could not be read: {trouble}"));
        return;
    }
    if graveyards.held.is_none() {
        theme::note(ui, "reading game_graveyard_zone and world_safe_locs_facing\u{2026}");
        return;
    }
    let id = place.id;
    let edits = session.server_edits.clone();
    let facing = graveyards.facing(&edits, id);
    let mut degrees = facing.map_or(0.0, |(radians, _)| radians.to_degrees());
    theme::row(ui, "facing", |ui| {
        if ui
            .add(egui::DragValue::new(&mut degrees).speed(1.0).suffix("\u{b0}"))
            .on_hover_text(
                "world_safe_locs_facing: the way a spirit faces on appearing. The row holds \
                 it in radians. Read at startup only.",
            )
            .changed()
        {
            graveyards::set_facing(session, graveyards, id, degrees.rem_euclid(360.0).to_radians(), now);
        }
    });
    if facing.is_none() {
        theme::note(ui, "No facing row: a spirit faces north.");
    }

    ui.add_space(4.0);
    let links = graveyards.links_of(&edits, id);
    let serving = links.iter().filter(|shown| shown.life != Life::Delete).count();
    theme::heading(ui, &format!("Serves ({serving})"));
    if links.is_empty() {
        theme::note(ui, "No zone: the server never sends a spirit here. Link zone adds one.");
    }
    for shown in &links {
        let removed = shown.life == Life::Delete;
        ui.horizontal(|ui| {
            let name = format!("{} ({})", graveyards.zone_name(shown.link.zone), shown.link.zone);
            let text = egui::RichText::new(name).color(match removed {
                true => theme::INK_FAINT,
                false => theme::INK,
            });
            ui.label(match removed {
                true => text.strikethrough(),
                false => text,
            });
            if removed {
                if ui.small_button("Keep").clicked() {
                    let key = shown.link.key();
                    let label = format!("{} {}", vale_mangos::graveyard::ZONE, key.text());
                    let gesture = crate::session::Gesture { label: "Keep graveyard link", subject: &label, now };
                    session.set_server_row(vale_mangos::graveyard::ZONE, &key, None, Some(gesture));
                }
                return;
            }
            let mut faction = shown.link.faction;
            let named = |value: u32| {
                FACTIONS
                    .iter()
                    .find(|known| known.value == value)
                    .map_or_else(|| value.to_string(), |known| known.name.to_string())
            };
            egui::ComboBox::from_id_salt(("graveyard link side", id, shown.link.zone, shown.link.patch_max))
                .selected_text(named(faction))
                .show_ui(ui, |ui| {
                    for known in FACTIONS {
                        ui.selectable_value(&mut faction, known.value, known.name);
                    }
                });
            if faction != shown.link.faction {
                graveyards::set_faction(session, shown, faction, now);
            }
            if ui.small_button("Unlink").clicked() {
                graveyards::remove_link(session, shown, now);
            }
        });
        for fault in shown.link.check() {
            ui.label(egui::RichText::new(fault).size(theme::SMALL).color(theme::WARN));
        }
    }
}
