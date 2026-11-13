//! The terrain brush's controls, in two places, and the switch that moves
//! placed objects with the ground, which the grade and road panels repeat. The
//! state is [`crate::tools::ground::terrain`].
//!
//! ```text
//! viewport toolbar   Sculpt or Select Vertices, Only Inside Selection,
//!                    Objects Follow and the Keys button; under them, in
//!                    Sculpt the brush preset and the ten modes, and in
//!                    Select Vertices the footprint
//! inspector          in Sculpt the brush presets, the brush's settings,
//!                    Flatten's target, and the settings of Terrace, Erode,
//!                    Mountain and Crevice; the vertex selection: its height,
//!                    Level and Smooth, its tilt, and the locked vertices
//! ```
//!
//! The toolbar holds what decides what a click does and the switches about
//! the session. The brush's settings are in the inspector beside the presets,
//! because a preset is those settings: they are what Save writes into it. The
//! presets are [`crate::prefs::brushes`]; their grid is [`super::presets`].

use bevy_egui::egui;

use crate::prefs::brushes::{Ask, Brushes, Sculpt};
use crate::tools::ground::terrain::{FALLOFFS, MODES, SHAPES, Terrain, VertexAsk};
use crate::tools::ground::terrain;
use crate::ui::widgets::theme::{self, bar_group, bar_keys, bar_segment, bar_setting, bar_switch, field, property};

use vale_edit::ops::brush::{Erosion, Mode, Only};

/// The terrain tool's toolbar over the viewport:
///
/// ```text
/// [Sculpt | Select Vertices]  Brush  Only Inside Selection  Objects Follow  Keys
/// [Raise | Lower | Flatten | Smooth | Noise | Blur | Terrace | Erode | Mountain | Crevice]
/// ```
///
/// In Select Vertices the first line has no Brush and the second is the
/// footprint's radius and shape. Only Inside Selection is always drawn and
/// is unavailable while no vertex is selected; the tool switches it off
/// when the selection becomes empty (`tools::ground::terrain::vertex_asks`).
/// A set of which one is chosen is a [`theme::bar_group`]; a switch is a
/// [`theme::bar_switch`]. See [`theme::toolbar`].
pub(crate) fn toolbar(
    ctx: &egui::Context,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    terrain: &mut Terrain,
    brushes: &mut Brushes,
) -> Option<egui::Rect> {
    // While no preset is chosen, the one equal to the brush is. See
    // `crate::prefs::brushes`.
    brushes.follow_sculpt(terrain);
    theme::toolbar(ctx, "terrain-toolbar", viewport, corner, |ui| {
        // The first row: what the left button does, and the switches that
        // apply to either answer.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            bar_group(ui, |ui| {
                if bar_segment(ui, !terrain.selecting, "Sculpt")
                    .on_hover_text("The left button moves the ground with the brush.")
                    .clicked()
                {
                    terrain.selecting = false;
                }
                if bar_segment(ui, terrain.selecting, "Select Vertices")
                    .on_hover_text(
                        "The left button selects the vertices under the footprint, for the inspector \
                         to move, level, smooth, tilt or lock.",
                    )
                    .clicked()
                {
                    terrain.selecting = true;
                }
            });
            if !terrain.selecting {
                let names: Vec<&str> = brushes.sculpt.list.iter().map(|preset| preset.name.as_str()).collect();
                let modified = brushes.sculpt.modified(&Sculpt::of(terrain));
                let picked = bar_setting(ui, "Brush", |ui| {
                    super::presets::picker(ui, "terrain-brush", &names, brushes.sculpt.chosen, modified)
                });
                if let Some(at) = picked {
                    brushes.act_sculpt(Ask::Choose(at), terrain);
                }
            }
            let only_inside = ui
                .add_enabled_ui(terrain.selected > 0, |ui| {
                    bar_switch(ui, terrain.only_inside, "Only Inside Selection")
                })
                .inner
                .on_hover_text(
                    "On: a brush stroke moves the selected vertices and nothing else, and the \
                     selection is drawn green. Off: strokes move any vertex under the brush that \
                     is not locked. To keep vertices from moving, lock them.",
                )
                .on_disabled_hover_text(
                    "No vertex is selected. Choose Select Vertices and paint over some ground first.",
                );
            if only_inside.clicked() {
                terrain.only_inside = !terrain.only_inside;
            }
            objects_follow(ui, &mut terrain.objects_follow);
            bar_keys(ui, match terrain.selecting {
                false => &SCULPT_KEYS,
                true => &SELECT_KEYS,
            });
        });
        ui.add_space(2.0);
        // The second row: the brush's mode in Sculpt, the footprint in Select
        // Vertices.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            match terrain.selecting {
                false => bar_group(ui, |ui| {
                    for (key, (name, mode)) in MODES.into_iter().enumerate() {
                        // A flatten's target is set when the stroke begins, so
                        // the variant held here carries a stale height and is
                        // compared on its discriminant.
                        let on = std::mem::discriminant(&terrain.brush.mode) == std::mem::discriminant(&mode);
                        if bar_segment(ui, on, name).on_hover_text(terrain::mode_key(key)).clicked() && !on {
                            terrain.brush.mode = mode;
                        }
                    }
                }),
                true => {
                    bar_setting(ui, "Footprint", |ui| {
                        ui.add(
                            egui::DragValue::new(&mut terrain.brush.radius)
                                .speed(0.5)
                                .range(terrain::RADIUS)
                                .suffix(" yd"),
                        )
                        .on_hover_text("The radius a click selects within. Ctrl + wheel.");
                    });
                    bar_setting(ui, "Shape", |ui| {
                        bar_group(ui, |ui| {
                            for (name, shape) in SHAPES {
                                let on = terrain.brush.shape == shape;
                                if bar_segment(ui, on, name)
                                    .on_hover_text("The footprint's outline. Shift + 1, 2 or 3.")
                                    .clicked()
                                {
                                    terrain.brush.shape = shape;
                                }
                            }
                        });
                    });
                }
            }
        });
    })
}

