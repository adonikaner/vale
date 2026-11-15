//! What the doodad and WMO tools share: the toolbar over the viewport, and
//! in the inspector the chosen model with its preview, the placement
//! settings, and the Replace Model section. The models are listed in the
//! asset drawer under the viewport ([`crate::ui::shell::drawer`]), which
//! Place, Cluster and Replace Model open. The state is
//! [`crate::tools::placements::place`].
//!
//! ```text
//! toolbar     [Select | Place | Cluster]  Keys     Cluster on the doodad tool only
//!             Handles [Move | Turn | Off]          in Select
//!
//! inspector   Place     Model, Placement, Ground, Scatter
//!             Cluster   Models, Cluster, Placement, Ground, Scatter
//!             Select    the doodad or WMO panel's own sections
//! ```
//!
//! Ground is drawn on the doodad tool only.

use bevy_egui::egui;

use crate::tools::placements::gizmo::{Handles, HANDLES};
use crate::tools::placements::place::{Kind, Placing, Switch};
use crate::tools::placements::place;
use crate::tools::placements::swap::Swap;
use crate::ui::shell::drawer::{Drawer, Purpose};
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
                        // The building editor's, on a bar of its own.
                        Switch::Areas | Switch::Doodads | Switch::Lights => "",
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
                (Switch::Areas | Switch::Doodads | Switch::Lights, _) => &[],
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

/// The chosen model or models and the placement rows, shown in Place and
/// Cluster mode, which also open the asset drawer.
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
    drawer: &mut Drawer,
) -> bool {
    match placing.switch() {
        Switch::Select | Switch::Areas | Switch::Doodads | Switch::Lights => return false,
        Switch::Place => model(ui, placing, kind, portraits, drawer),
        Switch::Cluster => cluster(ui, placing, drawer),
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

/// Place mode's top half: the chosen model and its picture. The asset drawer
/// chooses it.
fn model(
    ui: &mut egui::Ui,
    placing: &mut Placing,
    kind: Kind,
    portraits: &mut crate::preview::portraits::Portraits,
    drawer: &mut Drawer,
) {
    theme::heading_about(ui, "Model", "Click a model in the asset drawer under the viewport to choose it.");
    match placing.path.clone() {
        Some(path) => {
            theme::value(ui, place::leaf(&path)).on_hover_text(&path);
            // A button that opens the chosen model's folder in the asset
            // drawer, because the next model is most likely to be in the same
            // folder.
            let folder = Placing::folder_of(&path);
            if placing.folder(kind) != folder && ui.button("Show Folder").clicked() {
                placing.open_folder(kind, &folder);
                placing.search.clear();
                drawer.show_folder(kind);
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
    // `crate::preview::portraits`. A tile's small picture in the asset drawer
    // identifies a model; this one is large enough to judge it, and a drag
    // turns it.
    if let Some(path) = placing.path.clone() {
        ui.add_space(4.0);
        preview_pane(ui, portraits, &path);
    }

    if let Some(picked) = drawer.offer(Purpose::Place(kind), placing.path.as_slice()) {
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
/// places, over how wide a circle and how far apart. The asset drawer adds
/// and removes models. See [`crate::tools::placements::cluster`].
///
/// A setting whose Random switch is on is two rows, From and To, each with
/// its own hover.
fn cluster(ui: &mut egui::Ui, placing: &mut Placing, drawer: &mut Drawer) {
    use crate::tools::placements::cluster::{MOST, WIDEST};
    theme::heading_about(
        ui,
        "Models",
        "Each doodad is one of these models, each equally likely. Click a model \
         in the asset drawer to add it, or a highlighted one to take it out.",
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

    if let Some(picked) = drawer.offer(Purpose::Cluster, &placing.cluster.models) {
        placing.cluster.toggle(&picked);
    }
}

/// The section a doodad or WMO panel shows for replacing the selected
/// placements' model: the chosen model's picture and the button that writes
/// it. The section opens the asset drawer, which chooses the model. See
/// [`crate::tools::placements::swap`].
///
/// `count` is how many placements are selected.
pub(super) fn replacing(
    ui: &mut egui::Ui,
    swap: &mut Swap,
    kind: Kind,
    count: usize,
    portraits: &mut crate::preview::portraits::Portraits,
    drawer: &mut Drawer,
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
                 position, rotation and scale. Click a model in the asset drawer \
                 to choose it."
            }
            Kind::Wmo => {
                "Each selected WMO takes the WMO chosen here and keeps its position \
                 and rotation. Its bounds are taken from the new WMO; a doodad set \
                 the new WMO lacks becomes set 0. Click a WMO in the asset drawer \
                 to choose it."
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
    if let Some(picked) = drawer.offer(Purpose::Replace(kind), swap.with.as_slice()) {
        swap.with = Some(picked);
    }
    ui.add_space(4.0);
}

/// Which of the kept lists a placement kind's models are starred in and
/// counted as recent in. The asset drawer reads and writes them.
pub(in crate::ui) fn list_of(kind: Kind) -> crate::prefs::favourites::Kind {
    match kind {
        Kind::Doodad => crate::prefs::favourites::Kind::Doodad,
        Kind::Wmo => crate::prefs::favourites::Kind::Wmo,
    }
}
