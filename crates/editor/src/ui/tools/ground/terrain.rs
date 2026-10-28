//! The terrain brush's controls, in two places, and the switch that moves
//! placed objects with the ground, which the grade and road panels repeat. The
//! state is [`crate::tools::ground::terrain`].
//!
//! ```text
//! viewport toolbar   Sculpt or Select Vertices; in Sculpt the five modes, the
//!                    brush's numbers, falloff and shape, and Flatten's target
//!                    on a line of its own; in Select Vertices the footprint;
//!                    Only Inside Selection, Objects Follow, the drawing tablet
//!                    and the keys
//! inspector          the vertex selection: its height, Level and Smooth, its
//!                    tilt, and the locked vertices
//! ```

use bevy_egui::egui;

use crate::tools::ground::terrain::{FALLOFFS, MODES, SHAPES, Terrain, VertexAsk};
use crate::tools::ground::terrain;
use crate::ui::widgets::theme::{self, bar_choice, bar_gap, bar_setting, bar_toggle, property};

use vale_edit::ops::brush::{Mode, Only};

/// The terrain tool's toolbar over the viewport:
///
/// ```text
/// Sculpt  Select Vertices | Raise  Lower  Flatten  Smooth  Noise
/// Radius  Strength  Core  (Scale) | Falloff  Shape | Only Inside Selection  Objects Follow | Tablet
/// Flatten: Target  Height | Both  Fill  Cut | Tilt  Uphill        (Flatten only)
/// the keys
/// ```
///
/// In Select Vertices the second line is the footprint's radius and shape.
/// Only Inside Selection is shown only while vertices are selected, the one
/// case in which it changes a stroke. See [`theme::toolbar`].
pub(crate) fn toolbar(
    ctx: &egui::Context,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    terrain: &mut Terrain,
    tablet: &mut crate::tablet::Tablet,
    pen: &crate::tablet::Pen,
) -> Option<egui::Rect> {
    theme::toolbar(ctx, "terrain-toolbar", viewport, corner, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            if bar_toggle(ui, !terrain.selecting, "Sculpt")
                .on_hover_text("The left button moves the ground with the brush.")
                .clicked()
            {
                terrain.selecting = false;
            }
            if bar_toggle(ui, terrain.selecting, "Select Vertices")
                .on_hover_text("The left button selects the vertices under the footprint, for the inspector to move, level, smooth, tilt or lock.")
                .clicked()
            {
                terrain.selecting = true;
            }
            if !terrain.selecting {
                bar_gap(ui);
                for (key, (name, mode)) in MODES.into_iter().enumerate() {
                    // A flatten's target is set when the stroke begins, so the
                    // variant held here carries a stale height and is compared
                    // on its discriminant.
                    let on = std::mem::discriminant(&terrain.brush.mode) == std::mem::discriminant(&mode);
                    if bar_toggle(ui, on, name).on_hover_text(format!("Key {}.", key + 1)).clicked() && !on {
                        terrain.brush.mode = mode;
                    }
                }
            }
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            match terrain.selecting {
                false => sculpt_settings(ui, terrain),
                true => {
                    bar_setting(ui, "Footprint", |ui| {
                        ui.add(egui::DragValue::new(&mut terrain.brush.radius).speed(0.5).range(terrain::RADIUS).suffix(" yd"))
                            .on_hover_text("The radius a click selects within. Ctrl + wheel.");
                    });
                    bar_setting(ui, "Shape", |ui| {
                        bar_choice(ui, "terrain-shape", &mut terrain.brush.shape, &SHAPES, |a, b| a == b)
                            .on_hover_text("The footprint's outline. Shift + 1, 2 or 3.");
                    });
                }
            }
            bar_gap(ui);
            if terrain.selected > 0
                && bar_toggle(ui, terrain.only_inside, "Only Inside Selection")
                    .on_hover_text(
                        "On: a brush stroke moves the selected vertices and nothing else, and the \
                         selection is drawn green. Off: strokes move any vertex under the brush that \
                         is not locked. To keep vertices from moving, lock them.",
                    )
                    .clicked()
            {
                terrain.only_inside = !terrain.only_inside;
            }
            if bar_toggle(ui, terrain.objects_follow, "Objects Follow")
                .on_hover_text(OBJECTS_FOLLOW)
                .clicked()
            {
                terrain.objects_follow = !terrain.objects_follow;
            }
            ui.menu_button("Tablet", |ui| crate::ui::tools::tablet::tablet(ui, tablet, pen))
                .response
                .on_hover_text("Drawing tablet settings: pen pressure and cursor motion.");
        });
        if !terrain.selecting && matches!(terrain.brush.mode, Mode::Flatten { .. }) {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                flatten_settings(ui, terrain);
            });
        }
        let keys = match terrain.selecting {
            false => {
                "Left button: apply \u{b7} Ctrl + wheel: radius \u{b7} Shift + wheel: strength \u{b7} \
                 Alt + wheel: core \u{b7} 1-5: mode \u{b7} 6-0: falloff \u{b7} Shift + 1-3: shape"
            }
            true => "Left button: select \u{b7} Shift: deselect \u{b7} Ctrl + wheel: footprint \u{b7} Shift + 1-3: shape",
        };
        ui.add(egui::Label::new(egui::RichText::new(keys).size(theme::SMALL).color(theme::INK_FAINT)).wrap());
    })
}

