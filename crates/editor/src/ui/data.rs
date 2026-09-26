//! The workspace a table is edited in: the browser, and the form beside it.
//!
//! ## The viewport is not the document here
//!
//! For every tool on the rail's world half — *Terrain* and *World* — the world
//! **is** the document: the
//! panels are around it and the thing being edited is in the middle. A spell is
//! not in the world, and a form squeezed into a 300-point inspector beside a
//! picture of some ground it has nothing to do with would be the worst of both.
//!
//! So a *Data* subject replaces the middle region: a searchable list on the
//! left and the row's fields in the rest. Everything else in the shell stays
//! exactly where it was — the same top bar, the same rail, the same view bar,
//! the same status line, and **the same undo stack**, so `Ctrl+Z` is the last
//! thing you did whether that was a wall or a spell's name.
//!
//! The shape is a list, a form, and a drill-down into the detail, inside the
//! shell rather than in windows of their own. The shell has no floating panels
//! for data, because a floating panel of loose controls reads as a debug
//! window.
//!
//! ## What a field is drawn as comes from the schema
//!
//! `vale_assets::tables::schema` says what each column *is* — a number, a
//! mask, a gate, one of a named set, a row id in another table, a string — and
//! this draws the widget that fits. Nothing here knows what a spell is: point
//! the browser at another table with a schema and it draws that instead.
//!
//! A number with no name is the failure mode that matters. 173 anonymous
//! columns is a table nobody can edit safely, which is why the schema came
//! before the panel.
//!
//! ## A reference is a name, a picture, and a way through
//!
//! The first draft drew a reference column as its number and the word
//! `SpellVisualKit` beside it, which is what the file holds and is no use to a
//! person: `4689` is not a thing anybody recognises. It is drawn now as what
//! the row *resolves to* — `Browser::describe`'s sentence — with the picture
//! where there is one, a dot saying whether the archives hold the model, a
//! `▶` for a sound, a picker that searches the target table by name, and the
//! name itself as the link that opens the row. The number is still there and
//! still editable; it is simply not the only thing on the line.
//!
//! ## A picker is a dialog, and a row can be made from a form
//!
//! The `…` beside a reference opens a dialog in the middle of the window — a
//! search box over the target table and the rows that match, with the
//! keyboard in the box — rather than a popup hanging off the button. A popup
//! is dismissed by egui on the first click it does not own, and a list that
//! has to be scrolled and typed into is not a thing a popup holds well.
//!
//! Beside the same reference: `+ new` when it names nothing, which makes a
//! blank row in the target table and points here at it; `copy` when it names
//! a kit or an effect, which copies that row and points here at the copy, so
//! the kit forty spells share becomes this visual's own; and `clone chain`
//! on a spell's visual, which copies the visual with every kit and effect it
//! names and points the spell at the copy. The list's own `+ New`, `Clone`
//! and `Delete` are the same three operations on the open table, and every
//! one of them is one entry on the undo stack. See `tools::tables`.
//!
//! ## The seven empty locales are folded
//!
//! Every string in `Spell.dbc` is eight columns and a flags word, and an enUS
//! install ships seven of the eight empty — so a form that drew them all was
//! two thirds blank boxes. They are real columns and they stay editable, under
//! an *Other locales* fold at the end of the section that is shut by default.
//!
//! ## …and the widths are deliberate
//!
//! [`LABEL`] and [`VALUE`] are what stop this reading as a debug dump. The
//! first draft put a small-text label and a default `DragValue` on one line and
//! the result was correct and unreadable: sixty rows of grey 11-point text with
//! nothing to fix the eye on. A label column wide enough for the longest name
//! in the schema, values at a common width, and a row tall enough to click is
//! the whole of the difference. The field index is a tooltip now rather than a
//! suffix on every label, along with what the schema knows about the column.

use super::theme;
use super::thumbnails::Thumbnails;
use crate::session::EditSession;
use crate::tools::tables::{self, Browser, Modal, RowLabel, MODEL_FOLDERS};
use vale_assets::tables::schema::{self, Column, Kind, Schema};
use vale_client::assets::GameAssets;
use bevy_egui::egui;

/// How wide the row list is.
const BROWSER_WIDTH: f32 = 320.0;
/// …and the gap between it and the form beside it, in points.
const FORM_GAP: f32 = 12.0;
/// How tall one row of it is, and how big the icon in it is — the editor's
/// own, so that this list and the item list are one list drawn twice. The
/// cache decodes at [`super::thumbnails::SIDE`]; an icon is a 64x64 BLP whose
/// 32 level is what comes back, so the picture is that level at its own size.
const ROW_HEIGHT: f32 = theme::LIST_ROW;
const ICON: f32 = theme::LIST_PICTURE;
/// The icon at the head of the form, which has room for more.
const HEAD_ICON: f32 = 36.0;
/// The label column. Wide enough for `EffectRealPointsPerLevel 1`, which is
/// the longest name in the spell schema.
///
/// These three are `super::rowform`'s, because the item and quest workspaces
/// draw their forms at the same widths: a change to one here is a change to
/// all three forms.
const LABEL: f32 = super::rowform::FORM_LABEL;
/// …and the common width of a value, so that a column of numbers lines up.
const VALUE: f32 = super::rowform::FORM_VALUE;
/// …and of a string, which wants the rest of the row.
const TEXT: f32 = super::rowform::FORM_TEXT;
/// How wide the visual chain is on the storyboard view, leaving the rest of the
/// middle for the preview.
const CHAIN_WIDTH: f32 = 480.0;
/// …and the narrowest the preview is allowed to become.
const STAGE_FLOOR: f32 = 360.0;
/// How wide a reference picker's dialog is, and how many rows a page of it
/// holds. A search over 22,360 spells is paged rather than cut off, so every
/// row is reachable; a page is what one scroll of the list comfortably is.
const PICKER_WIDTH: f32 = 380.0;

/// How tall the mask dialog's list may get before it scrolls.
///
/// The longest of them is 32 bits with a note under most of them, which is
/// well past a window; this is about a dozen rows, which is enough to see a
/// group without the dialog becoming the screen.
const BITS_HEIGHT: f32 = 420.0;
const PICKER_PAGE: usize = 100;
/// The model browser's rows: tall enough for a picture, and the picture's
/// side. See `crate::portraits`.
pub(super) const MODEL_ROW: f32 = 48.0;
const MODEL_PICTURE: f32 = 40.0;
/// …and how tall the model browser's own list is, which is what is left of a
/// 720-tall window once the preview pane under it has its room.
const MODEL_LIST_HEIGHT: f32 = 250.0;
/// The two entries after `MODEL_FOLDERS` on the browser's folder row: the
/// starred models and the recently used ones.
const STARRED_FOLDER: usize = MODEL_FOLDERS.len();
const RECENT_FOLDER: usize = MODEL_FOLDERS.len() + 1;
/// …and the icon picker, which is a grid: this many icons across at
/// [`super::thumbnails::ICON_SIDE`] points each, this many to a page.
const GRID_COLUMNS: usize = 8;
const GRID_PAGE: usize = 64;
/// The room an icon cell takes beyond the picture, for the hover ring.
const GRID_PAD: f32 = 8.0;

/// Everything the workspace reads, as one parameter.
///
/// It is four things and they arrive from four different places in the shell;
/// bundling them keeps the two entry points here to three arguments each rather
/// than seven.
pub struct Workspace<'a> {
    /// **Which data subject this is**, which decides the chain's tabs — see
    /// [`crate::tools::Tool::tabs`]. Carried rather than derived from
    /// `browser.table`, because a followed reference leaves the browser on a
    /// table that is not one of its chain's tabs and the tabs must not change
    /// under it.
    pub tool: crate::tools::Tool,
    pub session: &'a mut EditSession,
    pub browser: &'a mut Browser,
    pub assets: &'a GameAssets,
    pub thumbnails: &'a mut Thumbnails,
    /// The pictures of models, for the model browser's rows, and the two
    /// lists it keeps beside its folders.
    pub portraits: &'a mut crate::portraits::Portraits,
    pub favourites: &'a mut crate::favourites::Favourites,
    pub now: f64,
    /// **The two switches the model view's foot toggles**, which are the view
    /// bar's own: the debug overlay's wireframe and the world's particles.
    /// `None` where the caller has neither to hand — the inspector — and for
    /// the wireframe in a build without diagnostics, where the box is drawn
    /// disabled.
    pub wireframe: Option<&'a mut bool>,
    pub particles: Option<&'a mut bool>,
    /// **The lights on the open map**, for the one subject whose rows have a
    /// place — see [`crate::tools::lights`]. `None` where the caller has none
    /// to hand, which is the inspector.
    pub lights: Option<&'a mut crate::tools::lights::Lights>,
    /// **What time of day the world is showing**, in half-minutes past
    /// midnight, so a band's strip can mark where on it the viewport is
    /// standing. `vale_assets::tables::light::NOON` where the caller has no
    /// clock to hand.
    pub hour: u32,
}

impl Workspace<'_> {
    /// The archive path of a row's picture, or `None` for a row with none.
    ///
    /// A spell's is its icon, through the client's own tables and not the
    /// edited one: `SpellIcon` is a path by an id and changing that mapping is
    /// not what a spell editor is for. A `SpellIcon` row's is its own path.
    fn icon_of(&self, record: usize) -> Option<String> {
        match self.browser.table.as_str() {
            "Spell" => {
                let table = self.session.table("Spell")?;
                let icon_id = table.u32_at(record, schema::SPELL_ICON_FIELD)?;
                self.icon_by_id(icon_id)
            }
            "SpellIcon" => {
                let path = self.session.table("SpellIcon")?.string_at(record, 1)?;
                Some(with_blp(&path))
            }
            _ => None,
        }
    }

    /// …and by the `SpellIcon` row id itself, for the icon field's own preview.
    fn icon_by_id(&self, icon_id: u32) -> Option<String> {
        if icon_id == 0 {
            return None;
        }
        // The edited table first, so a re-pointed icon shows; the client's own
        // parse otherwise.
        if let Some(path) = self
            .session
            .table("SpellIcon")
            .and_then(|icons| icons.row_of(icon_id))
            .and_then(|row| self.session.table("SpellIcon")?.string_at(row, 1))
        {
            return Some(with_blp(&path));
        }
        let path = self
            .assets
            .display_tables()
            .ok()?
            .spellbook()?
            .icon_path(icon_id)?;
        Some(with_blp(&path))
    }

    /// Draw one at `side`, leaving a gap of the same size when there is nothing
    /// to draw — so a list of rows does not jitter as pictures arrive.
    fn icon(&mut self, ui: &mut egui::Ui, path: Option<String>, side: f32) {
        let size = egui::vec2(side, side);
        let Some(path) = path else {
            ui.allocate_space(size);
            return;
        };
        match self.picture(&path, side) {
            Some(id) => {
                ui.add(egui::Image::new(egui::load::SizedTexture::new(id, size)));
            }
            None => {
                ui.allocate_space(size);
            }
        }
    }

    /// The texture for a picture drawn at `side` points, asked for at the
    /// decode size that suits it: the list's level for a row, the file's own
    /// top level for anything larger.
    fn picture(&mut self, path: &str, side: f32) -> Option<egui::TextureId> {
        let decode = match side > super::thumbnails::SIDE as f32 {
            true => super::thumbnails::ICON_SIDE,
            false => super::thumbnails::SIDE,
        };
        self.thumbnails.want_at(path, decode);
        self.thumbnails.get_at(path, decode)
    }

    /// Whether the archives hold the model a `SpellVisualEffectName` row names.
    fn model_present(&self, id: u32) -> Option<bool> {
        let names = self.session.table("SpellVisualEffectName")?;
        let row = names.row_of(id)?;
        let declared = names.string_at(row, vale_assets::tables::spell::fields::EFFECT_MODEL)?;
        let path = vale_assets::world::m2::model_path(&declared);
        if path.is_empty() {
            return Some(false);
        }
        Some(
            self.assets
                .with_archive(|chain| Ok(chain.exists(&path)))
                .unwrap_or(false),
        )
    }
}

