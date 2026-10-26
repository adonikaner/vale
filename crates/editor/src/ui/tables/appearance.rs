//! What a row of a display table looks like, and the texture browser.
//!
//! The display forms (`CreatureDisplayInfo` and its `Extra`,
//! `CreatureModelData`, `GameObjectDisplayInfo` and `ItemDisplayInfo`) choose
//! models, other display rows and textures, and each of those is chosen by
//! looking at it. This file resolves a row to a picture ([`shown`]), draws it
//! in a picker's preview and in the inspector ([`pane`]), and holds the
//! texture browser ([`textures`]): the textures in the folder a field's file
//! is found in as a grid, the selected one large, and for a skin, the model
//! wearing it.
//!
//! A row of `CreatureDisplayInfo`, its `Extra` or `ItemDisplayInfo` is drawn
//! as the client resolves it, through the tables the client reads, so a row is
//! resolved once per publish of them and kept in `Browser::shown`. A
//! `CreatureModelData` row is its path in the project's copy wearing the
//! skins of a display that names it, kept the same way. A
//! `GameObjectDisplayInfo` path is read from the project's copy each time,
//! which is a string read.

use bevy_egui::egui;

use super::browser::Picture;
use super::Workspace;
use crate::ui::widgets::theme;
use crate::preview::portraits::Worn;
use crate::session::EditSession;
use crate::tables::{self, Modal, Shown};
use vale_assets::tables::item::Component;
use vale_client::assets::GameAssets;

/// What a text field of a display table names, where a browser can choose it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileField {
    /// A model's path, chosen in the model browser.
    Model,
    /// An item model's file name without its folder, chosen in the model
    /// browser. The slot the item is worn in decides the folder under
    /// `Item\ObjectComponents\`.
    ItemModel,
    /// A texture's name, chosen in the texture browser.
    Texture(Textures),
}

/// Which textures a texture field names, and so where the browser looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Textures {
    /// A creature's skin, in the folder of the display's model.
    Skin,
    /// An item model's skin, in the folder of the model the field `model`
    /// names. A cloak has no model, and its skin is in
    /// `Item\ObjectComponents\Cape\`.
    ItemSkin { model: usize },
    /// A bag icon, in `Interface\Icons\`.
    Icon,
    /// A texture painted on the wearer's body, in the component's folder
    /// under `Item\TextureComponents\`, named without its `_U`, `_M` or `_F`.
    Body(Component),
}

/// Which display-table text fields name a file. The indices are the display
/// schemas' (`vale_assets::tables::schema`).
pub(crate) fn file_field(table: &str, field: usize) -> Option<FileField> {
    match (table, field) {
        ("CreatureModelData", 2) | ("GameObjectDisplayInfo", 1) => Some(FileField::Model),
        ("ItemDisplayInfo", 1 | 2) => Some(FileField::ItemModel),
        ("CreatureDisplayInfo", 6..=8) => Some(FileField::Texture(Textures::Skin)),
        ("ItemDisplayInfo", 3 | 4) => Some(FileField::Texture(Textures::ItemSkin { model: field - 2 })),
        ("ItemDisplayInfo", 5) => Some(FileField::Texture(Textures::Icon)),
        ("ItemDisplayInfo", 14..=21) => {
            Some(FileField::Texture(Textures::Body(Component::ALL[field - 14])))
        }
        _ => None,
    }
}

/// The display tables whose rows have a picture, under the name they are
/// kept by. `None` for any other table.
pub(super) fn display_table(table: &str) -> Option<&'static str> {
    [
        "CreatureDisplayInfo",
        "CreatureDisplayInfoExtra",
        "CreatureModelData",
        "GameObjectDisplayInfo",
        "ItemDisplayInfo",
    ]
    .into_iter()
    .find(|name| *name == table)
}

/// A model with nothing on it: a doodad, an object, a model path in a form.
fn plain(path: String) -> Worn {
    Worn {
        path,
        skins: Vec::new(),
        hair: None,
        dress: Default::default(),
        look: None,
        cloak: None,
        hung: Vec::new(),
    }
}

