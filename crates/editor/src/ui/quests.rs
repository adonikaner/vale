//! The workspace a quest is edited in: the list, the form, and the quest as the
//! game would show it with who gives and takes it.
//!
//! ## Layout: the list, the form and the inspector
//!
//! ```text
//! left       which quest          the table, searched by title, entry or
//!                                 heading, or narrowed to one creature's quests
//! middle     what it is           131 columns in rowform's page layout
//! inspector  what a player sees   the log entry composed from the row, then
//!                                 the givers, the takers and the chain
//! ```
//!
//! The layout is [`super::items`]' with a different third part. The item
//! workspace's third part shows the item's appearance, which its row holds
//! only as an id. A quest's row is mostly text and ids, so the third part shows
//! what they resolve to: the sentence the log shows for
//! `ReqCreatureOrGOId2 = -1627`, the reward list, and two facts the row does
//! not hold at all, who hands the quest out and who takes it back.
//!
//! The third part is drawn in the shell's inspector ([`inspector`]), not in a
//! column of its own. The shell's rule is that the inspector describes the
//! selection; the spell workspace uses the same arrangement, with an inspector
//! that shows what the open row points at. A separate column beside the
//! inspector left the form 250 points wide in a 1280-point window, which is
//! narrower than one page row.
//!
//! ## Objectives and rewards are lines, not numbered columns
//!
//! `quest_template` spreads one objective over four columns a hundred apart:
//! `ReqCreatureOrGOId3`, `ReqCreatureOrGOCount3`, `ReqSpellCast3` and
//! `ObjectiveText3`. The form draws them as one line per objective — the id
//! with its picker, the count, and what the id resolves to — and draws the used
//! lines and one free one, so a quest with one kill objective is two lines and
//! not sixteen rows of zeros. [`pair_row`] is that line and it is used for every
//! id-and-number pair the table has: required items, source items, both kinds
//! of reward item, the five reputation rewards, the three reputation
//! conditions, the skill and the emotes.
//!
//! Every column is still on the form, under its own name in the hover text, and
//! every one is still a column edit on the undo stack.
//!
//! ## Every id is resolved, and every id has a picker
//!
//! A quest names items, creatures, game objects, other quests, spells,
//! factions, areas, skills and emotes. The first four are rows in the database
//! and are named through [`crate::tools::quests`]' batched lookups; the rest
//! are client tables opened through the session as the spell workspace opens
//! them. The `…` beside each opens [`picker`], one dialog over all of them,
//! with a switch for the columns whose sign picks the table.
//!
//! ## The creature and game-object tools' window is here
//!
//! [`holder_window`] lists the quests one creature or game object gives and
//! takes, adds and removes them, and opens any of them in the workspace. It
//! reads the same relation list from the other direction, so it is in this file
//! rather than in [`super::creatures`] and [`super::gameobjects`]. The
//! Quests button on a selected creature or object opens it.
//!
//! ## Nothing here reaches the database
//!
//! An edit goes into the project's store and onto the undo stack; a save writes
//! `sql\quests.sql`; Apply is on the bar's Server… with the other
//! subjects' — see [`super::sync`] and [`crate::server::quests`].

use super::rowform::{
    choice_cell, draft_or, finished, flags_cell, meaning, number_cell, number_means, page_row,
    page_spacing, paragraph_cell, revert_button, section, text_cell, unquote, RowAct, FORM_ROW,
};
use super::theme;
use super::thumbnails::Thumbnails;
use crate::session::EditSession;
use crate::tools::items::Items;
use crate::tools::quests::{
    Hit, Holder, Known, PickFor, Picker, Picture, Quests, Relation, Role, Target, PICK_LIMIT,
};
use vale_edit::dbc::DbcFile;
use crate::tools::Tool;
use vale_client::assets::GameAssets;
use vale_mangos::quest::{self, Column, Group, Kind};
use vale_mangos::row::Life;
use bevy_egui::egui;

/// How wide the quest list is.
const LIST_WIDTH: f32 = 320.0;

/// The gaps `super::items` draws its three columns with, for the same reasons.
const FORM_GAP: f32 = 12.0;
const TOP_GAP: f32 = 10.0;

/// How wide the second cell of a pair is: a count, a reputation value, a delay.
const COUNT: f32 = 64.0;

/// How big an item's icon is drawn beside its name.
const ICON: f32 = 18.0;

/// What every part of the workspace reads: the project, the quests, and what a
/// resolved id is drawn with.
pub struct Workspace<'a> {
    pub session: &'a mut EditSession,
    pub quests: &'a mut Quests,
    /// The item workspace's state, used only to look up a display id's icon,
    /// which it already memoises — see `crate::tools::items::Items::look`.
    pub items: &'a mut Items,
    pub assets: &'a GameAssets,
    pub thumbnails: &'a mut Thumbnails,
    /// The frame clock the undo stack folds a gesture by.
    pub now: f64,
}

/// What only the list and the head of the form need, which belongs to the
/// shell. The inspector draws its part without any of it.
pub struct Shell<'a> {
    /// The queue an apply's reloads go on, whose status the form shows.
    pub reloads: &'a mut crate::server::reload::Reloads,
    /// The bar's Server…, which is where every server operation is.
    pub server_panel: &'a mut super::popover::Popover,
    /// Which content patch the server is configured for — half the key a new
    /// row is written under.
    pub patch: u32,
    /// A tool to switch to, which the list's "back to Creatures" asks for.
    /// Answered rather than written, because the shell holds the tool while it
    /// draws this.
    pub switch_to: &'a mut Option<Tool>,
}

/// The open quest and its row, read once a frame and handed to everything that
/// draws it.
pub(super) struct Open {
    known: Known,
    row: quest::Row,
}

/// What one column shows: this project's value where it has one and the
/// database's otherwise.
struct Shown {
    showing: String,
    in_database: Option<String>,
    edited: bool,
}

fn shown(session: &EditSession, open: &Open, column: &Column) -> Shown {
    let edited = session
        .server_edits
        .get(quest::TEMPLATE, &open.known.key(), column.name)
        .map(str::to_string);
    let in_database = column.literal(&open.row);
    Shown {
        // A `NULL` is drawn as an empty value, not as the word. The six text
        // columns default to `NULL` in the DDL and about a fifth of the shipped
        // rows leave `EndText` that way; a box showing `NULL` would make the
        // user delete the word before typing.
        showing: edited
            .clone()
            .or_else(|| in_database.clone())
            .unwrap_or_else(|| match column.kind.is_text() {
                true => "''".to_string(),
                false => "0".to_string(),
            }),
        in_database,
        edited: edited.is_some(),
    }
}

/// The value of a column by name, as a number.
fn number_of(session: &EditSession, open: &Open, name: &str) -> i64 {
    quest::column(quest::TEMPLATE, name)
        .map(|column| shown(session, open, column).showing)
        .and_then(|showing| showing.trim().parse().ok())
        .unwrap_or(0)
}

/// The value of a column by name, as unquoted text.
fn text_of(session: &EditSession, open: &Open, name: &str) -> String {
    quest::column(quest::TEMPLATE, name)
        .map(|column| unquote(&shown(session, open, column).showing))
        .unwrap_or_default()
}

/// Write one column of the open quest.
///
/// Cleared rather than written when it matches what the database holds, so a
/// project does not claim to change a column back to the value it has.
fn commit(session: &mut EditSession, open: &Open, column: &Column, written: String, now: f64) {
    let in_database = column.literal(&open.row);
    let value = match Some(&written) == in_database.as_ref() {
        true => None,
        false => Some(written),
    };
    let gesture = format!("quest {} {}", open.known.entry, column.name);
    session.set_server_edit(
        quest::TEMPLATE,
        &open.known.key(),
        column.name,
        value,
        Some(crate::session::Gesture {
            label: "Edit quest",
            subject: &gesture,
            now,
        }),
    );
}

/// Draw the list and the form into the region the panels left.
pub fn draw(ui: &mut egui::Ui, mut work: Workspace<'_>, mut shell: Shell<'_>) {
    // Fill the region with the shell colour. The world is still drawn behind
    // the workspace, and an unpainted strip would show it through; see the
    // same note in `ui::data`.
    let all = ui.available_rect_before_wrap();
    ui.painter().rect_filled(all, 0.0, theme::SHELL);

    // The headings, named once both client tables are open.
    name_the_zones(&mut work);

    let open = open_now(&work);

    egui::Panel::left("quest-browser")
        .default_size(LIST_WIDTH)
        .min_size(240.0)
        .max_size(520.0)
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(8.0))
        .show(ui, |ui| {
            ui.add_space(TOP_GAP - 8.0);
            list(ui, &mut work, &mut shell);
        });

    let rect = ui.available_rect_before_wrap();
    let inset = egui::Rect::from_min_max(rect.min + egui::vec2(FORM_GAP, TOP_GAP), rect.max);
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(inset).layout(*ui.layout()));
    form(&mut inner, &mut work, &mut shell, open.as_ref());

    picker(
        ui.ctx(),
        work.session,
        work.quests,
        work.assets,
        work.thumbnails,
        open.as_ref(),
        work.now,
    );
}

/// The open quest with the project's edits over it, and its whole row — or
/// `None` while nothing is open or the row is still being read.
fn open_now(work: &Workspace<'_>) -> Option<Open> {
    let known = work.quests.open_quest(&work.session.server_edits)?;
    let row = work
        .quests
        .row
        .as_ref()
        .filter(|held| held.entry == known.entry)
        .map(|held| held.row.clone())?;
    Some(Open { known, row })
}

/// Name every quest's heading from `AreaTable.dbc` and `QuestSort.dbc`.
fn name_the_zones(work: &mut Workspace<'_>) {
    let ready = work.session.open_table(work.assets, "AreaTable")
        && work.session.open_table(work.assets, "QuestSort");
    let session = &*work.session;
    work.quests.name_the_zones(|zone_or_sort| {
        if !ready {
            return None;
        }
        let (table, id) = match zone_or_sort > 0 {
            true => ("AreaTable", zone_or_sort as u32),
            false => ("QuestSort", zone_or_sort.unsigned_abs()),
        };
        Some(name_in(session, table, id).unwrap_or_default())
    });
}

// ---------------------------------------------------------------------------
// The client tables a quest names
// ---------------------------------------------------------------------------

/// Which field of a client table holds a row's name.
///
/// Stated per table because there is no rule: measured with `vale dbc
/// <Table>` against the shipped files. `AreaTable` 11 and `QuestSort` 1 are
/// also the two `GetQuestLogTitle` reads, as
/// `vale_assets::tables::questsort` states; `Spell` 120 is the field
/// `vale_mangos::spell` maps to `spell_template.name`.
pub(super) fn name_field(table: &str) -> Option<usize> {
    match table {
        "AreaTable" => Some(11),
        "QuestSort" | "QuestInfo" | "Emotes" | "EmotesText" | "SpellFocusObject" | "Languages" => Some(1),
        "Faction" => Some(19),
        "SkillLine" => Some(3),
        "ItemSet" => Some(1),
        "Map" => Some(4),
        // `CreatureFamily` field 8, the same field
        // `vale_assets::tables::pet` reads the family's name from.
        "CreatureFamily" => Some(8),
        "Spell" => Some(120),
        _ => None,
    }
}

