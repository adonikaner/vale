//! **What is in the frame at all** — the fifteen subtractions, and the hour
//! they are lit by.
//!
//! Every switch here takes a layer *away*, which is a different instrument from
//! the seven on the render tab and from the six drawn over the top of them.
//! [`crate::render::tuning`] carries the whole argument for why: an attribution
//! in this project has always been a pair of runs differing in one layer, and
//! until these existed on a panel that meant a rebuild or a relaunch per
//! comparison.
//!
//! The clock is here rather than on the render tab because it is the same kind
//! of question — *what is in this frame* — asked about the light: "what does
//! this look like at dusk" and "is this wrong or is it right for midnight" had
//! no answer but waiting for the server's own clock to come round, which at the
//! world's rate is up to twenty-four minutes.

use bevy_egui::egui;

use crate::render::lamps::LampCount;
use crate::render::night::{Night, NightTuning};
use crate::render::sky::WorldClock;
use crate::render::tuning::{WorldTuning, SWITCHES};
use vale_assets::tables::light::DAY;

use super::{DIM, GOOD, WARN};

/// How many half-minutes one step of the hour slider moves.
///
/// Ten, which is five minutes of world time: fine enough that the six sky stops
/// and the eighteen light bands can each be walked onto deliberately, coarse
/// enough that dragging the slider does not re-resolve the light chain on every
/// pixel of travel.
const STEP: u32 = 10;

