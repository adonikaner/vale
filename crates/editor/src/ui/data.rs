//! The workspace a table is edited in: the browser, and the form beside it.
//!
//! ## Why a data subject replaces the viewport
//!
//! For the tools on the rail's world half, *Terrain* and *World*, the world is
//! the thing being edited: the panels surround it and it fills the middle. A
//! spell has no place in the world, and a 300-point inspector beside an
//! unrelated view of terrain is too narrow for its form.
//!
//! A *Data* subject therefore replaces the middle region with a searchable list
//! on the left and the row's fields in the rest. The rest of the shell is
//! unchanged: the same top bar, rail, view bar, status line and undo stack, so
//! `Ctrl+Z` undoes the last edit whether it was to a wall or to a spell's name.
//!
//! The layout is a list, a form, and a drill-down into the detail, all inside
//! the shell rather than in separate windows. The shell has no floating panels
//! for data, because a floating panel of loose controls looks like a debug
//! window.
//!
//! ## Three workspaces draw this panel
//!
//! Spells, with the spell chain and the skill tables as two rows of tabs.
//! Sets, the `ItemSet` table, which is a part of the Items workspace: the
//! strip at the head of the list switches to the item list and back
//! (`super::rail::parts`). Tables, whose list is every file of
//! `DBFilesClient\` until one is chosen ([`table_list`]), and which then
//! draws that table like any other, with a link back to the list.
//!
//! ## A row that exists only for another row is edited on that row's form
//!
//! The form is one table's columns, with three additions where a subject is
//! spread over tables. A spell's form has a Learning section ([`learning`]):
//! the `SkillLineAbility` rows that put the spell in a skill line, with
//! their classes, races and next rank, and the spells that teach it, each
//! with a button that makes one. A skill line's form lists the
//! `SkillRaceClassInfo` rows that say who has it ([`skill_line_members`]).
//! A set's item column says when the item's own `set_id` does not name the
//! set, and a picked item has it written.
//!
//! Each is optional: a spell in no skill line and with no teaching spell is
//! the common case and reads as one line each. The rows are ordinary rows of
//! their tables, drawn through [`field_in`] with the table named, so the
//! per-table tabs show the same rows. A row many rows share (a kit, a cast
//! time, an icon) stays a reference with a picker: editing it from one
//! row's form would change the others without saying so.
//!
//! ## The schema decides each field's widget
//!
//! `vale_assets::tables::schema` says what each column is: a number, a mask, a
//! gate, one of a named set, a row id in another table, or a string. This
//! module draws the matching widget. It has no knowledge of spells; pointed at
//! another table with a schema, it draws that table.
//!
//! The schema exists because a table of 173 unnamed numeric columns cannot be
//! edited safely. A table with no schema is still drawn and edited
//! ([`unschemad`]): every field as its number, with a float box where the
//! four bytes read as a float and a text box where they are the offset of a
//! string.
//!
//! ## An item column is named by the database
//!
//! `ItemSet`'s seventeen item columns hold `item_template` entries, and 1.12
//! ships no item table, so the name beside one and the picker behind its `…`
//! are the item form's: the quest tool's name cache and reference picker
//! (`super::reference`, `super::quests::picker`). With no world database the
//! column is its number.
//!
//! ## How a reference column is drawn
//!
//! A reference column shows what the target row resolves to
//! (`Browser::describe`'s sentence) rather than only its id, because an id
//! such as `4689` means nothing to a person. Beside it are the row's picture
//! where it has one, a dot saying whether the archives hold the model, a `▶`
//! for a sound, and a picker that searches the target table by name. The
//! resolved name is a link that opens the target row. The id stays on the line
//! and stays editable.
//!
//! ## Reference pickers, and creating rows from a form
//!
//! The `…` beside a reference opens a dialog in the middle of the window: a
//! search box over the target table, with keyboard focus, and the matching
//! rows. It is a dialog rather than a popup on the button because egui closes
//! a popup on the first click outside it, which does not suit a list that must
//! be scrolled and typed into.
//!
//! The same reference also offers: `+ new` when it names nothing, which makes a
//! blank row in the target table and points this column at it; `copy` when it
//! names a kit or an effect, which copies that row and points this column at
//! the copy, so a kit shared by forty spells becomes this visual's own; and
//! `clone chain` on a spell's visual, which copies the visual with every kit
//! and effect it names and points the spell at the copy. The list's `+ New`,
//! `Clone` and `Delete` are the same three operations on the open table. Each
//! of these operations is one entry on the undo stack. See `tools::tables`.
//!
//! ## Other locales are folded
//!
//! Every string in `Spell.dbc` is eight columns and a flags word, and an enUS
//! install leaves seven of the eight empty, so drawing them all would make two
//! thirds of the form blank boxes. They are real columns and stay editable,
//! under an *Other locales* fold at the end of the section that is closed by
//! default.
//!
//! ## Field widths
//!
//! [`LABEL`] and [`VALUE`] set the label column and the common value width. A
//! small-text label followed by a default-width `DragValue` on each line gave
//! sixty rows of grey 11-point text with no alignment, which was hard to read.
//! The form uses a label column wide enough for the longest name in the schema,
//! values at a common width, and rows tall enough to click. The field index and
//! what the schema knows about the column are in the label's tooltip rather
//! than a suffix on every label.

use super::theme;
use super::thumbnails::Thumbnails;
use crate::session::EditSession;
use crate::tools::tables::{self, Browser, Modal, RowLabel, MODEL_FOLDERS};
use vale_assets::tables::schema::{self, Column, Kind, Schema};
use vale_client::assets::GameAssets;
use bevy_egui::egui;

/// How wide the row list is.
const BROWSER_WIDTH: f32 = 320.0;
/// The gap between the row list and the form beside it, in points.
const FORM_GAP: f32 = 12.0;
/// The height of one row of the list and the size of its icon. These are the
/// editor's shared list sizes, so this list and the item list look the same.
/// The cache decodes at [`super::thumbnails::SIDE`]; an icon is a 64x64 BLP whose
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
/// The common width of a value, so that a column of numbers lines up.
const VALUE: f32 = super::rowform::FORM_VALUE;
/// The width of a string value, which takes the rest of the row.
const TEXT: f32 = super::rowform::FORM_TEXT;
/// How wide the visual chain is on the storyboard view, leaving the rest of the
/// middle for the preview.
const CHAIN_WIDTH: f32 = 480.0;
/// The minimum width of the storyboard preview.
const STAGE_FLOOR: f32 = 360.0;
/// How wide a reference picker's dialog is, and how many rows a page of it
/// holds. A search over 22,360 spells is paged rather than cut off, so every
/// row is reachable; a page is what one scroll of the list comfortably is.
const PICKER_WIDTH: f32 = 380.0;

/// How tall the mask dialog's list may get before it scrolls.
///
/// The longest mask has 32 bits, most with a note, which is taller than a
/// window. This height shows about a dozen rows, enough to see a group of bits
/// without the dialog filling the screen.
const BITS_HEIGHT: f32 = 420.0;
const PICKER_PAGE: usize = 100;
/// The model browser's rows: tall enough for a picture, and the picture's
/// side. See `crate::portraits`.
pub(super) const MODEL_ROW: f32 = 48.0;
const MODEL_PICTURE: f32 = 40.0;
/// The height of the model browser's list: what is left of a 720-tall window
/// after the preview pane under it.
const MODEL_LIST_HEIGHT: f32 = 250.0;
/// The two entries after `MODEL_FOLDERS` on the browser's folder row: the
/// starred models and the recently used ones.
const STARRED_FOLDER: usize = MODEL_FOLDERS.len();
const RECENT_FOLDER: usize = MODEL_FOLDERS.len() + 1;
/// The icon picker's grid: this many icons across at
/// [`super::thumbnails::ICON_SIDE`] points each, this many to a page.
const GRID_COLUMNS: usize = 8;
const GRID_PAGE: usize = 64;
/// The room an icon cell takes beyond the picture, for the hover ring.
const GRID_PAD: f32 = 8.0;

