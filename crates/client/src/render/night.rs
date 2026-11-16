//! An optional darker night, which the 1.12.1 client does not have.
//!
//! The other modules in this directory implement something the game defines:
//! a file format, a `.bls` program, a behaviour of the 1.12.1 client. This one
//! does not. The 1.12.1 client has no volumetric effects, and its night is
//! `Light.dbc`'s bands and nothing else. This module deviates from the 1.12.1
//! client on purpose. It is an option, not a correction.
//!
//! ## The light table's night, measured
//!
//! `vale light Azeroth`, map 0 at 00:00, the values every constant below was
//! chosen against:
//!
//! ```text
//! sun 97/130/162   fill 29/60/84   zenith 0/0/0   horizon 0/14/33   125..500y
//! ```
//!
//! The sky is already dark: the zenith is black and the horizon band is
//! 0/14/33. The light on the geometry is not dark. A surface facing the moon is
//! lit by `97/130/162 + 29/60/84` = 126/190/246, which is brighter than most
//! daylight textures survive, and a surface facing away is still lit by the
//! fill's 29/60/84. The world at midnight therefore looks like a flat
//! blue-grey afternoon under a black sky. That is how the 1.12.1 client draws
//! it, and this module changes it.
//!
//! The module's design depends on two consequences of that measurement:
//!
//! * The fill must be reduced, not the moon. Dimming both equally gives a dark,
//!   flat picture. Dimming the fill nearly four times as much as the moon gives
//!   a dark picture with shape in it, which is what distinguishes night from a
//!   lowered brightness setting. The ratio between the two floors is the
//!   contrast in the frame: 8:1 on the blue channel at map 0 midnight, against
//!   the table's 2:1.
//! * The sky needs almost no change. Darkening the six stops matters for the
//!   maps whose night sky is not already black, and on map 0 at midnight it
//!   changes nothing.
//!
//! ## The night factor is read from the light, not from the clock
//!
//! [`factor`] is a function of the atmosphere the light chain resolved, not of
//! [`crate::render::sky::WorldClock`], for three reasons, most important
//! first:
//!
//! * Positional lights and instances have no hour. A cave lit by its own
//!   `LightParams` row is dark at noon, and a clock-driven grade would leave it
//!   bright while the field outside got the night. Reading the light keeps the
//!   dark places dark.
//! * Every map states its own dusk. Alterac Valley's bands turn over at
//!   different hours from Durotar's, and a fixed 18:00..06:00 would apply one
//!   map's dusk to forty-three.
//! * Noon is unchanged to the bit. At map 0 noon the factor is exactly 0 and
//!   [`deepen`] is the identity, so every measurement this project has taken
//!   still holds with the option on. It defaults to off at the user's
//!   request; see [`NightTuning`].
//!
//! ## The mist is analytic and needs no extra pass
//!
//! There is no raymarch, froxel grid, extra pass or extra target. The mist is
//! the closed-form integral of an exponentially decaying density along the
//! view ray, evaluated in the fragment shaders that already run; see
//! `shaders/atmosphere.wgsl`'s `night_air`, which is two `exp`s and about
//! twenty other instructions behind a uniform branch that is false for the
//! whole frame in daylight.
//!
//! Its five parameters are carried in the fog uniform's scattering fields,
//! which are already bound for every world material and which nothing else in
//! this client uses: `fogged` passes `linear_fog` a scattering vector of zero,
//! so `directional_light_color` multiplies out, and
//! `directional_light_exponent` is never read under `FogFalloff::Linear`. See
//! [`NightAir::pack`], the one place that reinterpretation is written, and the
//! test beside it. The alternative was a bind group of this crate's own for
//! five floats; `sky`'s module note makes the same argument about the sun and
//! the fill.
//!
//! ## Why it is not a `WorldTuning` switch or a CVar
//!
//! It is not in [`crate::render::tuning::WorldTuning`], on purpose: every
//! switch there removes something, as a measurement instrument, and this adds
//! something. It is not a CVar either, because a new setting becomes a CVar
//! only where the 1.12.1 client would have somewhere to put it, and it would
//! not. It is a stated stand-in, with a flag (`--night`) and a control on the
//! debug window's world tab.

use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;

use vale_assets::tables::light::{Atmosphere, SKY_STOPS};

