//! The Assets workspace: the files the project adds to the archives, what
//! made each one, and the Import Texture dialog.
//!
//! ## The workspace's three parts, as in the other workspaces
//!
//! ```text
//! left       which file    every game file of the project but its tiles and
//!                          tables, filtered by where it came from and by
//!                          sort, searched by path, with Import and what has
//!                          fallen behind
//! middle     what it is    its path, what it is used as, what made it and
//!                          from what, a texture's dimensions and compression,
//!                          a model's skeleton and sequences, and what can be
//!                          done to it
//! inspector  what it is like  the texture, or the model, drawn
//! ```
//!
//! The list is the project folder, so a file the lab bakes, a minimap the
//! tile bake writes and a texture imported here are listed alike. See
//! [`crate::import::assets`] for what an import writes and why a file listed
//! here is in every picker.
//!
//! ## Imported and generated files
//!
//! A file is imported when the import record has it, and generated when one
//! of the editor's generators wrote it ([`crate::import::generated`]). The
//! list is filtered by the two. A generated file says what it was made from
//! and whether that changed after it was made, and offers to make it again: a
//! minimap picture is drawn again from its tile, a baked skin is baked again
//! from the rows that name it, a flight map is drawn again from the minimap
//! pictures. A generated file nothing names is unused and can be removed. An
//! imported file says what it was imported from, and Re-import opens the
//! dialog on that file with the choices it was imported with.
//!
//! With no file chosen, the middle lists what has fallen behind across the
//! project, each with the action that brings it up to date: tiles changed
//! after their minimap was drawn, appearances changed after their skin was
//! baked, and generated files nothing names.
//!
//! ## Filtering by sort: what a file is to the game, not its format
//!
//! Inside each, the list is filtered by what a file is to the game ([`Sort`]):
//! a ground texture, an icon, another texture, a model, a minimap picture, a
//! baked skin. The sorts offered are the ones the project has, so the filter
//! grows with the sorts of file the project adds rather than listing every
//! sort there could be. A ground texture's `_s` file belongs to its pair and
//! is shown on its plain file's row, not as a file of its own.
//!
//! Import takes a picture or a glTF model and opens the dialog for what it
//! is. A texture's purpose is chosen in its dialog, not by its folder, and so
//! is a model's ([`super::models`]).
//!
//! ## What else opens the import dialogs
//!
//! A display form's texture field opens the texture dialog, in the folder its
//! file is read from, and the texture tool's panel opens it for a ground
//! texture. The shell draws both dialogs whatever the tool is
//! ([`import_dialog`]); the workspace, the field and the panel only ask for
//! them.

use std::sync::Arc;

use bevy_egui::egui;

use crate::import::assets::{
    gloss_of, Ask, Assets, Choice, Gloss, Import, Own, Purpose, GROUND_ROOT, OTHER_FOLDER,
};
use crate::import::generated::{flight_map_of, Generator, Standing, Survey};
use crate::import::sources::Sources;
use crate::session::EditSession;
use crate::ui::subjects::listing;
use crate::ui::widgets::theme;
use vale_assets::world::blp::Encoding;
use vale_edit::manifest::Kind;

/// Everything the workspace draws from.
pub(crate) struct Work<'a> {
    pub(crate) session: &'a mut EditSession,
    pub(crate) assets: &'a mut Assets,
    pub(crate) game: &'a vale_client::assets::GameAssets,
    pub(crate) thumbnails: &'a mut crate::ui::widgets::thumbnails::Thumbnails,
    pub(crate) portraits: &'a mut crate::preview::portraits::Portraits,
    /// The map window's state, which draws minimap pictures.
    pub(crate) tiles: &'a mut crate::map::tiles::Tiles,
    /// The flight path tool's state, which draws flight map pictures.
    pub(crate) flights: &'a mut crate::tools::places::flightpaths::Flightpaths,
    /// The table browser, told when a rebake changes a row.
    pub(crate) browser: &'a mut crate::tables::browser::Browser,
    pub(crate) now: f64,
}

/// How large the inspector draws a texture, at most.
const PICTURE: f32 = 256.0;

/// Where a project file came from, as the list filters it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    All,
    Imported,
    Generated,
}

/// What a project file is to the game, as the list filters it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sort {
    Ground,
    Icon,
    Texture,
    Model,
    Minimap,
    MinimapIndex,
    NpcSkin,
    FlightMap,
    BakedModel,
    Other,
}

