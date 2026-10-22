//! The Assets workspace: the files the project adds to the archives, and the
//! Import Texture dialog.
//!
//! ## Three parts, as the other workspaces have
//!
//! ```text
//! left       which file    every game file of the project but its tiles and
//!                          tables, filtered by sort and searched by path,
//!                          with Import
//! middle     what it is    its path, what it is used as, a texture's
//!                          dimensions and compression, and what can be done
//!                          to it
//! inspector  what it is like  the texture, or the model, drawn
//! ```
//!
//! The list is the project folder, so a file the lab bakes, a minimap the
//! tile bake writes and a texture imported here are listed alike. See
//! [`crate::tools::assets`] for what an import writes and why a file listed
//! here is in every picker.
//!
//! ## Sorts, not file types
//!
//! The list is filtered by what a file is to the game ([`Sort`]): a ground
//! texture, an icon, another texture, a model, a minimap picture. The sorts
//! offered are the ones the project has, so the filter grows with the kinds of
//! file the project adds rather than listing every kind there could be. A
//! ground texture's `_s` file is its pair's, not a file of its own, and is
//! shown on its plain file's row.
//!
//! Import takes a file and decides from what it is which dialog follows. A
//! picture is the only kind it takes now, and the texture's purpose is chosen
//! in that dialog, not by its folder.
//!
//! ## The import dialog is not the workspace's alone
//!
//! A display form's texture field opens the same dialog, in the folder its
//! file is read from, and the texture tool's panel opens it for a ground
//! texture. The dialog is drawn by the shell whatever the tool is
//! ([`import_dialog`]), and the workspace, the field and the panel each only
//! ask for it.

use bevy_egui::egui;

use super::listing;
use super::theme;
use crate::session::EditSession;
use crate::tools::assets::{
    gloss_of, Ask, Assets, Choice, Gloss, Import, Own, Purpose, GROUND_ROOT, OTHER_FOLDER,
};
use vale_assets::world::blp::Encoding;
use vale_edit::manifest::Kind;

/// Everything the workspace draws from.
pub struct Work<'a> {
    pub session: &'a mut EditSession,
    pub assets: &'a mut Assets,
    pub game: &'a vale_client::assets::GameAssets,
    pub thumbnails: &'a mut super::thumbnails::Thumbnails,
    pub portraits: &'a mut crate::portraits::Portraits,
}

/// How large the inspector draws a texture, at most.
const PICTURE: f32 = 256.0;

/// What a project file is to the game, as the list filters it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sort {
    Ground,
    Icon,
    Texture,
    Model,
    Minimap,
    Other,
}

impl Sort {
    const ALL: [Sort; 6] = [Sort::Ground, Sort::Icon, Sort::Texture, Sort::Model, Sort::Minimap, Sort::Other];

    fn of(own: &Own) -> Sort {
        match own.kind {
            Kind::Texture => match Purpose::of_path(&own.vpath) {
                Purpose::Ground => Sort::Ground,
                Purpose::Icon => Sort::Icon,
                Purpose::Field | Purpose::Other => Sort::Texture,
            },
            Kind::Model => Sort::Model,
            Kind::Minimap => Sort::Minimap,
            _ => Sort::Other,
        }
    }

    /// The filter's name for the sort.
    fn plural(self) -> &'static str {
        match self {
            Sort::Ground => "Ground",
            Sort::Icon => "Icons",
            Sort::Texture => "Textures",
            Sort::Model => "Models",
            Sort::Minimap => "Minimaps",
            Sort::Other => "Other",
        }
    }

    /// What one file of the sort is, in its row and its details.
    fn word(self) -> &'static str {
        match self {
            Sort::Ground => "ground texture",
            Sort::Icon => "icon",
            Sort::Texture => "texture",
            Sort::Model => "model",
            Sort::Minimap => "minimap picture",
            Sort::Other => "file",
        }
    }
}

/// Draw the workspace into the region the panels left.
pub fn draw(ui: &mut egui::Ui, mut work: Work<'_>) {
    listing::frame(ui, &mut work, "assets", list, details);
}