/// A column's value as the file states it: an `int32`, so the `-1` that half
/// the effect columns carry reads as `-1` and not as `4294967295`.
///
/// The cut is at `i32::MAX` rather than at some threshold of taste: below it
/// the two spellings agree, and above it no column in these tables holds a
/// real value — a row id runs to five figures and a mask is drawn as hex.
fn signed(raw: u32) -> String {
    match raw > i32::MAX as u32 {
        true => format!("{}", raw as i32),
        false => format!("{raw}"),
    }
}

/// **The tables carry no extension**, because the files do not write one and
/// the client appends it. See `ui::framexml::decode_rgba`, which is the same
/// rule one crate along.
fn with_blp(path: &str) -> String {
    match path.to_ascii_lowercase().ends_with(".blp") {
        true => path.to_string(),
        false => format!("{path}.blp"),
    }
}

/// Which view of the open row is drawn.
///
/// **A view and not a panel.** `Spell.dbc` row 74 is Fireball either way; what
/// changes is whether you are looking at its 173 columns or at the sequence
/// they describe. A second panel would have needed a second row selection to
/// keep in step with the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Fields,
    Storyboard,
}

/// Draw the whole workspace into the region the panels left.
pub fn draw(
    ui: &mut egui::Ui,
    mut work: Workspace<'_>,
    board: &mut super::storyboard::Storyboard,
    stage: &mut crate::stage::Stage,
    lab: &mut crate::lab::Lab,
) {
    let table_name = work.browser.table.clone();
    if work.session.table(&table_name).is_none() {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(format!("opening {table_name}.dbc…")).color(theme::INK_DIM));
        return;
    }

    // **Both views paint their own ground.** The preview used to be a hole in
    // the chrome and this was the fields view only; the stage renders into an
    // image now, so nothing needs the world behind it — and leaving the strip
    // above the panels unpainted was the world bleeding through the top of the
    // storyboard.
    let all = ui.available_rect_before_wrap();
    ui.painter().rect_filled(all, 0.0, theme::SHELL);

    // **Docked rather than laid out by hand.** A `horizontal` layout hands its
    // children a horizontal cursor, so a list built inside one runs across the
    // top of the window instead of down the side of it — which is what the
    // first draft did, 22,360 rows wide. A panel takes its space from the `Ui`
    // and gives its contents a column, which is what the shell itself does.
    //
    // **This is the [`Surface::Middle`](crate::tools::Surface::Middle)
    // workspace and only that one.** A data subject whose rows have a place in
    // the world is drawn in the inspector instead — see
    // [`light_inspector`] — so there is one arrangement here rather than two,
    // and the list has the width a list of 22,360 rows wants.
    egui::Panel::left("data-browser")
        .default_size(BROWSER_WIDTH)
        .min_size(220.0)
        .max_size(520.0)
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(8.0))
        .show(ui, |ui| {
            list(ui, &mut work);
        });
    // **A gap between the list and the form.** The panel's own margin ends at
    // its edge, and the form drew from the first point after it, so `< back`
    // and the section heads sat against the list's border. A child `Ui` inset
    // by [`FORM_GAP`] rather than a frame, because the storyboard and the lab
    // dock panels of their own into whatever `Ui` they are handed and a frame's
    // content rectangle is not settled until its contents are.
    let rect = ui.available_rect_before_wrap();
    let inset = egui::Rect::from_min_max(rect.min + egui::vec2(FORM_GAP, 0.0), rect.max);
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(inset).layout(*ui.layout()));
    form(&mut inner, &mut work, board, stage, lab);
}

/// The left column: the chain's tabs, a search box, and the rows that match.
fn list(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    // **The chain as tabs**, which is the reference tool's sidebar: a spell,
    // its visual, its kits and its effects are four tables a person moves
    // between constantly, and a followed reference is not the only way to
    // reach one. Which tables they are is the tool's — see `Tool::tabs`, since
    // there are two chains. A table that is not one of them — `SpellIcon`,
    // reached by a follow — lights no tab, and a press on one still goes
    // there.
    let tabs = work.tool.tabs();
    let before: &'static str = tabs
        .iter()
        .find(|(_, table)| *table == work.browser.table)
        .map(|(_, table)| *table)
        .unwrap_or("");
    let mut current = before;
    ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
        let options: Vec<(&str, &'static str)> = tabs.to_vec();
        theme::segmented(ui, &mut current, &options, |a, b| a == b);
    });
    if current != before {
        work.browser.look_at(current);
    }
    ui.add_space(6.0);

    let table_name = work.browser.table.clone();
    let count = work
        .session
        .table(&table_name)
        .map(|table| table.record_count())
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(&table_name)
                .strong()
                .size(15.0)
                .color(theme::INK),
        );
        ui.label(theme::number(format!("{count} rows")));
    });
    ui.add_space(4.0);
    let width = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(&mut work.browser.query)
            .hint_text("name, id, or what it belongs to")
            .desired_width(width),
    );
    ui.add_space(4.0);
    table_actions(ui, work);
    ui.add_space(2.0);
    row_actions(ui, work);
    ui.add_space(2.0);

    // `matches` is rebuilt only when the query or the table changed, so this is
    // a borrow of a cached list rather than a search per frame.
    let matches: Vec<usize> = work.browser.matches(work.session).to_vec();
    ui.label(
        egui::RichText::new(format!("{} shown", matches.len()))
            .small()
            .color(theme::INK_FAINT),
    );
    ui.add_space(4.0);

    let pictured = matches!(table_name.as_str(), "Spell" | "SpellIcon");
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
            for at in range {
                let record = matches[at];
                let label = work.browser.describe(work.session, &table_name, record);
                let chosen = work.browser.open == Some(record);
                let icon = pictured.then(|| work.icon_of(record)).flatten();
                if row(ui, work, &label, icon, chosen).clicked() {
                    work.browser.open = Some(record);
                    work.browser.forget_buffers();
                }
            }
        });
}

/// `Undo`, `Redo`, `Save`, `Discard` — the four things that are about the
/// table as a file rather than about a row of it.
///
/// Undo and redo are the inspector's own pair drawn again here, because the
/// keyboard's `Ctrl+Z` is egui's while a text box has the keyboard, and in a
/// workspace made of text boxes one usually does. Discard puts the table back
/// to what is written down and forgets its entries on the stack — see
/// `EditSession::discard_table` for why forgetting is the only honest thing
/// to do with them.
fn table_actions(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let table_name = work.browser.table.clone();
    let unsaved = work.session.unsaved_tables.contains(&table_name);
    let undo = work.session.history.next_undo().map(|c| c.label.clone());
    let redo = work.session.history.next_redo().map(|c| c.label.clone());
    // The open row by its id rather than its index, so a discard that
    // renumbers the table can find it again.
    let open_id = work
        .browser
        .open
        .and_then(|record| work.session.table(&table_name)?.u32_at(record, 0));
    ui.horizontal(|ui| {
        let quarter = ((ui.available_width() - 3.0 * ui.spacing().item_spacing.x) / 4.0).max(40.0);
        let size = egui::vec2(quarter, 22.0);
        if ui
            .add_enabled(undo.is_some(), egui::Button::new("Undo").min_size(size))
            .on_hover_text(match &undo {
                Some(label) => format!("Ctrl+Z — {label}"),
                None => "nothing to undo".to_string(),
            })
            .clicked()
        {
            work.session.undo(work.assets);
            work.browser.forget_buffers();
        }
        if ui
            .add_enabled(redo.is_some(), egui::Button::new("Redo").min_size(size))
            .on_hover_text(match &redo {
                Some(label) => format!("Ctrl+Y — {label}"),
                None => "nothing to redo".to_string(),
            })
            .clicked()
        {
            work.session.redo(work.assets);
            work.browser.forget_buffers();
        }
        if ui
            .add_enabled(unsaved, egui::Button::new("Save").min_size(size))
            .on_hover_text(format!(
                "write {table_name}.dbc into the project folder. Ctrl+S writes every table and tile."
            ))
            .on_disabled_hover_text("nothing has changed since the last save")
            .clicked()
        {
            work.session.save_table(&table_name);
        }
        if ui
            .add_enabled(unsaved, egui::Button::new("Discard").min_size(size))
            .on_hover_text(format!(
                "drop every unsaved change to {table_name}: read the file again and forget its \
                 entries on the undo stack. The rows and fields go back to what the project \
                 folder, or the archives, hold."
            ))
            .on_disabled_hover_text("nothing has changed since the last save")
            .clicked()
            && work.session.discard_table(work.assets, &table_name)
        {
            work.browser.forget_buffers();
            work.browser.open = open_id.and_then(|id| work.session.table(&table_name)?.row_of(id));
        }
    });
}

/// `+ New`, `Clone`, `Delete` — the three things that make or unmake a row of
/// the open table, each one entry on the undo stack. See `tools::tables`.
fn row_actions(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let table_name = work.browser.table.clone();
    let open = work.browser.open;
    let id = open
        .and_then(|record| work.session.table(&table_name)?.u32_at(record, 0))
        .unwrap_or(0);
    let uses = match open {
        Some(_) => work.browser.used_by(work.session, &table_name, id).len(),
        None => 0,
    };
    ui.horizontal(|ui| {
        let third = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(40.0);
        let size = egui::vec2(third, 22.0);
        if ui
            .add_sized(size, egui::Button::new("+ New"))
            .on_hover_text(format!("a blank {table_name} row under the next id"))
            .clicked()
        {
            if let Some(at) = tables::add_row(work.session, &table_name) {
                work.browser.open_row(at);
            }
        }
        if ui
            .add_enabled(open.is_some(), egui::Button::new("Clone").min_size(size))
            .on_hover_text(
                "a copy of the open row under the next id; a copied name gets \" (copy)\"",
            )
            .clicked()
        {
            if let Some(at) =
                open.and_then(|record| tables::clone_row(work.session, &table_name, record))
            {
                work.browser.open_row(at);
            }
        }
        let warning = match uses {
            0 => "remove the open row".to_string(),
            n => format!("remove the open row; {n} open reference(s) will then point at nothing"),
        };
        if ui
            .add_enabled(open.is_some(), egui::Button::new("Delete").min_size(size))
            .on_hover_text(warning)
            .clicked()
        {
            if let Some(record) = open {
                if tables::delete_row(work.session, &table_name, record) {
                    work.browser.close_row();
                }
            }
        }
    });
}

