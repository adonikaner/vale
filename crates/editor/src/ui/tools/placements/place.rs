//! What the doodad and WMO tools share: the toolbar over the viewport, and
//! in the inspector the placement picker, which is the model browser by
//! folder, search, Starred and Recent, and the preview of the chosen model.
//! The state is [`crate::tools::placements::place`].
//!
//! ```text
//! toolbar     [Select | Place | Cluster]  Keys     Cluster on the doodad tool only
//!             Handles [Move | Turn | Off]          in Select
//!
//! inspector   Place     Model and the browser, Placement, Ground, Scatter
//!             Cluster   Models, Cluster, Add Models and the browser,
//!                       Placement, Ground, Scatter
//!             Select    the doodad or WMO panel's own sections
//! ```
//!
//! Ground is drawn on the doodad tool only.

use bevy_egui::egui;

use crate::tools::placements::gizmo::{Handles, HANDLES};
use crate::tools::placements::place::{Kind, Placing, Switch};
use crate::tools::placements::place;
use crate::tools::placements::swap::Swap;
use crate::ui::widgets::folders::{breadcrumb, folder_rows, split_folder};
use crate::ui::widgets::preview::preview_pane;
use crate::ui::widgets::theme::{self, bar_group, bar_keys, bar_segment, bar_setting, field, property};

/// The doodad or WMO tool's toolbar over the viewport:
///
/// ```text
/// [Select | Place | Cluster]  Keys
/// Handles [Move | Turn | Off]
/// ```
///
/// Cluster is the doodad tool's only. The second row is drawn in Select, the
/// one mode with a selection to move. The mode is one field of the placer
/// and the handles are one field of the gizmo, so each is a
/// [`theme::bar_group`]. The Keys button lists the chosen mode's controls.
/// See [`theme::toolbar`], and `crate::ui::tools::spawns::spawn::toolbar`,
/// which lays the spawn tools' out the same way.
pub(crate) fn toolbar(
    ctx: &egui::Context,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    kind: Kind,
    placing: &mut Placing,
    handles: &mut Handles,
) -> Option<egui::Rect> {
    let id = match kind {
        Kind::Doodad => "doodads-toolbar",
        Kind::Wmo => "wmos-toolbar",
    };
    theme::toolbar(ctx, id, viewport, corner, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let was = placing.switch();
            bar_group(ui, |ui| {
                for (name, to) in place::switches(kind) {
                    let hover = match to {
                        Switch::Select => "Click a placement to select it, and drag to move it.",
                        Switch::Place => "Choose a model in the inspector, then click the ground to place it.",
                        Switch::Cluster => {
                            "Choose models in the inspector, then click the ground to place several \
                             of them over a circle."
                        }
                    };
                    if bar_segment(ui, was == *to, name).on_hover_text(hover).clicked() && was != *to {
                        // Select puts down the model on the cursor and
                        // Place picks it up again. `aim` also does both;
                        // doing it here as well makes the bar and the panel
                        // correct on the frame of the switch.
                        placing.set_switch(*to);
                    }
                }
            });
            bar_keys(ui, match (placing.switch(), kind) {
                (Switch::Select, Kind::Doodad) => &super::doodads::KEYS,
                (Switch::Select, Kind::Wmo) => &super::wmos::KEYS,
                (Switch::Place, Kind::Doodad) => &PLACE_DOODAD_KEYS,
                (Switch::Place, Kind::Wmo) => &PLACE_WMO_KEYS,
                (Switch::Cluster, _) => &CLUSTER_KEYS,
            });
        });
        if placing.switch() == Switch::Select {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                bar_setting(ui, "Handles", |ui| {
                    bar_group(ui, |ui| {
                        for (name, to) in HANDLES {
                            let hover = match to {
                                Handles::Move => "Drag an arrow to move along one axis.",
                                Handles::Turn => "Drag a ring to turn about one axis.",
                                Handles::Off => "No handles: a click selects and a drag moves along the ground.",
                            };
                            if bar_segment(ui, *handles == to, name).on_hover_text(hover).clicked() {
                                *handles = to;
                            }
                        }
                    });
                });
            });
        }
    })
}