impl Sort {
    const ALL: [Sort; 10] = [
        Sort::Ground,
        Sort::Icon,
        Sort::Texture,
        Sort::Model,
        Sort::Minimap,
        Sort::MinimapIndex,
        Sort::NpcSkin,
        Sort::FlightMap,
        Sort::BakedModel,
        Sort::Other,
    ];

    /// The sort of a project file; `imported` says whether the import record
    /// has it, which is what tells an imported model from a baked one.
    fn of(own: &Own, imported: bool) -> Sort {
        match Generator::of(&own.vpath, imported) {
            Some(Generator::Minimap) => Sort::Minimap,
            Some(Generator::MinimapIndex) => Sort::MinimapIndex,
            Some(Generator::NpcSkin) => Sort::NpcSkin,
            Some(Generator::FlightMap) => Sort::FlightMap,
            Some(Generator::BakedModel) => Sort::BakedModel,
            None => match own.kind {
                Kind::Texture => match Purpose::of_path(&own.vpath) {
                    Purpose::Ground => Sort::Ground,
                    Purpose::Icon => Sort::Icon,
                    Purpose::Field | Purpose::Other => Sort::Texture,
                },
                Kind::Model => Sort::Model,
                _ => Sort::Other,
            },
        }
    }

    /// Whether the editor generates files of this sort.
    fn generated(self) -> bool {
        matches!(self, Sort::Minimap | Sort::MinimapIndex | Sort::NpcSkin | Sort::FlightMap | Sort::BakedModel)
    }

    /// The filter's name for the sort.
    fn plural(self) -> &'static str {
        match self {
            Sort::Ground => "Ground",
            Sort::Icon => "Icons",
            Sort::Texture => "Textures",
            Sort::Model => "Models",
            Sort::Minimap => "Minimaps",
            Sort::MinimapIndex => "Minimap index",
            Sort::NpcSkin => "NPC skins",
            Sort::FlightMap => "Flight maps",
            Sort::BakedModel => "Baked models",
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
            Sort::MinimapIndex => "minimap index",
            Sort::NpcSkin => "baked NPC skin",
            Sort::FlightMap => "flight map picture",
            Sort::BakedModel => "baked model",
            Sort::Other => "file",
        }
    }

    /// What made a generated file of the sort.
    fn made_by(self) -> Option<&'static str> {
        Some(match self {
            Sort::Minimap => "Drawn from its tile by the map window's Minimap button.",
            Sort::MinimapIndex => {
                "Written by the minimap drawing, with an entry for each tile the archives have no \
                 picture of."
            }
            Sort::NpcSkin => {
                "Baked by Rebake skin from a CreatureDisplayInfoExtra row's appearance and worn \
                 items. A rebake writes a new file and leaves the one before it unused."
            }
            Sort::FlightMap => "Drawn from the map's minimap pictures by the flight path tool.",
            Sort::BakedModel => {
                "Baked in the attachment lab: a shipped model with an offset, a turn or a scale \
                 in its vertices."
            }
            _ => return None,
        })
    }
}

/// Draw the workspace into the region the panels left.
pub(crate) fn draw(ui: &mut egui::Ui, mut work: Work<'_>) {
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
            let paired = Sort::of(own, false) == Sort::Ground
                && lower.ends_with("_s.blp")
                && has(&format!("{}.blp", &own.vpath[..own.vpath.len() - 6]));
            !paired
        })
        .cloned()
        .collect()
}

/// The survey of what has fallen behind, taken again when the project's files
/// or tables change. See [`crate::import::generated`].
fn survey(work: &mut Work<'_>, all: &[Own], sources: &Sources) -> Survey {
    work.assets.surveys.current(work.session, work.game, all, sources).clone()
}

/// What a row and the details say about a generated file: what it was made
/// from, and how it stands.
fn made_words(sort: Sort, made: Option<&crate::import::generated::Made>, vpath: &str) -> (String, Option<String>) {
    let Some(made) = made else {
        return (String::new(), None);
    };
    let from = match sort {
        Sort::Minimap => match &made.tile {
            Some((map, x, y)) => format!("tile {x}, {y} of {map}"),
            None => "no tile: md5translate.trs does not name it".to_string(),
        },
        Sort::NpcSkin => rows_words("CreatureDisplayInfoExtra", &made.rows),
        Sort::BakedModel => rows_words("SpellVisualEffectName", &made.rows),
        Sort::FlightMap => flight_map_of(vpath).map_or_else(String::new, |map| format!("the minimap pictures of map {map}")),
        _ => String::new(),
    };
    let standing = match &made.standing {
        Standing::Current => None,
        Standing::OutOfDate(why) => Some(format!("out of date: {why}")),
        Standing::Unused => Some(match sort {
            Sort::Minimap => "unused: no tile names it".to_string(),
            _ => "unused: no row names it".to_string(),
        }),
    };
    (from, standing)
}

