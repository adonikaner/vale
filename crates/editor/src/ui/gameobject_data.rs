//! The Game Objects workspace: every `gameobject_template` row as a list, the
//! open row's form, and what it looks like.
//!
//! The Creatures workspace's arrangement for the next table; see
//! [`super::creature_data`] for the three parts and why the form is the
//! template window's. What differs is the game object form's own: the 24
//! `data` columns are named by the row's type, a row is listed under its type
//! or the profession its lock needs, and the inspector has only the quest and
//! loot windows, which are the two a game object has. The rail tool it links
//! to and from is Objects.
//!
//! The list, its search and the form's head are drawn as the Creatures
//! workspace draws them; see [`super::listing`].

use bevy_egui::egui;

use super::gameobjects::Subject;
use super::listing;
use super::rowform::page_spacing;
use super::theme;
use crate::tools::creatures::{server_patch, TemplateSubject};
use crate::tools::gameobjects::Known;
use crate::tools::Tool;
use vale_mangos::gameobject;

/// Draw the workspace into the region the panels left.
pub fn draw(ui: &mut egui::Ui, mut subject: Subject<'_>) {
    listing::frame(ui, &mut subject, "gameobject-data", list, form);
    super::gameobjects::dialogs(ui.ctx(), &mut subject);
}

/// The left column.
fn list(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    // Templates and Displays, the workspace's two parts. The shell switches
    // after the frame is drawn.
    if let Some(part) = super::rail::parts(ui, Tool::GameObjectData) {
        subject.objects.switch_to = Some(part);
    }
    if !listing::head(ui, "Game Objects", &subject.objects.listed) {
        return;
    }
    let (edits, revision) = (&subject.session.server_edits, subject.session.server_edit_revision);
    let matches = listing::search(
        ui,
        &mut subject.objects.listed,
        "name, type or entry",
        edits,
        revision,
    );
    actions(ui, subject);
    ui.add_space(4.0);
    let at = listing::reveal(&mut subject.objects.listed, &matches);
    let mut clicked = None;
    let height = listing::ROW_HEIGHT;
    theme::list_area(ui, at, height).show_rows(ui, height, matches.len(), |ui, range| {
        for at in range {
            let index = matches[at];
            let Some(known) = subject
                .objects
                .listed
                .shown(index, &subject.session.server_edits)
            else {
                continue;
            };
            let chosen = subject.objects.listed.open == Some(known.entry);
            let response = row(ui, subject, &known, chosen);
            if response.clicked() {
                clicked = Some(known.entry);
            }
            listing::row_menu(&response, subject.session, gameobject::TEMPLATE, known.entry);
        }
    });
    if let Some(entry) = clicked {
        subject.objects.listed.revealed = Some(entry);
        subject.objects.open_listed(entry);
    }
}

/// One row: the model, the name, the type (or the profession a gathering
/// node needs), and the entry.
fn row(ui: &mut egui::Ui, subject: &mut Subject<'_>, known: &Known, chosen: bool) -> egui::Response {
    let locks = super::gameobjects::locks(&mut subject.objects.locks, subject.assets);
    let kind = super::gameobjects::gathered_by(locks, known).unwrap_or_else(|| known.type_word());
    let sub = format!("{kind}{}", listing::claim_word(known.claim));
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
    match subject.objects.model_of(subject.assets, known.display_id) {
        Some(path) => crate::portraits::paint(ui, subject.portraits, &path, shape.picture),
        None => {
            ui.painter().rect_filled(shape.picture, 3.0, theme::LINE);
        }
    }
    shape.response
}

/// New, Copy and Discard: the game object tool's three template buttons,
/// acting on the open row and opening what they make here.
fn actions(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    let now = subject.now;
    let patch = server_patch(subject.server);
    let shown = subject.objects.listed.subject(&subject.session.server_edits);
    let row_read = shown.as_ref().is_some_and(|shown| {
        subject
            .objects
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
                "Creates a gameobject_template row with an entry from 2,000,000 up, clear of \
                 upstream vmangos entries. It starts as a Generic object of size 1 with no \
                 model, and opens here.",
            )
            .clicked()
        {
            let entry = subject.objects.create_listed_template(
                subject.session,
                "New Game Object",
                patch,
                now,
            );
            subject.session.status = format!("gameobject_template {entry} created");
        }
        if ui
            .add_enabled(row_read, egui::Button::new("Copy").min_size(size))
            .on_hover_text(
                "Copies the open game object to a new entry, every column as shown: the \
                 project's value where it has one, otherwise the database value.",
            )
            .on_disabled_hover_text("Open a game object first.")
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject.session.status =
                    match subject.objects.copy_listed_template(subject.session, shown, patch, now) {
                        Some(entry) => format!("copied as gameobject_template {entry}"),
                        None => "the row is still being read".to_string(),
                    };
            }
        }
        if ui
            .add_enabled(created, egui::Button::new("Discard").min_size(size))
            .on_hover_text(
                "Removes the game object this project creates. The database is not changed. \
                 A row already in the database cannot be removed: .reload \
                 gameobject_template never drops an entry it has read.",
            )
            .on_disabled_hover_text("Only a game object this project created can be discarded.")
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject.objects.discard_template(subject.session, shown.entry, now);
                subject.session.status = format!("gameobject_template {} discarded", shown.entry);
            }
        }
    });
}