pub fn show(
    ui: &mut egui::Ui,
    world: &mut WorldTuning,
    clock: &mut WorldClock,
    night: &mut NightTuning,
    resolved: &Night,
    lamps: &LampCount,
) {
    ui.strong("Time of day");
    // The latch first, because it is what the slider means. With it on, the
    // slider is a read-out of the server's clock and moving it takes the
    // override; with it off, the slider is the world's hour.
    let mut follow = clock.override_half_minutes.is_none();
    if ui.checkbox(&mut follow, "follow the server's clock").changed() {
        clock.override_half_minutes = if follow {
            None
        } else {
            // **Taken from where the clock stands**, so turning the latch off
            // changes nothing until the slider is moved — the alternative is a
            // sky that jumps the moment you look at the panel.
            Some(clock.half_minutes)
        };
    }
    let mut half_minutes = clock.half_minutes.min(DAY - 1);
    let hours = half_minutes / 120;
    let minutes = (half_minutes % 120) / 2;
    let slider = ui.add_enabled(
        !follow,
        egui::Slider::new(&mut half_minutes, 0..=DAY - 1)
            .step_by(f64::from(STEP))
            .show_value(false)
            .text(format!("{hours:02}:{minutes:02}")),
    );
    if slider.changed() {
        clock.override_half_minutes = Some(half_minutes);
        clock.half_minutes = half_minutes;
    }
    // The four hours worth naming, since finding dawn by dragging is most of
    // what the slider is used for. `Light.dbc`'s own bands turn over at these.
    ui.horizontal(|ui| {
        for (label, hour) in [
            ("00:00", 0u32),
            ("06:00", 6),
            ("12:00", 12),
            ("18:00", 18),
        ] {
            if ui.add_enabled(!follow, egui::Button::new(label)).clicked() {
                let at = hour * 120;
                clock.override_half_minutes = Some(at);
                clock.half_minutes = at;
            }
        }
    });
    if !follow {
        ui.colored_label(WARN, "The hour is set by hand. The server's clock is ignored.");
    }

    ui.add_space(10.0);
    ui.strong("Layers drawn");
    ui.colored_label(DIM, "every layer defaults on; --without takes the same names");
    // **Off the list rather than written out**, so a switch added to
    // [`WorldTuning`] cannot be one nobody can reach: `SWITCHES` is where the
    // two are kept from drifting, and its own test asserts that no two entries
    // share a field.
    //
    // Two columns, because fifteen checkboxes down one edge makes the tab
    // taller than the window.
    ui.columns(2, |columns| {
        for (index, (label, field)) in SWITCHES.iter().enumerate() {
            let ui = &mut columns[index % 2];
            ui.checkbox(field(world), *label);
        }
    });

    // **The one thing a row of checkboxes cannot say**: which of them is
    // currently subtracting something. A panel left with three things switched
    // off two sessions ago is the cheapest possible explanation for "the world
    // looks wrong".
    let off: Vec<&str> = SWITCHES
        .iter()
        .filter(|(_, field)| !*field(world))
        .map(|(label, _)| *label)
        .collect();
    ui.add_space(6.0);
    if off.is_empty() {
        ui.colored_label(GOOD, "Every layer is drawn.");
    } else {
        ui.colored_label(WARN, format!("Not drawn: {}.", off.join(", ")));
        // …and the scripted form of exactly this state, so a subtraction found
        // by clicking can be re-taken in a shot without anyone having to
        // remember the flag's spelling.
        ui.colored_label(
            DIM,
            format!(
                "--without {}",
                off.iter()
                    .map(|label| label.replace(' ', ""))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        );
    }

    ui.add_space(10.0);
    // Under its own heading rather than in the grid above because it adds
    // rather than subtracts. See [`crate::render::night`]: it is this client's
    // one deliberate deviation from the light table, and an option rather than
    // a correction.
    ui.strong("Night darkening");
    ui.colored_label(
        DIM,
        "darkens the night below what Light.dbc states; no effect in daylight",
    );
    ui.checkbox(&mut night.enabled, "darken the night");
    ui.add_enabled(
        night.enabled,
        egui::Slider::new(&mut night.strength, 0.0..=1.0).text("strength"),
    );
    // What the light chain currently makes of it, which the checkbox cannot
    // say: the grade is driven by how dark the table already is, so at noon it
    // reads 0% with the option fully on and nothing is wrong.
    if !night.enabled {
        ui.colored_label(DIM, "Off. The night is the light table's own, ungraded.");
    } else if resolved.factor <= 0.0 {
        ui.colored_label(
            DIM,
            "0%. The light here is daylight, so nothing is darkened.",
        );
    } else {
        ui.colored_label(
            GOOD,
            format!(
                "{:.0}% applied, mist deck at {:.0}y.",
                resolved.factor * 100.0,
                resolved.deck
            ),
        );
    }
    ui.colored_label(
        DIM,
        if night.enabled {
            format!("--night {:.2}", night.strength)
        } else {
            "--night off".to_string()
        },
    );

    // **The lamps are their own switch and their own cost**, which is why they
    // are not folded into the checkbox above: the grade is arithmetic in
    // shaders that were already running, and this is twenty-four point lights
    // Bevy has to assign to clusters every frame. Two runs differing in this
    // one box are what price them. See [`crate::render::lamps`].
    ui.add_space(6.0);
    ui.add_enabled_ui(night.enabled, |ui| {
        ui.checkbox(&mut night.lamps, "torches and fires cast light");
        // **Two pairs, because a lamp indoors and a lamp in the open want
        // different numbers** — see
        // [`crate::render::night::NightTuning::indoor_lamp_strength`]. Four
        // sliders rather than two is the point of the whole change and not an
        // accident of it: the two halves were reported as separate complaints
        // ("outdoor torches are dim", "indoors looks completely fake") and they
        // move in opposite directions.
        ui.add_enabled_ui(night.lamps, |ui| {
            ui.add(
                egui::Slider::new(&mut night.lamp_strength, 0.0..=3.0).text("lamp strength · open"),
            );
            ui.add(egui::Slider::new(&mut night.lamp_reach, 0.25..=4.0).text("lamp reach · open"));
            ui.add(
                egui::Slider::new(&mut night.indoor_lamp_strength, 0.0..=3.0)
                    .text("lamp strength · indoors"),
            );
            ui.add(
                egui::Slider::new(&mut night.indoor_lamp_reach, 0.25..=4.0)
                    .text("lamp reach · indoors"),
            );
            ui.add(
                egui::Slider::new(&mut night.lamp_fade, 0.05..=2.0)
                    .text("fade in/out (s)")
                    .suffix("s"),
            );
            // **Zero is the A/B, not just the off switch.** A still lamp is
            // written once and then not again; a wavering one is written every
            // frame, re-extracted and re-clustered. This slider is how that
            // cost is priced without a rebuild.
            ui.add(
                egui::Slider::new(&mut night.lamp_flicker, 0.0..=3.0).text("flame flicker"),
            );
        });
    });
    if night.enabled && night.lamps {
        let lit = lamps.lit;
        let burning = lamps.candidates;
        ui.colored_label(
            if lit >= crate::render::lamps::BUDGET { WARN } else { GOOD },
            format!(
                "{lit} lit of {burning} in view, budget {}.",
                crate::render::lamps::BUDGET
            ),
        );
    }
}