/// The table a name is read from when it is not the referenced table itself.
///
/// `FactionTemplate.dbc` carries no name: field 1 of a row is the `Faction.dbc`
/// id whose name it goes by. `creature_template.faction` and
/// `gameobject_template.faction` both name a `FactionTemplate` row, so a
/// faction is searched and resolved through that hop.
const FACTION_TEMPLATE: &str = "FactionTemplate";
const FACTION_TEMPLATE_FACTION: usize = 1;

/// Open every table a name of `table` is read from. `false` when one is
/// missing.
pub(super) fn open_for_names(session: &mut EditSession, assets: &GameAssets, table: &str) -> bool {
    let opened = session.open_table(assets, table);
    match table {
        FACTION_TEMPLATE => opened && session.open_table(assets, "Faction"),
        _ => opened,
    }
}

/// What one record of an open client table is called. `None` for a record
/// with no name, or a table with no name field.
pub(super) fn record_name(session: &EditSession, table: &str, record: usize) -> Option<String> {
    let open = session.table(table)?;
    if table == FACTION_TEMPLATE {
        let faction = open.u32_at(record, FACTION_TEMPLATE_FACTION)?;
        return name_in(session, "Faction", faction);
    }
    open.string_at(record, name_field(table)?)
        .filter(|name| !name.is_empty())
}

/// What a row of an open client table is called. `None` for a table that is not
/// open, a row it does not hold, or a table with no name field.
pub(super) fn name_in(session: &EditSession, table: &str, id: u32) -> Option<String> {
    let open = session.table(table)?;
    let record = open.row_of(id)?;
    record_name(session, table, record)
}

/// What a column's reference is searched and resolved as, or `None` for a
/// table this panel cannot name a row of, which draws the number alone.
pub(super) fn target_of(table: &'static str) -> Option<Target> {
    match table {
        vale_mangos::item::TEMPLATE => Some(Target::Item),
        vale_mangos::creature::TEMPLATE => Some(Target::Creature),
        "gameobject_template" => Some(Target::Object),
        quest::TEMPLATE => Some(Target::Quest),
        FACTION_TEMPLATE => Some(Target::Dbc(FACTION_TEMPLATE)),
        other => match vale_mangos::lists::list(other) {
            Some(list) => Some(Target::List(list.table)),
            None => name_field(other).map(|_| Target::Dbc(other)),
        },
    }
}

/// What a signed column's two readings are called in the picker's switch.
fn either_words(column: &str) -> (&'static str, &'static str) {
    match column {
        "ZoneOrSort" => ("An area", "A quest sort"),
        "PrevQuestId" | "NextQuestId" => ("Rewarded first", "Active at the time"),
        _ => ("A creature", "A game object"),
    }
}

// ---------------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------------

/// The left column: what there is, what matches, and what makes or unmakes a row.
fn list(ui: &mut egui::Ui, work: &mut Workspace<'_>, shell: &mut Shell<'_>) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Quests")
                .strong()
                .size(15.0)
                .color(theme::INK),
        );
        ui.label(theme::number(format!("{} rows", work.quests.count())));
    })
    .response
    .on_hover_text(
        "quest_template, as the server would load it: one row per quest, at the highest \
         content patch at or below the server's own.",
    );

    if let Some(trouble) = work.quests.trouble.clone() {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
        theme::note(
            ui,
            "These are rows in vmangos' database rather than files. Server\u{2026} on the \
             top bar is where this machine's is set, or VALE_MANGOSD.",
        );
        return;
    }
    if work.quests.all.is_empty() {
        theme::waiting(ui, "reading quest_template\u{2026}");
        return;
    }

    narrowing(ui, work, shell);

    ui.add_space(4.0);
    let width = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(&mut work.quests.query)
            .hint_text("title, entry, zone, or what kind of quest")
            .desired_width(width),
    );
    ui.add_space(4.0);
    row_actions(ui, work, shell);
    ui.add_space(2.0);

    let revision = work.session.server_edit_revision;
    let matches: Vec<usize> = work
        .quests
        .matches(&work.session.server_edits, revision)
        .to_vec();
    ui.label(
        egui::RichText::new(format!("{} shown", matches.len()))
            .small()
            .color(theme::INK_FAINT),
    );
    ui.add_space(4.0);

    let mut asked: Option<(Known, RowAct)> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, theme::LIST_ROW, matches.len(), |ui, range| {
            for at in range {
                let Some(known) = work.quests.shown(matches[at], &work.session.server_edits) else {
                    continue;
                };
                let chosen = work.quests.open == Some(known.entry);
                let response = row(ui, &known, chosen);
                if response.clicked() {
                    asked = Some((known.clone(), RowAct::Open));
                }
                response.context_menu(|ui| {
                    let act = super::rowform::row_menu(ui, "quest", known.entry, known.claim, chosen);
                    if let Some(act) = act {
                        asked = Some((known.clone(), act));
                    }
                });
            }
        });
    if let Some((known, act)) = asked {
        let ctx = ui.ctx().clone();
        act_on(&ctx, work, shell.patch, &known, act);
    }
}

/// Carry out one act on one quest. The buttons over the list and a row's
/// right-click menu both call this, so an act is the same from either.
fn act_on(ctx: &egui::Context, work: &mut Workspace<'_>, patch: u32, known: &Known, act: RowAct) {
    let now = work.now;
    match act {
        RowAct::Open => work.quests.open = Some(known.entry),
        RowAct::Copy => {
            work.quests.duplicate(work.session, patch, now);
        }
        RowAct::Remove => work.quests.remove(work.session, known, now),
        RowAct::Keep => work.quests.keep(work.session, known, now),
        RowAct::CopyEntry => {
            ctx.copy_text(known.entry.to_string());
            work.session.status = format!("quest entry {} is on the clipboard", known.entry);
        }
    }
}

/// Which creature's quests the list is narrowed to, when it is, with the
/// way back to that creature and the way out of the narrowing.
fn narrowing(ui: &mut egui::Ui, work: &mut Workspace<'_>, shell: &mut Shell<'_>) {
    let Some((holder, id)) = work.quests.of_holder else {
        return;
    };
    let name = work
        .quests
        .holder(holder, id, &work.session.server_edits)
        .unwrap_or_else(|| format!("{} {id}", holder.word()));
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(theme::ACCENT_SUNK)
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(format!("Given or taken by {name}"))
                    .color(theme::INK)
                    .size(12.5),
            )
            .on_hover_text(format!(
                "{} template entry {id}. The list is the quests it gives or takes, this \
                 project's relations included.",
                holder.word()
            ));
            ui.horizontal(|ui| {
                if ui
                    .small_button("show every quest")
                    .on_hover_text("Stop narrowing the list to one creature's quests.")
                    .clicked()
                {
                    work.quests.of_holder = None;
                    work.quests.forget_matches();
                }
                if holder == Holder::Creature
                    && ui
                        .small_button("back to Creatures")
                        .on_hover_text("Switch to the creature tool, with its selection as it was.")
                        .clicked()
                {
                    *shell.switch_to = Some(Tool::Creatures);
                }
            });
        });
}

/// New, Copy and Remove.
///
/// Unlike an item, a quest can be removed — see `vale_mangos::quest`, where
/// the reason is. A quest in the database is marked: it stays in the list in
/// red until Apply, and Keep takes the mark off. One this project created
/// is in no database, so removing it gives the claim up.
fn row_actions(ui: &mut egui::Ui, work: &mut Workspace<'_>, shell: &mut Shell<'_>) {
    let open = work.quests.open_quest(&work.session.server_edits);
    ui.horizontal(|ui| {
        let third = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(40.0);
        let size = egui::vec2(third, 22.0);
        if ui
            .add(egui::Button::new("+ New").min_size(size))
            .on_hover_text(
                "A new quest_template row, numbered above anything upstream will reach: an \
                 ordinary level 1 quest with no objectives, which the server loads. Nobody \
                 gives it until a giver is added on the right.",
            )
            .clicked()
        {
            let (patch, now) = (shell.patch, work.now);
            work.quests.create(work.session, "New Quest", patch, now);
        }
        if ui
            .add_enabled(open.is_some(), egui::Button::new("Copy").min_size(size))
            .on_hover_text(
                "A copy of this quest under a new entry, with every column as it is drawn. \
                 Its givers and takers are not copied.",
            )
            .on_disabled_hover_text("Open a quest first.")
            .clicked()
        {
            if let Some(known) = open.as_ref() {
                let ctx = ui.ctx().clone();
                act_on(&ctx, work, shell.patch, known, RowAct::Copy);
            }
        }
        let removed = open
            .as_ref()
            .is_some_and(|known| known.claim == Life::Delete);
        let label = match removed {
            true => "Keep",
            false => "Remove",
        };
        if ui
            .add_enabled(open.is_some(), egui::Button::new(label).min_size(size))
            .on_hover_text(match removed {
                true => "Take the removal mark off this quest.",
                false => {
                    "Mark this quest for removal. Apply deletes every content-patch version \
                     of it and its rows in the four relation tables, \
                     areatrigger_involvedrelation and locales_quest, after writing the \
                     statements that restore them. A quest this project created is given \
                     up instead."
                }
            })
            .on_disabled_hover_text("Open a quest first.")
            .clicked()
        {
            if let Some(known) = open.as_ref() {
                let ctx = ui.ctx().clone();
                let act = match removed {
                    true => RowAct::Keep,
                    false => RowAct::Remove,
                };
                act_on(&ctx, work, shell.patch, known, act);
            }
        }
    });
}

/// One row of the list — [`theme::list_row`], with no picture: a quest has none.
fn row(ui: &mut egui::Ui, known: &Known, chosen: bool) -> egui::Response {
    let (sub, tint) = match known.claim {
        Life::Insert => (format!("{} \u{b7} new", known.sub()), theme::INK),
        Life::Delete => (
            format!("{} \u{b7} marked for removal", known.sub()),
            theme::BAD,
        ),
        Life::Update => (known.sub(), theme::INK),
    };
    let title = match known.title.is_empty() {
        true => "(untitled)",
        false => known.title.as_str(),
    };
    theme::list_row(
        ui,
        theme::ListRow {
            title,
            sub: &sub,
            trailing: &known.entry.to_string(),
            tint,
            picture: false,
        },
        chosen,
    )
    .response
}

// ---------------------------------------------------------------------------
// The form
// ---------------------------------------------------------------------------

