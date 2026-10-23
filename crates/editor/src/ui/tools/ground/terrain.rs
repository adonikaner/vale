//! The terrain brush's panel, and the switch that moves placed objects with
//! the ground, which the grade and road panels repeat. The state is
//! [`crate::tools::ground::terrain`].

use bevy_egui::egui;

use crate::tools::ground::terrain::{FALLOFFS, MODES, SHAPES, Terrain};
use crate::tools::ground::terrain;
use crate::ui::widgets::theme;

use vale_edit::ops::brush::Mode;

/// The height brush.
pub(in crate::ui::tools) fn brush(ui: &mut egui::Ui, terrain: &mut Terrain) {
    // What the left button does: move the ground, or choose vertices. It is
    // above the brush because it decides which of the two panels follows.
    theme::heading(ui, "Pointer");
    theme::segmented(
        ui,
        &mut terrain.selecting,
        &[("Sculpt", false), ("Select vertices", true)],
        |a, b| a == b,
    );
    ui.add_space(4.0);
    if terrain.selecting {
        vertex_panel(ui, terrain);
        return;
    }

    theme::heading(ui, "Brush");
    theme::segmented(ui, &mut terrain.brush.mode, &MODES, |a, b| {
        // A flatten's target is set when the stroke begins, so the variant held
        // here carries a stale height and cannot be compared whole.
        std::mem::discriminant(a) == std::mem::discriminant(b)
    });

    ui.add_space(4.0);
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut terrain.brush.radius)
                .speed(0.5)
                .range(terrain::RADIUS)
                .suffix(" yd"),
        );
    });
    // The limits are the tool's own constants, which document the reason for
    // each. A range written as a literal in the panel would have no stated
    // reason.
    let (label, range, speed) = match terrain.brush.mode {
        Mode::Raise | Mode::Lower => ("strength", terrain::STRENGTH, 0.25),
        _ => ("strength", terrain::RATE, 0.05),
    };
    theme::row(ui, label, |ui| {
        ui.add(
            egui::DragValue::new(&mut terrain.brush.strength)
                .speed(speed)
                .range(range)
                .suffix(match terrain.brush.mode {
                    Mode::Raise | Mode::Lower => " yd/s",
                    _ => "/s",
                }),
        );
    });
    // The fraction of the radius that applies full strength: the inner ring
    // the preview draws. See `vale_edit::ops::brush::Falloff::over`.
    theme::row(ui, "core", |ui| {
        ui.add(
            egui::DragValue::new(&mut terrain.brush.core)
                .speed(0.01)
                .range(crate::tools::CORE)
                .fixed_decimals(2),
        );
    });
    // Shown for Noise only, the one mode that reads it. A field shown but
    // ignored in four modes of five would suggest that the panel's fields do
    // not all apply.
    if matches!(terrain.brush.mode, Mode::Noise) {
        theme::row(ui, "scale", |ui| {
            ui.add(
                egui::DragValue::new(&mut terrain.brush.scale)
                    .speed(0.5)
                    .range(terrain::SCALE)
                    .suffix(" yd"),
            );
        });
        theme::note(
            ui,
            "noise feature size; under 4 yd is finer than the vertex grid",
        );
    }

    ui.add_space(4.0);
    theme::heading(ui, "Falloff");
    theme::segmented(ui, &mut terrain.brush.falloff, &FALLOFFS, |a, b| a == b);

    ui.add_space(4.0);
    theme::heading(ui, "Shape");
    theme::segmented(ui, &mut terrain.brush.shape, &SHAPES, |a, b| a == b);
    theme::note(ui, "the square shape is aligned to the chunk grid");

    if matches!(terrain.brush.mode, Mode::Flatten { .. }) {
        flatten_panel(ui, terrain);
    }

    ui.add_space(4.0);
    theme::heading(ui, "Objects");
    objects_follow(ui, &mut terrain.objects_follow);

    // The selection, while there is one, and the locks, while there are any.
    // What is done with either is on the Select vertices half.
    if terrain.selected > 0 || terrain.locked > 0 {
        ui.add_space(4.0);
        theme::heading(ui, "Vertices");
        if terrain.selected > 0 {
            only_inside(ui, terrain);
        }
        if terrain.locked > 0 {
            theme::note(
                ui,
                format!("{} locked on the open tiles; strokes leave them in place", terrain.locked),
            );
        }
    }

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button paints");
    theme::note(ui, "ctrl + wheel resizes · shift + wheel the strength");
    theme::note(ui, "alt + wheel the core");
    theme::note(ui, "1 - 5 pick the mode · 6 - 0 the falloff");
    theme::note(ui, "shift + 1 · 2 · 3 pick the shape");
}