/// An item model wearing `texture` in the object skin's slot, which is slot 4,
/// as `crate::subjects::items::resolve_look` dresses one.
fn item_worn(path: String, texture: String) -> Worn {
    Worn {
        skins: vec![String::new(), String::new(), String::new(), String::new(), texture],
        ..plain(path)
    }
}

/// What the row `id` of `table` looks like.
pub(super) fn shown(work: &mut Workspace<'_>, table: &'static str, id: u32) -> Shown {
    if table == "GameObjectDisplayInfo" {
        return Shown {
            worn: model_of(work, table, 1, id).map(plain),
            why: "The row names no model.",
            ..Shown::default()
        };
    }
    let generation = work.assets.tables_generation();
    if work.browser.shown_at != generation {
        work.browser.shown.clear();
        work.browser.shown_at = generation;
    }
    if let Some(had) = work.browser.shown.get(&(table, id)) {
        return had.clone();
    }
    let found = resolve(work, table, id);
    work.browser.shown.insert((table, id), found.clone());
    found
}

/// The model path the field `field` of the row `id` of `table` names, in the
/// project's copy, or `None` for an empty field.
fn model_of(work: &mut Workspace<'_>, table: &str, field: usize, id: u32) -> Option<String> {
    work.browser
        .record_of(work.session, table, id)
        .and_then(|record| work.session.table(table)?.string_at(record, field))
        .filter(|path| !path.trim().is_empty())
        .map(|path| vale_assets::world::m2::model_path(&path).to_ascii_lowercase())
}

/// [`shown`] for a row of a table the client reads, worked out.
fn resolve(work: &mut Workspace<'_>, table: &str, id: u32) -> Shown {
    match table {
        // A creature model has no skin of its own: it is drawn wearing the
        // skins of the first display that names it, or bare where none does.
        "CreatureModelData" => {
            let Some(path) = model_of(work, table, 2, id) else {
                return Shown {
                    why: "The row names no model.",
                    ..Shown::default()
                };
            };
            let worn = work
                .browser
                .used_by(work.session, "CreatureModelData", id)
                .into_iter()
                .filter(|at| at.table == "CreatureDisplayInfo")
                .find_map(|at| {
                    let display = work.session.table("CreatureDisplayInfo")?.u32_at(at.record, 0)?;
                    crate::subjects::displays::resolve_worn(work.assets, display)
                })
                .map(|worn| Worn { path: path.clone(), ..worn })
                .unwrap_or_else(|| plain(path));
            Shown {
                worn: Some(worn),
                ..Shown::default()
            }
        }
        "CreatureDisplayInfo" => Shown {
            worn: crate::subjects::displays::resolve_worn(work.assets, id),
            why: "This display does not resolve to a model yet: its ModelId names no model, \
                  or the edit has not reached the client's tables.",
            ..Shown::default()
        },
        // An appearance is drawn on the first display that wears it.
        "CreatureDisplayInfoExtra" => {
            let display = work
                .browser
                .used_by(work.session, "CreatureDisplayInfoExtra", id)
                .into_iter()
                .find(|at| at.table == "CreatureDisplayInfo")
                .and_then(|at| work.session.table("CreatureDisplayInfo")?.u32_at(at.record, 0));
            Shown {
                worn: display.and_then(|display| crate::subjects::displays::resolve_worn(work.assets, display)),
                why: match display {
                    Some(_) => "The display that wears this appearance does not resolve to a \
                                model yet.",
                    None => "No CreatureDisplayInfo row names this appearance, so there is no \
                             display to draw it on.",
                },
                ..Shown::default()
            }
        }
        "ItemDisplayInfo" => {
            let inventory_type = item_slot(work, id);
            let Some(look) = crate::subjects::items::resolve_look(work.assets, id, inventory_type, 1, 0)
            else {
                return Shown {
                    why: "The client's ItemDisplayInfo has no row with this id yet: the edit \
                          has not reached the client's tables.",
                    ..Shown::default()
                };
            };
            let own = look.models.first().cloned();
            let body = match own {
                Some(_) => None,
                None => crate::subjects::items::resolve_body(work.assets, id, inventory_type, 1, 0)
                    .filter(|body| {
                        look.painted() > 0
                            || look.geoset_groups.iter().any(|group| *group != 0)
                            || body.cloak.is_some()
                    }),
            };
            Shown {
                on_a_body: own.is_none() && body.is_some(),
                worn: own.or(body),
                icon: look.icon,
                why: "This appearance has no model and paints nothing on the body.",
            }
        }
        _ => Shown::default(),
    }
}