/// One row of the list: the picture, two lines of text, and the id on the
/// right. The whole rectangle is the button.
///
/// [`theme::list_row`] is the row, and every list in the editor is drawn by it
/// — see its own note. What is this panel's is only what goes in the picture,
/// which here is a spell icon out of the thumbnail cache.
fn row(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    label: &RowLabel,
    icon: Option<String>,
    chosen: bool,
) -> egui::Response {
    let title = match label.title.is_empty() {
        true => format!("row {}", label.id),
        false => label.title.clone(),
    };
    let shape = theme::list_row(
        ui,
        theme::ListRow {
            title: &title,
            sub: &label.sub,
            trailing: &label.id.to_string(),
            tint: theme::INK,
            picture: icon.is_some(),
        },
        chosen,
    );
    if let Some(path) = icon {
        if let Some(texture) = work.picture(&path, ICON) {
            ui.painter().image(
                texture,
                shape.picture,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }
    shape.response
}

/// The right column: the open row, section by section.
fn form(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    board: &mut super::storyboard::Storyboard,
    stage: &mut crate::stage::Stage,
    lab: &mut crate::lab::Lab,
) {
    let table_name = work.browser.table.clone();
    // A row removed, or an undo of one added, leaves an index past the end.
    if work
        .browser
        .open
        .zip(
            work.session
                .table(&table_name)
                .map(|table| table.record_count()),
        )
        .is_some_and(|(open, count)| open >= count)
    {
        work.browser.close_row();
    }
    let Some(record) = work.browser.open else {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Choose a row on the left.")
                    .size(14.0)
                    .color(theme::INK_DIM),
            );
        });
        stage.showing = None;
        stage.lab = false;
        modals(ui, work);
        return;
    };

    let schema = schema::for_table(&table_name);

    ui.add_space(8.0);
    head(ui, work, record, stage, lab);

    if table_name == "Light" {
        place_strip(ui, work, record);
    }

    // **Only a spell has a second view**, because only a spell names a chain.
    // The switch is not drawn at all on a table that has one view: an offer
    // that does nothing is worse than no offer.
    let has_story = table_name == "Spell";
    if has_story {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.allocate_ui(egui::vec2(240.0, 24.0), |ui| {
                theme::segmented(
                    ui,
                    &mut work.browser.view,
                    &[("Fields", View::Fields), ("Storyboard", View::Storyboard)],
                    |a, b| a == b,
                );
            });
        });
    } else {
        work.browser.view = View::Fields;
    }
    ui.add_space(8.0);

    if work.browser.view == View::Storyboard {
        let spell = work
            .session
            .table(&table_name)
            .and_then(|table| table.u32_at(record, 0));
        // **The pane has a floor and the chain gets the rest.** A preview
        // squeezed to a sliver is not a preview, and the first draft's fixed
        // 480-point chain left exactly that on a 1280-wide window.
        let here = ui.available_width();
        let chain = CHAIN_WIDTH.min((here - STAGE_FLOOR).max(300.0));
        egui::Panel::left("storyboard-chain")
            .default_size(chain)
            .min_size(280.0)
            .max_size((here - STAGE_FLOOR).max(300.0))
            .resizable(true)
            .frame(egui::Frame::new().fill(theme::SHELL).inner_margin(4.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("data-storyboard")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                        super::storyboard::draw(ui, work, board, record);
                    });
            });
        super::storyboard::stage_pane(ui, stage, spell, &board.notes, board);
        modals(ui, work);
        return;
    }

    // **An effect always has the pane**, drawn like the storyboard: the fields
    // on the left, the stage on the right with the model by itself — and, once
    // **Position on character…** is pressed, the lab's card above the fields
    // and the mannequin under the model. See `crate::lab`.
    let id = work
        .session
        .table(&table_name)
        .and_then(|table| table.u32_at(record, 0))
        .unwrap_or(0);
    if table_name == "SpellVisualEffectName" {
        if !lab.is_open_on(id) {
            super::lab::open_alone(lab, stage, work.browser, work.session, record);
        } else if let Some(table) = work.session.table(&table_name) {
            // The row's `Model` and `Scale` as they stand now — see
            // `Lab::follow_row`, which is what makes an edit to either show
            // in the pane beside it.
            use vale_assets::tables::spell::{effect_scale, fields};
            let model = table
                .string_at(record, fields::EFFECT_MODEL)
                .unwrap_or_default();
            let scale = effect_scale(table.f32_at(record, fields::EFFECT_SCALE));
            lab.follow_row(&model, scale);
        }
        // The browser's preview reaches the stage through the lab.
        lab.preview = work.browser.model_preview.clone();
        stage.showing = None;
        let here = ui.available_width();
        // The lab's card wants the width; an effect's five fields do not, so
        // the model view gives the pane the room. Two panel ids, so each
        // mode keeps the width it was dragged to.
        let on_character = lab.on_character(id);
        let form_width = match on_character {
            true => (CHAIN_WIDTH + 140.0).min((here - STAGE_FLOOR).max(320.0)),
            false => 360.0_f32.min((here - STAGE_FLOOR).max(320.0)),
        };
        // **Bounded to the panel as it was drawn last frame.** The panel's
        // inner `Ui` reports more width than the panel shows — a note wrapped
        // a hundred points past its edge — so the contents are held to the
        // rectangle the panel actually took, one frame behind a resize.
        let known = lab.panel_width;
        let shown = egui::Panel::left(match on_character {
            true => "lab-form",
            false => "model-form",
        })
        .default_size(form_width)
        .min_size(320.0)
        .max_size((here - STAGE_FLOOR).max(320.0))
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::SHELL).inner_margin(4.0))
        .show(ui, |ui| {
            if known > 0.0 {
                ui.set_max_width(known - 24.0);
            }
            egui::ScrollArea::vertical()
                .id_salt("data-lab")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if known > 0.0 {
                        ui.set_max_width(known - 24.0);
                    }
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 7.0);
                    ui.spacing_mut().interact_size.y = 24.0;
                    if lab.on_character(id) {
                        super::lab::card(ui, work, lab, record);
                        ui.add_space(8.0);
                    }
                    fields(ui, work, record, schema);
                });
        });
        lab.panel_width = shown.response.rect.width();
        super::lab::pane(ui, work, stage, lab);
        modals(ui, work);
        return;
    }
    // Off the storyboard and the lab, nothing is being previewed — which is
    // what takes the units off the stage and gives the camera back.
    stage.showing = None;
    stage.lab = false;

    egui::ScrollArea::vertical()
        .id_salt("data-form")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Rows tall enough to click and spaced enough to read. Set on the
            // form's own `Ui` rather than in the theme: this is the one panel in
            // the editor that is a page of fields rather than a few controls.
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 7.0);
            ui.spacing_mut().interact_size.y = 24.0;
            fields(ui, work, record, schema);
        });
    modals(ui, work);
}

/// The row's fields, section by section — or numbered, for a table with no
/// schema.
fn fields(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    record: usize,
    schema: Option<&'static Schema>,
) {
    match schema {
        Some(schema) => {
            for section in schema.sections {
                // **A light's sphere is drawn in the world's own units.**
                // The five columns behind it are in 1/36 of a yard measured
                // from the corner of the map — `557760`, `0`, `966720` — which
                // is not a number anybody can read or type. The same five
                // fields, converted, are below.
                if schema.table == "Light" && section.name == "Sphere" {
                    sphere_block(ui, work, record);
                    continue;
                }
                section_block(ui, work, record, schema, section);
            }
        }
        // **A table with no schema is still worth opening.** Numbered
        // fields with guessed types is what `vale dbc <Table>`
        // prints, and it is how the next schema gets written.
        None => unschemad(ui, work, record),
    }
    ui.add_space(24.0);
}

/// **A light's whole form, in the inspector** — what is picked in the world,
/// and the numbers behind it.
///
/// This is the lights tool's only panel. The subject is reached by pointing at
/// it in the viewport rather than by finding it in a list, so the shell's own
/// rule puts it here: the inspector is about the selection. The list of all 374
/// is a dialog behind **Browse…**, for finding one that is not on screen.
///
/// What it holds, in the order the work is done: which light this is, a button
/// to go and look at it, where it is and how far it reaches — in yards, not in
/// the file's 1/36 of one — and then the five `LightParams` rows it uses under
/// the five conditions, which is where the colours are.
pub fn light_inspector(ui: &mut egui::Ui, mut work: Workspace<'_>) {
    if !work.session.open_table(work.assets, "Light") {
        theme::note(ui, "opening Light.dbc…");
        return;
    }
    browse_button(ui, &mut work);
    ui.add_space(6.0);

    // **Whatever the browser is looking at, not always `Light`.** A press on a
    // `ParamsClear` link is a jump to `LightParams`, and the bands hang off
    // that row; a panel that drew the light's own form regardless would make
    // every one of those five links do nothing visible.
    //
    // **And the browser is not re-pointed here.** It was, on every frame,
    // against the light selected in the world — which undid a followed
    // reference on the frame after the press. Nothing needs it: the two places
    // that change the selection (the pick in the viewport, and the browse
    // dialog) each point the browser at the row themselves, which is the one
    // moment it should move. That is the same rule, and the same fault, as
    // `tables::open_tables`' own note about the rail's table.
    let table_name = work.browser.table.clone();
    let on_the_light = table_name == "Light";
    let Some(record) = work.browser.open else {
        theme::note(
            ui,
            "Click a light in the world to edit it. Every light near the camera \
             is marked; the one you pick gets its falloff drawn and its numbers \
             here.",
        );
        modals(ui, &mut work);
        return;
    };

    // **The way back from a followed reference**, which the middle workspace
    // has in its own heading and this one had nowhere at all: a jump between
    // tables with no way back is a dead end.
    if !work.browser.back.is_empty() {
        ui.horizontal(|ui| {
            if ui.button("< back").clicked() {
                work.browser.go_back();
            }
            if !on_the_light {
                ui.label(
                    egui::RichText::new(format!("in {table_name}"))
                        .size(12.0)
                        .color(theme::INK_FAINT),
                );
            }
        });
        ui.add_space(4.0);
    }

    let label = work.browser.describe(work.session, &table_name, record);
    let title = match label.title.is_empty() {
        true => format!("{table_name} row"),
        false => label.title.clone(),
    };
    theme::heading(ui, &title);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{table_name}.dbc  row {record}  ·  id {}",
                label.id
            ))
            .size(12.0)
            .color(theme::INK_FAINT),
        );
        if work.session.unsaved_tables.contains(&table_name) {
            ui.label(egui::RichText::new("unsaved").color(theme::WARN));
        }
    });
    ui.add_space(4.0);
    if on_the_light {
        place_strip(ui, &mut work, record);
        ui.add_space(6.0);
    }

    let schema = schema::for_table(&table_name);
    egui::ScrollArea::vertical()
        .id_salt("light-form")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            match (on_the_light, schema) {
                // The light's own form: the sphere in the world's units, then
                // the five `LightParams` rows it uses under the five
                // conditions, which is where the colours are. Drawn through the
                // ordinary section so the picker, the resolved name and the
                // link all behave as they do everywhere else.
                (true, Some(schema)) => {
                    sphere_block(ui, &mut work, record);
                    if let Some(section) = schema
                        .sections
                        .iter()
                        .find(|section| section.name == "Conditions")
                    {
                        section_block(ui, &mut work, record, schema, section);
                    }
                }
                // Anything followed out of it — a `LightParams` row, a
                // `LightSkybox` — is the ordinary form for that table.
                _ => {
                    fields(ui, &mut work, record, schema);
                    // **…and, for a params row, its bands.** They are the
                    // reason to be on this row at all: every colour the world
                    // is drawn in hangs off it, and nothing points at the two
                    // tables holding them, so this is the only route there.
                    if table_name == "LightParams" {
                        if let Some(id) = work
                            .session
                            .table("LightParams")
                            .and_then(|table| table.u32_at(record, 0))
                        {
                            let hour = work.hour;
                            super::bands::blocks(ui, &mut work, id, hour);
                        }
                    }
                }
            }
            ui.add_space(12.0);
        });
    modals(ui, &mut work);
}

