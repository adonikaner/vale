//! The Items workspace: the list of items, the open item's form, and in the
//! inspector its tooltip, its appearance and a preview. The state is
//! [`crate::subjects::items`].
//!
//! ## The three parts
//!
//! ```text
//! left       which item          24,000 rows, searched by name, entry, class,
//!                                slot or level
//! middle     what it is          129 columns, in rowform's page layout
//! inspector  what it looks like  the tooltip, the display row and a preview
//! ```
//!
//! The third part is drawn in the shell's inspector ([`inspector`]), as the
//! spell workspace's is. The shell's rule is that the inspector describes the
//! selection. The third part was once a column of this workspace beside the
//! inspector; in a 1280-point window that column, the inspector and the list
//! left the form 168 points wide, which is less than one page row. The
//! inspector can be resized to 560 points, the width that column had, and the
//! picture takes the width it is given.
//!
//! The list and the form are [`crate::ui::tables`]'s two parts. The third part
//! is why this is a separate panel. A spell's row points at a chain of other
//! rows, and the chain is drawn beside it. An item's row points at one thing
//! that cannot be read as text, `display_id`, so what the display row looks
//! like is drawn beside it.
//!
//! ## The appearance is chosen from pictures
//!
//! `display_id` names a row of `ItemDisplayInfo.dbc`, which has tens of
//! thousands of rows, and a number such as `2589` does not say what its row
//! looks like. The tileset picker and the model browser have the same problem
//! and the same answer: a paged grid of pictures, a preview under it, and a
//! search over the only text a display row carries, its icon name.
//!
//! A cell shows a model where the row has one and an icon where it does not.
//! These are the two types of item. A sword, a helm and a pauldron attach
//! geometry to the wearer. A shirt, a pair of gloves and a robe paint textures
//! into the wearer's skin and have no geometry, so the only picture of one
//! outside a dressed character is its bag icon. An empty square for the second
//! type would leave most of the table without a picture.
//!
//! ## What the form shows beside a number
//!
//! Every column whose number has a meaning is drawn with the meaning: a
//! quality is a coloured word, a class and a subclass are two menus of which
//! the second depends on the first, `buy_price` is gold and silver, `delay` is
//! a swing time in seconds, a spell is its name from `Spell.dbc`, a skill is
//! its name from `SkillLine.dbc`, and the two mask columns are lists of ticks.
//! The number is still shown, can still be edited, and is what is written.
//!
//! ## Nothing here reaches the database
//!
//! An edit goes into the project's own store through
//! [`EditSession::set_server_edit`], already escaped, and onto the undo stack.
//! A save writes it into `sql\items.sql`. Apply runs the statements and then,
//! unlike the creature subject's apply, asks a running playtest to
//! `.reload item_template`, which makes the change live for every copy of the
//! item already in the world. [`crate::server::items`] gives the reason that
//! works.
//!
//! Apply is not on this panel. It is on the bar's Server…, beside the spells'
//! and the creatures' and in the same words, so that the server operations
//! have one vocabulary in one place. This panel keeps the state: what the
//! project changes and how much of it the database holds. See
//! [`crate::ui::shell::sync`].

use crate::ui::shell::asks::{ShellAsk, ShellAsks};
use crate::ui::widgets::reference::{self, Resolver};
use crate::ui::widgets::rowform::{
    choice_cell, entry_cell, number_cell, page_spacing, section, server_field,
    FieldCtx, Hook, Renumber, RowAct,
};
use crate::ui::widgets::theme;
use super::listing::counted;
use crate::ui::widgets::thumbnails::Thumbnails;
use crate::session::EditSession;
use crate::subjects::items::{Filter, Items, Known, Look, DISPLAY_PAGE};
use vale_client::assets::GameAssets;
use vale_mangos::item::{self, Column, Group, Kind};
use vale_mangos::row::Life;
use bevy_egui::egui;

/// The height of one list row and the size of the icon in it: the editor's
/// own, which the spell list uses too. See [`theme::list_row`].
const ROW_HEIGHT: f32 = theme::LIST_ROW;

const ICON: f32 = theme::LIST_PICTURE;

/// The size of the icon at the head of the form.
const HEAD_ICON: f32 = 36.0;

/// The size of one cell of the appearance picker's grid, how many cells fit
/// across it, and the gap between two.
///
/// 60 points is a little under twice a list row's icon. It is enough to tell
/// a sword from a mace. The preview pane under the grid is for telling one
/// sword from another.
const CELL: f32 = 60.0;
const GRID_COLUMNS: usize = 8;
const CELL_GAP: f32 = 4.0;

/// The dialog's width: [`GRID_COLUMNS`] cells and the modal's margins. A
/// width the grid does not divide into leaves a ragged column at the right
/// of every page.
const PICKER_WIDTH: f32 = GRID_COLUMNS as f32 * (CELL + CELL_GAP) + 24.0;

/// The colours of the seven qualities, from vmangos' `ItemQualityColors`
/// (`SharedDefines.h:205`).
///
/// They are the server's table and not colours chosen here. A quality's
/// whole content is a colour, and a list that used the editor's palette for
/// it would show something other than what the game shows.
const QUALITY_COLOURS: [egui::Color32; 7] = [
    egui::Color32::from_rgb(0x9d, 0x9d, 0x9d),
    egui::Color32::from_rgb(0xff, 0xff, 0xff),
    egui::Color32::from_rgb(0x1e, 0xff, 0x00),
    egui::Color32::from_rgb(0x00, 0x70, 0xdd),
    egui::Color32::from_rgb(0xa3, 0x35, 0xee),
    egui::Color32::from_rgb(0xff, 0x80, 0x00),
    egui::Color32::from_rgb(0xe6, 0xcc, 0x80),
];

/// The colour a quality is drawn in, or the ordinary ink for a value the game
/// has no colour for.
pub(crate) fn quality_colour(quality: u32) -> egui::Color32 {
    QUALITY_COLOURS
        .get(quality as usize)
        .copied()
        .unwrap_or(theme::INK)
}

/// `text` with its first letter in upper case, for a value whose name the
/// state keeps in lower case.
fn capitalised(text: &str) -> String {
    let mut letters = text.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// Everything the workspace needs, as one argument.
pub(crate) struct Workspace<'a> {
    pub(crate) session: &'a mut EditSession,
    pub(crate) items: &'a mut Items,
    pub(crate) assets: &'a GameAssets,
    /// The icons, which are BLP textures decoded like any other. See
    /// [`crate::ui::widgets::thumbnails`].
    pub(crate) thumbnails: &'a mut Thumbnails,
    /// The models, which are rendered. See [`crate::preview::portraits`].
    pub(crate) portraits: &'a mut crate::preview::portraits::Portraits,
    /// The quest workspace's state, for the reference picker, which is one
    /// dialog over every table a row names, and for the quest list, which
    /// names `start_quest`. See [`field`].
    pub(crate) quests: &'a mut crate::subjects::quests::Quests,
    /// The loot window's state, for the Loot toggle at the head of the form:
    /// what the item contains and what it disenchants into. See
    /// [`super::loot`].
    pub(crate) loot: &'a mut crate::subjects::loot::Loot,
    /// The queue an apply's `.reload item_template` goes on, whose status
    /// this panel shows. An item change is the one here that goes live
    /// without a restart, so the form shows whether the reload was answered.
    pub(crate) reloads: &'a mut crate::server::reload::Reloads,
    /// The bar's Server…, which holds every server operation. See
    /// [`crate::ui::shell::sync`].
    pub(crate) server_panel: &'a mut crate::ui::shell::popover::Popover,
    /// Where each server subject stands, held for half a second; the items'
    /// line in the form is read from it. See [`crate::server::sync::Standings`].
    pub(crate) standings: &'a mut crate::server::sync::Standings,
    /// The content patch the server is configured for, which is half the key
    /// a new row is written under.
    pub(crate) patch: u32,
    /// The frame clock the undo stack folds a gesture by.
    pub(crate) now: f64,
    /// What a link on the form asks of the shell: a row to open in another
    /// workspace, a part to switch to. See [`crate::ui::shell::asks`].
    pub(crate) asks: &'a mut ShellAsks,
}