/// Everything the workspace reads, as one parameter.
///
/// Its four fields come from four different places in the shell. Bundling them
/// keeps the two entry points here to three arguments each rather than seven.
pub struct Workspace<'a> {
    /// The data subject, which decides the chain's tabs (see
    /// [`crate::tools::Tool::tabs`]). It is passed in rather than derived from
    /// `browser.table`, because a followed reference can leave the browser on
    /// a table that is not one of the chain's tabs, and the tabs must not
    /// change when that happens.
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
    /// The two switches toggled at the foot of the model view. They are the
    /// view bar's own: the debug overlay's wireframe and the world's particles.
    /// `None` when the caller has neither (the inspector), and for the
    /// wireframe in a build without diagnostics, where the checkbox is drawn
    /// disabled.
    pub wireframe: Option<&'a mut bool>,
    pub particles: Option<&'a mut bool>,
    /// The lights on the open map, for the one subject whose rows have a place
    /// in the world (see [`crate::tools::lights`]). `None` when the caller has
    /// none, which is the case in the inspector.
    pub lights: Option<&'a mut crate::tools::lights::Lights>,
    /// The quest tool's state, which holds the item name cache and the
    /// reference picker an item column uses. `None` when there is no world
    /// database to name an item from, and in the inspector.
    pub quests: Option<&'a mut crate::tools::quests::Quests>,
    /// The time of day the world is showing, in half-minutes past midnight, so
    /// a band's strip can mark the viewport's current time on it.
    /// `vale_assets::tables::light::NOON` when the caller has no clock.
    pub hour: u32,
}

impl Workspace<'_> {
    /// The archive path of a row's picture, or `None` for a row with none.
    ///
    /// A spell's picture is its icon: `SpellIcon` maps an id to a path, and
    /// the spell's icon id is looked up there by `icon_by_id`. A `SpellIcon`
    /// row's picture is its own path. A skill line's is the icon on its
    /// spellbook tab, and an ability's is its spell's.
    fn icon_of(&mut self, record: usize) -> Option<String> {
        match self.browser.table.as_str() {
            "SkillLine" => {
                let icon_id = self.session.table("SkillLine")?.u32_at(record, 21)?;
                self.icon_by_id(icon_id)
            }
            "SkillLineAbility" => {
                let spell = self.session.table("SkillLineAbility")?.u32_at(record, 2)?;
                let at = self.browser.record_of(self.session, "Spell", spell)?;
                let icon_id = self
                    .session
                    .table("Spell")?
                    .u32_at(at, schema::SPELL_ICON_FIELD)?;
                self.icon_by_id(icon_id)
            }
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

    /// The archive path of an icon by its `SpellIcon` row id, also used for the
    /// icon field's own preview.
    fn icon_by_id(&self, icon_id: u32) -> Option<String> {
        spell_icon_path(self.session, self.assets, icon_id)
    }

    /// Draw a picture at `side`, or leave a gap of the same size when there is
    /// nothing to draw, so a list of rows does not shift as pictures load.
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
/// The cut is at `i32::MAX`: below it the signed and unsigned readings agree,
/// and above it no column in these tables holds a real value, since a row id
/// has at most five digits and a mask is drawn as hex.
fn signed(raw: u32) -> String {
    match raw > i32::MAX as u32 {
        true => format!("{}", raw as i32),
        false => format!("{raw}"),
    }
}

/// Append `.blp` to a table's texture path unless it already has it. The
/// tables store paths without an extension and the client appends it. See
/// `ui::framexml::decode_rgba`, which applies the same rule in another crate.
/// The archive path of a spell icon by its `SpellIcon` row id, or `None` for
/// id 0 and an id no row has. Shared with the reference picker's spell rows.
///
/// The session's own `SpellIcon` is read first, so an icon the project
/// re-points shows, and the client's parse is read when the session has not
/// opened the table.
pub(super) fn spell_icon_path(
    session: &EditSession,
    assets: &GameAssets,
    icon_id: u32,
) -> Option<String> {
    if icon_id == 0 {
        return None;
    }
    if let Some(path) = session
        .table("SpellIcon")
        .and_then(|icons| icons.row_of(icon_id))
        .and_then(|row| session.table("SpellIcon")?.string_at(row, 1))
    {
        return Some(with_blp(&path));
    }
    let path = assets
        .display_tables()
        .ok()?
        .spellbook()?
        .icon_path(icon_id)?;
    Some(with_blp(&path))
}

fn with_blp(path: &str) -> String {
    match path.to_ascii_lowercase().ends_with(".blp") {
        true => path.to_string(),
        false => format!("{path}.blp"),
    }
}

/// Which view of the open row is drawn.
///
/// This is a view rather than a second panel. `Spell.dbc` row 74 is Fireball
/// in both views; one shows its 173 columns and the other the sequence they
/// describe. A second panel would need a second row selection kept in step
/// with the first.
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
    // The Tables workspace with no table chosen: the list is the tables.
    if table_name.is_empty() {
        let all = ui.available_rect_before_wrap();
        ui.painter().rect_filled(all, 0.0, theme::SHELL);
        browser_panel(ui, |ui| table_list(ui, &mut work));
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Choose a table on the left.")
                    .size(14.0)
                    .color(theme::INK_DIM),
            );
        });
        stage.showing = None;
        stage.lab = false;
        return;
    }
    if work.session.table(&table_name).is_none() {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(format!("opening {table_name}.dbc…")).color(theme::INK_DIM));
        return;
    }

    // Both views paint their own background. The stage renders into an image,
    // so no view needs the world visible behind it. When only the fields view
    // painted a background, the world showed through the strip above the
    // panels at the top of the storyboard.
    let all = ui.available_rect_before_wrap();
    ui.painter().rect_filled(all, 0.0, theme::SHELL);

    // The list is a docked panel rather than part of a `horizontal` layout. A
    // `horizontal` layout gives its children a horizontal cursor, so a list
    // built inside one runs across the top of the window instead of down the
    // side: 22,360 rows wide. A panel takes its space from the `Ui` and gives
    // its contents a column, as the shell itself does.
    //
    // This draws the [`Surface::Middle`](crate::tools::Surface::Middle)
    // workspace only. A data subject whose rows have a place in the world is
    // drawn in the inspector instead (see [`light_inspector`]), so this
    // function has one layout, and the list keeps the width that 22,360 rows
    // need.
    browser_panel(ui, |ui| list(ui, &mut work));
    // Leave a gap between the list and the form. The panel's margin ends at its
    // edge, so without a gap `< back` and the section heads touch the list's
    // border. The gap is a child `Ui` inset by [`FORM_GAP`] rather than a
    // frame, because the storyboard and the lab dock their own panels into the
    // `Ui` they are given, and a frame's content rectangle is not settled until
    // its contents are laid out.
    let rect = ui.available_rect_before_wrap();
    let inset = egui::Rect::from_min_max(rect.min + egui::vec2(FORM_GAP, 0.0), rect.max);
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(inset).layout(*ui.layout()));
    form(&mut inner, &mut work, board, stage, lab);
}

/// The docked panel the list is drawn in, for the rows of a table and for
/// the Tables workspace's list of tables.
fn browser_panel(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Panel::left("data-browser")
        .default_size(BROWSER_WIDTH)
        .min_size(220.0)
        .max_size(520.0)
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(8.0))
        .show(ui, contents);
}