/// The inventory type an `ItemDisplayInfo` row is drawn as, which decides
/// which folder its model is found in and which geosets a body switches to.
/// The row does not say: an item names a display, not the other way round.
/// So it is the folder its model is in, or the body part it paints most
/// visibly.
fn item_slot(work: &mut Workspace<'_>, id: u32) -> u32 {
    const CHEST: u32 = 5;
    let Some(record) = work.browser.record_of(work.session, "ItemDisplayInfo", id) else {
        return CHEST;
    };
    let Some(rows) = work.session.table("ItemDisplayInfo") else {
        return CHEST;
    };
    let text = |field: usize| rows.string_at(record, field).filter(|text| !text.trim().is_empty());
    if let Some(name) = text(1) {
        return match item_model_path(work.assets, &name).map(|(folder, _)| folder) {
            Some("Head") => 1,
            Some("Shoulder") => 3,
            Some("Shield") => 14,
            _ => 21,
        };
    }
    let paints = |component: Component| text(14 + component.index()).is_some();
    if text(3).is_some() && !Component::ALL.into_iter().any(paints) {
        return 16;
    }
    match () {
        _ if paints(Component::TorsoUpper) || paints(Component::ArmUpper) => CHEST,
        _ if paints(Component::LegUpper) || paints(Component::LegLower) => 7,
        _ if paints(Component::Foot) => 8,
        _ if paints(Component::Hand) => 10,
        _ if paints(Component::ArmLower) => 9,
        _ if paints(Component::TorsoLower) => 6,
        _ => CHEST,
    }
}

/// The folders under `Item\ObjectComponents\` an item model can be in.
const ITEM_FOLDERS: [&str; 4] = ["Head", "Shoulder", "Weapon", "Shield"];

/// The folder an item display's model is in and the model's archive path,
/// from the bare name its row carries: the first of the four folders that
/// holds it. A helm is cut per race and is found as the human male's.
pub(super) fn item_model_path(assets: &GameAssets, name: &str) -> Option<(&'static str, String)> {
    let path = vale_assets::world::m2::model_path(name.trim());
    let stem = path.strip_suffix(".m2").unwrap_or(&path).to_string();
    assets
        .with_archive(|chain| {
            Ok(ITEM_FOLDERS.into_iter().find_map(|folder| {
                let file = match folder {
                    "Head" => format!("{stem}_hum.m2"),
                    _ => format!("{stem}.m2"),
                };
                let path = format!("Item\\ObjectComponents\\{folder}\\{file}");
                chain.exists(&path).then_some((folder, path))
            }))
        })
        .ok()
        .flatten()
}

/// What an `ItemDisplayInfo` model field is set to for the model at `path`:
/// its file name, as `.mdx` as the shipped rows write it, and for a helm,
/// without the race and gender a helm's files are cut for.
pub(super) fn item_model_name(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file.strip_suffix(".m2").unwrap_or(file);
    let lower = path.to_ascii_lowercase();
    let stem = match lower.starts_with("item\\objectcomponents\\head\\") {
        true => strip_race(stem),
        false => stem,
    };
    format!("{stem}.mdx")
}

/// A helm's file name without its `_hum`, `_orf` and the like.
fn strip_race(stem: &str) -> &str {
    let bytes = stem.as_bytes();
    let at = stem.len().saturating_sub(4);
    let cut = bytes.len() > 4
        && bytes[at] == b'_'
        && bytes[at + 1..at + 3].iter().all(u8::is_ascii_alphabetic)
        && matches!(bytes[at + 3].to_ascii_lowercase(), b'm' | b'f');
    match cut {
        true => &stem[..at],
        false => stem,
    }
}

/// The folder a `CreatureDisplayInfo` row's skins are found in: the folder of
/// the path its `CreatureModelData` row names, with its trailing backslash.
fn skin_folder(session: &EditSession, record: usize) -> Option<String> {
    let model_id = session.table("CreatureDisplayInfo")?.u32_at(record, 1)?;
    let models = session.table("CreatureModelData")?;
    let path = models.string_at(models.row_of(model_id)?, 2)?;
    let at = path.rfind(['\\', '/'])?;
    Some(path[..=at].replace('/', "\\"))
}

