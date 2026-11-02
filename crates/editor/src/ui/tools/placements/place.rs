//! The placement picker shared by the doodad and WMO panels: the model
//! browser by folder, search, Starred and Recent, and the preview of the
//! chosen model. The state is [`crate::tools::placements::place`].

use bevy_egui::egui;

use crate::tools::placements::place::{Kind, Placing, Switch};
use crate::tools::placements::place;
use crate::tools::placements::swap::Swap;
use crate::ui::widgets::folders::{breadcrumb, folder_rows, split_folder};
use crate::ui::widgets::preview::preview_pane;
use crate::ui::widgets::theme;

/// The mode switch, and the model picker shown in Place and Cluster mode.
///
/// Shared by the two placement tools, because choosing a model is the same
/// action whichever list it goes into; see [`crate::tools::placements::place`] for the rest
/// of the reasoning. Returns `true` while the tool is in Place or Cluster
/// mode, which tells the caller to draw the placement rows instead of the
/// selection.
pub(super) fn placing(
    ui: &mut egui::Ui,
    placing: &mut Placing,
    kind: Kind,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
) -> bool {
    let was = placing.switch();
    let mut to = was;
    theme::segmented(ui, &mut to, place::switches(kind), |a, b| a == b);
    if to != was {
        // Switching to Select drops whatever was on the cursor. `aim` also
        // does this; doing it here as well makes the panel correct on the
        // frame of the switch rather than the frame after.
        placing.set_switch(to);
    }
    match placing.switch() {
        Switch::Select => return false,
        Switch::Place => model(ui, placing, kind, portraits, favourites),
        Switch::Cluster => cluster(ui, placing, portraits, favourites),
    }
    placement(ui, placing, kind);
    true
}

/// Place mode's top half: the chosen model, its picture, and the browser
/// that chooses it.
fn model(
    ui: &mut egui::Ui,
    placing: &mut Placing,
    kind: Kind,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
) {
    ui.add_space(4.0);
    theme::heading(ui, "Model");
    match placing.path.clone() {
        Some(path) => {
            ui.label(egui::RichText::new(place::leaf(&path)).color(theme::INK))
                .on_hover_text(&path);
            // A button that opens the chosen model's folder, because the next
            // model is most likely to be in the same folder.
            let folder = Placing::folder_of(&path);
            if placing.folder(kind) != folder
                && ui
                    .small_button(egui::RichText::new("show folder").color(theme::INK_DIM))
                    .clicked()
            {
                placing.open_folder(kind, &folder);
            }
        }
        None => theme::note(ui, "no model chosen; pick one below"),
    }
    // A model the archives will not open is reported here. Without this the
    // cursor stays empty and a click writes a record for a file that is not
    // there. [`crate::tools::placements::place`] describes the `.mdx` case that made this
    // necessary.
    if let Some(bad) = placing.trouble.clone() {
        ui.label(
            egui::RichText::new(format!("{} failed to load", place::leaf(&bad)))
                .color(theme::BAD)
                .size(theme::SMALL),
        )
        .on_hover_text(bad);
    }

    // A large preview of the chosen model, under its name; see
    // `crate::preview::portraits`. A row's small picture identifies a model; this one
    // is large enough to judge it, and a drag turns it.
    //
    // It sits under the chosen model's name, not under the list. The list is
    // three hundred points tall in a panel of six hundred, so a pane under it
    // would start below the fold and need scrolling to. The browser's dialog
    // places its pane under its list by the same rule, because that is where
    // its chosen model is named.
    if let Some(path) = placing.path.clone() {
        ui.add_space(4.0);
        preview_pane(ui, portraits, &path);
    }

    ui.add_space(4.0);
    ui.add(
        egui::TextEdit::singleline(&mut placing.search)
            .hint_text("search the models")
            .desired_width(f32::INFINITY),
    );

    let chosen: Vec<String> = placing.path.iter().cloned().collect();
    let search = placing.search.clone();
    if let Some(picked) = browse(ui, placing, kind, &search, &chosen, portraits, favourites) {
        favourites.used(list_of(kind), &picked);
        let (turn, scale) = (placing.turn, placing.scale);
        placing.arm(kind, picked, turn, scale);
    }
}