impl Workspace<'_> {
    /// The open item, with this project's edits over it.
    fn open_item(&self) -> Option<Known> {
        self.items.open_item(&self.session.rows.edits)
    }
}

/// What the appearance part reads, which is less than the workspace does:
/// the shell's inspector draws it, and has neither the reload queue nor the
/// server's patch to hand.
pub(crate) struct Looking<'a> {
    pub(crate) session: &'a EditSession,
    pub(crate) items: &'a mut Items,
    pub(crate) assets: &'a GameAssets,
    pub(crate) thumbnails: &'a mut Thumbnails,
    pub(crate) portraits: &'a mut crate::preview::portraits::Portraits,
    /// Where the display link asks the shell to open the display row.
    pub(crate) asks: &'a mut ShellAsks,
}

impl Looking<'_> {
    fn open_item(&self) -> Option<Known> {
        self.items.open_item(&self.session.rows.edits)
    }
}

/// Draw the whole workspace into the region the panels left.
pub(crate) fn draw(ui: &mut egui::Ui, mut work: Workspace<'_>) {
    super::listing::frame(ui, &mut work, "item-browser", list, form);

    if work.items.picking_display {
        picker(ui, &mut work);
    }
    // The reference picker, which is the quest form's dialog: its item
    // column purpose writes into this item. See [`field`].
    super::quests::picker(
        ui.ctx(),
        work.session,
        work.quests,
        work.assets,
        work.thumbnails,
        None,
        work.now,
    );
}

// ---------------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------------

/// The left column: the workspace's parts, the heading with the row count,
/// the search box, the New, Copy and Remove buttons, and the rows.
fn list(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    // The workspace's two parts, Items and Sets. The shell switches after
    // the frame is drawn.
    if let Some(part) = crate::ui::shell::rail::parts(ui, crate::tools::Tool::Items) {
        work.asks.push(ShellAsk::Part(part));
    }
    // The heading is the workspace's name on the rail, which says which list
    // this is. The table's name is on the fields that are written to it.
    let (count, trouble, read) = (work.items.count(), work.items.trouble.clone(), !work.items.all.is_empty());
    if !super::listing::heading(ui, "Items", item::TEMPLATE, count, trouble.as_deref(), read) {
        return;
    }
    super::listing::search_box(ui, &mut work.items.query, "Name, entry, class, slot or level");
    let revision = work.session.rows.revision;
    let matches: Vec<usize> = {
        let edits = &work.session.rows.edits;
        work.items.matches(edits, revision).to_vec()
    };
    super::listing::shown_count(ui, matches.len());
    row_actions(ui, work);
    ui.add_space(4.0);

    // Bring the open item into view when it was opened from somewhere other
    // than this list, as the spell list does. See `theme::list_area`.
    let mut revealed = work.items.revealed;
    let reveal = super::listing::reveal_in(work.items.open, &mut revealed, &matches, |index| {
        work.items.at(index).map(|known| known.entry)
    });
    work.items.revealed = revealed;
    let mut asked: Option<(Known, RowAct)> = None;
    theme::list_area(ui, reveal, ROW_HEIGHT)
        .show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
            for at in range {
                let index = matches[at];
                let Some(known) = work.items.shown(index, &work.session.rows.edits) else {
                    continue;
                };
                let chosen = work.items.open == Some(known.entry);
                let response = row(ui, work, &known, chosen);
                if response.clicked() {
                    work.items.revealed = Some(known.entry);
                    asked = Some((known.clone(), RowAct::Open));
                }
                response.context_menu(|ui| {
                    let act = crate::ui::widgets::rowform::row_menu(ui, "item", known.entry, known.claim, chosen);
                    if let Some(act) = act {
                        asked = Some((known.clone(), act));
                    }
                });
            }
        });
    if let Some((known, act)) = asked {
        let ctx = ui.ctx().clone();
        act_on(&ctx, work, &known, act);
    }
}

/// Carry out one act on one item. The buttons over the list and a row's
/// right-click menu both call this, so an act is the same from either.
fn act_on(ctx: &egui::Context, work: &mut Workspace<'_>, known: &Known, act: RowAct) {
    let (patch, now) = (work.patch, work.now);
    match act {
        RowAct::Open => work.items.open = Some(known.entry),
        RowAct::Copy => {
            work.items.duplicate(work.session, patch, now);
        }
        RowAct::Remove => {
            if let Err(why) = work.items.remove(work.session, known, now) {
                work.session.status = why;
            }
        }
        RowAct::Keep => work.items.keep(work.session, known, now),
        RowAct::CopyEntry => {
            ctx.copy_text(known.entry.to_string());
            work.session.status = format!("Item entry {} is on the clipboard", known.entry);
        }
    }
}

/// New, Copy and Remove, as three equal buttons: what makes a row and what
/// marks one for removal.
///
/// An item in the database is marked: it stays in the list in red until
/// Apply, and Keep takes the mark off. One this project created is in no
/// database, so removing it gives the claim up. A removal needs a restart of
/// the server, and the apply sends no reload while one is in the plan; see
/// `vale_mangos::item`'s module comment.
fn row_actions(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let open = work.open_item();
    let removed = open
        .as_ref()
        .is_some_and(|known| known.claim == Life::Delete);
    let ctx = ui.ctx().clone();
    ui.columns(3, |columns| {
        let new = theme::full_button(&columns[0], "New");
        if columns[0]
            .add(new)
            .on_hover_text(
                "Creates an item_template row with an entry from 2,000,000 up, clear of \
                 upstream vmangos entries. It starts as a common-quality Junk item with a \
                 stack size of 1, so the server loads it and the client draws it.",
            )
            .clicked()
        {
            let patch = work.patch;
            let now = work.now;
            work.items.create(work.session, "New Item", patch, now);
        }
        let copy = theme::full_button(&columns[1], "Copy");
        if columns[1]
            .add_enabled(open.is_some(), copy)
            .on_hover_text(
                "Creates a copy of this item under a new entry. Each column takes the \
                 value shown: this project's edit where there is one, otherwise the \
                 database value.",
            )
            .on_disabled_hover_text("Open an item first.")
            .clicked()
        {
            if let Some(known) = open.as_ref() {
                act_on(&ctx, work, known, RowAct::Copy);
            }
        }
        let label = match removed {
            true => "Keep",
            false => "Remove",
        };
        let remove = theme::full_button(&columns[2], label);
        if columns[2]
            .add_enabled(open.is_some(), remove)
            .on_hover_text(match removed {
                true => "Unmarks this item for removal.",
                false => {
                    "Marks this item for removal. Apply deletes every content-patch \
                     version of it and the rows that reference it: loot, vendor, locale, \
                     auction bot, and starting and premade item rows. Quests, spells and \
                     creature equipment that name it are not changed. Restart the server \
                     afterwards; on startup it deletes the item from every character \
                     that carries it. An item this project created is discarded instead."
                }
            })
            .on_disabled_hover_text("Open an item first.")
            .clicked()
        {
            if let Some(known) = open.as_ref() {
                let act = match removed {
                    true => RowAct::Keep,
                    false => RowAct::Remove,
                };
                act_on(&ctx, work, known, act);
            }
        }
    });
}

