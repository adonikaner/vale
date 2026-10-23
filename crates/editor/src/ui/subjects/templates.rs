//! The Creatures and Game Objects workspaces: every template row of one spawn
//! subject as a list, the open row's form, and what it looks like. The view
//! of [`crate::subjects::templates`].
//!
//! ## Three parts, as the item workspace has
//!
//! ```text
//! left       which template    every row the server loads, searched by name,
//!                              the subject's second line, or entry
//! middle     what it is        the template window's form: every column
//! inspector  what it is like   the model, a summary, Place in World, and the
//!                              windows for what the template has besides its
//!                              row: quests and loot for both, and events,
//!                              spells, gossip, vendor and trainer for a
//!                              creature
//! ```
//!
//! ## The form is the template window's
//!
//! The middle is the subject's [`SpawnView::template_form`] over the open
//! row, under the same entry box. Every edit is the tool's own, on the same
//! undo stack and in the same block of the bar's Server…, so a row edited
//! here and in the template window is one row with one set of edits. A game
//! object's `data` columns are named by its type here as in the window.
//!
//! ## Where it meets the spawn tool
//!
//! The rail's Creatures and Objects tools edit spawns. A selected spawn's
//! panel, and a template chosen in its Place picker, have Open in Workspace,
//! which opens the template here. The inspector here has Place in World,
//! which switches to the rail tool in Place with the open template chosen, so
//! the next click on the ground spawns one.
//!
//! The list, its search and the form's head are drawn as the item and quest
//! workspaces draw theirs; see [`super::listing`].

use bevy_egui::egui;

use super::listing;
use crate::server::settings::server_patch;
use crate::subjects::listing::Listed;
use crate::tools::spawns::spawn::{Template, TemplateSubject};
use crate::ui::tools::spawns::spawn::{SpawnView, Subject};
use crate::ui::widgets::rowform::page_spacing;
use crate::ui::widgets::theme;

/// Draw the workspace into the region the panels left.
pub(crate) fn draw<S: SpawnView>(ui: &mut egui::Ui, mut subject: Subject<'_, S>) {
    listing::frame(ui, &mut subject, S::TEMPLATE, list::<S>, form::<S>);
    crate::ui::tools::spawns::spawn::dialogs(ui.ctx(), &mut subject);
}

/// The left column.
fn list<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>) {
    // Templates and Displays, the workspace's two parts. The shell switches
    // after the frame is drawn.
    if let Some(part) = crate::ui::shell::rail::parts(ui, S::WORKSPACE) {
        subject.asks.push(crate::ui::shell::asks::ShellAsk::Link(part));
    }
    if !listing::head(ui, S::TEXT.workspace, &subject.tool.listed) {
        return;
    }
    let (edits, revision) = (&subject.session.rows.edits, subject.session.rows.revision);
    let matches = listing::search(ui, &mut subject.tool.listed, S::TEXT.list_search, edits, revision);
    actions(ui, subject);
    ui.add_space(4.0);
    let at = listing::reveal(&mut subject.tool.listed, &matches);
    let mut clicked = None;
    let height = listing::ROW_HEIGHT;
    theme::list_area(ui, at, height).show_rows(ui, height, matches.len(), |ui, range| {
        for at in range {
            let index = matches[at];
            let Some(known) = subject.tool.listed.shown(index, &subject.session.rows.edits) else {
                continue;
            };
            let chosen = subject.tool.listed.open == Some(known.entry());
            let response = row(ui, subject, &known, chosen);
            if response.clicked() {
                clicked = Some(known.entry());
            }
            listing::row_menu(&response, subject.session, S::TEMPLATE, known.entry());
        }
    });
    if let Some(entry) = clicked {
        subject.tool.listed.revealed = Some(entry);
        subject.tool.open_listed(entry);
    }
}

/// One row: the template's picture, its name, the subject's line about it,
/// and its entry.
fn row<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>, known: &S::Known, chosen: bool) -> egui::Response {
    let sub = format!("{}{}", S::list_line(subject, known), listing::claim_word(known.claim()));
    let shape = theme::list_row(
        ui,
        theme::ListRow {
            title: known.name(),
            sub: &sub,
            trailing: &known.entry().to_string(),
            tint: theme::INK,
            picture: true,
        },
        chosen,
    );
    // Asked for on every frame the row is drawn, so only the rows on screen
    // are rendered. See `crate::preview::portraits`.
    if !S::picture(ui, subject, known.display_id(), shape.picture) {
        ui.painter().rect_filled(shape.picture, 3.0, theme::LINE);
    }
    shape.response
}

