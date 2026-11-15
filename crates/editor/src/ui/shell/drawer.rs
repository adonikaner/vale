//! The asset drawer: the panel under the viewport that lists the tilesets or
//! the models for the inspector section that asked for it this frame.
//!
//! ```text
//! header  the list's name, what a click does, the search box, the picture
//!         size, and Hide
//! left    Starred and Recent (models only), then the folders, opened along
//!         the path to the folder shown
//! right   the folder shown, then its folders and files as a grid of
//!         pictures; while searching, every match in the catalogue
//! ```
//!
//! ## Why the lists are not in the inspector
//!
//! The inspector is a vertical `ScrollArea`. egui gives a `ScrollArea` inside
//! it the height between its own top and the bottom of the visible panel, and
//! no less than `min_scrolled_size`, 64 points. A list that started low in the
//! inspector was therefore two rows tall, and its height changed as the
//! inspector scrolled. The drawer is a docked panel, so its height is set by
//! its resize edge alone.
//!
//! ## How a section asks for the drawer
//!
//! A section calls [`Drawer::offer`] in each frame it wants the drawer, with a
//! [`Purpose`] and the paths it shows as chosen. The inspector is drawn before
//! the drawer, so the drawer shows the offer made in the same frame, and it is
//! not drawn in a frame without one. A click is kept with the purpose it was
//! made for and returned by the next frame's `offer` with that purpose, so it
//! cannot reach a different section.
//!
//! ## Where the folder and the search text are kept
//!
//! In the tools' own fields: `Textures::folder` and `Textures::search` for the
//! tilesets, `Placing::folder` and `Placing::search` for the models. The
//! inspector's Show Folder button and the Import Texture dialog set them, and
//! the drawer shows what they hold.

use bevy::prelude::Resource;
use bevy_egui::egui;

use crate::prefs::favourites::Favourites;
use crate::preview::portraits::{self, Portraits};
use crate::session::files::Spellings;
use crate::tools::ground::textures::Textures;
use crate::tools::placements::place::{Kind, Placing, SEP};
use crate::ui::tools::placements::place::list_of;
use crate::ui::widgets::folders::{breadcrumb, folder_row, split, split_folder};
use crate::ui::widgets::theme;
use crate::ui::widgets::thumbnails::{self, Thumbnails};

/// The drawer's height before it is first resized, in points.
const HEIGHT: f32 = 280.0;

/// The least height the resize edge allows: the header, the path row and one
/// row of pictures at the largest size.
const MIN_HEIGHT: f32 = 190.0;

/// The width of the folder list on the left, in points.
const TREE_WIDTH: f32 = 210.0;

/// The range of a picture's side, in points. 128 is the side of a model's
/// picture (`portraits::SIDE`), so a larger tile would only enlarge its pixels.
const TILE: std::ops::RangeInclusive<f32> = 48.0..=128.0;

/// The space between two tiles of the grid, in points.
const GAP: f32 = 6.0;

/// The height of one line of a tile's label, in points.
const LINE: f32 = 14.0;

/// What a click in the drawer is for. Each is one inspector section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// The texture brush's texture, which the Textures and Chunks tools share.
    Brush,
    /// The road tool's surface or verge texture, as its Sets control chooses.
    Road,
    /// The model Place mode puts on the cursor.
    Place(Kind),
    /// A model added to the cluster, or taken out when it is already in it.
    Cluster,
    /// The model Replace Model writes over the selected placements' model.
    Replace(Kind),
    /// A doodad added inside the building editor's building.
    BuildingDoodad,
}

impl Purpose {
    /// The model kind the drawer lists, or `None` for the tilesets.
    fn models(self) -> Option<Kind> {
        match self {
            Purpose::Brush | Purpose::Road => None,
            Purpose::Place(kind) | Purpose::Replace(kind) => Some(kind),
            Purpose::Cluster | Purpose::BuildingDoodad => Some(Kind::Doodad),
        }
    }