/// The middle: what this project changes, then the open row's columns.
fn form(ui: &mut egui::Ui, work: &mut Workspace<'_>, shell: &mut Shell<'_>, open: Option<&Open>) {
    egui::ScrollArea::vertical()
        .id_salt("quest-form")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            page_spacing(ui);
            edits_block(ui, work, shell);
            let Some(open) = open else {
                match work.quests.open {
                    None => {
                        theme::heading(ui, "No quest open");
                        theme::note(
                            ui,
                            "Choose one on the left, or make one. Nothing reaches the \
                             database until Apply, which is on the bar's Server\u{2026}.",
                        );
                    }
                    Some(_) => {
                        theme::waiting(ui, "reading the row\u{2026}");
                    }
                }
                return;
            };
            head(ui, open);
            ui.add_space(6.0);
            // A quest marked for removal is read and not edited. A `Delete`
            // row carries no columns, so an edit typed into one would be in the
            // store and in no statement.
            let removed = open.known.claim == Life::Delete;
            ui.add_enabled_ui(!removed, |ui| groups(ui, work, open));
        });
}

/// What this project changes about the server's quests, and the way to the
/// panel that applies it — the same block [`super::items`] draws for items.
fn edits_block(ui: &mut egui::Ui, work: &mut Workspace<'_>, shell: &mut Shell<'_>) {
    let standing = super::sync::standing(super::sync::Half::Quests, work.session, work.assets);
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
    ui.horizontal(|ui| {
        if ui
            .button("Server\u{2026}")
            .on_hover_text(
                "Apply these rows, put them back, or give them up \u{2014} every server \
                 operation is on one panel. An applied quest change is live on a reload of \
                 quest_template and of each relation table written.",
            )
            .clicked()
        {
            shell.server_panel.show();
        }
        if let Some(status) = shell.reloads.status() {
            ui.label(egui::RichText::new(status).small().color(theme::INK_DIM));
        }
    });
}

/// The head of the form: the title, then the key and what kind of quest it is.
fn head(ui: &mut egui::Ui, open: &Open) {
    let known = &open.known;
    let title = match known.title.is_empty() {
        true => "(untitled)",
        false => known.title.as_str(),
    };
    ui.label(
        egui::RichText::new(title)
            .strong()
            .size(17.0)
            .color(theme::INK),
    );
    ui.horizontal(|ui| {
        ui.label(theme::number(format!("entry {}", known.entry)));
        if ui
            .small_button("copy")
            .on_hover_text("Put the entry on the clipboard.")
            .clicked()
        {
            ui.ctx().copy_text(known.entry.to_string());
        }
        ui.label(
            egui::RichText::new(format!("patch {}", known.patch))
                .small()
                .color(theme::INK_FAINT),
        )
        .on_hover_text(
            "`quest_template` is keyed by entry and patch together. The server loads the \
             highest patch at or below its own WowPatch, and this is that row.",
        );
        ui.label(
            egui::RichText::new(known.sub())
                .small()
                .color(theme::INK_DIM),
        );
    });
    match known.claim {
        Life::Insert => {
            ui.label(
                egui::RichText::new("New: this row is in no database yet. Apply writes it.")
                    .small()
                    .color(theme::WARN),
            );
        }
        Life::Delete => {
            ui.label(
                egui::RichText::new(
                    "Marked for removal. Apply deletes it with its relations; Keep, under \
                     the search box, takes the mark off.",
                )
                .small()
                .color(theme::BAD),
            );
        }
        Life::Update => {}
    }
}

/// The order the sections are drawn in, which is a reader's and not the
/// table's: what it is, what it says, what it asks, what it gives, who may take
/// it, where it sits among the others, and the rest.
const SECTIONS: [Group; 8] = [
    Group::Identity,
    Group::Text,
    Group::Objectives,
    Group::Rewards,
    Group::Requirements,
    Group::Chain,
    Group::Emotes,
    Group::Advanced,
];

/// The sections that are expanded when a quest is first opened.
const OPEN: [Group; 4] = [
    Group::Identity,
    Group::Text,
    Group::Objectives,
    Group::Rewards,
];

/// Every section: the composed lines a group has first, then the columns of the
/// group that no line drew, in the table's order.
fn groups(ui: &mut egui::Ui, work: &mut Workspace<'_>, open: &Open) {
    let key = open.known.key();
    for group in SECTIONS {
        let columns: Vec<&'static Column> = quest::TEMPLATE_COLUMNS
            .iter()
            .filter(|column| column.group == group)
            .collect();
        let edited = columns.iter().any(|column| {
            work.session
                .server_edits
                .get(quest::TEMPLATE, &key, column.name)
                .is_some()
        });
        section(
            ui,
            ("quest", group.name()),
            group.name(),
            edited,
            OPEN.contains(&group),
            |ui| {
                let mut drawn: Vec<&'static str> = Vec::new();
                match group {
                    Group::Objectives => objective_lines(ui, work, open, &mut drawn),
                    Group::Rewards => reward_lines(ui, work, open, &mut drawn),
                    Group::Requirements => requirement_lines(ui, work, open, &mut drawn),
                    Group::Emotes => emote_lines(ui, work, open, &mut drawn),
                    _ => {}
                }
                for column in columns {
                    if !drawn.contains(&column.name) {
                        field(ui, work, open, column);
                    }
                }
            },
        );
    }
    ui.add_space(24.0);
}

/// One column on a row of its own.
fn field(ui: &mut egui::Ui, work: &mut Workspace<'_>, open: &Open, column: &'static Column) {
    let state = shown(work.session, open, column);
    page_row(ui, column.name, column.about, state.edited, |ui| {
        if let Some(written) = cell(ui, work, open, column, &state.showing) {
            commit(work.session, open, column, written, work.now);
        }
        revert(ui, work, open, column, &state);
    });
}

/// The revert arrow for one column — not drawn on a row this project creates,
/// for [`super::items`]' reason: every column of one is the project's own.
fn revert(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    open: &Open,
    column: &Column,
    state: &Shown,
) {
    let creating = open.known.claim == Life::Insert;
    if !state.edited || creating || !revert_button(ui, state.in_database.as_deref()) {
        return;
    }
    let gesture = format!("quest {} {} revert", open.known.entry, column.name);
    work.session.set_server_edit(
        quest::TEMPLATE,
        &open.known.key(),
        column.name,
        None,
        Some(crate::session::Gesture {
            label: "Revert column",
            subject: &gesture,
            now: work.now,
        }),
    );
}

/// The `entry` column, which is the row's own key and is editable anyway —
/// `super::items::entry_field` for this subject. Committed on Enter or on
/// leaving the box, because a key that moved on every digit typed would re-key
/// the quest four times on the way to `20000`.
///
/// What it does is [`crate::tools::quests::Quests::rekey`]: the project's claim
/// is re-keyed, its relation rows and its other claims that name the quest
/// follow, and the apply moves the row with every column of the world database
/// that names a quest by entry.
fn entry_cell(ui: &mut egui::Ui, work: &mut Workspace<'_>, open: &Open, id: egui::Id) {
    let known = &open.known;
    let id = id.with("entry");
    let mut text = draft_or(ui, id, known.entry.to_string());
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .desired_width(90.0)
            .horizontal_align(egui::Align::Center),
    );
    // What the trouble was, kept until the box is touched again.
    let trouble_id = id.with("trouble");
    if let Some(text) = finished(ui, id, &response, text) {
        let trouble = match text.trim().parse::<u32>() {
            Err(_) => Some(format!("{text:?} is not an entry")),
            Ok(wanted) => {
                let now = work.now;
                work.quests.rekey(work.session, known, wanted, now).err()
            }
        };
        ui.data_mut(|data| match trouble {
            Some(why) => {
                data.insert_temp(trouble_id, why);
            }
            None => {
                data.remove_temp::<String>(trouble_id);
            }
        });
    }
    if let Some(why) = ui.data_mut(|data| data.get_temp::<String>(trouble_id)) {
        ui.label(egui::RichText::new(why).small().color(theme::BAD));
        return;
    }
    let (word, colour) = match (known.claim, known.read_entry != known.entry) {
        (Life::Insert, _) => ("renumbers this new quest".to_string(), theme::INK_FAINT),
        (_, true) => (
            format!(
                "the database has it at {} until this is applied",
                known.read_entry
            ),
            theme::WARN,
        ),
        _ => (
            "moves the row, and what names it in the world database follows".to_string(),
            theme::INK_FAINT,
        ),
    };
    ui.label(egui::RichText::new(word).small().color(colour))
        .on_hover_text(format!(
            "vmangos has no foreign keys, so the apply writes the cascade itself: every \
             content-patch version of the row, and the {} columns of the world database \
             that name a quest by entry: the four relation tables, \
             areatrigger_involvedrelation, locales_quest, game_event_quest, the chain \
             columns of other quests, item_template.start_quest. Put back returns all of \
             it.\n\nNot reached: character_queststatus in the characters database, so a \
             character part way through the quest loses it.",
            quest::REFERENCES.len()
        ));
}

/// The control a column's kind asks for, answering the literal it wrote.
fn cell(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    open: &Open,
    column: &'static Column,
    showing: &str,
) -> Option<String> {
    // One id per quest and column — see `super::items::field`.
    let id = ui.make_persistent_id(("quest", open.known.entry, column.name));
    match column.kind {
        Kind::Key if column.name == "entry" => {
            entry_cell(ui, work, open, id);
            None
        }
        Kind::Key => {
            ui.label(theme::number(showing));
            None
        }
        Kind::Text => text_cell(ui, id.with("text"), showing),
        Kind::Paragraph => paragraph_cell(ui, id.with("text"), showing, 5),
        Kind::Flags(bits) => flags_cell(ui, showing, bits, id),
        Kind::Choice(values) => choice_cell(ui, showing, values, id),
        Kind::Ref(_) | Kind::Either(_, _) => {
            let written = id_and_pick(ui, work, column, showing);
            resolved(ui, work, column, showing);
            written
        }
        Kind::SignedMoney => {
            let written = number_cell(ui, Kind::Signed, showing);
            let copper: i64 = showing.trim().parse().unwrap_or(0);
            match copper {
                0 => meaning(ui, "nothing"),
                copper if copper > 0 => {
                    meaning(ui, format!("pays {}", quest::money_words(copper as u64)))
                }
                copper => meaning(
                    ui,
                    format!(
                        "costs {} to hand in",
                        quest::money_words(copper.unsigned_abs())
                    ),
                ),
            }
            written
        }
        kind => {
            let written = number_cell(ui, kind, showing);
            if let Some(means) = number_means(kind, showing) {
                meaning(ui, means);
            }
            written
        }
    }
}

/// The number half of a reference, and the `…` that opens the picker.
fn id_and_pick(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    column: &'static Column,
    showing: &str,
) -> Option<String> {
    let (kind, targets) = match column.kind {
        Kind::Either(positive, negative) => {
            (Kind::Signed, target_of(positive).zip(target_of(negative)))
        }
        _ => (Kind::Unsigned, None),
    };
    let written = number_cell(ui, kind, showing);
    let single = match column.kind {
        Kind::Ref(table) => target_of(table),
        _ => None,
    };
    if single.is_none() && targets.is_none() {
        return written;
    }
    if ui
        .add(
            egui::Button::new(egui::RichText::new("\u{2026}").size(13.0))
                .min_size(egui::vec2(24.0, FORM_ROW)),
        )
        .on_hover_text(format!("choose {} by name", column.name))
        .clicked()
    {
        let negative = showing.trim().starts_with('-');
        let target = match (single, targets) {
            (Some(target), _) => target,
            (None, Some((positive, negative_target))) => match negative {
                true => negative_target,
                false => positive,
            },
            (None, None) => return written,
        };
        let mut picker = Picker::new(
            target,
            PickFor::Column {
                column: column.name,
                negated: targets.is_some() && negative,
            },
        );
        picker.either = targets;
        work.quests.picker = Some(picker);
    }
    written
}