/// Whether the deep night is applied at all, and how far.
///
/// Off by default, at the user's request, so the night is the light table's
/// own unless `--night` or the debug window's world tab turns it on. Turned
/// on, [`factor`] is still zero in daylight, so the client at noon is
/// unchanged either way. [`NightTuning::on`] is the setting with it on.
#[derive(Resource, Clone, Copy, PartialEq)]
pub struct NightTuning {
    pub enabled: bool,
    /// How much of the grade to apply, 0..1. A dimmer rather than a boolean
    /// because the right darkness has intermediate answers, and finding one by
    /// rebuilding is the cost [`crate::render::tuning`] exists to remove.
    pub strength: f32,
    /// Whether burning sources light the world around them; see
    /// [`crate::render::lamps`], the other half of this deviation.
    ///
    /// A switch separate from [`Self::enabled`] because the two have separate
    /// costs: the grade is arithmetic in shaders that already run, and the
    /// lamps are twenty-four point lights that Bevy must cluster. Turning this
    /// off with the grade on measures the lamps' cost.
    pub(crate) lamps: bool,
    /// How bright a lamp in the open is, as a multiple of
    /// [`crate::render::lamps`]' own value. It is on the panel so that a torch
    /// that is too bright or too dim can be corrected without a rebuild.
    pub(crate) lamp_strength: f32,
    /// How far a lamp in the open reaches, as a multiple of
    /// [`crate::render::lamps`]' own value.
    pub(crate) lamp_reach: f32,
    /// How bright a lamp inside a building is, as a multiple of
    /// [`crate::render::lamps`]' own value. Indoor lamps need different values
    /// from lamps in the open.
    ///
    /// The split is per lamp, not per camera. A room is small, walled and
    /// already carries its own `MOLT` ambient, so a torch that gives a useful
    /// pool of light on a road gives a flat wash indoors. With enough of them
    /// in one room the result looks like no lighting model at all, which
    /// matches the report: "indoors, the lighting looks completely fake because
    /// of all the emitter lights usually present."
    ///
    /// Classifying by the camera would be cheaper and wrong. A doorway puts
    /// lamps on both sides of the threshold in view at once, so every lamp on
    /// screen would change together as the player stepped through: a
    /// screen-wide flicker at a boundary players cross constantly. Classifying
    /// by each lamp's position costs an AABB test per candidate and is stable
    /// across the doorway, because each lamp keeps the same answer.
    ///
    /// See [`crate::render::lamps::light_the_flames`], which does the test, and
    /// [`crate::render::wmos::Interior::holds`], the same test
    /// `world::entities::light_entities` makes for every unit.
    pub(crate) indoor_lamp_strength: f32,
    /// How far a lamp inside a building reaches, as a multiple of
    /// [`crate::render::lamps`]' own value.
    pub(crate) indoor_lamp_reach: f32,
    /// How long a lamp takes to fade in and to fade out, in seconds.
    /// [`crate::render::lamps::Slot::level`] is the curve the fade follows.
    ///
    /// It is on the panel for the same reason as the four lamp values above:
    /// whether a fade looks abrupt is judged by looking, not by measuring, and
    /// rebuilding to try each value is the cost [`crate::render::tuning`]
    /// exists to remove.
    pub(crate) lamp_fade: f32,
    /// How much a flame wavers, as a multiplier on each lamp's own amplitude.
    /// [`crate::render::lamps::LampLight::flicker`] holds the per-source split.
    ///
    /// A global scale rather than a per-lamp amplitude, because the question
    /// it answers is whether the effect is worth its cost. At 0 every lamp is
    /// steady and the pass again writes nothing for a settled slot, which gives
    /// the A/B that measures the cost. See [`crate::render::lamps::flame`].
    pub(crate) lamp_flicker: f32,
}

impl Default for NightTuning {
    /// Off, with the strength and lamp settings [`NightTuning::on`] uses, so
    /// that ticking the switch on the debug window gives the full grade.
    fn default() -> Self {
        NightTuning {
            enabled: false,
            ..NightTuning::on()
        }
    }
}