/// What Place and Cluster mode share below the models: the turn and scale,
/// the slope, Scatter, and the keys.
fn placement(ui: &mut egui::Ui, placing: &mut Placing, kind: Kind) {
    ui.add_space(4.0);
    theme::heading(ui, "Placement");
    theme::row(ui, "turn", |ui| {
        ui.add(
            egui::DragValue::new(&mut placing.turn)
                .speed(1.0)
                .fixed_decimals(1)
                .suffix("°"),
        );
    });
    if kind == Kind::Doodad {
        theme::row(ui, "scale", |ui| {
            ui.add(
                egui::DragValue::new(&mut placing.scale)
                    .speed(0.01)
                    .range(crate::tools::placements::doodads::SCALE_RANGE)
                    .fixed_decimals(3)
                    .suffix("x"),
            );
        });
    }

    if placing.clustering() {
        theme::note(
            ui,
            "The turn turns the whole cluster about its centre. With Scatter \
             off, every doodad takes this turn and scale.",
        );
    }

    // Slope alignment: a drop is leaned onto the ground under the cursor, and
    // the ghost shows the lean. Doodads only. See `Placing::align`.
    if kind == Kind::Doodad {
        ui.add_space(4.0);
        theme::heading(ui, "Ground");
        ui.checkbox(&mut placing.align, "align each placement to the slope")
            .on_hover_text(
                "Tilts each placed model to the ground normal under the \
                 cursor, keeping the current turn, instead of using the \
                 scatter tilt. The preview shows the result before the click.",
            );
    }

    // Scatter: every drop rolls the next one's turn, size and lean inside
    // these bounds. See `crate::tools::placements::place`'s module comment.
    ui.add_space(4.0);
    theme::heading(ui, "Scatter");
    match placing.clustering() {
        true => ui
            .checkbox(&mut placing.scatter.on, "randomise each doodad")
            .on_hover_text(
                "Gives each doodad of the cluster its own random turn, scale \
                 and tilt within the ranges below. The previews show what the \
                 next click writes.",
            ),
        false => ui
            .checkbox(&mut placing.scatter.on, "randomise after each placement")
            .on_hover_text(
                "After each placement, picks a random turn, scale and tilt for \
                 the next one within the ranges below. The preview shows what \
                 the next click writes.",
            ),
    };
    if placing.scatter.on {
        theme::row(ui, "turn", |ui| {
            ui.checkbox(&mut placing.scatter.turn, "any angle");
        });
        if kind == Kind::Doodad {
            theme::row(ui, "scale", |ui| {
                ui.add(
                    egui::DragValue::new(&mut placing.scatter.size.0)
                        .speed(0.01)
                        .range(crate::tools::placements::doodads::SCALE_RANGE)
                        .fixed_decimals(2)
                        .suffix("x"),
                );
                ui.label(egui::RichText::new("to").color(theme::INK_FAINT));
                ui.add(
                    egui::DragValue::new(&mut placing.scatter.size.1)
                        .speed(0.01)
                        .range(crate::tools::placements::doodads::SCALE_RANGE)
                        .fixed_decimals(2)
                        .suffix("x"),
                );
            });
            theme::row(ui, "tilt", |ui| {
                ui.add(
                    egui::DragValue::new(&mut placing.scatter.tilt_max)
                        .speed(0.5)
                        .range(0.0..=90.0)
                        .fixed_decimals(1)
                        .suffix("°"),
                )
                .on_hover_text(
                    "Maximum tilt from vertical, in any direction. Zero keeps the model upright.",
                );
            });
        }
        // A cluster draws its members' values with its layout, which New
        // layout redraws.
        if !placing.clustering()
            && ui
                .small_button("Randomise now")
                .on_hover_text("Pick new random values for the next placement without placing anything.")
                .clicked()
        {
            placing.roll();
        }
    }

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    match placing.clustering() {
        true => {
            theme::note(ui, "left button places the cluster · a new layout follows");
            theme::note(ui, ", and . turn the cluster · shift turns three times as far");
            theme::note(ui, "ctrl + wheel scales");
            theme::note(ui, "escape returns to select");
        }
        false => {
            theme::note(ui, "left button places · the model stays on the cursor");
            theme::note(ui, ", and . turn · shift turns three times as far");
            if kind == Kind::Doodad {
                theme::note(ui, "ctrl + wheel scales");
            }
            theme::note(ui, "escape clears the model from the cursor");
        }
    }
    theme::note(ui, "alt + mouse turns the model · ctrl snaps to 15°");
    theme::note(
        ui,
        match kind {
            Kind::Doodad => "1, 2 and 3 choose select, place and cluster · tab toggles select",
            Kind::Wmo => "tab, or 1 and 2, switch select and place",
        },
    );
}