    /// The drawer's heading.
    fn title(self) -> &'static str {
        match self.models() {
            None => "Tilesets",
            Some(Kind::Doodad) => "Doodads",
            Some(Kind::Wmo) => "WMOs",
        }
    }

    /// What a click on a file does, shown beside the heading.
    fn about(self) -> &'static str {
        match self {
            Purpose::Brush => "Click a tileset to paint with it.",
            Purpose::Road => "Click a tileset to set the road's surface or verge texture.",
            Purpose::Place(_) => "Click a model to put it on the cursor.",
            Purpose::Cluster => "Click a model to add it to the cluster, or a highlighted one to take it out.",
            Purpose::Replace(_) => "Click a model to choose it for Replace Model.",
            Purpose::BuildingDoodad => "Click a model to put it on the cursor inside the building.",
        }
    }
}

/// What the grid shows for a model catalogue: a folder, or one of the two
/// lists kept in `crate::prefs::favourites`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Showing {
    #[default]
    Folder,
    Starred,
    Recent,
}

/// The section that asked for the drawer this frame, and what it has chosen.
#[derive(Debug)]
struct Offer {
    purpose: Purpose,
    chosen: Vec<String>,
}

/// The drawer's state. See the module comment.
#[derive(Resource, Debug)]
pub(crate) struct Drawer {
    /// This frame's offer, taken by [`draw`].
    offered: Option<Offer>,
    /// A file clicked in the drawer in the last frame, with the purpose it was
    /// clicked for.
    picked: Option<(Purpose, String)>,
    /// Whether the drawer is reduced to its header row.
    hidden: bool,
    /// The side of a tile's picture, in points.
    tile: f32,
    /// What the grid shows, per model kind.
    showing: [Showing; 2],
}

impl Default for Drawer {
    fn default() -> Drawer {
        Drawer {
            offered: None,
            picked: None,
            hidden: false,
            tile: 80.0,
            showing: [Showing::Folder; 2],
        }
    }
}

impl Drawer {
    /// Show the drawer this frame for `purpose`, with `chosen` marked.
    /// Answers the path clicked for the same purpose in the last frame.
    pub(crate) fn offer(&mut self, purpose: Purpose, chosen: &[String]) -> Option<String> {
        self.offered = Some(Offer { purpose, chosen: chosen.to_vec() });
        match self.picked.take() {
            Some((asked, path)) if asked == purpose => Some(path),
            other => {
                self.picked = other;
                None
            }
        }
    }

    /// Show the open folder of a model kind rather than Starred or Recent.
    /// For the inspector's Show Folder button, which sets the folder.
    pub(crate) fn show_folder(&mut self, kind: Kind) {
        self.showing[kind as usize] = Showing::Folder;
    }
}

/// One tile of the grid.
enum Entry {
    /// A folder under the one shown: its full prefix, its name as spelled,
    /// and how many files are under it.
    Folder(String, String, usize),
    /// A file, by its catalogue path.
    File(String),
}

/// What the tree and the grid asked for this frame.
enum Go {
    Folder(String),
    Show(Showing),
}