/// **Find a light that is not on screen** — the list, as a dialog.
///
/// The tool is built around pointing at what you want, and that answers every
/// question except "where is the one I cannot see". A button rather than a
/// standing panel, because the standing panel was the thing in the way.
fn browse_button(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let count = work
        .session
        .table("Light")
        .map(|table| table.record_count())
        .unwrap_or(0);
    if ui
        .add_sized(
            egui::vec2(ui.available_width(), 24.0),
            egui::Button::new(format!("Browse all {count} lights…")),
        )
        .on_hover_text("For a light that is not on screen. Pick one and the camera goes to it.")
        .clicked()
    {
        work.browser.modal = Some(Modal::Rows { table: "Light" });
        work.browser.pick_query.clear();
        work.browser.pick_page = 0;
        work.browser.pick_focus = true;
    }
}

/// **A light's position and falloff, in yards** — the `Sphere` section, drawn
/// in the world's units instead of the file's.
///
/// **The unit changes and the names do not.** `FalloffStart` and `FalloffEnd`
/// are the columns' own names and keep them; the three coordinates are shown
/// as the world's `x`, `y` and `z`, because that is what they are once
/// converted and they are not the columns they come from — the axes swap, so a
/// field holding the world's x is `InternalZ` in the file. Each one's tooltip
/// says which column it writes and in what unit.
///
/// The five columns underneath are
/// [`light_field`](vale_assets::tables::light::light_field)'s: three
/// coordinates in the internal representation every placement in the game is
/// in, scaled by 36, and two radii in the same unit. Every one of them is
/// converted here rather than shown raw, and typing into one converts back —
/// [`vale_assets::world::adt::placement_from_world`] is the direction that
/// loading the game's own tiles never exercises, so it is called rather than
/// restated.
///
/// The raw columns are still reachable, folded away underneath, because they
/// are what is actually written to the file and a person checking a diff wants
/// to see them.
fn sphere_block(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    use vale_assets::tables::light::{light_field as lf, YARDS_PER_UNIT};
    use vale_assets::world::adt::{placement_from_world, placement_to_world};

    let Some(table) = work.session.table("Light") else {
        return;
    };
    let raw = |field: usize| table.f32_at(record, field).unwrap_or(0.0);
    let at = placement_to_world([
        raw(lf::INTERNAL_X) * YARDS_PER_UNIT,
        raw(lf::INTERNAL_Y) * YARDS_PER_UNIT,
        raw(lf::INTERNAL_Z) * YARDS_PER_UNIT,
    ]);
    let (mut x, mut y, mut z) = (at[0], at[1], at[2]);
    let mut start = raw(lf::FALLOFF_START) * YARDS_PER_UNIT;
    let mut end = raw(lf::FALLOFF_END) * YARDS_PER_UNIT;
    let (was_x, was_y, was_z, was_start, was_end) = (x, y, z, start, end);

    egui::CollapsingHeader::new(egui::RichText::new("Sphere").size(14.0).strong())
        .default_open(true)
        .show(ui, |ui| {
            let row = |ui: &mut egui::Ui, name: &str, value: &mut f32, tip: &str| {
                ui.horizontal(|ui| {
                    let label_width = LABEL.min(ui.available_width() * 0.45);
                    ui.add_sized(
                        egui::vec2(label_width, 22.0),
                        egui::Label::new(egui::RichText::new(name).color(theme::INK_DIM))
                            .truncate(),
                    )
                    .on_hover_text(tip);
                    ui.add_sized(
                        egui::vec2(VALUE, 22.0),
                        egui::DragValue::new(value).speed(1.0).suffix(" yd"),
                    );
                });
            };
            row(
                ui,
                "x",
                &mut x,
                "The world's x. Stored as InternalZ, which is northward from \
                 the corner of the map in 1/36 of a yard.",
            );
            row(
                ui,
                "y",
                &mut y,
                "The world's y. Stored as InternalX, which is westward from \
                 the corner of the map in 1/36 of a yard.",
            );
            row(
                ui,
                "z",
                &mut z,
                "The world's z. Stored as InternalY, in 1/36 of a yard, and \
                 the one coordinate a drag in the world does not move — a \
                 pointer aims at two.",
            );
            ui.add_space(4.0);
            row(
                ui,
                "FalloffStart",
                &mut start,
                "Within this radius the light applies at full strength. The \
                 inner sphere in the world, and it can be dragged there. \
                 Stored in 1/36 of a yard.",
            );
            row(
                ui,
                "FalloffEnd",
                &mut end,
                "…and by this radius it has faded to nothing. Zero makes this \
                 the map's default light, which applies everywhere and whose \
                 position is not read. Stored in 1/36 of a yard.",
            );
        });

    // **Written only when one of them moved**, and all five under one gesture,
    // so a drag on the x box is one entry on the stack and not one per pixel.
    if (x, y, z, start, end) == (was_x, was_y, was_z, was_start, was_end) {
        return;
    }
    let placement = placement_from_world([x, y, z]);
    let store = |yards: f32| (yards / YARDS_PER_UNIT).to_bits();
    tables::set_fields(
        work.session,
        "Light",
        record,
        &[
            (lf::INTERNAL_X, store(placement[0])),
            (lf::INTERNAL_Y, store(placement[1])),
            (lf::INTERNAL_Z, store(placement[2])),
            (lf::FALLOFF_START, store(start)),
            (lf::FALLOFF_END, store(end)),
        ],
        "Edit light sphere",
        "Light sphere",
        work.now,
    );
}

/// **Where a light is, and how to go and look at it** — the strip under a
/// `Light` row's heading.
///
/// The row's three coordinates are stored in a representation nobody can read
/// (`612096, 0, 998400`, in 1/36 of a yard from the corner of the map), so the
/// form alone cannot tell you where the light you are editing *is*. This says
/// it in the world's own numbers, keeps the ring in the viewport on the row
/// that is open, and flies the camera there.
///
/// It is drawn only for `Light`. The other four tables of the chain have no
/// position: a `LightParams` row is a look, and where it applies is whichever
/// lights point at it.
fn place_strip(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    let Some(id) = work
        .session
        .table("Light")
        .and_then(|table| table.u32_at(record, 0))
    else {
        return;
    };
    let Some(lights) = work.lights.as_deref_mut() else {
        return;
    };
    // **Opening a row selects its light.** The pick does this the other way
    // round already; without this half, a row reached from the list or by
    // following a reference leaves the viewport highlighting the previous one,
    // which is the ring a person would then go and drag.
    if lights.selected != Some(id) {
        lights.selected = Some(id);
    }
    let Some(&mark) = lights.marks.iter().find(|mark| mark.id == id) else {
        // A light on another map. The row still edits — a `Light` row names its
        // own map — and there is nothing to fly to from here.
        theme::note(
            ui,
            "This light is on another map. Open that map to see it in the world.",
        );
        return;
    };

    ui.horizontal(|ui| {
        if mark.everywhere {
            // The default light applies everywhere and its coordinates are not
            // read, so offering to fly to them would be offering to fly to a
            // number that means nothing.
            ui.label(
                egui::RichText::new("the whole map")
                    .size(13.0)
                    .color(theme::INK_DIM),
            );
        } else {
            if ui
                .button("Fly to")
                .on_hover_text("Put the camera on this light. The height is the row's own.")
                .clicked()
            {
                lights.fly_to(&mark);
            }
            ui.label(
                theme::number(format!(
                    "{:.0}, {:.0}, {:.0}",
                    mark.at.x, mark.at.y, mark.at.z
                ))
                .color(theme::INK_DIM),
            );
            ui.label(
                egui::RichText::new(format!("{:.0}..{:.0} yd", mark.start, mark.end))
                    .size(13.0)
                    .color(theme::INK_DIM),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut lights.show_falloff, "falloff")
                .on_hover_text(
                    "Draw this light's two spheres, or only its marker. Every \
                     light near the camera is marked either way.",
                );
        });
    });
}

/// The head of the form: where it came from, the picture, the name, the id,
/// and who points at it.
fn head(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    record: usize,
    stage: &mut crate::stage::Stage,
    lab: &mut crate::lab::Lab,
) {
    let table_name = work.browser.table.clone();
    let label = work.browser.describe(work.session, &table_name, record);
    let icon = work.icon_of(record);
    let mut open_lab = false;
    let mut clone_chain = false;
    ui.horizontal(|ui| {
        if !work.browser.back.is_empty() && ui.button("< back").clicked() {
            work.browser.go_back();
        }
        work.icon(ui, icon, HEAD_ICON);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                let title = match label.title.is_empty() {
                    true => format!("{table_name} row"),
                    false => label.title.clone(),
                };
                ui.label(egui::RichText::new(title).strong().size(17.0).color(theme::INK));
                ui.label(theme::number(format!("#{}", label.id)).color(theme::INK_DIM));
                if work.session.unsaved_tables.contains(&table_name) {
                    ui.label(egui::RichText::new("unsaved").color(theme::WARN));
                }
                // **What can be done to this row that is not a field.**
                match table_name.as_str() {
                    "SpellVisual" => {
                        clone_chain = ui
                            .button("Clone chain")
                            .on_hover_text(
                                "Copy this visual with every kit and effect it names, each rewired \
                                 to the copies, and open the copy. The original and the spells that \
                                 share it are untouched.",
                            )
                            .clicked();
                    }
                    "SpellVisualEffectName" if !lab.on_character(label.id) => {
                        open_lab = ui
                            .button("Position on character…")
                            .on_hover_text(
                                "Stand this model on a body at a real attachment point, move it, \
                                 and bake the offset into a copy of the model. 1.12 has no offset \
                                 fields: the position lives in the file.",
                            )
                            .clicked();
                    }
                    _ => {}
                }
            });
            ui.horizontal(|ui| {
                if !label.sub.is_empty() {
                    ui.label(egui::RichText::new(&label.sub).color(theme::INK_DIM));
                }
                ui.label(
                    egui::RichText::new(format!("{table_name}.dbc · row {record}"))
                        .small()
                        .color(theme::INK_FAINT),
                );
            });
        });
    });
    // **Who points here**, in a line — the full list is in the inspector. A
    // spell is pointed at by other spells only through trigger columns, which
    // is rarely what somebody opened it for, so the line is for the chain
    // tables.
    if table_name != "Spell" {
        used_by_line(ui, work, &table_name, label.id);
    }
    if open_lab {
        super::lab::open_on(lab, stage, work.browser, work.session, record);
    }
    if clone_chain {
        if let Some(done) = tables::clone_chain(work.session, label.id, None) {
            let copy = done.new_id("SpellVisual", label.id).unwrap_or(0);
            work.session.status = format!(
                "visual {} copied to {copy}: {} kits, {} effects",
                label.id,
                done.count("SpellVisualKit"),
                done.count("SpellVisualEffectName")
            );
            follow_reference(work, "SpellVisual", copy);
        }
    }
}