impl NightTuning {
    /// The deep night at full strength, with its lamps.
    pub fn on() -> Self {
        NightTuning {
            enabled: true,
            strength: 1.0,
            lamps: true,
            // Reach outdoors and strength indoors are the two values the
            // report asked to change, and they act on its two complaints. A
            // torch on a road was dim and not very useful because its pool was
            // too small, which is reach. A room looked flat because many
            // overlapping emitters each lit it at full strength, which is
            // strength.
            lamp_strength: 1.0,
            lamp_reach: 2.0,
            indoor_lamp_strength: 0.5,
            indoor_lamp_reach: 1.0,
            lamp_fade: 0.6,
            lamp_flicker: 1.0,
        }
    }

    /// `--night <off|on|0..1>`: what the flag parses to.
    ///
    /// `off`, `no`, `none` and `0` disable it; `on`, `yes` and `full` are
    /// [`Self::on`]; anything else is a strength. A value that will not parse
    /// warns and leaves the default, which is off, as `WorldTuning::without`
    /// does with a name it does not know.
    ///
    /// `on` exists because the flag always takes a value. A bare `--night`
    /// would consume the character name, since every flag in [`crate::Args`]
    /// reads the argument after it; see the parse loop.
    pub fn parse(value: Option<&str>) -> NightTuning {
        let Some(value) = value else {
            return NightTuning::default();
        };
        let text = value.trim().to_ascii_lowercase();
        if matches!(text.as_str(), "off" | "no" | "none" | "0") {
            return NightTuning {
                enabled: false,
                strength: 0.0,
                lamps: false,
                ..NightTuning::default()
            };
        }
        if matches!(text.as_str(), "on" | "yes" | "full") {
            return NightTuning::on();
        }
        match text.parse::<f32>() {
            Ok(strength) if (0.0..=1.0).contains(&strength) => NightTuning {
                enabled: strength > 0.0,
                strength,
                ..NightTuning::on()
            },
            _ => {
                warn!("--night {value}: expected `off` or a strength in 0..1");
                NightTuning::default()
            }
        }
    }

    /// The factor to grade by for an atmosphere, or zero when the option is off.
    pub(crate) fn applied_to(&self, atmosphere: &Atmosphere) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        factor(atmosphere) * self.strength.clamp(0.0, 1.0)
    }
}

/// How deep the night is, from 0 in daylight to 1 at its deepest, and the
/// height of the mist deck.
///
/// Written by [`crate::render::sky::resolve`], which is the one place the
/// atmosphere is decided; read by the fog uniform, by the present pass's grade
/// and by the HUD. A resource rather than three recomputations, because
/// [`factor`] would otherwise be answered differently by whoever asked last.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct Night {
    /// [`NightTuning::applied_to`] for the atmosphere in force.
    pub(crate) factor: f32,
    /// The height the mist's density is quoted at, in the world's own Z,
    /// which is Bevy's Y unchanged; see [`crate::render::axes`]. See
    /// [`NightAir::deck`].
    pub deck: f32,
}

/// The luminance of a band, in the file's own space.
///
/// Rec. 709 weights, which is what [`factor`] wants rather than a mean: the
/// table's daylight sun is a saturated orange whose green channel is what
/// falls over the day, and an unweighted mean of `255/136/0` reads
/// nearly as bright as `255/247/222`.
fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Where the ramp ends, in [`luma`] of `diffuse + ambient`. See [`NIGHT_LUMA`].
const DAY_LUMA: f32 = 0.90;

/// Where the ramp starts, in [`luma`] of `diffuse + ambient`.
///
/// Both ends are measured from `vale light Azeroth`, not round numbers. The
/// sum is 1.089 at noon, 0.973 at 06:00, 0.896 at 21:00 and 0.708 at 00:00 and
/// 03:00: a range under four tenths, so the window is narrow and could not be
/// chosen by eye.
///
/// [`DAY_LUMA`] sits just above dusk so that 21:00, which the table still
/// lights with an orange sun, grades at almost zero. This value sits just
/// under the midnight value so the bottom of the night gets the full grade
/// rather than a value close to it.
const NIGHT_LUMA: f32 = 0.72;