fn rows_words(table: &str, rows: &[u32]) -> String {
    let ids: Vec<String> = rows.iter().take(6).map(u32::to_string).collect();
    match rows.len() {
        0 => format!("no {table} row"),
        n if n > 6 => format!("{table} {} and {} more", ids.join(", "), n - 6),
        _ => format!("{table} {}", ids.join(", ")),
    }
}

/// The left column: Import, where the files came from, the sorts the project
/// has, what has fallen behind, the search and the files.
fn list(ui: &mut egui::Ui, work: &mut Work<'_>) {
    let all: Vec<Own> = work.assets.files(work.session).to_vec();
    let sources = work.assets.sources(work.session).clone();
    let survey = survey(work, &all, &sources);
    let files = shown_files(&all);
    let sort_of = |own: &Own| Sort::of(own, sources.of(&own.vpath).is_some());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Assets").strong().size(15.0).color(theme::INK));
        ui.label(theme::number(match files.len() {
            1 => "1 file".to_string(),
            count => format!("{count} files"),
        }));
    })
    .response
    .on_hover_text(
        "The project's own game files other than tiles and tables: imported textures and \
         models, and the files the editor generates: minimap pictures, baked NPC skins, \
         baked models, flight maps. Each is read in place of the archives' file at its path, \
         listed in the editor's pickers, and published with the project.",
    );
    ui.add_space(4.0);
    if ui
        .add_sized(egui::vec2(ui.available_width(), 24.0), egui::Button::new("Import\u{2026}"))
        .on_hover_text(
            "Choose a PNG, TGA or BLP picture, or a glTF 2.0 model (.glb, .gltf). The next step \
             asks what it is for: a ground texture, an icon or another texture; a creature, a \
             doodad or another model.",
        )
        .clicked()
    {
        work.assets.ask = Some(Ask::Choose {
            folder: OTHER_FOLDER.to_string(),
            name: None,
            for_field: None,
            models: true,
        });
    }
    ui.add_space(4.0);

    // Where the files came from. The choice is the viewer's own and is kept
    // in egui's memory, as the sort's is.
    let origin_id = egui::Id::new("assets-origin");
    let mut origin: Origin = ui.data(|data| data.get_temp::<Origin>(origin_id)).unwrap_or(Origin::All);
    let generated = files.iter().filter(|own| sort_of(own).generated()).count();
    let imported = files.len() - generated;
    if generated > 0 && imported > 0 {
        let options = [
            (format!("All {}", files.len()), Origin::All),
            (format!("Imported {imported}"), Origin::Imported),
            (format!("Generated {generated}"), Origin::Generated),
        ];
        let options: Vec<(&str, Origin)> = options.iter().map(|(name, o)| (name.as_str(), *o)).collect();
        ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
            theme::segmented(ui, &mut origin, &options, |a, b| a == b);
        });
        ui.add_space(4.0);
    } else {
        origin = Origin::All;
    }
    ui.data_mut(|data| data.insert_temp(origin_id, origin));
    let in_origin = |sort: Sort| match origin {
        Origin::All => true,
        Origin::Imported => !sort.generated(),
        Origin::Generated => sort.generated(),
    };

    // The sorts the project has, and All.
    let filter_id = egui::Id::new("assets-filter");
    let mut filter: Option<Sort> = ui.data(|data| data.get_temp::<Option<Sort>>(filter_id)).flatten();
    let present: Vec<(Sort, usize)> = Sort::ALL
        .iter()
        .filter(|sort| in_origin(**sort))
        .map(|sort| (*sort, files.iter().filter(|own| sort_of(own) == *sort).count()))
        .filter(|(_, count)| *count > 0)
        .collect();
    if filter.is_some_and(|sort| !present.iter().any(|(had, _)| *had == sort)) {
        filter = None;
    }
    let in_view: usize = present.iter().map(|(_, count)| count).sum();
    if present.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui.selectable_label(filter.is_none(), format!("All {in_view}")).clicked() {
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

    if origin != Origin::Imported {
        behind_summary(ui, work, &survey);
    }

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
        .filter(|own| in_origin(sort_of(own)))
        .filter(|own| filter.is_none_or(|sort| sort_of(own) == sort))
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
            let sort = sort_of(own);
            // A ground texture says whether its pair is there, since without it
            // the client draws the ground as a missing texture. See
            // `crate::import::assets`.
            let unpaired = gloss_of(&own.vpath).is_some_and(|gloss| !all.iter().any(|o| o.vpath.eq_ignore_ascii_case(&gloss)));
            let (from, standing) = made_words(sort, survey.made.get(&own.vpath.to_ascii_lowercase()), &own.vpath);
            let pair = match (sort, unpaired) {
                (Sort::Ground, false) => " \u{b7} with _s",
                (Sort::Ground, true) => " \u{b7} no _s file",
                _ => "",
            };
            let about = match (&standing, from.is_empty()) {
                (Some(standing), _) => format!(" \u{b7} {standing}"),
                (None, false) => format!(" \u{b7} {from}"),
                (None, true) => String::new(),
            };
            let sub = format!(
                "{} \u{b7} {}{pair}{about} \u{b7} {}",
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
                    tint: match unpaired || standing.is_some() {
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

/// The lines of what has fallen behind, in the list column, each with its
/// action. Nothing when nothing has.
fn behind_summary(ui: &mut egui::Ui, work: &mut Work<'_>, survey: &Survey) {
    if survey.undrawn.is_empty() && survey.rebake.is_empty() && survey.unused.is_empty() {
        return;
    }
    egui::Frame::new()
        .fill(theme::SUNK)
        .corner_radius(4.0)
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new("Out of date").strong().color(theme::WARN));
            catch_up(ui, work, survey);
        });
    ui.add_space(6.0);
}

