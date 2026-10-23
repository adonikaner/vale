//! The client-table workspace and the views drawn inside it: the workspace a
//! table is edited in, with the browser and the form beside it.
//!
//! ```text
//! browser.rs    the list of tables and the list of rows, with their buttons
//! form.rs       the open row's form: its head, its sections, the rows that
//!               use it, and the stage beside a visual's or a kit's fields
//! field.rs      one field drawn by its schema column, and an item set's item
//!               column
//! reference.rs  a column that names a row of another table, following it,
//!               and what the open row references
//! commands.rs   the commands on a row, carried out, and the right-click menus
//!               of a row and a field
//! learning.rs   a spell's Learning section and a skill line's members
//! zones.rs      an area's sub-areas, on an `AreaTable` row's form
//! unschemad.rs  a table with no schema: every field as its number
//! previews.rs   the previews of a row: the stage beside a form and in a
//!               dialog, and a display row's picture
//! sounds.rs     the buttons that play a sound column's sounds
//! modals/       the dialogs a form opens: the reference pickers, a table's
//!               rows, a mask, a texture and a model
//! storyboard.rs the one view of a row that is not a form: a spell's visual
//!               chain as its sequence of phases
//! appearance.rs what the display tables' rows look like, for their pickers
//!               and the inspector, and the texture browser their texture
//!               fields open
//! ```
//!
//! This file holds what the files above share: [`Workspace`], the sizes, the
//! entry point [`draw`], and the table inspector, [`table`].
//!
//! ## Why a data subject replaces the viewport
//!
//! For the tools on the rail's world half, Terrain and World, the world is
//! the thing being edited: the panels surround it and it fills the middle. A
//! spell has no place in the world, and a 300-point inspector beside an
//! unrelated view of terrain is too narrow for its form.
//!
//! A Data subject therefore replaces the middle region with a searchable list
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
//! (`crate::ui::shell::rail::parts`). Tables, whose list is every file of
//! `DBFilesClient\` until one is chosen ([`browser::table_list`]), and which then
//! draws that table like any other, with a link back to the list.
//!
//! ## A row that exists only for another row is edited on that row's form
//!
//! The form is one table's columns, with three additions where a subject is
//! spread over tables. A spell's form has a Learning section ([`learning::learning`]):
//! the `SkillLineAbility` rows that put the spell in a skill line, with
//! their classes, races and next rank, and the spells that teach it, each
//! with a button that makes one. A skill line's form lists the
//! `SkillRaceClassInfo` rows that say who has it ([`learning::skill_line_members`]).
//! A set's item column says when the item's own `set_id` does not name the
//! set, and a picked item has it written.
//!
//! Each is optional: a spell in no skill line and with no teaching spell is
//! the common case and reads as one line each. The rows are ordinary rows of
//! their tables, drawn through [`field::field_in`] with the table named, so the
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
//! ([`unschemad::unschemad`]): every field as its number, with a float box where the
//! four bytes read as a float and a text box where they are the offset of a
//! string.
//!
//! ## An item column is named by the database
//!
//! `ItemSet`'s seventeen item columns hold `item_template` entries, and 1.12
//! ships no item table, so the name beside one and the picker behind its `…`
//! are the item form's: the quest tool's name cache and reference picker
//! (`crate::ui::widgets::reference`, `crate::ui::subjects::quests::picker`). With no world database the
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
//! of these operations is one entry on the undo stack. See `crate::tables`.
//!
//! ## A picker over the spell chain plays its rows
//!
//! A visual has no name and a kit is named by its models, so neither can be
//! chosen from a list of labels. The picker for a `SpellVisual`, a
//! `SpellVisualKit` or a `SpellVisualEffectName` column has the preview in
//! the dialog ([`modals::pick::pick_previewing`]): a press on a row plays it, and a double
//! press, Enter or Use writes it. `Look like a spell…` ([`modals::looks_like::looks_like`])
//! chooses a spell's visual by another spell, played, and either shares that
//! spell's visual or clones it. The Visuals and Kits tabs have the same
//! preview beside their fields ([`previews::chain_show`]). The stage is one picture,
//! so while a dialog plays, the pane behind it draws a placeholder.
//!
//! ## Commands and right-click menus
//!
//! What is done to a row other than editing a field is a
//! `crate::tables::commands::Command`, carried out by [`commands::run_command`]. A row's
//! right-click menu ([`commands::row_menu`]) lists the table's commands, and the
//! buttons on the list and the form run the same ones, so every menu entry
//! is also a button and both do the same thing. A field's name takes a
//! right-click too ([`commands::field_menu`]), and so does the title of a block drawn
//! for another table's row.
//!
//! ## Other locales are folded
//!
//! Every string in `Spell.dbc` is eight columns and a flags word, and an enUS
//! install leaves seven of the eight empty, so drawing them all would make two
//! thirds of the form blank boxes. They are real columns and stay editable,
//! under an Other locales fold at the end of the section that is closed by
//! default.
//!
//! ## Field widths
//!
//! [`LABEL`] and [`VALUE`] set the label column and the common value width. A
//! small-text label followed by a default-width `DragValue` on each line makes
//! sixty rows of grey 11-point text with no alignment, which is hard to read.
//! The form uses a label column wide enough for the longest name in the schema,
//! values at a common width, and rows tall enough to click. The field index and
//! what the schema knows about the column are in the label's tooltip rather
//! than a suffix on every label.

