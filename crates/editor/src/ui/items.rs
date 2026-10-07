//! The workspace an item is edited in: the list, the form, and a picture of
//! what the row says it looks like.
//!
//! ## Three parts, because an item is three questions
//!
//! ```text
//! left       which item          24,000 rows, searched by name, entry or kind
//! middle     what it is          129 columns, in rowform's page layout
//! inspector  what it looks like  the display row, as a picture
//! ```
//!
//! The third part is drawn in the shell's inspector ([`inspector`]), which
//! is the spell workspace's arrangement and the shell's own rule: the inspector
//! is about the selection. It was a column of this workspace's own beside an
//! inspector that had nothing to say, and at a 1280-point window the two of
//! them and the list left the form 168 points — less than one page row. The
//! inspector is resizable to the same 560 points that column was, and the
//! picture takes whatever width it is given.
//!
//! [`crate::ui::data`]'s two parts plus one, and the third is the reason this
//! is not that panel. A spell's row points at a chain of other rows and the
//! useful thing to draw beside it is the chain; an item's row points at exactly
//! one thing you cannot read — `display_id`, whose only meaning is what it
//! looks like — and the useful thing to draw beside it is that.
//!
//! ## The appearance is chosen by looking at it
//!
//! `display_id` is a row of `ItemDisplayInfo.dbc` and there are tens of
//! thousands of them. `2589` is not a choice anybody can make by reading, which
//! is the tileset picker's problem and the model browser's problem and is
//! solved here the same way: a grid of pictures, paged, with a preview under it
//! and a search over the one piece of text a display row carries, which is its
//! icon name.
//!
//! A cell is a model where the row has one and an icon where it does not,
//! and that is not a fallback: it is what the two kinds of item are. A
//! sword, a helm and a pauldron hang geometry off the wearer and can be looked
//! at; a shirt, a pair of gloves and a robe paint textures into the wearer's
//! own skin and have no geometry at all, so the only picture of one that exists
//! outside a dressed character is the icon in the bag. Drawing an empty square
//! for the second kind would be drawing nothing for most of the table.
//!
//! ## What the form says that the number does not
//!
//! Every column whose number means something is drawn as the something: a
//! quality is a coloured word, a class and a subclass are two menus where the
//! second depends on the first, `buy_price` is gold and silver, `delay` is a
//! swing in seconds, a spell is its name out of `Spell.dbc`, a skill is its
//! name out of `SkillLine.dbc`, and the two mask columns are lists of ticks.
//! The number is still there, still editable, and still what is written.
//!
//! ## Nothing here reaches the database
//!
//! An edit goes into the project's own store through
//! [`EditSession::set_server_edit`], already escaped, and onto the undo stack.
//! A save writes it into `sql\items.sql`; Apply is what runs the
//! statements, and — unlike the creature half — it then asks a running playtest
//! to `.reload item_template`, which makes the change live for every copy of
//! that item already in the world. See [`crate::server::items`], where the
//! reason that works is.
//!
//! Apply is not on this panel. It is on the bar's Server…, with the
//! spells' and the creatures' and in the same words, because one vocabulary in
//! one place is worth more than a button where the edit was made. What this
//! panel keeps is the state — what the project changes and how much of it the
//! database holds — which is about the item on screen. See [`super::sync`].

use super::rowform::{
    choice_cell, draft_or, finished, flags_cell, meaning, number_cell, number_means, page_row,
    page_spacing, revert_button, section, text_cell, RowAct, FORM_ROW, FORM_VALUE,
};
use super::theme;
use super::thumbnails::Thumbnails;
use crate::session::EditSession;
use crate::tools::items::{Filter, Items, Known, Look, DISPLAY_PAGE};
use vale_client::assets::GameAssets;
use vale_mangos::item::{self, Column, Group, Kind};
use vale_mangos::row::Life;
use bevy_egui::egui;

/// How wide the item list is.
const LIST_WIDTH: f32 = 320.0;

/// …and the gap between the list and the form, which is `ui::data`'s own: a
/// panel's margin ends at its edge, and without this the section heads sit
/// against the list's border.
const FORM_GAP: f32 = 12.0;

/// …and the gap above everything, under the top bar.
///
/// The docked panels carry their own margin and the middle region carries none,
/// so the form's first line sat against the bar while the two columns beside it
/// did not. It is applied to all three rather than to the middle alone, because
/// what a person sees is one strip of chrome across the window and 8 points on
/// two thirds of it reads as a mistake.
const TOP_GAP: f32 = 10.0;

/// How tall one row of the list is, and how big the icon in it is drawn — the
/// editor's own, so that this list and the spell list are one list drawn
/// twice. See [`theme::list_row`].
const ROW_HEIGHT: f32 = theme::LIST_ROW;

const ICON: f32 = theme::LIST_PICTURE;

/// …and the one at the head of the form, which has room for more.
const HEAD_ICON: f32 = 36.0;

/// How big one cell of the appearance picker's grid is, and how many fit
/// across it.
///
/// 60 points is a little under twice a list row's icon: enough to tell a sword
/// from a mace, which is what a grid is for, and not enough to tell this sword
/// from the one beside it — which is what the preview pane under it is for.
const CELL: f32 = 60.0;
const GRID_COLUMNS: usize = 8;
const CELL_GAP: f32 = 4.0;

/// …so the dialog is exactly that many cells wide, rather than a number that
/// happens to fit today. A width the grid does not divide into leaves a ragged
/// column on the right of every page.
const PICKER_WIDTH: f32 = GRID_COLUMNS as f32 * (CELL + CELL_GAP) + 24.0;

/// What the seven qualities are drawn in, from `ItemQualityColors`
/// (`SharedDefines.h:205`).
///
/// The server's own table rather than a set of colours chosen here: a quality
/// is the one column of an item whose whole content is a colour, and a list
/// that used the editor's palette for it would be showing something other than
/// what the game shows.
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
pub fn quality_colour(quality: u32) -> egui::Color32 {
    QUALITY_COLOURS
        .get(quality as usize)
        .copied()
        .unwrap_or(theme::INK)
}