/// What has fallen behind across the project, and the action for each: draw
/// the minimaps of changed tiles, rebake the skins of changed appearances,
/// remove what nothing names.
fn catch_up(ui: &mut egui::Ui, work: &mut Work<'_>, survey: &Survey) {
    let open_map = work.session.map.clone();
    let here: Vec<(u32, u32)> = survey
        .undrawn
        .iter()
        .filter(|(map, ..)| map.eq_ignore_ascii_case(&open_map))
        .map(|(_, x, y)| (*x, *y))
        .collect();
    let elsewhere = survey.undrawn.len() - here.len();
    if !survey.undrawn.is_empty() {
        ui.label(match survey.undrawn.len() {
            1 => "1 tile changed after its minimap was drawn.".to_string(),
            n => format!("{n} tiles changed after their minimaps were drawn."),
        })
        .on_hover_text(
            "A tile saved after its picture in textures\\Minimap\\, or a tile the project changed \
             whose picture is still the archives'. The minimap and the flight map show the ground \
             as it was.",
        );
        ui.horizontal_wrapped(|ui| {
            if !here.is_empty() {
                let seconds = here.len() as f32 * crate::map::tiles::MINIMAP_SECONDS;
                if ui
                    .add_enabled(!work.tiles.drawing, egui::Button::new(format!("Draw {} on {open_map}", here.len())))
                    .on_hover_text(format!("Draws the minimaps of these tiles in the background, about {seconds:.0} s."))
                    .on_disabled_hover_text("Minimaps are being drawn.")
                    .clicked()
                {
                    work.tiles.redraw = here.clone();
                    work.session.status = format!("drawing {} minimaps", here.len());
                }
            }
            if elsewhere > 0 {
                ui.label(
                    egui::RichText::new(format!("{elsewhere} on other maps: open the map to draw them."))
                        .small()
                        .color(theme::INK_DIM),
                );
            }
        });
        ui.add_space(4.0);
    }
    if !survey.rebake.is_empty() {
        ui.label(match survey.rebake.len() {
            1 => "1 appearance changed after its skin was baked.".to_string(),
            n => format!("{n} appearances changed after their skins were baked."),
        })
        .on_hover_text(
            "CreatureDisplayInfoExtra rows whose race, face, hair, skin or worn items differ from \
             the archives' row while BakeName still names a skin not baked from the row. The \
             renderer draws the old skin until it is baked again.",
        );
        if ui
            .button(format!("Rebake {}", survey.rebake.len()))
            .on_hover_text("Bakes each row's skin again and points its BakeName at the new file. One undo entry per row.")
            .clicked()
        {
            let mut done = 0usize;
            let mut failed: Vec<String> = Vec::new();
            for &id in &survey.rebake {
                match crate::tables::bake::rebake(work.session, work.game, id, work.now) {
                    Ok(_) => done += 1,
                    Err(why) => failed.push(why),
                }
            }
            work.browser.forget_buffers();
            work.session.status = match failed.first() {
                None => format!("rebaked {done} skins"),
                Some(first) => format!("rebaked {done} skins; {} failed, first: {first}", failed.len()),
            };
        }
        ui.add_space(4.0);
    }
    if !survey.unused.is_empty() {
        ui.label(match survey.unused.len() {
            1 => "1 generated file is named by nothing.".to_string(),
            n => format!("{n} generated files are named by nothing."),
        })
        .on_hover_text(
            "Baked skins no CreatureDisplayInfoExtra row names, which is what the skin before a \
             rebake becomes, and baked models no SpellVisualEffectName row names. They are \
             published with the project all the same.",
        );
        // Removal takes two presses: the files are deleted from the project
        // folder, which no undo puts back.
        let confirm_id = egui::Id::new("assets-remove-unused");
        let confirming = ui.data(|data| data.get_temp::<bool>(confirm_id).unwrap_or(false));
        ui.horizontal_wrapped(|ui| match confirming {
            false => {
                if ui
                    .button(egui::RichText::new(format!("Remove {}", survey.unused.len())).color(theme::BAD))
                    .on_hover_text("Delete them from the project. Undo does not restore them.")
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
                    for vpath in &survey.unused {
                        work.thumbnails.forget(vpath);
                        work.session.remove_file(vpath);
                    }
                    work.session.status = format!("removed {} unused generated files", survey.unused.len());
                    if work.assets.selected.as_ref().is_some_and(|s| survey.unused.contains(s)) {
                        work.assets.selected = None;
                    }
                    work.assets.surveys.forget();
                    ui.data_mut(|data| data.remove_temp::<bool>(confirm_id));
                }
                if ui.button("Cancel").clicked() {
                    ui.data_mut(|data| data.remove_temp::<bool>(confirm_id));
                }
            }
        });
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
            let key = model_portrait(work, &own.vpath, false);
            crate::preview::portraits::paint(ui, work.portraits, &key, rect);
        }
        _ => {
            ui.painter().rect_filled(rect, 3.0, theme::LINE);
        }
    }
}