/// `Used by 9 spells: Fireball, Fireball, … · 2 kits` — clickable.
fn used_by_line(ui: &mut egui::Ui, work: &mut Workspace<'_>, table_name: &str, id: u32) {
    let uses = work.browser.used_by(work.session, table_name, id);
    if uses.is_empty() {
        theme::note(ui, "Nothing open points at this row.");
        return;
    }
    let mut follow: Option<(String, usize)> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(
            egui::RichText::new("Used by")
                .small()
                .color(theme::INK_FAINT),
        );
        // Grouped by table, the spells first because they are what a person is
        // looking for.
        let mut sources: Vec<String> = uses.iter().map(|at| at.table.clone()).collect();
        sources.sort();
        sources.dedup();
        sources.sort_by_key(|table| table != "Spell");
        for (which, source) in sources.iter().enumerate() {
            let here: Vec<usize> = uses
                .iter()
                .filter(|at| &at.table == source)
                .map(|at| at.record)
                .collect();
            if which > 0 {
                ui.label(egui::RichText::new("·").small().color(theme::INK_FAINT));
            }
            ui.label(
                egui::RichText::new(format!("{} {}", here.len(), plural(source, here.len())))
                    .small()
                    .color(theme::INK_DIM),
            );
            let mut shown = Vec::new();
            for &at in &here {
                let label = work.browser.describe(work.session, source, at);
                let name = match label.title.is_empty() {
                    true => format!("#{}", label.id),
                    false => label.title.clone(),
                };
                // The same spell nine times over is nine ranks; say it once.
                if shown.contains(&name) {
                    continue;
                }
                if shown.len() >= 6 {
                    ui.label(egui::RichText::new("…").small().color(theme::INK_FAINT));
                    break;
                }
                if ui
                    .add(egui::Link::new(egui::RichText::new(&name).small()))
                    .clicked()
                {
                    follow = Some((source.clone(), at));
                }
                shown.push(name);
            }
        }
    });
    if let Some((table, record)) = follow {
        let id = work
            .session
            .table(&table)
            .and_then(|open| open.u32_at(record, 0))
            .unwrap_or(0);
        follow_reference(work, &table, id);
    }
}

/// `spell` / `spells`, by the table's own name.
fn plural(table: &str, count: usize) -> String {
    let word = match table {
        "Spell" => "spell",
        "SpellVisual" => "visual",
        "SpellVisualKit" => "kit",
        other => other,
    };
    match count {
        1 => word.to_string(),
        _ => format!("{word}s"),
    }
}

/// One section of the form: its fields, with the locale columns folded.
fn section_block(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    record: usize,
    schema: &Schema,
    section: &schema::Section,
) {
    let (main, other): (Vec<&Column>, Vec<&Column>) = section
        .fields
        .iter()
        .filter_map(|&field| schema.column(field))
        .partition(|column| !matches!(column.kind, Kind::Locale(_) | Kind::LocaleFlags));
    egui::CollapsingHeader::new(egui::RichText::new(section.name).size(14.0).strong())
        .default_open(true)
        .show(ui, |ui| {
            for column in main {
                field_row(ui, work, record, column);
            }
            if !other.is_empty() {
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!("Other locales ({})", other.len()))
                        .size(12.0)
                        .color(theme::INK_DIM),
                )
                .id_salt((section.name, "locales"))
                .default_open(false)
                .show(ui, |ui| {
                    for column in other {
                        field_row(ui, work, record, column);
                    }
                });
            }
        });
}

/// One field: its name, and the widget its kind asks for.
fn field_row(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize, column: &Column) {
    let table_name = work.browser.table.clone();
    let Some(raw) = work
        .session
        .table(&table_name)
        .and_then(|table| table.u32_at(record, column.field))
    else {
        return;
    };

    ui.horizontal(|ui| {
        // The index and what the schema knows are a tooltip: read when wanted,
        // and not sixty `[117]`s down the page otherwise.
        let mut tip = format!("field {}", column.field);
        if !column.about.is_empty() {
            tip.push_str("\n\n");
            tip.push_str(column.about);
        }
        // **At most [`LABEL`], and at most half of what there is.** The
        // constant is the middle workspace's, where the form has 700 points to
        // itself; the same form is drawn in the inspector now, and a fixed 210
        // there left the resolved name of every reference clipped against the
        // window's edge.
        let label_width = LABEL.min(ui.available_width() * 0.45);
        ui.add_sized(
            egui::vec2(label_width, 22.0),
            egui::Label::new(egui::RichText::new(column.name).color(theme::INK_DIM)).truncate(),
        )
        .on_hover_text(tip);
        match column.kind {
            Kind::Id => {
                ui.label(theme::number(format!("{raw}")));
            }
            Kind::Int | Kind::LocaleFlags => {
                let mut value = raw;
                if number(ui, &mut value).changed() {
                    write(work, record, column, value);
                }
            }
            // **Unused is signed for `reference`'s reason**, and it is the
            // column most likely to be holding the `-1` this table writes for
            // nothing: `SpellVisualKit`'s own unused slot carries one on
            // almost every row. An `Int` stays unsigned — those are counts and
            // durations — and so do the locale flags, which are a mask.
            Kind::Signed | Kind::Unused => {
                let mut value = raw as i32;
                if number(ui, &mut value).changed() {
                    write(work, record, column, value as u32);
                }
            }
            Kind::Float => {
                let mut value = f32::from_bits(raw);
                let response = ui.add_sized(
                    egui::vec2(VALUE, 22.0),
                    egui::DragValue::new(&mut value).speed(0.01),
                );
                if response.changed() {
                    write(work, record, column, value.to_bits());
                }
            }
            Kind::Flags(bits) => {
                let mut value = raw;
                if number(ui, &mut value).changed() {
                    write(work, record, column, value);
                }
                // **The list, where there is one.** A mask is read bit by bit
                // by everything that consumes it, so the number is the
                // *encoding* and the bits are the value — see
                // `vale_assets::tables::spellbits`. Without this a person
                // setting "castable while dead" had to know 0x00800000.
                if !bits.is_empty()
                    && ui
                        .add(
                            egui::Button::new(egui::RichText::new("…").size(13.0))
                                .min_size(egui::vec2(24.0, 22.0)),
                        )
                        .on_hover_text(format!("tick the bits of {}", column.name))
                        .clicked()
                {
                    work.browser.modal = Some(Modal::Bits {
                        record,
                        field: column.field,
                        column: column.name,
                        bits,
                    });
                }
                ui.label(theme::number(format!("0x{raw:08X}")));
                // …and what is set, in words, without opening anything. The
                // one-line form is what a form is read down; the dialog is for
                // changing it.
                let set = schema::named_bits(raw, bits);
                if !set.is_empty() {
                    ui.label(
                        egui::RichText::new(set.join(" · "))
                            .small()
                            .color(theme::INK_DIM),
                    );
                }
                // **A bit nothing names is said rather than hidden**, because
                // it is either a bit this list has not learnt or a value
                // somebody has typed by hand, and both are worth seeing.
                let unnamed = schema::unnamed_bits(raw, bits);
                if unnamed != 0 {
                    ui.label(
                        egui::RichText::new(format!("+0x{unnamed:08X} unnamed"))
                            .small()
                            .color(theme::WARN),
                    );
                }
            }
            Kind::Bool => {
                let mut on = raw != 0;
                if ui.checkbox(&mut on, "").changed() {
                    write(work, record, column, u32::from(on));
                }
                // A gate with a value that is not 0 or 1 is said, not hidden.
                if raw > 1 {
                    ui.label(theme::number(format!("{raw}")).color(theme::WARN));
                }
            }
            Kind::Enum(names) => {
                // A value the list does not name is shown as the number,
                // which is what the game does — **as a signed one**, for
                // `reference`'s reason: these columns are `int32` and the
                // "nothing here" they carry is `-1`, which as a `u32` reads
                // as `4294967295`.
                let shown = names
                    .iter()
                    .find(|&&(value, _)| value == raw)
                    .map(|&(_, name)| name.to_string())
                    .unwrap_or_else(|| signed(raw));
                let mut chosen = raw;
                egui::ComboBox::from_id_salt((record, column.field))
                    .selected_text(egui::RichText::new(shown).size(13.5))
                    .width(TEXT * 0.6)
                    .show_ui(ui, |ui| {
                        for &(value, name) in names {
                            ui.selectable_value(&mut chosen, value, format!("{value}  {name}"));
                        }
                    });
                if chosen != raw {
                    write(work, record, column, chosen);
                }
            }
            Kind::Colour => {
                let mut rgb = [
                    ((raw >> 16) & 0xff) as u8,
                    ((raw >> 8) & 0xff) as u8,
                    (raw & 0xff) as u8,
                ];
                // **The top byte rides through untouched.** It is zero on all
                // but one shipped `LightIntBand` row, and a picker that
                // rebuilt the word from three channels would quietly clear the
                // `0xff` that row carries — see `Kind::Colour`.
                let keep = raw & 0xff00_0000;
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    let packed =
                        (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
                    write(work, record, column, keep | packed);
                }
                ui.label(
                    theme::number(format!("{} {} {}", rgb[0], rgb[1], rgb[2]))
                        .color(theme::INK_DIM),
                );
                // The word itself, because the packing order is the thing most
                // likely to be in doubt when somebody is reading this column.
                ui.label(theme::number(format!("0x{raw:06X}")).color(theme::INK_DIM));
            }
            Kind::Reference(points_at) => reference(ui, work, record, column, points_at, raw),
            Kind::Text | Kind::Locale(_) => {
                let wide = column.name.contains("Description");
                let text = work
                    .browser
                    .buffer(work.session, record, column.field)
                    .clone();
                let mut editing = text.clone();
                // No wider than the row has room for: beside the lab's pane
                // the form is narrow, and a fixed width ran off its edge.
                let width = TEXT.min((ui.available_width() - 90.0).max(140.0));
                let response = match wide {
                    true => ui.add(
                        egui::TextEdit::multiline(&mut editing)
                            .desired_rows(3)
                            .desired_width(width),
                    ),
                    false => ui.add(
                        egui::TextEdit::singleline(&mut editing)
                            .desired_width(width)
                            .min_size(egui::vec2(width, 24.0)),
                    ),
                };
                if editing != text {
                    *work.browser.buffer(work.session, record, column.field) = editing.clone();
                }
                // **Written when the box is left, not on every keystroke.**
                // The string block is append-only, so a write per character
                // would put one dead copy of every prefix of the word in the
                // file. See `vale_edit::dbc`.
                if response.lost_focus() {
                    let now_in_file = work
                        .session
                        .table(&table_name)
                        .and_then(|table| table.string_at(record, column.field))
                        .unwrap_or_default();
                    if editing != now_in_file {
                        tables::set_text(
                            work.session,
                            &table_name,
                            record,
                            column.field,
                            &editing,
                            &format!("Edit {}", column.name),
                            work.now,
                        );
                    }
                }
                // A model path is checked against the archives as it is
                // typed, since a path that opens nothing draws nothing.
                if table_name == "SpellVisualEffectName"
                    && column.field == vale_assets::tables::spell::fields::EFFECT_MODEL
                {
                    let id = work
                        .session
                        .table(&table_name)
                        .and_then(|table| table.u32_at(record, 0))
                        .unwrap_or(0);
                    presence(ui, work.model_present(id));
                    if ui
                        .small_button("browse…")
                        .on_hover_text("every model in the archives, by name")
                        .clicked()
                    {
                        work.browser.modal = Some(Modal::Models {
                            record,
                            field: column.field,
                        });
                        work.browser.model_query.clear();
                        work.browser.pick_focus = true;
                    }
                }
            }
        }
    });
}