/// The Sculpt mode's mouse and keyboard controls, for the toolbar's Keys
/// button.
const SCULPT_KEYS: [(&str, &str); 8] = [
    ("Left button", "Apply the brush"),
    ("Ctrl + wheel", "Radius"),
    ("Shift + wheel", "Strength"),
    ("Alt + wheel", "Core"),
    ("1 to 5", "Raise, Lower, Flatten, Smooth, Noise"),
    ("Shift + 4 to 8", "Blur, Terrace, Erode, Mountain, Crevice"),
    ("6 to 0", "Falloff"),
    ("Shift + 1 to 3", "Shape"),
];

/// The Select Vertices mode's controls, for the same button.
const SELECT_KEYS: [(&str, &str); 4] = [
    ("Left button", "Select the vertices under the footprint"),
    ("Shift + left button", "Deselect them"),
    ("Ctrl + wheel", "Footprint radius"),
    ("Shift + 1 to 3", "Footprint shape"),
];

/// The Brush Settings section's rows: radius, strength, core, the scale for
/// the modes that read it, falloff and shape. The section is
/// [`super::presets::settings`].
///
/// A row is shown only for the modes that read it. A row shown but ignored
/// would suggest that the settings do not all apply.
fn brush_settings(ui: &mut egui::Ui, terrain: &mut Terrain) {
    let mode = terrain.brush.mode;
    property(ui, "Radius", "The outer ring. Ctrl + wheel over the ground.", |ui| {
        field(ui, egui::DragValue::new(&mut terrain.brush.radius).speed(0.5).range(terrain::RADIUS).suffix(" yd"));
    });
    // The limits are the tool's own constants, which document the reason for
    // each. A range written as a literal in the panel would have no stated
    // reason.
    let (range, speed, suffix) = match mode.climbs() {
        true => (terrain::STRENGTH, 0.25, " yd/s"),
        false => (terrain::RATE, 0.05, "/s"),
    };
    let about = match (mode, terrain.brush.erosion) {
        (Mode::Erode, Erosion::Water) => {
            "Drops of water a second for each vertex under the brush. Shift + wheel over the ground."
        }
        (Mode::Erode, Erosion::Thermal) => "Collapse passes a second. Shift + wheel over the ground.",
        (Mode::Crevice, _) => "How fast the trench deepens at its centre. Shift + wheel over the ground.",
        _ => "How fast a held stroke works. Shift + wheel over the ground.",
    };
    property(ui, "Strength", about, |ui| {
        field(ui, egui::DragValue::new(&mut terrain.brush.strength).speed(speed).range(range).suffix(suffix));
    });
    // Crevice has a fixed profile, so the core and falloff do not apply.
    let shaped = !matches!(mode, Mode::Crevice);
    // The fraction of the radius that applies full strength: the inner ring
    // the preview draws. See `vale_edit::ops::brush::Falloff::over`.
    if shaped {
        property(
            ui,
            "Core",
            "The share of the radius at full strength, drawn as the inner ring. Alt + wheel over the ground.",
            |ui| {
                field(ui, egui::DragValue::new(&mut terrain.brush.core).speed(0.01).range(crate::tools::CORE).fixed_decimals(2));
            },
        );
    }
    let scale = match mode {
        Mode::Noise => Some("The noise's feature size. Under 4 yd is finer than the vertex grid."),
        Mode::Mountain => Some("The distance between ridges."),
        Mode::Blur => Some("Bumps narrower than this are removed; wider ground keeps its shape."),
        _ => None,
    };
    if let Some(about) = scale {
        property(ui, "Scale", about, |ui| {
            field(ui, egui::DragValue::new(&mut terrain.brush.scale).speed(0.5).range(terrain::SCALE).suffix(" yd"));
        });
    }
    if shaped {
        property(ui, "Falloff", "How the strength fades from the inner ring to the outer ring. Keys 6 to 0.", |ui| {
            theme::choice(ui, "terrain-falloff", &mut terrain.brush.falloff, &FALLOFFS, |a, b| a == b);
        });
    }
    property(
        ui,
        "Shape",
        "The brush's outline. The square is aligned to the chunk grid. Shift + 1, 2 or 3.",
        |ui| {
            // A drop-down list and not three segments: at the inspector's
            // narrowest the three names do not fit the row.
            theme::choice(ui, "terrain-shape", &mut terrain.brush.shape, &SHAPES, |a, b| a == b);
        },
    );
}