/// Draw the drawer docked at the bottom of `root`, when a section offered it
/// this frame and `room` is true. `room` is false while a workspace covers
/// the viewport or a playtest runs.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    root: &mut egui::Ui,
    room: bool,
    drawer: &mut Drawer,
    textures: &mut Textures,
    placing: &mut Placing,
    thumbnails: &mut Thumbnails,
    portraits: &mut Portraits,
    favourites: &mut Favourites,
) {
    // A click the section did not take this frame was made for a section that
    // is no longer drawn.
    drawer.picked = None;
    let Some(offer) = drawer.offered.take() else {
        return;
    };
    if !room {
        return;
    }
    let purpose = offer.purpose;
    let kind = purpose.models();
    let (catalogue, spell, mut folder, mut search) = match kind {
        None => (
            textures.catalogue.as_slice(),
            std::sync::Arc::clone(&textures.spellings),
            textures.folder.clone(),
            textures.search.clone(),
        ),
        Some(kind) => (
            placing.catalogue(kind),
            std::sync::Arc::clone(&placing.spellings),
            placing.folder(kind).to_string(),
            placing.search.clone(),
        ),
    };
    let was_search = search.clone();
    let mut go: Option<Go> = None;

    let panel = match drawer.hidden {
        // A separate id, because a bottom panel keeps the height its content
        // had in the last frame, and the header alone would become the height
        // the drawer opens at.
        true => egui::Panel::bottom("editor-drawer-hidden").resizable(false),
        false => egui::Panel::bottom("editor-drawer")
            .resizable(true)
            .default_size(HEIGHT)
            .min_size(MIN_HEIGHT)
            .max_size((root.available_height() * 0.7).max(MIN_HEIGHT)),
    };
    panel.frame(crate::ui::side(theme::PANEL)).show(root, |ui| {
        header(ui, drawer, purpose, &mut search);
        if drawer.hidden {
            return;
        }
        let body = ui.available_rect_before_wrap();
        if !catalogue.iter().any(|path| !path.is_empty()) {
            theme::note(
                ui,
                match kind {
                    None => "Reading the tileset list\u{2026}",
                    Some(_) => "Reading the model list\u{2026}",
                },
            );
            ui.advance_cursor_after_rect(body);
            return;
        }
        let split_at = body.left() + TREE_WIDTH.min(body.width() * 0.4);
        let tree_rect = egui::Rect::from_min_max(body.min, egui::pos2(split_at, body.max.y));
        let grid_rect = egui::Rect::from_min_max(egui::pos2(split_at + 9.0, body.min.y), body.max);
        ui.painter().vline(split_at + 4.0, body.y_range(), egui::Stroke::new(1.0, theme::LINE));

        let showing = match kind {
            Some(kind) if search.is_empty() => drawer.showing[kind as usize],
            _ => Showing::Folder,
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(tree_rect), |ui| {
            let asked = tree_pane(ui, catalogue, &folder, showing, kind, favourites, &spell, purpose.title());
            go = asked.or(go.take());
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(grid_rect), |ui| {
            let asked = grid_pane(
                ui,
                catalogue,
                &folder,
                &search,
                showing,
                &offer,
                drawer,
                thumbnails,
                portraits,
                favourites,
                &spell,
            );
            go = asked.or(go.take());
        });
        ui.advance_cursor_after_rect(body);
    });

    // A folder chosen in the tree or the grid ends the search, since the
    // grid shows matches while there is one.
    match go {
        Some(Go::Folder(to)) => {
            folder = to.to_ascii_lowercase();
            if !folder.is_empty() && !folder.ends_with(SEP) {
                folder.push(SEP);
            }
            search.clear();
            if let Some(kind) = kind {
                drawer.showing[kind as usize] = Showing::Folder;
            }
        }
        Some(Go::Show(showing)) => {
            search.clear();
            if let Some(kind) = kind {
                drawer.showing[kind as usize] = showing;
            }
        }
        None => {}
    }
    match kind {
        None => {
            if textures.folder != folder {
                textures.folder = folder;
            }
            if search != was_search {
                textures.search = search;
            }
        }
        Some(kind) => {
            if placing.folder(kind) != folder {
                placing.open_folder(kind, &folder);
            }
            if search != was_search {
                placing.search = search;
            }
        }
    }
}

/// The header row: the heading and what a click does, then at the right the
/// Hide or Show button, the picture size and the search box.
fn header(ui: &mut egui::Ui, drawer: &mut Drawer, purpose: Purpose, search: &mut String) {
    ui.horizontal(|ui| {
        ui.add(
            egui::Label::new(egui::RichText::new(purpose.title().to_uppercase()).size(theme::SMALL).color(theme::INK_FAINT))
                .selectable(false),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (label, hint) = match drawer.hidden {
                true => ("Show", "Show the list."),
                false => ("Hide", "Reduce the drawer to this row. The section that opened it keeps its choice."),
            };
            if ui.button(label).on_hover_text(hint).clicked() {
                drawer.hidden = !drawer.hidden;
            }
            if drawer.hidden {
                return;
            }
            ui.spacing_mut().slider_width = 80.0;
            ui.add(egui::Slider::new(&mut drawer.tile, TILE).show_value(false))
                .on_hover_text("The size of the pictures.");
            ui.add(
                egui::TextEdit::singleline(search)
                    .hint_text(match purpose.models() {
                        None => "Search the tilesets",
                        Some(_) => "Search the models",
                    })
                    .desired_width(220.0),
            );
            ui.add(
                egui::Label::new(egui::RichText::new(purpose.about()).size(theme::SMALL).color(theme::INK_DIM))
                    .truncate(),
            );
        });
    });
    ui.add_space(4.0);
}