pub(crate) mod appearance;
pub(crate) mod browser;
pub(crate) mod commands;
pub(crate) mod field;
pub(crate) mod form;
pub(crate) mod learning;
pub(crate) mod modals;
pub(crate) mod previews;
pub(crate) mod reference;
pub(crate) mod sounds;
pub(crate) mod storyboard;
pub(crate) mod unschemad;
pub(crate) mod zones;

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tables::browser::Browser;
use crate::ui::tables::browser::{browser_panel, list, table_list};
use crate::ui::tables::form::form;
use crate::ui::widgets::theme;
use crate::ui::widgets::thumbnails::Thumbnails;

use vale_assets::tables::schema;
use vale_client::assets::GameAssets;

/// The height of one row of the list and the size of its icon. These are the
/// editor's shared list sizes, so this list and the item list look the same.
/// The cache decodes at [`crate::ui::widgets::thumbnails::SIDE`]; an icon is a 64x64 BLP whose
/// 32 level is what comes back, so the picture is that level at its own size.
const ROW_HEIGHT: f32 = theme::LIST_ROW;
const ICON: f32 = theme::LIST_PICTURE;

/// The label column. Wide enough for `EffectRealPointsPerLevel 1`, which is
/// the longest name in the spell schema.
///
/// These three are `crate::ui::widgets::rowform`'s, because the item and quest workspaces
/// draw their forms at the same widths: a change to one here is a change to
/// all three forms.
pub(in crate::ui) const LABEL: f32 = crate::ui::widgets::rowform::FORM_LABEL;

/// The common width of a value, so that a column of numbers lines up.
pub(in crate::ui) const VALUE: f32 = crate::ui::widgets::rowform::FORM_VALUE;

/// The width of a string value, which takes the rest of the row.
const TEXT: f32 = crate::ui::widgets::rowform::FORM_TEXT;