/// What one file of `DBFilesClient\` is to the editor and to the server, for
/// its row in the Tables workspace's list.
fn table_about(name: &str) -> String {
    let fields = match schema::for_table(name) {
        Some(_) => "named fields",
        None => "numbered fields",
    };
    let rows = crate::server::rows::MAPPED
        .iter()
        .find(|(dbc, _)| *dbc == name)
        .map(|(_, table)| *table);
    match (rows, vale_mangos::datadir::server_reads_dbc(name)) {
        (Some(table), _) => format!("{fields} · the server reads {table}"),
        (None, true) => format!("{fields} · the server reads the file"),
        (None, false) => format!("{fields} · not sent to the server"),
    }
}

/// The Tables workspace's list before a table is chosen: every file of
/// `DBFilesClient\` the archives list, searched by name.
///
/// A row says how the table is drawn (named fields where a schema describes
/// it, numbered fields where none does) and how this editor sends an edit to
/// the server: as rows of a table, as the copied file, or not at all. The
/// last is a statement about the editor: vmangos reads some of those tables
/// from SQL tables this editor does not write, `AreaTable` among them. The mark on the
/// right says the project carries an edited copy.
fn table_list(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    if work.browser.table_names.is_none() {
        let mut names: Vec<String> = work
            .assets
            .with_archive(|chain| Ok(chain.list_prefix("DBFilesClient\\")))
            .unwrap_or_default()
            .iter()
            .filter_map(|path| {
                let file = tables::basename(path);
                let stem = file.get(..file.len().checked_sub(4)?)?;
                file[stem.len()..]
                    .eq_ignore_ascii_case(".dbc")
                    .then(|| schema::table_name(stem).to_string())
            })
            .collect();
        names.sort_by_key(|name| name.to_ascii_lowercase());
        names.dedup();
        work.browser.table_names = Some(names);
    }
    let names = work.browser.table_names.clone().unwrap_or_default();

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Tables")
                .strong()
                .size(15.0)
                .color(theme::INK),
        );
        ui.label(theme::number(format!("{} files", names.len())));
    })
    .response
    .on_hover_text(
        "Every file of DBFilesClient\\ in the archives. A table with a schema opens as \
         named fields; any other opens as numbered fields.",
    );
    ui.add_space(4.0);
    let width = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(&mut work.browser.table_query)
            .hint_text("table name")
            .desired_width(width),
    );
    ui.add_space(4.0);
    let query = work.browser.table_query.trim().to_ascii_lowercase();
    let matches: Vec<&String> = names
        .iter()
        .filter(|name| query.is_empty() || name.to_ascii_lowercase().contains(&query))
        .collect();
    ui.label(
        egui::RichText::new(format!("{} shown", matches.len()))
            .small()
            .color(theme::INK_FAINT),
    );
    ui.add_space(4.0);

    let mut chosen: Option<String> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
            for at in range {
                let name = matches[at];
                let carried = work
                    .session
                    .project
                    .path_for(&vale_assets::tables::dbc::dbc_path(name))
                    .is_some_and(|disk| disk.exists());
                let mark = match (work.session.unsaved_tables.contains(name), carried) {
                    (true, _) => "unsaved",
                    (false, true) => "edited",
                    (false, false) => "",
                };
                let shape = theme::list_row(
                    ui,
                    theme::ListRow {
                        title: name,
                        sub: &table_about(name),
                        trailing: mark,
                        tint: theme::INK,
                        picture: false,
                    },
                    false,
                );
                if shape.response.clicked() {
                    chosen = Some(name.clone());
                }
            }
        });
    if let Some(name) = chosen {
        work.browser.look_at(&name);
    }
}

/// The left column: the workspace's parts and the chain's tabs, a search box,
/// and the rows that match.
fn list(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    // The parts of a workspace of several tools, which for this panel is
    // Items and Sets. The shell switches after the frame is drawn.
    if let Some(part) = super::rail::parts(ui, work.tool) {
        work.browser.switch_to = Some(part);
    }
    // The Tables workspace reaches every table from its list, so its way
    // to another table is back to that list.
    if work.tool == crate::tools::Tool::Tables
        && ui
            .add(egui::Link::new(
                egui::RichText::new("\u{2039} Tables").color(theme::ACCENT),
            ))
            .on_hover_text("Back to the list of tables.")
            .clicked()
    {
        work.browser.look_at(tables::ANY);
        work.browser.back.clear();
        return;
    }
    // The chain's tables as tabs, like the reference tool's sidebar. A spell,
    // its visual, its kits and its effects are four tables a person switches
    // between often, so they need a way in other than following a reference.
    // The tool decides which tables they are (see `Tool::tabs`), because there
    // are two chains. A table outside the chain, such as `SpellIcon` reached by
    // following a reference, highlights no tab, and pressing a tab still opens
    // that tab's table.
    let tabs = work.tool.tabs();
    let before: &'static str = tabs
        .iter()
        .find(|(_, table)| *table == work.browser.table)
        .map(|(_, table)| *table)
        .unwrap_or("");
    // A single tab is not drawn, and more than a row's worth are drawn in
    // rows: the spell workspace's chain, then its skill tables.
    let mut current = before;
    if tabs.len() > 1 {
        for row in tabs.chunks(tables::TAB_ROW) {
            ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
                theme::segmented(ui, &mut current, row, |a, b| a == b);
            });
        }
        ui.add_space(6.0);
    }
    if current != before {
        work.browser.look_at(current);
    }

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

    let pictured = matches!(
        table_name.as_str(),
        "Spell" | "SpellIcon" | "SkillLine" | "SkillLineAbility"
    );
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

/// `Undo`, `Redo`, `Save`, `Discard`: the four actions on the table as a file
/// rather than on one of its rows.
///
/// Undo and redo repeat the inspector's pair here, because egui takes
/// `Ctrl+Z` while a text box has keyboard focus, and in a workspace made of
/// text boxes one usually does. Discard reloads the table from the file and
/// removes its entries from the undo stack. `EditSession::discard_table`
/// explains why the entries are removed rather than kept.
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
/// [`theme::list_row`] draws the row, as it does for every list in the editor
/// (see its own note). This function adds only the picture, which here is a
/// spell icon from the thumbnail cache.
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

    // Only a spell has a second view, because only a spell names a chain. On a
    // table with one view the switch is not drawn, since it would do nothing.
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
        // The preview pane has a minimum width, `STAGE_FLOOR`, and the chain
        // gets the rest. A fixed 480-point chain left the preview a sliver on a
        // 1280-wide window.
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

    // An effect always has the preview pane, laid out like the storyboard: the
    // fields on the left and the stage on the right with the model alone. Once
    // *Position on character…* is pressed, the lab's card is added above the
    // fields and the mannequin under the model. See `crate::lab`.
    let id = work
        .session
        .table(&table_name)
        .and_then(|table| table.u32_at(record, 0))
        .unwrap_or(0);
    if table_name == "SpellVisualEffectName" {
        if !lab.is_open_on(id) {
            super::lab::open_alone(lab, stage, work.browser, work.session, record);
        } else if let Some(table) = work.session.table(&table_name) {
            // Pass the row's current `Model` and `Scale` to
            // `Lab::follow_row`, which makes an edit to either show in the
            // pane beside the form.
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
        // The lab's card needs width and an effect's five fields do not, so
        // the model view gives the extra room to the pane. The two modes use
        // different panel ids, so each keeps the width it was dragged to.
        let on_character = lab.on_character(id);
        let form_width = match on_character {
            true => (CHAIN_WIDTH + 140.0).min((here - STAGE_FLOOR).max(320.0)),
            false => 360.0_f32.min((here - STAGE_FLOOR).max(320.0)),
        };
        // The contents are limited to the panel's width from the last frame.
        // The panel's inner `Ui` reports more width than the panel shows, which
        // let a note wrap a hundred points past its edge. Using the width the
        // panel actually took lags a resize by one frame.
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
    // Outside the storyboard and the lab nothing is previewed. Clearing these
    // removes the units from the stage and returns the camera.
    stage.showing = None;
    stage.lab = false;

    egui::ScrollArea::vertical()
        .id_salt("data-form")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Rows tall enough to click and spaced enough to read. This is set
            // on the form's own `Ui` rather than in the theme, because this is
            // the one panel in the editor that is a page of fields rather than
            // a few controls.
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
            // A spell's skill lines and teaching spells are rows of other
            // tables that exist only for the spell, so they are edited on
            // the spell's form, above its own columns.
            if schema.table == "Spell" {
                learning(ui, work, record);
            }
            for section in schema.sections {
                // A light's sphere is drawn in the world's units. Its five
                // columns are in 1/36 of a yard measured from the corner of
                // the map (for example `557760`, `0`, `966720`), which cannot
                // be read or typed sensibly. `sphere_block` shows the same
                // five fields converted.
                if schema.table == "Light" && section.name == "Sphere" {
                    sphere_block(ui, work, record);
                    continue;
                }
                section_block(ui, work, record, schema, section);
            }
            // …and so are the rows that say who has a skill line.
            if schema.table == "SkillLine" {
                skill_line_members(ui, work, record);
            }
        }
        // A table with no schema still opens, as numbered fields with guessed
        // types. This is what `vale dbc <Table>` prints, and it is the
        // starting point for writing that table's schema.
        None => unschemad(ui, work, record),
    }
    ui.add_space(24.0);
}