/// Cluster mode's top half: the cluster's models, how many doodads a click
/// places, over how wide a circle and how far apart, and the browser that
/// adds and removes models. See [`crate::tools::placements::cluster`].
fn cluster(
    ui: &mut egui::Ui,
    placing: &mut Placing,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
) {
    use crate::tools::placements::cluster::{MOST, WIDEST};
    ui.add_space(4.0);
    theme::heading(ui, "Models");
    // The models, each with a button that takes it out.
    let mut drop: Option<usize> = None;
    for (at, path) in placing.cluster.models.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui
                .small_button("×")
                .on_hover_text("Take this model out of the cluster.")
                .clicked()
            {
                drop = Some(at);
            }
            ui.label(egui::RichText::new(place::leaf(path)).color(theme::INK))
                .on_hover_text(path);
        });
    }
    if let Some(at) = drop {
        placing.cluster.models.remove(at);
    }
    theme::note(
        ui,
        match placing.cluster.models.is_empty() {
            true => "No models. Click models below to add them to the cluster.",
            false => {
                "Each doodad is one of these, each equally likely. Click a model \
                 under Add Models to add it, or a highlighted one to take it out."
            }
        },
    );

    // The settings come before the browser, which is several hundred points
    // tall: under it they would start below the fold.
    ui.add_space(4.0);
    theme::heading(ui, "Cluster");
    theme::row(ui, "count", |ui| {
        let cluster = &mut placing.cluster;
        match cluster.random_count {
            false => {
                ui.add(egui::DragValue::new(&mut cluster.count).speed(0.2).range(1..=MOST))
                    .on_hover_text("How many doodads one click places.");
            }
            true => {
                ui.add(egui::DragValue::new(&mut cluster.count_range.0).speed(0.2).range(1..=MOST))
                    .on_hover_text("The fewest doodads one click places.");
                ui.label(egui::RichText::new("to").color(theme::INK_FAINT));
                ui.add(egui::DragValue::new(&mut cluster.count_range.1).speed(0.2).range(1..=MOST))
                    .on_hover_text("The most doodads one click places.");
            }
        }
    });
    theme::row(ui, "", |ui| {
        ui.checkbox(&mut placing.cluster.random_count, "random count")
            .on_hover_text(
                "Each click places a number of doodads drawn between the two \
                 bounds, rather than the same number every time.",
            );
    });
    theme::row(ui, "radius", |ui| {
        let cluster = &mut placing.cluster;
        fn yards(value: &mut f32) -> egui::DragValue<'_> {
            egui::DragValue::new(value)
                .speed(0.25)
                .range(1.0..=WIDEST)
                .fixed_decimals(1)
                .suffix(" yd")
        }
        match cluster.random_radius {
            false => {
                ui.add(yards(&mut cluster.radius)).on_hover_text(
                    "The radius of the circle the doodads are spread over, drawn round the pointer.",
                );
            }
            true => {
                ui.add(yards(&mut cluster.radius_range.0))
                    .on_hover_text("The smallest radius one click covers.");
                ui.label(egui::RichText::new("to").color(theme::INK_FAINT));
                ui.add(yards(&mut cluster.radius_range.1))
                    .on_hover_text("The largest radius one click covers.");
            }
        }
    });
    theme::row(ui, "", |ui| {
        ui.checkbox(&mut placing.cluster.random_radius, "random radius")
            .on_hover_text(
                "Each click spreads the doodads over a circle whose radius is \
                 drawn between the two bounds. The circle round the pointer \
                 shows the next click's.",
            );
    });
    let widest = placing.cluster.widest() * 2.0;
    theme::row(ui, "spacing", |ui| {
        let cluster = &mut placing.cluster;
        fn apart(value: &mut f32, widest: f32) -> egui::DragValue<'_> {
            egui::DragValue::new(value)
                .speed(0.05)
                .range(0.0..=widest)
                .fixed_decimals(1)
                .suffix(" yd")
        }
        match cluster.random_spacing {
            false => {
                ui.add(apart(&mut cluster.spacing, widest))
                    .on_hover_text("The least distance between two doodads of one cluster.");
            }
            true => {
                ui.add(apart(&mut cluster.spacing_range.0, widest))
                    .on_hover_text("The smallest spacing one click keeps.");
                ui.label(egui::RichText::new("to").color(theme::INK_FAINT));
                ui.add(apart(&mut cluster.spacing_range.1, widest))
                    .on_hover_text("The largest spacing one click keeps.");
            }
        }
    });
    theme::row(ui, "", |ui| {
        ui.checkbox(&mut placing.cluster.random_spacing, "random spacing")
            .on_hover_text(
                "Each click keeps its doodads apart by a spacing drawn between \
                 the two bounds: some clusters dense, some sparse.",
            );
    });
    let (laid, asked) = (placing.cluster.layout.len() as u32, placing.cluster.asked);
    let spacing = placing.cluster.laid_spacing;
    let next = match (laid < asked, placing.cluster.random_count) {
        (true, _) => Some(format!("next click: {laid} of {asked} fit at {spacing:.1} yd spacing")),
        (false, true) => Some(format!("next click: {laid} doodads")),
        (false, false) => None,
    };
    if let Some(next) = next.filter(|_| !placing.cluster.models.is_empty()) {
        theme::note(ui, next);
    }
    if ui
        .small_button("New layout")
        .on_hover_text(
            "Draw new places, models, a new count, radius and spacing where \
             they are random, and with \
             Scatter on, new turns, sizes and tilts for the next click.",
        )
        .clicked()
    {
        placing.cluster.redraw();
    }

    ui.add_space(4.0);
    theme::heading(ui, "Add Models");
    ui.add(
        egui::TextEdit::singleline(&mut placing.search)
            .hint_text("search the models")
            .desired_width(f32::INFINITY),
    );
    let chosen = placing.cluster.models.clone();
    let search = placing.search.clone();
    if let Some(picked) = browse(ui, placing, Kind::Doodad, &search, &chosen, portraits, favourites) {
        favourites.used(list_of(Kind::Doodad), &picked);
        placing.cluster.toggle(&picked);
    }
}