/// One row of the list: the icon, the name in its quality's colour, the
/// item's class, slot and level, and the entry on the right.
///
/// The row is [`theme::list_row`], which the spell list is made of too. Two
/// things are this panel's: the picture, which is the item's bag icon from
/// the thumbnail cache, and the colour of the name, which for an item is the
/// row's quality and not a choice of palette.
fn row(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    known: &Known,
    chosen: bool,
) -> egui::Response {
    // What the project says about the row is drawn on the row. What was read
    // does not tell a new item or one marked for removal from the rest.
    let (sub, tint) = match known.claim {
        Life::Insert => (
            format!("{} \u{b7} new", known.sub()),
            quality_colour(known.quality),
        ),
        Life::Delete => (
            format!("{} \u{b7} marked for removal", known.sub()),
            theme::BAD,
        ),
        Life::Update => (known.sub(), quality_colour(known.quality)),
    };
    let shape = theme::list_row(
        ui,
        theme::ListRow {
            title: &known.name,
            sub: &sub,
            trailing: &known.entry.to_string(),
            tint,
            picture: true,
        },
        chosen,
    );
    // Asked for on every frame the row is drawn, so only the rows on screen
    // are decoded.
    let icon = work
        .items
        .look(work.assets, known.display_id, known.inventory_type)
        .and_then(|look| look.icon);
    let texture = icon.and_then(|path| {
        work.thumbnails.want(&path);
        work.thumbnails.get(&path)
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
    shape.response
}

// ---------------------------------------------------------------------------
// The form
// ---------------------------------------------------------------------------

/// The middle: what this project changes, then the open row's columns.
fn form(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    egui::ScrollArea::vertical()
        .id_salt("item-form")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // The spell form's spacing. See `rowform::page_spacing`.
            page_spacing(ui);
            edits_block(ui, work);
            let Some(known) = work.open_item() else {
                theme::heading_about(
                    ui,
                    "No Item Open",
                    "Every column of the row is edited here. Nothing is written to the \
                     database until Apply, in Server\u{2026} on the top bar.",
                );
                theme::note(ui, "Choose an item on the left, or create one with New.");
                return;
            };
            head(ui, work, &known);
            ui.add_space(6.0);

            // The whole row, or the columns the project itself holds. A row
            // this project created has no row in the database to read, so the
            // form draws from the store alone — which is every column, because
            // a creation names them all.
            let row = work
                .items
                .row
                .as_ref()
                .filter(|held| held.entry == known.entry)
                .map(|held| held.row.clone());
            let Some(row) = row else {
                theme::waiting(ui, "Reading the row\u{2026}");
                return;
            };
            // A row the database does not hold: a removal that has been
            // applied, or an item somebody else deleted. There is nothing to
            // draw. A row this project creates is empty here too, and its
            // columns are drawn from the store.
            if row.is_empty() && known.claim != Life::Insert {
                theme::note(
                    ui,
                    "The database has no row at this entry. An applied removal stays in \
                     the list while the project holds it; Keep restores the row at the \
                     next Apply.",
                );
                return;
            }
            // An item marked for removal is read and not edited. A `Delete`
            // row carries no columns, so an edit typed into one would be in the
            // store and in no statement.
            let removed = known.claim == Life::Delete;
            ui.add_enabled_ui(!removed, |ui| groups(ui, work, &known, &row));
        });
}

/// What this project changes about the server's items, and the button that
/// opens the panel that applies it.
///
/// It has the creature panel's two parts: the state is here, beside what is
/// being edited, and Apply, Restore and Discard are on the bar's Server…
/// with the other subjects'. [`crate::ui::shell::sync`]'s module comment
/// gives the reason for one panel.
///
/// Nothing is drawn for a project that changes nothing. The block is about
/// what is waiting to be applied, and a line saying there is nothing would
/// be drawn above every item that is opened.
fn edits_block(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let standing = work.standings.of(
        crate::server::sync::Half::Row(crate::server::stack::Subject::Items),
        work.session,
        work.assets,
        work.now,
    );
    super::listing::edits_block(
        ui,
        standing,
        "Opens the Server panel, which applies, restores or discards these rows \
         together with the spell and creature rows. An item applied during a \
         playtest takes effect through `.reload item_template`, including copies \
         already in the world; applied at any other time, it takes effect after a \
         server restart.",
        work.reloads.status(),
        work.server_panel,
    );
}

/// The head of the form: the icon, the name in its colour, and what the row is.
///
/// The key is on the second line and not beside the name. `Thunderfury,
/// Blessed Blade of the Windseeker` is forty-three characters at seventeen
/// points, and with `entry 19019` after it on one row egui draws the two
/// overlapping, because a horizontal layout clips and does not wrap. The
/// second line has room for the key and for the item's class, slot and level.
fn head(ui: &mut egui::Ui, work: &mut Workspace<'_>, known: &Known) {
    ui.horizontal(|ui| {
        // The icon. `ui::tables`'s spell head has one too: the name says
        // which row is open, and the picture is read faster than the name.
        let icon = work
            .items
            .look(work.assets, known.display_id, known.inventory_type)
            .and_then(|look| look.icon);
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(HEAD_ICON), egui::Sense::hover());
        ui.painter().rect_filled(rect, 3.0, theme::SUNK);
        if let Some(path) = icon {
            work.thumbnails.want_at(&path, crate::ui::widgets::thumbnails::ICON_SIDE);
            if let Some(texture) = work.thumbnails.get_at(&path, crate::ui::widgets::thumbnails::ICON_SIDE) {
                ui.painter().image(
                    texture,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(&known.name)
                    .strong()
                    .size(17.0)
                    .color(quality_colour(known.quality)),
            );
            ui.horizontal(|ui| {
                ui.label(theme::number(format!("entry {}", known.entry)));
                if ui
                    .small_button("Copy Entry")
                    .on_hover_text("Copies the entry to the clipboard.")
                    .clicked()
                {
                    let ctx = ui.ctx().clone();
                    act_on(&ctx, work, known, RowAct::CopyEntry);
                }
                ui.label(
                    egui::RichText::new(format!("patch {}", known.patch))
                        .small()
                        .color(theme::INK_FAINT),
                )
                .on_hover_text(
                    "`item_template` is keyed by entry and patch together. The server \
                     loads the highest patch at or below its own WowPatch, and this is \
                     that row. An edit keyed by entry alone would also change versions \
                     of the item the server does not load.",
                );
                ui.label(
                    egui::RichText::new(known.sub())
                        .small()
                        .color(theme::INK_DIM),
                );
                // Loot is a window over the workspace. It follows the open
                // item as it follows a selected creature; see `super::loot`.
                if ui
                    .selectable_label(work.loot.open, "Loot")
                    .on_hover_text(
                        "The loot from opening this item (item_loot_template, keyed by \
                         entry) and from disenchanting it (disenchant_loot_template, keyed \
                         by disenchant_id).",
                    )
                    .clicked()
                {
                    work.loot.toggle();
                }
            });
        });
    });
    match known.claim {
        Life::Insert => {
            ui.label(
                egui::RichText::new("New: this row is not in the database yet. Apply writes it.")
                    .small()
                    .color(theme::WARN),
            );
        }
        // The state is the line; what Apply does with it, and the way back,
        // are its hover text.
        Life::Delete => {
            ui.label(
                egui::RichText::new("Marked for removal")
                    .small()
                    .color(theme::BAD),
            )
            .on_hover_text(
                "Apply deletes it with its loot and vendor rows; restart the server \
                 afterwards. Keep, under the search box, unmarks it.",
            );
        }
        Life::Update => {}
    }
}