/// The left pane: Starred and Recent for a model catalogue, then the folders
/// from the top, with each folder on the path to `here` opened.
#[allow(clippy::too_many_arguments)]
fn tree_pane(
    ui: &mut egui::Ui,
    catalogue: &[String],
    here: &str,
    showing: Showing,
    kind: Option<Kind>,
    favourites: &Favourites,
    spell: &Spellings,
    salt: &str,
) -> Option<Go> {
    let mut go: Option<Go> = None;
    let row = ui.spacing().interact_size.y + 4.0;
    egui::ScrollArea::vertical()
        .id_salt(("drawer-tree", salt))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if let Some(kind) = kind {
                let list = list_of(kind);
                for (name, which, count) in [
                    ("Starred", Showing::Starred, favourites.starred(list).len()),
                    ("Recent", Showing::Recent, favourites.recent(list).len()),
                ] {
                    if list_row(ui, name, which, count, showing == which, row).clicked() {
                        go = Some(Go::Show(which));
                    }
                }
                ui.add_space(4.0);
            }
            let lit = (showing == Showing::Folder).then_some(here);
            if let Some(to) = tree(ui, catalogue, here, lit, "", 0, row, spell) {
                go = Some(Go::Folder(to));
            }
        });
    go
}

/// The folders under `prefix`, `depth` levels in, and under each one that
/// `here` is inside, its own folders. Answers the folder a click asked for.
///
/// A click on a closed folder opens it. A click on the folder shown closes
/// it and shows the folder above. A click on an open folder above the one
/// shown shows it, which closes the folders under it.
#[allow(clippy::too_many_arguments)]
fn tree(
    ui: &mut egui::Ui,
    catalogue: &[String],
    here: &str,
    lit: Option<&str>,
    prefix: &str,
    depth: usize,
    row: f32,
    spell: &Spellings,
) -> Option<String> {
    let mut go: Option<String> = None;
    let (folders, _) = split_folder(catalogue, prefix);
    for (name, count) in folders {
        let folder = format!("{prefix}{name}{SEP}");
        let open = here.starts_with(&folder);
        let shown = match spell.name_of(&folder) {
            "" => name.as_str(),
            spelled => spelled,
        };
        // The highlight goes under the row, which is painted after it is
        // allocated, so its place in the paint order is reserved first.
        let under = ui.painter().add(egui::Shape::Noop);
        let response = folder_row(ui, shown, count, depth, open, row);
        if lit == Some(folder.as_str()) {
            ui.painter().set(under, egui::Shape::rect_filled(response.rect, egui::CornerRadius::same(3), theme::ACCENT_SUNK));
        }
        if response.clicked() {
            go = Some(match here == folder {
                true => prefix.to_string(),
                false => folder.clone(),
            });
        }
        if open {
            go = tree(ui, catalogue, here, lit, &folder, depth + 1, row, spell).or(go);
        }
    }
    go
}

