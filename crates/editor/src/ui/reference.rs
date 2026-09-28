//! A column that names a row of another table, drawn after its number: the
//! button that chooses one by name, and the name the number resolves to,
//! as a link to where that row is edited.
//!
//! The creature and game object forms draw every `Kind::Ref` column through
//! [`cell`]. The quest and item forms keep their own arrangement of the same
//! parts (`super::quests::resolved`, `super::items::reference_field`),
//! because each resolves some tables from a list it already holds in memory.
//! What is shared is the picker, [`super::quests::picker`], and the tables
//! it can search, [`super::quests::target_of`].
//!
//! ## What a name resolves through
//!
//! ```text
//! item_template          the quest tool's name cache, read from the database
//! creature_template      the same cache, by holder
//! gameobject_template    the same cache, by holder
//! quest_template         the quest list, which is in memory
//! FactionTemplate        Faction.dbc, through the row's faction field
//! any other DBC          the table, opened through the session, and its name
//!                        field from `super::quests::name_field`
//! ```
//!
//! A table none of those cover draws the number alone, which is what the
//! column is. `Lock` is such a table: the game object form draws what a lock
//! needs beside the number itself, out of `Lock.dbc`.
//!
//! ## What a click opens
//!
//! A DBC row opens the table browser and a quest opens the quest workspace,
//! through the two mailboxes on `crate::tools::quests::Quests` that the shell
//! answers after everything is drawn. An item opens the item workspace
//! through `Quests::show_item`. A creature or a game object is named and not
//! linked: the tool that edits one is the tool this form is already in, and
//! its selection is a spawn, not a template.

use bevy_egui::egui;

use super::quests::{name_in, open_for_names, target_of};
use super::rowform::{meaning, FORM_ROW};
use super::theme;
use crate::session::EditSession;
use crate::tools::quests::{ColumnTarget, Holder, PickFor, Picker, Quests, Target};
use vale_client::assets::GameAssets;

/// What resolving a name needs.
pub struct Resolver<'a> {
    pub session: &'a mut EditSession,
    pub assets: &'a GameAssets,
    pub quests: &'a mut Quests,
}

/// The picker button and the resolved name, after a value cell the caller
/// has drawn. `target` is where the picker writes; `None` draws no button,
/// for a column that is read and not edited.
///
/// The name goes beside the number when the row has room for it. When it
/// does not, which is the 300-point inspector's case, the id is answered
/// instead and the caller draws it on a row of its own through [`name`].
pub fn cell(
    ui: &mut egui::Ui,
    resolver: &mut Resolver<'_>,
    table: &'static str,
    showing: &str,
    target: Option<ColumnTarget>,
) -> Option<u32> {
    if let (Some(kind), Some(target)) = (target_of(table), target) {
        if ui
            .add(
                egui::Button::new(egui::RichText::new("\u{2026}").size(13.0))
                    .min_size(egui::vec2(24.0, FORM_ROW)),
            )
            .on_hover_text(format!("choose {} by name", target.column))
            .clicked()
        {
            resolver.quests.picker = Some(Picker::new(kind, PickFor::ServerColumn(target)));
        }
    }
    let id: u32 = showing.trim().parse().unwrap_or(0);
    if id == 0 {
        ui.label(egui::RichText::new("none").small().color(theme::INK_FAINT));
        return None;
    }
    if ui.available_width() < NAME_ROOM {
        return Some(id);
    }
    name(ui, resolver, table, id);
    None
}

/// How much of a row a name needs beside the number, in points. Below this
/// the name goes on the next row.
pub const NAME_ROOM: f32 = 140.0;

/// The name a reference resolves to, with its link.
pub fn name(ui: &mut egui::Ui, resolver: &mut Resolver<'_>, table: &'static str, id: u32) {
    match target_of(table) {
        Some(Target::Item) => item_name(ui, resolver, id),
        Some(Target::Creature) => holder_name(ui, resolver, Holder::Creature, id),
        Some(Target::Object) => holder_name(ui, resolver, Holder::Object, id),
        Some(Target::Quest) => quest_link(ui, resolver, id),
        Some(Target::Dbc(dbc)) => dbc_link(ui, resolver, dbc, id),
        None => meaning(ui, format!("{table} {id}")),
    }
}