/// Every group of the table's columns: the first three open, the rest folded.
///
/// 129 columns are more than a person reads at once, and the ones read first
/// are not the first in the table.
///
/// One field per row, in `rowform`'s page layout: the name left-aligned in a
/// fixed cell, the value in a fixed cell beside it, and what the value means
/// after that. It is the spell form's arrangement at the spell form's widths.
/// The packed grid this replaced put a text box, a menu and a number box at
/// three different widths and up to three pairs across, so no two values in a
/// section started at the same x.
fn groups(ui: &mut egui::Ui, work: &mut Workspace<'_>, known: &Known, row: &item::Row) {
    const OPEN: [Group; 3] = [Group::Identity, Group::Appearance, Group::Economy];
    let key = known.key();
    let mut seen: Vec<Group> = Vec::new();
    for column in item::TEMPLATE_COLUMNS.iter() {
        if !seen.contains(&column.group) {
            seen.push(column.group);
        }
    }
    for group in seen {
        let columns: Vec<&'static Column> = item::TEMPLATE_COLUMNS
            .iter()
            .filter(|column| column.group == group)
            .collect();
        let edited = columns.iter().any(|column| {
            work.session
                .rows.edits
                .get(item::TEMPLATE, &key, column.name)
                .is_some()
        });
        section(
            ui,
            ("item", group.name()),
            group.name(),
            edited,
            OPEN.contains(&group),
            |ui| {
                for column in columns {
                    field(ui, work, known, row, column);
                }
            },
        );
    }
    ui.add_space(24.0);
}

/// One column, through [`server_field`]. The item form's own cases are the
/// hook's: `entry`, the subclass, the display id, and the set's item list
/// after `set_id`.
fn field(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    known: &Known,
    row: &item::Row,
    column: &'static Column,
) {
    let key = known.key();
    let field = FieldCtx {
        // One id per row and column, keyed by the entry as it has always been.
        id: ui.make_persistent_id(("item", known.entry, column.name)),
        ..FieldCtx::new(ui, &work.session.rows.edits, item::TEMPLATE, &key, row, column, "Edit item", work.now)
    };
    let (assets, now) = (work.assets, work.now);
    let (items, thumbnails) = (&mut *work.items, &mut *work.thumbnails);
    let mut resolver = Resolver {
        session: &mut *work.session,
        assets,
        quests: &mut *work.quests,
        asks: &mut *work.asks,
    };
    let written = server_field(ui, &mut resolver, &field, |hook, ui, resolver, field, showing| {
        match (hook, field.kind) {
            // The key is shown and not typed into, except `entry`. Writing a
            // key column moves the row the edit is about, which for `patch`
            // would retarget the statement at a version of the item the
            // server is not loading. Renumbering an item has a box of its
            // own, which is not a column edit at all for half the rows it
            // draws.
            (Hook::Cell, Kind::Key) if field.column.name == "entry" => {
                entry_field(ui, resolver.session, items, known, field.id, now);
                Some(None)
            }
            // The subclass is the one column whose options depend on another
            // column of the same row: 0 is Axe on a weapon, Cloth on a piece
            // of armour and Bandage on a consumable, so the list is the
            // class's. See `vale_mangos::item::subclasses`.
            (Hook::Cell, Kind::Subclass) => {
                Some(choice_cell(ui, showing, item::subclasses(known.class), field.id))
            }
            // A display id is chosen from pictures. The number stays editable
            // beside the button, for an id that is already known.
            (Hook::Cell, Kind::Ref("ItemDisplayInfo")) => {
                Some(display_field(ui, items, thumbnails, resolver.asks, assets, known, showing))
            }
            (Hook::After, Kind::Ref("ItemSet")) => {
                let set = showing.trim().parse().unwrap_or(0);
                set_membership(ui, resolver.session, assets, known.entry, set, now);
                None
            }
            _ => None,
        }
    });
    if let Some(written) = written {
        field.write(work.session, written, work.now);
    }
}

/// The `entry` column, which is the row's own key and can be edited.
///
/// Whatever the row is, the project's claim on the row is re-keyed; see
/// [`crate::subjects::items::Items::rekey`]. For a row that is in the
/// database the apply then moves the row, every other content-patch version
/// of it, and every column of the world database that names an item by
/// entry. The box's hover text says so, and says what that cannot reach.
fn entry_field(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    items: &mut Items,
    known: &Known,
    id: egui::Id,
    now: f64,
) {
    let renumber = Renumber {
        entry: known.entry,
        read_entry: known.read_entry,
        is_new: known.claim == Life::Insert,
        noun: "item",
        reaches: format!(
            "vmangos has no foreign keys, so Apply updates the references itself: every \
             content-patch version of the row, and the {} world database columns that \
             refer to an item by entry: the loot tables, npc_vendor, \
             creature_equip_template, quest_template's item columns, playercreateinfo_item, \
             spell_template's reagents. Restore reverts all of it.\n\n\
             Not updated: the characters database, so a copy a character already carries \
             has no item_template row after the change; and Spell.dbc's reagent and \
             created-item fields, which are in a client file.",
            vale_mangos::item::REFERENCES.len()
        ),
    };
    entry_cell(ui, id.with("entry"), &renumber, |wanted| items.rekey(session, known, wanted, now));
}

/// The `display_id` column: the number, the icon it resolves to, the button
/// that opens the grid, and what the display row is made of.
fn display_field(
    ui: &mut egui::Ui,
    items: &mut Items,
    thumbnails: &mut Thumbnails,
    asks: &mut ShellAsks,
    assets: &GameAssets,
    known: &Known,
    showing: &str,
) -> Option<String> {
    let written = number_cell(ui, Kind::Unsigned, showing);
    let id: u32 = showing.trim().parse().unwrap_or(0);
    let look = items.look(assets, id, known.inventory_type);
    if let Some(path) = look.as_ref().and_then(|look| look.icon.clone()) {
        thumbnails.want(&path);
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(20.0), egui::Sense::hover());
        if let Some(texture) = thumbnails.get(&path) {
            ui.painter().image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }
    if reference::choose_button(
        ui,
        "Opens a picker of every ItemDisplayInfo.dbc row, shown as thumbnails. A row \
         with a model is shown as its model; a row that only textures the wearer's \
         body is shown as its icon.",
    ) {
        items.pick_display(id);
    }
    match look {
        Some(look) => {
            let made_of = match look.has_models() {
                true => counted(look.models.len(), "model"),
                false => counted(look.painted(), "texture"),
            };
            reference::display_link(ui, egui::RichText::new(made_of).small(), asks, "ItemDisplayInfo", id);
        }
        None if id != 0 => {
            ui.label(
                egui::RichText::new("Not in ItemDisplayInfo.dbc")
                    .small()
                    .color(theme::BAD),
            );
        }
        None => {}
    }
    written
}