/// Everything the workspace needs, as one argument.
pub struct Workspace<'a> {
    pub session: &'a mut EditSession,
    pub items: &'a mut Items,
    pub assets: &'a GameAssets,
    /// The icons, which are BLP textures decoded like any other — see
    /// [`super::thumbnails`].
    pub thumbnails: &'a mut Thumbnails,
    /// …and the models, which are rendered — see [`crate::portraits`].
    pub portraits: &'a mut crate::portraits::Portraits,
    /// The quest workspace's state, for two things it already has: the
    /// reference picker, which is one dialog over every table a row names, and
    /// the quest list, which names `start_quest`. See [`reference_field`].
    pub quests: &'a mut crate::tools::quests::Quests,
    /// …and the loot window's, for the Loot toggle at the head of the
    /// form: what the item contains and what it disenchants into — see
    /// [`super::loot`].
    pub loot: &'a mut crate::tools::loot::Loot,
    /// Where this machine's server is, which the form's own sentences read.
    pub server: &'a crate::server::settings::ServerSettings,
    /// …and the queue an apply's `.reload item_template` goes on, whose status
    /// this panel shows: an item change is the one here that goes live without
    /// a restart, so whether the reload landed is worth saying beside the item.
    pub reloads: &'a mut crate::server::reload::Reloads,
    /// …and the bar's Server…, which is where every server operation is.
    /// See [`super::sync`].
    pub server_panel: &'a mut super::popover::Popover,
    /// Which content patch the server is configured for — half the key a new
    /// row is written under.
    pub patch: u32,
    /// The frame clock the undo stack folds a gesture by.
    pub now: f64,
}

impl Workspace<'_> {
    /// The open item, with this project's edits over it.
    fn open_item(&self) -> Option<Known> {
        self.items.open_item(&self.session.server_edits)
    }
}

/// What the appearance part reads, which is less than the workspace does:
/// the shell's inspector draws it, and has neither the reload queue nor the
/// server's patch to hand.
pub struct Looking<'a> {
    pub session: &'a EditSession,
    pub items: &'a mut Items,
    pub assets: &'a GameAssets,
    pub thumbnails: &'a mut Thumbnails,
    pub portraits: &'a mut crate::portraits::Portraits,
}

impl Looking<'_> {
    fn open_item(&self) -> Option<Known> {
        self.items.open_item(&self.session.server_edits)
    }
}

/// Draw the whole workspace into the region the panels left.
pub fn draw(ui: &mut egui::Ui, mut work: Workspace<'_>) {
    // It paints its own ground. The world goes on being drawn behind this
    // and a workspace that left a strip unpainted would be the world bleeding
    // through it — `ui::data`'s own note.
    let all = ui.available_rect_before_wrap();
    ui.painter().rect_filled(all, 0.0, theme::SHELL);

    egui::Panel::left("item-browser")
        .default_size(LIST_WIDTH)
        .min_size(240.0)
        .max_size(520.0)
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(8.0))
        .show(ui, |ui| {
            ui.add_space(TOP_GAP - 8.0);
            list(ui, &mut work);
        });

    let rect = ui.available_rect_before_wrap();
    let inset = egui::Rect::from_min_max(rect.min + egui::vec2(FORM_GAP, TOP_GAP), rect.max);
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(inset).layout(*ui.layout()));
    form(&mut inner, &mut work);

    if work.items.picking_display {
        picker(ui, &mut work);
    }
    // The reference picker, which is the quest form's dialog: its item
    // column purpose writes into this item. See [`reference_field`].
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

/// The left column: what there is, what matches, and the four things that are
/// about the table rather than about a row of it.
fn list(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    // The workspace's two parts, Items and Sets. The shell switches after
    // the frame is drawn.
    if let Some(part) = super::rail::parts(ui, crate::tools::Tool::Items) {
        work.items.switch_to = Some(part);
    }
    ui.horizontal(|ui| {
        // The rail's own word. The table's name belongs on the fields that
        // are written to it, and the heading of a list is where somebody looks
        // to know which list they are in.
        ui.label(
            egui::RichText::new("Items")
                .strong()
                .size(15.0)
                .color(theme::INK),
        );
        ui.label(theme::number(format!("{} rows", work.items.count())));
    })
    .response
    .on_hover_text("item_template as the server loads it: one row per item, at the highest content patch at or below the server's patch.");

    if let Some(trouble) = work.items.trouble.clone() {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
        theme::note(
            ui,
            "Items are rows in the vmangos world database, not files. Set the database \
             connection in Server\u{2026} on the top bar, or with VALE_MANGOSD.",
        );
        return;
    }
    if work.items.all.is_empty() {
        theme::waiting(ui, "reading item_template\u{2026}");
        return;
    }

    ui.add_space(4.0);
    let width = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(&mut work.items.query)
            .hint_text("name, entry, class, slot or level")
            .desired_width(width),
    );
    ui.add_space(4.0);
    row_actions(ui, work);
    ui.add_space(2.0);

    let revision = work.session.server_edit_revision;
    let matches: Vec<usize> = {
        let edits = &work.session.server_edits;
        work.items.matches(edits, revision).to_vec()
    };
    ui.label(
        egui::RichText::new(format!("{} shown", matches.len()))
            .small()
            .color(theme::INK_FAINT),
    );
    ui.add_space(4.0);

    // Bring the open item into view when it was opened from somewhere other
    // than this list, as the spell list does. See `theme::list_area`.
    let mut reveal = None;
    if work.items.open != work.items.revealed {
        match work.items.open {
            Some(entry) => {
                let found = matches
                    .iter()
                    .position(|&index| work.items.at(index).is_some_and(|known| known.entry == entry));
                if found.is_some() {
                    reveal = found;
                    work.items.revealed = Some(entry);
                }
            }
            None => work.items.revealed = None,
        }
    }
    let mut asked: Option<(Known, RowAct)> = None;
    theme::list_area(ui, reveal, ROW_HEIGHT)
        .show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
            for at in range {
                let index = matches[at];
                let Some(known) = work.items.shown(index, &work.session.server_edits) else {
                    continue;
                };
                let chosen = work.items.open == Some(known.entry);
                let response = row(ui, work, &known, chosen);
                if response.clicked() {
                    work.items.revealed = Some(known.entry);
                    asked = Some((known.clone(), RowAct::Open));
                }
                response.context_menu(|ui| {
                    let act = super::rowform::row_menu(ui, "item", known.entry, known.claim, chosen);
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
            work.session.status = format!("item entry {} is on the clipboard", known.entry);
        }
    }
}