/// Everything the workspace reads, as one parameter.
///
/// Its four fields come from four different places in the shell. Bundling them
/// keeps the two entry points here to three arguments each rather than seven.
pub(crate) struct Workspace<'a> {
    /// The data subject, which decides the chain's tabs (see
    /// [`crate::tools::Tool::tabs`]). It is passed in rather than derived from
    /// `browser.table`, because a followed reference can leave the browser on
    /// a table that is not one of the chain's tabs, and the tabs must not
    /// change when that happens.
    pub(crate) tool: crate::tools::Tool,
    pub(crate) session: &'a mut EditSession,
    pub(crate) browser: &'a mut Browser,
    pub(crate) assets: &'a GameAssets,
    pub(crate) thumbnails: &'a mut Thumbnails,
    /// The pictures of models, for the model browser's rows, and the two
    /// lists it keeps beside its folders.
    pub(crate) portraits: &'a mut crate::preview::portraits::Portraits,
    pub(crate) favourites: &'a mut crate::prefs::favourites::Favourites,
    pub(crate) now: f64,
    /// The two switches toggled at the foot of the model view. They are the
    /// view bar's own: the debug overlay's wireframe and the world's particles.
    /// `None` when the caller has neither (the inspector), and for the
    /// wireframe in a build without diagnostics, where the checkbox is drawn
    /// disabled.
    pub(crate) wireframe: Option<&'a mut bool>,
    pub(crate) particles: Option<&'a mut bool>,
    /// The lights on the open map, for the one subject whose rows have a place
    /// in the world (see [`crate::tools::places::lights`]). `None` when the caller has
    /// none, which is the case in the inspector.
    pub(crate) lights: Option<&'a mut crate::tools::places::lights::Lights>,
    /// The quest tool's state, which holds the item name cache and the
    /// reference picker an item column uses. `None` when there is no world
    /// database to name an item from, and in the inspector.
    pub(crate) quests: Option<&'a mut crate::subjects::quests::Quests>,
    /// The time of day the world is showing, in half-minutes past midnight, so
    /// a band's strip can mark the viewport's current time on it.
    /// `vale_assets::tables::light::NOON` when the caller has no clock.
    pub(crate) hour: u32,
    /// What the workspace asks of the shell: a part to switch to, an area to
    /// paint, a texture to import. See [`crate::ui::shell::asks`].
    pub(crate) asks: &'a mut crate::ui::shell::asks::ShellAsks,
    /// Where a light's Fly to button sends its request. See
    /// [`crate::camera::FlyTo`].
    pub(crate) fly: &'a mut bevy::prelude::Messages<crate::camera::FlyTo>,
}

/// What the workspace's middle draws besides the form, which the
/// inspector's view of a table does not: the spell storyboard, the stage its
/// preview renders on, and the attachment lab.
pub(crate) struct Story<'a> {
    pub(crate) board: &'a mut storyboard::Storyboard,
    pub(crate) stage: &'a mut crate::preview::stage::Stage,
    pub(crate) lab: &'a mut crate::preview::lab::Lab,
}