/// A spell's Learning section: the skill lines it is in and the spells that
/// teach it, which are rows of other tables that exist only for this spell.
///
/// Neither is required. Most spells are in no skill line and have no teaching
/// spell, and the section then says so in one line each and offers to make
/// one. A row made here is an ordinary row of its table: the Abilities tab
/// lists it, and `open` goes to it.
fn learning(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    use vale_assets::tables::spellbook::spell_fields;
    let Some((spell, teaches)) = work.session.table("Spell").and_then(|spells| {
        let learns = spells.u32_at(record, spell_fields::EFFECT)? == vale_mangos::trainer::LEARN_SPELL;
        let taught = spells.u32_at(record, spell_fields::EFFECT_TRIGGER_SPELL)?;
        Some((spells.u32_at(record, 0)?, learns.then_some(taught)))
    }) else {
        return;
    };
    egui::CollapsingHeader::new(egui::RichText::new("Learning").size(14.0).strong())
        .default_open(true)
        .show(ui, |ui| {
            // A teaching spell says what it teaches, which is the one thing
            // about it that matters to a person.
            if let Some(taught) = teaches.filter(|taught| *taught != 0) {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("This is a teaching spell for").color(theme::INK_DIM));
                    let name = work
                        .browser
                        .describe_id(work.session, "Spell", taught)
                        .map(|label| label.line())
                        .unwrap_or_else(|| format!("spell {taught}"));
                    if ui
                        .add(egui::Link::new(egui::RichText::new(name).color(theme::ACCENT)))
                        .on_hover_text(format!("open Spell {taught}"))
                        .clicked()
                    {
                        follow_reference(work, "Spell", taught);
                    }
                });
                ui.add_space(4.0);
            }
            skill_lines_of(ui, work, spell);
            ui.add_space(8.0);
            taught_by(ui, work, spell);
        });
}

/// One row of another table drawn on the open row's form: a heading line
/// with what the row is, its id, a link to the row itself and a button that
/// removes it, then the fields named. Answers whether remove was pressed.
fn owned_row(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    schema: &'static Schema,
    row: usize,
    title: &str,
    fields: &[usize],
    more: &[usize],
) -> bool {
    let table = schema.table;
    let id = work
        .session
        .table(table)
        .and_then(|open| open.u32_at(row, 0))
        .unwrap_or(0);
    let mut remove = false;
    egui::Frame::new()
        .fill(theme::PANEL)
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).color(theme::INK));
                ui.label(theme::number(format!("#{id}")).color(theme::INK_FAINT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    remove = ui
                        .small_button("remove")
                        .on_hover_text(format!("Remove this {table} row. One undo entry."))
                        .clicked();
                    if ui
                        .small_button("open")
                        .on_hover_text(format!("Open {table} {id} with every field."))
                        .clicked()
                    {
                        follow_reference(work, table, id);
                    }
                });
            });
            for column in fields.iter().filter_map(|&field| schema.column(field)) {
                field_in(ui, work, table, row, column);
            }
            if !more.is_empty() {
                egui::CollapsingHeader::new(
                    egui::RichText::new("Skill values").size(12.0).color(theme::INK_DIM),
                )
                .id_salt((table, id, "more"))
                .default_open(false)
                .show(ui, |ui| {
                    for column in more.iter().filter_map(|&field| schema.column(field)) {
                        field_in(ui, work, table, row, column);
                    }
                });
            }
        });
    ui.add_space(2.0);
    remove
}

/// The skill lines a spell is in, as the `SkillLineAbility` rows that name
/// it, each with its skill line, its classes and races, whether learning the
/// line gives the spell, and the spell that supersedes it.
fn skill_lines_of(ui: &mut egui::Ui, work: &mut Workspace<'_>, spell: u32) {
    use tables::ability;
    ui.label(egui::RichText::new("Skill lines").color(theme::INK_DIM));
    if !work.session.open_table(work.assets, ability::TABLE) {
        theme::note(ui, "SkillLineAbility did not open");
        return;
    }
    let rows = tables::abilities_of(work.browser, work.session, spell);
    if rows.is_empty() {
        theme::note(
            ui,
            "In no skill line, as most spells are. The spellbook files such a spell under \
             General, and the client leaves a class trainer's service for it out of the \
             training window.",
        );
    }
    let mut remove: Option<usize> = None;
    for &row in &rows {
        let skill = work
            .session
            .table(ability::TABLE)
            .and_then(|open| open.u32_at(row, ability::SKILL))
            .unwrap_or(0);
        let title = work
            .browser
            .describe_id(work.session, "SkillLine", skill)
            .map(|label| label.title)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "no skill line chosen".to_string());
        if owned_row(
            ui,
            work,
            &schema::SKILL_LINE_ABILITY,
            row,
            &title,
            &[
                ability::SKILL,
                ability::CLASSES,
                ability::RACES,
                ability::LEARN_ON_GET_SKILL,
                ability::SUPERSEDED_BY,
            ],
            &[
                ability::REQ_SKILL_VALUE,
                ability::MIN_VALUE,
                ability::MAX_VALUE,
                ability::REQ_TRAIN_POINTS,
            ],
        ) {
            remove = Some(row);
        }
    }
    if ui
        .button("+ Add to a skill line")
        .on_hover_text(
            "A new SkillLineAbility row naming this spell, and the picker for its skill \
             line. One undo entry. A mask left at zero is every class or every race.",
        )
        .clicked()
    {
        if let Some(at) = tables::add_ability(work.session, spell) {
            work.browser.modal = Some(Modal::Pick {
                table: ability::TABLE.to_string(),
                record: at,
                field: ability::SKILL,
                points_at: "SkillLine",
            });
            work.browser.pick_query.clear();
            work.browser.pick_focus = true;
        }
    }
    if let Some(row) = remove {
        tables::delete_row(work.session, ability::TABLE, row);
    }
}