/// The file name of a path.
fn basename(vpath: &str) -> &str {
    vpath.rsplit(['\\', '/']).next().unwrap_or(vpath)
}

/// The folder of a path, with its trailing backslash.
fn folder(vpath: &str) -> &str {
    match vpath.rfind(['\\', '/']) {
        Some(at) => &vpath[..=at],
        None => "",
    }
}

/// A size in bytes as a person reads it.
fn size_words(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// The project's files as the list shows them: a ground texture's `_s` file
/// is left out when its plain file is in the project, since it is shown on
/// that file's row.
fn shown_files(files: &[Own]) -> Vec<Own> {
    let has = |vpath: &str| files.iter().any(|own| own.vpath.eq_ignore_ascii_case(vpath));
    files
        .iter()
        .filter(|own| {
            let lower = own.vpath.to_ascii_lowercase();
            let paired = Sort::of(own) == Sort::Ground
                && lower.ends_with("_s.blp")
                && has(&format!("{}.blp", &own.vpath[..own.vpath.len() - 6]));
            !paired
        })
        .cloned()
        .collect()
}

/// The left column: Import, the sorts the project has, the search and the
/// files.
fn list(ui: &mut egui::Ui, work: &mut Work<'_>) {
    let all: Vec<Own> = work.assets.files(work.session).to_vec();
    let files = shown_files(&all);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Assets").strong().size(15.0).color(theme::INK));
        ui.label(theme::number(match files.len() {
            1 => "1 file".to_string(),
            count => format!("{count} files"),
        }));
    })
    .response
    .on_hover_text(
        "The project's own game files other than tiles and tables: imported textures, \
         baked models, minimap pictures. Each is read in place of the archives' file at \
         its path, listed in the editor's pickers, and published with the project.",
    );
    ui.add_space(4.0);
    if ui
        .add_sized(egui::vec2(ui.available_width(), 24.0), egui::Button::new("Import\u{2026}"))
        .on_hover_text(
            "Choose a PNG, TGA or BLP picture. The next step asks what it is for: a ground \
             texture, an icon or another texture.",
        )
        .clicked()
    {
        work.assets.ask = Some(Ask::Choose {
            folder: OTHER_FOLDER.to_string(),
            name: None,
            for_field: None,
        });
    }
    ui.add_space(4.0);
    // The sorts the project has, and All. The choice is the viewer's own and
    // is kept in egui's memory.
    let filter_id = egui::Id::new("assets-filter");
    let mut filter: Option<Sort> = ui.data(|data| data.get_temp::<Option<Sort>>(filter_id)).flatten();
    let present: Vec<(Sort, usize)> = Sort::ALL
        .iter()
        .map(|sort| (*sort, files.iter().filter(|own| Sort::of(own) == *sort).count()))
        .filter(|(_, count)| *count > 0)
        .collect();
    if filter.is_some_and(|sort| !present.iter().any(|(had, _)| *had == sort)) {
        filter = None;
    }
    if present.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui
                .selectable_label(filter.is_none(), format!("All {}", files.len()))
                .clicked()
            {
                filter = None;
            }
            for (sort, count) in &present {
                if ui
                    .selectable_label(filter == Some(*sort), format!("{} {count}", sort.plural()))
                    .clicked()
                {
                    filter = Some(*sort);
                }
            }
        });
        ui.add_space(4.0);
    }
    ui.data_mut(|data| data.insert_temp(filter_id, filter));
    let width = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(&mut work.assets.query)
            .hint_text("words in the path")
            .desired_width(width),
    );
    let words: Vec<String> = work
        .assets
        .query
        .to_ascii_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let shown: Vec<&Own> = files
        .iter()
        .filter(|own| filter.is_none_or(|sort| Sort::of(own) == sort))
        .filter(|own| {
            let path = own.vpath.to_ascii_lowercase();
            words.iter().all(|word| path.contains(word.as_str()))
        })
        .collect();
    ui.label(
        egui::RichText::new(format!("{} shown", shown.len()))
            .small()
            .color(theme::INK_FAINT),
    );
    ui.add_space(4.0);
    if files.is_empty() {
        theme::note(ui, "The project adds no files yet. Import writes the first.");
        return;
    }
    let height = listing::ROW_HEIGHT;
    let mut clicked = None;
    theme::list_area(ui, None, height).show_rows(ui, height, shown.len(), |ui, range| {
        for own in &shown[range] {
            let chosen = work.assets.selected.as_deref() == Some(own.vpath.as_str());
            let sort = Sort::of(own);
            // A ground texture says whether its pair is there, since without it
            // the client draws nothing. See `tools::assets`.
            let unpaired = gloss_of(&own.vpath).is_some_and(|gloss| !all.iter().any(|o| o.vpath.eq_ignore_ascii_case(&gloss)));
            let pair = match (sort, unpaired) {
                (Sort::Ground, false) => " \u{b7} with _s",
                (Sort::Ground, true) => " \u{b7} no _s file",
                _ => "",
            };
            let sub = format!(
                "{} \u{b7} {}{pair} \u{b7} {}",
                folder(&own.vpath).trim_end_matches('\\'),
                sort.word(),
                size_words(own.bytes)
            );
            let shape = theme::list_row(
                ui,
                theme::ListRow {
                    title: basename(&own.vpath),
                    sub: &sub,
                    trailing: "",
                    tint: match unpaired {
                        true => theme::WARN,
                        false => theme::INK,
                    },
                    picture: true,
                },
                chosen,
            );
            picture(ui, work, own, shape.picture);
            if shape.response.clicked() {
                clicked = Some(own.vpath.clone());
            }
        }
    });
    if clicked.is_some() {
        work.assets.selected = clicked;
    }
}