/// What a flatten goes to: which height, which way it may move the ground,
/// and whether the target is level or a tilted plane.
fn flatten_panel(ui: &mut egui::Ui, terrain: &mut Terrain) {
    use vale_edit::ops::brush::Only;
    ui.add_space(4.0);
    theme::heading(ui, "Flatten to");
    ui.checkbox(&mut terrain.flatten_to_cursor, "height under the pointer")
        .on_hover_text("Sampled where the stroke starts. Off: the height set below.");
    if !terrain.flatten_to_cursor {
        theme::row(ui, "height", |ui| {
            ui.add(
                egui::DragValue::new(&mut terrain.flatten_height)
                    .speed(0.25)
                    .fixed_decimals(1)
                    .suffix(" yd"),
            );
        });
    }
    theme::segmented(
        ui,
        &mut terrain.brush.only,
        &[("Both", Only::Both), ("Fill", Only::Fill), ("Cut", Only::Cut)],
        |a, b| a == b,
    );
    theme::note(
        ui,
        match terrain.brush.only {
            Only::Both => "raises low ground and lowers high ground",
            Only::Fill => "only raises low ground",
            Only::Cut => "only lowers high ground",
        },
    );
    tilt_rows(ui, &mut terrain.tilt_angle, &mut terrain.tilt_toward);
    if terrain.tilt_angle > 0.0 {
        theme::note(ui, "the slope pivots where the stroke starts");
    }
}

/// A tilt's two numbers: how steep, and which way is up. Answers whether
/// either changed.
fn tilt_rows(ui: &mut egui::Ui, angle: &mut f32, toward: &mut f32) -> bool {
    let mut changed = false;
    theme::row(ui, "tilt", |ui| {
        changed |= ui.add(
            egui::DragValue::new(angle)
                .speed(0.25)
                .range(0.0..=60.0)
                .fixed_decimals(0)
                .suffix("\u{b0}"),
        )
        .on_hover_text("Slope in degrees. 0 is level.")
        .changed();
        if *angle == 0.0 {
            theme::note(ui, "level");
        }
    });
    // The bearing only matters once there is a slope.
    if *angle > 0.0 {
        theme::row(ui, "uphill", |ui| {
            changed |= ui.add(
                egui::DragValue::new(&mut *toward)
                    .speed(1.0)
                    .range(0.0..=359.0)
                    .fixed_decimals(0)
                    .suffix("\u{b0}"),
            )
            .on_hover_text("Compass bearing of the uphill side. 0 is north, 90 east.")
            .changed();
            theme::note(ui, compass(*toward));
        });
    }
    changed
}

/// The nearest of the eight compass points to a bearing in degrees.
fn compass(bearing: f32) -> &'static str {
    const POINTS: [&str; 8] = ["north", "north-east", "east", "south-east", "south", "south-west", "west", "north-west"];
    POINTS[((bearing.rem_euclid(360.0) / 45.0).round() as usize) % 8]
}

/// Whether brush strokes move only the selected vertices.
fn only_inside(ui: &mut egui::Ui, terrain: &mut Terrain) {
    ui.checkbox(&mut terrain.only_inside, "Only inside the selection").on_hover_text(
        "On: a brush stroke moves the selected vertices and nothing else, and the \
         selection is drawn green. Off: strokes move any vertex under the brush that \
         is not locked. To keep vertices from moving, lock them.",
    );
}