/// The model picker and the placement rows, shown in Place and Cluster mode.
///
/// Shared by the two placement tools, because choosing a model is the same
/// action whichever list it goes into; see [`crate::tools::placements::place`] for the rest
/// of the reasoning. Returns `true` while the tool is in Place or Cluster
/// mode, which tells the caller that the panel is drawn and the selection's
/// sections are not.
///
/// The mode is chosen on the toolbar, [`toolbar`], and the panel has no
/// instruction lines for it: the toolbar's Keys button lists its controls.
pub(super) fn placing(
    ui: &mut egui::Ui,
    placing: &mut Placing,
    kind: Kind,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
) -> bool {
    match placing.switch() {
        Switch::Select => return false,
        Switch::Place => model(ui, placing, kind, portraits, favourites),
        Switch::Cluster => cluster(ui, placing, portraits, favourites),
    }
    placement(ui, placing, kind);
    true
}

/// Place mode's mouse and keyboard controls on the doodad tool, for the Keys
/// button.
const PLACE_DOODAD_KEYS: [(&str, &str); 9] = [
    ("Left button", "Place the model, which stays on the cursor"),
    (", and .", "Turn"),
    ("Shift + , or .", "Turn three times as far"),
    ("Alt + mouse", "Turn the model"),
    ("Ctrl + Alt + mouse", "Turn in steps of 15\u{b0}"),
    ("Ctrl + wheel", "Scale"),
    ("Escape", "Clear the model from the cursor"),
    ("1, 2, 3", "Select, Place, Cluster"),
    ("Tab", "Switch between Select and placing"),
];

/// Place mode's controls on the WMO tool, which has no scale and no Cluster
/// mode.
const PLACE_WMO_KEYS: [(&str, &str); 8] = [
    ("Left button", "Place the model, which stays on the cursor"),
    (", and .", "Turn"),
    ("Shift + , or .", "Turn three times as far"),
    ("Alt + mouse", "Turn the model"),
    ("Ctrl + Alt + mouse", "Turn in steps of 15\u{b0}"),
    ("Escape", "Clear the model from the cursor"),
    ("1, 2", "Select, Place"),
    ("Tab", "Switch between Select and Place"),
];

/// Cluster mode's controls.
const CLUSTER_KEYS: [(&str, &str); 9] = [
    ("Left button", "Place the cluster; a new layout follows"),
    (", and .", "Turn the cluster"),
    ("Shift + , or .", "Turn three times as far"),
    ("Alt + mouse", "Turn the model"),
    ("Ctrl + Alt + mouse", "Turn in steps of 15\u{b0}"),
    ("Ctrl + wheel", "Scale"),
    ("Escape", "Return to Select"),
    ("1, 2, 3", "Select, Place, Cluster"),
    ("Tab", "Switch between Select and placing"),
];