/// What a reference resolves to, after the number.
fn resolved(ui: &mut egui::Ui, work: &mut Workspace<'_>, column: &'static Column, showing: &str) {
    let value: i64 = showing.trim().parse().unwrap_or(0);
    if value == 0 {
        ui.label(egui::RichText::new("none").small().color(theme::INK_FAINT));
        return;
    }
    let table = match column.kind {
        Kind::Ref(table) => table,
        Kind::Either(positive, negative) => match value > 0 {
            true => positive,
            false => negative,
        },
        _ => return,
    };
    let id = value.unsigned_abs() as u32;
    // The sign of a signed quest reference changes what it means (for
    // `PrevQuestId`, rewarded first or active at the time), so that reading is
    // drawn before the quest's title.
    if matches!(column.kind, Kind::Either(quest::TEMPLATE, _)) {
        let (rewarded, active) = either_words(column.name);
        meaning(
            ui,
            match value > 0 {
                true => rewarded,
                false => active,
            },
        );
    }
    match target_of(table) {
        Some(Target::Item) => item_name(ui, work, id),
        Some(Target::Creature) => holder_name(ui, work, Holder::Creature, id),
        Some(Target::Object) => holder_name(ui, work, Holder::Object, id),
        Some(Target::Quest) => quest_link(ui, work, id),
        Some(Target::Dbc(table)) => {
            if !open_for_names(work.session, work.assets, table) {
                meaning(ui, format!("{table}\u{2026}"));
                return;
            }
            match name_in(work.session, table, id) {
                Some(name) => {
                    ui.label(egui::RichText::new(name).color(theme::INK));
                    ui.label(egui::RichText::new(table).small().color(theme::INK_FAINT));
                }
                None => {
                    ui.label(
                        egui::RichText::new(format!("not a row of {table}"))
                            .small()
                            .color(theme::BAD),
                    );
                }
            }
        }
        Some(Target::List(_)) | None => meaning(ui, format!("{table} {id}")),
    }
}