/// A reference column: the number, what it resolves to, and the ways through.
///
/// See the module comment. The number stays editable because a person who
/// knows the id types it; the picker is for the one who does not.
fn reference(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    record: usize,
    column: &Column,
    points_at: &'static str,
    raw: u32,
) {
    // **Signed, because the column is.** A reference holds a row id, which is
    // positive, or one of the table's two ways of saying nothing — `0` and
    // `-1`. Drawn as the `u32` it is read as, the second of those is
    // `4294967295`, which is a number nobody recognises sitting where an id
    // goes: the kits are full of them, since `blank_defaults` and the shipped
    // rows both write `-1`. The label beside it has always said "none"; this
    // makes the box agree.
    let mut value = raw as i32;
    if number(ui, &mut value).changed() {
        write(work, record, column, value as u32);
    }
    // **The picker**, which searches the target table by what its rows resolve
    // to. A dialog rather than a popup — see the module comment.
    if ui
        .add(
            egui::Button::new(egui::RichText::new("…").size(13.0)).min_size(egui::vec2(24.0, 22.0)),
        )
        .on_hover_text(format!("choose a {points_at} row by name"))
        .clicked()
    {
        work.browser.modal = Some(Modal::Pick {
            record,
            field: column.field,
            points_at,
        });
        work.browser.pick_query.clear();
        work.browser.pick_focus = true;
    }
    let table_name = work.browser.table.clone();
    let chain_table = matches!(
        points_at,
        "SpellVisual" | "SpellVisualKit" | "SpellVisualEffectName"
    );

    // **Whether `0` means "nothing" is asked of the table, not assumed.**
    // Every table of the spell chain numbers from 1, so a `0` there is one of
    // the two ways a column says nothing. `Map.dbc` numbers from 0 and map 0
    // is Azeroth, so `Light.Map` read by that rule showed `none` on 232 of
    // 374 lights — the most common value in the column, drawn as absent. The
    // table itself settles it: if it has a row 0, `0` is a reference to it.
    let zero_is_a_row = work
        .session
        .table(points_at)
        .is_some_and(|table| table.row_of(0).is_some());
    if (raw == 0 && !zero_is_a_row) || raw == u32::MAX {
        ui.label(egui::RichText::new("none").color(theme::INK_FAINT));
        ui.label(
            egui::RichText::new(points_at)
                .small()
                .color(theme::INK_FAINT),
        );
        if chain_table
            && ui
                .small_button("+ new")
                .on_hover_text(format!(
                    "make a blank {points_at} row and point this field at it"
                ))
                .clicked()
        {
            if let Some(id) =
                tables::link_new(work.session, &table_name, record, column.field, points_at)
            {
                work.browser.forget_buffers();
                follow_reference(work, points_at, id);
            }
        }
        return;
    }

    // What the row is, in words — and a picture where the table has one.
    if points_at == "SpellIcon" {
        let icon = work.icon_by_id(raw);
        work.icon(ui, icon, ICON);
    }
    if points_at == "SpellVisualEffectName" {
        presence(ui, work.model_present(raw));
    }
    // The target may not be open yet — the chain arrives one table a frame —
    // in which case the number is all there is for a moment.
    let resolved = work
        .session
        .open_table(work.assets, points_at)
        .then(|| work.browser.describe_id(work.session, points_at, raw))
        .flatten();
    match resolved {
        Some(label) => {
            let title = match label.title.is_empty() {
                true => format!("{points_at} {raw}"),
                false => label.title.clone(),
            };
            if ui
                .add(egui::Link::new(
                    egui::RichText::new(title).color(theme::ACCENT),
                ))
                .on_hover_text(format!("open {points_at} {raw}"))
                .clicked()
            {
                follow_reference(work, points_at, raw);
            }
            if !label.sub.is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&label.sub)
                            .small()
                            .color(theme::INK_DIM),
                    )
                    .truncate(),
                );
            }
        }
        None => {
            ui.label(
                egui::RichText::new(format!("not a row of {points_at}"))
                    .small()
                    .color(theme::BAD),
            );
        }
    }
    if points_at == "SoundEntries"
        && ui
            .small_button("▶")
            .on_hover_text("play this sound")
            .clicked()
    {
        work.browser.audition.push(raw);
    }
    // **Making a shared row this row's own.** A kit or an effect is copied and
    // this field pointed at the copy; a visual is copied with its whole chain.
    if matches!(points_at, "SpellVisualKit" | "SpellVisualEffectName")
        && ui
            .small_button("copy")
            .on_hover_text(
                "copy the row this points at and point this field at the copy, leaving \
                 the original to whatever else names it",
            )
            .clicked()
    {
        if let Some(id) =
            tables::unshare(work.session, &table_name, record, column.field, points_at)
        {
            work.browser.forget_buffers();
            follow_reference(work, points_at, id);
        }
    }
    if points_at == "SpellVisual"
        && ui
            .small_button("clone chain")
            .on_hover_text(
                "copy this visual with every kit and effect it names, and point this field \
                 at the copy: this row then names the copy and every other spell keeps \
                 the original",
            )
            .clicked()
    {
        if let Some(done) =
            tables::clone_chain(work.session, raw, Some((&table_name, record, column.field)))
        {
            let copy = done.new_id("SpellVisual", raw).unwrap_or(0);
            work.session.status = format!(
                "visual {raw} copied to {copy} and assigned: {} kits, {} effects",
                done.count("SpellVisualKit"),
                done.count("SpellVisualEffectName")
            );
        }
    }
}

/// A dot saying whether the archives hold a model, with the reason on hover.
fn presence(ui: &mut egui::Ui, present: Option<bool>) {
    let (colour, why) = match present {
        Some(true) => (theme::GOOD, "the archives hold this model"),
        Some(false) => (theme::BAD, "not in the archives: this effect draws nothing"),
        None => (theme::INK_FAINT, "SpellVisualEffectName is not open yet"),
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, colour);
    response.on_hover_text(why);
}