/// The middle: what the project changes, then the open row's columns, with
/// the data columns named by the row's type.
fn form(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    egui::ScrollArea::vertical()
        .id_salt("gameobject-data-form")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            page_spacing(ui);
            listing::edits_block(
                ui,
                super::sync::Half::GameObjects,
                subject.session,
                subject.assets,
                subject.server_panel,
            );
            let Some(shown) = subject.objects.listed.subject(&subject.session.server_edits) else {
                listing::nothing_open(ui, &subject.objects.listed, "game object");
                return;
            };
            listing::form_head(ui, &shown, gameobject::TEMPLATE);
            ui.horizontal(|ui| {
                let colour = match shown.entry != shown.read_entry {
                    true => theme::WARN,
                    false => theme::INK_DIM,
                };
                ui.label(egui::RichText::new("entry").color(colour))
                    .on_hover_text("The gameobject_template entry. Changing it renumbers the row.");
                let id = ui.make_persistent_id(("gameobject-data-entry", shown.patch));
                super::gameobjects::entry_field(ui, subject, &shown, id);
            });
            ui.add_space(6.0);
            let row = subject
                .objects
                .template
                .as_ref()
                .filter(|held| held.entry == shown.entry)
                .map(|held| held.row.clone());
            let Some(row) = row else {
                theme::waiting(ui, "reading the row\u{2026}");
                return;
            };
            let key = shown.key();
            let form = super::gameobjects::Form {
                table: gameobject::TEMPLATE,
                key: &key,
                row: &row,
                spawn: None,
            };
            super::gameobjects::groups(
                ui,
                subject,
                &form,
                &[
                    gameobject::Group::Identity,
                    gameobject::Group::Appearance,
                    gameobject::Group::Stats,
                ],
            );
        });
}

/// The third part, in the shell's inspector: the open game object's model, a
/// summary, Place in World, and its quest and loot windows.
pub fn inspector(ui: &mut egui::Ui, mut subject: Subject<'_>) {
    let subject = &mut subject;
    let Some(known) = subject.objects.listed.open_row(&subject.session.server_edits) else {
        theme::note(ui, "Open a game object on the left to see its model and its other tables.");
        return;
    };
    ui.label(egui::RichText::new(known.label()).strong().size(14.0));
    match subject.objects.model_of(subject.assets, known.display_id) {
        // As wide as the inspector; see `ui::creature_data`.
        Some(path) => {
            let side = ui.available_width();
            super::inspector::preview_pane_at(ui, subject.portraits, &path, side)
        }
        None => theme::note(
            ui,
            "displayId is 0 or has no GameObjectDisplayInfo.dbc row. The editor draws a spawn \
             of it as a marker. Traps and spell focus objects usually have no model.",
        ),
    }
    ui.add_space(4.0);
    let lock = known.lock().map(|id| {
        let locks = super::gameobjects::locks(&mut subject.objects.locks, subject.assets);
        format!("{id} \u{2014} {}", super::gameobjects::lock_words(locks, id))
    });
    egui::Grid::new("gameobject-data-summary")
        .num_columns(2)
        .spacing([8.0, 2.0])
        .show(ui, |ui| {
            let mut rows: Vec<(&str, String)> = vec![
                ("Type", known.type_word()),
                ("Display", known.display_id.to_string()),
                ("Size", format!("{}", known.size)),
                ("Faction", known.faction.to_string()),
                ("Flags", gameobject::mask_words(&gameobject::FLAGS, known.flags)),
            ];
            if let Some(lock) = lock {
                rows.push(("Lock", lock));
            }
            if let Some(loot) = known.loot() {
                rows.push(("Loot", format!("gameobject_loot_template {loot}")));
            }
            for (name, value) in rows {
                ui.label(egui::RichText::new(name).color(theme::INK_DIM));
                ui.label(egui::RichText::new(value).color(theme::INK));
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    if ui
        .button("Place in World")
        .on_hover_text(
            "Switches to the rail's Objects tool in Place with this game object chosen. The \
             next click on the ground creates a spawn of it.",
        )
        .clicked()
    {
        subject.objects.mode = crate::tools::place::Mode::Place;
        subject.objects.new_spawn.chosen = Some(known.clone());
        subject.objects.switch_to = Some(Tool::GameObjects);
    }
    ui.add_space(6.0);
    theme::heading(ui, "Other tables");
    super::gameobjects::windows(ui, subject, known.entry, known.label(), false);
}