/// The locked vertices: how many the open tiles hold, and the four ways to
/// change them.
fn vertex_locks(ui: &mut egui::Ui, terrain: &mut Terrain) {
    use crate::tools::ground::terrain::VertexAsk;
    let any = terrain.selected > 0;
    theme::note(
        ui,
        match terrain.locked {
            0 => "none on the open tiles".to_string(),
            n => format!("{n} on the open tiles, drawn red"),
        },
    );
    ui.horizontal_wrapped(|ui| {
        for (label, ask, about, enabled) in [
            (
                "Lock selection",
                VertexAsk::Lock,
                "Add the selected vertices to their tiles' locked vertices.",
                any,
            ),
            (
                "Unlock selection",
                VertexAsk::Unlock,
                "Remove the selected vertices from their tiles' locked vertices.",
                any,
            ),
            (
                "Lock tile sides",
                VertexAsk::LockSides,
                "Lock the vertices on all four sides of each tile that holds part of the \
                 selection. A tile's side is shared with the tile beside it, so this keeps \
                 the ground at the seam level with a tile that is not being edited.",
                any,
            ),
            (
                "Unlock all",
                VertexAsk::UnlockAll,
                "Unlock every vertex of every open tile.",
                terrain.locked > 0,
            ),
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
        "no height operation moves a locked vertex: brush strokes, Grade, Stitch, \
         chunk paste, height import, and Level, Smooth, Tilt and height here. Locks \
         are saved in the project at once and are not on the undo history.",
    );
}

/// The terrain tool's panel while the left button selects vertices: the
/// footprint a press selects, and what is done with the selection.
fn vertex_panel(ui: &mut egui::Ui, terrain: &mut Terrain) {
    use crate::tools::ground::terrain::VertexAsk;
    theme::heading(ui, "Footprint");
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut terrain.brush.radius)
                .speed(0.5)
                .range(terrain::RADIUS)
                .suffix(" yd"),
        );
    });
    theme::segmented(ui, &mut terrain.brush.shape, &SHAPES, |a, b| a == b);

    ui.add_space(4.0);
    theme::heading(ui, "Selection");
    let any = terrain.selected > 0;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(match terrain.selected {
                0 => "nothing selected".to_string(),
                n => format!("{n} selected"),
            })
            .color(match any {
                true => theme::INK,
                false => theme::INK_DIM,
            }),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(any, egui::Button::new("Clear").small())
                .clicked()
            {
                terrain.ask = Some(VertexAsk::Clear);
            }
        });
    });
    // Dragging the mean height moves every selected vertex by the same
    // amount, so the shape they make is kept.
    theme::row(ui, "height", |ui| {
        let mut mean = terrain.selected_mean;
        let moved = ui
            .add_enabled(
                any,
                egui::DragValue::new(&mut mean)
                    .speed(0.1)
                    .fixed_decimals(1)
                    .suffix(" yd"),
            )
            .on_hover_text("Mean height. Drag or type to move the selection up or down.")
            .on_disabled_hover_text("Select some vertices first.");
        if moved.changed() && mean != terrain.selected_mean {
            terrain.ask = Some(VertexAsk::MoveTo(mean));
        }
    });
    ui.horizontal(|ui| {
        for (label, ask, about) in [
            ("Level", VertexAsk::Level, "Flatten the selection at its mean height."),
            (
                "Smooth",
                VertexAsk::Smooth,
                "Smooth the selection. Click again to smooth more.",
            ),
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

    ui.add_space(4.0);
    theme::heading(ui, "Tilt");
    // The selection follows the sliders, pivoting on its centre.
    let [angle, toward] = &mut terrain.vertex_tilt;
    if tilt_rows(ui, angle, toward) && any {
        terrain.ask = Some(VertexAsk::Tilt);
    }
    ui.checkbox(&mut terrain.tilt_flat, "flatten onto the slope")
        .on_hover_text("Off: the selection tilts and keeps its relief. On: it becomes a flat plane at the tilt.");
    theme::note(
        ui,
        match any {
            true => "changing these tilts the selection",
            false => "select some vertices to tilt them",
        },
    );

    ui.add_space(4.0);
    theme::heading(ui, "Locked vertices");
    vertex_locks(ui, terrain);

    ui.add_space(4.0);
    theme::heading(ui, "Brush");
    only_inside(ui, terrain);
    objects_follow(ui, &mut terrain.objects_follow);

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button selects \u{b7} with shift, deselects");
    theme::note(ui, "ctrl + wheel resizes the footprint");
    theme::note(ui, "shift + 1 \u{b7} 2 \u{b7} 3 pick the shape");
}

/// The switch the height brush and the grade share: whether what stands on
/// the ground is carried when the ground moves. One function, so the two
/// panels word it alike.
pub(super) fn objects_follow(ui: &mut egui::Ui, on: &mut bool) {
    ui.checkbox(on, "Doodads and WMOs follow the ground").on_hover_text(
        "When the ground moves, every doodad and WMO on it moves up or down by the \
         same amount, in the same undo entry. Off: placements keep their stored \
         height and the ground moves through them. Creature and game object spawns \
         are rows in the world database and are not moved.",
    );
}

/// The height brush's inspector panel, with the tablet's settings under it.
pub(crate) fn inspect(ui: &mut egui::Ui, _: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    brush(ui, &mut editing.terrain);
    crate::ui::tools::tablet::tablet(ui, &mut editing.tablet, &editing.pen);
}