/// The list a doodad or WMO panel shows for replacing the selected
/// placements' model: the model browser, the chosen model's picture, and the
/// button that writes it. See [`crate::tools::placements::swap`].
///
/// `count` is how many placements are selected.
pub(super) fn replacing(
    ui: &mut egui::Ui,
    swap: &mut Swap,
    placing: &mut Placing,
    kind: Kind,
    count: usize,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
) {
    if swap.open != Some(kind) {
        return;
    }
    let noun = match (kind, count) {
        (Kind::Doodad, 1) => "doodad",
        (Kind::Doodad, _) => "doodads",
        (Kind::Wmo, 1) => "WMO",
        (Kind::Wmo, _) => "WMOs",
    };
    ui.add_space(4.0);
    theme::heading(ui, "Replace model");
    theme::note(
        ui,
        match kind {
            Kind::Doodad => {
                "Each selected doodad takes the model chosen here and keeps its \
                 position, rotation and scale."
            }
            Kind::Wmo => {
                "Each selected WMO takes the WMO chosen here and keeps its position \
                 and rotation. Its bounds are taken from the new WMO; a doodad set \
                 the new WMO lacks becomes set 0."
            }
        },
    );
    match swap.with.clone() {
        Some(path) => {
            ui.label(egui::RichText::new(place::leaf(&path)).color(theme::INK))
                .on_hover_text(&path);
            preview_pane(ui, portraits, &path);
        }
        None => theme::note(ui, "no model chosen; pick one below"),
    }
    ui.horizontal(|ui| {
        let ready = swap.with.is_some() && count > 0;
        if ui
            .add_enabled(ready, egui::Button::new(format!("Replace {count} {noun}")))
            .on_hover_text("One undo entry for every placement replaced.")
            .clicked()
        {
            swap.asked = Some(kind);
        }
        if ui.small_button("Cancel").clicked() {
            swap.close();
        }
    });
    ui.add(
        egui::TextEdit::singleline(&mut swap.search)
            .hint_text("search the models")
            .desired_width(f32::INFINITY),
    );
    let chosen: Vec<String> = swap.with.iter().cloned().collect();
    let search = swap.search.clone();
    if let Some(picked) = browse(ui, placing, kind, &search, &chosen, portraits, favourites) {
        favourites.used(list_of(kind), &picked);
        swap.with = Some(picked);
    }
    ui.add_space(4.0);
}

