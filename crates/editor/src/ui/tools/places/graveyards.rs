//! The graveyard tool's panel: the selected safe place, its name and position,
//! the way a spirit faces there, and the zones it serves.
//!
//! The place writes through `crate::tables`; the facing and the links
//! are rows of the project's store, written through `crate::tools::places::graveyards`
//! and drawn only once the two server tables have been read. See
//! `crate::tools::places::graveyards` for the pointer.
//!
//! ```text
//! above the scroll   the count of graveyards, and New Graveyard and Link
//!                    Zone, which arm the pointer
//! Graveyard <id>     the name
//! Location           X, Y and Z, then Fly To, Snap to Ground and Remove
//! Server Rows        the facing
//! Linked Zones       one row per link: the zone, the faction and Unlink
//! Server             the button that opens the Server panel
//! ```
//!
//! With no graveyard selected, a Selection section takes the place of the
//! four in the middle.

use bevy_egui::egui;

use super::triggers::arm_button;
use crate::ui::widgets::theme::{self, property};
use crate::session::EditSession;
use crate::tools::places::graveyards::{self, Armed, Graveyards};
use crate::tables;
use vale_assets::tables::safeloc::fields as sf;
use vale_edit::dbc::places::{self, SafeLoc};
use vale_mangos::graveyard::FACTIONS;
use vale_mangos::row::Life;

/// What the panel is drawn from.
pub(crate) struct Subject<'a> {
    pub(crate) session: &'a mut EditSession,
    pub(crate) graveyards: &'a mut Graveyards,
    pub(crate) assets: &'a vale_client::assets::GameAssets,
    pub(crate) now: f64,
    /// The top bar's Server… popover, which holds Apply and Put back.
    pub(crate) server_panel: &'a mut crate::ui::shell::popover::Popover,
    /// Where the Fly To button sends its request. See
    /// [`crate::camera::FlyTo`].
    pub(crate) fly: &'a mut bevy::prelude::Messages<crate::camera::FlyTo>,
}

pub(crate) fn draw(ui: &mut egui::Ui, subject: Subject<'_>) {
    let Subject {
        session,
        graveyards,
        assets,
        now,
        server_panel,
        fly,
    } = subject;
    for table in [places::SAFE_LOCS, tables::zones::area::TABLE] {
        if !session.open_table(assets, table) {
            theme::note(ui, format!("Opening {table}.dbc\u{2026}"));
            return;
        }
    }

    theme::note(ui, format!("{} graveyards on this map", graveyards.points.list.len()));
    ui.add_space(4.0);
    controls(ui, graveyards);

    egui::ScrollArea::vertical()
        .id_salt("graveyards")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            match graveyards.points.selected.and_then(|id| graveyards.place(id).cloned()) {
                Some(place) => {
                    theme::heading(ui, &format!("Graveyard {}", place.id));
                    place_block(ui, session, graveyards, fly, &place, now);
                    theme::heading(ui, "Server Rows");
                    server_half(ui, session, graveyards, &place, now);
                }
                None => {
                    theme::heading_about(
                        ui,
                        "Selection",
                        "Click a graveyard in the world. Grey graveyards are linked to no zone, \
                         so the server never sends a ghost there.",
                    );
                    property(ui, "Selected", "", |ui| {
                        theme::value(ui, "None");
                    });
                }
            }
            theme::heading_about(
                ui,
                "Server",
                "The server reads WorldSafeLocs.dbc and world_safe_locs_facing at startup \
                 only. Links are live on `.reload game_graveyard_zone`.",
            );
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
            ui.add_space(12.0);
        });
}

