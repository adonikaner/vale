//! A flight node's two mounts on its card, each as the creature's picture
//! and name, and the dialog one is chosen in.
//!
//! The dialog has the display picker's shape (`crate::ui::subjects::displays`):
//! a search, a grid of rendered models, and a large preview of the one under
//! the pointer with the button that chooses it. Its rows are creatures and
//! not display rows, because the column holds a `creature_template` entry:
//! the creatures other nodes already fly on are listed first, and a search
//! by name or entry lists any other. The state is
//! [`crate::tools::places::mounts`].

use bevy_egui::egui;

use super::flightpaths::value_then;
use crate::preview::portraits::Portraits;
use crate::session::EditSession;
use crate::tables;
use crate::tools::places::flightpaths::Flightpaths;
use crate::tools::places::mounts::{side_name, Mount, MountPick, Mounts};
use crate::ui::widgets::{reference, theme};
use vale_assets::tables::taxi::node_fields as nf;
use vale_client::assets::GameAssets;
use vale_edit::dbc::taxi;

/// A cell of the grid, in points.
const CELL: f32 = 60.0;
/// The gap between cells.
const GAP: f32 = 4.0;
/// How many cells fit across the dialog.
const COLUMNS: usize = 8;
/// The dialog's width: the grid, and the modal's own margins.
const WIDTH: f32 = COLUMNS as f32 * (CELL + GAP) + 24.0;

/// What a creature is called in a row: its name and entry, or its entry
/// alone when the database has not named it.
fn words(mount: Option<&Mount>, entry: u32) -> String {
    match (entry, mount) {
        (0, _) => "None".to_string(),
        (_, Some(mount)) if !mount.name.is_empty() => format!("{} ({entry})", mount.name),
        _ => format!("Creature {entry}"),
    }
}

/// One mount column of a node on its card: the creature's picture, its name
/// and entry, and the button that opens the picker.
pub(super) fn row(
    ui: &mut egui::Ui,
    mounts: &mut Mounts,
    portraits: &mut Portraits,
    assets: &GameAssets,
    node: &taxi::Node,
    side: usize,
) {
    let entry = node.mounts[side];
    let mount = mounts.mount(entry).cloned();
    let about = match side {
        0 => {
            "MountCreatureId 1, a creature_template entry. The server offers the node to the Horde only when \
             this is set, and flies a character on this creature. The standard Horde mount is the wind rider, 2224."
        }
        _ => {
            "MountCreatureId 2, a creature_template entry. The server offers the node to the Alliance only \
             when this is set. The standard Alliance mount is the gryphon, 541."
        }
    };
    theme::property(ui, &format!("{} Mount", side_name(side)), about, |ui| {
        let size = ui.spacing().interact_size.y + 4.0;
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
        match mount.as_ref().and_then(|mount| mounts.worn(assets, mount.display)) {
            Some(worn) => {
                let key = portraits.want_worn(&worn);
                crate::preview::portraits::paint(ui, portraits, &key, rect);
            }
            None if entry != 0 => {
                ui.painter().rect_filled(rect, 3.0, theme::DEAD);
            }
            None => {}
        }
        let colour = match entry {
            0 => theme::INK_DIM,
            _ => theme::INK,
        };
        let chosen = value_then(ui, words(mount.as_ref(), entry), colour, |ui| {
            reference::choose_button(
                ui,
                "Choose the creature from pictures: the mounts other nodes use, or any creature searched by \
                 name or entry. None withholds the node from this faction.",
            )
        });
        if chosen {
            mounts.open(node, side);
        }
    });
}