/// The spells that teach a spell, and a button that makes one.
///
/// A trainer's list names a teaching spell and not the spell a player ends
/// up with, so a spell a trainer is to teach needs one. A spell learned any
/// other way needs none.
fn taught_by(ui: &mut egui::Ui, work: &mut Workspace<'_>, spell: u32) {
    ui.label(egui::RichText::new("Taught by").color(theme::INK_DIM));
    let teachers = tables::teachers_of(work.browser, work.session, spell);
    if teachers.is_empty() {
        theme::note(
            ui,
            "No teaching spell, as most spells have none. A trainer's list names a teaching \
             spell: one whose first effect is Learn Spell and which names this spell.",
        );
    }
    let mut follow: Option<u32> = None;
    for &(row, instant) in &teachers {
        let label = work.browser.describe(work.session, "Spell", row);
        ui.horizontal(|ui| {
            if ui
                .add(egui::Link::new(
                    egui::RichText::new(label.line()).color(theme::ACCENT),
                ))
                .on_hover_text(format!("open Spell {}", label.id))
                .clicked()
            {
                follow = Some(label.id);
            }
            // The rank, which is what tells one teaching spell's row from the
            // next rank's.
            if !label.sub.is_empty() {
                ui.label(egui::RichText::new(&label.sub).small().color(theme::INK_DIM));
            }
            ui.label(
                egui::RichText::new(match instant {
                    true => "instant",
                    false => "has a cast time",
                })
                .small()
                .color(theme::INK_DIM),
            );
        });
    }
    if let Some(id) = follow {
        follow_reference(work, "Spell", id);
    }
    // A trainer's list wants an instant one. A spell with only a teaching
    // spell that has a cast time, which is the kind a book casts, is offered
    // an instant one beside it.
    if !teachers.iter().any(|&(_, instant)| instant) {
        let label = match teachers.is_empty() {
            true => "+ Create a teaching spell",
            false => "+ Create an instant teaching spell",
        };
        if ui
            .button(label)
            .on_hover_text(
                "A new spell under the next id that teaches this one: Learn Spell as its \
                 first effect, an instant cast, and this spell's name, rank and icon. One \
                 undo entry. The Trainer window can then add it to a trainer's list.",
            )
            .clicked()
        {
            match tables::add_teaching_spell(work.session, spell) {
                Some(id) => work.session.status = format!("spell {id} teaches spell {spell}"),
                None => work.session.status = "the teaching spell was not made".to_string(),
            }
        }
    }
}

/// A skill line's two integrated blocks: who has it, as the
/// `SkillRaceClassInfo` rows that name the line, and how many spells are in
/// it, with a link to them.
fn skill_line_members(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    use tables::{ability, race_class};
    let Some(skill) = work
        .session
        .table("SkillLine")
        .and_then(|lines| lines.u32_at(record, 0))
    else {
        return;
    };
    egui::CollapsingHeader::new(egui::RichText::new("Who has it").size(14.0).strong())
        .default_open(true)
        .show(ui, |ui| {
            if !work.session.open_table(work.assets, race_class::TABLE) {
                theme::note(ui, "SkillRaceClassInfo did not open");
                return;
            }
            let rows = tables::race_class_rows_of(work.browser, work.session, skill);
            if rows.is_empty() {
                theme::note(
                    ui,
                    "No race or class has this line: no SkillRaceClassInfo row names it. \
                     Its spells then go on the spellbook's General tab.",
                );
            }
            let mut remove: Option<usize> = None;
            for &row in &rows {
                let title = work.browser.describe(work.session, race_class::TABLE, row).sub;
                if owned_row(
                    ui,
                    work,
                    &schema::SKILL_RACE_CLASS_INFO,
                    row,
                    &title,
                    &[
                        race_class::CLASSES,
                        race_class::RACES,
                        race_class::FLAGS,
                        race_class::MIN_LEVEL,
                        race_class::SKILL_TIER,
                    ],
                    &[],
                ) {
                    remove = Some(row);
                }
            }
            if ui
                .button("+ Give it to races and classes")
                .on_hover_text(
                    "A new SkillRaceClassInfo row naming this line, for every race and every \
                     class, with no flag set. One undo entry.",
                )
                .clicked()
            {
                tables::add_race_class_row(work.session, skill);
            }
            if let Some(row) = remove {
                tables::delete_row(work.session, race_class::TABLE, row);
            }
        });

    egui::CollapsingHeader::new(egui::RichText::new("Spells in it").size(14.0).strong())
        .default_open(true)
        .show(ui, |ui| {
            if !work.session.open_table(work.assets, ability::TABLE) {
                theme::note(ui, "SkillLineAbility did not open");
                return;
            }
            let count = tables::abilities_in(work.browser, work.session, skill).len();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(match count {
                        1 => "1 ability".to_string(),
                        n => format!("{n} abilities"),
                    })
                    .color(theme::INK),
                );
                if count > 0
                    && ui
                        .add(egui::Link::new(
                            egui::RichText::new("list them").color(theme::ACCENT),
                        ))
                        .on_hover_text(
                            "Open the Abilities tab searched by this line's name. A spell is \
                             added to the line from the spell's own form.",
                        )
                        .clicked()
                {
                    let name = work
                        .session
                        .table("SkillLine")
                        .and_then(|lines| lines.string_at(record, 3))
                        .unwrap_or_default();
                    work.browser.back.push(("SkillLine".to_string(), record));
                    work.browser.look_at(ability::TABLE);
                    work.browser.query = name;
                }
            });
        });
}