/// What a click on empty ground does: the two modes the pointer can be armed
/// in, as equal buttons, and under them the zone the pointer is over while
/// Link Zone is armed.
fn controls(ui: &mut egui::Ui, graveyards: &mut Graveyards) {
    ui.columns(2, |columns| {
        let armed = graveyards.armed == Armed::NewPlace;
        let button = arm_button(armed, "New Graveyard").min_size(egui::vec2(columns[0].available_width(), 24.0));
        if columns[0]
            .add(button)
            .on_hover_text("Armed, a click on the ground makes a graveyard there. Escape disarms.")
            .clicked()
        {
            graveyards.armed = match armed {
                true => Armed::Nothing,
                false => Armed::NewPlace,
            };
        }
        let linking = graveyards.armed == Armed::LinkZone;
        let button = arm_button(linking, "Link Zone").min_size(egui::vec2(columns[1].available_width(), 24.0));
        let link = columns[1].add_enabled(graveyards.points.selected.is_some() && graveyards.held().is_some(), button);
        if link
            .on_hover_text(
                "Armed, a click on the ground links the selected graveyard to the zone \
                 under the click, for both factions. Escape disarms.",
            )
            .on_disabled_hover_text("Select a graveyard. Available once the server's rows are read.")
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
            None => "The pointer is not over open ground.".to_string(),
        };
        theme::note(ui, under);
    }
}

/// The place's name and position.
fn place_block(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    graveyards: &mut Graveyards,
    fly: &mut bevy::prelude::Messages<crate::camera::FlyTo>,
    place: &SafeLoc,
    now: f64,
) {
    let mut name = place.name.clone();
    property(ui, "Name", "", |ui| {
        if theme::text_field(ui, &mut name)
            .on_hover_text("WorldSafeLocs.dbc name column. The game does not show it.")
            .changed()
        {
            tables::fields::set_text(session, places::SAFE_LOCS, place.record, sf::NAME, &name, "Rename graveyard", now);
            graveyards.stale();
        }
    });
    theme::heading(ui, "Location");
    let mut at = place.at;
    let mut moved = false;
    for (axis, label) in ["X", "Y", "Z"].iter().enumerate() {
        property(ui, label, "", |ui| {
            moved |= theme::field(ui, egui::DragValue::new(&mut at[axis]).speed(0.25).suffix(" yd")).changed();
        });
    }
    if moved {
        graveyards::move_place(session, place, bevy::math::Vec3::from(at), now);
        graveyards.stale();
    }
    // The commands, as equal buttons two to a row.
    ui.add_space(6.0);
    ui.columns(2, |columns| {
        let button = theme::full_button(&columns[0], "Fly To");
        if columns[0].add(button).clicked() {
            fly.write(crate::camera::FlyTo::at(place.at.into()));
        }
        let button = theme::full_button(&columns[1], "Snap to Ground");
        if columns[1]
            .add(button)
            .on_hover_text("Sets z to the ground under the graveyard, where the ground is open.")
            .clicked()
        {
            if let Some(ground) = crate::tools::placements::doodads::ground_height(session, at[0], at[1]) {
                graveyards::move_place(session, place, bevy::math::Vec3::new(at[0], at[1], ground), now);
                graveyards.stale();
            }
        }
    });
    let shipped = graveyards.points.is_shipped(place.id);
    let mut remove = false;
    ui.columns(2, |columns| {
        let button = theme::full_button(&columns[0], match shipped {
            true => "Remove\u{2026}",
            false => "Remove",
        });
        remove = columns[0]
            .add(button)
            .on_hover_text(
                "Removes the row from WorldSafeLocs.dbc, and the links and the facing that \
                 name it from the project's server rows.",
            )
            .clicked();
    });
    if remove {
        match shipped {
            true => graveyards.points.confirm_remove = Some(place.id),
            false => {
                let line = graveyards::remove_place(session, graveyards, place.id, now);
                session.status = line;
            }
        }
    }
    if graveyards.points.confirm_remove == Some(place.id) {
        ui.label(
            egui::RichText::new(
                "This is a shipped graveyard. Each zone linked to it then sends ghosts to \
                 its next nearest linked graveyard, or to none.",
            )
            .size(theme::SMALL)
            .color(theme::WARN),
        );
        ui.columns(2, |columns| {
            let button = theme::full_button(&columns[0], "Remove Graveyard");
            if columns[0].add(button).clicked() {
                let line = graveyards::remove_place(session, graveyards, place.id, now);
                session.status = line;
            }
            let button = theme::full_button(&columns[1], "Cancel");
            if columns[1].add(button).clicked() {
                graveyards.points.confirm_remove = None;
            }
        });
    }
}