/// An item's icon and its name in its quality's colour, as a link: a click
/// on either opens the item in the item workspace. The name is underlined
/// while the pointer is over it and keeps its quality colour rather than a
/// link colour, because the colour states the item's quality.
fn item_name(ui: &mut egui::Ui, work: &mut Workspace<'_>, entry: u32) {
    let Some(found) = work.quests.item(entry, &work.session.server_edits) else {
        match work.quests.item_known(entry, &work.session.server_edits) {
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
    let icon = work
        .items
        .look(work.assets, found.display_id, 0)
        .and_then(|look| look.icon);
    let (rect, picture) = ui.allocate_exact_size(egui::Vec2::splat(ICON), egui::Sense::click());
    ui.painter().rect_filled(rect, 2.0, theme::SUNK);
    if let Some(path) = icon {
        work.thumbnails.want(&path);
        if let Some(texture) = work.thumbnails.get(&path) {
            ui.painter().image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }
    let colour = super::items::quality_colour(found.quality);
    let name = ui.add(
        egui::Label::new(egui::RichText::new(&found.name).color(colour))
            .sense(egui::Sense::click()),
    );
    if name.hovered() {
        let under = name.rect.bottom() - 1.0;
        ui.painter()
            .hline(name.rect.x_range(), under, egui::Stroke::new(1.0, colour));
    }
    let clicked = (picture | name)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Open item {entry} in the item workspace."))
        .clicked();
    if clicked {
        work.quests.show_item = Some(entry);
    }
}

/// A creature's or game object's name, with which of the two it is.
fn holder_name(ui: &mut egui::Ui, work: &mut Workspace<'_>, holder: Holder, id: u32) {
    match work.quests.holder(holder, id, &work.session.server_edits) {
        Some(name) => {
            ui.label(egui::RichText::new(name).color(theme::INK));
            ui.label(
                egui::RichText::new(holder.word())
                    .small()
                    .color(theme::INK_FAINT),
            );
        }
        None => match work.quests.holder_known(holder, id, &work.session.server_edits) {
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

/// Another quest's title, as a link that opens it.
fn quest_link(ui: &mut egui::Ui, work: &mut Workspace<'_>, entry: u32) {
    match work.quests.title_of(entry, &work.session.server_edits) {
        Some(title) => {
            if ui
                .add(egui::Link::new(
                    egui::RichText::new(title).color(theme::ACCENT),
                ))
                .on_hover_text(format!("open quest {entry}"))
                .clicked()
            {
                work.quests.open = Some(entry);
            }
        }
        None => {
            ui.label(
                egui::RichText::new("no such quest")
                    .small()
                    .color(theme::BAD),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The composed lines
// ---------------------------------------------------------------------------

/// A column of the template by a composed name, or `None` for a name the schema
/// does not have — which draws nothing rather than panicking on a typo.
fn numbered(stem: &str, slot: usize) -> Option<&'static Column> {
    quest::column(quest::TEMPLATE, &format!("{stem}{slot}"))
}

/// How many slots of a numbered run to draw: every used one and one free
/// one after the last, up to all of them.
fn slots_to_draw(used: &[bool]) -> usize {
    let last = used
        .iter()
        .rposition(|used| *used)
        .map(|at| at + 1)
        .unwrap_or(0);
    (last + 1).min(used.len())
}

/// One line for two columns: an id with its picker, a joining word, a
/// number, and what the id resolves to. See the module comment.
#[allow(clippy::too_many_arguments)]
fn pair_row(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    open: &Open,
    label: &str,
    first: &'static Column,
    joiner: &str,
    second: &'static Column,
    drawn: &mut Vec<&'static str>,
) {
    drawn.push(first.name);
    drawn.push(second.name);
    let a = shown(work.session, open, first);
    let b = shown(work.session, open, second);
    let about = format!(
        "{}: {}\n{}: {}",
        first.name, first.about, second.name, second.about
    );
    page_row(ui, label, &about, a.edited || b.edited, |ui| {
        if let Some(written) = id_and_pick(ui, work, first, &a.showing) {
            commit(work.session, open, first, written, work.now);
        }
        ui.label(egui::RichText::new(joiner).color(theme::INK_FAINT));
        let was: i64 = b.showing.trim().parse().unwrap_or(0);
        let mut value = was;
        let response = ui.add_sized(
            egui::vec2(COUNT, FORM_ROW),
            egui::DragValue::new(&mut value).speed(0.25),
        );
        if response.changed() && value != was {
            commit(work.session, open, second, value.to_string(), work.now);
        }
        if let Some(means) = number_means(second.kind, &b.showing) {
            meaning(ui, means);
        }
        resolved(ui, work, first, &a.showing);
        revert(ui, work, open, first, &a);
        revert(ui, work, open, second, &b);
    });
}

/// A numbered run of pairs — `ReqItemId1..4` with `ReqItemCount1..4` — drawn as
/// the used slots and one free one. Every slot's columns are marked drawn,
/// shown or not, so the ones left out are not drawn again as bare fields.
#[allow(clippy::too_many_arguments)]
fn pair_run(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    open: &Open,
    label: &str,
    first_stem: &str,
    joiner: &str,
    second_stem: &str,
    slots: usize,
    drawn: &mut Vec<&'static str>,
) {
    let pairs: Vec<(&'static Column, &'static Column)> = (1..=slots)
        .filter_map(|slot| numbered(first_stem, slot).zip(numbered(second_stem, slot)))
        .collect();
    let used: Vec<bool> = pairs
        .iter()
        .map(|(first, second)| {
            number_of(work.session, open, first.name) != 0
                || number_of(work.session, open, second.name) != 0
        })
        .collect();
    let show = slots_to_draw(&used);
    for (at, (first, second)) in pairs.into_iter().enumerate() {
        if at >= show {
            drawn.push(first.name);
            drawn.push(second.name);
            continue;
        }
        pair_row(
            ui,
            work,
            open,
            &format!("{label} {}", at + 1),
            first,
            joiner,
            second,
            drawn,
        );
    }
}

/// A small heading inside a section, over a run of composed lines.
fn run_heading(ui: &mut egui::Ui, text: &str, about: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text).size(12.0).color(theme::INK_DIM))
        .on_hover_text(about);
}

/// What the quest asks for, a line per objective.
fn objective_lines(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    open: &Open,
    drawn: &mut Vec<&'static str>,
) {
    run_heading(
        ui,
        "Kill, use or cast on",
        "ReqCreatureOrGOId, ReqCreatureOrGOCount, ReqSpellCast and ObjectiveText, a line per \
         objective. The id is a creature, or a game object when it is negative.",
    );
    let used: Vec<bool> = (1..=4)
        .map(|slot| {
            number_of(work.session, open, &format!("ReqCreatureOrGOId{slot}")) != 0
                || number_of(work.session, open, &format!("ReqCreatureOrGOCount{slot}")) != 0
                || number_of(work.session, open, &format!("ReqSpellCast{slot}")) != 0
                || !text_of(work.session, open, &format!("ObjectiveText{slot}")).is_empty()
        })
        .collect();
    let show = slots_to_draw(&used);
    for slot in 1..=4 {
        let (Some(id), Some(count), Some(cast), Some(text)) = (
            numbered("ReqCreatureOrGOId", slot),
            numbered("ReqCreatureOrGOCount", slot),
            numbered("ReqSpellCast", slot),
            numbered("ObjectiveText", slot),
        ) else {
            continue;
        };
        if slot > show {
            drawn.extend([id.name, count.name, cast.name, text.name]);
            continue;
        }
        pair_row(
            ui,
            work,
            open,
            &format!("Objective {slot}"),
            id,
            "\u{d7}",
            count,
            drawn,
        );
        // `ReqSpellCast` and `ObjectiveText` qualify the objective line above,
        // and are drawn only when that slot is used.
        if used[slot - 1] {
            field(ui, work, open, cast);
            field(ui, work, open, text);
        }
        drawn.extend([cast.name, text.name]);
    }

    run_heading(
        ui,
        "Bring",
        "ReqItemId and ReqItemCount: items handed over with the quest.",
    );
    pair_run(
        ui,
        work,
        open,
        "Item",
        "ReqItemId",
        "\u{d7}",
        "ReqItemCount",
        4,
        drawn,
    );

    run_heading(
        ui,
        "Source items",
        "ReqSourceId and ReqSourceCount: items the quest needs in the bags without asking for \
         them, such as what a required item is made from. They are taken away with the quest.",
    );
    pair_run(
        ui,
        work,
        open,
        "Source",
        "ReqSourceId",
        "\u{d7}",
        "ReqSourceCount",
        4,
        drawn,
    );

    run_heading(
        ui,
        "On accepting",
        "What is handed over or cast when the quest is taken.",
    );
    if let (Some(item), Some(count)) = (
        quest::column(quest::TEMPLATE, "SrcItemId"),
        quest::column(quest::TEMPLATE, "SrcItemCount"),
    ) {
        pair_row(ui, work, open, "Given item", item, "\u{d7}", count, drawn);
    }
    run_heading(ui, "Also", "A reputation to reach, and a time limit.");
    if let (Some(faction), Some(value)) = (
        quest::column(quest::TEMPLATE, "RepObjectiveFaction"),
        quest::column(quest::TEMPLATE, "RepObjectiveValue"),
    ) {
        pair_row(
            ui,
            work,
            open,
            "Reach reputation",
            faction,
            "\u{2265}",
            value,
            drawn,
        );
    }
}

/// What the quest gives, a line per reward.
fn reward_lines(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    open: &Open,
    drawn: &mut Vec<&'static str>,
) {
    run_heading(
        ui,
        "One of",
        "RewChoiceItemId and RewChoiceItemCount: the player takes one.",
    );
    pair_run(
        ui,
        work,
        open,
        "Choice",
        "RewChoiceItemId",
        "\u{d7}",
        "RewChoiceItemCount",
        6,
        drawn,
    );
    run_heading(
        ui,
        "Always",
        "RewItemId and RewItemCount: every one of these is given.",
    );
    pair_run(
        ui,
        work,
        open,
        "Item",
        "RewItemId",
        "\u{d7}",
        "RewItemCount",
        4,
        drawn,
    );
    run_heading(
        ui,
        "Reputation",
        "RewRepFaction and RewRepValue. A negative value takes reputation away.",
    );
    pair_run(
        ui,
        work,
        open,
        "Faction",
        "RewRepFaction",
        "+",
        "RewRepValue",
        5,
        drawn,
    );
    run_heading(ui, "And", "Money, experience, a spell and a letter.");
}

/// Who may take it: the three id-and-value pairs, then the rest.
fn requirement_lines(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    open: &Open,
    drawn: &mut Vec<&'static str>,
) {
    for (label, first, joiner, second) in [
        ("Skill", "RequiredSkill", "\u{2265}", "RequiredSkillValue"),
        (
            "Reputation at least",
            "RequiredMinRepFaction",
            "\u{2265}",
            "RequiredMinRepValue",
        ),
        (
            "Reputation below",
            "RequiredMaxRepFaction",
            "<",
            "RequiredMaxRepValue",
        ),
    ] {
        if let (Some(first), Some(second)) = (
            quest::column(quest::TEMPLATE, first),
            quest::column(quest::TEMPLATE, second),
        ) {
            pair_row(ui, work, open, label, first, joiner, second, drawn);
        }
    }
}

/// The emotes, each with its delay.
fn emote_lines(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    open: &Open,
    drawn: &mut Vec<&'static str>,
) {
    run_heading(ui, "While offering", "DetailsEmote and DetailsEmoteDelay.");
    pair_run(
        ui,
        work,
        open,
        "Emote",
        "DetailsEmote",
        "after",
        "DetailsEmoteDelay",
        4,
        drawn,
    );
    run_heading(
        ui,
        "While rewarding",
        "OfferRewardEmote and OfferRewardEmoteDelay.",
    );
    pair_run(
        ui,
        work,
        open,
        "Emote",
        "OfferRewardEmote",
        "after",
        "OfferRewardEmoteDelay",
        4,
        drawn,
    );
    run_heading(ui, "While handing in", "IncompleteEmote and CompleteEmote.");
}

// ---------------------------------------------------------------------------
// The inspector's part
// ---------------------------------------------------------------------------

/// The shell's inspector, while the quest workspace is up: the quest as the
/// log would show it, who gives and takes it, and where it sits in a chain.
pub fn inspector(ui: &mut egui::Ui, mut work: Workspace<'_>) {
    let open = open_now(&work);
    let work = &mut work;
    let Some(open) = open.as_ref() else {
        theme::heading(ui, "In the log");
        match work.quests.open {
            None => theme::note(ui, "No quest open."),
            Some(_) => theme::waiting(ui, "reading the row\u{2026}"),
        }
        return;
    };
    log_entry(ui, work, open);
    ui.add_space(8.0);
    relations(ui, work, open);
    ui.add_space(8.0);
    chain(ui, work, open);
    ui.add_space(16.0);
}

/// What the text codes in a quest's text become, for a preview.
///
/// `$B` is a line break. `$N`, `$C` and `$R` are the character's name, class
/// and race, drawn as the word in angle brackets because a preview has no
/// character; `$G male:female;` draws its first half. The codes are
/// case-insensitive in the client, and both cases ship.
pub fn preview_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }
        match chars.next().map(|code| code.to_ascii_lowercase()) {
            Some('b') => out.push('\n'),
            Some('n') => out.push_str("<name>"),
            Some('c') => out.push_str("<class>"),
            Some('r') => out.push_str("<race>"),
            Some('g') => {
                // `$Gmale:female;` — the first word, up to the colon.
                let rest: String = chars.by_ref().take_while(|next| *next != ';').collect();
                out.push_str(rest.split(':').next().unwrap_or("").trim());
            }
            Some(other) => {
                out.push('$');
                out.push(other);
            }
            None => out.push('$'),
        }
    }
    out
}

/// The log entry, composed from the row with this project's edits over it.
fn log_entry(ui: &mut egui::Ui, work: &mut Workspace<'_>, open: &Open) {
    theme::heading(ui, "In the log");
    let session = &*work.session;
    let title = text_of(session, open, "Title");
    ui.label(
        egui::RichText::new(title)
            .strong()
            .size(15.0)
            .color(theme::INK),
    );
    let level = number_of(session, open, "QuestLevel");
    let kind = number_of(session, open, "Type") as u32;
    let mut line = format!("[{level}]");
    if kind != 0 {
        line.push_str(&format!(" {}", quest::value_word(&quest::TYPES, kind)));
    }
    let suggested = number_of(session, open, "SuggestedPlayers");
    if suggested > 0 {
        line.push_str(&format!(" \u{b7} suggested players: {suggested}"));
    }
    ui.label(egui::RichText::new(line).small().color(theme::INK_DIM));

    let body = |ui: &mut egui::Ui, text: String| {
        if !text.trim().is_empty() {
            ui.label(
                egui::RichText::new(preview_text(&text))
                    .color(theme::INK)
                    .size(12.5),
            );
        }
    };
    ui.add_space(4.0);
    body(ui, text_of(session, open, "Objectives"));

    // The counted objectives, as the log words them: a name and `0/n`.
    let mut lines: Vec<String> = Vec::new();
    for slot in 1..=4 {
        let id = number_of(work.session, open, &format!("ReqCreatureOrGOId{slot}"));
        let count = number_of(work.session, open, &format!("ReqCreatureOrGOCount{slot}"));
        if id == 0 || count == 0 {
            continue;
        }
        let own = text_of(work.session, open, &format!("ObjectiveText{slot}"));
        let name = match own.is_empty() {
            false => Some(own),
            true => {
                let holder = match id > 0 {
                    true => Holder::Creature,
                    false => Holder::Object,
                };
                let found = work.quests.holder(holder, id.unsigned_abs() as u32, &work.session.server_edits);
                // The client appends " slain" to a creature's own name.
                found.map(|name| match holder {
                    Holder::Creature => format!("{name} slain"),
                    Holder::Object => name,
                })
            }
        };
        lines.push(format!(
            "{}: 0/{count}",
            name.unwrap_or_else(|| format!("{id}"))
        ));
    }
    for slot in 1..=4 {
        let id = number_of(work.session, open, &format!("ReqItemId{slot}"));
        let count = number_of(work.session, open, &format!("ReqItemCount{slot}"));
        if id <= 0 || count == 0 {
            continue;
        }
        let found = work.quests.item(id as u32, &work.session.server_edits);
        let name = found
            .map(|item| item.name)
            .unwrap_or_else(|| format!("item {id}"));
        lines.push(format!("{name}: 0/{count}"));
    }
    let end = text_of(work.session, open, "EndText");
    if !end.is_empty() {
        lines.push(end);
    }
    for line in &lines {
        ui.label(
            egui::RichText::new(format!("  \u{2022} {line}"))
                .color(theme::INK_DIM)
                .size(12.0),
        );
    }

    let details = text_of(work.session, open, "Details");
    if !details.trim().is_empty() {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Description")
                .strong()
                .size(12.5)
                .color(theme::INK),
        );
        body(ui, details);
    }

    // The rewards, in the client's own two groups.
    let mut choices: Vec<(u32, i64)> = Vec::new();
    let mut always: Vec<(u32, i64)> = Vec::new();
    for slot in 1..=6 {
        let id = number_of(work.session, open, &format!("RewChoiceItemId{slot}"));
        if id > 0 {
            choices.push((
                id as u32,
                number_of(work.session, open, &format!("RewChoiceItemCount{slot}")),
            ));
        }
    }
    for slot in 1..=4 {
        let id = number_of(work.session, open, &format!("RewItemId{slot}"));
        if id > 0 {
            always.push((
                id as u32,
                number_of(work.session, open, &format!("RewItemCount{slot}")),
            ));
        }
    }
    let money = number_of(work.session, open, "RewOrReqMoney");
    let xp = number_of(work.session, open, "RewXP");
    let spell = number_of(work.session, open, "RewSpell");
    if choices.is_empty() && always.is_empty() && money == 0 && xp == 0 && spell == 0 {
        return;
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new("Rewards")
            .strong()
            .size(12.5)
            .color(theme::INK),
    );
    let reward = |ui: &mut egui::Ui, work: &mut Workspace<'_>, (id, count): (u32, i64)| {
        ui.horizontal(|ui| {
            item_name(ui, work, id);
            if count > 1 {
                ui.label(egui::RichText::new(format!("\u{d7}{count}")).color(theme::INK_DIM));
            }
        });
    };
    if !choices.is_empty() {
        ui.label(
            egui::RichText::new("You will be able to choose one of these rewards:")
                .small()
                .color(theme::INK_DIM),
        );
        for choice in choices {
            reward(ui, work, choice);
        }
    }
    if !always.is_empty() || money > 0 || spell > 0 {
        ui.label(
            egui::RichText::new("You will receive:")
                .small()
                .color(theme::INK_DIM),
        );
        for item in always {
            reward(ui, work, item);
        }
    }
    if money > 0 {
        ui.label(egui::RichText::new(quest::money_words(money as u64)).color(theme::INK));
    }
    if money < 0 {
        ui.label(
            egui::RichText::new(format!(
                "Required money: {}",
                quest::money_words(money.unsigned_abs())
            ))
            .color(theme::WARN),
        );
    }
    if spell > 0 && work.session.open_table(work.assets, "Spell") {
        let name = name_in(work.session, "Spell", spell as u32)
            .unwrap_or_else(|| format!("spell {spell}"));
        ui.label(egui::RichText::new(format!("Spell: {name}")).color(theme::INK));
    }
    if xp > 0 {
        ui.label(
            egui::RichText::new(format!("{xp} experience at level {level}"))
                .small()
                .color(theme::INK_DIM),
        )
        .on_hover_text(
            "RewXP is what the quest gives at its own level. The server scales it down for \
             a character above that level; the client never shows it.",
        );
    }
}

/// Who hands the quest out and who takes it back, with the way to add and
/// remove one.
fn relations(ui: &mut egui::Ui, work: &mut Workspace<'_>, open: &Open) {
    let entry = open.known.entry;
    let all = work
        .quests
        .relations_of_quest(entry, &work.session.server_edits);
    for role in [Role::Gives, Role::Takes] {
        theme::heading(
            ui,
            match role {
                Role::Gives => "Given by",
                Role::Takes => "Taken by",
            },
        );
        let here: Vec<(Relation, Life)> = all
            .iter()
            .copied()
            .filter(|(relation, _)| relation.role == role)
            .collect();
        if here.is_empty() {
            theme::note(
                ui,
                match role {
                    Role::Gives => {
                        "Nobody. It can still be started by an item's start_quest, by \
                         another quest's NextQuestInChain, or by a script."
                    }
                    Role::Takes => "Nobody. A quest nobody takes cannot be handed in.",
                },
            );
        }
        for (relation, life) in here {
            relation_row(ui, work, relation, life);
        }
        ui.horizontal(|ui| {
            for holder in [Holder::Creature, Holder::Object] {
                if ui
                    .small_button(format!("+ {}\u{2026}", holder.word()))
                    .on_hover_text(format!(
                        "Add a row to {}: a {} that {} this quest. It names a template \
                         entry, so every spawn of it does.",
                        crate::tools::quests::table_of(holder, role),
                        holder.word(),
                        role.word()
                    ))
                    .clicked()
                {
                    let target = match holder {
                        Holder::Creature => Target::Creature,
                        Holder::Object => Target::Object,
                    };
                    work.quests.picker =
                        Some(Picker::new(target, PickFor::Relation { holder, role }));
                }
            }
        });
    }
}

/// One relation of the open quest: who, what this project says about the row,
/// and the button that removes or keeps it. Clicking the name narrows the list
/// to that creature's quests.
fn relation_row(ui: &mut egui::Ui, work: &mut Workspace<'_>, relation: Relation, life: Life) {
    ui.horizontal(|ui| {
        let colour = match life {
            Life::Delete => theme::BAD,
            _ => theme::ACCENT,
        };
        let name = work
            .quests
            .holder(relation.holder, relation.id, &work.session.server_edits)
            .unwrap_or_else(|| format!("{} {}", relation.holder.word(), relation.id));
        if ui
            .add(egui::Link::new(egui::RichText::new(name).color(colour)))
            .on_hover_text("narrow the list to the quests this one gives or takes")
            .clicked()
        {
            work.quests.show_quests_of(relation.holder, relation.id);
        }
        ui.label(theme::number(relation.id.to_string()).color(theme::INK_FAINT));
        if relation.holder == Holder::Object {
            ui.label(
                egui::RichText::new("game object")
                    .small()
                    .color(theme::INK_FAINT),
            );
        }
        match life {
            Life::Insert => {
                ui.label(egui::RichText::new("new").small().color(theme::WARN));
            }
            Life::Delete => {
                ui.label(
                    egui::RichText::new("marked for removal")
                        .small()
                        .color(theme::BAD),
                );
            }
            Life::Update => {}
        }
        let (label, about) = match life {
            Life::Delete => ("keep", "Take the removal mark off this row."),
            Life::Insert => (
                "\u{d7}",
                "Give up this relation, which is in no database yet.",
            ),
            Life::Update => (
                "\u{d7}",
                "Mark this row for removal. Apply deletes it, after writing the statement \
                 that restores it.",
            ),
        };
        if ui.small_button(label).on_hover_text(about).clicked() {
            let now = work.now;
            match life {
                Life::Delete => work.quests.relate(work.session, relation, now),
                _ => work.quests.unrelate(work.session, relation, now),
            }
        }
    });
}

/// Where the quest sits among the others: what it needs, what it leads to,
/// and which other quests name it as a prerequisite, which no column of its
/// own row records.
fn chain(ui: &mut egui::Ui, work: &mut Workspace<'_>, open: &Open) {
    theme::heading(ui, "Chain");
    let entry = open.known.entry as i32;
    let mut any = false;
    let line = |ui: &mut egui::Ui, work: &mut Workspace<'_>, word: &str, other: u32| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(word).small().color(theme::INK_DIM));
            quest_link(ui, work, other);
            ui.label(theme::number(other.to_string()).color(theme::INK_FAINT));
        });
    };
    let prev = number_of(work.session, open, "PrevQuestId");
    if prev != 0 {
        any = true;
        let word = match prev > 0 {
            true => "after",
            false => "while active:",
        };
        line(ui, work, word, prev.unsigned_abs() as u32);
    }
    for (column, word) in [
        ("NextQuestId", "unlocks"),
        ("NextQuestInChain", "offers next"),
    ] {
        let next = number_of(work.session, open, column);
        if next != 0 {
            any = true;
            line(ui, work, word, next.unsigned_abs() as u32);
        }
    }
    // The other direction, from the list in memory: what names this quest.
    let named_by: Vec<(u32, &'static str)> = work
        .quests
        .all
        .iter()
        .chain(work.quests.created.iter())
        .filter(|other| other.entry as i32 != entry)
        .filter_map(|other| {
            if other.prev.abs() == entry {
                Some((other.entry, "required by"))
            } else if other.next.abs() == entry {
                Some((other.entry, "unlocked by"))
            } else if other.next_in_chain as i32 == entry {
                Some((other.entry, "offered after"))
            } else {
                None
            }
        })
        .take(12)
        .collect();
    for (other, word) in named_by {
        any = true;
        line(ui, work, word, other);
    }
    if !any {
        theme::note(ui, "It stands alone: no quest names it and it names none.");
    }
}