/// The Flatten section, while the mode is Flatten: which height, which way
/// the ground may move, and whether the target is level or a tilted plane.
fn flatten_settings(ui: &mut egui::Ui, terrain: &mut Terrain) {
    theme::heading(ui, "Flatten");
    property(ui, "Target", "Where the height the brush flattens to comes from.", |ui| {
        ui.checkbox(&mut terrain.flatten_to_cursor, "Height under the pointer")
            .on_hover_text("Sampled where the stroke starts. Off: the height set below.");
    });
    // Always drawn, and unavailable while the target is the pointer's height:
    // a row that appears when a box is cleared moves every row under it.
    property(ui, "Height", "The height the brush flattens to, while Target is off.", |ui| {
        ui.add_enabled_ui(!terrain.flatten_to_cursor, |ui| {
            field(ui, egui::DragValue::new(&mut terrain.flatten_height).speed(0.25).fixed_decimals(1).suffix(" yd"))
        })
        .inner
        .on_disabled_hover_text("Not read while the target is the height under the pointer.");
    });
    property(ui, "Direction", "Which way the ground may move toward the target.", |ui| {
        theme::property_segmented(
            ui,
            &mut terrain.brush.only,
            &[("Both", Only::Both), ("Fill", Only::Fill), ("Cut", Only::Cut)],
            |a, b| a == b,
        );
    });
    tilt_rows(ui, &mut terrain.tilt_angle, &mut terrain.tilt_toward);
    theme::note(
        ui,
        match terrain.brush.only {
            Only::Both => "Raises low ground and lowers high ground. The slope pivots where the stroke starts.",
            Only::Fill => "Only raises low ground. The slope pivots where the stroke starts.",
            Only::Cut => "Only lowers high ground. The slope pivots where the stroke starts.",
        },
    );
}