/// The Sculpt line of the toolbar: radius, strength, core, the Noise scale,
/// falloff and shape.
fn sculpt_settings(ui: &mut egui::Ui, terrain: &mut Terrain) {
    bar_setting(ui, "Radius", |ui| {
        ui.add(egui::DragValue::new(&mut terrain.brush.radius).speed(0.5).range(terrain::RADIUS).suffix(" yd"))
            .on_hover_text("The outer ring. Ctrl + wheel.");
    });
    // The limits are the tool's own constants, which document the reason for
    // each. A range written as a literal in the panel would have no stated
    // reason.
    let (range, speed, suffix) = match terrain.brush.mode {
        Mode::Raise | Mode::Lower => (terrain::STRENGTH, 0.25, " yd/s"),
        _ => (terrain::RATE, 0.05, "/s"),
    };
    bar_setting(ui, "Strength", |ui| {
        ui.add(egui::DragValue::new(&mut terrain.brush.strength).speed(speed).range(range).suffix(suffix))
            .on_hover_text("How fast a held stroke works. Shift + wheel.");
    });
    // The fraction of the radius that applies full strength: the inner ring
    // the preview draws. See `vale_edit::ops::brush::Falloff::over`.
    bar_setting(ui, "Core", |ui| {
        ui.add(egui::DragValue::new(&mut terrain.brush.core).speed(0.01).range(crate::tools::CORE).fixed_decimals(2))
            .on_hover_text("The share of the radius at full strength, drawn as the inner ring. Alt + wheel.");
    });
    // Shown for Noise only, the one mode that reads it. A field shown but
    // ignored in four modes of five would suggest that the fields do not all
    // apply.
    if matches!(terrain.brush.mode, Mode::Noise) {
        bar_setting(ui, "Scale", |ui| {
            ui.add(egui::DragValue::new(&mut terrain.brush.scale).speed(0.5).range(terrain::SCALE).suffix(" yd"))
                .on_hover_text("The noise's feature size. Under 4 yd is finer than the vertex grid.");
        });
    }
    bar_gap(ui);
    bar_setting(ui, "Falloff", |ui| {
        bar_choice(ui, "terrain-falloff", &mut terrain.brush.falloff, &FALLOFFS, |a, b| a == b)
            .on_hover_text("How the strength fades from the inner ring to the outer ring. Keys 6 to 0.");
    });
    bar_setting(ui, "Shape", |ui| {
        bar_choice(ui, "terrain-shape", &mut terrain.brush.shape, &SHAPES, |a, b| a == b)
            .on_hover_text("The brush's outline. The square is aligned to the chunk grid. Shift + 1, 2 or 3.");
    });
}

/// The Flatten line of the toolbar: which height, which way the ground may
/// move, and whether the target is level or a tilted plane.
fn flatten_settings(ui: &mut egui::Ui, terrain: &mut Terrain) {
    ui.label(egui::RichText::new("Flatten").color(theme::INK));
    ui.add_space(4.0);
    if bar_toggle(ui, terrain.flatten_to_cursor, "Pointer Height")
        .on_hover_text("Flatten to the height under the pointer where the stroke starts. Off: the height set beside it.")
        .clicked()
    {
        terrain.flatten_to_cursor = !terrain.flatten_to_cursor;
    }
    if !terrain.flatten_to_cursor {
        bar_setting(ui, "Height", |ui| {
            ui.add(egui::DragValue::new(&mut terrain.flatten_height).speed(0.25).fixed_decimals(1).suffix(" yd"));
        });
    }
    bar_gap(ui);
    for (name, only, about) in [
        ("Both", Only::Both, "Raise low ground and lower high ground."),
        ("Fill", Only::Fill, "Only raise low ground."),
        ("Cut", Only::Cut, "Only lower high ground."),
    ] {
        if bar_toggle(ui, terrain.brush.only == only, name).on_hover_text(about).clicked() {
            terrain.brush.only = only;
        }
    }
    bar_gap(ui);
    bar_setting(ui, "Tilt", |ui| {
        ui.add(egui::DragValue::new(&mut terrain.tilt_angle).speed(0.25).range(0.0..=60.0).fixed_decimals(0).suffix("\u{b0}"))
            .on_hover_text("Slope of the target plane in degrees. 0 is level. The slope pivots where the stroke starts.");
    });
    // The bearing only matters once there is a slope.
    if terrain.tilt_angle > 0.0 {
        bar_setting(ui, "Uphill", |ui| {
            ui.add(egui::DragValue::new(&mut terrain.tilt_toward).speed(1.0).range(0.0..=359.0).fixed_decimals(0).suffix("\u{b0}"))
                .on_hover_text("Compass bearing of the uphill side. 0 is north, 90 east.");
            ui.label(egui::RichText::new(compass(terrain.tilt_toward)).size(theme::SMALL).color(theme::INK_DIM));
        });
    }
}