// ---------------------------------------------------------------------------
// The reference picker
// ---------------------------------------------------------------------------

/// Search a client table by name, which is the one target the tool cannot
/// search for itself: the table is opened through the session.
fn search_a_table(session: &mut EditSession, assets: &GameAssets, picker: &mut Picker) {
    let Target::Dbc(table) = picker.target else {
        return;
    };
    let asked = (picker.target, picker.query.trim().to_ascii_lowercase());
    if picker.built.as_ref() == Some(&asked) {
        return;
    }
    if !open_for_names(session, assets, table) {
        return;
    }
    let Some(open) = session.table(table) else {
        return;
    };
    let query = asked.1.clone();
    picker.built = Some(asked);
    let by_id: Option<u32> = query.parse().ok();
    picker.hits.clear();
    for record in 0..open.record_count() {
        let Some(id) = open.u32_at(record, 0) else {
            continue;
        };
        let name = record_name(session, table, record).unwrap_or_default();
        let hit =
            query.is_empty() || by_id == Some(id) || name.to_ascii_lowercase().contains(&query);
        if !hit || name.is_empty() {
            continue;
        }
        let mut hit = Hit {
            id,
            title: name,
            ..Hit::default()
        };
        if table == "Spell" {
            describe_spell(session, open, record, &mut hit);
        }
        picker.hits.push(hit);
        if picker.hits.len() >= PICK_LIMIT {
            break;
        }
    }
}

/// A spell row as the spell list draws it, with its effects added.
///
/// Ranks of one spell share its name (there are nine Fireballs), so the rank,
/// field 129, leads the second line. The effect names follow it, which is what
/// tells a teaching spell (`LEARN_SPELL`) from the spell it teaches. The hover
/// has one line per effect, with its aura and the spell it triggers or
/// teaches, by name.
fn describe_spell(session: &EditSession, open: &DbcFile, record: usize, hit: &mut Hit) {
    use vale_assets::tables::spellbook::spell_fields as field;
    use vale_assets::tables::spellnames::{aura_name, effect_name};
    let number = |at: usize| open.u32_at(record, at).unwrap_or(0);
    let mut names: Vec<String> = Vec::new();
    for slot in 0..3 {
        let effect = number(field::EFFECT + slot);
        if effect == 0 {
            continue;
        }
        let name = effect_name(effect)
            .map(str::to_string)
            .unwrap_or_else(|| format!("effect {effect}"));
        let mut line = name.clone();
        let aura = number(field::EFFECT_APPLY_AURA + slot);
        if aura != 0 {
            let word = aura_name(aura)
                .map(str::to_string)
                .unwrap_or_else(|| format!("aura {aura}"));
            line.push_str(&format!(" {word}"));
        }
        let trigger = number(field::EFFECT_TRIGGER_SPELL + slot);
        if trigger != 0 {
            let called = open
                .row_of(trigger)
                .and_then(|row| record_name(session, "Spell", row))
                .unwrap_or_default();
            line.push_str(&format!(" \u{2192} {called} {trigger}"));
        }
        names.push(name);
        hit.about.push(line);
    }
    let rank = open.string_at(record, 129).unwrap_or_default();
    hit.sub = [rank, names.join(", ")]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
    let icon = number(vale_assets::tables::schema::SPELL_ICON_FIELD);
    hit.picture = (icon != 0).then_some(Picture::SpellIcon(icon));
}