/// A row's picture: the texture itself, or the model's portrait.
fn picture(ui: &mut egui::Ui, work: &mut Work<'_>, own: &Own, rect: egui::Rect) {
    match own.kind {
        Kind::Texture | Kind::Minimap => {
            work.thumbnails.want(&own.vpath);
            match work.thumbnails.get(&own.vpath) {
                Some(id) => {
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
        Kind::Model => {
            let path = own.vpath.to_ascii_lowercase();
            work.portraits.want(&path);
            crate::portraits::paint(ui, work.portraits, &path, rect);
        }
        _ => {
            ui.painter().rect_filled(rect, 3.0, theme::LINE);
        }
    }
}

/// A texture file's header, as the details read it.
struct Header {
    width: u32,
    height: u32,
    words: String,
    mips: bool,
}

/// The header of a BLP's bytes.
fn header(bytes: &[u8]) -> Option<Header> {
    if bytes.len() < 0x14 || &bytes[0..4] != b"BLP2" {
        return None;
    }
    let width = u32::from_le_bytes([bytes[0x0C], bytes[0x0D], bytes[0x0E], bytes[0x0F]]);
    let height = u32::from_le_bytes([bytes[0x10], bytes[0x11], bytes[0x12], bytes[0x13]]);
    let words = match (bytes[0x08], bytes[0x09], bytes[0x0A]) {
        (1, depth, _) => format!("palettized, {depth}-bit alpha"),
        (2, 0, _) => Encoding::Dxt1.name().to_string(),
        (2, 1, _) => Encoding::Dxt1Alpha.name().to_string(),
        (2, _, 7) => "DXT5".to_string(),
        (2, ..) => Encoding::Dxt3.name().to_string(),
        (3, ..) => "raw BGRA".to_string(),
        (other, ..) => format!("compression {other}"),
    };
    Some(Header {
        width,
        height,
        words,
        mips: bytes[0x0B] != 0,
    })
}

/// What a sort of file is used for, in the details.
fn used_as(sort: Sort) -> &'static str {
    match sort {
        Sort::Ground => "Listed in the texture tool's tilesets and painted on the terrain.",
        Sort::Icon => "Named by an item's display or a spell's icon row.",
        Sort::Texture => "Read wherever a model, a table or the interface names its path.",
        Sort::Model => "Listed in the placement tools and the model browser.",
        Sort::Minimap => "Drawn by the minimap and the map for its tile.",
        Sort::Other => "Read wherever the game asks for its path.",
    }
}

/// The middle: the selected file's path, what it is, what it holds, and what
/// can be done to it.
fn details(ui: &mut egui::Ui, work: &mut Work<'_>) {
    let Some(vpath) = work.assets.selected.clone() else {
        theme::heading(ui, "No file chosen");
        theme::note(
            ui,
            "Choose a file on the left, or import one. A file here is read in place of the \
             archives' file at its path and is published with the project.",
        );
        return;
    };
    let own = work
        .assets
        .files(work.session)
        .iter()
        .find(|own| own.vpath == vpath)
        .cloned();
    let Some(own) = own else {
        theme::heading(ui, "No file chosen");
        theme::note(ui, "The chosen file is no longer in the project.");
        return;
    };
    let sort = Sort::of(&own);
    // A ground texture's pair: its `_s` file, in the project or the archives.
    let gloss = gloss_of(&own.vpath);
    let gloss_own = gloss.as_ref().and_then(|gloss| {
        work.assets
            .files(work.session)
            .iter()
            .find(|other| other.vpath.eq_ignore_ascii_case(gloss))
            .cloned()
    });
    let gloss_shipped = gloss
        .as_ref()
        .is_some_and(|gloss| gloss_own.is_none() && in_archives(ui.ctx(), work.game, gloss));
    let unpaired = gloss.is_some() && gloss_own.is_none() && !gloss_shipped;

    ui.label(egui::RichText::new(basename(&own.vpath)).strong().size(17.0).color(theme::INK));
    ui.label(egui::RichText::new(&own.vpath).small().color(theme::INK_DIM));
    ui.add_space(6.0);
    egui::Grid::new("asset-details")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Type").color(theme::INK_DIM));
            let mut word = sort.word().to_string();
            word[..1].make_ascii_uppercase();
            ui.label(&word).on_hover_text(used_as(sort));
            ui.end_row();
            ui.label(egui::RichText::new("Size").color(theme::INK_DIM));
            ui.label(match &gloss_own {
                Some(pair) => format!("{}, and {} for its _s file", size_words(own.bytes), size_words(pair.bytes)),
                None => size_words(own.bytes),
            });
            ui.end_row();
            if matches!(own.kind, Kind::Texture | Kind::Minimap) {
                if let Some(head) = work.session.project.read(&own.vpath).and_then(|bytes| header(&bytes)) {
                    ui.label(egui::RichText::new("Dimensions").color(theme::INK_DIM));
                    ui.label(format!("{}x{}", head.width, head.height));
                    ui.end_row();
                    ui.label(egui::RichText::new("Compression").color(theme::INK_DIM));
                    ui.label(head.words);
                    ui.end_row();
                    ui.label(egui::RichText::new("Mipmaps").color(theme::INK_DIM));
                    ui.label(match head.mips {
                        true => "yes",
                        false => "no",
                    });
                    ui.end_row();
                }
            }
            if let Some(gloss) = &gloss {
                ui.label(egui::RichText::new("Gloss file").color(theme::INK_DIM));
                match (&gloss_own, gloss_shipped) {
                    (Some(_), _) => ui.label(basename(gloss)),
                    (None, true) => ui.label(format!("{} (the archives')", basename(gloss))),
                    (None, false) => ui.label(egui::RichText::new("missing").color(theme::BAD)),
                };
                ui.end_row();
            }
            let shipped = in_archives(ui.ctx(), work.game, &own.vpath);
            ui.label(egui::RichText::new("Archives").color(theme::INK_DIM));
            ui.label(match shipped {
                true => egui::RichText::new("replaces the archives' file at this path").color(theme::WARN),
                false => egui::RichText::new("a new path").color(theme::INK),
            });
            ui.end_row();
        });
    ui.add_space(4.0);
    ui.label(egui::RichText::new(used_as(sort)).small().color(theme::INK_DIM));
    if unpaired {
        ui.add_space(6.0);
        theme::note(
            ui,
            "This ground texture has no _s file. With its specular setting on, the 1.12 client \
             opens that file in place of this one and draws the ground as a missing texture. \
             Write Its _s File writes both again.",
        );
    }
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if unpaired
            && ui
                .button("Write Its _s File\u{2026}")
                .on_hover_text(
                    "Open the import dialog on this texture to write it again with its _s \
                     gloss file, at most 256x256.",
                )
                .clicked()
        {
            work.assets.ask = Some(Ask::Rewrite {
                vpath: own.vpath.clone(),
                purpose: None,
            });
        }
        if sort == Sort::Texture
            && ui
                .button("Use as Ground Texture\u{2026}")
                .on_hover_text(
                    "Open the import dialog on this texture to write a ground texture from it \
                     under Tileset\\, with its _s file. This file is kept.",
                )
                .clicked()
        {
            work.assets.ask = Some(Ask::Rewrite {
                vpath: own.vpath.clone(),
                purpose: Some(Purpose::Ground),
            });
        }
        if ui
            .button("Copy Path")
            .on_hover_text("The path as a table field or a model names it.")
            .clicked()
        {
            ui.ctx().copy_text(own.vpath.clone());
            work.session.status = format!("{} is on the clipboard", own.vpath);
        }
        if own.kind == Kind::Texture
            && ui
                .button("Replace\u{2026}")
                .on_hover_text("Import another picture at this path, under this name.")
                .clicked()
        {
            work.assets.ask = Some(Ask::Choose {
                folder: folder(&own.vpath).to_string(),
                name: Some(basename(&own.vpath).trim_end_matches(".blp").trim_end_matches(".BLP").to_string()),
                for_field: None,
            });
        }
        // Removal takes two presses: the file is deleted from the project
        // folder, which no undo puts back.
        let confirm_id = egui::Id::new(("asset-remove", own.vpath.clone()));
        let confirming = ui.data(|data| data.get_temp::<bool>(confirm_id).unwrap_or(false));
        match confirming {
            false => {
                if ui
                    .button(egui::RichText::new("Remove").color(theme::BAD))
                    .on_hover_text(match gloss_own {
                        Some(_) => {
                            "Delete the texture and its _s file from the project. The archives' \
                             files at these paths, if there are any, are read again. Undo does \
                             not restore them."
                        }
                        None => {
                            "Delete the file from the project. The archives' file at this path, \
                             if there is one, is read again. Undo does not restore it."
                        }
                    })
                    .clicked()
                {
                    ui.data_mut(|data| data.insert_temp(confirm_id, true));
                }
            }
            true => {
                if ui
                    .button(egui::RichText::new("Confirm Remove").color(theme::BAD))
                    .clicked()
                {
                    work.assets.ask = Some(Ask::Remove(own.vpath.clone()));
                    ui.data_mut(|data| data.remove_temp::<bool>(confirm_id));
                }
                if ui.button("Cancel").clicked() {
                    ui.data_mut(|data| data.remove_temp::<bool>(confirm_id));
                }
            }
        }
    });
}