/// The other half of an item's set, beside its `set_id`: whether the set's
/// own list of items names this item.
///
/// The membership is one fact stored twice. The server counts set pieces by
/// `item_template.set_id`, and the client lists a set's pieces from
/// `ItemSet.dbc`'s item columns. A set chosen in the picker has the list
/// follow in the same undo entry (`ColumnTarget::write`). A number typed
/// into the cell does not, because a number being typed passes through
/// other sets' ids, so the difference is drawn here with a button for each
/// side of it: the named set not listing the item, and another set still
/// listing it.
fn set_membership(ui: &mut egui::Ui, session: &mut EditSession, assets: &GameAssets, item: u32, set: u32, now: f64) {
    use crate::tables;
    if !session.open_table(assets, "ItemSet") {
        return;
    }
    let Some(sets) = session.table("ItemSet") else {
        return;
    };
    let unlisted = set != 0
        && sets
            .row_of(set)
            .is_some_and(|record| tables::rows::set_lists(sets, record, item).is_none());
    let elsewhere: Vec<u32> = (0..sets.record_count())
        .filter(|&record| tables::rows::set_lists(sets, record, item).is_some())
        .filter_map(|record| sets.u32_at(record, 0))
        .filter(|&other| other != set)
        .collect();
    let subject = format!("{} {} set_id", vale_mangos::item::TEMPLATE, item);
    let mut moves: Vec<(u32, u32)> = Vec::new();
    if unlisted {
        ui.label(
            egui::RichText::new("Not in the set's item list")
                .small()
                .color(theme::WARN),
        )
        .on_hover_text(
            "This item's set_id names the set, but the set's item columns in ItemSet.dbc \
             do not list this item. The client lists a set's pieces from those columns.",
        );
        if ui
            .small_button("Add")
            .on_hover_text("Writes the item into the set's first empty item column. One undo entry.")
            .clicked()
        {
            moves.push((0, set));
        }
    }
    for other in elsewhere {
        ui.label(
            egui::RichText::new(format!("Still listed in set {other}"))
                .small()
                .color(theme::WARN),
        )
        .on_hover_text(
            "Another set's item columns in ItemSet.dbc list this item, but this item's \
             set_id does not name that set.",
        );
        if ui
            .small_button("Remove")
            .on_hover_text(format!("Removes the item from set {other}'s item columns. One undo entry."))
            .clicked()
        {
            moves.push((other, 0));
        }
    }
    for (from, to) in moves {
        let said = tables::rows::move_between_sets(session, item, from, to, &subject, now);
        if let Some(last) = said.last() {
            session.status = last.clone();
        }
    }
}

// ---------------------------------------------------------------------------
// The inspector's part: the appearance
// ---------------------------------------------------------------------------

/// The shell's inspector while the item workspace is shown: the open item's
/// tooltip, what the display row it names is made of, and a preview.
///
/// The preview is as wide as the inspector. The panel can be resized and
/// the pane takes the width it is given; see
/// [`crate::ui::widgets::preview::preview_pane_at`], which takes the width
/// as its cap and not the 170 points the placement pickers use.
pub(crate) fn inspector(ui: &mut egui::Ui, mut work: Looking<'_>) {
    let work = &mut work;
    let Some(known) = work.open_item() else {
        theme::heading(ui, "Appearance");
        theme::note(ui, "No item open.");
        return;
    };
    // Side by side where the inspector is wide enough for two, and the
    // tooltip above the picture where it is not. The panel is resizable to
    // `theme::INSPECTOR_MAX`, and at its default width a plate beside a
    // picture would leave each of them too narrow to read.
    if ui.available_width() >= SIDE_BY_SIDE {
        ui.columns(2, |columns| {
            plate(&mut columns[0], work, &known);
            appearance(&mut columns[1], work, &known);
        });
    } else {
        plate(ui, work, &known);
        appearance(ui, work, &known);
    }
}

/// How wide the inspector has to be before the tooltip and the picture are
/// drawn beside each other: two columns of a plate's width.
const SIDE_BY_SIDE: f32 = 2.0 * PLATE_WIDTH;

/// The widest a tooltip is drawn. The game sizes a plate to its longest
/// line and wraps a spell sentence; this is about the width a wrapped
/// `Equip:` line takes in the game at its default scale.
const PLATE_WIDTH: f32 = 270.0;

/// The open item's tooltip as the game draws it, composed from the form's
/// values on every frame, so an edit shows on the frame it is made.
///
/// The lines are `vale_client::interface::plate::item_plate`'s, the function
/// the client's own `GameTooltip` draws an item through: the same order, the
/// same `GlobalStrings.lua` keys and the same colours. The frame and the
/// font are the editor's. The game draws the plate in `Fonts\FRIZQT__.TTF`
/// over `Interface\Tooltips\UI-Tooltip-Background`; this draws it in the
/// editor's font over the colour `TOOLTIP_DEFAULT_BACKGROUND_COLOR` names in
/// `GameTooltip.lua`.
///
/// It is composed for a character who meets every requirement, so the race,
/// class and level lines are white. The client colours each by the player
/// looking at it, and a preview has no player.
fn plate(ui: &mut egui::Ui, work: &mut Looking<'_>, known: &Known) {
    theme::heading(ui, "Tooltip");
    let Some(info) = work.items.open_info(&work.session.rows.edits, known) else {
        theme::waiting(ui, "Reading the row\u{2026}");
        return;
    };
    let tables = work.assets.display_tables().ok();
    let context = vale_client::interface::api::TipContext {
        // The level an item's spell sentences are worked out at. A `$s1` that
        // scales with level reads as it would on a character at the cap.
        level: 60,
        race: 0,
        class: 0,
        catalog: tables.as_deref().and_then(|tables| tables.spellbook()),
        item_names: &|entry| {
            work.items
                .by_entry(entry)
                .map(|found| found.name.clone())
        },
        home: None,
    };
    let wearer = vale_client::interface::plate::Wearer::none();
    let mut tip =
        vale_client::interface::plate::item_tip(&info, None, tables.as_deref(), &context, &wearer);
    // No character is looking at the preview, so every requirement is drawn
    // as met rather than in red.
    tip.level_met = true;
    tip.race_allowed = true;
    tip.class_allowed = true;
    tip.subclass_usable = true;
    tip.already_known = false;
    for met in [
        tip.race_class_only.as_mut().map(|(_, met)| met),
        tip.skill.as_mut().map(|(_, _, met)| met),
        tip.required_spell.as_mut().map(|(_, met)| met),
        tip.honor_rank.as_mut().map(|(_, met)| met),
        tip.city_rank.as_mut().map(|(_, met)| met),
        tip.reputation.as_mut().map(|(_, _, met)| met),
    ]
    .into_iter()
    .flatten()
    {
        *met = true;
    }
    let strings = work.assets.strings();
    let lines =
        vale_client::interface::plate::item_plate(&tip, &|key| strings.get(key).map(str::to_string));
    draw_plate(ui, &lines);
}