/// One row of the picker, drawn as the spell list and the item list draw
/// theirs: the icon, an item's name in its quality's colour, and the second
/// line, with [`Hit::about`] as the hover. Answers whether it was clicked.
///
/// The icon is asked for on every frame the row is drawn, as in those lists:
/// only rows on screen pay for a decode. A row whose icon has not been decoded
/// yet draws an empty square in its place.
fn hit_row(
    ui: &mut egui::Ui,
    session: &EditSession,
    assets: &GameAssets,
    thumbnails: &mut Thumbnails,
    hit: &Hit,
) -> bool {
    let shape = theme::list_row(
        ui,
        theme::ListRow {
            title: &hit.title,
            sub: &hit.sub,
            trailing: &hit.id.to_string(),
            tint: hit
                .quality
                .map(super::items::quality_colour)
                .unwrap_or(theme::INK),
            picture: hit.picture.is_some(),
        },
        false,
    );
    if let Some(picture) = hit.picture {
        let path = match picture {
            Picture::SpellIcon(icon) => super::data::spell_icon_path(session, assets, icon),
            Picture::ItemDisplay(display) => crate::tools::items::icon_of(assets, display),
        };
        let texture = path.and_then(|path| {
            thumbnails.want(&path);
            thumbnails.get(&path)
        });
        match texture {
            Some(id) => {
                ui.painter().image(
                    id,
                    shape.picture,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            None => {
                ui.painter().rect_filled(shape.picture, 3.0, theme::SUNK);
            }
        }
    }
    let response = match hit.about.is_empty() {
        true => shape.response,
        false => shape.response.on_hover_text(hit.about.join("
")),
    };
    response.clicked()
}

/// One dialog over every table a quest names.
///
/// A dialog rather than a popup for `ui::data`'s reason: it is typed into and
/// scrolled. Drawn against the context, so the workspace and the creature
/// window both reach it.
pub(super) fn picker(
    ctx: &egui::Context,
    session: &mut EditSession,
    quests: &mut Quests,
    assets: &GameAssets,
    thumbnails: &mut Thumbnails,
    open: Option<&Open>,
    now: f64,
) {
    // Once a pass, whichever panel asks first; see `Quests::picker_pass`.
    let pass = ctx.cumulative_pass_nr();
    if quests.picker_pass == Some(pass) {
        return;
    }
    quests.picker_pass = Some(pass);
    let Some(mut picker) = quests.picker.take() else {
        return;
    };
    search_a_table(session, assets, &mut picker);

    let mut close = false;
    let mut chosen: Option<Hit> = None;
    let mut clear = false;
    let response = egui::Modal::new(egui::Id::new("quest-pick")).show(ctx, |ui| {
        ui.set_width(420.0);
        let heading = match &picker.purpose {
            PickFor::Column { column, .. } => format!("Choose {column}"),
            PickFor::Relation { holder, role } => {
                format!(
                    "Choose the {} that {} this quest",
                    holder.word(),
                    role.word()
                )
            }
            PickFor::QuestOf { role, .. } => format!("Choose a quest it {}", role.word()),
            PickFor::ServerColumn(target) => format!("Choose {}", target.column),
            PickFor::LootItem { table, entry } => format!("Choose an item for {table} {entry}"),
            PickFor::VendorItem { table, entry } => format!("Choose an item for {table} {entry}"),
            PickFor::TrainerSpell { table, entry } => format!("Choose a spell for {table} {entry}"),
            PickFor::ScriptCell { column, .. } => format!("Choose {column}"),
            PickFor::TableField { column, .. } => format!("Choose {column}"),
        };
        ui.label(egui::RichText::new(heading).strong().size(14.0));

        // The switch, for a column whose sign picks the table.
        if let (Some((positive, negative)), PickFor::Column { column, negated }) =
            (picker.either, &mut picker.purpose)
        {
            let (first, second) = either_words(column);
            let mut negative_now = *negated;
            ui.allocate_ui(egui::vec2(320.0, 24.0), |ui| {
                theme::segmented(
                    ui,
                    &mut negative_now,
                    &[(first, false), (second, true)],
                    |a, b| a == b,
                );
            });
            if negative_now != *negated {
                *negated = negative_now;
                picker.target = match negative_now {
                    true => negative,
                    false => positive,
                };
                picker.built = None;
                picker.hits.clear();
            }
        }

        let hint = match picker.target {
            Target::Item | Target::Creature | Target::Object => "part of a name, or an entry",
            Target::List(_) => "a creature that uses it, what it holds, or an id",
            _ => "name, or an id",
        };
        let search = ui.add(
            egui::TextEdit::singleline(&mut picker.query)
                .hint_text(hint)
                .desired_width(400.0),
        );
        if picker.focus {
            search.request_focus();
            picker.focus = false;
        }
        let database = matches!(
            picker.target,
            Target::Item | Target::Creature | Target::Object
        );
        if database && picker.query.trim().is_empty() {
            theme::note(
                ui,
                "Type part of a name. This table is searched in the database.",
            );
        } else if picker.searching() {
            // A search running while the last one's rows are still listed says
            // so too, above them, so a list that is about to change is not
            // taken for the answer.
            theme::waiting(ui, "searching\u{2026}");
        } else if picker.hits.is_empty() {
            theme::note(ui, "Nothing matches.");
        }
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .auto_shrink([false, true])
            .show_rows(ui, theme::LIST_ROW, picker.hits.len(), |ui, range| {
                for hit in &picker.hits[range] {
                    if hit_row(ui, session, assets, thumbnails, hit) {
                        chosen = Some(hit.clone());
                    }
                }
            });
        if picker.hits.len() >= PICK_LIMIT {
            theme::note(
                ui,
                format!("The first {PICK_LIMIT}. Type more to narrow it."),
            );
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                close = true;
            }
            if matches!(
                picker.purpose,
                PickFor::Column { .. }
                    | PickFor::ServerColumn(_)
                    | PickFor::ScriptCell { .. }
                    | PickFor::TableField { .. }
            ) && ui
                    .button("Set to none")
                    .on_hover_text("Write 0, which for every reference here means none.")
                    .clicked()
            {
                clear = true;
            }
            ui.label(
                egui::RichText::new("Esc closes")
                    .small()
                    .color(theme::INK_FAINT),
            );
        });
    });
    if response.should_close() {
        close = true;
    }

    if let Some(hit) = chosen.as_ref() {
        quests.learn(picker.target, hit);
    }
    match (&picker.purpose, chosen, clear) {
        (PickFor::Column { column, negated }, picked, cleared) if picked.is_some() || cleared => {
            if let (Some(open), Some(column)) = (open, quest::column(quest::TEMPLATE, column)) {
                let value = match (picked, negated) {
                    (None, _) => "0".to_string(),
                    (Some(hit), true) => format!("-{}", hit.id),
                    (Some(hit), false) => hit.id.to_string(),
                };
                commit(session, open, column, value, now);
            }
            close = true;
        }
        (PickFor::ServerColumn(target), picked, cleared) if picked.is_some() || cleared => {
            let written = picked
                .map(|hit| hit.id.to_string())
                .unwrap_or_else(|| "0".to_string());
            target.write(session, written, now);
            close = true;
        }
        (PickFor::Relation { holder, role }, Some(hit), _) => {
            if let Some(open) = open {
                let relation = Relation {
                    holder: *holder,
                    role: *role,
                    id: hit.id,
                    quest: open.known.entry,
                };
                quests.relate(session, relation, now);
            }
            close = true;
        }
        (PickFor::QuestOf { holder, id, role }, Some(hit), _) => {
            let relation = Relation {
                holder: *holder,
                role: *role,
                id: *id,
                quest: hit.id,
            };
            quests.relate(session, relation, now);
            close = true;
        }
        (PickFor::LootItem { table, entry }, Some(hit), _) => {
            quests.loot_pick = Some((table, *entry, hit.id));
            close = true;
        }
        (PickFor::VendorItem { table, entry } | PickFor::TrainerSpell { table, entry }, Some(hit), _) => {
            quests.service_pick = Some((table, *entry, hit.id));
            close = true;
        }
        (PickFor::ScriptCell { table, id, row, column }, picked, cleared)
            if picked.is_some() || cleared =>
        {
            let value = picked.map(|hit| hit.id).unwrap_or(0);
            quests.script_pick = Some((table, *id, *row, column, value));
            close = true;
        }
        (PickFor::TableField { table, record, field, column }, picked, cleared)
            if picked.is_some() || cleared =>
        {
            let value = picked.map(|hit| hit.id).unwrap_or(0);
            // An item column of a set: the two items' `set_id` follows once
            // their rows are read. See `Quests::follow_sets`.
            if table == "ItemSet" && crate::tools::tables::SET_ITEMS.contains(field) {
                let row = session.table(table).map(|sets| {
                    (sets.u32_at(*record, 0).unwrap_or(0), sets.u32_at(*record, *field).unwrap_or(0))
                });
                if let Some((set, left)) = row.filter(|(_, left)| *left != value) {
                    quests.set_follows.push(crate::tools::quests::SetFollow {
                        set,
                        left,
                        joined: value,
                    });
                }
            }
            crate::tools::tables::set_field(
                session,
                table,
                *record,
                *field,
                value,
                &format!("Edit {column}"),
                now,
            );
            close = true;
        }
        _ => {}
    }
    if !close {
        quests.picker = Some(picker);
    }
}

// ---------------------------------------------------------------------------
// The creature and game-object tools' window
// ---------------------------------------------------------------------------

/// Everything the quest window of a selected creature or game object needs.
pub struct HolderQuests<'a> {
    pub session: &'a mut EditSession,
    pub quests: &'a mut Quests,
    pub assets: &'a GameAssets,
    /// What the selection is, as the one thing a relation needs of it — see
    /// [`HolderIs`].
    pub is: HolderIs,
    pub switch_to: &'a mut Option<Tool>,
    /// The icons the reference picker's spell and item rows draw.
    pub thumbnails: &'a mut Thumbnails,
    pub now: f64,
}

/// What a relation needs of the thing that holds it, which is a different
/// column for each kind of holder — see [`holder_window`].
pub enum HolderIs {
    /// A creature's `npc_flags`, and its template's key for writing them.
    Creature { npc_flags: u32, template_key: vale_mangos::row::Key },
    /// A game object's `type`, and its template's key for writing it.
    Object { kind: u32, template_key: vale_mangos::row::Key },
}

/// `GAMEOBJECT_TYPE_QUESTGIVER`, from `GameObjectDefines.h:29`.
const OBJECT_QUESTGIVER: u32 = 2;

/// `UNIT_NPC_FLAG_QUESTGIVER`, from `UnitDefines.h:658`.
const QUESTGIVER: u32 = 0x2;

/// The quests one creature or game object gives and takes, in a window
/// over the world.
///
/// The relation tables name a template entry, so the window says before
/// anything else that it is about every spawn of the thing and not the one that
/// was clicked.
///
/// The flag is checked here because the server does not check it.
/// `LoadCreatureQuestRelations` has the test for `UNIT_NPC_FLAG_QUESTGIVER`
/// commented out (`ObjectMgr.cpp:8966`), so a relation on a creature without
/// the flag loads without a word — and the 1.12 client, which decides what a
/// right-click on a unit does from its npc flags, never asks that creature for
/// its quests. The window offers to set it.
///
/// A game object's counterpart is its type. `LoadGameobjectQuestRelations`
/// logs a relation on an object that is not `GAMEOBJECT_TYPE_QUESTGIVER`
/// (`ObjectMgr.cpp:8936`) and loads it anyway, and the client offers quests
/// only from that type. The window says so, and offers the type — which
/// renames the object's `data` columns, so it is a button and not a default.
pub fn holder_window(ctx: &egui::Context, mut subject: HolderQuests<'_>) -> Option<egui::Rect> {
    let (holder, entry, name) = subject.quests.window_for.clone()?;
    let (template, noun) = match holder {
        Holder::Creature => (vale_mangos::creature::TEMPLATE, "creature"),
        Holder::Object => (vale_mangos::gameobject::TEMPLATE, "game object"),
    };
    let mut keep_open = true;
    let mut switch: Option<Tool> = None;
    let shown = egui::Window::new(format!("{name} \u{2014} quests"))
        .id(egui::Id::new("holder-quests"))
        .open(&mut keep_open)
        .default_size([380.0, 420.0])
        .default_pos([300.0, 120.0])
        .resizable(true)
        .frame(
            egui::Frame::default()
                .fill(theme::SHELL)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(format!(
                    "{template} entry {entry}: every spawn of it gives and takes these."
                ))
                .small()
                .color(theme::WARN),
            );
            if let Some(trouble) = subject.quests.trouble.clone() {
                ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
                return;
            }
            if subject.quests.all.is_empty() {
                theme::waiting(ui, "reading the quest tables\u{2026}");
                return;
            }
            let all = subject.quests.relations_of_holder(
                holder,
                entry,
                &subject.session.server_edits,
            );
            let kept = all.iter().any(|(_, life)| *life != Life::Delete);
            if kept {
                not_a_questgiver(ui, &mut subject, entry);
            }
            // The workspace's `relation_row` takes a `Workspace`, of which this
            // path would read only four fields, and the rest are not reachable
            // from here, so the window draws its rows itself with `window_row`.
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for role in [Role::Gives, Role::Takes] {
                        theme::heading(
                            ui,
                            match role {
                                Role::Gives => "Gives",
                                Role::Takes => "Takes",
                            },
                        );
                        let here: Vec<(Relation, Life)> = all
                            .iter()
                            .copied()
                            .filter(|(relation, _)| relation.role == role)
                            .collect();
                        if here.is_empty() {
                            theme::note(ui, "None.");
                        }
                        for (relation, life) in here {
                            window_row(ui, &mut subject, relation, life, &mut switch);
                        }
                        if ui
                            .small_button("+ quest\u{2026}")
                            .on_hover_text(format!(
                                "Add a row to {}: a quest this {noun} {}.",
                                crate::tools::quests::table_of(holder, role),
                                role.word()
                            ))
                            .clicked()
                        {
                            subject.quests.picker = Some(Picker::new(
                                Target::Quest,
                                PickFor::QuestOf { holder, id: entry, role },
                            ));
                        }
                    }
                });
            ui.add_space(6.0);
            if ui
                .button("Open in the quest workspace")
                .on_hover_text(format!(
                    "Switch to Quests with the list narrowed to this {noun}'s quests, \
                     where each one's text, objectives and rewards are edited.",
                ))
                .clicked()
            {
                subject.quests.show_quests_of(holder, entry);
                subject.quests.open = all.first().map(|(relation, _)| relation.quest);
                switch = Some(Tool::Quests);
            }
        });
    if !keep_open {
        subject.quests.window_for = None;
    }
    if switch.is_some() {
        *subject.switch_to = switch;
    }
    picker(
        ctx,
        subject.session,
        subject.quests,
        subject.assets,
        subject.thumbnails,
        None,
        subject.now,
    );
    shown.map(|shown| shown.response.rect)
}