/// New, Copy and Remove — the three things that make or unmake a row.
///
/// An item in the database is marked: it stays in the list in red until
/// Apply, and Keep takes the mark off. One this project created is in no
/// database, so removing it gives the claim up. A removal needs a restart of
/// the server, and the apply sends no reload while one is in the plan — see
/// `vale_mangos::item`'s module comment.
fn row_actions(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let open = work.open_item();
    ui.horizontal(|ui| {
        let third = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(40.0);
        let size = egui::vec2(third, 22.0);
        if ui
            .add(egui::Button::new("+ New").min_size(size))
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
        if ui
            .add_enabled(open.is_some(), egui::Button::new("Copy").min_size(size))
            .on_hover_text(
                "Creates a copy of this item under a new entry. Each column takes the \
                 value shown: this project's edit where there is one, otherwise the \
                 database value.",
            )
            .on_disabled_hover_text("Open an item first.")
            .clicked()
        {
            if let Some(known) = open.as_ref() {
                let ctx = ui.ctx().clone();
                act_on(&ctx, work, known, RowAct::Copy);
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
                let ctx = ui.ctx().clone();
                let act = match removed {
                    true => RowAct::Keep,
                    false => RowAct::Remove,
                };
                act_on(&ctx, work, known, act);
            }
        }
    });
}