/// New, Copy and Discard: the tool's three template buttons, acting on the
/// open row and opening what they make here.
fn actions<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>) {
    let now = subject.now;
    let patch = server_patch(subject.server);
    let shown = subject.tool.listed.subject(&subject.session.rows.edits);
    let row_read = shown
        .as_ref()
        .is_some_and(|shown| subject.tool.template.as_ref().is_some_and(|held| held.entry == shown.entry));
    let created = shown.as_ref().is_some_and(TemplateSubject::is_new);
    let noun = S::WORDS.noun;
    ui.horizontal(|ui| {
        let third = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(40.0);
        let size = egui::vec2(third, 22.0);
        if ui.add(egui::Button::new("+ New").min_size(size)).on_hover_text(S::TEXT.new_listed).clicked() {
            let entry = subject.tool.create_listed_template(subject.session, S::WORDS.new_name, patch, now);
            subject.session.status = format!("{} {entry} created", S::TEMPLATE);
        }
        if ui
            .add_enabled(row_read, egui::Button::new("Copy").min_size(size))
            .on_hover_text(format!(
                "Copies the open {noun} to a new entry, every column as shown: the \
                 project's value where it has one, otherwise the database value."
            ))
            .on_disabled_hover_text(format!("Open a {noun} first."))
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject.session.status = match subject.tool.copy_listed_template(subject.session, shown, patch, now) {
                    Some(entry) => format!("copied as {} {entry}", S::TEMPLATE),
                    None => "the row is still being read".to_string(),
                };
            }
        }
        if ui
            .add_enabled(created, egui::Button::new("Discard").min_size(size))
            .on_hover_text(format!(
                "Removes the {noun} this project creates. The database is not changed. A \
                 row already in the database cannot be removed: .reload {} never drops an \
                 entry it has read.",
                S::TEMPLATE
            ))
            .on_disabled_hover_text(format!("Only a {noun} this project created can be discarded."))
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject.tool.discard_template(subject.session, shown.entry, now);
                subject.session.status = format!("{} {} discarded", S::TEMPLATE, shown.entry);
            }
        }
    });
}

/// The middle: what the project changes, then the open row's columns.
fn form<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>) {
    egui::ScrollArea::vertical().id_salt((S::TEMPLATE, "data-form")).auto_shrink([false, false]).show(ui, |ui| {
        page_spacing(ui);
        let standing = subject.standings.of(
            crate::server::sync::Half::Row(S::STACK),
            subject.session,
            subject.assets,
            subject.now,
        );
        listing::edits_block(ui, standing, listing::TABLE_LIVE, None, subject.server_panel);
        let Some(shown) = subject.tool.listed.subject(&subject.session.rows.edits) else {
            listing::nothing_open(ui, &subject.tool.listed, S::WORDS.noun);
            return;
        };
        listing::form_head(ui, &shown, S::TEMPLATE);
        ui.horizontal(|ui| {
            crate::ui::tools::spawns::spawn::entry_label::<S>(ui, &shown);
            let id = ui.make_persistent_id((S::TEMPLATE, "data-entry", shown.patch));
            crate::ui::tools::spawns::spawn::entry_field(ui, subject, &shown, id);
        });
        ui.add_space(6.0);
        let row = subject.tool.template.as_ref().filter(|held| held.entry == shown.entry).map(|held| held.row.clone());
        let Some(row) = row else {
            theme::waiting(ui, "reading the row\u{2026}");
            return;
        };
        S::template_form(ui, subject, &shown.key(), &row);
    });
}

/// The third part, in the shell's inspector: the open template's model, a
/// summary, Place in World, and the windows for what the template has besides
/// its row.
pub(crate) fn inspector<S: SpawnView>(ui: &mut egui::Ui, mut subject: Subject<'_, S>) {
    let subject = &mut subject;
    let Some(known) = subject.tool.listed.open_row(&subject.session.rows.edits) else {
        theme::note(ui, format!("Open a {} on the left to see its model and its other tables.", S::WORDS.noun));
        return;
    };
    ui.label(egui::RichText::new(known.label()).strong().size(14.0));
    // As wide as the inspector, as the item workspace's preview is.
    let side = ui.available_width();
    if !S::preview(ui, subject, known.display_id(), Some(side)) {
        theme::note(ui, S::TEXT.no_display);
    }
    ui.add_space(4.0);
    S::list_summary(ui, subject, &known);
    ui.add_space(6.0);
    if ui.button("Place in World").on_hover_text(S::TEXT.place).clicked() {
        subject.tool.mode = crate::tools::placements::place::Mode::Place;
        subject.tool.new_spawn.chosen = Some(known.clone());
        subject.asks.push(crate::ui::shell::asks::ShellAsk::Link(S::TOOL));
    }
    ui.add_space(6.0);
    theme::heading(ui, "Other tables");
    S::list_windows(ui, subject, &known);
}

/// A workspace's inspector panel, its third part: the open template's model,
/// a summary, and the windows for its other tables.
pub(crate) fn inspect<S: SpawnView>(
    ui: &mut egui::Ui,
    subject: crate::ui::tools::Subject<'_>,
    editing: &mut crate::ui::Editing<'_>,
) {
    let crate::ui::tools::Subject { session, assets, now, server, server_panel, standings, .. } = subject;
    inspector(
        ui,
        S::subject(
            session,
            editing,
            crate::ui::tools::spawns::Server { settings: server, panel: server_panel, standings },
            assets,
            now,
            false,
        ),
    );
}

/// A workspace in the middle of a window, over the spawn tool's own state.
pub(crate) fn workspace<S: SpawnView>(
    ui: &mut egui::Ui,
    _: crate::tools::Tool,
    bench: crate::ui::workspace::Bench<'_, '_, '_>,
) {
    let crate::ui::workspace::Bench { session, editing, playing, popovers, assets, now, .. } = bench;
    draw(ui, S::subject(session, editing, crate::ui::workspace::server(playing, popovers), assets, now, false));
}