/// The workspace's third part, in the shell's inspector: the chosen file,
/// drawn.
pub fn inspector(ui: &mut egui::Ui, mut work: Work<'_>) {
    let work = &mut work;
    let Some(vpath) = work.assets.selected.clone() else {
        theme::note(ui, "Choose a file to see it.");
        return;
    };
    ui.label(egui::RichText::new(basename(&vpath)).strong().size(14.0));
    match Kind::of(&vpath) {
        Kind::Texture | Kind::Minimap => {
            let side = PICTURE as u32;
            work.thumbnails.want_at(&vpath, side);
            let width = ui.available_width().min(PICTURE);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, width), egui::Sense::hover());
            match work.thumbnails.get_at(&vpath, side) {
                Some(id) => {
                    // A checkerboard behind it, so alpha shows as alpha.
                    checker(ui, rect);
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
        Kind::Model => {
            let path = vpath.to_ascii_lowercase();
            let side = ui.available_width();
            super::inspector::preview_pane_at(ui, work.portraits, &path, side);
        }
        _ => theme::note(ui, "No picture for this kind of file."),
    }
}

/// Whether the archives, read past the project, hold a file at `vpath`. A
/// whole read, so the answer is kept per path in egui's memory: the dialog
/// asks every frame and the path changes only as it is typed.
fn in_archives(ctx: &egui::Context, game: &vale_client::assets::GameAssets, vpath: &str) -> bool {
    let key = egui::Id::new(("asset-in-archives", vpath.to_ascii_lowercase()));
    if let Some(known) = ctx.data(|data| data.get_temp::<bool>(key)) {
        return known;
    }
    let found = game
        .with_archive(|chain| Ok(chain.read_past_overlay(vpath).is_ok()))
        .unwrap_or(false);
    ctx.data_mut(|data| data.insert_temp(key, found));
    found
}

/// A grey checkerboard filling `rect`, under a texture with alpha.
pub(super) fn checker(ui: &egui::Ui, rect: egui::Rect) {
    const CELL: f32 = 12.0;
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(60));
    let (columns, rows) = ((rect.width() / CELL).ceil() as usize, (rect.height() / CELL).ceil() as usize);
    for row in 0..rows {
        for column in (row % 2..columns).step_by(2) {
            let min = rect.min + egui::vec2(column as f32 * CELL, row as f32 * CELL);
            let cell = egui::Rect::from_min_size(min, egui::vec2(CELL, CELL)).intersect(rect);
            painter.rect_filled(cell, 0.0, egui::Color32::from_gray(90));
        }
    }
}