/// The catalogue as folders rather than one list of several thousand names.
/// Returns the model a click chose.
///
/// A flat list would put 5,816 models and 816 buildings in one scroller, where
/// every model has to be found by typing, which works only when the name is
/// already known. The archives' own directories are the game's
/// grouping (`vale catalogue` prints them, seventeen at the top level), so
/// the folders are taken from them rather than invented.
///
/// Search still runs flat across everything. A search limited to the open
/// folder would find nothing for a model one directory over.
///
/// The starred and recent lists come before the folders, collapsible, and
/// take no space when empty. Scattering a copse uses the same three models
/// repeatedly, and without these lists each one needs a search; see
/// [`crate::prefs::favourites`]. Every row shows the model's picture (see
/// [`crate::preview::portraits`]), because a file name does not show what the model
/// looks like.
///
/// `search` is the box above it: the placer's own, or the replace list's.
/// `chosen` are the models drawn as chosen: the placer's one model, the
/// cluster's list, or the replace list's choice.
/// The folder shown is the placer's in both, so the replace list opens where
/// the placer was last browsing.
fn browse(
    ui: &mut egui::Ui,
    placing: &mut Placing,
    kind: Kind,
    search: &str,
    chosen: &[String],
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
) -> Option<String> {
    let needle = search.to_ascii_lowercase();
    let catalogue = placing.catalogue(kind);
    if !catalogue.iter().any(|path| !path.is_empty()) {
        theme::note(ui, "reading the model list…");
        return None;
    }
    let total = catalogue.len();
    let list = list_of(kind);
    let spell = std::sync::Arc::clone(&placing.spellings);

    // Searching: every match, wherever it is, with its folder under it.
    if !needle.is_empty() {
        let matches: Vec<String> = catalogue
            .iter()
            .filter(|path| !path.is_empty() && path.to_ascii_lowercase().contains(&needle))
            .cloned()
            .collect();
        theme::note(ui, format!("{} of {total} match", matches.len()));
        return rows(
            ui, "search", &matches, chosen, true, portraits, favourites, list, &spell,
        );
    }

    let mut picked: Option<String> = None;
    for (name, paths) in [
        ("Starred", favourites.starred(list)),
        ("Recent", favourites.recent(list)),
    ] {
        if paths.is_empty() {
            continue;
        }
        egui::CollapsingHeader::new(
            egui::RichText::new(format!("{name} ({})", paths.len()))
                .size(theme::SMALL)
                .color(theme::INK_DIM),
        )
        .id_salt(("kept", name, kind as u8))
        .default_open(name == "Starred")
        .show(ui, |ui| {
            picked = rows(ui, name, &paths, chosen, false, portraits, favourites, list, &spell)
                .or(picked.take());
        });
    }

    // Browsing: the way back up, then the folders, then what is in this one.
    let here = placing.folder(kind).to_string();
    let mut go = breadcrumb(ui, &here, "", &spell);
    let (folders, files) = split_folder(catalogue, &here);
    theme::note(
        ui,
        match folders.len() {
            0 => format!("{} models", files.len()),
            n => format!("{n} folders · {} models", files.len()),
        },
    );

    let row = ui.spacing().interact_size.y + 6.0;
    egui::ScrollArea::vertical()
        .id_salt("models")
        .max_height(300.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            go = folder_rows(ui, &here, &folders, row, &spell).or(go.take());
            picked = rows(
                ui, "here", &files, chosen, false, portraits, favourites, list, &spell,
            )
            .or(picked.take());
        });
    if let Some(folder) = go {
        placing.open_folder(kind, &folder);
    }
    picked
}

/// Which of the kept lists a placement kind's picker reads and writes.
fn list_of(kind: Kind) -> crate::prefs::favourites::Kind {
    match kind {
        Kind::Doodad => crate::prefs::favourites::Kind::Doodad,
        Kind::Wmo => crate::prefs::favourites::Kind::Wmo,
    }
}

/// A run of model rows. Returns the one a click chose.
///
/// Uses `show_rows`, as the tileset list does: the archives name thousands of
/// models and only the dozen on screen need building. Each row here requests a
/// rendered picture, so only the rows on screen are rendered. The row is the
/// workspace model browser's (`crate::ui::widgets::models::model_row`): the picture, the file name
/// over its folder, and a star.
#[allow(clippy::too_many_arguments)]
fn rows(
    ui: &mut egui::Ui,
    salt: &str,
    paths: &[String],
    chosen: &[String],
    scroller: bool,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
    list: crate::prefs::favourites::Kind,
    spell: &crate::session::files::Spellings,
) -> Option<String> {
    let mut picked: Option<String> = None;
    let mut draw = |ui: &mut egui::Ui, range: std::ops::Range<usize>| {
        for path in &paths[range] {
            let on = chosen.iter().any(|had| had.eq_ignore_ascii_case(path));
            let hit = crate::ui::widgets::models::model_row(ui, portraits, favourites, list, path, spell.of(path), on);
            if hit.starred {
                favourites.toggle(list, path);
            }
            if hit.clicked || hit.double {
                picked = Some(path.clone());
            }
        }
    };
    match scroller {
        true => {
            egui::ScrollArea::vertical()
                .id_salt(salt)
                .max_height(300.0)
                .auto_shrink([false, true])
                .show_rows(ui, crate::ui::widgets::models::MODEL_ROW, paths.len(), |ui, range| {
                    draw(ui, range)
                });
        }
        // Already inside the browser's own scroller, which also holds the
        // folders above these rows; a nested scroller would draw two bars.
        false => draw(ui, 0..paths.len()),
    }
    picked
}
