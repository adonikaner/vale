//! The grade tool's panel. The state is [`crate::tools::ground::grade`].

use bevy_egui::egui;

use crate::session::EditSession;
use crate::ui::tools::ground::terrain::objects_follow;
use crate::ui::widgets::theme::{self, field, property};

/// The grading tool: two points, a width, and the ramp between them.
///
/// The panel is mostly a report: which ends are picked, how long the grade is
/// and how steep. The slope decides whether the ramp is walkable, and two
/// clicks in the viewport do not show it.
///
/// How the ends are picked is on the Ends heading's info mark. The tool has
/// no toolbar over the viewport to hold it.
pub(in crate::ui::tools) fn grading(
    ui: &mut egui::Ui,
    grading: &mut crate::tools::ground::grade::Grading,
    session: &mut EditSession,
) {
    use crate::tools::ground::grade;

    theme::heading_about(
        ui,
        "Ends",
        "Click a start, then an end; a third click starts over. Each end takes \
         the ground height at the click, and either height can be edited.\n\n\
         The length and the slope are shown once both ends are picked.",
    );
    for (name, end) in [
        ("Start", grading.from.as_mut()),
        ("End", grading.to.as_mut()),
    ] {
        match end {
            Some(end) => {
                property(ui, name, "", |ui| {
                    field(
                        ui,
                        egui::DragValue::new(&mut end.height)
                            .speed(0.25)
                            .suffix(" z")
                            .fixed_decimals(1),
                    )
                    .on_hover_text(
                        "The height of this end of the ramp, taken from the ground at the \
                         click. Edit it to grade to a height that has no ground at it, \
                         such as a terrace or a bridge deck.",
                    );
                });
                // Where the end is, on a line of its own: beside the height
                // the two coordinates are wider than the row.
                theme::note(ui, format!("At {:.1}, {:.1}", end.at[0], end.at[1]));
            }
            None => property(ui, name, "", |ui| {
                ui.label(egui::RichText::new("Not picked").color(theme::INK_FAINT));
            }),
        }
    }

    // The length and slope between the two ends, which the viewport does not
    // show.
    if let Some((grade, slope)) = grading.grade().and_then(|g| g.slope().map(|s| (g, s))) {
        property(ui, "Length", "", |ui| {
            theme::value(ui, format!("{:.1} y", grade.length()));
        });
        property(ui, "Slope", "", |ui| {
            let degrees = slope.atan().to_degrees();
            // 1.12's limit is about 50 degrees; above it a character
            // slides rather than climbs. The colour marks slopes near and
            // over that limit.
            let colour = match degrees.abs() {
                d if d > 50.0 => theme::BAD,
                d if d > 35.0 => theme::WARN,
                _ => theme::INK,
            };
            ui.label(
                egui::RichText::new(format!("{:.1}:1  ({degrees:.0}\u{b0})", 1.0 / slope.abs().max(1e-3)))
                    .color(colour),
            );
        });
    }

    theme::heading(ui, "Shape");
    // The three were sliders. A field's drag speed is chosen for its range:
    // half a yard a point for the two distances, a hundredth for the
    // strength.
    property(ui, "Width", "", |ui| {
        field(
            ui,
            egui::DragValue::new(&mut grading.width)
                .speed(0.5)
                .range(2.0..=200.0)
                .fixed_decimals(0)
                .suffix(" yd"),
        )
        .on_hover_text("How wide the flat part of the ramp is, from its centre line to a side.");
    });
    property(ui, "Falloff", "", |ui| {
        field(
            ui,
            egui::DragValue::new(&mut grading.falloff)
                .speed(0.5)
                .range(0.0..=200.0)
                .fixed_decimals(0)
                .suffix(" yd"),
        )
        .on_hover_text(
            "How far past the width the ramp blends back into the surrounding ground. \
             Zero leaves a vertical face along both sides.",
        );
    });
    property(ui, "Strength", "", |ui| {
        field(
            ui,
            egui::DragValue::new(&mut grading.strength)
                .speed(0.01)
                .range(0.05..=1.0)
                .fixed_decimals(2),
        )
        .on_hover_text(
            "How far toward the ramp the ground moves. Applied once per press, not \
             accumulated over a stroke.",
        );
    });

    objects_follow(ui, &mut grading.objects_follow);

    ui.add_space(6.0);
    let ready = grading.grade().and_then(|g| g.slope()).is_some();
    if theme::primary(ui, "Apply Grade").clicked() && ready {
        let moved = grade::apply(session, grading);
        grading.said = match moved {
            0 => "Nothing moved: the ground already has that shape".to_string(),
            n => format!("{n} chunks graded"),
        };
        session.status = grading.said.clone();
    }
    if !ready {
        theme::note(ui, "Pick two ends at different points.");
    }
    ui.add_space(2.0);
    if ui.button("Clear Ends").clicked() {
        grading.clear();
    }
    if !grading.said.is_empty() {
        ui.add_space(4.0);
        theme::note(ui, grading.said.clone());
    }
}

/// The grade tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, .. } = subject;
    grading(ui, &mut editing.grading, session);
}
