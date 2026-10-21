//! What the Creatures and Game Objects workspaces draw the same way: the
//! docked list on the left with its heading and search box, the block that
//! says what the project changes, and the head of the form.
//!
//! The workspaces themselves are [`super::creature_data`] and
//! [`super::gameobject_data`]; the list they draw is
//! [`crate::tools::listing::Listing`]. Nothing here names either subject.

use bevy_egui::egui;

use super::theme;
use crate::tools::creatures::TemplateSubject;
use crate::tools::listing::{Listed, Listing};
use vale_mangos::row::Life;

/// How wide the list is at first. The item list's width, for the same rows.
const LIST_WIDTH: f32 = 320.0;

/// The gap between the list and the form, and above both; `ui::items` gives
/// the reasons.
const FORM_GAP: f32 = 12.0;
const TOP_GAP: f32 = 10.0;

/// The height of one list row, the editor's own.
pub const ROW_HEIGHT: f32 = theme::LIST_ROW;

/// Paint the workspace's ground, dock the list on the left and draw the form
/// in what is left. The list and the form both take the subject, so each is a
/// function over it rather than a closure.
pub fn frame<S>(
    ui: &mut egui::Ui,
    subject: &mut S,
    id: &'static str,
    list: fn(&mut egui::Ui, &mut S),
    form: fn(&mut egui::Ui, &mut S),
) {
    // The world is drawn behind the workspace, so a strip left unpainted
    // would show it through.
    let all = ui.available_rect_before_wrap();
    ui.painter().rect_filled(all, 0.0, theme::SHELL);
    egui::Panel::left(id)
        .default_size(LIST_WIDTH)
        .min_size(240.0)
        .max_size(520.0)
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(8.0))
        .show(ui, |ui| {
            ui.add_space(TOP_GAP - 8.0);
            list(ui, subject);
        });
    let rect = ui.available_rect_before_wrap();
    let inset = egui::Rect::from_min_max(rect.min + egui::vec2(FORM_GAP, TOP_GAP), rect.max);
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(inset).layout(*ui.layout()));
    form(&mut inner, subject);
}

/// The list's heading: the workspace's name, the row count, and why there is
/// nothing when there is nothing. Answers whether the list can be drawn.
pub fn head<K: Listed>(ui: &mut egui::Ui, name: &str, list: &Listing<K>) -> bool {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(name).strong().size(15.0).color(theme::INK));
        ui.label(theme::number(format!("{} rows", list.count())));
    })
    .response
    .on_hover_text(format!(
        "{} as the server loads it: one row per entry, at the highest content patch \
         at or below the server's patch.",
        K::TABLE
    ));
    if let Some(trouble) = list.trouble.clone() {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
        theme::note(
            ui,
            "These are rows in the vmangos world database. Set the database connection \
             in Server\u{2026} on the top bar, or with VALE_MANGOSD.",
        );
        return false;
    }
    if !list.is_read() {
        theme::waiting(ui, format!("reading {}\u{2026}", K::TABLE));
        return false;
    }
    true
}

/// The search box and the count under it. Answers the matching rows.
pub fn search<K: Listed>(
    ui: &mut egui::Ui,
    list: &mut Listing<K>,
    hint: &str,
    edits: &vale_mangos::row::Edits,
    revision: u64,
) -> Vec<usize> {
    ui.add_space(4.0);
    let width = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(&mut list.query)
            .hint_text(hint)
            .desired_width(width),
    );
    let matches = list.matches(edits, revision).to_vec();
    ui.label(
        egui::RichText::new(format!("{} shown", matches.len()))
            .small()
            .color(theme::INK_FAINT),
    );
    ui.add_space(4.0);
    matches
}

/// The index of the open row among the matches, when the list should scroll
/// to it: it was opened from somewhere other than this list.
pub fn reveal<K: Listed>(list: &mut Listing<K>, matches: &[usize]) -> Option<usize> {
    if list.open == list.revealed {
        return None;
    }
    let Some(entry) = list.open else {
        list.revealed = None;
        return None;
    };
    let found = matches
        .iter()
        .position(|&index| list.at(index).is_some_and(|row| row.entry() == entry));
    if found.is_some() {
        list.revealed = Some(entry);
    }
    found
}

/// What the project says of a row, as the end of its second line.
pub fn claim_word(claim: Life) -> &'static str {
    match claim {
        Life::Insert => " \u{b7} new",
        _ => "",
    }
}

/// A row's right-click menu: the entry to the clipboard.
pub fn row_menu(
    response: &egui::Response,
    session: &mut crate::session::EditSession,
    table: &str,
    entry: u32,
) {
    response.context_menu(|ui| {
        if ui.button(format!("Copy entry {entry}")).clicked() {
            ui.ctx().copy_text(entry.to_string());
            session.status = format!("{table} entry {entry} is on the clipboard");
            ui.close();
        }
    });
}

/// What the project changes in one subject's rows, and the way to the panel
/// that applies it. Nothing is drawn when the project changes nothing.
pub fn edits_block(
    ui: &mut egui::Ui,
    half: super::sync::Half,
    session: &crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    server_panel: &mut super::popover::Popover,
) {
    let standing = super::sync::standing(half, session, assets);
    if standing.quiet() {
        return;
    }
    theme::heading(ui, "This project");
    let colour = match standing.outstanding {
        0 => theme::INK_DIM,
        _ => theme::WARN,
    };
    ui.label(egui::RichText::new(standing.line()).color(colour));
    for refused in &standing.refused {
        ui.label(egui::RichText::new(refused).small().color(theme::BAD));
    }
    ui.add_space(4.0);
    if ui
        .button("Server\u{2026}")
        .on_hover_text(
            "Opens the Server panel, which applies, restores or discards these rows. The \
             server reads this table at startup, so restart it after Apply.",
        )
        .clicked()
    {
        server_panel.show();
    }
    ui.add_space(6.0);
}

/// The head of the form: the row's name, and what this project says of it or
/// how far an edit reaches. The template window's head without the count of
/// spawns on the map, which a workspace has not read.
pub fn form_head(ui: &mut egui::Ui, shown: &TemplateSubject, table: &str) {
    ui.label(
        egui::RichText::new(&shown.label)
            .strong()
            .size(17.0)
            .color(theme::INK),
    );
    let (line, colour) = match shown.claim {
        Life::Insert => (
            format!(
                "{table} {} \u{b7} new: not in the database yet. Apply writes it; restart the \
                 server to load it.",
                shown.entry
            ),
            theme::ACCENT,
        ),
        _ => (
            format!(
                "{table} {} at content patch {} \u{b7} edits change every spawn of this \
                 entry, on every map",
                shown.entry, shown.patch
            ),
            theme::WARN,
        ),
    };
    ui.label(egui::RichText::new(line).small().color(colour));
}

/// The waiting line or the empty form, for a form with no open row. `noun` is
/// what a row is called.
pub fn nothing_open<K: Listed>(ui: &mut egui::Ui, list: &Listing<K>, noun: &str) {
    if list.open.is_some() && !list.is_read() {
        theme::waiting(ui, format!("reading {}\u{2026}", K::TABLE));
        return;
    }
    theme::heading(ui, &format!("No {noun} open"));
    theme::note(
        ui,
        format!(
            "Choose a {noun} on the left, or create one with + New. Every column of {} is \
             edited here; nothing is written to the database until Apply, in Server\u{2026} \
             on the top bar.",
            K::TABLE
        ),
    );
}