/// The folder the textures a field names are in, with its trailing
/// backslash. `None` when the row does not lead to one.
fn folder(
    session: &EditSession,
    assets: &GameAssets,
    table: &str,
    record: usize,
    of: Textures,
) -> Option<String> {
    match of {
        Textures::Skin => skin_folder(session, record),
        Textures::ItemSkin { model } => {
            let name = session
                .table(table)?
                .string_at(record, model)
                .filter(|name| !name.trim().is_empty());
            match name {
                Some(name) => item_model_path(assets, &name)
                    .map(|(folder, _)| format!("Item\\ObjectComponents\\{folder}\\")),
                None => Some("Item\\ObjectComponents\\Cape\\".to_string()),
            }
        }
        Textures::Icon => Some("Interface\\Icons\\".to_string()),
        Textures::Body(component) => {
            Some(format!("Item\\TextureComponents\\{}\\", component.directory()))
        }
    }
}

/// A texture's name from its path: the file name without `.blp`, and for a
/// body texture without the `_U`, `_M` or `_F` a field leaves off.
fn texture_name(path: &str, of: Textures) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let cut = file.len().saturating_sub(4);
    let stem = match file.get(cut..).is_some_and(|tail| tail.eq_ignore_ascii_case(".blp")) {
        true => &file[..cut],
        false => file,
    };
    match of {
        Textures::Body(_) => crate::import::assets::FieldTarget {
            table: String::new(),
            record: 0,
            field: 0,
            suffix: "_U",
        }
        .field_text(stem),
        _ => stem.to_string(),
    }
}

/// The texture a field's text names, as an archive path, for the thumbnail
/// beside it. A body texture is the first of its three files that exists.
fn texture_of(
    session: &EditSession,
    assets: &GameAssets,
    table: &str,
    record: usize,
    of: Textures,
    name: &str,
) -> Option<String> {
    let folder = folder(session, assets, table, record, of)?;
    match of {
        Textures::Body(_) => assets
            .with_archive(|chain| {
                Ok(["U", "M", "F"]
                    .into_iter()
                    .map(|end| format!("{folder}{name}_{end}.blp"))
                    .find(|path| chain.exists(path)))
            })
            .ok()
            .flatten(),
        _ => Some(format!("{folder}{name}.blp")),
    }
}

/// The textures the browser offers for a field, as each one's name and path:
/// the folder's own `.blp` files, not its subfolders'. A body texture's three
/// files are one name, shown by the first of `_U`, `_M` and `_F`.
fn listed(session: &EditSession, assets: &GameAssets, folder: &str, of: Textures) -> Vec<(String, String)> {
    let lower = folder.to_ascii_lowercase();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut files: Vec<String> = session
        .listed(assets, folder)
        .into_iter()
        .filter(|path| path.ends_with(".blp") && !path[lower.len()..].contains('\\'))
        .collect();
    // `_u` sorts after `_f` and `_m`; a body texture is shown by its `_u`.
    files.sort_by_key(|path| !path.ends_with("_u.blp"));
    // A name is written into a table field, so it is spelled as the
    // archives spell the file, as the shipped rows are.
    let spell = session.spellings(assets);
    for path in files {
        let name = texture_name(spell.of(&path), of);
        if !out.iter().any(|(had, _)| had.eq_ignore_ascii_case(&name)) {
            out.push((name, path));
        }
    }
    out.sort_by_key(|(name, _)| name.to_ascii_lowercase());
    out
}

/// What the browse button beside a texture field says.
fn browse_hint(of: Textures) -> &'static str {
    match of {
        Textures::Skin => "Choose a texture in the folder of the display's model, or import a \
                           picture there.",
        Textures::ItemSkin { .. } => "Choose a texture in the folder of the item's model, or \
                                      import a picture there.",
        Textures::Icon => "Choose an icon, or import a picture as one.",
        Textures::Body(_) => "Choose a texture for this part of the body, or import a picture \
                              as one.",
    }
}