/// The section for Terrace, Erode, Mountain or Crevice: the settings only
/// that mode reads. Nothing for the other modes.
fn mode_settings(ui: &mut egui::Ui, terrain: &mut Terrain) {
    let brush = &mut terrain.brush;
    match brush.mode {
        Mode::Terrace { .. } => {
            theme::heading(ui, "Terrace");
            property(ui, "Step", "The height of one step.", |ui| {
                field(ui, egui::DragValue::new(&mut brush.step).speed(0.1).range(terrain::STEP).suffix(" yd"));
            });
            property(ui, "Sharpness", "How much of each step is level. 1: level steps with vertical risers. 0: a smooth slope of steps.", |ui| {
                field(ui, egui::DragValue::new(&mut brush.sharpness).speed(0.01).range(0.0..=1.0).fixed_decimals(2));
            });
            theme::note(ui, "The steps are measured from the height where the stroke starts, so that point is on a step.");
        }
        Mode::Erode => {
            theme::heading(ui, "Erode");
            property(ui, "Type", "What wears the ground.", |ui| {
                theme::property_segmented(
                    ui,
                    &mut brush.erosion,
                    &[("Water", Erosion::Water), ("Thermal", Erosion::Thermal)],
                    |a, b| a == b,
                );
            });
            if brush.erosion == Erosion::Thermal {
                property(ui, "Talus", "The steepest slope left. Steeper slopes collapse to it.", |ui| {
                    field(ui, egui::DragValue::new(&mut brush.talus).speed(0.25).range(5.0..=80.0).fixed_decimals(0).suffix("\u{b0}"));
                });
            }
            theme::note(
                ui,
                match brush.erosion {
                    Erosion::Water => "Drops of water run downhill from inside the brush, cutting gullies and leaving the ground they carry lower down.",
                    Erosion::Thermal => "Slopes steeper than the talus angle collapse, moving ground from the top of each slope to its foot.",
                },
            );
        }
        Mode::Mountain => {
            theme::heading(ui, "Mountain");
            property(ui, "Ridges", "How much of the height follows the ridges. 0 raises like Raise; 1 raises only along the ridges.", |ui| {
                field(ui, egui::DragValue::new(&mut brush.ridges).speed(0.01).range(0.0..=1.0).fixed_decimals(2));
            });
        }
        Mode::Crevice => {
            theme::heading(ui, "Crevice");
            property(ui, "Lip", "The share of the dug ground piled on the rims. 1 keeps all of it; 0 piles none.", |ui| {
                field(ui, egui::DragValue::new(&mut brush.lip).speed(0.01).range(0.0..=1.0).fixed_decimals(2));
            });
            theme::note(ui, "The trench is the inner half of the brush and the rims are the outer half.");
        }
        _ => {}
    }
}

/// A tilt's two numbers as property rows: how steep, and which way is up.
/// Answers whether either changed.
fn tilt_rows(ui: &mut egui::Ui, angle: &mut f32, toward: &mut f32) -> bool {
    let mut changed = false;
    // Each field writes what its number means after the number: "0° Level",
    // "135° South-east". The parser reads the number and ignores the rest, so
    // the text can be typed over or edited in place.
    property(ui, "Tilt", "Slope in degrees. 0 is level.", |ui| {
        changed |= field(
            ui,
            egui::DragValue::new(angle)
                .speed(0.25)
                .range(0.0..=60.0)
                .custom_formatter(|degrees, _| match degrees == 0.0 {
                    true => "0\u{b0}  Level".to_string(),
                    false => format!("{degrees:.0}\u{b0}"),
                })
                .custom_parser(leading_number),
        )
        .changed();
    });
    // Drawn at a tilt of 0 as well, where it changes nothing yet: the bearing
    // can be set before the slope, and the rows under it do not move when
    // the tilt leaves 0.
    property(
        ui,
        "Uphill",
        "Compass bearing of the uphill side. 0 is north, 90 east. It has no effect while Tilt is 0.",
        |ui| {
            changed |= field(
                ui,
                egui::DragValue::new(&mut *toward)
                    .speed(1.0)
                    .range(0.0..=359.0)
                    .custom_formatter(|degrees, _| format!("{degrees:.0}\u{b0}  {}", compass(degrees as f32)))
                    .custom_parser(leading_number),
            )
            .changed();
        },
    );
    changed
}

/// The number a text starts with, for a field whose text continues after its
/// number. `None` when it starts with none.
fn leading_number(text: &str) -> Option<f64> {
    let text = text.trim_start();
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(text.len());
    text[..end].parse().ok()
}

