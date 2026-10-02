//! The hover card: a small panel beside the pointer that identifies the
//! creature or game object under it.
//!
//! Drawn only under the tool that owns the population, and only while that
//! tool's own pick reports a spawn under the pointer
//! ([`crate::tools::creatures::Creatures::hovered`],
//! [`crate::tools::gameobjects::GameObjects::hovered`]). Those picks already
//! clear themselves over a panel, during a drag and while placing, so the card
//! inherits all three conditions and adds none of its own.
//!
//! Layout: the name as a title, then a short table of attributes — label on
//! the left, value on the right. It carries identification only. Everything
//! else about the spawn is on the form, one click away.

use bevy_egui::egui;

use super::theme;
use crate::tools::Tool;
use vale_mangos::creature;
use vale_mangos::row::Edits;

/// Distance from the pointer to the card's top-left corner, in points. Clears
/// the arrow cursor so the card does not cover what is being pointed at.
const OFFSET: egui::Vec2 = egui::vec2(16.0, 18.0);

/// The widest the card is allowed to become before a value wraps.
const MAX_WIDTH: f32 = 240.0;

/// One card's contents.
struct Card {
    title: String,
    subtitle: Option<String>,
    rows: Vec<(&'static str, String)>,
    /// A pending change to the row, shown as the last line in the warning colour.
    status: Option<&'static str>,
}

/// Draw the card for whatever the active tool's pick reports, if anything.
pub fn draw(
    ctx: &egui::Context,
    tool: Tool,
    creatures: &crate::tools::creatures::Creatures,
    objects: &mut crate::tools::gameobjects::GameObjects,
    edits: &Edits,
    assets: &vale_client::assets::GameAssets,
) {
    let card = match tool {
        Tool::Creatures if creatures.drag.is_none() => creatures
            .hovered_edited(Some(edits))
            .map(|spawn| creature_card(&spawn)),
        Tool::GameObjects if objects.drag.is_none() => objects
            .hovered_edited(Some(edits))
            .map(|spawn| object_card(&spawn, objects, assets)),
        _ => None,
    };
    let Some(card) = card else { return };
    let Some(pointer) = ctx.pointer_hover_pos() else {
        return;
    };
    egui::Area::new(egui::Id::new("world-hover-card"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .fixed_pos(pointer + OFFSET)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::RAISED)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_max_width(MAX_WIDTH);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(
                        egui::RichText::new(&card.title)
                            .size(12.5)
                            .strong()
                            .color(theme::INK),
                    );
                    if let Some(subtitle) = &card.subtitle {
                        ui.label(egui::RichText::new(subtitle).size(theme::SMALL).color(theme::INK_DIM));
                    }
                    ui.add_space(3.0);
                    egui::Grid::new("world-hover-card-rows")
                        .num_columns(2)
                        .spacing(egui::vec2(10.0, 1.0))
                        .show(ui, |ui| {
                            for (label, value) in &card.rows {
                                ui.label(
                                    egui::RichText::new(*label).size(theme::SMALL).color(theme::INK_FAINT),
                                );
                                ui.label(egui::RichText::new(value).size(theme::SMALL).color(theme::INK));
                                ui.end_row();
                            }
                        });
                    if let Some(status) = card.status {
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new(status).size(theme::SMALL).color(theme::WARN));
                    }
                });
        });
}

fn creature_card(spawn: &crate::tools::creatures::Spawn) -> Card {
    let mut rows = Vec::new();
    let (low, high) = spawn.level;
    rows.push((
        "Level",
        match low == high {
            true => low.to_string(),
            false => format!("{low}–{high}"),
        },
    ));
    if spawn.rank != 0 {
        rows.push(("Rank", creature::value_word(&creature::RANKS, spawn.rank)));
    }
    // The same table the icons over a creature's head are drawn from, so the
    // card and the icons name the same services in the same order.
    let services: Vec<&str> = super::servicemarks::offered(spawn.npc_flags)
        .map(|service| service.name)
        .collect();
    if !services.is_empty() {
        rows.push(("Services", services.join(", ")));
    }
    rows.push(("Entry", spawn.entry.to_string()));
    rows.push(("GUID", spawn.guid.to_string()));
    Card {
        title: spawn
            .name
            .clone()
            .unwrap_or_else(|| "Missing template".to_string()),
        subtitle: spawn
            .subname
            .as_deref()
            .filter(|text| !text.is_empty())
            .map(|text| format!("<{text}>")),
        rows,
        status: status(spawn.is_new(), spawn.is_removed()),
    }
}

fn object_card(
    spawn: &crate::tools::gameobjects::Spawn,
    objects: &mut crate::tools::gameobjects::GameObjects,
    assets: &vale_client::assets::GameAssets,
) -> Card {
    let mut rows = Vec::new();
    let title = match &spawn.known {
        Some(known) => {
            rows.push(("Type", known.type_word()));
            let locks = super::gameobjects::locks(&mut objects.locks, assets);
            match super::gameobjects::gathered_by(locks, known) {
                Some(node) => rows.push(("Gathering", node)),
                None => {
                    if let Some(lock) = known.lock() {
                        rows.push(("Requires", super::gameobjects::lock_words(locks, lock)));
                    }
                }
            }
            known.name.clone()
        }
        None => "Missing template".to_string(),
    };
    rows.push(("Entry", spawn.entry.to_string()));
    rows.push(("GUID", spawn.guid.to_string()));
    Card {
        title,
        subtitle: None,
        rows,
        status: status(spawn.is_new(), spawn.is_removed()),
    }
}

fn status(new: bool, removed: bool) -> Option<&'static str> {
    match (new, removed) {
        (true, _) => Some("Pending insert"),
        (_, true) => Some("Pending delete"),
        _ => None,
    }
}