/// A texture field's thumbnail and its browse button, beside the text box.
pub(super) fn texture_field(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table: &str,
    record: usize,
    field: usize,
    of: Textures,
    text: &str,
) {
    let name = text.trim();
    let path = match name.is_empty() {
        true => None,
        false => texture_of(work.session, work.assets, table, record, of, name),
    };
    if let Some(path) = path {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
        square(ui, work.thumbnails, &path, rect, false);
    }
    if ui.small_button("browse\u{2026}").on_hover_text(browse_hint(of)).clicked() {
        work.browser.modal = Some(Modal::Textures { record, field });
        work.browser.texture_preview = None;
        work.browser.texture_query.clear();
        work.browser.texture_page = 0;
        work.browser.pick_focus = true;
    }
}

/// A texture filling `rect`, from the thumbnail cache, over a checkerboard
/// when `checked` so alpha shows as alpha.
fn square(
    ui: &egui::Ui,
    thumbnails: &mut crate::ui::widgets::thumbnails::Thumbnails,
    path: &str,
    rect: egui::Rect,
    checked: bool,
) {
    // The grid's cells are drawn from the small decode, a little enlarged;
    // only a preview is worth the large one.
    let side = match rect.width() > 2.0 * crate::ui::widgets::thumbnails::ICON_SIDE as f32 {
        true => 256,
        false => crate::ui::widgets::thumbnails::ICON_SIDE,
    };
    thumbnails.want_at(path, side);
    match thumbnails.get_at(path, side) {
        Some(id) => {
            if checked {
                crate::ui::import::assets::checker(ui, rect);
            }
            ui.painter().image(
                id,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            ui.painter().rect_filled(rect, 3.0, theme::SUNK);
        }
    }
}

/// The texture browser's width: the grid, and the preview beside it.
const COLUMNS: usize = 5;
const GRID_WIDTH: f32 = COLUMNS as f32 * (CELL + 8.0);
const PREVIEW_WIDTH: f32 = 300.0;
/// One cell of the grid, in points, and how many a page holds.
const CELL: f32 = 76.0;
const PAGE: usize = 60;
/// The height of a cell's name under it.
const LABEL_HEIGHT: f32 = 18.0;
/// The grid's height: four rows of cells with their names, and a little.
const GRID_HEIGHT: f32 = 4.0 * (CELL + LABEL_HEIGHT + 8.0);

/// The texture browser, while it is open on the field at `(record, field)`
/// of `table`. Answers whether it closes.
pub(super) fn textures(ui: &mut egui::Ui, work: &mut Workspace<'_>, table: &str, record: usize, field: usize) -> bool {
    let Some(FileField::Texture(of)) = file_field(table, field) else {
        return true;
    };
    let column = vale_assets::tables::schema::for_table(table)
        .and_then(|schema| schema.column(field))
        .map(|column| column.name)
        .unwrap_or("Texture");
    let folder = folder(work.session, work.assets, table, record, of);
    // The folder's textures, listed again when the folder or the project's
    // files change.
    if let Some(folder) = folder.as_deref() {
        let revision = work.session.files_revision;
        let stale = work
            .browser
            .textures
            .as_ref()
            .is_none_or(|(had, at, _)| had != folder || *at != revision);
        if stale {
            let found = listed(work.session, work.assets, folder, of);
            work.browser.textures = Some((folder.to_string(), revision, found));
        }
    }
    // The dialog opens on the texture the field names.
    if work.browser.texture_preview.is_none() {
        let current = work
            .session
            .table(table)
            .and_then(|open| open.string_at(record, field))
            .filter(|name| !name.trim().is_empty());
        work.browser.texture_preview = current
            .and_then(|name| texture_of(work.session, work.assets, table, record, of, name.trim()));
    }
    let mut close = false;
    let mut picked: Option<String> = None;
    let response = egui::Modal::new(egui::Id::new("data-textures"))
        .backdrop_color(egui::Color32::from_black_alpha(90))
        .show(ui.ctx(), |ui| {
            ui.set_width(GRID_WIDTH + PREVIEW_WIDTH + 16.0);
            ui.label(egui::RichText::new(column).strong().size(14.0));
            let Some(folder) = folder.clone() else {
                theme::note(
                    ui,
                    match of {
                        Textures::Skin => "The display's ModelId names no CreatureModelData row \
                                           with a path, so there is no folder to look for skins in.",
                        _ => "The item's model is in no folder of Item\\ObjectComponents\\, so \
                              there is no folder to look for its skin in.",
                    },
                );
                if ui.button("Close").clicked() {
                    close = true;
                }
                return;
            };
            ui.label(egui::RichText::new(&folder).small().color(theme::INK_DIM));
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(GRID_WIDTH);
                    grid(ui, work, &mut picked);
                });
                ui.vertical(|ui| {
                    ui.set_width(PREVIEW_WIDTH);
                    preview(ui, work, table, record, field, of);
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let spell = work.session.spellings(work.assets);
                let chosen = work.browser.texture_preview.as_deref().map(|path| texture_name(spell.of(path), of));
                if ui
                    .add_enabled(
                        chosen.is_some(),
                        egui::Button::new(format!("Use {}", chosen.as_deref().unwrap_or("nothing chosen")))
                            .fill(theme::ACCENT),
                    )
                    .on_hover_text("Set the field to this texture's name.")
                    .clicked()
                {
                    picked = chosen;
                }
                if ui
                    .button("Import\u{2026}")
                    .on_hover_text(match of {
                        Textures::Body(_) => "Import a PNG, TGA or BLP picture into this folder \
                                              and set the field to it. It is written with _U, \
                                              for either body.",
                        _ => "Import a PNG, TGA or BLP picture into this folder and set the \
                              field to it.",
                    })
                    .clicked()
                {
                    work.asks.push(crate::ui::shell::asks::ShellAsk::ImportTexture {
                        folder: folder.clone(),
                        target: crate::import::assets::FieldTarget {
                            table: table.to_string(),
                            record,
                            field,
                            suffix: match of {
                                Textures::Body(_) => "_U",
                                _ => "",
                            },
                        },
                    });
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                ui.label(
                    egui::RichText::new("click previews \u{b7} double-click or Use chooses it")
                        .small()
                        .color(theme::INK_FAINT),
                );
            });
        });
    if let Some(name) = picked {
        let label = format!("Edit {column}");
        tables::fields::set_text(work.session, table, record, field, &name, &label, work.now);
        work.browser.forget_buffers();
        close = true;
    }
    if response.should_close() {
        close = true;
    }
    if close {
        work.browser.texture_preview = None;
    }
    close
}