/// A number as a [`property`] row: the name, then a [`field`]. `about`, when
/// not empty, is shown when either the name or the field is hovered.
pub(super) fn number_row(ui: &mut egui::Ui, name: &str, about: &str, value: egui::DragValue<'_>) -> egui::Response {
    property(ui, name, about, |ui| {
        let response = field(ui, value);
        match about.is_empty() {
            true => response,
            false => response.on_hover_text(about),
        }
    })
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
    theme::heading_about(ui, "Model", "Click a model in the list below to choose it.");
    match placing.path.clone() {
        Some(path) => {
            theme::value(ui, place::leaf(&path)).on_hover_text(&path);
            // A button that opens the chosen model's folder, because the next
            // model is most likely to be in the same folder.
            let folder = Placing::folder_of(&path);
            if placing.folder(kind) != folder && ui.button("Show Folder").clicked() {
                placing.open_folder(kind, &folder);
            }
        }
        None => theme::note(ui, "No model chosen"),
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
    // would start below the visible part of the panel and need scrolling to.
    // The browser's dialog places its pane under its list by the same rule,
    // because that is where its chosen model is named.
    if let Some(path) = placing.path.clone() {
        ui.add_space(4.0);
        preview_pane(ui, portraits, &path);
    }

    ui.add_space(4.0);
    ui.add(
        egui::TextEdit::singleline(&mut placing.search)
            .hint_text("Search the models")
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
/// the slope, and Scatter.
fn placement(ui: &mut egui::Ui, placing: &mut Placing, kind: Kind) {
    use crate::tools::placements::doodads::SCALE_RANGE;
    match placing.clustering() {
        true => theme::heading_about(
            ui,
            "Placement",
            "The turn turns the whole cluster about its centre. With Scatter \
             off, every doodad takes this turn and scale.",
        ),
        false => theme::heading(ui, "Placement"),
    }
    number_row(
        ui,
        "Turn",
        "",
        egui::DragValue::new(&mut placing.turn).speed(1.0).fixed_decimals(1).suffix("\u{b0}"),
    );
    if kind == Kind::Doodad {
        number_row(
            ui,
            "Scale",
            "",
            egui::DragValue::new(&mut placing.scale).speed(0.01).range(SCALE_RANGE).fixed_decimals(3).suffix("x"),
        );
    }

    // Slope alignment: a drop is leaned onto the ground under the cursor, and
    // the ghost shows the lean. Doodads only. See `Placing::align`.
    if kind == Kind::Doodad {
        theme::heading(ui, "Ground");
        ui.checkbox(&mut placing.align, "Align each placement to the slope")
            .on_hover_text(
                "Tilts each placed model to the ground normal under the \
                 cursor, keeping the current turn, instead of using the \
                 scatter tilt. The preview shows the result before the click.",
            );
    }

    // Scatter: every drop rolls the next one's turn, size and lean inside
    // these bounds. See `crate::tools::placements::place`'s module comment.
    theme::heading(ui, "Scatter");
    match placing.clustering() {
        true => ui
            .checkbox(&mut placing.scatter.on, "Randomise each doodad")
            .on_hover_text(
                "Gives each doodad of the cluster its own random turn, scale \
                 and tilt within the ranges below. The previews show what the \
                 next click writes.",
            ),
        false => ui
            .checkbox(&mut placing.scatter.on, "Randomise after each placement")
            .on_hover_text(
                "After each placement, picks a random turn, scale and tilt for \
                 the next one within the ranges below. The preview shows what \
                 the next click writes.",
            ),
    };
    if placing.scatter.on {
        property(ui, "Turn", "", |ui| {
            ui.checkbox(&mut placing.scatter.turn, "Any angle");
        });
        if kind == Kind::Doodad {
            // The two bounds are a row each. Side by side with their suffix,
            // they do not fit beside the name at the inspector's narrowest.
            number_row(
                ui,
                "Scale From",
                "",
                egui::DragValue::new(&mut placing.scatter.size.0).speed(0.01).range(SCALE_RANGE).fixed_decimals(2).suffix("x"),
            );
            number_row(
                ui,
                "Scale To",
                "",
                egui::DragValue::new(&mut placing.scatter.size.1).speed(0.01).range(SCALE_RANGE).fixed_decimals(2).suffix("x"),
            );
            number_row(
                ui,
                "Tilt",
                "Maximum tilt from vertical, in any direction. Zero keeps the model upright.",
                egui::DragValue::new(&mut placing.scatter.tilt_max).speed(0.5).range(0.0..=90.0).fixed_decimals(1).suffix("\u{b0}"),
            );
        }
        // A cluster draws its members' values with its layout, which New
        // Layout redraws.
        if !placing.clustering()
            && ui
                .button("Randomise Now")
                .on_hover_text("Pick new random values for the next placement without placing anything.")
                .clicked()
        {
            placing.roll();
        }
    }
}

/// Cluster mode's top half: the cluster's models, how many doodads a click
/// places, over how wide a circle and how far apart, and the browser that
/// adds and removes models. See [`crate::tools::placements::cluster`].
///
/// A setting whose Random switch is on is two rows, From and To, each with
/// its own hover.
fn cluster(
    ui: &mut egui::Ui,
    placing: &mut Placing,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
) {
    use crate::tools::placements::cluster::{MOST, WIDEST};
    theme::heading_about(
        ui,
        "Models",
        "Each doodad is one of these models, each equally likely. Click a model \
         under Add Models to add it, or a highlighted one to take it out.",
    );
    // The models, each with a button that takes it out.
    let mut drop: Option<usize> = None;
    for (at, path) in placing.cluster.models.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui
                .small_button("\u{d7}")
                .on_hover_text("Take this model out of the cluster.")
                .clicked()
            {
                drop = Some(at);
            }
            theme::value(ui, place::leaf(path)).on_hover_text(path);
        });
    }
    if let Some(at) = drop {
        placing.cluster.models.remove(at);
    }
    if placing.cluster.models.is_empty() {
        theme::note(ui, "No models");
    }

    // The settings come before the browser, which is several hundred points
    // tall: under it they would start below the visible part of the panel.
    theme::heading(ui, "Cluster");
    let cluster = &mut placing.cluster;
    match cluster.random_count {
        false => {
            number_row(
                ui,
                "Count",
                "How many doodads one click places.",
                egui::DragValue::new(&mut cluster.count).speed(0.2).range(1..=MOST),
            );
        }
        true => {
            number_row(
                ui,
                "Count From",
                "The fewest doodads one click places.",
                egui::DragValue::new(&mut cluster.count_range.0).speed(0.2).range(1..=MOST),
            );
            number_row(
                ui,
                "Count To",
                "The most doodads one click places.",
                egui::DragValue::new(&mut cluster.count_range.1).speed(0.2).range(1..=MOST),
            );
        }
    }
    ui.checkbox(&mut cluster.random_count, "Random count")
        .on_hover_text(
            "Each click places a number of doodads drawn between the two \
             bounds, rather than the same number every time.",
        );
    fn yards(value: &mut f32) -> egui::DragValue<'_> {
        egui::DragValue::new(value)
            .speed(0.25)
            .range(1.0..=WIDEST)
            .fixed_decimals(1)
            .suffix(" yd")
    }
    match cluster.random_radius {
        false => {
            number_row(
                ui,
                "Radius",
                "The radius of the circle the doodads are spread over, drawn round the pointer.",
                yards(&mut cluster.radius),
            );
        }
        true => {
            number_row(ui, "Radius From", "The smallest radius one click covers.", yards(&mut cluster.radius_range.0));
            number_row(ui, "Radius To", "The largest radius one click covers.", yards(&mut cluster.radius_range.1));
        }
    }
    ui.checkbox(&mut cluster.random_radius, "Random radius")
        .on_hover_text(
            "Each click spreads the doodads over a circle whose radius is \
             drawn between the two bounds. The circle round the pointer \
             shows the next click's.",
        );
    let widest = cluster.widest() * 2.0;
    fn apart(value: &mut f32, widest: f32) -> egui::DragValue<'_> {
        egui::DragValue::new(value)
            .speed(0.05)
            .range(0.0..=widest)
            .fixed_decimals(1)
            .suffix(" yd")
    }
    match cluster.random_spacing {
        false => {
            number_row(
                ui,
                "Spacing",
                "The least distance between two doodads of one cluster.",
                apart(&mut cluster.spacing, widest),
            );
        }
        true => {
            number_row(
                ui,
                "Spacing From",
                "The smallest spacing one click keeps.",
                apart(&mut cluster.spacing_range.0, widest),
            );
            number_row(
                ui,
                "Spacing To",
                "The largest spacing one click keeps.",
                apart(&mut cluster.spacing_range.1, widest),
            );
        }
    }
    ui.checkbox(&mut cluster.random_spacing, "Random spacing")
        .on_hover_text(
            "Each click keeps its doodads apart by a spacing drawn between \
             the two bounds: some clusters dense, some sparse.",
        );
    let (laid, asked) = (placing.cluster.layout.len() as u32, placing.cluster.asked);
    let spacing = placing.cluster.laid_spacing;
    let next = match (laid < asked, placing.cluster.random_count) {
        (true, _) => Some(format!("Next click: {laid} of {asked} fit at {spacing:.1} yd spacing")),
        (false, true) => Some(format!("Next click: {laid} doodads")),
        (false, false) => None,
    };
    if let Some(next) = next.filter(|_| !placing.cluster.models.is_empty()) {
        theme::note(ui, next);
    }
    if ui
        .button("New Layout")
        .on_hover_text(
            "Draw new places, models, a new count, radius and spacing where \
             they are random, and with \
             Scatter on, new turns, sizes and tilts for the next click.",
        )
        .clicked()
    {
        placing.cluster.redraw();
    }

    theme::heading(ui, "Add Models");
    ui.add(
        egui::TextEdit::singleline(&mut placing.search)
            .hint_text("Search the models")
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
        (Kind::Doodad, 1) => "Doodad",
        (Kind::Doodad, _) => "Doodads",
        (Kind::Wmo, 1) => "WMO",
        (Kind::Wmo, _) => "WMOs",
    };
    theme::heading_about(
        ui,
        "Replace Model",
        match kind {
            Kind::Doodad => {
                "Each selected doodad takes the model chosen here and keeps its \
                 position, rotation and scale. Click a model in the list below to \
                 choose it."
            }
            Kind::Wmo => {
                "Each selected WMO takes the WMO chosen here and keeps its position \
                 and rotation. Its bounds are taken from the new WMO; a doodad set \
                 the new WMO lacks becomes set 0. Click a WMO in the list below to \
                 choose it."
            }
        },
    );
    match swap.with.clone() {
        Some(path) => {
            theme::value(ui, place::leaf(&path)).on_hover_text(&path);
            preview_pane(ui, portraits, &path);
        }
        None => theme::note(ui, "No model chosen"),
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
        if ui.button("Cancel").clicked() {
            swap.close();
        }
    });
    ui.add(
        egui::TextEdit::singleline(&mut swap.search)
            .hint_text("Search the models")
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
        theme::note(ui, "Reading the model list\u{2026}");
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
            n => format!("{n} folders \u{b7} {} models", files.len()),
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