/// The Starred or Recent row of the left pane: a painted mark, the name and
/// how many models the list holds.
fn list_row(ui: &mut egui::Ui, name: &str, which: Showing, count: usize, on: bool, row: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row - 2.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if on || response.hovered() {
            let fill = match on {
                true => theme::ACCENT_SUNK,
                false => theme::RAISED,
            };
            painter.rect_filled(rect, egui::CornerRadius::same(3), fill);
        }
        let mark = egui::pos2(rect.left() + 12.0, rect.center().y);
        match which {
            Showing::Starred => theme::star(painter, mark, 6.0, true),
            _ => {
                let stroke = egui::Stroke::new(1.2, theme::INK_DIM);
                painter.circle_stroke(mark, 5.5, stroke);
                painter.line_segment([mark, mark + egui::vec2(0.0, -3.5)], stroke);
                painter.line_segment([mark, mark + egui::vec2(2.5, 0.0)], stroke);
            }
        }
        let text = painter.with_clip_rect(rect);
        text.text(
            egui::pos2(rect.left() + 24.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(theme::LIST_TITLE),
            theme::INK,
        );
        text.text(
            egui::pos2(rect.right() - 6.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::proportional(theme::SMALL),
            theme::INK_FAINT,
        );
    }
    response
}

/// The right pane: where the grid is and how much it holds, then the grid.
/// A click on a file is kept in `drawer.picked`; a click on a folder is
/// answered.
#[allow(clippy::too_many_arguments)]
fn grid_pane(
    ui: &mut egui::Ui,
    catalogue: &[String],
    here: &str,
    search: &str,
    showing: Showing,
    offer: &Offer,
    drawer: &mut Drawer,
    thumbnails: &mut Thumbnails,
    portraits: &mut Portraits,
    favourites: &mut Favourites,
    spell: &Spellings,
) -> Option<Go> {
    let kind = offer.purpose.models();
    let noun = match kind {
        None => "tilesets",
        Some(_) => "models",
    };
    let mut go: Option<Go> = None;
    let needle = search.to_ascii_lowercase();
    let entries: Vec<Entry> = if !needle.is_empty() {
        let matches: Vec<Entry> = catalogue
            .iter()
            .filter(|path| !path.is_empty() && path.to_ascii_lowercase().contains(&needle))
            .map(|path| Entry::File(path.clone()))
            .collect();
        theme::note(ui, format!("{} of {} {noun} match", matches.len(), catalogue.len()));
        matches
    } else if let (Some(kind), Showing::Starred | Showing::Recent) = (kind, showing) {
        let (name, paths) = match showing {
            Showing::Starred => ("Starred", favourites.starred(list_of(kind))),
            _ => ("Recent", favourites.recent(list_of(kind))),
        };
        theme::note(ui, format!("{name} \u{b7} {} {noun}", paths.len()));
        paths.into_iter().map(Entry::File).collect()
    } else {
        if let Some(to) = breadcrumb(ui, here, "", spell) {
            go = Some(Go::Folder(to));
        }
        let (folders, files) = split_folder(catalogue, here);
        theme::note(
            ui,
            match folders.len() {
                0 => format!("{} {noun}", files.len()),
                n => format!("{n} folders \u{b7} {} {noun}", files.len()),
            },
        );
        folders
            .into_iter()
            .map(|(name, count)| {
                let folder = format!("{here}{name}{SEP}");
                let shown = match spell.name_of(&folder) {
                    "" => name,
                    spelled => spelled.to_string(),
                };
                Entry::Folder(folder, shown, count)
            })
            .chain(files.into_iter().map(Entry::File))
            .collect()
    };
    ui.add_space(2.0);

    // A file's folder is a second line wherever the files are not all from
    // the one folder named above the grid.
    let two_lines = !needle.is_empty() || showing != Showing::Folder;
    let side = drawer.tile;
    let height = side + 4.0 + LINE * if two_lines { 2.0 } else { 1.0 };
    let width = ui.available_width() - ui.spacing().scroll.allocated_width();
    let columns = ((width + GAP) / (side + GAP)).floor().max(1.0) as usize;
    let rows = entries.len().div_ceil(columns);
    // One scroll position per folder and per list, so opening a folder starts
    // at its top.
    let salt = match (needle.is_empty(), showing) {
        (false, _) => format!("search {}", offer.purpose.title()),
        (true, Showing::Folder) => format!("folder {}", here),
        (true, other) => format!("{other:?} {}", offer.purpose.title()),
    };
    // `show_rows` builds only the rows on screen, so only their pictures are
    // asked for. See `crate::ui::widgets::thumbnails` and
    // `crate::preview::portraits`.
    egui::ScrollArea::vertical()
        .id_salt(("drawer-grid", salt))
        .auto_shrink([false, false])
        .show_rows(ui, height, rows, |ui, range| {
            for at in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    let end = ((at + 1) * columns).min(entries.len());
                    for entry in &entries[at * columns..end] {
                        match entry {
                            Entry::Folder(folder, shown, count) => {
                                let response = tile(ui, side, height, false);
                                if ui.is_rect_visible(response.rect) {
                                    let square = picture_rect(response.rect, side);
                                    folder_picture(ui.painter(), square, *count);
                                    label(ui, square, shown, None, false);
                                }
                                if response.on_hover_text(spell.of(folder).to_string()).clicked() {
                                    go = Some(Go::Folder(folder.clone()));
                                }
                            }
                            Entry::File(path) => {
                                let on = offer.chosen.iter().any(|had| had.eq_ignore_ascii_case(path));
                                file_tile(
                                    ui, drawer, offer.purpose, path, on, side, height, two_lines, thumbnails, portraits,
                                    favourites, spell,
                                );
                            }
                        }
                    }
                });
            }
        });
    go
}