/// One row of the list: the icon, the name in its quality's colour, what kind
/// of thing it is, and the entry on the right.
///
/// [`theme::list_row`] is the row, and it is the same row the spell list is
/// made of — see its own note, which is why this is a dozen lines rather than
/// fifty. Two things are this panel's: the picture, which is the item's bag
/// icon out of the thumbnail cache, and the colour of the name, which for
/// an item is a fact about the row rather than a choice of palette.
fn row(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    known: &Known,
    chosen: bool,
) -> egui::Response {
    // What the project says about the row, on the row. A new item and an
    // edited one are different things and both are invisible in a list that
    // only shows what was read.
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
    // Asked for on every frame the row is drawn, which is what keeps it
    // fresh: only the rows actually on screen pay for a decode.
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
            // The spell form's own spacing — see `rowform::page_spacing`.
            page_spacing(ui);
            edits_block(ui, work);
            let Some(known) = work.open_item() else {
                theme::heading(ui, "No item open");
                theme::note(
                    ui,
                    "Choose an item on the left, or create one with + New. Every column \
                     of the row is edited here; nothing is written to the database until \
                     Apply, in Server\u{2026} on the top bar.",
                );
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
                theme::waiting(ui, "reading the row\u{2026}");
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

/// What this project changes about the server's items, and the way to the
/// panel that applies it.
///
/// The creature panel's own block one subject along, with the same two parts:
/// the state here, where the thing being edited is, and the verbs on the bar's
/// Server… with the other two subjects'. See [`super::sync`], whose module
/// comment is the argument for one panel.
fn edits_block(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let standing = super::sync::standing(super::sync::Half::Items, work.session, work.assets);

    // A project that changes nothing says nothing. The block is about what
    // is waiting to be applied, and a standing sentence explaining that there
    // is none is a line of chrome above every item anybody ever opens.
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
    ui.horizontal(|ui| {
        if ui
            .button("Server\u{2026}")
            .on_hover_text(
                "Opens the Server panel, which applies, restores or discards these rows \
                 together with the spell and creature rows. An item applied during a \
                 playtest takes effect through `.reload item_template`, including copies \
                 already in the world; applied at any other time, it takes effect after a \
                 server restart.",
            )
            .clicked()
        {
            work.server_panel.show();
        }
        if let Some(status) = work.reloads.status() {
            ui.label(egui::RichText::new(status).small().color(theme::INK_DIM));
        }
    });
}

/// The head of the form: the icon, the name in its colour, and what the row is.
///
/// The key is on the second line and not beside the name. `Thunderfury,
/// Blessed Blade of the Windseeker` is forty-three characters at seventeen
/// points; put `entry 19019` after it on one row and egui lays the two out
/// overlapping, because a horizontal layout clips rather than wraps. The
/// second line has room for the key and for what the item is, which are the
/// same kind of fact.
fn head(ui: &mut egui::Ui, work: &mut Workspace<'_>, known: &Known) {
    ui.horizontal(|ui| {
        // The icon, at the head of the form. `ui::data`'s spell head has
        // one for the same reason: a name tells you which row is open and the
        // picture tells you what it is, and the second is the faster of the two
        // to read.
        let icon = work
            .items
            .look(work.assets, known.display_id, known.inventory_type)
            .and_then(|look| look.icon);
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(HEAD_ICON), egui::Sense::hover());
        ui.painter().rect_filled(rect, 3.0, theme::SUNK);
        if let Some(path) = icon {
            work.thumbnails.want_at(&path, super::thumbnails::ICON_SIDE);
            if let Some(texture) = work.thumbnails.get_at(&path, super::thumbnails::ICON_SIDE) {
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
                    .small_button("copy")
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
                // Loot is a window over the workspace, following the open
                // item as it follows a selected creature — see `super::loot`.
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
        Life::Delete => {
            ui.label(
                egui::RichText::new(
                    "Marked for removal. Apply deletes it with its loot and vendor rows; \
                     restart the server afterwards. Keep, under the search box, unmarks it.",
                )
                .small()
                .color(theme::BAD),
            );
        }
        Life::Update => {}
    }
}

/// Every group of the table's columns: the first three open, the rest folded.
///
/// 129 columns is more than a person reads at once and the ones worth reading
/// first are not the ones that come first in the table.
///
/// One field per row, in `rowform`'s page layout: the name centred in a
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
        let columns: Vec<&Column> = item::TEMPLATE_COLUMNS
            .iter()
            .filter(|column| column.group == group)
            .collect();
        let edited = columns.iter().any(|column| {
            work.session
                .server_edits
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

/// One column: its name, its value, what the value means, and whether this
/// project has changed it.
fn field(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    known: &Known,
    row: &item::Row,
    column: &Column,
) {
    let key = known.key();
    let edited = work
        .session
        .server_edits
        .get(item::TEMPLATE, &key, column.name)
        .map(str::to_string);

    // What the field shows: this project's value where it has one, and the
    // database's otherwise.
    let in_database = column.literal(row);
    let showing = edited.clone().or_else(|| in_database.clone());

    // One id per row and column. Two items have the same column names, and
    // egui keys a widget's own state — a text box's draft, a menu being open —
    // on its id: sharing one would carry the half-typed name of one item into
    // the next one clicked.
    let id = ui.make_persistent_id(("item", known.entry, column.name));
    page_row(ui, column.name, column.about, edited.is_some(), |ui| {
        let Some(showing) = showing else {
            ui.label(egui::RichText::new("NULL").color(theme::INK_FAINT));
            return;
        };
        let written = match column.kind {
            // The key is shown and not typed into — except `entry`.
            // Writing a key column moves the row the edit is about, which for
            // `patch` would silently retarget the statement at a version of the
            // item the server is not loading. Renumbering an item is a thing
            // people want, so it has a field of its own: see [`entry_field`],
            // which is not a column edit at all for half the rows it draws.
            Kind::Key if column.name == "entry" => {
                entry_field(ui, work, known, id);
                None
            }
            Kind::Key => {
                ui.label(theme::number(&showing));
                None
            }
            Kind::Text => text_cell(ui, id.with("text"), &showing),
            Kind::Flags(bits) => flags_cell(ui, &showing, bits, id),
            Kind::Choice(values) => choice_cell(ui, &showing, values, id),
            // The subclass is the one column whose options depend on another
            // column of the same row: 0 is Axe on a weapon, Cloth on a
            // piece of armour and Bandage on a consumable, so the list is
            // the class's — see `vale_mangos::item::subclasses`.
            Kind::Subclass => choice_cell(ui, &showing, item::subclasses(known.class), id),
            // A display id is picked by looking at it. The number stays
            // editable beside the button, because an id read off a wiki is a
            // thing people have in hand.
            Kind::Ref("ItemDisplayInfo") => display_field(ui, work, known, &showing),
            Kind::Ref(table) => reference_field(
                ui,
                work,
                known,
                column,
                table,
                &showing,
                in_database.clone(),
            ),
            _ => {
                let written = number_cell(ui, column.kind, &showing);
                // What the number means, beside it: 1478900 is 147g 89s and
                // 1800000 is half an hour, and a form that shows only the
                // number makes those the same kind of thing.
                if let Some(means) = number_means(column.kind, &showing) {
                    meaning(ui, means);
                }
                written
            }
        };
        if let Some(written) = written {
            // Cleared rather than written when it matches what the database
            // holds: a project should not claim to change a column back to the
            // value it already has, and a statement for one does nothing.
            let value = match Some(&written) == in_database.as_ref() {
                true => None,
                false => Some(written),
            };
            let gesture = format!("item {} {}", known.entry, column.name);
            work.session.set_server_edit(
                item::TEMPLATE,
                &key,
                column.name,
                value,
                Some(crate::session::Gesture {
                    label: "Edit item",
                    subject: &gesture,
                    now: work.now,
                }),
            );
        }
        // No revert arrow on a row this project creates. Every column of
        // one is the project's own and there is nothing in the database to put
        // it back to — clearing it would leave the `INSERT` naming one column
        // fewer than the table has, which is a value nobody chose.
        let creating = known.claim == Life::Insert;
        if edited.is_some() && !creating && revert_button(ui, in_database.as_deref()) {
            let gesture = format!("item {} {} revert", known.entry, column.name);
            work.session.set_server_edit(
                item::TEMPLATE,
                &key,
                column.name,
                None,
                Some(crate::session::Gesture {
                    label: "Revert column",
                    subject: &gesture,
                    now: work.now,
                }),
            );
        }
    });
}

/// The `entry` column, which is the row's own key and is editable anyway.
///
/// Committed on Enter or on leaving the box rather than per keystroke, for
/// `rowform::text_field`'s reason one step further along: a key that moved on
/// every digit typed would re-key the row four times on the way to `20000`, and
/// three of those are entries somebody else may own.
///
/// What it does is one thing whatever the row is: the project's claim on the row
/// is re-keyed — see [`crate::tools::items::Items::rekey`]. For a row that is in
/// the database the apply then moves the row, every other content-patch version
/// of it, and every column of the world database that names an item by entry;
/// the sentence under the box says so, and says what that cannot reach.
fn entry_field(ui: &mut egui::Ui, work: &mut Workspace<'_>, known: &Known, id: egui::Id) {
    let id = id.with("entry");
    let mut text = draft_or(ui, id, known.entry.to_string());
    // The value cell's own size, so the key lines up with the numbers under it.
    let response = ui.add_sized(
        egui::vec2(FORM_VALUE, FORM_ROW),
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .horizontal_align(egui::Align::Center),
    );
    // What the trouble was, kept until the box is touched again. A refusal
    // raised on the frame Enter was pressed and forgotten on the next one is a
    // refusal nobody reads.
    let trouble_id = id.with("trouble");
    if let Some(text) = finished(ui, id, &response, text) {
        let trouble = match text.trim().parse::<u32>() {
            Err(_) => Some(format!("{text:?} is not a valid entry number")),
            Ok(wanted) => {
                let now = work.now;
                match work.items.rekey(work.session, known, wanted, now) {
                    Ok(()) => None,
                    Err(why) => Some(why),
                }
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
    // …and what pressing Enter would do.
    let (word, colour) = match (known.claim, known.read_entry != known.entry) {
        (Life::Insert, _) => ("changes this new item's entry".to_string(), theme::INK_FAINT),
        (_, true) => (
            format!(
                "the database row is entry {} until applied",
                known.read_entry
            ),
            theme::WARN,
        ),
        _ => (
            "changes the row's entry and every world database reference to it".to_string(),
            theme::INK_FAINT,
        ),
    };
    ui.label(egui::RichText::new(word).small().color(colour))
        .on_hover_text(format!(
            "vmangos has no foreign keys, so Apply updates the references itself: every \
             content-patch version of the row, and the {} world database columns that \
             refer to an item by entry: the loot tables, npc_vendor, \
             creature_equip_template, quest_template's item columns, playercreateinfo_item, \
             spell_template's reagents. Restore reverts all of it.\n\n\
             Not updated: the characters database, so a copy a character already carries \
             has no item_template row after the change; and Spell.dbc's reagent and \
             created-item fields, which are in a client file.",
            vale_mangos::item::REFERENCES.len()
        ));
}

/// The `display_id` column: the number, the icon it resolves to, and the
/// button that opens the grid.
fn display_field(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    known: &Known,
    showing: &str,
) -> Option<String> {
    let written = number_cell(ui, Kind::Unsigned, showing);
    let id: u32 = showing.trim().parse().unwrap_or(0);
    let look = work.items.look(work.assets, id, known.inventory_type);
    if let Some(path) = look.as_ref().and_then(|look| look.icon.clone()) {
        work.thumbnails.want(&path);
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(20.0), egui::Sense::hover());
        if let Some(texture) = work.thumbnails.get(&path) {
            ui.painter().image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }
    if ui
        .small_button("choose\u{2026}")
        .on_hover_text(
            "Opens a picker of every ItemDisplayInfo.dbc row, shown as thumbnails. A row \
             with a model is shown as its model; a row that only textures the wearer's \
             body is shown as its icon.",
        )
        .clicked()
    {
        work.items.pick_display(id);
    }
    match look {
        Some(look) if look.has_models() => {
            ui.label(
                egui::RichText::new(format!("{} model(s)", look.models.len()))
                    .small()
                    .color(theme::INK_DIM),
            );
        }
        Some(look) => {
            ui.label(
                egui::RichText::new(format!("{} texture(s)", look.painted()))
                    .small()
                    .color(theme::INK_DIM),
            );
        }
        None if id != 0 => {
            ui.label(
                egui::RichText::new("not in ItemDisplayInfo.dbc")
                    .small()
                    .color(theme::BAD),
            );
        }
        None => {}
    }
    written
}

/// A reference to another table: the number, the `…` that chooses one by
/// name, and what it resolves to — as a link to where that row is edited.
///
/// The quest form's arrangement, and its picker: [`super::quests::target_of`]
/// says which tables a name can be searched in, and the dialog writes the
/// chosen id into this item through its item column purpose. The number stays
/// editable beside the button, because an id read off a wiki is a thing people
/// have in hand.
///
/// What a click on the name opens is where that row is edited: a spell, a
/// skill, a faction, an area or a map opens the table browser on the row, an
/// item opens it here, and a quest opens the quest workspace. The DBCs are
/// opened on demand through the session, exactly as the spell workspace opens
/// them. A table this editor cannot open, or a row it does not have, draws the
/// number alone — which is what the column is.
fn reference_field(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    known: &Known,
    column: &Column,
    table: &'static str,
    showing: &str,
    in_database: Option<String>,
) -> Option<String> {
    let written = number_cell(ui, Kind::Unsigned, showing);
    if let Some(target) = super::quests::target_of(table) {
        if ui
            .add(
                egui::Button::new(egui::RichText::new("\u{2026}").size(13.0))
                    .min_size(egui::vec2(24.0, FORM_ROW)),
            )
            .on_hover_text(format!("choose {} by name", column.name))
            .clicked()
        {
            work.quests.picker = Some(crate::tools::quests::Picker::new(
                target,
                crate::tools::quests::PickFor::ServerColumn(crate::tools::quests::ColumnTarget {
                    table: item::TEMPLATE,
                    key: known.key(),
                    column: column.name,
                    in_database,
                    label: "Edit item",
                    subject: format!("item {} {}", known.entry, column.name),
                }),
            ));
        }
    }
    let id: u32 = showing.trim().parse().unwrap_or(0);
    if id == 0 {
        ui.label(egui::RichText::new("none").small().color(theme::INK_FAINT));
        if table == "ItemSet" {
            set_membership(ui, work, known, 0);
        }
        return written;
    }
    // The item's own table is answered from the list that is already in
    // memory, and a link to another item opens it in this workspace.
    if table == item::TEMPLATE {
        match work.items.by_entry(id).cloned() {
            Some(found) => {
                let found = found
                    .with_edits(&work.session.server_edits)
                    .unwrap_or(found);
                let colour = quality_colour(found.quality);
                if link(ui, &found.name, colour, &format!("Open item {id}.")) {
                    work.items.open = Some(id);
                }
            }
            None => {
                ui.label(
                    egui::RichText::new("no such item")
                        .small()
                        .color(theme::BAD),
                );
            }
        }
        return written;
    }
    // …and a quest's title from the quest list, which the quest tool reads
    // while this workspace is up.
    if table == vale_mangos::quest::TEMPLATE {
        match work.quests.title_of(id, &work.session.server_edits) {
            Some(title) => {
                let hover = format!("Open quest {id} in the quest workspace.");
                if link(ui, &title, theme::ACCENT, &hover) {
                    work.items.show_quest = Some(id);
                }
            }
            None if work.quests.all.is_empty() => meaning(ui, "\u{2026}"),
            None => {
                ui.label(
                    egui::RichText::new("no such quest")
                        .small()
                        .color(theme::BAD),
                );
            }
        }
        return written;
    }
    if !is_a_dbc(table) {
        ui.label(
            egui::RichText::new(format!("{table} {id}"))
                .small()
                .color(theme::INK_FAINT),
        );
        return written;
    }
    if !work.session.open_table(work.assets, table) {
        ui.label(
            egui::RichText::new(format!("{table}\u{2026}"))
                .small()
                .color(theme::INK_FAINT),
        );
        return written;
    }
    let record = work.session.table(table).and_then(|open| open.row_of(id));
    let Some(record) = record else {
        ui.label(egui::RichText::new("no such row").small().color(theme::BAD));
        return written;
    };
    let name = name_of(work, table, record).unwrap_or_else(|| id.to_string());
    let hover = format!("Open {table} {id} in the table browser.");
    if link(ui, &name, theme::ACCENT, &hover) {
        work.items.show_row = Some((table, id));
    }
    if table == "ItemSet" {
        set_membership(ui, work, known, id);
    }
    // A spell's rank, which is the only thing that tells nine Fireballs
    // apart — field 129, the one the quest picker lists under the name.
    if table == "Spell" {
        let rank = work
            .session
            .table(table)
            .and_then(|open| open.string_at(record, 129))
            .filter(|rank| !rank.is_empty());
        if let Some(rank) = rank {
            ui.label(egui::RichText::new(rank).small().color(theme::INK_FAINT));
        }
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
fn set_membership(ui: &mut egui::Ui, work: &mut Workspace<'_>, known: &Known, set: u32) {
    use crate::tools::tables;
    if !work.session.open_table(work.assets, "ItemSet") {
        return;
    }
    let Some(sets) = work.session.table("ItemSet") else {
        return;
    };
    let item = known.entry;
    let unlisted = set != 0
        && sets
            .row_of(set)
            .is_some_and(|record| tables::set_lists(sets, record, item).is_none());
    let elsewhere: Vec<u32> = (0..sets.record_count())
        .filter(|&record| tables::set_lists(sets, record, item).is_some())
        .filter_map(|record| sets.u32_at(record, 0))
        .filter(|&other| other != set)
        .collect();
    let subject = format!("item {item} set_id");
    let mut moves: Vec<(u32, u32)> = Vec::new();
    if unlisted {
        ui.label(
            egui::RichText::new("not in the set's item list")
                .small()
                .color(theme::WARN),
        )
        .on_hover_text(
            "This item's set_id names the set, but the set's item columns in ItemSet.dbc \
             do not list this item. The client lists a set's pieces from those columns.",
        );
        if ui
            .small_button("add")
            .on_hover_text("Writes the item into the set's first empty item column. One undo entry.")
            .clicked()
        {
            moves.push((0, set));
        }
    }
    for other in elsewhere {
        ui.label(
            egui::RichText::new(format!("still listed in set {other}"))
                .small()
                .color(theme::WARN),
        )
        .on_hover_text(
            "Another set's item columns in ItemSet.dbc list this item, but this item's \
             set_id does not name that set.",
        );
        if ui
            .small_button("remove")
            .on_hover_text(format!("Removes the item from set {other}'s item columns. One undo entry."))
            .clicked()
        {
            moves.push((other, 0));
        }
    }
    for (from, to) in moves {
        let said = tables::move_between_sets(work.session, item, from, to, &subject, work.now);
        if let Some(last) = said.last() {
            work.session.status = last.clone();
        }
    }
}

/// A name that opens something, in the colour it is drawn in and
/// underlined while the pointer is over it — the quest form's item link.
/// Answers whether it was clicked.
fn link(ui: &mut egui::Ui, text: &str, colour: egui::Color32, hover: &str) -> bool {
    let name = ui.add(
        egui::Label::new(egui::RichText::new(text).color(colour)).sense(egui::Sense::click()),
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

/// Whether a reference names a client table this editor can open.
fn is_a_dbc(table: &str) -> bool {
    matches!(
        table,
        "Spell"
            | "SkillLine"
            | "Faction"
            | "Lock"
            | "AreaTable"
            | "Map"
            | "ItemDisplayInfo"
            | "ItemSet"
    )
}

/// What a row of one of those tables is called.
///
/// Which field holds the name is per table and is `super::quests::name_field`'s
/// measured list: 3 for `SkillLine`, 19 for `Faction`, 11 for `AreaTable`, 4 for
/// `Map`, 120 for `Spell`, 1 for `ItemSet`. This read field 1 for any table with no schema, which
/// is an integer in all four of those and drew nothing or a fragment of another
/// row's string. A table that list does not know falls back to the schema's
/// column called `Name` — its field, not its position in the column list,
/// which for `Spell` are different numbers. `Lock` has no name at all and draws
/// its id.
fn name_of(work: &mut Workspace<'_>, table: &str, record: usize) -> Option<String> {
    let field = super::quests::name_field(table).or_else(|| {
        vale_assets::tables::schema::for_table(table)?
            .columns
            .iter()
            .find(|column| column.name.eq_ignore_ascii_case("Name"))
            .map(|column| column.field)
    });
    let open = work.session.table(table)?;
    let name = field
        .and_then(|field| open.string_at(record, field))
        .filter(|text| !text.is_empty());
    name.or_else(|| open.u32_at(record, 0).map(|id| id.to_string()))
}

// ---------------------------------------------------------------------------
// The inspector's part: the appearance
// ---------------------------------------------------------------------------

/// The shell's inspector, while the item workspace is up: a picture of what
/// the open item looks like, and what the display row it names is made of.
///
/// The picture is as wide as the inspector. The panel is resizable and the
/// pane takes what it is given, so how much of the window is spent on looking
/// at the thing is the person's to say — see
/// [`super::inspector::preview_pane_at`], which takes the width as its cap
/// rather than the 170 points the placement pickers use.
pub fn inspector(ui: &mut egui::Ui, mut work: Looking<'_>) {
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
        ui.add_space(10.0);
        appearance(ui, work, &known);
    }
}

/// How wide the inspector has to be before the tooltip and the picture are
/// drawn beside each other: two columns of a plate's width.
const SIDE_BY_SIDE: f32 = 2.0 * PLATE_WIDTH;

/// The widest a tooltip is drawn. The game sizes a plate to its longest
/// line and wraps a spell sentence; this is about the width a wrapped
/// `Equip:` line takes on the reference at its default scale.
const PLATE_WIDTH: f32 = 270.0;

/// The open item's tooltip, as the game would draw it, composed from the
/// form's values on every frame, so an edit shows on the frame it is made.
///
/// The lines are `vale_client::interface::plate::item_plate`'s, which is the
/// function the client's own `GameTooltip` draws an item through: the same
/// order, the same `GlobalStrings.lua` keys and the same colours. What is the
/// editor's is the frame and the font. The game draws the plate in
/// `Fonts\FRIZQT__.TTF` over `Interface\Tooltips\UI-Tooltip-Background`; this
/// draws it in the editor's own font over the colour
/// `TOOLTIP_DEFAULT_BACKGROUND_COLOR` names in `GameTooltip.lua`.
///
/// Composed for a character who meets every requirement, so the race,
/// class and level lines are white. The client colours each by the player
/// looking at it, and there is no player here; red on every restricted item
/// would say something about nobody.
fn plate(ui: &mut egui::Ui, work: &mut Looking<'_>, known: &Known) {
    theme::heading(ui, "Tooltip");
    let Some(info) = work.items.open_info(&work.session.server_edits, known) else {
        theme::waiting(ui, "reading the row\u{2026}");
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
    let wearer = vale_client::interface::api::Wearer::none();
    let mut tip =
        vale_client::interface::api::item_tip(&info, None, tables.as_deref(), &context, &wearer);
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

/// What the open item looks like: the picture, the body it is drawn on,
/// and what the display row is made of.
fn appearance(ui: &mut egui::Ui, work: &mut Looking<'_>, known: &Known) {
    let known = known.clone();
    theme::heading(ui, "Appearance");
    let display_id = known.display_id;
    let look = work
        .items
        .look(work.assets, display_id, known.inventory_type);
    ui.horizontal(|ui| {
        ui.label(theme::number(format!("display {display_id}")));
        if ui.small_button("choose\u{2026}").clicked() {
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
    // The icon beside its name, small. It is still a fact about the row and
    // it is still what a bag shows; what it is no longer is the picture, for
    // the kind of item that has a better one.
    ui.horizontal(|ui| {
        icon_square(ui, work, look.icon.as_deref(), ICON);
        ui.label(
            egui::RichText::new(&look.icon_name)
                .small()
                .color(theme::INK_DIM),
        );
    });

    ui.add_space(6.0);
    let side = ui.available_width();
    // A model where the row has one, and the body it paints where it does
    // not. See the module comment: these are the two kinds of item, and both
    // of them have a picture — what changes is whether the thing being looked
    // at is the item or the wearer.
    let on_a_body = match look.models.first() {
        Some(worn) => {
            let key = work.portraits.want_large_worn(worn);
            super::inspector::preview_pane_at(ui, work.portraits, &key, side);
            if look.models.len() > 1 {
                ui.label(
                    egui::RichText::new(format!(
                        "{} more model(s) \u{2014} the preview shows the left shoulder",
                        look.models.len() - 1
                    ))
                    .small()
                    .color(theme::INK_FAINT),
                );
            }
            false
        }
        // No model: a body only where the row would actually change one.
        //
        // Three kinds of row land here and only two of them are worn. A shirt,
        // a glove and a robe paint components of the wearer's composite; a
        // cloak paints none of them and is the wearer's own group-15 geoset
        // with a texture, which is why the dressing is asked rather than the
        // row; and some rows switch a geoset group without painting. A trade
        // good, a bag, a reagent and a quest item are none of the three:
        // they are an icon and nothing else, and a body drawn for one would be
        // a naked human standing in for a stack of copper ore.
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
                    super::inspector::preview_pane_at(ui, work.portraits, &key, side);
                    theme::note(
                        ui,
                        "This appearance has no model: it textures the wearer's body, so \
                         the preview draws it on a body in its bind pose.",
                    );
                    true
                }
                // Nothing is worn, or no body resolved for this race. The icon
                // is the only picture of it there is, which for most of these
                // rows is also all the game ever shows.
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

    // Which body, for both kinds that have one. A helm is cut per race and
    // gender; a garment is painted onto whichever body is standing there.
    body_picker(ui, work, &known, on_a_body);

    ui.add_space(6.0);
    look_detail(ui, &look);
}

/// One decoded icon in a square of its own, or the empty well where there is
/// none.
///
/// The well is drawn either way, so a column does not jump as the picture
/// arrives — the list rows' own rule one panel along.
fn icon_square(ui: &mut egui::Ui, work: &mut Looking<'_>, path: Option<&str>, side: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::hover());
    ui.painter().rect_filled(rect, 3.0, theme::SUNK);
    let Some(path) = path else { return };
    // Decoded at the size it is drawn: the list's level for a small square, the
    // file's own top level for anything larger.
    let level = match side > super::thumbnails::SIDE as f32 {
        true => super::thumbnails::ICON_SIDE,
        false => super::thumbnails::SIDE,
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

/// Which body the preview uses — the two cases where it changes the
/// picture.
///
/// A helm's row names `Helm_Plate_D_04.mdx` and the archive holds sixteen
/// files, eight race codes times two genders; and an appearance with no model
/// of its own is drawn on a body, which is a different body per race. Every
/// other item is one file whatever is wearing it, so the picker is not drawn.
fn body_picker(ui: &mut egui::Ui, work: &mut Looking<'_>, known: &Known, on_a_body: bool) {
    // A helm, or anything drawn on a body. Everything else is one model with
    // one file, and a picker that changed nothing would be a control that lies.
    if !on_a_body && known.inventory_type != 1 {
        return;
    }
    const RACES: [(u8, &str); 8] = [
        (1, "Human"),
        (2, "Orc"),
        (3, "Dwarf"),
        (4, "Night Elf"),
        (5, "Undead"),
        (6, "Tauren"),
        (7, "Gnome"),
        (8, "Troll"),
    ];
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("on").small().color(theme::INK_DIM));
        let was = work.items.race;
        let mut race = was;
        egui::ComboBox::from_id_salt("item-race")
            .selected_text(
                RACES
                    .iter()
                    .find(|(id, _)| *id == race)
                    .map(|(_, name)| *name)
                    .unwrap_or("Human"),
            )
            .show_ui(ui, |ui| {
                for (id, name) in RACES {
                    ui.selectable_value(&mut race, id, name);
                }
            });
        let mut gender = work.items.gender;
        egui::ComboBox::from_id_salt("item-gender")
            .selected_text(match gender {
                0 => "male",
                _ => "female",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut gender, 0, "male");
                ui.selectable_value(&mut gender, 1, "female");
            });
        if race != was || gender != work.items.gender {
            work.items.race = race;
            work.items.gender = gender;
        }
    })
    .response
    .on_hover_text(
        "Race and gender of the preview body. Does not change the row. A helmet has one \
         model per race and gender \u{2014} the display row names one file and the \
         archive holds sixteen \u{2014} and a garment is painted onto the body that \
         wears it.",
    );
}

/// What the display row is made of, under the picture.
fn look_detail(ui: &mut egui::Ui, look: &Look) {
    if look.has_models() {
        egui::CollapsingHeader::new("Models")
            .id_salt("item-models")
            .default_open(false)
            .show(ui, |ui| {
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
        egui::CollapsingHeader::new(format!("Textures ({})", look.painted()))
            .id_salt("item-textures")
            .default_open(false)
            .show(ui, |ui| {
                theme::note(
                    ui,
                    "Painted into the wearer's own 256x256 body composite, one per \
                     component.",
                );
                use vale_assets::tables::item::Component;
                for (component, path) in Component::ALL.iter().zip(look.textures.iter()) {
                    if path.is_empty() {
                        continue;
                    }
                    ui.label(
                        egui::RichText::new(format!("{:?}: {path}", component))
                            .small()
                            .color(theme::INK_DIM),
                    );
                }
            });
    }
    if look.geoset_groups.iter().any(|group| *group != 0) {
        ui.label(
            egui::RichText::new(format!(
                "geosets {} / {} / {}",
                look.geoset_groups[0], look.geoset_groups[1], look.geoset_groups[2]
            ))
            .small()
            .color(theme::INK_DIM),
        )
        .on_hover_text(
            "geosetGroup[3]: the geoset variant the wearer switches to, such as a cuff, \
             a boot leg or a robe skirt. 0 is the default, the bare body.",
        );
    }
    if look.helmet_hides.iter().any(|id| *id != 0) {
        ui.label(
            egui::RichText::new(format!(
                "hides {} / {}",
                look.helmet_hides[0], look.helmet_hides[1]
            ))
            .small()
            .color(theme::INK_DIM),
        )
        .on_hover_text(
            "helmetGeosetVis, per gender: a HelmetGeosetVisData.dbc row that lists, per \
             race, which of the wearer's geosets (hair, facial hair, ears) this helmet \
             hides.",
        );
    }
}

// ---------------------------------------------------------------------------
// The appearance picker
// ---------------------------------------------------------------------------

/// Every appearance, as pictures.
///
/// A dialog rather than a popup, for `ui::data`'s reason: egui dismisses a
/// popup on the first click it does not own, and a grid that has to be scrolled
/// and typed into is not a thing a popup holds well.
fn picker(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let Some(known) = work.open_item() else {
        work.items.close_picker();
        return;
    };
    let mut close = false;
    let mut chosen: Option<u32> = None;
    let response = egui::Modal::new(egui::Id::new("item-display-pick")).show(ui.ctx(), |ui| {
        ui.set_width(PICKER_WIDTH);
        ui.label(
            egui::RichText::new("Appearance for this item")
                .strong()
                .size(14.0),
        );
        ui.horizontal(|ui| {
            let box_ = ui.add(
                egui::TextEdit::singleline(&mut work.items.display_query)
                    .hint_text("icon name or display id")
                    .desired_width(PICKER_WIDTH - 260.0),
            );
            if work.items.display_focus {
                box_.request_focus();
                work.items.display_focus = false;
            }
            // This narrowing is by the item rather than by what was typed.
            // 29,604 rows is three hundred pages; the slot this
            // item is worn in cuts it to the appearances that could go
            // there, which is what somebody choosing one is after. See
            // `crate::tools::items::fits_slot`.
            let was = work.items.filter;
            let mut filter = was;
            egui::ComboBox::from_id_salt("item-display-filter")
                .selected_text(filter.name())
                .show_ui(ui, |ui| {
                    for option in Filter::ALL {
                        ui.selectable_value(&mut filter, option, option.name());
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
            theme::note(
                ui,
                "No matches. Search text is matched against the display row's icon \
                 name, its only text. The filter above may also exclude appearances \
                 this item's slot allows.",
            );
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
        // The one under the pointer, large. A cell is 60 points and is
        // enough to tell a sword from a mace; it is not enough to tell this
        // sword from the one beside it, which is what the pane is for.
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
                    "{total} of {} appearance(s) \u{b7} Esc closes",
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
            Some(crate::session::Gesture {
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
            crate::portraits::paint(ui, work.portraits, &key, inner);
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
/// Answers whether Use this one was pressed, so that choosing from the pane
/// and clicking the cell itself are the same gesture reaching the same place —
/// see [`picker`], which is where the column is written.
fn preview(ui: &mut egui::Ui, work: &mut Workspace<'_>, known: &Known, id: u32) -> bool {
    let look = work.items.look(work.assets, id, known.inventory_type);
    let mut used = false;
    ui.horizontal(|ui| {
        match look.as_ref().and_then(|look| look.models.first()) {
            Some(worn) => {
                let key = work.portraits.want_large_worn(worn);
                crate::portraits::paint_large(ui, work.portraits, &key, 120.0);
            }
            None => {
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(120.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 4.0, theme::SUNK);
                if let Some(path) = look.as_ref().and_then(|look| look.icon.clone()) {
                    work.thumbnails.want_at(&path, super::thumbnails::ICON_SIDE);
                    if let Some(texture) =
                        work.thumbnails.get_at(&path, super::thumbnails::ICON_SIDE)
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
                        true => format!("{} model(s) on the wearer", look.models.len()),
                        false => format!("{} body texture(s)", look.painted()),
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
                        egui::RichText::new("not in ItemDisplayInfo.dbc")
                            .small()
                            .color(theme::BAD),
                    );
                }
            }
            // The same answer a click on the cell gives. The column is
            // written in one place, by [`picker`]; this only says which id.
            used = ui
                .button("Use this display")
                .on_hover_text("Writes this display id into the item's display_id and closes the picker.")
                .clicked();
        });
    });
    used
}

/// A pager, in `ui::data::pager`'s shape: the page a caller is on, the total,
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

    /// The references this form resolves are the ones the editor can open, and
    /// the two server tables are not among them: there is no file to read a
    /// `quest_template` row out of.
    #[test]
    fn only_client_tables_are_resolved_by_name() {
        assert!(is_a_dbc("Spell"));
        assert!(is_a_dbc("SkillLine"));
        assert!(!is_a_dbc("quest_template"));
        assert!(!is_a_dbc(item::TEMPLATE));
    }
}