/// A light's whole form, drawn in the inspector: the light picked in the world
/// and the numbers behind it.
///
/// This is the lights tool's only panel. A light is chosen by picking it in
/// the viewport rather than from a list, and the shell's rule is that the
/// inspector shows the selection, so the form is drawn here. The list of all
/// 374 lights is a dialog behind *Browse…*, for finding one that is not on
/// screen.
///
/// Its contents, in the order they are used: which light this is, a button to
/// fly the camera to it, its position and radius in yards rather than the
/// file's 1/36 of a yard, and then the five `LightParams` rows it uses under
/// the five conditions, which hold the colours.
pub fn light_inspector(ui: &mut egui::Ui, mut work: Workspace<'_>) {
    if !work.session.open_table(work.assets, "Light") {
        theme::note(ui, "opening Light.dbc…");
        return;
    }
    browse_button(ui, &mut work);
    ui.add_space(6.0);

    // Draw whatever table the browser has open, which is not always `Light`.
    // Pressing a `ParamsClear` link opens `LightParams`, and the bands belong
    // to that row. If this panel always drew the light's own form, those five
    // links would have no visible effect.
    //
    // The browser is not re-pointed here. Re-pointing it at the selected light
    // on every frame undid a followed reference on the frame after the press.
    // The two places that change the selection, the pick in the viewport and
    // the browse dialog, each point the browser at the row themselves, which
    // is the only time it should move. `tables::open_tables` has a note on the
    // same rule for the rail's table.
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

    // A `< back` button to return from a followed reference. The middle
    // workspace has one in its heading; without this one the inspector had no
    // way back after following a reference to another table.
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
                // conditions, which hold the colours. The conditions are drawn
                // by the ordinary `section_block`, so the picker, the resolved
                // name and the link behave as they do elsewhere.
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
                // A row reached by following a reference from the light, such
                // as a `LightParams` or `LightSkybox` row, gets the ordinary
                // form for its table.
                _ => {
                    fields(ui, &mut work, record, schema);
                    // A `LightParams` row also shows its bands, which hold
                    // every colour the world is drawn in. No column points at
                    // the two tables that hold the bands, so this is the only
                    // way to reach them.
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

/// A button that opens the list of all lights as a dialog, for finding a light
/// that is not on screen.
///
/// The tool is built around picking a light in the viewport, which cannot
/// reach a light that is off screen. The list is a dialog behind a button
/// rather than a permanent panel, because a permanent panel took space the
/// form needed.
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

/// A light's position and falloff in yards: the `Sphere` section, drawn in the
/// world's units instead of the file's.
///
/// The unit changes and the radius names do not: `FalloffStart` and
/// `FalloffEnd` keep the columns' names. The three coordinates are labelled as
/// the world's `x`, `y` and `z`, because after conversion they no longer match
/// the columns they come from. The axes swap, so the field holding the world's
/// x is `InternalZ` in the file. Each field's tooltip names the column it
/// writes and its unit.
///
/// The five columns are
/// [`light_field`](vale_assets::tables::light::light_field)'s: three
/// coordinates in the internal representation used by every placement in the
/// game, scaled by 36, and two radii in the same unit. All five are converted
/// here rather than shown raw, and typing into one converts back.
/// [`vale_assets::world::adt::placement_from_world`] does the conversion back;
/// loading the game's own tiles never uses that direction, so it is called
/// here rather than reimplemented.
///
/// The raw columns are still available, folded away underneath, because they
/// are what is written to the file and a person checking a diff needs them.
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

    // Write only when a value changed, and write all five under one gesture, so
    // a drag on the x box is one entry on the undo stack rather than one per
    // pixel.
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

/// The strip under a `Light` row's heading: where the light is, and a button
/// to fly the camera there.
///
/// The row's three coordinates are stored in a form that cannot be read at a
/// glance (`612096, 0, 998400`, in 1/36 of a yard from the corner of the map),
/// so the form alone does not show where the light is. This strip gives the
/// position in world coordinates, keeps the viewport's ring on the open row,
/// and flies the camera there.
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
    // Opening a row selects its light. Picking a light already opens its row;
    // without this direction, a row reached from the list or by following a
    // reference left the viewport highlighting the previous light, which is
    // the ring a person would then drag.
    if lights.selected != Some(id) {
        lights.selected = Some(id);
    }
    let Some(&mark) = lights.marks.iter().find(|mark| mark.id == id) else {
        // A light on another map. The row can still be edited, because a
        // `Light` row names its own map, but there is nothing to fly to here.
        theme::note(
            ui,
            "This light is on another map. Open that map to see it in the world.",
        );
        return;
    };

    ui.horizontal(|ui| {
        if mark.everywhere {
            // The default light applies everywhere and its coordinates are not
            // read, so there is no position to fly to.
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
                // Actions on this row other than editing a field.
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
    // One line listing the rows that point here; the full list is in the
    // inspector. Other spells point at a spell only through trigger columns,
    // which is rarely why a spell is opened, so the line is shown only for the
    // chain tables.
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
        // Grouped by table, with spells first because they are usually what a
        // person is looking for.
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
                // A name repeated nine times is one spell's nine ranks; show it
                // once.
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

/// One field of the open table: its name, and the widget its kind asks for.
fn field_row(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize, column: &Column) {
    let table_name = work.browser.table.clone();
    field_in(ui, work, &table_name, record, column);
}

/// One field of one record of any open table.
///
/// The table is named because the record is not always a row of the table
/// the browser is on: a spell's form draws fields of the `SkillLineAbility`
/// rows that name the spell. A text column is edited only on its own
/// table's form, since the text buffers are the open row's.
fn field_in(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table_name: &str,
    record: usize,
    column: &Column,
) {
    let Some(raw) = work
        .session
        .table(table_name)
        .and_then(|table| table.u32_at(record, column.field))
    else {
        return;
    };

    ui.horizontal(|ui| {
        // The field index and the schema's note go in a tooltip rather than
        // beside each label, where they would add sixty `[117]`-style suffixes
        // down the page.
        let mut tip = format!("field {}", column.field);
        if !column.about.is_empty() {
            tip.push_str("\n\n");
            tip.push_str(column.about);
        }
        // The label is at most [`LABEL`] wide and at most 45% of the available
        // width. [`LABEL`] suits the middle workspace, where the form has 700
        // points. The same form is also drawn in the inspector, where a fixed
        // 210 clipped the resolved name of every reference at the window's
        // edge.
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
                    write(work, table_name, record, column, value);
                }
            }
            // `Unused` is drawn signed for the same reason as `reference`. It is
            // the column most likely to hold the `-1` this table writes for
            // "nothing": `SpellVisualKit`'s unused slot holds one on almost
            // every row. An `Int` stays unsigned, because those are counts and
            // durations, and so do the locale flags, which are a mask.
            Kind::Signed | Kind::Unused => {
                let mut value = raw as i32;
                if number(ui, &mut value).changed() {
                    write(work, table_name, record, column, value as u32);
                }
            }
            Kind::Float => {
                let mut value = f32::from_bits(raw);
                let response = ui.add_sized(
                    egui::vec2(VALUE, 22.0),
                    egui::DragValue::new(&mut value).speed(0.01),
                );
                if response.changed() {
                    write(work, table_name, record, column, value.to_bits());
                }
            }
            Kind::Flags(bits) => {
                let mut value = raw;
                if number(ui, &mut value).changed() {
                    write(work, table_name, record, column, value);
                }
                // A button that opens the list of named bits, when the mask
                // has one. Everything that reads a mask reads it bit by bit,
                // so the bits are the value and the number is only their
                // encoding (see `vale_assets::tables::spellbits`). Without the
                // list, setting "castable while dead" required knowing
                // 0x00800000.
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
                        table: table_name.to_string(),
                        record,
                        field: column.field,
                        column: column.name,
                        bits,
                    });
                }
                ui.label(theme::number(format!("0x{raw:08X}")));
                // The names of the set bits, shown on the row so they can be
                // read without opening the dialog. The dialog is for changing
                // them.
                let set = schema::named_bits(raw, bits);
                if !set.is_empty() {
                    ui.label(
                        egui::RichText::new(set.join(" · "))
                            .small()
                            .color(theme::INK_DIM),
                    );
                }
                // Set bits with no name are shown rather than hidden. Such a
                // bit is either missing from the list or was typed by hand,
                // and the user needs to see both cases.
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
                    write(work, table_name, record, column, u32::from(on));
                }
                // A gate with a value other than 0 or 1 shows the value.
                if raw > 1 {
                    ui.label(theme::number(format!("{raw}")).color(theme::WARN));
                }
            }
            Kind::Enum(names) => {
                // A value the list does not name is shown as a number, as
                // the game does. It is shown signed for the same reason as
                // `reference`: these columns are `int32`, and their "nothing
                // here" value is `-1`, which reads as `4294967295` as a `u32`.
                let shown = names
                    .iter()
                    .find(|&&(value, _)| value == raw)
                    .map(|&(_, name)| name.to_string())
                    .unwrap_or_else(|| signed(raw));
                let mut chosen = raw;
                egui::ComboBox::from_id_salt((table_name, record, column.field))
                    .selected_text(egui::RichText::new(shown).size(13.5))
                    .width(TEXT * 0.6)
                    .show_ui(ui, |ui| {
                        for &(value, name) in names {
                            ui.selectable_value(&mut chosen, value, format!("{value}  {name}"));
                        }
                    });
                if chosen != raw {
                    write(work, table_name, record, column, chosen);
                }
            }
            Kind::Colour => {
                let mut rgb = [
                    ((raw >> 16) & 0xff) as u8,
                    ((raw >> 8) & 0xff) as u8,
                    (raw & 0xff) as u8,
                ];
                // The top byte is kept unchanged. It is zero on all but one
                // shipped `LightIntBand` row, and rebuilding the word from the
                // three channels would clear the `0xff` that row carries. See
                // `Kind::Colour`.
                let keep = raw & 0xff00_0000;
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    let packed =
                        (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
                    write(work, table_name, record, column, keep | packed);
                }
                ui.label(
                    theme::number(format!("{} {} {}", rgb[0], rgb[1], rgb[2]))
                        .color(theme::INK_DIM),
                );
                // The packed word as hex, because the channel order is the
                // part of this column a reader is most likely to doubt.
                ui.label(theme::number(format!("0x{raw:06X}")).color(theme::INK_DIM));
            }
            Kind::Reference(points_at) => {
                reference(ui, work, table_name, record, column, points_at, raw)
            }
            Kind::Item => item_cell(ui, work, table_name, record, column, raw),
            // A text column of a row drawn on another table's form is shown
            // and not edited: the buffers hold the open row's text.
            Kind::Text | Kind::Locale(_) if table_name != work.browser.table => {
                let text = work
                    .session
                    .table(table_name)
                    .and_then(|table| table.string_at(record, column.field))
                    .unwrap_or_default();
                ui.label(egui::RichText::new(text).color(theme::INK));
            }
            Kind::Text | Kind::Locale(_) => {
                let wide = column.name.contains("Description");
                let text = work
                    .browser
                    .buffer(work.session, record, column.field)
                    .clone();
                let mut editing = text.clone();
                // Limited to the room left on the row. Beside the lab's pane
                // the form is narrow, and a fixed width ran past its edge.
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
                // Written when the text box loses focus, not on every
                // keystroke. The string block is append-only, so a write per
                // character would leave an unused copy of every prefix of the
                // word in the file. See `vale_edit::dbc`.
                if response.lost_focus() {
                    let now_in_file = work
                        .session
                        .table(table_name)
                        .and_then(|table| table.string_at(record, column.field))
                        .unwrap_or_default();
                    if editing != now_in_file {
                        tables::set_text(
                            work.session,
                            table_name,
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
                        .table(table_name)
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

/// An item column: the entry, a picker over `item_template`, and the item's
/// name as a link to the item workspace.
///
/// The name and the picker are the item form's, read from the world
/// database through the quest tool's cache. Without a database the entry is
/// drawn alone, since nothing in the archives names an item.
fn item_cell(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table_name: &str,
    record: usize,
    column: &Column,
    raw: u32,
) {
    use crate::tools::quests::{PickFor, Picker, SetFollow, Target};
    let mut value = raw;
    if number(ui, &mut value).changed() {
        write(work, table_name, record, column, value);
    }
    let table = table_name.to_string();
    // The set this row is, when the column is one of a set's items.
    let set = (table_name == "ItemSet")
        .then(|| work.session.table(table_name)?.u32_at(record, 0))
        .flatten();
    let Some(quests) = work.quests.as_deref_mut() else {
        ui.label(egui::RichText::new("item").small().color(theme::INK_FAINT))
            .on_hover_text(
                "An item_template entry. There is no world database to read its name from.",
            );
        return;
    };
    if ui
        .add(
            egui::Button::new(egui::RichText::new("…").size(13.0)).min_size(egui::vec2(24.0, 22.0)),
        )
        .on_hover_text("choose an item by name")
        .clicked()
    {
        quests.picker = Some(Picker::new(
            Target::Item,
            PickFor::TableField {
                table,
                record,
                field: column.field,
                column: column.name,
            },
        ));
    }
    if raw == 0 {
        ui.label(egui::RichText::new("none").color(theme::INK_FAINT));
        return;
    }
    super::reference::name(
        ui,
        &mut super::reference::Resolver {
            session: work.session,
            assets: work.assets,
            quests,
        },
        vale_mangos::item::TEMPLATE,
        raw,
    );
    // The other half of the membership: the item's own `set_id`, which is
    // what the server counts set pieces by. An item picked through the
    // dialog has it written for it; one typed as a number, or one the
    // shipped data disagrees about, is marked here with a button.
    let Some(set) = set else {
        return;
    };
    let Some(found) = quests.item(raw, &work.session.server_edits) else {
        return;
    };
    let pending = quests.set_follows.iter().any(|follow| follow.joined == raw);
    if found.set_id != set && !pending {
        ui.label(
            egui::RichText::new(match found.set_id {
                0 => "its set_id is 0".to_string(),
                other => format!("its set_id is {other}"),
            })
            .small()
            .color(theme::WARN),
        )
        .on_hover_text(
            "The set lists this item, and the item's own row does not name the set. The \
             server counts set pieces by item_template.set_id, so the item does not count \
             towards the bonuses until it does.",
        );
        if ui
            .small_button("join")
            .on_hover_text(format!("Write set_id {set} on this item's row. One undo entry."))
            .clicked()
        {
            quests.set_follows.push(SetFollow {
                set,
                left: 0,
                joined: raw,
            });
        }
    }
}

/// A reference column: the id, what it resolves to, and the controls to pick
/// or follow it.
///
/// See the module comment. The id stays editable for a person who knows it;
/// the picker is for one who does not.
fn reference(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table_name: &str,
    record: usize,
    column: &Column,
    points_at: &'static str,
    raw: u32,
) {
    // Drawn signed, because the column is signed. A reference holds a row id,
    // which is positive, or one of the table's two values for "nothing": `0`
    // and `-1`. Drawn as the `u32` it is read as, `-1` appears as
    // `4294967295` in the id box. The kits hold many of these, because
    // `blank_defaults` and the shipped rows both write `-1`. The label beside
    // the box says "none" for it, and the signed box now agrees.
    let mut value = raw as i32;
    if number(ui, &mut value).changed() {
        write(work, table_name, record, column, value as u32);
    }
    // The picker, which searches the target table by what its rows resolve
    // to. It is a dialog rather than a popup; see the module comment.
    if ui
        .add(
            egui::Button::new(egui::RichText::new("…").size(13.0)).min_size(egui::vec2(24.0, 22.0)),
        )
        .on_hover_text(format!("choose a {points_at} row by name"))
        .clicked()
    {
        work.browser.modal = Some(Modal::Pick {
            table: table_name.to_string(),
            record,
            field: column.field,
            points_at,
        });
        work.browser.pick_query.clear();
        work.browser.pick_focus = true;
    }
    let chain_table = matches!(
        points_at,
        "SpellVisual" | "SpellVisualKit" | "SpellVisualEffectName"
    );

    // Whether `0` means "nothing" depends on the target table. Every table of
    // the spell chain numbers from 1, so a `0` there is one of the two
    // "nothing" values. `Map.dbc` numbers from 0 and map 0 is Azeroth, so
    // treating `0` as nothing showed `none` for `Light.Map` on 232 of 374
    // lights, the most common value in the column. The rule used here: if the
    // target table has a row 0, `0` is a reference to it.
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
                tables::link_new(work.session, table_name, record, column.field, points_at)
            {
                work.browser.forget_buffers();
                follow_reference(work, points_at, id);
            }
        }
        return;
    }

    // What the target row is, in words, with a picture where the table has one.
    if points_at == "SpellIcon" {
        let icon = work.icon_by_id(raw);
        work.icon(ui, icon, ICON);
    }
    if points_at == "SpellVisualEffectName" {
        presence(ui, work.model_present(raw));
    }
    // The target table may not be open yet, because the chain's tables open
    // one per frame. Until it opens, only the id is shown.
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
    // Copying a shared row so that only this row uses it. A kit or an effect
    // is copied and this field pointed at the copy; a visual is copied with
    // its whole chain.
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
            tables::unshare(work.session, table_name, record, column.field, points_at)
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
            tables::clone_chain(work.session, raw, Some((table_name, record, column.field)))
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
/// Both are paged rather than truncated. Cutting a picker at two hundred hits
/// and the model browser at three hundred left most of the twelve thousand
/// icons or six thousand models unreachable. A page is turned with the arrows,
/// `PageUp` and `PageDown`, and a search narrows the list. The page returns to
/// the first whenever the hits are rebuilt.
fn modals(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let Some(modal) = work.browser.modal.clone() else {
        return;
    };
    let table_name = work.browser.table.clone();
    let mut close = false;
    match modal {
        Modal::Pick {
            table,
            record,
            field,
            points_at,
        } => {
            if !work.session.open_table(work.assets, points_at) {
                work.browser.modal = None;
                return;
            }
            let column_name = schema::for_table(&table)
                .and_then(|schema| schema.column(field))
                .map(|column| column.name)
                .unwrap_or("this field");
            // An icon is chosen by its picture, so the icon table is shown as a
            // grid of pictures at the file's own size rather than as rows with
            // a thumbnail beside a file name.
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
                    &table,
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
                // A light chosen here is also selected in the world, and the
                // camera flies to it. This list is opened to find a light that
                // is not on screen, so the view has to move to show it.
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
            table,
            record,
            field,
            column,
            bits,
        } => {
            // The mask as a list of checkboxes. Every bit gets a row with its
            // name, its note where vmangos wrote one, and its mask, and the
            // value is rebuilt from the checkboxes rather than typed.
            let raw = work
                .session
                .table(&table)
                .and_then(|open| open.u32_at(record, field))
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
                    "The names follow vmangos: 1.12 ships no table that names a mask's bits.",
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
                    // A button clears every bit at once, rather than up to
                    // thirty-two clicks; it is the usual way to empty a mask.
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
                tables::set_field(
                    work.session,
                    &table,
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
            // The dialog opens with the field's current model previewed.
            // Otherwise it opened with no preview pane and *Use* disabled,
            // reading "nothing chosen", even though the row already named a
            // model. The preview is set here rather than where the dialog is
            // opened, because there are two such places: the form's `browse…`
            // and `--browse`.
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
            // A lighter backdrop than a picker's, because the stage behind the
            // dialog is part of it: pressing a row previews that model on the
            // stage, and a dark backdrop would hide the preview.
            let response = egui::Modal::new(egui::Id::new("data-models"))
                .backdrop_color(egui::Color32::from_black_alpha(60))
                .show(ui.ctx(), |ui| {
                    ui.set_width(PICKER_WIDTH + 200.0);
                    ui.label(egui::RichText::new("Model").strong().size(14.0));
                    ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
                        // The archives' folders, then the person's two lists,
                        // starred and recent; see `crate::favourites`.
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
                        // Shorter than the previous 380, because the preview
                        // pane sits under the list: at 380 the two together
                        // pushed the dialog's buttons below the bottom of a
                        // 720-tall window.
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
                    // The previewed model, under the list. The stage behind
                    // the dialog also shows it, which is why the backdrop is
                    // lighter; this pane shows it without looking past the
                    // dialog. It is the same pane the placement pickers use.
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
        egui::FontId::proportional(theme::SMALL),
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
/// to the pages that exist and returns the range of the list on that page.
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
/// an empty range for an empty list. There is always at least one page, so
/// every page number is valid.
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

/// Open the table a reference names, at the row it names.
///
/// The table is opened here, not only looked up. `SpellVisual.dbc` is not
/// open until something asks for it, and the first thing to ask for it is the
/// press on the link. When this function only looked the table up, the button
/// did nothing.
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
///
/// Each field is four bytes and is drawn as the number they hold. Where the
/// column holds the offsets of strings (`tables::text_fields`), a text box
/// edits the string; where the bytes read as a float of ordinary size, a
/// second box edits the float. Both are readings of the same four bytes, and the number
/// box always writes them as they are, so a field that is neither can still
/// be set.
fn unschemad(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    let table_name = work.browser.table.clone();
    let Some(table) = work.session.table(&table_name) else {
        return;
    };
    let fields = table.field_count();
    theme::note(
        ui,
        "No schema for this table: the columns are numbered. A text box is drawn where \
         the column holds the offsets of strings, and a float box where the four bytes \
         read as a float. Both are guesses about the column. See \
         vale_assets::tables::schema.",
    );
    for field in 0..fields {
        let Some(raw) = work
            .session
            .table(&table_name)
            .and_then(|table| table.u32_at(record, field))
        else {
            continue;
        };
        // Whether the column holds strings is asked of the column, since a
        // small number can be the offset of a string on one row.
        let is_text = work
            .browser
            .text_fields(work.session, &table_name)
            .get(field)
            .copied()
            .unwrap_or(false);
        ui.horizontal(|ui| {
            ui.add_sized(
                egui::vec2(70.0, 22.0),
                egui::Label::new(theme::number(format!("[{field}]"))),
            );
            // Field 0 is the row's id, which only a new row mints.
            if field == 0 {
                ui.label(theme::number(format!("{raw}")));
                ui.label(egui::RichText::new("id").small().color(theme::INK_FAINT));
                return;
            }
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
            if is_text {
                loose_text(ui, work, record, field);
                return;
            }
            let mut float = f32::from_bits(raw);
            if raw != 0 && float.is_finite() && float.abs() > 1.0e-4 && float.abs() < 1.0e6 {
                let response = ui
                    .add_sized(
                        egui::vec2(VALUE, 22.0),
                        egui::DragValue::new(&mut float).speed(0.01),
                    )
                    .on_hover_text("The same four bytes read as a float.");
                if response.changed() {
                    tables::set_field(
                        work.session,
                        &table_name,
                        record,
                        field,
                        float.to_bits(),
                        &format!("Edit field {field}"),
                        work.now,
                    );
                }
                ui.label(egui::RichText::new("f32").small().color(theme::INK_FAINT));
            }
        });
    }
}

/// The text box for a numbered field that holds a string, written when the
/// box loses focus, as a schema's text column is and for the same reason:
/// the string block is append-only.
fn loose_text(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize, field: usize) {
    let table_name = work.browser.table.clone();
    let text = work.browser.buffer(work.session, record, field).clone();
    let mut editing = text.clone();
    let width = TEXT.min((ui.available_width() - 40.0).max(140.0));
    let response = ui.add(
        egui::TextEdit::singleline(&mut editing)
            .desired_width(width)
            .min_size(egui::vec2(width, 24.0)),
    );
    if editing != text {
        *work.browser.buffer(work.session, record, field) = editing.clone();
    }
    if response.lost_focus() {
        let now_in_file = work
            .session
            .table(&table_name)
            .and_then(|table| table.string_at(record, field))
            .unwrap_or_default();
        if editing != now_in_file {
            tables::set_text(
                work.session,
                &table_name,
                record,
                field,
                &editing,
                &format!("Edit field {field}"),
                work.now,
            );
        }
    }
}

fn write(work: &mut Workspace<'_>, table_name: &str, record: usize, column: &Column, value: u32) {
    tables::set_field(
        work.session,
        table_name,
        record,
        column.field,
        value,
        &format!("Edit {}", column.name),
        work.now,
    );
}

/// What this row points at, and what points at it: the inspector's part of
/// the workspace.
///
/// It is in the inspector rather than in the middle because it describes the
/// selection, and the shell's rule is that the inspector shows the selection.
/// See [`super`].
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
                .add(egui::Link::new(egui::RichText::new(text).size(theme::SMALL)))
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

    // The rows that point at this one, looked up in the reference index.
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
                    .add(egui::Link::new(egui::RichText::new(name).size(theme::SMALL)))
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

    /// `-1` is drawn as `-1`. Half the effect columns of a kit hold it, since
    /// `blank_defaults` and the shipped rows write it for "nothing here".
    /// Drawn as the `u32` the reader returns, it would show as `4294967295`
    /// where a row id belongs.
    #[test]
    fn a_column_that_says_nothing_reads_as_minus_one() {
        assert_eq!(signed(u32::MAX), "-1");
        assert_eq!(signed(0), "0");
        assert_eq!(signed(3068), "3068");
        assert_eq!(signed(i32::MAX as u32), "2147483647");
        assert_eq!(signed(i32::MAX as u32 + 1), "-2147483648");
    }
}