/// Draw the dialog while one is open, and write the choice to the node's
/// field.
pub(super) fn window(
    ctx: &egui::Context,
    mounts: &mut Mounts,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    assets: &GameAssets,
    portraits: &mut Portraits,
    now: f64,
) {
    // Once a pass, as the display picker is drawn: a second draw of one pass
    // would give egui two widgets with one id.
    let pass = ctx.cumulative_pass_nr();
    let pass_id = egui::Id::new("mount-pick-pass");
    if ctx.data(|data| data.get_temp::<u64>(pass_id)) == Some(pass) {
        return;
    }
    ctx.data_mut(|data| data.insert_temp(pass_id, pass));
    let Some(mut open) = mounts.pick.take() else { return };
    if !crate::ui::dialog_home(&mut open.home, ctx) {
        mounts.pick = Some(open);
        return;
    }
    let in_use = mounts.in_use();
    let mut close = false;
    let mut chosen: Option<u32> = None;
    let response = egui::Modal::new(egui::Id::new("mount-pick")).show(ctx, |ui| {
        ui.set_width(WIDTH);
        crate::ui::subjects::listing::dialog_title(
            ui,
            &format!("{} mount at {}", side_name(open.side), open.node_name),
            "The creature a character is flown on from this node. The server flies its first display. Search \
             text is matched against creature names; a number is matched against the entry.",
        );
        ui.label(
            egui::RichText::new(format!("MountCreatureId {} \u{b7} creature_template", open.side + 1))
                .small()
                .color(theme::INK_DIM),
        );
        let search = ui.add(
            egui::TextEdit::singleline(&mut open.query)
                .hint_text("Part of a creature's name, or an entry")
                .desired_width(WIDTH - 24.0),
        );
        if open.focus {
            search.request_focus();
            open.focus = false;
        }
        if let Some(trouble) = open.trouble().or(mounts.trouble()) {
            ui.label(egui::RichText::new(format!("World database not read: {trouble}")).small().color(theme::WARN));
        }

        egui::ScrollArea::vertical()
            .id_salt("mount-grid")
            .max_height(CELL * 4.0 + 64.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let searched = !open.query.trim().is_empty();
                if searched {
                    let hits = open.hits.clone();
                    let heading = match (open.searching(), hits.len()) {
                        (true, _) => "Searching\u{2026}".to_string(),
                        (false, 0) => "No creature matches.".to_string(),
                        (false, n) if n >= crate::tools::places::mounts::SEARCH_LIMIT => format!("First {n} matches"),
                        (false, n) => format!("Matches ({n})"),
                    };
                    theme::note(ui, heading);
                    if grid(ui, &mut open, mounts, assets, portraits, &hits) {
                        chosen = open.preview;
                    }
                    ui.add_space(6.0);
                }
                theme::note(ui, "Mounts the flight nodes use");
                if grid(ui, &mut open, mounts, assets, portraits, &in_use) {
                    chosen = open.preview;
                }
            });

        ui.add_space(6.0);
        if let Some(entry) = open.preview {
            let mount = in_use.iter().chain(&open.hits).find(|mount| mount.entry == entry).cloned();
            if preview(ui, mounts, assets, portraits, entry, mount.as_ref()) {
                chosen = Some(entry);
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                close = true;
            }
            if ui
                .button("Set to None")
                .on_hover_text(format!("Writes 0. The server then does not offer the node to the {}.", side_name(open.side)))
                .clicked()
            {
                chosen = Some(0);
            }
            ui.label(egui::RichText::new("Esc closes").small().color(theme::INK_FAINT));
        });
    });
    if let Some(entry) = chosen {
        tables::fields::set_fields(
            session,
            taxi::NODES,
            open.record,
            &[(nf::MOUNT + open.side, entry)],
            "Change flight node mounts",
            &format!("TaxiNodes {} mounts", open.node),
            now,
        );
        flights.stale();
        close = true;
    }
    if response.should_close() {
        close = true;
    }
    if !close {
        mounts.pick = Some(open);
    }
}

/// A grid of creatures. `true` when one was clicked, which is then the
/// picker's preview.
fn grid(
    ui: &mut egui::Ui,
    open: &mut MountPick,
    mounts: &mut Mounts,
    assets: &GameAssets,
    portraits: &mut Portraits,
    listed: &[Mount],
) -> bool {
    let mut clicked = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
        for mount in listed {
            let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(CELL), egui::Sense::click());
            let on = open.showing == mount.entry;
            if on || response.hovered() {
                let fill = match on {
                    true => theme::ACCENT_SUNK,
                    false => theme::RAISED,
                };
                ui.painter().rect_filled(rect, 3.0, fill);
            }
            if response.hovered() {
                open.preview = Some(mount.entry);
            }
            let inner = rect.shrink(3.0);
            match mounts.worn(assets, mount.display) {
                Some(worn) => {
                    let key = portraits.want_worn(&worn);
                    crate::preview::portraits::paint(ui, portraits, &key, inner);
                }
                None => {
                    ui.painter().rect_filled(inner, 2.0, theme::DEAD);
                }
            }
            if response.on_hover_text(words(Some(mount), mount.entry)).clicked() {
                open.preview = Some(mount.entry);
                clicked = true;
            }
        }
    });
    clicked
}

/// The creature under the pointer, large, with its name, entry, display and
/// model, and the button that chooses it. `true` when the button was pressed.
fn preview(
    ui: &mut egui::Ui,
    mounts: &mut Mounts,
    assets: &GameAssets,
    portraits: &mut Portraits,
    entry: u32,
    mount: Option<&Mount>,
) -> bool {
    let mut used = false;
    ui.horizontal(|ui| {
        let worn = mount.and_then(|mount| mounts.worn(assets, mount.display));
        match worn.as_ref() {
            Some(worn) => {
                let key = portraits.want_large_worn(worn);
                let pane = crate::preview::portraits::pane_id(ui);
                crate::preview::portraits::paint_large(ui, portraits, pane, &key, 120.0);
            }
            None => {
                let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(120.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 4.0, theme::SUNK);
            }
        }
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(words(mount, entry)).color(theme::INK).size(13.0));
            match (mount, worn.as_ref()) {
                (Some(mount), Some(worn)) => {
                    ui.label(theme::number(format!("display {}", mount.display)));
                    ui.label(egui::RichText::new(&worn.path).small().color(theme::INK_DIM));
                }
                (Some(mount), None) => {
                    ui.label(
                        egui::RichText::new(format!("Display {} has no model in the game files", mount.display))
                            .small()
                            .color(theme::BAD),
                    );
                }
                (None, _) => {
                    ui.label(egui::RichText::new("The database has not named this creature").small().color(theme::INK_DIM));
                }
            }
            used = ui
                .button("Use This Creature")
                .on_hover_text("Writes this creature's entry into the node's mount column and closes the picker.")
                .clicked();
        });
    });
    used
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mount_is_called_by_name_and_entry() {
        let gryphon = Mount { entry: 541, name: "Riding Gryphon".to_string(), display: 1149 };
        assert_eq!(words(Some(&gryphon), 541), "Riding Gryphon (541)");
        assert_eq!(words(None, 541), "Creature 541");
        assert_eq!(words(Some(&Mount { name: String::new(), ..gryphon }), 541), "Creature 541");
        assert_eq!(words(None, 0), "None");
    }
}