impl<'a> Workspace<'a> {
    /// The workspace of `tool` out of the shell's state, at the hour the
    /// world shows. `lights` is whether it has the map's lights and `quests`
    /// whether it has the item names and the reference picker, which need a
    /// world database. The view bar's two switches are left `None`.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::ui) fn new(
        tool: crate::tools::Tool,
        session: &'a mut EditSession,
        editing: &'a mut crate::ui::Editing<'_>,
        assets: &'a GameAssets,
        now: f64,
        lights: bool,
        quests: bool,
    ) -> Workspace<'a> {
        Workspace::with_story(tool, session, editing, assets, now, lights, quests).0
    }

    /// [`Workspace::new`] with the [`Story`] the middle draws beside it.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::ui) fn with_story(
        tool: crate::tools::Tool,
        session: &'a mut EditSession,
        editing: &'a mut crate::ui::Editing<'_>,
        assets: &'a GameAssets,
        now: f64,
        lights: bool,
        quests: bool,
    ) -> (Workspace<'a>, Story<'a>) {
        let story = Story {
            board: &mut editing.storyboard,
            stage: &mut editing.stage,
            lab: &mut editing.lab,
        };
        let work = Workspace {
            tool,
            session,
            browser: &mut editing.browser,
            assets,
            thumbnails: &mut editing.thumbnails,
            portraits: &mut editing.portraits,
            favourites: &mut editing.favourites,
            now,
            wireframe: None,
            particles: None,
            lights: lights.then_some(&mut *editing.lights),
            quests: quests.then_some(&mut *editing.quests),
            hour: editing.clock.half_minutes,
            asks: &mut editing.asks,
            fly: &mut editing.fly,
        };
        (work, story)
    }
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
        let decode = match side > crate::ui::widgets::thumbnails::SIDE as f32 {
            true => crate::ui::widgets::thumbnails::ICON_SIDE,
            false => crate::ui::widgets::thumbnails::SIDE,
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

/// The archive path of a spell icon by its `SpellIcon` row id, or `None` for
/// id 0 and an id no row has. Shared with the reference picker's spell rows.
///
/// The session's own `SpellIcon` is read first, so an icon the project
/// re-points shows, and the client's parse is read when the session has not
/// opened the table.
pub(in crate::ui) fn spell_icon_path(
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

/// Append `.blp` to a table's texture path unless it already has it. The
/// tables store paths without an extension and the client appends it. See
/// `ui::mesh::textures::decode_rgba`, which applies the same rule in another
/// crate.
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
pub(crate) enum View {
    #[default]
    Fields,
    Storyboard,
}

/// Draw the whole workspace into the region the panels left.
pub(crate) fn draw(ui: &mut egui::Ui, mut work: Workspace<'_>, story: Story<'_>) {
    let Story { board, stage, lab } = story;
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
        stage.close();
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
    // drawn in the inspector instead (see [`crate::ui::tools::places::lights::light_inspector`]), so this
    // function has one layout, and the list keeps the width that 22,360 rows
    // need.
    browser_panel(ui, |ui| list(ui, &mut work));
    // Leave a gap between the list and the form. The panel's margin ends at its
    // edge, so without a gap `< back` and the section heads touch the list's
    // border. The gap is a child `Ui` inset by [`theme::FORM_GAP`] rather than a
    // frame, because the storyboard and the lab dock their own panels into the
    // `Ui` they are given, and a frame's content rectangle is not settled until
    // its contents are laid out.
    let rect = ui.available_rect_before_wrap();
    let inset = egui::Rect::from_min_max(rect.min + egui::vec2(theme::FORM_GAP, 0.0), rect.max);
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(inset).layout(*ui.layout()));
    form(&mut inner, &mut work, board, stage, lab);
}

/// A client table: what the open row points at, and whether the table is
/// saved.
///
/// The form is in the middle of the shell rather than here; [`crate::ui::tables`]
/// says why. The inspector shows what is true of the selection, which for a
/// row is its references, plus the one button that acts on the file rather
/// than the row.
pub(in crate::ui) fn table(ui: &mut egui::Ui, mut work: Workspace<'_>) {
    let name = work.browser.table.clone();
    // The Tables workspace on its list of tables: nothing is selected.
    if name.is_empty() {
        theme::heading(ui, "Table");
        theme::note(ui, "Choose a table to see its file and the open row's references.");
        return;
    }
    let session = &mut *work.session;
    theme::heading(ui, "Table");
    theme::row(ui, "file", |ui| {
        ui.label(theme::number(format!("{name}.dbc")));
    });
    if let Some(open) = session.table(&name) {
        let rows = open.record_count();
        let fields = open.field_count();
        theme::row(ui, "rows", |ui| {
            ui.label(theme::number(format!("{rows} x {fields}")));
        });
    }
    let unsaved = session.unsaved_tables.contains(&name);
    theme::row(ui, "status", |ui| match unsaved {
        true => {
            ui.label(egui::RichText::new("unsaved").color(theme::WARN));
        }
        false => {
            ui.label(egui::RichText::new("saved").color(theme::INK_DIM));
        }
    });
    if unsaved && theme::primary(ui, "Save table").clicked() {
        session.save_table(&name);
    }
    ui.add_space(4.0);

    // A display table's open row, drawn.
    previews::display_preview(ui, &mut work);
    ui.add_space(4.0);

    reference::references(ui, &mut work);
}

/// A table workspace's inspector panel: what the open row points at. The form
/// itself is in the middle, where the viewport would be ([`workspace`]),
/// following the shell's rule that the inspector shows the selection.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { tool, session, assets, now, .. } = subject;
    table(ui, Workspace::new(tool, session, editing, assets, now, false, false));
}

/// A table workspace in the middle of a window: the table browser on the
/// tool's tables.
pub(crate) fn workspace(ui: &mut egui::Ui, tool: crate::tools::Tool, bench: crate::ui::workspace::Bench<'_, '_, '_>) {
    let crate::ui::workspace::Bench { session, editing, playing, assets, now, wireframe, particles, .. } = bench;
    // The item names and the item picker need the database; without one an
    // item column is its number.
    let named = playing.server.resolve().is_some();
    let (mut work, story) = Workspace::with_story(tool, session, editing, assets, now, true, named);
    // The model view's foot toggles the same two the view bar does, on the
    // same edited copies, where the window has a view bar.
    work.wireframe = wireframe;
    work.particles = particles;
    draw(ui, work, story);
    // The reference picker, which an item column of a client table opens:
    // the quest form's dialog, writing into the table's field.
    crate::ui::subjects::quests::picker(
        ui.ctx(),
        session,
        &mut editing.quests,
        assets,
        &mut editing.thumbnails,
        None,
        now,
    );
}
