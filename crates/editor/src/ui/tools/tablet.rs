//! The drawing tablet section of the brush panels. The state is `crate::tablet`.

use bevy_egui::egui;

use crate::ui::widgets::theme;

/// The drawing tablet settings, and what the pen is doing.
///
/// The settings apply to every brush and are written to one file, so the
/// section is the same on each brush that reads them. See [`crate::tablet`].
pub(super) fn tablet(ui: &mut egui::Ui, tablet: &mut crate::tablet::Tablet, pen: &crate::tablet::Pen) {
    ui.add_space(6.0);
    theme::heading(ui, "Drawing tablet");
    let mut changed = false;
    changed |= ui
        .checkbox(&mut tablet.pressure_strength, "Pen pressure scales strength")
        .on_hover_text(
            "While the pen tip is on the tablet, the brush's strength is multiplied by the \
             pressure, 0 to 1. A mouse stroke is not affected.",
        )
        .changed();
    changed |= ui
        .checkbox(&mut tablet.pressure_radius, "Pen pressure scales radius")
        .on_hover_text(
            "While the pen tip is on the tablet, the brush's radius is multiplied by the \
             pressure, 0 to 1. The ring shows the radius in use.",
        )
        .changed();
    changed |= ui
        .checkbox(&mut tablet.pointer_position, "Motion from cursor position")
        .on_hover_text(
            "Right-drag turns the view by the change in the cursor's position instead of \
             raw mouse motion. A tablet reports raw motion as absolute positions, which \
             spin the view. On automatically while a Windows Ink pen is in range; turn it \
             on for a tablet driver with Windows Ink switched off.",
        )
        .changed();
    if changed {
        tablet.save();
    }
    theme::note(
        ui,
        match (pen.hooked, pen.seen, pen.in_range, pen.contact) {
            (false, ..) => "pen input is read on Windows only".to_string(),
            (true, false, ..) => {
                "no Windows Ink pen seen yet; pressure needs Windows Ink on in the tablet driver"
                    .to_string()
            }
            (true, true, _, true) => format!("pen on the tablet, pressure {:.2}", pen.pressure),
            (true, true, true, false) => "pen in range".to_string(),
            (true, true, false, false) => "pen out of range".to_string(),
        },
    );
    theme::note(ui, "pen tip clicks · barrel button held turns the view");
}