/// One file's tile: its picture, its name, its folder when `two_lines`, and
/// for a model the star. A click is kept in `drawer.picked` for `purpose`.
#[allow(clippy::too_many_arguments)]
fn file_tile(
    ui: &mut egui::Ui,
    drawer: &mut Drawer,
    purpose: Purpose,
    path: &str,
    on: bool,
    side: f32,
    height: f32,
    two_lines: bool,
    thumbnails: &mut Thumbnails,
    portraits: &mut Portraits,
    favourites: &mut Favourites,
    spell: &Spellings,
) {
    let response = tile(ui, side, height, on);
    if !ui.is_rect_visible(response.rect) {
        return;
    }
    let square = picture_rect(response.rect, side);
    let shown = spell.of(path);
    let (folder, _) = split(shown);
    match purpose.models() {
        None => texture_picture(ui, thumbnails, path, square),
        Some(_) => portraits::paint(ui, portraits, path, square),
    }
    label(ui, square, stem(shown), two_lines.then_some(folder), on);

    let mut starred = false;
    if let Some(kind) = purpose.models() {
        let list = list_of(kind);
        let starred_now = favourites.is_starred(list, path);
        // Drawn on a starred model, and on any model under the pointer so it
        // can be starred.
        if starred_now || response.contains_pointer() {
            let star = egui::Rect::from_min_size(
                egui::pos2(square.right() - 18.0, square.top() + 2.0),
                egui::vec2(16.0, 16.0),
            );
            let hint = match starred_now {
                true => "Remove this model from Starred.",
                false => "Add this model to Starred.",
            };
            starred = theme::star_button(ui, ui.id().with(("drawer-star", path)), star, starred_now, hint);
            if starred {
                favourites.toggle(list, path);
            }
        }
    }
    if !starred && response.on_hover_text(shown.to_string()).clicked() {
        if let Some(kind) = purpose.models() {
            favourites.used(list_of(kind), path);
        }
        drawer.picked = Some((purpose, path.to_string()));
    }
}