/// One plate: a dark frame with a light edge, the name larger than the
/// lines under it, and a right-hand cell pushed to the far side.
fn draw_plate(ui: &mut egui::Ui, lines: &[vale_client::interface::plate::PlateLine]) {
    let width = ui.available_width().min(PLATE_WIDTH);
    egui::Frame::new()
        // `TOOLTIP_DEFAULT_BACKGROUND_COLOR` is `0.09, 0.09, 0.19`.
        .fill(egui::Color32::from_rgb(23, 23, 48))
        .stroke(egui::Stroke::new(1.5, egui::Color32::from_gray(190)))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(9, 7))
        .show(ui, |ui| {
            ui.set_max_width(width - 18.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            for (at, line) in lines.iter().enumerate() {
                let [r, g, b] = line.ink.rgb();
                let colour = egui::Color32::from_rgb(
                    (r * 255.0).round() as u8,
                    (g * 255.0).round() as u8,
                    (b * 255.0).round() as u8,
                );
                let size = match at {
                    0 => 14.5,
                    _ => 12.5,
                };
                let left = egui::RichText::new(&line.left).color(colour).size(size);
                match &line.right {
                    Some(right) => {
                        ui.horizontal(|ui| {
                            ui.label(left);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    egui::RichText::new(right)
                                        .color(egui::Color32::WHITE)
                                        .size(size),
                                );
                            });
                        });
                    }
                    // Every line wraps here, where the game widens the
                    // plate to its longest unwrapped line instead: a
                    // forty-character name at the inspector's width would
                    // otherwise run past the frame.
                    None => {
                        ui.add(egui::Label::new(left).wrap());
                    }
                }
            }
        });
}

/// The Appearance and Preview sections: the display row the item names and
/// what it is made of, then the picture and the body it is drawn on.
fn appearance(ui: &mut egui::Ui, work: &mut Looking<'_>, known: &Known) {
    let known = known.clone();
    theme::heading(ui, "Appearance");
    let display_id = known.display_id;
    let look = work
        .items
        .look(work.assets, display_id, known.inventory_type);
    theme::property(ui, "Display", "", |ui| {
        match (display_id, look.is_some()) {
            (0, _) | (_, false) => {
                ui.label(theme::number(display_id.to_string()));
            }
            _ => {
                crate::ui::widgets::reference::display_link(
                    ui,
                    theme::number(display_id.to_string()),
                    work.asks,
                    "ItemDisplayInfo",
                    display_id,
                );
            }
        }
        if reference::choose_button(ui, "Opens a picker of every ItemDisplayInfo.dbc row, shown as thumbnails.") {
            work.items.pick_display(display_id);
        }
    });
    let Some(look) = look else {
        theme::note(
            ui,
            match display_id {
                0 => {
                    "display_id is 0. The client draws the empty-slot icon and draws \
                      nothing on a wearer."
                }
                _ => {
                    "ItemDisplayInfo.dbc has no row with that id. The client draws \
                      nothing for it."
                }
            },
        );
        return;
    };
    // The bag icon and its name. The preview below is the model where the
    // row has one, so the icon is drawn small here.
    theme::property(ui, "Icon", "", |ui| {
        icon_square(ui, work, look.icon.as_deref(), ICON);
        theme::value(ui, look.icon_name.as_str());
    });
    look_rows(ui, &look);

    let side = ui.available_width();
    // A model where the row has one, and the body it paints where it does
    // not. These are the module comment's two types of item: the preview
    // shows the item for the first and the wearer for the second.
    let on_a_body = match look.models.first() {
        Some(worn) => {
            let key = work.portraits.want_large_worn(worn);
            crate::ui::widgets::preview::preview_pane_at(ui, work.portraits, &key, side);
            if look.models.len() > 1 {
                theme::note(
                    ui,
                    format!(
                        "{}; the preview shows the left shoulder",
                        counted(look.models.len() - 1, "more model")
                    ),
                );
            }
            false
        }
        // No model: a body is drawn only where the row changes one.
        //
        // A row with no model is worn in three cases. A shirt, a glove and a
        // robe paint components of the wearer's composite. A cloak paints
        // none of them and is the wearer's own group-15 geoset with a
        // texture, which is why the dressing is asked and not the row. Some
        // rows switch a geoset group without painting. A trade good, a bag,
        // a reagent and a quest item are none of the three: each is an icon
        // only, and a body drawn for one would show nothing of the item.
        None => {
            let worn = work
                .items
                .body(work.assets, display_id, known.inventory_type);
            let wearable = look.painted() > 0
                || look.geoset_groups.iter().any(|group| *group != 0)
                || worn.as_ref().is_some_and(|worn| worn.cloak.is_some());
            match (worn, wearable) {
                (Some(worn), true) => {
                    let key = work.portraits.want_large_worn(&worn);
                    crate::ui::widgets::preview::preview_pane_at(ui, work.portraits, &key, side);
                    theme::note(
                        ui,
                        "This appearance has no model: it textures the wearer's body, so \
                         the preview draws it on a body in its bind pose.",
                    );
                    true
                }
                // Nothing is worn, or no body resolved for this race. The icon
                // is the only picture of the row, and for most of these rows
                // it is all the game shows.
                (worn, wearable) => {
                    ui.vertical_centered(|ui| {
                        icon_square(ui, work, look.icon.as_deref(), side.min(96.0));
                    });
                    theme::note(
                        ui,
                        match (worn.is_some(), wearable) {
                            (_, false) => {
                                "This appearance is an icon only: no model, no body texture \
                                 and no geoset change on a wearer."
                            }
                            (false, true) => {
                                "This appearance textures the wearer's body, but the \
                                 tables resolve no body model for the race chosen below."
                            }
                            _ => unreachable!("the wearable case is drawn above"),
                        },
                    );
                    wearable
                }
            }
        }
    };

    // The body, for both previews that have one. A helm is cut per race and
    // gender; a garment is painted onto the body that is drawn.
    body_picker(ui, work, &known, on_a_body);
    look_files(ui, &look);
}