/// How dark the resolved light already is, from 0 to 1.
///
/// A `smoothstep` rather than a lerp because the ends matter more than the
/// middle: a linear ramp puts a visible seam at the hour the grade reaches
/// zero, and the table's own band interpolation is already linear underneath
/// this.
pub(crate) fn factor(atmosphere: &Atmosphere) -> f32 {
    let lit = luma([
        atmosphere.diffuse[0] + atmosphere.ambient[0],
        atmosphere.diffuse[1] + atmosphere.ambient[1],
        atmosphere.diffuse[2] + atmosphere.ambient[2],
    ]);
    let t = ((DAY_LUMA - lit) / (DAY_LUMA - NIGHT_LUMA)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What a graded colour is tinted toward: a cold moonlit blue.
///
/// This is the only constant here with a reason outside the renderer. Human
/// scotopic vision is rod-driven, blue-shifted (the Purkinje effect) and
/// nearly colourless. Night scenes on film rely on it, and it makes a dark
/// picture read as night rather than as under-exposure. [`COLD_SATURATION`]
/// is the colourless part and this is the blue-shifted part.
///
/// Both were reduced after the first test. Applied fully, the scotopic
/// argument produces a grey picture, which does not look like night in a game
/// with a saturated look: every lit surface looked washed out, and the warm
/// bands that remained looked grey-orange. The shift is kept and its strength
/// is a matter of taste. The hue of what the light falls on must survive,
/// because that hue is the game's.
const COLD: [f32; 3] = [0.70, 0.84, 1.00];

/// How much of a band's own hue survives the shift. 1 keeps all of it; 0 is
/// monochrome. See [`COLD`] for why this is close to the top of its range.
const COLD_SATURATION: f32 = 0.85;

/// `c` desaturated toward its own luminance and tinted by [`COLD`].
///
/// In the file's own space, like every other arithmetic on a band — see
/// `atmosphere.wgsl`'s module note on why a sum does not survive a transfer
/// function. This is a mix and a product, and both are taken where the table
/// states its numbers.
fn cold(c: [f32; 3]) -> [f32; 3] {
    let grey = luma(c);
    let mut out = [0.0; 3];
    for k in 0..3 {
        out[k] = (grey + (c[k] - grey) * COLD_SATURATION) * COLD[k];
    }
    out
}

/// The fraction of the fill (ambient) that remains at the deepest night.
///
/// The lowest floor, shared with [`SHEEN_FLOOR`], and the one with the largest
/// effect. At map 0 midnight it takes the fill from 29/60/84 to 5/11/18, so a
/// surface facing away from the moon goes from lit blue to nearly black. That
/// restores contrast to a night the table draws flat.
const FILL_FLOOR: f32 = 0.22;

/// The fraction of the moon (diffuse) that remains at the deepest night:
/// almost four times [`FILL_FLOOR`], on purpose. See the module note: dimming
/// both equally gives a flat dark picture, and the ratio between the two
/// floors is the contrast in the frame, 8:1 on the blue channel at map 0
/// midnight against the table's 2:1.
const MOON_FLOOR: f32 = 0.85;

/// The fraction of each sky dome stop that remains, zenith first.
///
/// The values rise toward the horizon so the remaining light sits where the
/// sun set, as in a real night sky. On map 0 at midnight the zenith band is
/// already `0/0/0` and this changes nothing; it is for the maps whose night
/// sky is not black.
const SKY_FLOOR: [f32; SKY_STOPS] = [0.25, 0.30, 0.36, 0.42, 0.50, 0.55];

/// The fraction of the sheen's halo colour that remains: `Light.dbc`'s band
/// 9, which `atmosphere.wgsl`'s `sun_sheen` adds over shiny ground.
///
/// The lowest floor of the four, because the band is a warm near-white
/// (`255/247/222` on map 0), and a cream highlight along a road at midnight is
/// the most visibly wrong thing the ungraded night draws.
const SHEEN_FLOOR: f32 = 0.22;

/// How far the fog's far distance is pulled in, at the deepest night.
///
/// 500 yards becomes 275 on map 0. This is not a performance measure; the
/// terrain streamer keeps its 3x3 either way. Air that cannot be seen through
/// is most of what makes a dark place feel enclosed.
const FOG_END_PULL: f32 = 0.55;

/// How far the fog's near distance is pulled in, at the deepest night. It is
/// pulled harder than [`FOG_END_PULL`], so the fade starts close and runs
/// over a long distance rather than beginning at the edge of vision.
const FOG_START_PULL: f32 = 0.30;

/// Grade an atmosphere into the night, in place.
///
/// `n` is [`NightTuning::applied_to`]'s result. At `n == 0` this is exactly the
/// identity, so turning the option on moves no daylight measurement.
pub(crate) fn deepen(atmosphere: &mut Atmosphere, n: f32) {
    if n <= 0.0 {
        return;
    }
    let n = n.min(1.0);
    // `mix(band, cold(band) * floor, n)`, as a closure because it is applied to
    // nine bands.
    let toward = |c: [f32; 3], floor: f32| {
        let target = cold(c);
        let mut out = [0.0; 3];
        for k in 0..3 {
            out[k] = c[k] + (target[k] * floor - c[k]) * n;
        }
        out
    };
    atmosphere.ambient = toward(atmosphere.ambient, FILL_FLOOR);
    atmosphere.diffuse = toward(atmosphere.diffuse, MOON_FLOOR);
    atmosphere.sun_halo = toward(atmosphere.sun_halo, SHEEN_FLOOR);
    for (stop, floor) in atmosphere.sky.iter_mut().zip(SKY_FLOOR) {
        *stop = toward(*stop, floor);
    }
    // The distances last. `fog_start <= fog_end` has to survive, and it does
    // because the start is pulled harder than the end from a value that was
    // already the smaller of the two.
    atmosphere.fog_end *= 1.0 + (FOG_END_PULL - 1.0) * n;
    atmosphere.fog_start *= 1.0 + (FOG_START_PULL - 1.0) * n;
    debug_assert!(atmosphere.fog_start <= atmosphere.fog_end);
}

/// The mist, as the five values the shader reads.
///
/// See `shaders/atmosphere.wgsl`'s `night_air` for what each one does to a
/// fragment. They are here rather than there because they are a function of the
/// night factor, which is a CPU quantity, and because the packing below is the
/// half that can be tested.
#[derive(Clone, Copy, PartialEq, Debug, ShaderType)]
pub(crate) struct NightAir {
    /// The night factor itself, and the shader's own early-out: at zero
    /// `night_air` returns its argument before touching anything else, which is
    /// one uniform-coherent compare per fragment in daylight.
    pub night: f32,
    /// Density at [`Self::deck`], per yard. At full night this gives an optical
    /// depth of about 1.1 over the graded 275-yard fog distance, so roughly a
    /// third of a distant fragment survives.
    pub density: f32,
    /// The height over which that density falls by `1/e`, in yards. Small
    /// enough that a hilltop stands above the mist and a valley floor is
    /// buried in it; that height dependence is the reason for using the
    /// integral.
    pub falloff: f32,
    /// How hard the mist glows toward the moon.
    pub glow: f32,
    /// The height of the deck, in world Z.
    ///
    /// The player's own altitude, re-read whenever the light resolves, which
    /// is every four yards of travel; see `sky::RESOLVE_CELL`. A fixed height
    /// does not work: this game's ground runs from below −500 in Thousand
    /// Needles to above +700 inside Ironforge, so any single height either
    /// covers half the world or is invisible in the other half. Real air does
    /// not follow the ground, but a deck that does is the only version that is
    /// visible everywhere.
    pub deck: f32,
}

/// Density at the deck at full night, per yard. See [`NightAir::density`].
///
/// Halved from 0.004 after the first test. The mist mixes every fragment
/// toward the horizon band, so density directly sets how washed out the
/// picture is. At 0.004 a surface thirty yards away had lost a tenth of itself
/// to the air and one at a hundred yards a third, which across a whole frame
/// looks like haze over everything rather than mist in the low ground.
const DENSITY: f32 = 0.0022;

/// The `1/e` height of that density, in yards. See [`NightAir::falloff`].
const FALLOFF: f32 = 30.0;

/// How much of the moon's own colour the mist adds back, looking at it.
///
/// Reduced from 0.6, and its exponent tightened with it (see
/// `atmosphere.wgsl`'s `NIGHT_GLOW_POWER`). Together they set how much of the
/// sky is inside the halo. At 0.6 with an exponent of 8 the halo covered most
/// of the sky, which at any hour the table still lights warm gave a broad
/// orange wash rather than a moon.
const GLOW: f32 = 0.45;

impl NightAir {
    /// The air for a night factor and a deck height.
    pub fn new(night: f32, deck: f32) -> NightAir {
        NightAir {
            night,
            density: DENSITY * night,
            falloff: FALLOFF,
            glow: GLOW * night,
            deck,
        }
    }

    /// The five values, packed into the two fog fields that carry them.
    ///
    /// `DistanceFog::directional_light_color` and `::directional_light_exponent`
    /// are Bevy's light-dispersion controls, and this client does not use
    /// them: `atmosphere.wgsl`'s `fogged` passes `linear_fog` a scattering
    /// vector of `vec3(0.0)`, so `scattering_adjusted_fog_color` multiplies the
    /// colour by zero whatever it holds, and `linear_fog` never reads the
    /// exponent. Under `FogFalloff::Linear` they are five floats already bound
    /// at group 0 for every world material, and no bind group of this crate's
    /// own could be cheaper.
    ///
    /// The round trip is exact: `bevy_pbr`'s fog extraction is
    /// `LinearRgba::from(color).to_vec4()` with no clamp, so a
    /// `Color::LinearRgba` built here arrives in the shader unchanged.
    ///
    /// If a later Bevy version reads either field for another purpose, this is
    /// the place to change. The symptom would be a glow around the sun in
    /// daylight, not an error.
    pub(crate) fn pack(&self) -> (Color, f32) {
        (
            Color::LinearRgba(LinearRgba::new(
                self.night,
                self.density,
                self.falloff,
                self.glow,
            )),
            self.deck,
        )
    }
}

pub(crate) struct NightPlugin;

impl Plugin for NightPlugin {
    /// The shader half of this module is not registered here, on purpose.
    /// `shaders/night.wgsl` is loaded by [`crate::render::sky`] beside the two
    /// libraries in its dependency chain; see the note there. A library that
    /// `atmosphere.wgsl` imports must not depend on a plugin that can be
    /// removed.
    fn build(&self, app: &mut App) {
        app.init_resource::<NightTuning>()
            .init_resource::<Night>()
            // After the light is resolved, which writes `Night`; see
            // `sky::resolve`. Without this ordering the present pass is graded
            // for the previous frame's atmosphere, which shows as a visible
            // pulse at a dusk transition.
            .add_systems(Update, grade_present.after(crate::render::sky::apply));
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report.run_if(crate::ui::report::watched));
    }
}

/// Hand the full-screen quad the night factor.
///
/// The present pass is the only place in this client that sees a finished
/// picture, which is what the vignette and the shadow crush need: both are
/// display transforms over the frame rather than terms of the light, and doing
/// either per-surface would apply them once per overlapping draw.
fn grade_present(
    night: Res<Night>,
    quad: Query<&bevy::sprite_render::MeshMaterial2d<crate::render::present::PresentMaterial>>,
    mut materials: ResMut<Assets<crate::render::present::PresentMaterial>>,
) {
    if !night.is_changed() {
        return;
    }
    for handle in &quad {
        // `get_mut` marks the asset changed on `DerefMut`, which is what
        // rebuilds the bind group — so this is guarded above rather than run
        // every frame. See `present::repoint` for what that machinery costs.
        if let Some(mut material) = materials.get_mut(&handle.0) {
            if material.grade.night != night.factor {
                material.grade.night = night.factor;
            }
        }
    }
}

/// [`crate::ui::report`] slot, beside `sky`'s at 10.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(11);

/// Shows how deep the night is on the HUD. The depth cannot be judged by
/// looking, because the graded picture is meant to be dark.
#[cfg(feature = "diagnostics")]
fn report(
    night: Res<Night>,
    tuning: Res<NightTuning>,
    mut hud: ResMut<crate::ui::report::HudReport>,
) {
    if !night.is_changed() && !tuning.is_changed() {
        return;
    }
    hud.set(
        crate::ui::report::Section::World,
        SLOT,
        "night",
        if !tuning.enabled {
            "off".to_string()
        } else {
            format!(
                "{:.0}% at x{:.2}  deck {:.0}y",
                night.factor * 100.0,
                tuning.strength,
                night.deck,
            )
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Map 0's own bands at an hour, off `vale light Azeroth`.
    fn at(sun: [i32; 3], fill: [i32; 3]) -> Atmosphere {
        let mut atmosphere = Atmosphere::PLACEHOLDER;
        atmosphere.diffuse = sun.map(|c| c as f32 / 255.0);
        atmosphere.ambient = fill.map(|c| c as f32 / 255.0);
        atmosphere
    }

    fn midnight() -> Atmosphere {
        at([97, 130, 162], [29, 60, 84])
    }

    /// Noon is exactly the identity, so every daylight measurement in this
    /// project still holds with the option on.
    #[test]
    fn daylight_is_untouched_to_the_bit() {
        let mut atmosphere = at([255, 136, 0], [104, 130, 154]);
        let before = atmosphere;
        let n = factor(&atmosphere);
        assert_eq!(n, 0.0, "map 0 at noon is not night");
        deepen(&mut atmosphere, n);
        assert_eq!(atmosphere.diffuse, before.diffuse);
        assert_eq!(atmosphere.ambient, before.ambient);
        assert_eq!(atmosphere.sky, before.sky);
        assert_eq!(atmosphere.fog_end, before.fog_end);
        assert_eq!(atmosphere.fog_start, before.fog_start);
    }

    /// Midnight gets the full grade.
    #[test]
    fn midnight_is_the_full_grade() {
        let mut atmosphere = midnight();
        assert_eq!(factor(&atmosphere), 1.0);
        deepen(&mut atmosphere, 1.0);
        let byte = |c: [f32; 3]| c.map(|v| (v * 255.0).round() as i32);
        // The module note's measurement: the fill nearly disappears, most of
        // the moon remains, and the difference between them gives the picture
        // its shape.
        assert_eq!(byte(atmosphere.ambient), [5, 11, 18]);
        assert_eq!(byte(atmosphere.diffuse), [60, 92, 133]);
        // A moonlit surface is still legible; one facing away is not.
        let lit = byte(atmosphere.ambient)[2] + byte(atmosphere.diffuse)[2];
        assert!(lit > 100, "the moonlit side must stay readable, got {lit}");
        assert!(byte(atmosphere.ambient).iter().all(|c| *c < 24));
    }

    /// The full-screen grade follows the factor back to zero.
    ///
    /// `sky::resolve`'s own test checks that leaving the world sets
    /// [`Night::factor`] back to zero. This test checks what the player sees,
    /// one system later, which is where the reported fault was: the present
    /// pass grades every pixel of `WorldFrame`, and the glue scene is drawn
    /// through the world camera into the same image, so a factor left non-zero
    /// leaves the login screen crushed, cooled and vignetted.
    ///
    /// It is a wiring test because the two are joined only by an `is_changed`
    /// guard and an `.after`, and neither fails visibly when it stops holding.
    #[test]
    fn the_full_screen_grade_follows_the_factor_down() {
        use crate::render::present::{NightGrade, PresentMaterial};
        use bevy::sprite_render::MeshMaterial2d;

        let mut app = App::new();
        app.init_resource::<Assets<PresentMaterial>>()
            .insert_resource(Night {
                factor: 1.0,
                deck: -412.5,
            })
            .add_systems(Update, grade_present);
        let handle = app
            .world_mut()
            .resource_mut::<Assets<PresentMaterial>>()
            .add(PresentMaterial {
                frame: Handle::default(),
                grade: NightGrade::default(),
            });
        app.world_mut().spawn(MeshMaterial2d(handle.clone()));

        app.update();
        let graded = |app: &App| {
            app.world()
                .resource::<Assets<PresentMaterial>>()
                .get(&handle)
                .expect("the quad's material")
                .grade
                .night
        };
        assert_eq!(graded(&app), 1.0, "midnight grades the whole frame");

        // Leaving the world sets the factor to zero.
        app.world_mut().resource_mut::<Night>().factor = 0.0;
        app.update();
        assert_eq!(
            graded(&app),
            0.0,
            "the character screen must be drawn ungraded"
        );
    }

    /// The screens before the world are never graded, with the option on.
    ///
    /// `Atmosphere::PLACEHOLDER` is what `sky::resolve` leaves standing when
    /// there is no session — before the first login, and again the moment one
    /// ends — so the login screen, character select and the Dark Portal behind
    /// them are lit by it. Its factor has to be exactly zero or all three read
    /// as a faded night, which is the fault that was reported. It is asserted
    /// here rather than reasoned about because the
    /// placeholder is the mean of nineteen daylight rows and a table edit could
    /// move it under the ramp without anything else noticing.
    #[test]
    fn the_glue_screens_are_never_night() {
        assert_eq!(factor(&Atmosphere::PLACEHOLDER), 0.0);
        let mut lit = Atmosphere::PLACEHOLDER;
        let n = NightTuning::on().applied_to(&lit);
        deepen(&mut lit, n);
        assert_eq!(lit.ambient, Atmosphere::PLACEHOLDER.ambient);
        assert_eq!(lit.diffuse, Atmosphere::PLACEHOLDER.diffuse);
        // …and with nothing burning, since the lamps are scaled by the same
        // number — see `render::lamps`.
        assert_eq!(NightAir::new(factor(&Atmosphere::PLACEHOLDER), 0.0).density, 0.0);
    }

    /// 21:00 is dusk and not night, which is the hour the ramp was placed
    /// against: the table still lights it with an orange sun, and a grade that
    /// engaged there would drop the evening into the dark an hour early.
    #[test]
    fn dusk_is_barely_graded() {
        assert!(factor(&at([255, 109, 0], [102, 92, 119])) < 0.05);
        // …and 06:00, on the other side of it, not at all.
        assert_eq!(factor(&at([255, 103, 0], [93, 125, 156])), 0.0);
    }

    /// The grade is monotone in `n` and never leaves the representable range —
    /// the two things a band handed to a shader has to satisfy, and neither is
    /// obvious from a mix whose target is itself a function of the input.
    #[test]
    fn the_grade_only_ever_darkens() {
        let graded = |n| {
            let mut a = midnight();
            deepen(&mut a, n);
            a
        };
        let mut previous = graded(0.0);
        for step in 1..=10 {
            let next = graded(step as f32 / 10.0);
            for k in 0..3 {
                assert!(next.ambient[k] <= previous.ambient[k] + 1e-6, "step {step}");
                assert!(next.diffuse[k] <= previous.diffuse[k] + 1e-6, "step {step}");
                assert!((0.0..=1.0).contains(&next.ambient[k]));
                assert!((0.0..=1.0).contains(&next.diffuse[k]));
            }
            assert!(next.fog_end <= previous.fog_end);
            assert!(next.fog_start <= next.fog_end);
            previous = next;
        }
    }

    /// The five values pass through the fog fields unchanged. This is the part
    /// of the transport that could break without an error; see
    /// [`NightAir::pack`].
    #[test]
    fn the_air_packs_into_the_fog_uniform_exactly() {
        let air = NightAir::new(0.75, -412.5);
        let (colour, exponent) = air.pack();
        let packed = LinearRgba::from(colour);
        assert_eq!(packed.red, air.night);
        assert_eq!(packed.green, air.density);
        assert_eq!(packed.blue, air.falloff);
        assert_eq!(packed.alpha, air.glow);
        assert_eq!(exponent, air.deck);
        // A deck below sea level is the ordinary case in half this game's
        // zones, so the field has to carry a negative unchanged.
        assert!(exponent < 0.0);
        // …and at no night there is nothing for the shader to do, which is what
        // its own early-out reads.
        assert_eq!(NightAir::new(0.0, 0.0).night, 0.0);
        assert_eq!(NightAir::new(0.0, 0.0).density, 0.0);
    }

    /// The default is off. `off` and a strength of zero are off too: a flag
    /// that disabled the grade but left the factor non-zero would leave the
    /// shader's branch live for nothing.
    #[test]
    fn the_flag_parses_the_three_forms() {
        let midnight = midnight();
        assert_eq!(NightTuning::default().applied_to(&midnight), 0.0);
        assert_eq!(NightTuning::parse(None).applied_to(&midnight), 0.0);
        assert_eq!(NightTuning::parse(Some("off")).applied_to(&midnight), 0.0);
        assert_eq!(NightTuning::parse(Some("0")).applied_to(&midnight), 0.0);
        assert_eq!(NightTuning::parse(Some("0.5")).applied_to(&midnight), 0.5);
        assert_eq!(NightTuning::parse(Some("on")).applied_to(&midnight), 1.0);
        // Out of range warns and leaves the default, rather than grading by 4.
        assert_eq!(NightTuning::parse(Some("4")).applied_to(&midnight), 0.0);
        assert_eq!(NightTuning::parse(Some("dark")).applied_to(&midnight), 0.0);
        // The switch is honoured whatever the light says.
        let off = NightTuning {
            enabled: false,
            ..NightTuning::on()
        };
        assert_eq!(off.applied_to(&midnight), 0.0);
    }
}