/// Allocate one tile, `side` wide and `height` tall, and paint its background:
/// the accent when `on`, a raised fill under the pointer.
fn tile(ui: &mut egui::Ui, side: f32, height: f32, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if on || response.hovered() {
            let fill = match on {
                true => theme::ACCENT_SUNK,
                false => theme::RAISED,
            };
            painter.rect_filled(rect, egui::CornerRadius::same(3), fill);
        }
        if on {
            painter.rect_stroke(
                rect,
                egui::CornerRadius::same(3),
                egui::Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
    }
    response
}

/// The square a tile's picture is drawn in: the top of the tile, inset 3
/// points.
fn picture_rect(tile: egui::Rect, side: f32) -> egui::Rect {
    egui::Rect::from_min_size(tile.min + egui::vec2(3.0, 3.0), egui::Vec2::splat(side - 6.0))
}

/// A tileset's picture, decoded at the smallest of the two grid sizes that is
/// not smaller than the tile. A sunk square stands for it until it is
/// decoded.
fn texture_picture(ui: &egui::Ui, thumbnails: &mut Thumbnails, path: &str, square: egui::Rect) {
    let pixels = match square.width() <= thumbnails::ICON_SIDE as f32 {
        true => thumbnails::ICON_SIDE,
        false => 2 * thumbnails::ICON_SIDE,
    };
    thumbnails.want_at(path, pixels);
    let painter = ui.painter();
    match thumbnails.get_at(path, pixels) {
        Some(texture) => painter.image(
            texture,
            square,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        ),
        None => painter.rect_filled(square, egui::CornerRadius::same(2), theme::SUNK),
    };
}

/// A folder's picture: a painted folder with how many files are under it.
fn folder_picture(painter: &egui::Painter, square: egui::Rect, count: usize) {
    let w = square.width();
    let body = egui::Rect::from_min_max(
        egui::pos2(square.left() + w * 0.1, square.top() + w * 0.28),
        egui::pos2(square.right() - w * 0.1, square.bottom() - w * 0.12),
    );
    let tab = egui::Rect::from_min_size(
        egui::pos2(body.left(), body.top() - w * 0.09),
        egui::vec2(w * 0.32, w * 0.12),
    );
    painter.rect_filled(tab, egui::CornerRadius::same(2), theme::EDGE);
    painter.rect_filled(body, egui::CornerRadius::same(3), theme::RAISED_HOT);
    painter.text(
        body.center(),
        egui::Align2::CENTER_CENTER,
        count.to_string(),
        egui::FontId::proportional(theme::SMALL),
        theme::INK_DIM,
    );
}

/// A tile's label under its picture: the name, and the folder under it when
/// given. Each line is cut with an ellipsis at the picture's width.
fn label(ui: &egui::Ui, square: egui::Rect, name: &str, folder: Option<&str>, on: bool) {
    let colour = match on {
        true => theme::INK,
        false => theme::INK_DIM,
    };
    let mut top = square.bottom() + 2.0;
    for (text, size, colour) in [(Some(name), 12.0, colour), (folder, theme::SMALL, theme::INK_FAINT)] {
        let Some(text) = text else { continue };
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), egui::FontId::proportional(size), colour);
        job.wrap = egui::text::TextWrapping::truncate_at_width(square.width());
        let galley = ui.painter().layout_job(job);
        let at = egui::pos2(square.center().x - galley.size().x * 0.5, top);
        ui.painter().galley(at, galley, colour);
        top += LINE;
    }
}

/// A file's name without its folder and extension.
fn stem(path: &str) -> &str {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A click is answered only to the section it was made for, and only
    /// once.
    #[test]
    fn a_pick_reaches_only_its_own_section() {
        let mut drawer = Drawer {
            picked: Some((Purpose::Place(Kind::Doodad), "world\\tree.m2".to_string())),
            ..Drawer::default()
        };
        assert_eq!(drawer.offer(Purpose::Replace(Kind::Doodad), &[]), None);
        assert_eq!(
            drawer.offer(Purpose::Place(Kind::Doodad), &[]),
            Some("world\\tree.m2".to_string())
        );
        assert_eq!(drawer.offer(Purpose::Place(Kind::Doodad), &[]), None);
    }

    /// A tile names the file without its folder or extension.
    #[test]
    fn a_tile_names_the_file_alone() {
        assert_eq!(stem("Tileset\\Elwynn\\ElwynnGrassBase.blp"), "ElwynnGrassBase");
        assert_eq!(stem("World\\wmo\\Azeroth\\Buildings\\Inn.wmo"), "Inn");
        assert_eq!(stem("noextension"), "noextension");
    }
}