/// The Import Texture dialog, when one is open. Drawn by the shell under any
/// tool, since a form field and the texture tool's panel open it too.
///
/// The first choice is what the texture is for ([`Purpose`]); the folder, the
/// size and the files written follow from it, and the rows under it are only
/// the ones that purpose has.
pub fn import_dialog(ctx: &egui::Context, assets: &mut Assets, session: &EditSession, game: &vale_client::assets::GameAssets) {
    let Some(import) = assets.import.as_mut() else { return };
    let mut close = false;
    let mut write = false;
    let response = egui::Modal::new(egui::Id::new("import-texture")).show(ctx, |ui| {
        ui.set_width(580.0);
        ui.label(egui::RichText::new("Import Texture").strong().size(15.0));
        ui.label(
            egui::RichText::new(import.source.display().to_string())
                .small()
                .color(theme::INK_DIM),
        );
        ui.add_space(6.0);
        // What it is for, first: everything under it follows from this. A
        // field's import is for its field and offers no other purpose.
        match &import.for_field {
            Some(target) => {
                let column = vale_assets::tables::schema::for_table(&target.table)
                    .and_then(|schema| schema.column(target.field))
                    .map(|column| column.name)
                    .unwrap_or("the field");
                ui.label(format!("For {} {column}", target.table));
            }
            None => {
                let mut purpose = import.purpose;
                let options: Vec<(&str, Purpose)> =
                    Purpose::OFFERED.iter().map(|purpose| (purpose.name(), *purpose)).collect();
                ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
                    theme::segmented(ui, &mut purpose, &options, |a, b| a == b);
                });
                if purpose != import.purpose {
                    import.set_purpose(purpose);
                }
            }
        }
        ui.label(egui::RichText::new(import.purpose.hint()).small().color(theme::INK_DIM));
        ui.add_space(6.0);
        ui.horizontal_top(|ui| {
            source_picture(ui, import);
            ui.vertical(|ui| {
                let alpha = match Encoding::for_pixels(&import.rgba) {
                    Encoding::Dxt1 => "opaque",
                    Encoding::Dxt1Alpha => "alpha on or off",
                    Encoding::Dxt3 => "graded alpha",
                };
                ui.label(format!("{}x{}, {alpha}", import.width, import.height));
                ui.add_space(4.0);
                egui::Grid::new("import-texture-form")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| form(ui, import));
            });
        });
        ui.add_space(6.0);
        let (width, height) = import.written_size();
        let mips = match import.mips {
            true => ", with mips",
            false => "",
        };
        let vpath = import.vpath();
        match import.gloss_vpath() {
            Some(gloss) => {
                ui.label(format!("Writes {vpath} (DXT1) and {gloss} (DXT3): {width}x{height}{mips}"));
            }
            None => {
                ui.label(format!("Writes {vpath}: {width}x{height}, {}{mips}", import.encoding().name()));
            }
        }
        if session.project.has(&vpath) {
            ui.label(
                egui::RichText::new("Replaces the project's file at this path.")
                    .small()
                    .color(theme::WARN),
            );
        } else if in_archives(ui.ctx(), game, &vpath) {
            ui.label(
                egui::RichText::new("Replaces the archives' file at this path, wherever it is used.")
                    .small()
                    .color(theme::WARN),
            );
        }
        if import.for_field.is_some() {
            ui.label(
                egui::RichText::new("The field that asked is set to this name once it is written.")
                    .small()
                    .color(theme::INK_DIM),
            );
        }
        let problems = import.problems();
        for problem in &problems {
            ui.label(egui::RichText::new(problem).small().color(theme::BAD));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(problems.is_empty(), egui::Button::new("Import").fill(theme::ACCENT))
                .clicked()
            {
                write = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if response.should_close() {
        close = true;
    }
    if write {
        assets.ask = Some(Ask::Import);
    } else if close {
        assets.import = None;
    }
}

/// The dialog's rows: where, under what name, and the choices the purpose has.
fn form(ui: &mut egui::Ui, import: &mut Import) {
    ui.label(egui::RichText::new("Folder").color(theme::INK_DIM));
    match import.purpose {
        // Only the folder under `Tileset\` is typed, so a ground texture
        // cannot be put where the texture tool does not look.
        Purpose::Ground => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(egui::RichText::new(GROUND_ROOT).color(theme::INK_DIM));
                let root = GROUND_ROOT.len();
                let mut under = import
                    .folder
                    .get(root..)
                    .unwrap_or("")
                    .trim_end_matches('\\')
                    .to_string();
                let typed = ui.add(
                    egui::TextEdit::singleline(&mut under)
                        .desired_width(160.0)
                        .hint_text("Custom"),
                );
                if typed.changed() {
                    let under = under.trim().trim_matches(['\\', '/']);
                    import.folder = match under.is_empty() {
                        true => GROUND_ROOT.to_string(),
                        false => format!("{GROUND_ROOT}{under}\\"),
                    };
                }
            });
        }
        // An icon's folder is the one every icon is in, and a field's is the
        // one its file is read from: neither is typed.
        Purpose::Icon | Purpose::Field => {
            ui.label(egui::RichText::new(&import.folder).color(theme::INK_DIM));
        }
        Purpose::Other => {
            ui.add(egui::TextEdit::singleline(&mut import.folder).desired_width(220.0));
        }
    }
    ui.end_row();
    ui.label(egui::RichText::new("Name").color(theme::INK_DIM));
    ui.add(egui::TextEdit::singleline(&mut import.name).desired_width(220.0));
    ui.end_row();
    ui.label(egui::RichText::new("Compression").color(theme::INK_DIM));
    if import.ground() {
        ui.label("DXT1, and DXT3 for its _s file").on_hover_text(
            "As every shipped ground texture is: the plain file has no alpha, and the _s file \
             carries the gloss in its alpha.",
        );
    } else {
        let shown = match import.choice {
            Choice::Auto => format!("Automatic: {}", Encoding::for_pixels(&import.rgba).name()),
            Choice::Fixed(encoding) => encoding.name().to_string(),
        };
        egui::ComboBox::from_id_salt("import-texture-compression")
            .selected_text(shown)
            .width(220.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut import.choice, Choice::Auto, "Automatic")
                    .on_hover_text("The least that keeps the picture's alpha.");
                for encoding in [Encoding::Dxt1, Encoding::Dxt1Alpha, Encoding::Dxt3] {
                    ui.selectable_value(&mut import.choice, Choice::Fixed(encoding), encoding.name());
                }
            });
    }
    ui.end_row();
    ui.label(egui::RichText::new("Mipmaps").color(theme::INK_DIM));
    ui.checkbox(&mut import.mips, "Write the mip chain").on_hover_text(
        "Every smaller level down to 1x1. Without them a texture shimmers when drawn small. \
         Most shipped textures have them.",
    );
    ui.end_row();
    let odd = !import.width.is_power_of_two() || !import.height.is_power_of_two();
    if odd {
        ui.label(egui::RichText::new("Resize").color(theme::INK_DIM));
        ui.checkbox(&mut import.resize, "Up to powers of two")
            .on_hover_text("Each side up to the next power of two, by bilinear filtering.");
        ui.end_row();
    }
    // The purpose's size, where it has one and the picture is larger.
    if let Some(side) = import.purpose.largest() {
        if import.width.max(import.height) > side {
            ui.label(egui::RichText::new("Size").color(theme::INK_DIM));
            ui.checkbox(&mut import.shrink, format!("Down to {side}x{side}"))
                .on_hover_text(match import.purpose {
                    Purpose::Icon => "Every icon the game ships is 64x64, and the interface draws them at that size.",
                    _ => "No ground texture the game ships is larger than 256x256, and a larger one is \
                          not known to draw in the 1.12 client.",
                });
            ui.end_row();
        }
    }
    if import.ground() {
        ui.label(egui::RichText::new("Gloss").color(theme::INK_DIM));
        ui.horizontal(|ui| {
            ui.radio_value(&mut import.gloss, Gloss::Matte, "Matte")
                .on_hover_text("No highlight from the sun on this ground.");
            ui.radio_value(&mut import.gloss, Gloss::FromAlpha, "From the alpha").on_hover_text(
                "The picture's alpha is the gloss: where it is high, the sun's highlight shows. \
                 Shipped ground averages about 10%.",
            );
        });
        ui.end_row();
    }
}