/// A tilt's two numbers as property rows: how steep, and which way is up.
/// Answers whether either changed.
fn tilt_rows(ui: &mut egui::Ui, angle: &mut f32, toward: &mut f32) -> bool {
    let mut changed = false;
    property(ui, "Tilt", "Slope in degrees. 0 is level.", |ui| {
        changed |= ui
            .add(egui::DragValue::new(angle).speed(0.25).range(0.0..=60.0).fixed_decimals(0).suffix("\u{b0}"))
            .changed();
        if *angle == 0.0 {
            theme::note(ui, "Level");
        }
    });
    // The bearing only matters once there is a slope.
    if *angle > 0.0 {
        property(ui, "Uphill", "Compass bearing of the uphill side. 0 is north, 90 east.", |ui| {
            changed |= ui
                .add(egui::DragValue::new(&mut *toward).speed(1.0).range(0.0..=359.0).fixed_decimals(0).suffix("\u{b0}"))
                .changed();
            theme::note(ui, compass(*toward));
        });
    }
    changed
}

/// The nearest of the eight compass points to a bearing in degrees.
fn compass(bearing: f32) -> &'static str {
    const POINTS: [&str; 8] = ["North", "North-east", "East", "South-east", "South", "South-west", "West", "North-west"];
    POINTS[((bearing.rem_euclid(360.0) / 45.0).round() as usize) % 8]
}

/// The terrain tool's inspector: the vertex selection and the locked
/// vertices. In Sculpt the selection is only described, since it is changed
/// in Select Vertices.
fn panel(ui: &mut egui::Ui, terrain: &mut Terrain) {
    match terrain.selecting {
        true => {
            title(ui, "Select Vertices");
            theme::note(ui, "Click the ground to select the vertices under the footprint. The footprint is on the toolbar.");
            selection(ui, terrain);
            tilt(ui, terrain);
        }
        false => {
            title(ui, "Sculpt");
            theme::note(
                ui,
                "Hold the left button over the ground to apply the brush. The brush settings are on \
                 the toolbar over the viewport.",
            );
            theme::heading(ui, "Selection");
            property(ui, "Selected", "", |ui| {
                ui.label(egui::RichText::new(selected_words(terrain.selected)).color(theme::INK));
            });
            if terrain.selected > 0 {
                theme::note(
                    ui,
                    match terrain.only_inside {
                        true => "Strokes move only the selected vertices (Only Inside Selection on the toolbar).",
                        false => "Strokes move any unlocked vertex. Only Inside Selection on the toolbar limits them to the selection.",
                    },
                );
            }
        }
    }
    locks(ui, terrain);
}

/// The size of the panel's title.
const TITLE: f32 = 17.0;

/// The panel's title.
fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).strong().size(TITLE).color(theme::INK));
}

/// How many vertices are selected, in words.
fn selected_words(selected: usize) -> String {
    match selected {
        0 => "None".to_string(),
        1 => "1 vertex".to_string(),
        n => format!("{n} vertices"),
    }
}