/// The dialog that is up, if one is: a reference picker or the model browser.
/// Drawn once per frame from the form, whichever field opened it.
///
/// **Both are paged.** The first draft cut a picker at two hundred hits and
/// the model browser at three hundred, which for the twelve thousand icons or
/// the six thousand models is a list most of which cannot be reached. A page
/// is turned with the arrows, `PageUp` and `PageDown`, and a search narrows
/// it as before — and the page is put back to the first whenever the hits
/// are rebuilt.
fn modals(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let Some(modal) = work.browser.modal.clone() else {
        return;
    };
    let table_name = work.browser.table.clone();
    let mut close = false;
    match modal {
        Modal::Pick {
            record,
            field,
            points_at,
        } => {
            if !work.session.open_table(work.assets, points_at) {
                work.browser.modal = None;
                return;
            }
            let column_name = schema::for_table(&table_name)
                .and_then(|schema| schema.column(field))
                .map(|column| column.name)
                .unwrap_or("this field");
            // **An icon is chosen by looking at it**, so the icon table is a
            // grid of pictures at the file's own size rather than rows with a
            // thumbnail beside a file name.
            let grid = points_at == "SpellIcon";
            let width = match grid {
                true => {
                    GRID_COLUMNS as f32 * (super::thumbnails::ICON_SIDE as f32 + GRID_PAD + 6.0)
                        + 12.0
                }
                false => PICKER_WIDTH,
            };
            let per_page = match grid {
                true => GRID_PAGE,
                false => PICKER_PAGE,
            };
            let mut picked: Option<u32> = None;
            let response = egui::Modal::new(egui::Id::new("data-pick")).show(ui.ctx(), |ui| {
                ui.set_width(width);
                ui.label(
                    egui::RichText::new(format!("{points_at} for {column_name}"))
                        .strong()
                        .size(14.0),
                );
                let box_ = ui.add(
                    egui::TextEdit::singleline(&mut work.browser.pick_query)
                        .hint_text("name or id")
                        .desired_width(width),
                );
                if work.browser.pick_focus {
                    box_.request_focus();
                    work.browser.pick_focus = false;
                }
                let total = work
                    .browser
                    .picks(work.session, points_at)
                    .map(|hits| hits.len())
                    .unwrap_or(0);
                let range = pager(ui, &mut work.browser.pick_page, total, per_page);
                let rows: Vec<usize> = work
                    .browser
                    .picks(work.session, points_at)
                    .map(|hits| hits[range.clone()].to_vec())
                    .unwrap_or_default();
                let page = work.browser.pick_page;
                if grid {
                    egui::ScrollArea::vertical()
                        .id_salt(("pick-grid", page))
                        .max_height(
                            GRID_PAGE as f32 / GRID_COLUMNS as f32
                                * (super::thumbnails::ICON_SIDE as f32 + GRID_PAD + 6.0),
                        )
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                                for here in rows {
                                    let label =
                                        work.browser.describe(work.session, points_at, here);
                                    let path = work
                                        .session
                                        .table("SpellIcon")
                                        .and_then(|icons| icons.string_at(here, 1))
                                        .map(|path| with_blp(&path));
                                    if icon_cell(ui, work, path, &label).clicked() {
                                        picked = Some(label.id);
                                    }
                                }
                            });
                        });
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt(("pick-rows", page))
                        .max_height(380.0)
                        .auto_shrink([false, true])
                        .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                            for at in range {
                                let here = rows[at];
                                let label = work.browser.describe(work.session, points_at, here);
                                if row(ui, work, &label, None, false).clicked() {
                                    picked = Some(label.id);
                                }
                            }
                        });
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    ui.label(
                        egui::RichText::new("Esc closes · PageUp and PageDown turn the pages")
                            .small()
                            .color(theme::INK_FAINT),
                    );
                });
            });
            if let Some(id) = picked {
                tables::set_field(
                    work.session,
                    &table_name,
                    record,
                    field,
                    id,
                    &format!("Edit {column_name}"),
                    work.now,
                );
                close = true;
            }
            if response.should_close() {
                close = true;
            }
        }
        Modal::Rows { table } => {
            if !work.session.open_table(work.assets, table) {
                work.browser.modal = None;
                return;
            }
            let mut picked: Option<u32> = None;
            let response = egui::Modal::new(egui::Id::new("data-rows")).show(ui.ctx(), |ui| {
                ui.set_width(PICKER_WIDTH);
                ui.label(egui::RichText::new(table).strong().size(14.0));
                let box_ = ui.add(
                    egui::TextEdit::singleline(&mut work.browser.pick_query)
                        .hint_text("name, id, or where it is")
                        .desired_width(PICKER_WIDTH),
                );
                if work.browser.pick_focus {
                    box_.request_focus();
                    work.browser.pick_focus = false;
                }
                let total = work
                    .browser
                    .picks(work.session, table)
                    .map(|hits| hits.len())
                    .unwrap_or(0);
                let range = pager(ui, &mut work.browser.pick_page, total, PICKER_PAGE);
                let rows: Vec<usize> = work
                    .browser
                    .picks(work.session, table)
                    .map(|hits| hits[range.clone()].to_vec())
                    .unwrap_or_default();
                let page = work.browser.pick_page;
                egui::ScrollArea::vertical()
                    .id_salt(("rows-list", page))
                    .max_height(380.0)
                    .auto_shrink([false, true])
                    .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                        for at in range {
                            let here = rows[at];
                            let label = work.browser.describe(work.session, table, here);
                            if row(ui, work, &label, None, false).clicked() {
                                picked = Some(label.id);
                            }
                        }
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    ui.label(
                        egui::RichText::new("Esc closes · PageUp and PageDown turn the pages")
                            .small()
                            .color(theme::INK_FAINT),
                    );
                });
            });
            if let Some(id) = picked {
                work.browser.look_at(table);
                let session = &*work.session;
                work.browser.follow(session, table, id);
                // **Chosen here means chosen in the world too**, and the camera
                // goes to it: the whole reason to open this list is a light that
                // cannot be seen, so leaving the view where it was would answer
                // half the question.
                if table == "Light" {
                    if let Some(lights) = work.lights.as_deref_mut() {
                        lights.selected = Some(id);
                        if let Some(mark) = lights.marks.iter().copied().find(|m| m.id == id) {
                            if !mark.everywhere {
                                lights.fly_to(&mark);
                            }
                        }
                    }
                }
                close = true;
            }
            if response.should_close() {
                close = true;
            }
        }
        Modal::Bits {
            record,
            field,
            column,
            bits,
        } => {
            // **The mask as what it is: a list of switches.** Every bit gets a
            // row — its name, its note where vmangos wrote one, and its mask —
            // and the value is rebuilt from the ticks rather than typed.
            let raw = work
                .session
                .table(&work.browser.table.clone())
                .and_then(|table| table.u32_at(record, field))
                .unwrap_or(0);
            let mut wanted = raw;
            let response = egui::Modal::new(egui::Id::new("data-bits")).show(ui.ctx(), |ui| {
                ui.set_width(PICKER_WIDTH);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(column).strong().size(14.0));
                    ui.label(theme::number(format!("0x{raw:08X}")));
                });
                theme::note(
                    ui,
                    "The names are vmangos' — 1.12 ships no table for these, so the server's \
                     own enums are the only authority there is.",
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(BITS_HEIGHT)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (mask, name, note) in bits {
                            let mut on = wanted & mask != 0;
                            ui.horizontal(|ui| {
                                if ui.checkbox(&mut on, *name).changed() {
                                    wanted = match on {
                                        true => wanted | mask,
                                        false => wanted & !mask,
                                    };
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            theme::number(format!("0x{mask:08X}"))
                                                .color(theme::INK_FAINT),
                                        );
                                    },
                                );
                            });
                            if !note.is_empty() {
                                ui.label(
                                    egui::RichText::new(*note).small().color(theme::INK_FAINT),
                                );
                            }
                        }
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                    // **Clearing is a button** rather than thirty-two clicks,
                    // and it is the ordinary way a mask is emptied.
                    if ui.button("None").on_hover_text("clear every bit").clicked() {
                        wanted = 0;
                    }
                    ui.label(
                        egui::RichText::new("Esc closes")
                            .small()
                            .color(theme::INK_FAINT),
                    );
                });
            });
            if wanted != raw {
                let table_name = work.browser.table.clone();
                tables::set_field(
                    work.session,
                    &table_name,
                    record,
                    field,
                    wanted,
                    &format!("Edit {column}"),
                    work.now,
                );
            }
            if response.should_close() {
                close = true;
            }
        }
        Modal::Models { record, field } => {
            if work.browser.models.is_none() {
                let all = work
                    .assets
                    .with_archive(|chain| Ok(chain.list_prefix("")))
                    .unwrap_or_default();
                work.browser.models = Some(
                    all.into_iter()
                        .filter(|path| path.ends_with(".m2"))
                        .collect(),
                );
            }
            // **It opens on the model the field already names.** Without this
            // the dialog came up with nothing previewed, its pane absent, and
            // **Use** greyed out reading "nothing chosen" — on a row whose
            // model is written three lines above the button. Seeded here
            // rather than at either door, because there are two: the form's
            // `browse…` and `--browse`.
            if work.browser.model_preview.is_none() {
                let current = work
                    .session
                    .table(&table_name)
                    .and_then(|table| table.string_at(record, field))
                    .filter(|declared| !declared.trim().is_empty())
                    .map(|declared| {
                        vale_assets::world::m2::model_path(&declared).to_ascii_lowercase()
                    });
                work.browser.model_preview = current;
            }
            let mut picked: Option<String> = None;
            // **A lighter backdrop than a picker's**, because the stage behind
            // it is part of this dialog: a press on a row previews that model
            // there, and a backdrop that hid it would hide the whole point.
            let response = egui::Modal::new(egui::Id::new("data-models"))
                .backdrop_color(egui::Color32::from_black_alpha(60))
                .show(ui.ctx(), |ui| {
                    ui.set_width(PICKER_WIDTH + 200.0);
                    ui.label(egui::RichText::new("Model").strong().size(14.0));
                    ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
                        // The archives' folders, then the two lists the person
                        // keeps — see `crate::favourites`.
                        let mut options: Vec<(&str, usize)> =
                            MODEL_FOLDERS.iter().enumerate().map(|(at, (name, _))| (*name, at)).collect();
                        options.push(("Starred", STARRED_FOLDER));
                        options.push(("Recent", RECENT_FOLDER));
                        let before = work.browser.model_folder;
                        theme::segmented(ui, &mut work.browser.model_folder, &options, |a, b| a == b);
                        if before != work.browser.model_folder {
                            work.browser.model_page = 0;
                        }
                    });
                    let box_ = ui.add(
                        egui::TextEdit::singleline(&mut work.browser.model_query)
                            .hint_text("words from the path")
                            .desired_width(ui.available_width()),
                    );
                    if box_.changed() {
                        work.browser.model_page = 0;
                    }
                    if work.browser.pick_focus {
                        box_.request_focus();
                        work.browser.pick_focus = false;
                    }
                    let folder = MODEL_FOLDERS
                        .get(work.browser.model_folder)
                        .map(|(_, prefix)| *prefix)
                        .unwrap_or("");
                    let words: Vec<String> = work
                        .browser
                        .model_query
                        .to_ascii_lowercase()
                        .split_whitespace()
                        .map(str::to_string)
                        .collect();
                    let kind = crate::favourites::Kind::Effect;
                    let source: Vec<String> = match work.browser.model_folder {
                        STARRED_FOLDER => work.favourites.starred(kind),
                        RECENT_FOLDER => work.favourites.recent(kind),
                        _ => work
                            .browser
                            .models
                            .as_deref()
                            .unwrap_or(&[])
                            .iter()
                            .filter(|path| path.starts_with(folder))
                            .cloned()
                            .collect(),
                    };
                    let hits: Vec<String> = source
                        .into_iter()
                        .filter(|path| words.iter().all(|word| path.contains(word.as_str())))
                        .collect();
                    if hits.is_empty() && words.is_empty() {
                        theme::note(
                            ui,
                            match work.browser.model_folder {
                                STARRED_FOLDER => "nothing starred yet: press the star on a row",
                                RECENT_FOLDER => "nothing used yet: a model taken with Use lands here",
                                _ => "no models under this folder",
                            },
                        );
                    }
                    let range = pager(ui, &mut work.browser.model_page, hits.len(), PICKER_PAGE);
                    let page = work.browser.model_page;
                    let previewing = work.browser.model_preview.clone();
                    let rows = &hits[range];
                    let portraits = &mut *work.portraits;
                    let favourites = &mut *work.favourites;
                    let browser = &mut *work.browser;
                    egui::ScrollArea::vertical()
                        .id_salt(("models", page))
                        // **Shorter than the 380 it was**, because the preview
                        // pane goes under it: the two together at the old
                        // height put the dialog's own buttons past the foot of
                        // a 720-tall window.
                        .max_height(MODEL_LIST_HEIGHT)
                        .auto_shrink([false, true])
                        .show_rows(ui, MODEL_ROW, rows.len(), |ui, range| {
                            for at in range {
                                let path = &rows[at];
                                let lit = previewing.as_deref() == Some(path.as_str());
                                let hit = model_row(ui, portraits, favourites, kind, path, lit);
                                if hit.starred {
                                    favourites.toggle(kind, path);
                                }
                                if hit.double {
                                    picked = Some(path.clone());
                                } else if hit.clicked {
                                    browser.model_preview = Some(path.clone());
                                }
                            }
                        });
                    // **The model being previewed, under the list.** The stage
                    // behind the dialog shows it too, and that is what the
                    // lighter backdrop is for; this is the one that can be
                    // seen without looking past a dialog, and it is the same
                    // pane the placement pickers carry.
                    if let Some(path) = work.browser.model_preview.clone() {
                        ui.add_space(4.0);
                        super::inspector::preview_pane(ui, work.portraits, &path);
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let chosen = work.browser.model_preview.clone();
                        let name = chosen.as_deref().map(tables::basename).unwrap_or("nothing chosen");
                        if ui
                            .add_enabled(
                                chosen.is_some(),
                                egui::Button::new(egui::RichText::new(format!("Use {name}")).color(
                                    egui::Color32::from_rgb(0x0C, 0x16, 0x1E),
                                ))
                                .fill(theme::ACCENT),
                            )
                            .on_hover_text("point this effect's Model at the row being previewed")
                            .clicked()
                        {
                            picked = chosen;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        ui.label(
                            egui::RichText::new(
                                "a press previews the model on the stage; a double press, or Use, takes it",
                            )
                            .small()
                            .color(theme::INK_FAINT),
                        );
                    });
                });
            if let Some(path) = picked {
                tables::set_text(
                    work.session,
                    &table_name,
                    record,
                    field,
                    &path,
                    "Edit Model",
                    work.now,
                );
                work.favourites.used(crate::favourites::Kind::Effect, &path);
                work.browser.forget_buffers();
                close = true;
            }
            if response.should_close() {
                close = true;
            }
            if close {
                work.browser.model_preview = None;
            }
        }
    }
    if close {
        work.browser.modal = None;
    }
}

/// What a press on a model row did.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RowHit {
    pub clicked: bool,
    pub double: bool,
    /// The star was pressed, which is not a press on the row.
    pub starred: bool,
}

/// One row of a model list: the picture, the file's name with its folder
/// under it, and a star at the right.
///
/// Shared with the inspector's placement picker, which lists the same kind
/// of thing one tool over; `kind` is which list the star writes to.
pub(super) fn model_row(
    ui: &mut egui::Ui,
    portraits: &mut crate::portraits::Portraits,
    favourites: &crate::favourites::Favourites,
    kind: crate::favourites::Kind,
    path: &str,
    lit: bool,
) -> RowHit {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), MODEL_ROW - 2.0),
        egui::Sense::click(),
    );
    let mut hit = RowHit::default();
    if !ui.is_rect_visible(rect) {
        return hit;
    }
    if lit || response.hovered() {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(3),
            match lit {
                true => theme::ACCENT_SUNK,
                false => theme::RAISED,
            },
        );
    }
    let picture = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 3.0, rect.center().y - MODEL_PICTURE * 0.5),
        egui::vec2(MODEL_PICTURE, MODEL_PICTURE),
    );
    crate::portraits::paint(ui, portraits, path, picture);
    let star = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 14.0, rect.center().y),
        egui::vec2(18.0, 18.0),
    );
    let text = ui.painter().with_clip_rect(egui::Rect::from_min_max(
        egui::pos2(picture.right() + 8.0, rect.top()),
        egui::pos2(star.left() - 4.0, rect.bottom()),
    ));
    let (folder, file) = match path.rfind(['\\', '/']) {
        Some(at) => (&path[..at], &path[at + 1..]),
        None => ("", path),
    };
    text.text(
        egui::pos2(picture.right() + 8.0, rect.center().y - 7.0),
        egui::Align2::LEFT_CENTER,
        file,
        egui::FontId::proportional(12.0),
        match lit {
            true => theme::INK,
            false => theme::INK_DIM,
        },
    );
    text.text(
        egui::pos2(picture.right() + 8.0, rect.center().y + 7.0),
        egui::Align2::LEFT_CENTER,
        folder,
        egui::FontId::proportional(10.0),
        theme::INK_FAINT,
    );
    let on = favourites.is_starred(kind, path);
    let hint = match on {
        true => "Take this model off the starred list.",
        false => "Star this model, so it is one click away under Starred.",
    };
    hit.starred = theme::star_button(ui, ui.id().with(("star", path)), star, on, hint);
    if !hit.starred {
        let response = response.on_hover_text(path.to_string());
        hit.double = response.double_clicked();
        hit.clicked = response.clicked();
    }
    hit
}

