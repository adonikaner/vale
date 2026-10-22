//! The Creatures workspace: every `creature_template` row as a list, the open
//! row's form, and what it looks like.
//!
//! ## Three parts, as the item workspace has
//!
//! ```text
//! left       which creature    every row the server loads, searched by name,
//!                              subname or entry
//! middle     what it is        the template window's form: every column
//! inspector  what it is like   the model, a summary, Place in World, and the
//!                              quests, loot, events, spells, gossip, vendor
//!                              and trainer windows
//! ```
//!
//! ## The form is the template window's
//!
//! The middle is [`crate::ui::tools::spawns::creatures::groups`] over the open row, under the
//! same entry box. Every edit is the creature tool's own, on the same undo
//! stack and in the same Creatures block of the bar's Server…, so a row
//! edited here and in the template window is one row with one set of edits.
//! See [`crate::subjects::creature_data`].
//!
//! ## Where it meets the creature tool
//!
//! The rail's Creatures tool edits spawns. A selected spawn's panel, and a
//! creature chosen in its Place picker, have Open in Workspace, which opens
//! the template here. The inspector here has Place in World, which switches
//! to the rail tool in Place with the open creature chosen, so the next click
//! on the ground spawns one.
//!
//! The list, its search and the form's head are drawn as the Game Objects
//! workspace draws them; see [`super::listing`].

use bevy_egui::egui;

use crate::ui::tools::spawns::creatures::Subject;
use super::listing;
use crate::ui::widgets::rowform::page_spacing;
use crate::ui::widgets::theme;
use crate::tools::spawns::creatures::{server_patch, Known, TemplateSubject};
use crate::tools::Tool;
use vale_mangos::creature;

/// Draw the workspace into the region the panels left.
pub fn draw(ui: &mut egui::Ui, mut subject: Subject<'_>) {
    listing::frame(ui, &mut subject, "creature-data", list, form);
    crate::ui::tools::spawns::creatures::dialogs(ui.ctx(), &mut subject);
}

/// The left column.
fn list(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    // Templates and Displays, the workspace's two parts. The shell switches
    // after the frame is drawn.
    if let Some(part) = crate::ui::shell::rail::parts(ui, Tool::CreatureData) {
        subject.creatures.switch_to = Some(part);
    }
    if !listing::head(ui, "Creatures", &subject.creatures.listed) {
        return;
    }
    let (edits, revision) = (&subject.session.server_edits, subject.session.server_edit_revision);
    let matches = listing::search(
        ui,
        &mut subject.creatures.listed,
        "name, subname or entry",
        edits,
        revision,
    );
    actions(ui, subject);
    ui.add_space(4.0);
    let at = listing::reveal(&mut subject.creatures.listed, &matches);
    let mut clicked = None;
    let height = listing::ROW_HEIGHT;
    theme::list_area(ui, at, height).show_rows(ui, height, matches.len(), |ui, range| {
        for at in range {
            let index = matches[at];
            let Some(known) = subject
                .creatures
                .listed
                .shown(index, &subject.session.server_edits)
            else {
                continue;
            };
            let chosen = subject.creatures.listed.open == Some(known.entry);
            let response = row(ui, subject, &known, chosen);
            if response.clicked() {
                clicked = Some(known.entry);
            }
            listing::row_menu(&response, subject.session, creature::TEMPLATE, known.entry);
        }
    });
    if let Some(entry) = clicked {
        subject.creatures.listed.revealed = Some(entry);
        subject.creatures.open_listed(entry);
    }
}

/// The levels as a list or a summary shows them: one number when the two
/// are the same.
fn levels(known: &Known) -> String {
    match known.level.0 == known.level.1 {
        true => format!("{}", known.level.0),
        false => format!("{}\u{2013}{}", known.level.0, known.level.1),
    }
}

/// One row: the creature's portrait, its name, its subname and levels, and
/// its entry.
fn row(ui: &mut egui::Ui, subject: &mut Subject<'_>, known: &Known, chosen: bool) -> egui::Response {
    let levels = format!("level {}", levels(known));
    let sub = match known.subname.as_deref().filter(|text| !text.is_empty()) {
        Some(subname) => format!("<{subname}> \u{b7} {levels}{}", listing::claim_word(known.claim)),
        None => format!("{levels}{}", listing::claim_word(known.claim)),
    };
    let shape = theme::list_row(
        ui,
        theme::ListRow {
            title: &known.name,
            sub: &sub,
            trailing: &known.entry.to_string(),
            tint: theme::INK,
            picture: true,
        },
        chosen,
    );
    // Asked for on every frame the row is drawn, so only the rows on screen
    // are rendered. See `crate::preview::portraits`.
    match crate::ui::tools::spawns::creatures::worn(subject, known.display_id) {
        Some(worn) => {
            let key = subject.portraits.want_worn(&worn);
            crate::preview::portraits::paint(ui, subject.portraits, &key, shape.picture);
        }
        None => {
            ui.painter().rect_filled(shape.picture, 3.0, theme::LINE);
        }
    }
    shape.response
}