/// An item's name in its quality's colour, as a link that opens the item
/// workspace on it.
fn item_name(ui: &mut egui::Ui, resolver: &mut Resolver<'_>, entry: u32) {
    let Some(found) = resolver.quests.item(entry, &resolver.session.server_edits) else {
        match resolver.quests.item_known(entry, &resolver.session.server_edits) {
            true => {
                ui.label(
                    egui::RichText::new("no such item")
                        .small()
                        .color(theme::BAD),
                );
            }
            false => meaning(ui, "\u{2026}"),
        }
        return;
    };
    let colour = super::items::quality_colour(found.quality);
    if link(ui, &found.name, colour, &format!("Open item {entry}.")) {
        resolver.quests.show_item = Some(entry);
    }
}

/// A creature's or a game object's name, out of the quest tool's cache.
fn holder_name(ui: &mut egui::Ui, resolver: &mut Resolver<'_>, holder: Holder, id: u32) {
    match resolver.quests.holder(holder, id, &resolver.session.server_edits) {
        Some(name) => {
            ui.label(egui::RichText::new(name).color(theme::INK));
        }
        None => match resolver.quests.holder_known(holder, id, &resolver.session.server_edits) {
            true => {
                ui.label(
                    egui::RichText::new(format!("no such {}", holder.word()))
                        .small()
                        .color(theme::BAD),
                );
            }
            false => meaning(ui, "\u{2026}"),
        },
    }
}

/// A quest's title, as a link that opens the quest workspace on it.
fn quest_link(ui: &mut egui::Ui, resolver: &mut Resolver<'_>, entry: u32) {
    match resolver
        .quests
        .title_of(entry, &resolver.session.server_edits)
    {
        Some(title) => {
            let hover = format!("Open quest {entry} in the quest workspace.");
            if link(ui, &title, theme::ACCENT, &hover) {
                resolver.quests.show_quest = Some(entry);
            }
        }
        None if resolver.quests.all.is_empty() => meaning(ui, "\u{2026}"),
        None => {
            ui.label(
                egui::RichText::new("no such quest")
                    .small()
                    .color(theme::BAD),
            );
        }
    }
}

/// A client table's row by name, as a link that opens the table browser on
/// it. The table is opened through the session the first time it is asked
/// for.
fn dbc_link(ui: &mut egui::Ui, resolver: &mut Resolver<'_>, table: &'static str, id: u32) {
    if !open_for_names(resolver.session, resolver.assets, table) {
        meaning(ui, format!("{table}\u{2026}"));
        return;
    }
    let Some(name) = name_in(resolver.session, table, id) else {
        ui.label(
            egui::RichText::new(format!("not a row of {table}"))
                .small()
                .color(theme::BAD),
        );
        return;
    };
    let hover = format!("Open {table} {id} in the table browser.");
    if link(ui, &name, theme::ACCENT, &hover) {
        resolver.quests.show_row = Some((table, id));
    }
    // A spell's rank, which is what tells nine Fireballs apart: field 129,
    // the one the picker lists under the name.
    if table == "Spell" {
        let rank = resolver
            .session
            .table(table)
            .and_then(|open| open.row_of(id).and_then(|record| open.string_at(record, 129)))
            .filter(|rank| !rank.is_empty());
        if let Some(rank) = rank {
            ui.label(egui::RichText::new(rank).small().color(theme::INK_FAINT));
        }
    }
}

/// A name that opens something: drawn in `colour`, underlined while the
/// pointer is over it. `true` when it was clicked.
pub fn link(ui: &mut egui::Ui, text: &str, colour: egui::Color32, hover: &str) -> bool {
    let name = ui.add(
        egui::Label::new(egui::RichText::new(text).color(colour))
            .truncate()
            .sense(egui::Sense::click()),
    );
    if name.hovered() {
        let under = name.rect.bottom() - 1.0;
        ui.painter()
            .hline(name.rect.x_range(), under, egui::Stroke::new(1.0, colour));
    }
    name.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(hover)
        .clicked()
}