/// How wide the faction list of a linked zone's row is, in points: the
/// longest name, Alliance, with the list's arrow.
const LINK_FACTION: f32 = 96.0;

/// The facing and the links.
fn server_half(ui: &mut egui::Ui, session: &mut EditSession, graveyards: &mut Graveyards, place: &SafeLoc, now: f64) {
    if let Some(trouble) = graveyards.trouble() {
        theme::note(ui, format!("The server's rows could not be read: {trouble}"));
        return;
    }
    if graveyards.held().is_none() {
        theme::note(ui, "Reading game_graveyard_zone and world_safe_locs_facing\u{2026}");
        return;
    }
    let id = place.id;
    let edits = session.rows.edits.clone();
    let facing = graveyards.facing(&edits, id);
    let mut degrees = facing.map_or(0.0, |(radians, _)| radians.to_degrees());
    property(ui, "Facing", "", |ui| {
        if theme::field(ui, egui::DragValue::new(&mut degrees).speed(1.0).suffix("\u{b0}"))
            .on_hover_text(
                "world_safe_locs_facing: the facing of a ghost on arrival. The row stores \
                 it in radians. Read at startup only.",
            )
            .changed()
        {
            graveyards::set_facing(session, graveyards, id, degrees.rem_euclid(360.0).to_radians(), now);
        }
    });
    if facing.is_none() {
        theme::note(ui, "No world_safe_locs_facing row: a ghost faces north.");
    }

    let links = graveyards.links_of(&edits, id);
    let serving = links.iter().filter(|shown| shown.life != Life::Delete).count();
    theme::heading_about(
        ui,
        &format!("Linked Zones ({serving})"),
        "Link Zone, above, adds a linked zone.",
    );
    if links.is_empty() {
        theme::note(ui, "No linked zone: the server never sends a ghost here.");
    }
    for shown in &links {
        let removed = shown.life == Life::Delete;
        // The row's controls are laid out first, from the right, so the
        // zone's name is cut with an ellipsis to the room they leave and the
        // row is never wider than the panel.
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match removed {
                    true => {
                        if ui.small_button("Cancel removal").clicked() {
                            let key = shown.link.key();
                            let label = format!("{} {}", vale_mangos::graveyard::ZONE, key.text());
                            let gesture =
                                crate::server::store::Gesture { label: "Cancel graveyard link removal", subject: &label, now };
                            session.set_server_row(vale_mangos::graveyard::ZONE, &key, None, Some(gesture));
                        }
                    }
                    false => {
                        if ui.small_button("Unlink").clicked() {
                            graveyards::remove_link(session, shown, now);
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
                            .width(LINK_FACTION)
                            .show_ui(ui, |ui| {
                                for known in FACTIONS {
                                    ui.selectable_value(&mut faction, known.value, known.name);
                                }
                            });
                        if faction != shown.link.faction {
                            graveyards::set_faction(session, shown, faction, now);
                        }
                    }
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let name = format!("{} ({})", graveyards.zone_name(shown.link.zone), shown.link.zone);
                    let text = egui::RichText::new(name).color(match removed {
                        true => theme::INK_FAINT,
                        false => theme::INK,
                    });
                    ui.add(
                        egui::Label::new(match removed {
                            true => text.strikethrough(),
                            false => text,
                        })
                        .truncate(),
                    );
                });
            });
        });
        for fault in shown.link.check() {
            ui.label(egui::RichText::new(fault).size(theme::SMALL).color(theme::WARN));
        }
    }
}

/// The graveyard tool's inspector panel. Graveyards, like area triggers, are
/// client table rows that are places, picked in the viewport, with server
/// rows beside them.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, assets, now, server_panel, .. } = subject;
    draw(
        ui,
        Subject {
            session,
            graveyards: &mut editing.graveyards,
            assets,
            now,
            server_panel,
            fly: &mut editing.fly,
        },
    );
}