/// The portrait key a model is drawn under, for a row (`large` false) or the
/// inspector's pane. An imported creature model is drawn wearing the skins
/// its import wrote, since it names no file for them and draws magenta bare;
/// any other model by its path.
fn model_portrait(work: &mut Work<'_>, vpath: &str, large: bool) -> String {
    let path = vpath.to_ascii_lowercase();
    let skins = work
        .assets
        .sources(work.session)
        .of(vpath)
        .map(|entry| crate::import::models::imported_skins(entry, folder(vpath), &work.session.project))
        .unwrap_or_default();
    if skins.iter().all(String::is_empty) {
        if !large {
            work.portraits.want(&path);
        }
        return path;
    }
    let worn = crate::preview::portraits::Worn {
        path,
        skins,
        hair: None,
        dress: Default::default(),
        look: None,
        cloak: None,
        hung: Vec::new(),
    };
    match large {
        true => work.portraits.want_large_worn(&worn),
        false => work.portraits.want_worn(&worn),
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

/// What a model file holds, as the details list it.
#[derive(Debug, Clone)]
struct ModelFacts {
    vertices: usize,
    triangles: usize,
    bones: usize,
    /// Each texture's type and file name.
    textures: Vec<(u32, String)>,
    /// Each sequence's animation id, length in milliseconds and whether it
    /// loops.
    sequences: Vec<(u16, u32, bool)>,
}

/// The facts of the project's model at `vpath`, read once per change of the
/// project's files and kept in egui's memory.
fn model_facts(ui: &egui::Ui, session: &EditSession, vpath: &str) -> Option<Arc<ModelFacts>> {
    let key = egui::Id::new(("asset-model-facts", vpath.to_ascii_lowercase(), session.files_revision));
    if let Some(known) = ui.data(|data| data.get_temp::<Option<Arc<ModelFacts>>>(key)) {
        return known;
    }
    let facts = session.project.read(vpath).and_then(|bytes| vale_assets::world::m2::M2::parse(&bytes).ok()).map(|model| {
        let (bones, sequences) = match &model.skeleton {
            Some(skeleton) => (
                skeleton.bones.len(),
                skeleton.sequences.iter().map(|s| (s.id, s.end - s.start, s.flags & 1 == 0)).collect(),
            ),
            None => (0, Vec::new()),
        };
        Arc::new(ModelFacts {
            vertices: model.positions.len(),
            triangles: model.triangle_count(),
            bones,
            textures: model.textures.iter().map(|t| (t.kind, t.file_name.clone())).collect(),
            sequences,
        })
    });
    ui.data_mut(|data| data.insert_temp(key, facts.clone()));
    facts
}

/// What a sort of file is used for, in the details.
fn used_as(sort: Sort) -> &'static str {
    match sort {
        Sort::Ground => "Listed in the texture tool's tilesets and painted on the terrain.",
        Sort::Icon => "Named by an item's display or a spell's icon row.",
        Sort::Texture => "Read wherever a model, a table or the interface names its path.",
        Sort::Model => "Listed in the placement tools and the model browser.",
        Sort::Minimap => "Drawn by the minimap and the map for its tile.",
        Sort::MinimapIndex => "Read by the client to find each tile's minimap picture.",
        Sort::NpcSkin => "The body texture of the NPCs whose CreatureDisplayInfoExtra row names it as BakeName.",
        Sort::FlightMap => "Drawn behind the nodes of the map's flight map.",
        Sort::BakedModel => "The model of the spell effects whose SpellVisualEffectName row names it.",
        Sort::Other => "Read wherever the game asks for its path.",
    }
}

/// The middle: the selected file's path, what it is, what made it, what it
/// holds, and what can be done to it; with no file chosen, what has fallen
/// behind across the project.
fn details(ui: &mut egui::Ui, work: &mut Work<'_>) {
    let all: Vec<Own> = work.assets.files(work.session).to_vec();
    let sources = work.assets.sources(work.session).clone();
    let survey = survey(work, &all, &sources);
    let Some(vpath) = work.assets.selected.clone() else {
        theme::heading(ui, "No file chosen");
        theme::note(
            ui,
            "Choose a file on the left, or import one. A file here is read in place of the \
             archives' file at its path and is published with the project.",
        );
        ui.add_space(8.0);
        if survey.undrawn.is_empty() && survey.rebake.is_empty() && survey.unused.is_empty() {
            theme::note(ui, "Every generated file is up to date with what it was made from.");
        } else {
            theme::heading(ui, "Out of date");
            catch_up(ui, work, &survey);
        }
        return;
    };
    let Some(own) = all.iter().find(|own| own.vpath == vpath).cloned() else {
        theme::heading(ui, "No file chosen");
        theme::note(ui, "The chosen file is no longer in the project.");
        return;
    };
    let imported = sources.of(&own.vpath).cloned();
    let sort = Sort::of(&own, imported.is_some());
    let made = survey.made.get(&own.vpath.to_ascii_lowercase());
    let (from, standing) = made_words(sort, made, &own.vpath);
    // A ground texture's pair: its `_s` file, in the project or the archives.
    let gloss = gloss_of(&own.vpath);
    let gloss_own = gloss.as_ref().and_then(|gloss| {
        all.iter()
            .find(|other| other.vpath.eq_ignore_ascii_case(gloss))
            .cloned()
    });
    let gloss_shipped = gloss
        .as_ref()
        .is_some_and(|gloss| gloss_own.is_none() && in_archives(ui.ctx(), work.game, gloss));
    let unpaired = gloss.is_some() && gloss_own.is_none() && !gloss_shipped;
    let facts = (own.kind == Kind::Model).then(|| model_facts(ui, work.session, &own.vpath)).flatten();
    let tables = work.game.display_tables().ok();
    let animations = tables.as_deref().and_then(|t| t.animations());

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
            if let Some(entry) = &imported {
                ui.label(egui::RichText::new("Imported from").color(theme::INK_DIM));
                let there = entry.source.is_file();
                ui.label(match there {
                    true => egui::RichText::new(entry.source.display().to_string()).color(theme::INK),
                    false => egui::RichText::new(format!("{} (not found)", entry.source.display())).color(theme::WARN),
                });
                ui.end_row();
            }
            if sort.generated() && !from.is_empty() {
                ui.label(egui::RichText::new("Made from").color(theme::INK_DIM));
                ui.label(&from);
                ui.end_row();
            }
            if sort.generated() {
                ui.label(egui::RichText::new("Standing").color(theme::INK_DIM));
                match &standing {
                    None => ui.label(egui::RichText::new("up to date").color(theme::GOOD)),
                    Some(standing) => ui.label(egui::RichText::new(standing).color(theme::WARN)),
                };
                ui.end_row();
            }
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
            if let Some(facts) = &facts {
                ui.label(egui::RichText::new("Geometry").color(theme::INK_DIM));
                ui.label(format!("{} vertices, {} triangles", facts.vertices, facts.triangles));
                ui.end_row();
                ui.label(egui::RichText::new("Bones").color(theme::INK_DIM));
                ui.label(match facts.bones {
                    0 => "none animated".to_string(),
                    n => n.to_string(),
                });
                ui.end_row();
                ui.label(egui::RichText::new("Textures").color(theme::INK_DIM));
                let named: Vec<String> = facts
                    .textures
                    .iter()
                    .map(|(kind, file)| match kind {
                        0 => basename(file).to_string(),
                        11..=13 => format!("creature skin {}", kind - 10),
                        other => format!("type {other}"),
                    })
                    .collect();
                ui.label(named.join(", "));
                ui.end_row();
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
    if let Some(made_by) = sort.made_by() {
        ui.label(egui::RichText::new(made_by).small().color(theme::INK_DIM));
    }
    if let Some(facts) = facts.as_ref().filter(|f| !f.sequences.is_empty()) {
        ui.add_space(6.0);
        theme::heading(ui, "Sequences");
        egui::Grid::new("asset-sequences")
            .num_columns(3)
            .spacing([12.0, 2.0])
            .show(ui, |ui| {
                for (id, length, loops) in &facts.sequences {
                    let name = animations.and_then(|a| a.name(*id)).unwrap_or("?");
                    ui.label(format!("{id} {name}"));
                    ui.label(theme::number(format!("{length} ms")));
                    ui.label(
                        egui::RichText::new(match loops {
                            true => "loops",
                            false => "once",
                        })
                        .color(theme::INK_DIM),
                    );
                    ui.end_row();
                }
            });
    }
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
        made_again(ui, work, sort, made, &own.vpath);
        if let Some(entry) = &imported {
            if ui
                .add_enabled(entry.source.is_file(), egui::Button::new("Re-import\u{2026}"))
                .on_hover_text(format!(
                    "Open the import dialog on {} again, with the choices it was imported with.",
                    entry.source.display()
                ))
                .on_disabled_hover_text("The file it was imported from is no longer there.")
                .clicked()
            {
                work.assets.ask = Some(Ask::Reimport(own.vpath.clone()));
            }
        }
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
            && !sort.generated()
            && ui
                .button("Replace\u{2026}")
                .on_hover_text("Import another picture at this path, under this name.")
                .clicked()
        {
            work.assets.ask = Some(Ask::Choose {
                folder: folder(&own.vpath).to_string(),
                name: Some(basename(&own.vpath).trim_end_matches(".blp").trim_end_matches(".BLP").to_string()),
                for_field: None,
                models: false,
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

/// The button that makes a generated file again from what it was made from,
/// where its generator can be asked from here.
fn made_again(ui: &mut egui::Ui, work: &mut Work<'_>, sort: Sort, made: Option<&crate::import::generated::Made>, vpath: &str) {
    match sort {
        Sort::Minimap => {
            let Some((map, x, y)) = made.and_then(|m| m.tile.clone()) else { return };
            let here = map.eq_ignore_ascii_case(&work.session.map);
            if ui
                .add_enabled(here && !work.tiles.drawing, egui::Button::new("Draw Again"))
                .on_hover_text(format!("Draw this picture again from tile {x}, {y} in the background."))
                .on_disabled_hover_text(match here {
                    true => "Minimaps are being drawn.".to_string(),
                    false => format!("The tile is on {map}. Open that map to draw it again."),
                })
                .clicked()
            {
                work.tiles.redraw = vec![(x, y)];
                work.session.status = format!("drawing the minimap of tile {x}, {y}");
            }
        }
        Sort::NpcSkin => {
            let rows = made.map(|m| m.rows.clone()).unwrap_or_default();
            if ui
                .add_enabled(!rows.is_empty(), egui::Button::new("Rebake"))
                .on_hover_text(
                    "Bake the skin again from each row that names it, and point the row at the new \
                     file. A skin that comes out different is a new file, and this one is left \
                     unused.",
                )
                .on_disabled_hover_text("No CreatureDisplayInfoExtra row names this skin.")
                .clicked()
            {
                let mut said = Vec::new();
                for id in rows {
                    said.push(match crate::tables::bake::rebake(work.session, work.game, id, work.now) {
                        Ok(done) => done,
                        Err(why) => why,
                    });
                }
                work.browser.forget_buffers();
                work.session.status = said.join("; ");
            }
        }
        Sort::FlightMap => {
            let Some(map) = flight_map_of(vpath) else { return };
            let here = map == work.session.map_id;
            if ui
                .add_enabled(here, egui::Button::new("Draw Again"))
                .on_hover_text("Draw this picture again from the map's minimap pictures, with the flight map's box as it is.")
                .on_disabled_hover_text(format!("The picture is of map {map}. Open that map to draw it again."))
                .clicked()
            {
                use vale_edit::flightmap;
                let flight = work
                    .session
                    .open_table(work.game, flightmap::TABLE)
                    .then(|| work.session.table(flightmap::TABLE))
                    .flatten()
                    .and_then(|table| flightmap::row_of(table, map).and_then(|row| flightmap::read(table, row)));
                work.session.status = match flight {
                    Some(flight) => crate::tools::places::flightpaths::draw_flight_map(work.session, work.game, work.flights, &flight),
                    None => format!("{} has no row for map {map}", flightmap::TABLE),
                };
            }
        }
        _ => {}
    }
}

/// The workspace's third part, in the shell's inspector: the chosen file,
/// drawn.
pub(crate) fn inspector(ui: &mut egui::Ui, mut work: Work<'_>) {
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
            let key = model_portrait(work, &vpath, true);
            let side = ui.available_width();
            crate::ui::widgets::preview::preview_pane_at(ui, work.portraits, &key, side);
        }
        _ => theme::note(ui, "No picture for this kind of file."),
    }
}

/// Whether the archives, read past the project, hold a file at `vpath`. A
/// whole read, so the answer is kept per path in egui's memory: the dialog
/// asks every frame and the path changes only as it is typed.
pub(super) fn in_archives(ctx: &egui::Context, game: &vale_client::assets::GameAssets, vpath: &str) -> bool {
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
pub(in crate::ui) fn checker(ui: &egui::Ui, rect: egui::Rect) {
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

/// The Import Texture and Import Model dialogs, when one is open. Drawn by
/// the shell under any tool, since a form field and the texture tool's panel
/// open the first too.
///
/// The texture dialog's first choice is what the texture is for
/// ([`Purpose`]); the folder, the size and the files written follow from it,
/// and the rows under it are only the ones that purpose has.
pub(crate) fn import_dialog(ctx: &egui::Context, assets: &mut Assets, session: &EditSession, game: &vale_client::assets::GameAssets) {
    super::models::model_dialog(ctx, assets, session, game);
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

/// The Assets workspace's inspector panel, its third part: the chosen file,
/// drawn.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, assets, now, .. } = subject;
    inspector(
        ui,
        Work {
            session,
            assets: &mut editing.assets,
            game: assets,
            thumbnails: &mut editing.thumbnails,
            portraits: &mut editing.portraits,
            tiles: &mut editing.tiles,
            flights: &mut editing.flightpaths,
            browser: &mut editing.browser,
            now,
        },
    );
}

/// The Assets workspace in the middle of a window: the project's own files.
pub(crate) fn workspace(ui: &mut egui::Ui, _: crate::tools::Tool, bench: crate::ui::workspace::Bench<'_, '_, '_>) {
    let crate::ui::workspace::Bench { session, editing, assets, now, .. } = bench;
    draw(
        ui,
        Work {
            session,
            assets: &mut editing.assets,
            game: assets,
            thumbnails: &mut editing.thumbnails,
            portraits: &mut editing.portraits,
            tiles: &mut editing.tiles,
            flights: &mut editing.flightpaths,
            browser: &mut editing.browser,
            now,
        },
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
        assert_eq!(Sort::of(&files[0], false), Sort::Ground);
        assert_eq!(Sort::of(&files[3], false), Sort::Icon);
    }

    /// The generated sorts are the generators' files, and a model is the
    /// lab's unless it was imported.
    #[test]
    fn the_generated_sorts() {
        let own = |vpath: &str, kind: Kind| Own { vpath: vpath.to_string(), kind, bytes: 1 };
        let skin = own("Textures\\BakedNpcTextures\\x5_00ff.blp", Kind::Texture);
        assert_eq!(Sort::of(&skin, false), Sort::NpcSkin);
        assert!(Sort::NpcSkin.generated());
        let model = own("Creature\\Mage\\Mage.m2", Kind::Model);
        assert_eq!(Sort::of(&model, true), Sort::Model);
        assert_eq!(Sort::of(&model, false), Sort::BakedModel);
        assert!(!Sort::Model.generated());
    }
}