/// New, Copy and Discard: the creature tool's three template buttons, acting
/// on the open row and opening what they make here.
fn actions(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    let now = subject.now;
    let patch = server_patch(subject.server);
    let shown = subject.creatures.listed.subject(&subject.session.server_edits);
    let row_read = shown.as_ref().is_some_and(|shown| {
        subject
            .creatures
            .template
            .as_ref()
            .is_some_and(|held| held.entry == shown.entry)
    });
    let created = shown.as_ref().is_some_and(TemplateSubject::is_new);
    ui.horizontal(|ui| {
        let third = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(40.0);
        let size = egui::vec2(third, 22.0);
        if ui
            .add(egui::Button::new("+ New").min_size(size))
            .on_hover_text(
                "Creates a creature_template row with an entry above any upstream entry. It \
                 starts as a level 1 friendly humanoid with no model, and opens here.",
            )
            .clicked()
        {
            let entry = subject
                .creatures
                .create_listed_template(subject.session, "New Creature", patch, now);
            subject.session.status = format!("creature_template {entry} created");
        }
        if ui
            .add_enabled(row_read, egui::Button::new("Copy").min_size(size))
            .on_hover_text(
                "Copies the open creature to a new entry, every column as shown: the \
                 project's value where it has one, otherwise the database value.",
            )
            .on_disabled_hover_text("Open a creature first.")
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject.session.status = match subject
                    .creatures
                    .copy_listed_template(subject.session, shown, patch, now)
                {
                    Some(entry) => format!("copied as creature_template {entry}"),
                    None => "the row is still being read".to_string(),
                };
            }
        }
        if ui
            .add_enabled(created, egui::Button::new("Discard").min_size(size))
            .on_hover_text(
                "Removes the creature this project creates. The database is not changed. A \
                 row already in the database cannot be removed: .reload creature_template \
                 never drops an entry it has read.",
            )
            .on_disabled_hover_text("Only a creature this project created can be discarded.")
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject
                    .creatures
                    .discard_template(subject.session, shown.entry, now);
                subject.session.status = format!("creature_template {} discarded", shown.entry);
            }
        }
    });
}

/// The middle: what the project changes, then the open row's columns.
fn form(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    egui::ScrollArea::vertical()
        .id_salt("creature-data-form")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            page_spacing(ui);
            listing::edits_block(
                ui,
                crate::ui::shell::sync::Half::Creatures,
                subject.session,
                subject.assets,
                subject.server_panel,
            );
            let Some(shown) = subject.creatures.listed.subject(&subject.session.server_edits)
            else {
                listing::nothing_open(ui, &subject.creatures.listed, "creature");
                return;
            };
            listing::form_head(ui, &shown, creature::TEMPLATE);
            ui.horizontal(|ui| {
                let colour = match shown.entry != shown.read_entry {
                    true => theme::WARN,
                    false => theme::INK_DIM,
                };
                ui.label(egui::RichText::new("entry").color(colour))
                    .on_hover_text("The creature_template entry. Changing it renumbers the row.");
                let id = ui.make_persistent_id(("creature-data-entry", shown.patch));
                crate::ui::tools::spawns::creatures::entry_field(ui, subject, &shown, id);
            });
            ui.add_space(6.0);
            let row = subject
                .creatures
                .template
                .as_ref()
                .filter(|held| held.entry == shown.entry)
                .map(|held| held.row.clone());
            let Some(row) = row else {
                theme::waiting(ui, "reading the row\u{2026}");
                return;
            };
            crate::ui::tools::spawns::creatures::groups(
                ui,
                subject,
                creature::TEMPLATE,
                &shown.key(),
                &row,
                &[creature::Group::Identity, creature::Group::Appearance],
            );
        });
}

/// The third part, in the shell's inspector: the open creature's model, a
/// summary, Place in World, and the windows for what the creature has besides
/// its template.
pub fn inspector(ui: &mut egui::Ui, mut subject: Subject<'_>) {
    let subject = &mut subject;
    let Some(known) = subject.creatures.listed.open_row(&subject.session.server_edits) else {
        theme::note(ui, "Open a creature on the left to see its model and its other tables.");
        return;
    };
    ui.label(egui::RichText::new(known.label()).strong().size(14.0));
    match crate::ui::tools::spawns::creatures::worn(subject, known.display_id) {
        Some(worn) => {
            let key = subject.portraits.want_large_worn(&worn);
            // As wide as the inspector, as the item workspace's preview is.
            let side = ui.available_width();
            crate::ui::widgets::preview::preview_pane_at(ui, subject.portraits, &key, side);
        }
        None => theme::note(
            ui,
            "display_id1 is 0 or has no CreatureDisplayInfo.dbc row. The editor draws a spawn \
             of it as a marker.",
        ),
    }
    ui.add_space(4.0);
    egui::Grid::new("creature-data-summary")
        .num_columns(2)
        .spacing([8.0, 2.0])
        .show(ui, |ui| {
            for (name, value) in [
                ("Level", levels(&known)),
                ("Rank", creature::value_word(&creature::RANKS, known.rank)),
                ("Faction", known.faction.to_string()),
                ("Display", known.display_id.to_string()),
                ("NPC flags", creature::mask_words(&creature::NPC_FLAGS, known.npc_flags)),
            ] {
                ui.label(egui::RichText::new(name).color(theme::INK_DIM));
                match name == "Display" && known.display_id != 0 {
                    true => crate::ui::widgets::reference::display_link(
                        ui,
                        egui::RichText::new(value),
                        &mut subject.quests.show_row,
                        crate::subjects::displays::Table::Creature.name(),
                        known.display_id,
                    ),
                    false => {
                        ui.label(egui::RichText::new(value).color(theme::INK));
                    }
                }
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    if ui
        .button("Place in World")
        .on_hover_text(
            "Switches to the rail's Creatures tool in Place with this creature chosen. The \
             next click on the ground creates a spawn of it.",
        )
        .clicked()
    {
        subject.creatures.mode = crate::tools::placements::place::Mode::Place;
        subject.creatures.new_spawn.chosen = Some(known.clone());
        subject.creatures.switch_to = Some(Tool::Creatures);
    }
    ui.add_space(6.0);
    theme::heading(ui, "Other tables");
    crate::ui::tools::spawns::creatures::windows(ui, subject, known.entry, known.label(), None);
}