/// Say when the holder is not one the client asks for quests, and offer the
/// edit that makes it one — see [`holder_window`], where the two rules are.
fn not_a_questgiver(ui: &mut egui::Ui, subject: &mut HolderQuests<'_>, entry: u32) {
    match &subject.is {
        HolderIs::Creature { npc_flags, template_key } => {
            if npc_flags & QUESTGIVER != 0 {
                return;
            }
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "npc_flags has no QUESTGIVER bit, so the client will not ask this \
                     creature for its quests.",
                )
                .small()
                .color(theme::BAD),
            );
            if ui
                .small_button("Set QUESTGIVER")
                .on_hover_text(
                    "Write npc_flags with 0x2 added into creature_template. It is a \
                     creature edit: it is applied from the Creatures block of the Server \
                     panel and needs the server restarted.",
                )
                .clicked()
            {
                let subject_line = format!("creature {entry} npc_flags");
                subject.session.set_server_edit(
                    vale_mangos::creature::TEMPLATE,
                    template_key,
                    "npc_flags",
                    Some((npc_flags | QUESTGIVER).to_string()),
                    Some(crate::session::Gesture {
                        label: "Edit creature",
                        subject: &subject_line,
                        now: subject.now,
                    }),
                );
            }
        }
        HolderIs::Object { kind, template_key } => {
            if *kind == OBJECT_QUESTGIVER {
                return;
            }
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!(
                    "type is {}, not Quest giver, so the client will not ask this object \
                     for its quests and the server logs each relation at start.",
                    vale_mangos::gameobject::value_word(
                        &vale_mangos::gameobject::TYPES,
                        *kind
                    )
                ))
                .small()
                .color(theme::BAD),
            );
            if ui
                .small_button("Set type to Quest giver")
                .on_hover_text(
                    "Write type = 2 into gameobject_template. The 24 data columns are \
                     named by the type, so what they hold now is read as a quest giver's \
                     fields afterwards: check them on the object's form. It is a game \
                     object edit: it is applied from the Objects block of the Server panel \
                     and needs the server restarted.",
                )
                .clicked()
            {
                let subject_line = format!("gameobject {entry} type");
                subject.session.set_server_edit(
                    vale_mangos::gameobject::TEMPLATE,
                    template_key,
                    "type",
                    Some(OBJECT_QUESTGIVER.to_string()),
                    Some(crate::session::Gesture {
                        label: "Edit game object",
                        subject: &subject_line,
                        now: subject.now,
                    }),
                );
            }
        }
    }
}

/// One quest in the holder's window: its title as a link into the workspace,
/// its level, and the button that removes or keeps the relation.
fn window_row(
    ui: &mut egui::Ui,
    subject: &mut HolderQuests<'_>,
    relation: Relation,
    life: Life,
    switch: &mut Option<Tool>,
) {
    let known = subject
        .quests
        .shown_entry(relation.quest, &subject.session.server_edits);
    ui.horizontal(|ui| {
        let title = known
            .as_ref()
            .map(|known| known.title.clone())
            .unwrap_or_else(|| format!("quest {}", relation.quest));
        let colour = match life {
            Life::Delete => theme::BAD,
            _ => theme::ACCENT,
        };
        if ui
            .add(egui::Link::new(egui::RichText::new(title).color(colour)))
            .on_hover_text("open this quest in the quest workspace")
            .clicked()
        {
            subject.quests.open = Some(relation.quest);
            subject.quests.show_quests_of(relation.holder, relation.id);
            *switch = Some(Tool::Quests);
        }
        if let Some(known) = known.as_ref() {
            ui.label(
                egui::RichText::new(format!("level {}", known.level))
                    .small()
                    .color(theme::INK_DIM),
            );
        }
        ui.label(theme::number(relation.quest.to_string()).color(theme::INK_FAINT));
        match life {
            Life::Insert => {
                ui.label(egui::RichText::new("new").small().color(theme::WARN));
            }
            Life::Delete => {
                ui.label(
                    egui::RichText::new("marked for removal")
                        .small()
                        .color(theme::BAD),
                );
            }
            Life::Update => {}
        }
        let (label, about) = match life {
            Life::Delete => ("keep", "Take the removal mark off this row."),
            _ => (
                "\u{d7}",
                "Remove this relation. The quest itself is not touched.",
            ),
        };
        if ui.small_button(label).on_hover_text(about).clicked() {
            let now = subject.now;
            match life {
                Life::Delete => subject.quests.relate(subject.session, relation, now),
                _ => subject.quests.unrelate(subject.session, relation, now),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text codes a preview resolves, in both cases, and one it does not
    /// know left as it is.
    #[test]
    fn the_text_codes_are_resolved_for_a_preview() {
        assert_eq!(preview_text("Hello, $N.$B$BGo."), "Hello, <name>.\n\nGo.");
        assert_eq!(
            preview_text("young $c of the $R"),
            "young <class> of the <race>"
        );
        assert_eq!(preview_text("Well met, $Gsir:madam;!"), "Well met, sir!");
        assert_eq!(preview_text("costs $5"), "costs $5");
        assert_eq!(preview_text("trailing $"), "trailing $");
    }

    /// A numbered run draws its used slots and one free one, and never more
    /// than it has.
    #[test]
    fn a_run_draws_the_used_slots_and_one_free() {
        assert_eq!(slots_to_draw(&[false, false, false, false]), 1);
        assert_eq!(slots_to_draw(&[true, false, false, false]), 2);
        // A gap is drawn through: slot 3 is used, so 1..=4 are shown.
        assert_eq!(slots_to_draw(&[true, false, true, false]), 4);
        assert_eq!(slots_to_draw(&[true, true, true, true]), 4);
    }

    /// Every column a composed line names is a column of the table, so a
    /// renamed column upstream fails here rather than silently dropping a line
    /// from the form.
    #[test]
    fn every_composed_line_names_real_columns() {
        for (stem, slots) in [
            ("ReqCreatureOrGOId", 4),
            ("ReqCreatureOrGOCount", 4),
            ("ReqSpellCast", 4),
            ("ObjectiveText", 4),
            ("ReqItemId", 4),
            ("ReqItemCount", 4),
            ("ReqSourceId", 4),
            ("ReqSourceCount", 4),
            ("RewChoiceItemId", 6),
            ("RewChoiceItemCount", 6),
            ("RewItemId", 4),
            ("RewItemCount", 4),
            ("RewRepFaction", 5),
            ("RewRepValue", 5),
            ("DetailsEmote", 4),
            ("DetailsEmoteDelay", 4),
            ("OfferRewardEmote", 4),
            ("OfferRewardEmoteDelay", 4),
        ] {
            for slot in 1..=slots {
                assert!(numbered(stem, slot).is_some(), "{stem}{slot}");
            }
            assert!(
                numbered(stem, slots + 1).is_none(),
                "{stem} has more than {slots}"
            );
        }
        for name in [
            "SrcItemId",
            "SrcItemCount",
            "RequiredSkill",
            "RequiredSkillValue",
            "RequiredMinRepFaction",
            "RequiredMinRepValue",
            "RequiredMaxRepFaction",
            "RequiredMaxRepValue",
            "Title",
            "Details",
            "Objectives",
            "EndText",
            "QuestLevel",
            "Type",
            "SuggestedPlayers",
            "RewOrReqMoney",
            "RewXP",
            "RewSpell",
            "PrevQuestId",
            "NextQuestId",
            "NextQuestInChain",
        ] {
            assert!(quest::column(quest::TEMPLATE, name).is_some(), "{name}");
        }
    }

    /// Every section the form draws covers every group the table uses, so
    /// no column is left off the form.
    #[test]
    fn every_group_of_the_table_is_a_section() {
        for column in quest::TEMPLATE_COLUMNS.iter() {
            assert!(
                SECTIONS.contains(&column.group),
                "{} is in {}, which the form does not draw",
                column.name,
                column.group.name()
            );
        }
    }

    /// Every table a quest column references is one this panel can either name
    /// a row of or deliberately draws as a number.
    #[test]
    fn every_reference_has_a_target() {
        for column in quest::TEMPLATE_COLUMNS.iter() {
            let tables: Vec<&'static str> = match column.kind {
                Kind::Ref(table) => vec![table],
                Kind::Either(positive, negative) => vec![positive, negative],
                _ => Vec::new(),
            };
            for table in tables {
                assert!(target_of(table).is_some(), "{} names {table}", column.name);
            }
        }
    }
}
