//! The grade tool's panel. The state is [`crate::tools::ground::grade`].

use bevy_egui::egui;

use crate::session::EditSession;
use crate::ui::tools::ground::terrain::objects_follow;
use crate::ui::widgets::theme;

/// The grading tool: two points, a width, and the ramp between them.
///
/// The panel is mostly a report: which ends are picked, how long the grade is
/// and how steep. The slope decides whether the ramp is walkable, and two
/// clicks in the viewport do not show it.
pub(in crate::ui::tools) fn grading(
    ui: &mut egui::Ui,
    grading: &mut crate::tools::ground::grade::Grading,
    session: &mut EditSession,
) {
    use crate::tools::ground::grade;

    theme::heading(ui, "Grade");
    theme::note(
        ui,
        "Click a start, then an end; a third click starts over. Each end takes \
         the ground height at the click, and either height can be edited.",
    );

    ui.add_space(4.0);
    for (name, end) in [
        ("start", grading.from.as_mut()),
        ("end", grading.to.as_mut()),
    ] {
        theme::row(ui, name, |ui| match end {
            Some(end) => {
                ui.add(
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
                ui.label(theme::number(format!(
                    "{:>8.1} {:>8.1}",
                    end.at[0], end.at[1]
                )));
            }
            None => theme::note(ui, "not picked"),
        });
    }

    // The length and slope between the two ends, which the viewport does not
    // show.
    ui.add_space(2.0);
    match grading.grade().and_then(|g| g.slope().map(|s| (g, s))) {
        Some((grade, slope)) => {
            theme::row(ui, "length", |ui| {
                ui.label(theme::number(format!("{:.1} y", grade.length())));
            });
            theme::row(ui, "slope", |ui| {
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
                    theme::number(format!(
                        "{:.1}:1  ({degrees:.0}°)",
                        1.0 / slope.abs().max(1e-3)
                    ))
                    .color(colour),
                );
            });
        }
        None => theme::note(ui, "pick both ends to see the length and the slope"),
    }

    ui.add_space(6.0);
    theme::heading(ui, "Shape");
    theme::row(ui, "width", |ui| {
        ui.add(
            egui::Slider::new(&mut grading.width, 2.0..=200.0)
                .fixed_decimals(0)
                .suffix(" y"),
        )
        .on_hover_text("How wide the flat part of the ramp is, from its centre line to a side.");
    });
    theme::row(ui, "falloff", |ui| {
        ui.add(
            egui::Slider::new(&mut grading.falloff, 0.0..=200.0)
                .fixed_decimals(0)
                .suffix(" y"),
        )
        .on_hover_text(
            "How far past the width the ramp blends back into the surrounding ground. \
             Zero leaves a vertical face along both sides.",
        );
    });
    theme::row(ui, "strength", |ui| {
        ui.add(egui::Slider::new(&mut grading.strength, 0.05..=1.0).fixed_decimals(2))
            .on_hover_text(
                "How far toward the ramp the ground moves. Applied once per press, not \
                 accumulated over a stroke.",
            );
    });

    objects_follow(ui, &mut grading.objects_follow);

    ui.add_space(6.0);
    let ready = grading.grade().and_then(|g| g.slope()).is_some();
    if theme::primary(ui, "Apply grade").clicked() && ready {
        let moved = grade::apply(session, grading);
        grading.said = match moved {
            0 => "nothing moved: the ground already has that shape".to_string(),
            n => format!("{n} chunks graded"),
        };
        session.status = grading.said.clone();
    }
    if !ready {
        theme::note(ui, "pick two ends at different points");
    }
    ui.add_space(2.0);
    if ui.button("Clear ends").clicked() {
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