/// The page controls: first, back, `page 3 of 12 · 201–300 of 1,183`,
/// forward, last; `PageUp` and `PageDown` on the keyboard. Clamps the page
/// to what there is and answers the range of the list that is on it.
fn pager(
    ui: &mut egui::Ui,
    page: &mut usize,
    total: usize,
    per_page: usize,
) -> std::ops::Range<usize> {
    let (pages, _) = page_bounds(total, *page, per_page);
    let (back, forward) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::PageUp),
            i.key_pressed(egui::Key::PageDown),
        )
    });
    if back {
        *page = page.saturating_sub(1);
    }
    if forward {
        *page = (*page + 1).min(pages - 1);
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(*page > 0, egui::Button::new("|◀")).clicked() {
            *page = 0;
        }
        if ui.add_enabled(*page > 0, egui::Button::new("◀")).clicked() {
            *page -= 1;
        }
        let (_, range) = page_bounds(total, *page, per_page);
        let where_ = match total {
            0 => "no matches".to_string(),
            _ => format!(
                "page {} of {pages} · {}–{} of {total}",
                *page + 1,
                range.start + 1,
                range.end
            ),
        };
        ui.label(egui::RichText::new(where_).small().color(theme::INK_FAINT));
        if ui
            .add_enabled(*page + 1 < pages, egui::Button::new("▶"))
            .clicked()
        {
            *page += 1;
        }
        if ui
            .add_enabled(*page + 1 < pages, egui::Button::new("▶|"))
            .clicked()
        {
            *page = pages - 1;
        }
    });
    page_bounds(total, *page, per_page).1
}

/// How many pages a list of `total` makes at `per_page`, and which of its
/// rows page `page` holds — the last page when `page` is past the end, and
/// an empty range for an empty list. Always at least one page, so a page
/// number is always a page.
fn page_bounds(total: usize, page: usize, per_page: usize) -> (usize, std::ops::Range<usize>) {
    let per_page = per_page.max(1);
    let pages = total.div_ceil(per_page).max(1);
    let page = page.min(pages - 1);
    let start = (page * per_page).min(total);
    let end = (start + per_page).min(total);
    (pages, start..end)
}

/// One cell of the icon grid: the picture at its own size, a ring when the
/// pointer is on it, and the row's name, id and path on hover. The whole
/// cell is the button.
fn icon_cell(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    path: Option<String>,
    label: &RowLabel,
) -> egui::Response {
    let side = super::thumbnails::ICON_SIDE as f32;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(side + GRID_PAD, side + GRID_PAD),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(3),
            theme::RAISED,
            egui::Stroke::new(1.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    let inner = rect.shrink(GRID_PAD / 2.0);
    match path.as_deref().and_then(|path| work.picture(path, side)) {
        Some(id) => {
            egui::Image::new(egui::load::SizedTexture::new(id, inner.size())).paint_at(ui, inner);
        }
        None => {
            ui.painter()
                .rect_filled(inner, egui::CornerRadius::same(2), theme::SUNK);
        }
    }
    let title = match label.title.is_empty() {
        true => format!("row {}", label.id),
        false => label.title.clone(),
    };
    response.on_hover_text(match path {
        Some(path) => format!("{title}  #{}\n{path}", label.id),
        None => format!("{title}  #{}", label.id),
    })
}

/// A number field at the common width.
fn number<T: egui::emath::Numeric>(ui: &mut egui::Ui, value: &mut T) -> egui::Response {
    ui.add_sized(egui::vec2(VALUE, 22.0), egui::DragValue::new(value))
}

/// **Open the table a reference names, at the row it names.**
///
/// The table is opened here rather than only being looked up, which is the
/// whole of why the button used to do nothing: `SpellVisual.dbc` is not open
/// until something asks for it, and the first thing to ask is the press that
/// wants to look at it.
pub fn follow_reference(work: &mut Workspace<'_>, table: &str, id: u32) {
    if !work.session.open_table(work.assets, table) {
        return;
    }
    work.browser.index(work.session, table);
    if !work.browser.follow(work.session, table, id) {
        work.session.status = format!("{table} has no row {id}");
    }
}

/// A table with no schema: every field, numbered, with what it might be.
fn unschemad(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    let table_name = work.browser.table.clone();
    let Some(table) = work.session.table(&table_name) else {
        return;
    };
    let fields = table.field_count();
    theme::note(
        ui,
        "No schema for this table yet: the columns are numbered and their types \
         are guesses. See vale_assets::tables::schema.",
    );
    for field in 0..fields {
        let Some(raw) = work
            .session
            .table(&table_name)
            .and_then(|table| table.u32_at(record, field))
        else {
            continue;
        };
        let text = work
            .session
            .table(&table_name)
            .and_then(|table| table.string_at(record, field))
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.add_sized(
                egui::vec2(70.0, 22.0),
                egui::Label::new(theme::number(format!("[{field}]"))),
            );
            let mut value = raw;
            if number(ui, &mut value).changed() {
                tables::set_field(
                    work.session,
                    &table_name,
                    record,
                    field,
                    value,
                    &format!("Edit field {field}"),
                    work.now,
                );
            }
            let guess = f32::from_bits(raw);
            if raw != 0 && guess.is_finite() && guess.abs() > 1.0e-4 && guess.abs() < 1.0e6 {
                ui.label(egui::RichText::new(format!("f32 {guess:.3}")).color(theme::INK_FAINT));
            }
            if !text.is_empty() && text.is_ascii() {
                ui.label(egui::RichText::new(format!("{text:?}")).color(theme::INK_FAINT));
            }
        });
    }
}

fn write(work: &mut Workspace<'_>, record: usize, column: &Column, value: u32) {
    let table_name = work.browser.table.clone();
    tables::set_field(
        work.session,
        &table_name,
        record,
        column.field,
        value,
        &format!("Edit {}", column.name),
        work.now,
    );
}

/// **What this row points at, and what points at it** — the inspector's half
/// of the workspace.
///
/// It is in the inspector rather than in the middle because it is about the
/// selection, which is the rule this shell already has for where a control
/// goes. See [`super`].
pub fn references(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let table_name = work.browser.table.clone();
    let Some(record) = work.browser.open else {
        theme::note(ui, "Choose a row to see what it points at.");
        return;
    };
    let Some(schema) = schema::for_table(&table_name) else {
        return;
    };

    theme::heading(ui, "Points at");
    let mut drawn = 0;
    let mut follow: Option<(&'static str, u32)> = None;
    for column in schema.columns.iter() {
        let Kind::Reference(points_at) = column.kind else {
            continue;
        };
        let Some(value) = work
            .session
            .table(&table_name)
            .and_then(|table| table.u32_at(record, column.field))
        else {
            continue;
        };
        if value == 0 || value == u32::MAX {
            continue;
        }
        drawn += 1;
        let resolved = work.browser.describe_id(work.session, points_at, value);
        let text = match resolved {
            Some(label) if !label.title.is_empty() => format!("{} [{value}]", label.title),
            _ => format!("{points_at} {value}"),
        };
        ui.horizontal(|ui| {
            ui.add_sized(
                [theme::LABEL_WIDTH + 30.0, ui.spacing().interact_size.y],
                egui::Label::new(egui::RichText::new(column.name).color(theme::INK_DIM))
                    .truncate()
                    .selectable(false),
            );
            if ui
                .add(egui::Link::new(egui::RichText::new(text).size(11.5)))
                .on_hover_text(format!("open {points_at} {value}"))
                .clicked()
            {
                follow = Some((points_at, value));
            }
        });
    }
    if drawn == 0 {
        theme::note(ui, "Every reference column of this row is zero.");
    }
    if let Some((table, id)) = follow {
        follow_reference(work, table, id);
    }

    // …and the other direction, which is the whole of what an index is for.
    theme::heading(ui, "Used by");
    let id = work
        .session
        .table(&table_name)
        .and_then(|table| table.u32_at(record, 0))
        .unwrap_or(0);
    let uses = work.browser.used_by(work.session, &table_name, id);
    if uses.is_empty() {
        theme::note(ui, "Nothing open points at this row.");
        return;
    }
    let mut sources: Vec<String> = uses.iter().map(|at| at.table.clone()).collect();
    sources.sort();
    sources.dedup();
    sources.sort_by_key(|table| table != "Spell");
    let mut follow: Option<(String, usize)> = None;
    for source in sources {
        let here: Vec<&tables::Use> = uses.iter().filter(|at| at.table == source).collect();
        ui.label(
            egui::RichText::new(format!("{} {}", here.len(), plural(&source, here.len())))
                .small()
                .color(theme::INK_FAINT),
        );
        for at in here.iter().take(12) {
            let label = work.browser.describe(work.session, &source, at.record);
            let column = schema::for_table(&source)
                .and_then(|schema| schema.column(at.field))
                .map(|column| column.name)
                .unwrap_or("?");
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let name = match label.title.is_empty() {
                    true => format!("#{}", label.id),
                    false => format!("{} [{}]", label.title, label.id),
                };
                if ui
                    .add(egui::Link::new(egui::RichText::new(name).size(11.5)))
                    .on_hover_text(format!("open {source} {}", label.id))
                    .clicked()
                {
                    follow = Some((source.clone(), at.record));
                }
                ui.label(egui::RichText::new(column).small().color(theme::INK_FAINT));
            });
        }
        if here.len() > 12 {
            theme::note(ui, format!("…and {} more", here.len() - 12));
        }
    }
    if let Some((table, record)) = follow {
        let id = work
            .session
            .table(&table)
            .and_then(|open| open.u32_at(record, 0))
            .unwrap_or(0);
        follow_reference(work, &table, id);
    }
}

/// Whether a schema exists for a table, for a caller deciding what to draw.
pub fn described(table: &str) -> Option<&'static Schema> {
    schema::for_table(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page is a range into the list, the last page is short, a page past
    /// the end is the last one, and an empty list is one empty page.
    #[test]
    fn a_page_is_a_range_and_never_off_the_end() {
        assert_eq!(page_bounds(0, 0, 100), (1, 0..0));
        assert_eq!(page_bounds(0, 7, 100), (1, 0..0));
        assert_eq!(page_bounds(100, 0, 100), (1, 0..100));
        assert_eq!(page_bounds(101, 0, 100), (2, 0..100));
        assert_eq!(page_bounds(101, 1, 100), (2, 100..101));
        assert_eq!(page_bounds(1183, 2, 100), (12, 200..300));
        assert_eq!(page_bounds(1183, 11, 100), (12, 1100..1183));
        assert_eq!(
            page_bounds(1183, 50, 100),
            (12, 1100..1183),
            "past the end is the last page"
        );
        assert_eq!(page_bounds(5, 0, 0), (5, 0..1), "a page holds at least one");
    }

    /// **`-1` reads as `-1`.** Half the effect columns of a kit carry it —
    /// it is what `blank_defaults` and the shipped rows write for "nothing
    /// here" — and drawn as the `u32` the reader returns it is
    /// `4294967295`, sitting where a row id goes.
    #[test]
    fn a_column_that_says_nothing_reads_as_minus_one() {
        assert_eq!(signed(u32::MAX), "-1");
        assert_eq!(signed(0), "0");
        assert_eq!(signed(3068), "3068");
        assert_eq!(signed(i32::MAX as u32), "2147483647");
        assert_eq!(signed(i32::MAX as u32 + 1), "-2147483648");
    }
}