/// The browser's search, pages and grid. A double click puts the texture's
/// name in `picked`.
fn grid(ui: &mut egui::Ui, work: &mut Workspace<'_>, picked: &mut Option<String>) {
    let search = ui.add(
        egui::TextEdit::singleline(&mut work.browser.texture_query)
            .hint_text("words in the name")
            .desired_width(GRID_WIDTH),
    );
    if search.changed() {
        work.browser.texture_page = 0;
    }
    if work.browser.pick_focus {
        search.request_focus();
        work.browser.pick_focus = false;
    }
    let words: Vec<String> = work
        .browser
        .texture_query
        .to_ascii_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let hits: Vec<(String, String)> = work
        .browser
        .textures
        .as_ref()
        .map(|(_, _, found)| {
            found
                .iter()
                .filter(|(name, _)| {
                    let name = name.to_ascii_lowercase();
                    words.iter().all(|word| name.contains(word.as_str()))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if hits.is_empty() {
        theme::note(
            ui,
            match words.is_empty() {
                true => "No texture in this folder yet. Import one.",
                false => "No texture's name has those words.",
            },
        );
    }
    let range = crate::ui::widgets::models::pager(ui, &mut work.browser.texture_page, hits.len(), PAGE);
    let page = work.browser.texture_page;
    egui::ScrollArea::vertical()
        .id_salt(("data-textures-grid", page))
        .max_height(GRID_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            // Rows of a fixed number of cells, each cell a fixed size. A
            // wrapped layout inside the scroll area did not wrap, and a
            // folder of hundreds of textures widened the dialog past the
            // window.
            for row in hits[range].chunks(COLUMNS) {
                ui.horizontal(|ui| {
                    for (name, path) in row {
                        // The listing is lower case; the field's own path is
                        // as the row spells it.
                        let lit = work
                            .browser
                            .texture_preview
                            .as_deref()
                            .is_some_and(|previewed| previewed.eq_ignore_ascii_case(path));
                        let response = ui
                            .allocate_ui_with_layout(
                                egui::vec2(CELL, CELL + LABEL_HEIGHT),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_max_width(CELL);
                                    let (rect, response) = ui
                                        .allocate_exact_size(egui::vec2(CELL, CELL), egui::Sense::click());
                                    square(ui, work.thumbnails, path, rect, false);
                                    if lit {
                                        ui.painter().rect_stroke(
                                            rect,
                                            3.0,
                                            egui::Stroke::new(2.0, theme::ACCENT),
                                            egui::StrokeKind::Outside,
                                        );
                                    }
                                    ui.add(egui::Label::new(egui::RichText::new(name).small()).truncate());
                                    response.on_hover_text(name)
                                },
                            )
                            .inner;
                        if response.double_clicked() {
                            *picked = Some(name.clone());
                        } else if response.clicked() {
                            work.browser.texture_preview = Some(path.clone());
                        }
                    }
                });
            }
        });
}

/// The browser's preview of the selected texture: for a skin, the model
/// wearing it; and the texture itself, over a checkerboard.
fn preview(ui: &mut egui::Ui, work: &mut Workspace<'_>, table: &str, record: usize, field: usize, of: Textures) {
    let Some(path) = work.browser.texture_preview.clone() else {
        theme::note(ui, "Click a texture to preview it.");
        return;
    };
    let worn = match of {
        Textures::Skin => skin_worn(work, record, field, &path),
        Textures::ItemSkin { model } => work
            .session
            .table(table)
            .and_then(|rows| rows.string_at(record, model))
            .filter(|name| !name.trim().is_empty())
            .and_then(|name| item_model_path(work.assets, &name))
            .map(|(_, model)| item_worn(model, path.clone())),
        Textures::Icon | Textures::Body(_) => None,
    };
    let texture_side = match worn {
        Some(_) => 120.0,
        None => PREVIEW_WIDTH,
    };
    if let Some(worn) = worn {
        let key = work.portraits.want_large_worn(&worn);
        crate::ui::widgets::preview::preview_pane_at(ui, work.portraits, &key, PREVIEW_WIDTH - 40.0);
    }
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(texture_side, texture_side), egui::Sense::hover());
    square(ui, work.thumbnails, &path, rect, true);
    ui.add(egui::Label::new(egui::RichText::new(&path).small().color(theme::INK_DIM)).wrap());
}

/// The portrait key of the item model at `path` wearing the skin the
/// `ItemDisplayInfo` row at `record` names for the model field `field`, found
/// in the model's folder. The bare path where the row names no skin.
pub(super) fn item_model_key(
    work: &mut Workspace<'_>,
    table: &str,
    record: usize,
    field: usize,
    path: &str,
) -> String {
    let skin = work
        .session
        .table(table)
        .and_then(|rows| rows.string_at(record, field + 2))
        .filter(|name| !name.trim().is_empty());
    let folder = path.rfind('\\').map(|at| &path[..=at]);
    match (skin, folder) {
        (Some(skin), Some(folder)) => {
            let worn = item_worn(path.to_string(), format!("{folder}{}.blp", skin.trim()));
            work.portraits.want_large_worn(&worn)
        }
        _ => path.to_string(),
    }
}

/// The display at `record` with the skin at `path` in `field`'s slot: as the
/// client's tables dress it, or, for a row they do not hold yet, as the
/// project's tables name it.
fn skin_worn(work: &mut Workspace<'_>, record: usize, field: usize, path: &str) -> Option<Worn> {
    let displays = work.session.table("CreatureDisplayInfo")?;
    let id = displays.u32_at(record, 0)?;
    let folder = skin_folder(work.session, record)?;
    let model_id = displays.u32_at(record, 1)?;
    let skins: Vec<String> = (6..=8)
        .map(|at| {
            displays
                .string_at(record, at)
                .filter(|name| !name.trim().is_empty())
                .map(|name| format!("{folder}{name}.blp"))
                .unwrap_or_default()
        })
        .collect();
    let models = work.session.table("CreatureModelData")?;
    let model = models.string_at(models.row_of(model_id)?, 2)?;
    let mut worn = crate::subjects::displays::resolve_worn(work.assets, id).unwrap_or_else(|| Worn {
        skins: skins.clone(),
        ..plain(String::new())
    });
    worn.path = vale_assets::world::m2::model_path(&model);
    worn.skins.resize(3, String::new());
    worn.skins[field - 6] = path.to_string();
    Some(worn)
}

/// The picture of the row `id` of a display table: the model, as large as
/// `largest` allows, or why there is none, with an item's bag icon above it.
pub(super) fn pane(ui: &mut egui::Ui, work: &mut Workspace<'_>, table: &'static str, id: u32, largest: f32) {
    let shown = shown(work, table, id);
    if let Some(icon) = &shown.icon {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
            square(ui, work.thumbnails, icon, rect, false);
            ui.label(
                egui::RichText::new(icon.rsplit('\\').next().unwrap_or(icon))
                    .small()
                    .color(theme::INK_DIM),
            );
        });
    }
    match &shown.worn {
        Some(worn) => {
            let key = work.portraits.want_large_worn(worn);
            crate::ui::widgets::preview::preview_pane_at(ui, work.portraits, &key, largest);
            if shown.on_a_body {
                theme::note(
                    ui,
                    "This appearance has no model: it paints the wearer's body, so the preview \
                     draws it on a body.",
                );
            }
        }
        None => theme::note(ui, shown.why),
    }
}

