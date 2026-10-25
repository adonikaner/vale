//! A column that names a row of another table, drawn after its number: the
//! button that chooses one by name, and the name the number resolves to,
//! as a link to where that row is edited.
//!
//! Every server-row form draws a `Kind::Ref` column through [`cell`], and a
//! `Kind::RefList` column (a creature's `auras`) through [`list_cell`], by way
//! of `crate::ui::widgets::rowform::server_field`. The quest form's signed
//! references, whose sign picks the table, draw their name through [`name`].
//! The picker is [`crate::ui::subjects::quests::picker`], and the tables it can
//! search are [`crate::ui::subjects::quests::target_of`].
//!
//! ## What a name resolves through
//!
//! ```text
//! item_template          the quest tool's name cache, read from the database
//! creature_template      the same cache, by holder
//! gameobject_template    the same cache, by holder
//! quest_template         the quest list, which is in memory
//! a list table           what the id holds and who uses it, read from the
//!                        database into a third cache; see `vale_mangos::lists`
//! FactionTemplate        Faction.dbc, through the row's faction field
//! any other DBC          the table, opened through the session, and its name
//!                        field from `crate::ui::subjects::quests::name_field`
//! ```
//!
//! A table none of those cover draws the number alone, which is what the
//! column is. `Lock` is such a table: the game object form draws what a lock
//! needs beside the number itself, out of `Lock.dbc`.
//!
//! ## What a click opens
//!
//! A DBC row opens the table browser, a quest the quest workspace and an item
//! the item workspace, each through a [`ShellAsk`] the shell carries out after
//! everything is drawn. A creature or a game object is named and not linked:
//! the tool that edits one is the tool this form is already in, and its
//! selection is a spawn, not a template. A display id opens its display
//! workspace through [`display_link`].

use bevy_egui::egui;

use crate::ui::shell::asks::{ShellAsk, ShellAsks};
use crate::ui::subjects::quests::{name_in, open_for_names, target_of};
use super::rowform::{meaning, FORM_ROW, FORM_VALUE};
use super::theme;
use crate::session::EditSession;
use crate::subjects::quests::{ColumnTarget, Holder, PickFor, Picker, Quests, Target};
use vale_client::assets::GameAssets;

/// What resolving a name needs.
pub(crate) struct Resolver<'a> {
    pub(crate) session: &'a mut EditSession,
    pub(crate) assets: &'a GameAssets,
    pub(crate) quests: &'a mut Quests,
    /// Where a clicked name asks the shell to open its row.
    pub(crate) asks: &'a mut ShellAsks,
}

/// The `choose…` button that opens a dialog of rows to choose one from by
/// looking at them or searching them. `about` says what the dialog lists.
/// `true` when it was pressed.
pub(crate) fn choose_button(ui: &mut egui::Ui, about: &str) -> bool {
    ui.small_button("choose\u{2026}").on_hover_text(about).clicked()
}

/// The `…` button after a reference's number, which opens the reference
/// picker to choose the row by name. `about` is its hover text. `true` when
/// it was pressed.
pub(crate) fn by_name_button(ui: &mut egui::Ui, about: &str) -> bool {
    ui.add(
        egui::Button::new(egui::RichText::new("\u{2026}").size(13.0))
            .min_size(egui::vec2(24.0, FORM_ROW)),
    )
    .on_hover_text(about)
    .clicked()
}

/// The picker button and the resolved name, after a value cell the caller
/// has drawn. `target` is where the picker writes; `None` draws no button,
/// for a column that is read and not edited.
///
/// The name goes beside the number when the row has room for it. When it
/// does not, which is the 300-point inspector's case, the id is answered
/// instead and the caller draws it on a row of its own through [`name`].
pub(crate) fn cell(
    ui: &mut egui::Ui,
    resolver: &mut Resolver<'_>,
    table: &'static str,
    showing: &str,
    target: Option<ColumnTarget>,
) -> Option<u32> {
    if let (Some(kind), Some(target)) = (target_of(table), target) {
        if by_name_button(ui, &format!("Choose {} by name.", target.column)) {
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

/// A list column (`Kind::RefList`): one line per id, with the id, a button
/// that removes it and the name it resolves to, then a button that adds one
/// through the reference picker. `target` is where both write; `None` draws
/// the lines alone, for a column that is read and not edited.
///
/// Answers the literal a removal wrote. An addition is written by the picker
/// when a row is chosen in it; see [`ColumnTarget::add_to_list`]. A word of
/// the text that is not an id is listed as it is and can be removed.
pub(crate) fn list_cell(
    ui: &mut egui::Ui,
    resolver: &mut Resolver<'_>,
    table: &'static str,
    showing: &str,
    target: Option<ColumnTarget>,
) -> Option<String> {
    let text = vale_mangos::sql::unquote(showing);
    let items = crate::subjects::quests::list_items(&text);
    let mut written = None;
    ui.vertical(|ui| {
        if items.is_empty() {
            ui.add_sized(
                egui::vec2(FORM_VALUE, FORM_ROW),
                egui::Label::new(egui::RichText::new("none").small().color(theme::INK_FAINT)),
            );
        }
        for (at, item) in items.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized(egui::vec2(FORM_VALUE, FORM_ROW), egui::Label::new(theme::number(*item)));
                if target.is_some() && ui.small_button("x").on_hover_text(format!("Remove {item}.")).clicked() {
                    let mut kept = items.clone();
                    kept.remove(at);
                    written = Some(vale_mangos::sql::text(&kept.join(" ")));
                }
                match item.parse::<u32>() {
                    Ok(id) => name(ui, resolver, table, id),
                    Err(_) => {
                        ui.label(egui::RichText::new("not an id").small().color(theme::BAD));
                    }
                }
            });
        }
        if let (Some(kind), Some(target)) = (target_of(table), target) {
            let about = format!("Add a {table} row to {}, chosen by name or id.", target.column);
            if ui.small_button("add\u{2026}").on_hover_text(about).clicked() {
                resolver.quests.picker = Some(Picker::new(kind, PickFor::ListAdd(target)));
            }
        }
    });
    written
}