/// The nearest of the eight compass points to a bearing in degrees.
fn compass(bearing: f32) -> &'static str {
    const POINTS: [&str; 8] = ["North", "North-east", "East", "South-east", "South", "South-west", "West", "North-west"];
    POINTS[((bearing.rem_euclid(360.0) / 45.0).round() as usize) % 8]
}

/// The terrain tool's inspector. In Sculpt: the presets, the brush's
/// settings, Flatten's target, and the selection described; in Select
/// Vertices: the selection and its tilt. Both end with the locked vertices.
///
/// The panel has no title or instruction line for the mode: the toolbar's
/// lit segment names the mode, and each of its controls says what it does
/// when hovered.
fn panel(ui: &mut egui::Ui, terrain: &mut Terrain, brushes: &mut Brushes) {
    match terrain.selecting {
        true => {
            selection(ui, terrain);
            tilt(ui, terrain);
        }
        false => {
            let presets: Vec<(String, super::presets::Icon)> = brushes
                .sculpt
                .list
                .iter()
                .map(|preset| (preset.name.clone(), super::presets::Icon::of_sculpt(&preset.settings)))
                .collect();
            let modified = brushes.sculpt.modified(&Sculpt::of(terrain));
            if let Some(ask) = super::presets::section(ui, "terrain-brushes", &presets, brushes.sculpt.chosen, modified) {
                brushes.act_sculpt(ask, terrain);
            }
            super::presets::settings(ui, "terrain-brush-settings", |ui| {
                brush_settings(ui, terrain);
                if matches!(terrain.brush.mode, Mode::Flatten { .. }) {
                    flatten_settings(ui, terrain);
                }
                mode_settings(ui, terrain);
            });
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
            .add_enabled_ui(any, |ui| field(ui, egui::DragValue::new(&mut mean).speed(0.1).fixed_decimals(1).suffix(" yd")))
            .inner
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
    theme::heading_about(
        ui,
        "Locked Vertices",
        "No height operation moves a locked vertex: brush strokes, Grade, Stitch, chunk paste, \
         height import, and Level, Smooth, Tilt and Height here.\n\nLocks are saved in the \
         project at once and are not on the undo history. Locked vertices are drawn red.",
    );
    let any = terrain.selected > 0;
    property(ui, "Locked", "", |ui| {
        ui.label(
            egui::RichText::new(match terrain.locked {
                0 => "None on the open tiles".to_string(),
                n => format!("{n} on the open tiles"),
            })
            .color(theme::INK),
        );
    });
    let buttons = [
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
    ];
    // Two rows of two equal buttons, so the four have common edges.
    for pair in buttons.chunks(2) {
        ui.columns(2, |columns| {
            for (column, (label, ask, about, enabled)) in columns.iter_mut().zip(pair) {
                let disabled_why = match ask {
                    VertexAsk::UnlockAll => "No vertex on the open tiles is locked.",
                    _ => "Select some vertices first.",
                };
                let width = column.available_width();
                if column
                    .add_enabled(*enabled, egui::Button::new(*label).min_size(egui::vec2(width, 24.0)))
                    .on_hover_text(*about)
                    .on_disabled_hover_text(disabled_why)
                    .clicked()
                {
                    terrain.ask = Some(*ask);
                }
            }
        });
    }
}

/// What the Objects Follow switch does, for its hover text.
const OBJECTS_FOLLOW: &str = "When the ground moves, every doodad and WMO on it moves up or down by the \
                              same amount, in the same undo entry. Off: placements keep their stored \
                              height and the ground moves through them. Creature and game object spawns \
                              are rows in the world database and are not moved.";

/// The Objects Follow switch of the terrain, grade and road toolbars: whether
/// what stands on the ground is carried when the ground moves. One function,
/// so the three toolbars word it alike.
pub(super) fn objects_follow(ui: &mut egui::Ui, on: &mut bool) {
    if bar_switch(ui, *on, "Objects Follow").on_hover_text(OBJECTS_FOLLOW).clicked() {
        *on = !*on;
    }
}

/// The height brush's inspector panel. The modes are on the toolbar,
/// [`toolbar`].
pub(crate) fn inspect(ui: &mut egui::Ui, _: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    panel(ui, &mut editing.terrain, &mut editing.brushes);
}