/// The Selection section: how many, Clear, the mean height, Level and Smooth.
fn selection(ui: &mut egui::Ui, terrain: &mut Terrain) {
    theme::heading(ui, "Selection");
    let any = terrain.selected > 0;
    property(ui, "Selected", "", |ui| {
        ui.label(egui::RichText::new(selected_words(terrain.selected)).color(theme::INK));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(any, egui::Button::new("Clear").small()).clicked() {
                terrain.ask = Some(VertexAsk::Clear);
            }
        });
    });
    // Dragging the mean height moves every selected vertex by the same
    // amount, so the shape they make is kept.
    property(ui, "Height", "The mean height. Drag or type to move the selection up or down.", |ui| {
        let mut mean = terrain.selected_mean;
        let moved = ui
            .add_enabled(any, egui::DragValue::new(&mut mean).speed(0.1).fixed_decimals(1).suffix(" yd"))
            .on_disabled_hover_text("Select some vertices first.");
        if moved.changed() && mean != terrain.selected_mean {
            terrain.ask = Some(VertexAsk::MoveTo(mean));
        }
    });
    property(ui, "Shape", "", |ui| {
        for (label, ask, about) in [
            ("Level", VertexAsk::Level, "Flatten the selection at its mean height."),
            ("Smooth", VertexAsk::Smooth, "Smooth the selection. Click again to smooth more."),
        ] {
            if ui
                .add_enabled(any, egui::Button::new(label))
                .on_hover_text(about)
                .on_disabled_hover_text("Select some vertices first.")
                .clicked()
            {
                terrain.ask = Some(ask);
            }
        }
    });
}

/// The Tilt section. The selection follows the numbers, pivoting on its
/// centre.
fn tilt(ui: &mut egui::Ui, terrain: &mut Terrain) {
    theme::heading(ui, "Tilt");
    let any = terrain.selected > 0;
    let [angle, toward] = &mut terrain.vertex_tilt;
    if tilt_rows(ui, angle, toward) && any {
        terrain.ask = Some(VertexAsk::Tilt);
    }
    ui.checkbox(&mut terrain.tilt_flat, "Flatten onto the slope")
        .on_hover_text("Off: the selection tilts and keeps its relief. On: it becomes a flat plane at the tilt.");
    theme::note(
        ui,
        match any {
            true => "Changing the tilt tilts the selection about its centre.",
            false => "Select some vertices to tilt them.",
        },
    );
}

/// The Locked Vertices section: how many the open tiles hold, and the four
/// ways to change them, which need Select Vertices.
fn locks(ui: &mut egui::Ui, terrain: &mut Terrain) {
    theme::heading(ui, "Locked Vertices");
    let any = terrain.selected > 0;
    property(ui, "Locked", "Locked vertices are drawn red.", |ui| {
        ui.label(
            egui::RichText::new(match terrain.locked {
                0 => "None on the open tiles".to_string(),
                n => format!("{n} on the open tiles"),
            })
            .color(theme::INK),
        );
    });
    ui.horizontal_wrapped(|ui| {
        for (label, ask, about, enabled) in [
            ("Lock Selection", VertexAsk::Lock, "Add the selected vertices to their tiles' locked vertices.", any),
            (
                "Unlock Selection",
                VertexAsk::Unlock,
                "Remove the selected vertices from their tiles' locked vertices.",
                any,
            ),
            (
                "Lock Tile Sides",
                VertexAsk::LockSides,
                "Lock the vertices on all four sides of each tile that holds part of the \
                 selection. A tile's side is shared with the tile beside it, so this keeps \
                 the ground at the seam level with a tile that is not being edited.",
                any,
            ),
            ("Unlock All", VertexAsk::UnlockAll, "Unlock every vertex of every open tile.", terrain.locked > 0),
        ] {
            let disabled_why = match ask {
                VertexAsk::UnlockAll => "No vertex on the open tiles is locked.",
                _ => "Select some vertices first.",
            };
            if ui
                .add_enabled(enabled, egui::Button::new(label))
                .on_hover_text(about)
                .on_disabled_hover_text(disabled_why)
                .clicked()
            {
                terrain.ask = Some(ask);
            }
        }
    });
    theme::note(
        ui,
        "No height operation moves a locked vertex: brush strokes, Grade, Stitch, chunk paste, \
         height import, and Level, Smooth, Tilt and Height here. Locks are saved in the project \
         at once and are not on the undo history.",
    );
}

/// What the Objects Follow switch does, for its hover here and in the grade
/// and road panels.
const OBJECTS_FOLLOW: &str = "When the ground moves, every doodad and WMO on it moves up or down by the \
                              same amount, in the same undo entry. Off: placements keep their stored \
                              height and the ground moves through them. Creature and game object spawns \
                              are rows in the world database and are not moved.";

/// The switch the height brush and the grade share: whether what stands on
/// the ground is carried when the ground moves. One function, so the two
/// panels word it alike.
pub(super) fn objects_follow(ui: &mut egui::Ui, on: &mut bool) {
    ui.checkbox(on, "Doodads and WMOs follow the ground").on_hover_text(OBJECTS_FOLLOW);
}

/// The height brush's inspector panel. The brush and the drawing tablet are
/// on the toolbar, [`toolbar`].
pub(crate) fn inspect(ui: &mut egui::Ui, _: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    panel(ui, &mut editing.terrain);
}