/// How much of a row a name needs beside the number, in points. Below this
/// the name goes on the next row.
pub(crate) const NAME_ROOM: f32 = 140.0;

/// The name a reference resolves to, with its link.
pub(crate) fn name(ui: &mut egui::Ui, resolver: &mut Resolver<'_>, table: &'static str, id: u32) {
    match target_of(table) {
        Some(Target::Item) => item_name(ui, resolver, id),
        Some(Target::Creature) => holder_name(ui, resolver, Holder::Creature, id),
        Some(Target::Object) => holder_name(ui, resolver, Holder::Object, id),
        Some(Target::Quest) => quest_link(ui, resolver, id),
        Some(Target::Dbc(dbc)) => dbc_link(ui, resolver, dbc, id),
        Some(Target::List(list)) => list_line(ui, resolver, list, id),
        None => meaning(ui, format!("{table} {id}")),
    }
}

/// What one id of a server list holds, as the picker lists it: the title,
/// with the count and who uses it on the hover. See `vale_mangos::lists`.
fn list_line(ui: &mut egui::Ui, resolver: &mut Resolver<'_>, table: &'static str, id: u32) {
    match resolver.quests.listed(table, id, &resolver.session.rows.edits) {
        Some(found) => {
            ui.add(
                egui::Label::new(egui::RichText::new(&found.title).color(theme::INK)).truncate(),
            )
            .on_hover_text(format!("{}\n{}", found.title, found.sub));
        }
        None => match resolver.quests.listed_known(table, id) {
            true => {
                let noun = vale_mangos::lists::list(table).map_or(table, |list| list.noun);
                ui.label(
                    egui::RichText::new(format!("no such {noun}"))
                        .small()
                        .color(theme::BAD),
                );
            }
            false => meaning(ui, "\u{2026}"),
        },
    }
}

/// An item's name in its quality's colour, as a link that opens the item
/// workspace on it.
fn item_name(ui: &mut egui::Ui, resolver: &mut Resolver<'_>, entry: u32) {
    let Some(found) = resolver.quests.item(entry, &resolver.session.rows.edits) else {
        match resolver.quests.item_known(entry, &resolver.session.rows.edits) {
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
    let colour = crate::ui::subjects::items::quality_colour(found.quality);
    if link(ui, &found.name, colour, &format!("Open item {entry} in the item workspace.")) {
        resolver.asks.push(ShellAsk::ShowItem(entry));
    }
}

/// A creature's or a game object's name, out of the quest tool's cache.
fn holder_name(ui: &mut egui::Ui, resolver: &mut Resolver<'_>, holder: Holder, id: u32) {
    match resolver.quests.holder(holder, id, &resolver.session.rows.edits) {
        Some(name) => {
            ui.label(egui::RichText::new(name).color(theme::INK));
        }
        None => match resolver.quests.holder_known(holder, id, &resolver.session.rows.edits) {
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
        .title_of(entry, &resolver.session.rows.edits)
    {
        Some(title) => {
            let hover = format!("Open quest {entry} in the quest workspace.");
            if link(ui, &title, theme::ACCENT, &hover) {
                resolver.asks.push(ShellAsk::ShowQuest(entry));
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
        resolver.asks.push(ShellAsk::ShowRow(table, id));
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

/// A display row, named by its id or by what it draws, as a link that opens
/// the row in its display workspace through a [`ShellAsk::ShowRow`]. A
/// display row has no name of its own, so the caller chooses the text.
/// `Tool::for_table` decides the workspace.
pub(crate) fn display_link(
    ui: &mut egui::Ui,
    text: egui::RichText,
    asks: &mut ShellAsks,
    table: &'static str,
    id: u32,
) {
    let workspace = crate::tools::Tool::for_table(table).name();
    if link_text(ui, text, theme::ACCENT, &format!("Open {table} {id} in {workspace}.")) {
        asks.push(ShellAsk::ShowRow(table, id));
    }
}

/// A name that opens something: drawn in `colour`, underlined while the
/// pointer is over it. `true` when it was clicked.
pub(crate) fn link(ui: &mut egui::Ui, text: &str, colour: egui::Color32, hover: &str) -> bool {
    link_text(ui, egui::RichText::new(text), colour, hover)
}

/// [`link`] for text with its own size.
fn link_text(ui: &mut egui::Ui, text: egui::RichText, colour: egui::Color32, hover: &str) -> bool {
    let name = ui.add(
        egui::Label::new(text.color(colour))
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