/// The picture beside a display row in a picker's list: its model, or an
/// item's icon where the item has no model of its own.
pub(super) fn row_picture(work: &mut Workspace<'_>, table: &'static str, record: usize) -> Picture {
    let Some(id) = work.session.table(table).and_then(|rows| rows.u32_at(record, 0)) else {
        return Picture::Blank;
    };
    let shown = shown(work, table, id);
    match (&shown.worn, &shown.icon) {
        (Some(worn), _) if !shown.on_a_body => Picture::Model(work.portraits.want_worn(worn)),
        (_, Some(icon)) => Picture::Icon(icon.clone()),
        _ => Picture::Blank,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each display text field that names a file is offered the browser for
    /// it, and no other field is.
    #[test]
    fn each_file_field_has_its_browser() {
        assert_eq!(file_field("CreatureModelData", 2), Some(FileField::Model));
        assert_eq!(file_field("ItemDisplayInfo", 2), Some(FileField::ItemModel));
        assert_eq!(file_field("CreatureDisplayInfo", 7), Some(FileField::Texture(Textures::Skin)));
        assert_eq!(
            file_field("ItemDisplayInfo", 4),
            Some(FileField::Texture(Textures::ItemSkin { model: 2 }))
        );
        assert_eq!(file_field("ItemDisplayInfo", 5), Some(FileField::Texture(Textures::Icon)));
        assert_eq!(
            file_field("ItemDisplayInfo", 21),
            Some(FileField::Texture(Textures::Body(Component::Foot)))
        );
        assert_eq!(file_field("ItemDisplayInfo", 22), None);
        assert_eq!(file_field("Spell", 1), None);
    }

    /// An item model is named by its file alone, and a helm without the race
    /// its file was cut for.
    #[test]
    fn an_item_model_is_named_by_its_file() {
        assert_eq!(
            item_model_name("item\\objectcomponents\\weapon\\sword_1h_short_a_01.m2"),
            "sword_1h_short_a_01.mdx"
        );
        assert_eq!(
            item_model_name("item\\objectcomponents\\head\\helm_plate_d_04_hum.m2"),
            "helm_plate_d_04.mdx"
        );
        assert_eq!(
            item_model_name("item\\objectcomponents\\shoulder\\lshoulder_plate_a_01.m2"),
            "lshoulder_plate_a_01.mdx"
        );
    }

    /// A body texture is named without the suffix of the body it is cut for.
    #[test]
    fn a_body_texture_is_named_without_its_suffix() {
        let body = Textures::Body(Component::TorsoUpper);
        assert_eq!(
            texture_name("item\\texturecomponents\\torsouppertexture\\robe_chest_tu_u.blp", body),
            "robe_chest_tu"
        );
        assert_eq!(texture_name("interface\\icons\\inv_sword_04.blp", Textures::Icon), "inv_sword_04");
    }
}