/// The picture being imported, as it was read, scaled into a square.
fn source_picture(ui: &mut egui::Ui, import: &Import) {
    const SIDE: f32 = 160.0;
    let key = egui::Id::new(("import-texture-picture", import.source.clone()));
    let handle = ui.data(|data| data.get_temp::<egui::TextureHandle>(key));
    let handle = match handle {
        Some(handle) => handle,
        None => {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [import.width as usize, import.height as usize],
                &import.rgba,
            );
            let handle = ui.ctx().load_texture("import-texture-picture", image, Default::default());
            ui.data_mut(|data| data.insert_temp(key, handle.clone()));
            handle
        }
    };
    let scale = SIDE / import.width.max(import.height).max(1) as f32;
    let size = egui::vec2(import.width as f32 * scale, import.height as f32 * scale);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(SIDE, SIDE), egui::Sense::hover());
    let at = egui::Rect::from_center_size(rect.center(), size);
    checker(ui, at);
    ui.painter().image(
        handle.id(),
        at,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::blp;

    /// A header reads back as the shape the encoder wrote.
    #[test]
    fn a_written_header_reads_back() {
        let rgba = vec![200u8; 16 * 16 * 4];
        let file = blp::encode(&rgba, 16, 16, Encoding::Dxt3, true).expect("encoded");
        let head = header(&file).expect("a header");
        assert_eq!((head.width, head.height), (16, 16));
        assert_eq!(head.words, "DXT3");
        assert!(head.mips);
        assert_eq!(basename("Tileset\\Custom\\Grass.blp"), "Grass.blp");
        assert_eq!(folder("Tileset\\Custom\\Grass.blp"), "Tileset\\Custom\\");
    }

    /// A ground texture's `_s` file is shown on its plain file's row, and an
    /// `_s` file whose plain file is not in the project is listed itself.
    #[test]
    fn a_gloss_file_is_shown_with_its_pair() {
        let own = |vpath: &str| Own {
            vpath: vpath.to_string(),
            kind: Kind::Texture,
            bytes: 1,
        };
        let files = vec![
            own("Tileset\\Custom\\Grass.blp"),
            own("Tileset\\Custom\\Grass_s.blp"),
            own("Tileset\\Custom\\Moss_s.blp"),
            own("Interface\\Icons\\Sword.blp"),
        ];
        let shown: Vec<String> = shown_files(&files).into_iter().map(|own| own.vpath).collect();
        assert_eq!(
            shown,
            ["Tileset\\Custom\\Grass.blp", "Tileset\\Custom\\Moss_s.blp", "Interface\\Icons\\Sword.blp"]
        );
        assert_eq!(Sort::of(&files[0]), Sort::Ground);
        assert_eq!(Sort::of(&files[3]), Sort::Icon);
    }
}