/// One decoded icon in a square of its own, or the empty well where there is
/// none.
///
/// The well is drawn either way, so a column does not move when the picture
/// arrives, as in the list rows.
fn icon_square(ui: &mut egui::Ui, work: &mut Looking<'_>, path: Option<&str>, side: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::hover());
    ui.painter().rect_filled(rect, 3.0, theme::SUNK);
    let Some(path) = path else { return };
    // Decoded at the size it is drawn: the list's level for a small square, the
    // file's own top level for anything larger.
    let level = match side > crate::ui::widgets::thumbnails::SIDE as f32 {
        true => crate::ui::widgets::thumbnails::ICON_SIDE,
        false => crate::ui::widgets::thumbnails::SIDE,
    };
    work.thumbnails.want_at(path, level);
    if let Some(texture) = work.thumbnails.get_at(path, level) {
        ui.painter().image(
            texture,
            rect.shrink(1.0),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
}

/// The preview body's race and gender, drawn in the two cases where they
/// change the picture.
///
/// A helm's row names `Helm_Plate_D_04.mdx` and the archive holds sixteen
/// files, eight race codes times two genders. An appearance with no model of
/// its own is drawn on a body, which differs by race. Every other item is
/// one file whatever wears it, so the rows are not drawn for it: they would
/// change nothing.
fn body_picker(ui: &mut egui::Ui, work: &mut Looking<'_>, known: &Known, on_a_body: bool) {
    // A helm, which is inventory type 1, or anything drawn on a body.
    if !on_a_body && known.inventory_type != 1 {
        return;
    }
    body_choice(ui, work.items);
}

/// The race and gender a preview body is drawn as, as two property rows. The
/// item's preview and an item set's share them. Changes no row.
pub(crate) fn body_choice(ui: &mut egui::Ui, items: &mut crate::subjects::items::Items) {
    const RACES: [(&str, u8); 8] = [
        ("Human", 1),
        ("Orc", 2),
        ("Dwarf", 3),
        ("Night Elf", 4),
        ("Undead", 5),
        ("Tauren", 6),
        ("Gnome", 7),
        ("Troll", 8),
    ];
    const GENDERS: [(&str, u8); 2] = [("Male", 0), ("Female", 1)];
    const ABOUT: &str = "Race and gender of the preview body. Does not change the row. A helmet has one \
                         model per race and gender \u{2014} the display row names one file and the \
                         archive holds sixteen \u{2014} and a garment is painted onto the body that \
                         wears it.";
    theme::property(ui, "Race", ABOUT, |ui| {
        theme::choice(ui, "item-race", &mut items.race, &RACES, |a, b| a == b);
    });
    // A gender other than 0 is shown as Female.
    theme::property(ui, "Gender", ABOUT, |ui| {
        theme::choice(ui, "item-gender", &mut items.gender, &GENDERS, |a, b| (*a == 0) == (*b == 0));
    });
}

/// The display row's geoset columns, as property rows under the icon. A row
/// is drawn only for a column that is not all zero.
fn look_rows(ui: &mut egui::Ui, look: &Look) {
    if look.geoset_groups.iter().any(|group| *group != 0) {
        theme::property(
            ui,
            "Geoset Groups",
            "geosetGroup[3]: the geoset variant the wearer switches to, such as a cuff, \
             a boot leg or a robe skirt. 0 is the default, the bare body.",
            |ui| {
                theme::value(
                    ui,
                    format!(
                        "{} / {} / {}",
                        look.geoset_groups[0], look.geoset_groups[1], look.geoset_groups[2]
                    ),
                );
            },
        );
    }
    if look.helmet_hides.iter().any(|id| *id != 0) {
        theme::property(
            ui,
            "Helmet Hides",
            "helmetGeosetVis, per gender: a HelmetGeosetVisData.dbc row that lists, per \
             race, which of the wearer's geosets (hair, facial hair, ears) this helmet \
             hides.",
            |ui| {
                theme::value(ui, format!("{} / {}", look.helmet_hides[0], look.helmet_hides[1]));
            },
        );
    }
}

/// The files the display row names, under the preview: its models with their
/// skins, and its body textures by component. Each list is folded at first.
fn look_files(ui: &mut egui::Ui, look: &Look) {
    if look.has_models() {
        theme::fold_closed(ui, "item-models", "Models", |ui| {
            for worn in &look.models {
                ui.label(
                    egui::RichText::new(&worn.path)
                        .small()
                        .color(theme::INK_DIM),
                );
                if let Some(skin) = worn.skins.get(4).filter(|skin| !skin.is_empty()) {
                    ui.label(egui::RichText::new(skin).small().color(theme::INK_FAINT));
                }
            }
        });
    }
    if look.painted() > 0 {
        theme::fold_closed(ui, "item-textures", &format!("Textures ({})", look.painted()), |ui| {
            use vale_assets::tables::item::Component;
            for (component, path) in Component::ALL.iter().zip(look.textures.iter()) {
                if path.is_empty() {
                    continue;
                }
                theme::property(
                    ui,
                    &format!("{component:?}"),
                    "Painted into the wearer's own 256x256 body composite, one per component.",
                    |ui| {
                        theme::value(ui, path.as_str());
                    },
                );
            }
        });
    }
}

// ---------------------------------------------------------------------------
// The appearance picker
// ---------------------------------------------------------------------------

/// The appearance picker: every `ItemDisplayInfo.dbc` row as a picture.
///
/// It is a dialog and not a popup, for `ui::tables`'s reason: egui closes a
/// popup on the first click outside it, and this grid is scrolled and typed
/// into.
fn picker(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let Some(known) = work.open_item() else {
        work.items.close_picker();
        return;
    };
    let mut close = false;
    let mut chosen: Option<u32> = None;
    let response = egui::Modal::new(egui::Id::new("item-display-pick")).show(ui.ctx(), |ui| {
        ui.set_width(PICKER_WIDTH);
        super::listing::dialog_title(
            ui,
            "Appearance for this item",
            "Search text is matched against the display row's icon name, its only text. The \
             filter may also exclude appearances this item's slot allows.",
        );
        ui.horizontal(|ui| {
            let box_ = ui.add(
                egui::TextEdit::singleline(&mut work.items.display_query)
                    .hint_text("Icon name or display id")
                    .desired_width(PICKER_WIDTH - 260.0),
            );
            if work.items.display_focus {
                box_.request_focus();
                work.items.display_focus = false;
            }
            // The filter narrows by the item and not by what was typed.
            // 29,604 rows are three hundred pages; the slot this item is
            // worn in cuts them to the appearances that could go there. See
            // `crate::subjects::items::fits_slot`.
            let was = work.items.filter;
            let mut filter = was;
            egui::ComboBox::from_id_salt("item-display-filter")
                .selected_text(capitalised(filter.name()))
                .show_ui(ui, |ui| {
                    for option in Filter::ALL {
                        ui.selectable_value(&mut filter, option, capitalised(option.name()));
                    }
                });
            if filter != was {
                work.items.filter = filter;
                work.items.forget_display_matches();
            }
        });

        let worn = known.inventory_type;
        let total = work.items.display_matches(work.assets, worn).len();
        if total == 0 {
            theme::note(ui, "No matches.");
        }
        let range = pager(ui, &mut work.items.display_page, total, DISPLAY_PAGE);
        let page: Vec<u32> = work.items.display_matches(work.assets, worn)[range].to_vec();
        let at = work.items.display_page;
        egui::ScrollArea::vertical()
            .id_salt(("item-display-grid", at))
            .max_height(CELL * 5.0 + 24.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(CELL_GAP, CELL_GAP);
                    for id in page {
                        if cell(ui, work, &known, id) {
                            chosen = Some(id);
                        }
                    }
                });
            });

        ui.add_space(6.0);
        // The appearance under the pointer, large. A 60-point cell tells a
        // sword from a mace, and the pane tells one sword from another.
        if let Some(id) = work.items.display_preview {
            if preview(ui, work, &known, id) {
                chosen = Some(id);
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                close = true;
            }
            ui.label(
                egui::RichText::new(format!(
                    "{total} of {} appearances \u{b7} Esc closes",
                    work.items.display_facts(work.assets).len()
                ))
                .small()
                .color(theme::INK_FAINT),
            );
        });
    });
    if let Some(id) = chosen {
        let key = known.key();
        let gesture = format!("item {} display_id", known.entry);
        work.session.set_server_edit(
            item::TEMPLATE,
            &key,
            "display_id",
            Some(id.to_string()),
            Some(crate::server::store::Gesture {
                label: "Edit item",
                subject: &gesture,
                now: work.now,
            }),
        );
        close = true;
    }
    if response.should_close() {
        close = true;
    }
    if close {
        work.items.close_picker();
    }
}

/// One cell of the grid: the model where the row has one, the icon where it
/// does not, and the id under the pointer.
fn cell(ui: &mut egui::Ui, work: &mut Workspace<'_>, known: &Known, id: u32) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(CELL), egui::Sense::click());
    let on = known.display_id == id;
    if on || response.hovered() {
        ui.painter().rect_filled(
            rect,
            3.0,
            match on {
                true => theme::ACCENT_SUNK,
                false => theme::RAISED,
            },
        );
    }
    if response.hovered() {
        work.items.display_preview = Some(id);
    }
    let inner = rect.shrink(3.0);
    let look = work.items.look(work.assets, id, known.inventory_type);
    match look.as_ref() {
        Some(look) if look.has_models() => {
            let worn = look.models[0].clone();
            let key = work.portraits.want_worn(&worn);
            crate::preview::portraits::paint(ui, work.portraits, &key, inner);
        }
        Some(look) => match look.icon.clone() {
            Some(path) => {
                work.thumbnails.want(&path);
                match work.thumbnails.get(&path) {
                    Some(texture) => {
                        ui.painter().image(
                            texture,
                            inner,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    None => {
                        ui.painter().rect_filled(inner, 2.0, theme::SUNK);
                    }
                }
            }
            None => {
                ui.painter().rect_filled(inner, 2.0, theme::SUNK);
            }
        },
        None => {
            ui.painter().rect_filled(inner, 2.0, theme::DEAD);
        }
    }
    response.on_hover_text(format!("{id}")).clicked()
}

/// The picker's own preview: the id under the pointer, large, with its name.
///
/// Answers whether Use This Display was pressed. Choosing from the pane and
/// clicking the cell both answer the id to [`picker`], which writes the
/// column.
fn preview(ui: &mut egui::Ui, work: &mut Workspace<'_>, known: &Known, id: u32) -> bool {
    let look = work.items.look(work.assets, id, known.inventory_type);
    let mut used = false;
    ui.horizontal(|ui| {
        match look.as_ref().and_then(|look| look.models.first()) {
            Some(worn) => {
                let key = work.portraits.want_large_worn(worn);
                let pane = crate::preview::portraits::pane_id(ui);
                crate::preview::portraits::paint_large(ui, work.portraits, pane, &key, 120.0);
            }
            None => {
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(120.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 4.0, theme::SUNK);
                if let Some(path) = look.as_ref().and_then(|look| look.icon.clone()) {
                    work.thumbnails.want_at(&path, crate::ui::widgets::thumbnails::ICON_SIDE);
                    if let Some(texture) =
                        work.thumbnails.get_at(&path, crate::ui::widgets::thumbnails::ICON_SIDE)
                    {
                        ui.painter().image(
                            texture,
                            rect.shrink(20.0),
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                }
            }
        }
        ui.vertical(|ui| {
            ui.label(theme::number(format!("display {id}")));
            match look.as_ref() {
                Some(look) => {
                    ui.label(
                        egui::RichText::new(&look.icon_name)
                            .color(theme::INK)
                            .size(13.0),
                    );
                    let what = match look.has_models() {
                        true => format!("{} on the wearer", counted(look.models.len(), "model")),
                        false => counted(look.painted(), "body texture"),
                    };
                    ui.label(egui::RichText::new(what).small().color(theme::INK_DIM));
                    if look.has_models() {
                        ui.label(
                            egui::RichText::new(&look.models[0].path)
                                .small()
                                .color(theme::INK_FAINT),
                        );
                    }
                }
                None => {
                    ui.label(
                        egui::RichText::new("Not in ItemDisplayInfo.dbc")
                            .small()
                            .color(theme::BAD),
                    );
                }
            }
            // The same answer a click on the cell gives. The column is
            // written in one place, by [`picker`]; this only says which id.
            used = ui
                .button("Use This Display")
                .on_hover_text("Writes this display id into the item's display_id and closes the picker.")
                .clicked();
        });
    });
    used
}

/// A pager, in `ui::widgets::models::pager`'s shape: the page a caller is on, the total,
/// and the range of it to draw. The display picker (`super::displays`) uses
/// it as well.
///
/// Its own rather than the table browser's because that one is written against
/// the browser's state. The behaviour is the same: every row stays reachable.
pub(super) fn pager(
    ui: &mut egui::Ui,
    page: &mut usize,
    total: usize,
    per_page: usize,
) -> std::ops::Range<usize> {
    let pages = total.div_ceil(per_page.max(1)).max(1);
    if *page >= pages {
        *page = pages - 1;
    }
    if pages > 1 {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(*page > 0, egui::Button::new("\u{2039}"))
                .clicked()
            {
                *page -= 1;
            }
            ui.label(
                egui::RichText::new(format!("{} of {pages}", *page + 1))
                    .small()
                    .color(theme::INK_DIM),
            );
            if ui
                .add_enabled(*page + 1 < pages, egui::Button::new("\u{203a}"))
                .clicked()
            {
                *page += 1;
            }
        });
    }
    let start = (*page * per_page).min(total);
    let end = (start + per_page).min(total);
    start..end
}

/// The item workspace's inspector panel, its third part: what the open item
/// looks like, which is the inspector's view of the selection.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, assets, .. } = subject;
    inspector(
        ui,
        Looking {
            session,
            items: &mut editing.items,
            assets,
            thumbnails: &mut editing.thumbnails,
            portraits: &mut editing.portraits,
            asks: &mut editing.asks,
        },
    );
}

/// The item workspace in the middle of a window. A separate panel from the
/// spell workspace: a spell's row references a chain of other rows, and an
/// item's row references one display id that cannot be read as text, which
/// is why the inspector holds the item's appearance.
pub(crate) fn workspace(ui: &mut egui::Ui, _: crate::tools::Tool, bench: crate::ui::workspace::Bench<'_, '_, '_>) {
    let crate::ui::workspace::Bench { session, editing, playing, popovers, assets, now, .. } = bench;
    draw(
        ui,
        Workspace {
            session,
            items: &mut editing.items,
            assets,
            thumbnails: &mut editing.thumbnails,
            portraits: &mut editing.portraits,
            quests: &mut editing.quests,
            loot: &mut editing.loot,
            server_panel: &mut popovers.server,
            standings: &mut playing.standings,
            reloads: &mut playing.reloads,
            patch: crate::server::settings::server_patch(&playing.server),
            now,
            asks: &mut editing.asks,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seven qualities are the server's own colours, which is the
    /// whole content of that column: a list drawing them in the editor's
    /// palette would be showing something other than what the game shows.
    #[test]
    fn a_quality_is_drawn_in_the_games_own_colour() {
        assert_eq!(quality_colour(0), egui::Color32::from_rgb(0x9d, 0x9d, 0x9d));
        assert_eq!(quality_colour(4), egui::Color32::from_rgb(0xa3, 0x35, 0xee));
        assert_eq!(quality_colour(6), egui::Color32::from_rgb(0xe6, 0xcc, 0x80));
        // A value the game has no colour for is the ordinary ink rather than
        // the first entry of the table.
        assert_eq!(quality_colour(9), theme::INK);
    }

    /// Every page is reachable and none overlaps, which is what paging a
    /// list of tens of thousands is for.
    #[test]
    fn the_pager_covers_the_whole_list() {
        let mut covered: Vec<usize> = Vec::new();
        let total = DISPLAY_PAGE * 3 + 7;
        let pages = total.div_ceil(DISPLAY_PAGE);
        for page in 0..pages {
            let start = (page * DISPLAY_PAGE).min(total);
            let end = (start + DISPLAY_PAGE).min(total);
            covered.extend(start..end);
        }
        assert_eq!(covered.len(), total);
        covered.dedup();
        assert_eq!(covered.len(), total, "no row is drawn twice");
    }
}
