//! **A night this game does not have.**
//!
//! Every other module in this directory is a reading of something — a file
//! format, a `.bls` program, a behaviour of the 1.12.1 client. This one is not, and
//! that is stated first so nobody goes looking for the authority behind it:
//! 1.12 has no volumetric anything, its night is `Light.dbc`'s own bands and
//! nothing else, and what is here **deviates from the reference on purpose**.
//! It is an option rather than a correction.
//!
//! ## What the table's own night actually is, measured
//!
//! `vale light Azeroth`, map 0 at 00:00 — the numbers every constant below
//! was chosen against:
//!
//! ```text
//! sun 97/130/162   fill 29/60/84   zenith 0/0/0   horizon 0/14/33   125..500y
//! ```
//!
//! The **sky** is already dark: the zenith is literally black and the horizon
//! band is 0/14/33. What is not dark is the **light on the geometry**. A
//! surface facing the moon is lit by `97/130/162 + 29/60/84` = 126/190/246,
//! which is brighter than most daylight textures survive, and a surface facing
//! away is still lit by the fill's 29/60/84. So the world at midnight reads as
//! a flat blue-grey afternoon standing under a black sky — which is what the
//! reference looks like, and what this module exists to leave behind.
//!
//! Two things follow from that measurement and both are load-bearing:
//!
//! * **The fill is what has to go**, not the moon. Dimming both equally makes a
//!   dark flat picture; dimming the fill nearly four times as hard as the moon
//!   makes a dark picture with *shape* in it, which is the whole difference
//!   between "night" and "the brightness slider is down". The ratio between the
//!   two floors *is* the contrast in the frame: 8:1 on the blue channel at map 0
//!   midnight, against the table's own 2:1.
//! * **The sky needs almost nothing done to it.** Crushing the six stops is
//!   applied for the maps whose night sky is not already black, and on map 0 at
//!   midnight it changes nothing at all.
//!
//! ## The night factor is read off the light, not off the clock
//!
//! [`factor`] is a function of the atmosphere the light chain resolved, not of
//! [`crate::render::sky::WorldClock`]. Three reasons, in the order they matter:
//!
//! * **Positional lights and instances have no hour.** A cave lit by its own
//!   `LightParams` row is dark at noon, and a clock-driven grade would leave it
//!   bright while the field outside got the night. Reading the light means the
//!   dark places are dark.
//! * **Every map states its own dusk.** Alterac Valley's bands turn over at
//!   different hours from Durotar's, and a hard-coded 18:00..06:00 would be one
//!   map's dusk applied to forty-three.
//! * **Noon is untouched to the bit.** At map 0 noon the factor is exactly 0
//!   and [`deepen`] is the identity, so every measurement this project has ever
//!   taken still holds with the option on. That is why it can default to on.
//!
//! ## And the volumetric half is analytic, which is why it is free
//!
//! There is no raymarch, no froxel grid, no extra pass and no extra target. The
//! mist is the **closed-form integral of an exponentially-decaying density along
//! the view ray**, evaluated in the fragment shaders that were already running —
//! see `shaders/atmosphere.wgsl`'s `night_air`, which is two `exp`s and about
//! twenty other instructions, behind a uniform branch that is false for the
//! whole frame in daylight.
//!
//! Its five parameters ride in the **fog uniform's scattering fields**, which
//! are bound for every world material already and which nothing else in this
//! client uses: `fogged` passes `linear_fog` a scattering vector of zero, so
//! `directional_light_color` multiplies out and `directional_light_exponent` is
//! never read at all under `FogFalloff::Linear`. See [`NightAir::pack`], which
//! is the one place that reinterpretation is written down, and the test beside
//! it. The alternative was a bind group of this crate's own for five floats —
//! the same argument `sky`'s module note makes about the sun and the fill.
//!
//! ## What it is not
//!
//! It is **not** in [`crate::render::tuning::WorldTuning`], and that is
//! deliberate rather than an oversight: every switch there is a *subtraction*
//! used as an instrument, and this adds. It is not a CVar either, because the
//! test for a new setting — *would the real client have
//! somewhere to put this?* — answers no. So it is a stated stand-in, with a
//! flag (`--night`) and a control on the debug window's world tab.

use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;

use vale_assets::tables::light::{Atmosphere, SKY_STOPS};

/// Whether the deep night is applied at all, and how far.
///
/// **On by default**, which is safe for exactly one reason: [`factor`] is zero
/// in daylight, so an untouched client at noon is the client this project has
/// been measuring all along. It is the hours the reference draws as a bright
/// blue afternoon that change.
#[derive(Resource, Clone, Copy, PartialEq)]
pub struct NightTuning {
    pub enabled: bool,
    /// How much of the grade to apply, 0..1. A dimmer rather than a boolean
    /// because "is this too dark" is a question with a middle, and answering it
    /// by rebuilding is the tax [`crate::render::tuning`] exists to retire.
    pub strength: f32,
    /// **Whether things that burn light the world around them** — see
    /// [`crate::render::lamps`], which is the other half of this branch.
    ///
    /// Its own switch rather than part of [`Self::enabled`] because the two
    /// are separable questions with separable costs: the grade is arithmetic
    /// in shaders that were already running, and the lamps are twenty-four
    /// point lights that Bevy has to cluster. Turning this off and the grade
    /// on is how the second is priced.
    pub lamps: bool,
    /// How bright a lamp **standing in the open** is, as a multiple of
    /// [`crate::render::lamps`]' own value. On the panel because a torch that
    /// is too bright and a torch that is too dim are both one slider away from
    /// right, and neither is worth a rebuild.
    pub lamp_strength: f32,
    /// …and how far it reaches, on the same terms.
    pub lamp_reach: f32,
    /// …and the same pair for a lamp **inside a building**, which wants
    /// different numbers rather than the same ones.
    ///
    /// The split is per *lamp* and not per *camera*, and that is the whole
    /// design decision here. A room is small, walled and already carries its
    /// own `MOLT` ambient, so the same torch that reads as a useful pool of
    /// light on a road reads as a flat wash indoors — enough of them in one
    /// room and the result is indistinguishable from no lighting model at all,
    /// which is what was reported: *"indoors, the lighting looks completely
    /// fake because of all the emitter lights usually present."*
    ///
    /// Classifying by the **camera** would be cheaper and wrong: a doorway puts
    /// lamps on both sides of the threshold in view at once, so every lamp on
    /// screen would change together as the player stepped through — a global
    /// flicker on a boundary the player crosses constantly. Classifying by the
    /// lamp's own position costs an AABB test per candidate and is stable
    /// across the doorway, because each lamp keeps answering the same way.
    ///
    /// See [`crate::render::lamps::light_the_flames`], which does the test, and
    /// [`crate::render::wmos::Interior::holds`], which is the same one
    /// `world::entities::light_entities` already makes for every unit.
    pub indoor_lamp_strength: f32,
    /// …and how far an indoor lamp reaches.
    pub indoor_lamp_reach: f32,
    /// **How long a lamp takes to come up and to go out, in seconds** — see
    /// [`crate::render::lamps::Slot::level`], which is the curve it is spent
    /// along.
    ///
    /// On the panel for the reason the other four are: "does this look
    /// abrupt" is a question with a middle, it is answered by looking rather
    /// than by measuring, and rebuilding to try a number is the tax
    /// [`crate::render::tuning`] exists to retire.
    pub lamp_fade: f32,
    /// **How much a flame wavers**, as a multiplier on each lamp's own
    /// amplitude — see [`crate::render::lamps::LampLight::flicker`], which is
    /// where the per-source split lives.
    ///
    /// A global scale rather than the amplitude itself, because the interesting
    /// question is not "how much" for one lamp but whether the effect earns its
    /// cost at all: at 0 every lamp is as still as it was and the pass goes
    /// back to writing nothing for a settled slot, which is the A/B that prices
    /// it. See [`crate::render::lamps::flame`].
    pub lamp_flicker: f32,
}

impl Default for NightTuning {
    fn default() -> Self {
        NightTuning {
            enabled: true,
            strength: 1.0,
            lamps: true,
            // **Reach outdoors, strength indoors** — the two the report asked
            // for, and they are different levers for the two halves of it. A
            // torch on a road was "dim and not very useful": its *pool* was too
            // small rather than its centre too dark, which is reach. A room was
            // "completely fake": too many overlapping emitters each at full
            // strength, which is strength.
            lamp_strength: 1.0,
            lamp_reach: 2.0,
            indoor_lamp_strength: 0.5,
            indoor_lamp_reach: 1.0,
            lamp_fade: 0.6,
            lamp_flicker: 1.0,
        }
    }
}

impl NightTuning {
    /// `--night <off|0..1>`: what the flag parses to.
    ///
    /// `off`, `no`, `none` and `0` disable it; `on`, `yes` and `full` are the
    /// default; anything else is a fraction. A value that will not parse warns
    /// and leaves the default standing rather than silently measuring an
    /// ungraded world — the rule `WorldTuning::without` follows, for the same
    /// reason.
    ///
    /// **`on` exists because the flag always takes a value.** A bare `--night`
    /// would consume the character name, since every flag in
    /// [`crate::Args`] reads the argument after it; see the parse loop.
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
            return NightTuning::default();
        }
        match text.parse::<f32>() {
            Ok(strength) if (0.0..=1.0).contains(&strength) => NightTuning {
                enabled: strength > 0.0,
                strength,
                ..NightTuning::default()
            },
            _ => {
                warn!("--night {value}: expected `off` or a strength in 0..1");
                NightTuning::default()
            }
        }
    }

    /// The factor to grade by for an atmosphere, or zero when the option is off.
    pub fn applied_to(&self, atmosphere: &Atmosphere) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        factor(atmosphere) * self.strength.clamp(0.0, 1.0)
    }
}

/// **How deep the night is**, 0 in daylight and 1 at the bottom of it — and
/// where the mist deck sits.
///
/// Written by [`crate::render::sky::resolve`], which is the one place the
/// atmosphere is decided; read by the fog uniform, by the present pass's grade
/// and by the HUD. A resource rather than three recomputations, because
/// [`factor`] would otherwise be answered differently by whoever asked last.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct Night {
    /// [`NightTuning::applied_to`] for the atmosphere in force.
    pub factor: f32,
    /// The height the mist's density is quoted at, in the **world's** own Z —
    /// which is Bevy's Y unchanged, see [`crate::render::axes`]. See
    /// [`NightAir::deck`].
    pub deck: f32,
}

/// The luminance of a band, in the file's own space.
///
/// Rec. 709 weights, which is what [`factor`] wants rather than a mean: the
/// table's daylight sun is a saturated orange whose *green* channel is what
/// actually falls over the day, and an unweighted mean of `255/136/0` reads
/// nearly as bright as `255/247/222`.
fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Where the ramp ends, in [`luma`] of `diffuse + ambient`. See [`NIGHT_LUMA`].
const DAY_LUMA: f32 = 0.90;

/// Where it starts.
///
/// Both are measurements off `vale light Azeroth` rather than round numbers.
/// The sum runs **1.089 at noon**, 0.973 at 06:00, 0.896 at 21:00 and **0.708
/// at 00:00 and 03:00** — a range of under four tenths, which is why the window
/// is narrow and why picking it by eye would have been hopeless.
///
/// [`DAY_LUMA`] sits just above dusk so that 21:00 — which the table still
/// lights with an orange sun — grades at essentially zero, and this sits just
/// under the midnight value so the bottom of the night is the full grade rather
/// than something approaching it.
const NIGHT_LUMA: f32 = 0.72;

/// **How dark the light chain already thinks it is**, 0..1.
///
/// A `smoothstep` rather than a lerp because the ends matter more than the
/// middle: a linear ramp puts a visible seam at the hour the grade reaches
/// zero, and the table's own band interpolation is already linear underneath
/// this.
pub fn factor(atmosphere: &Atmosphere) -> f32 {
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
/// **This is the one constant here with a reason outside the renderer.** Human
/// scotopic vision is rod-driven, blue-shifted (the Purkinje effect) and very
/// nearly colourless; every night scene ever shot on film leans on it, and it
/// is what makes a dark picture read as *night* rather than as an
/// under-exposure. [`COLD_SATURATION`] is the colourless half and this is the
/// blue-shifted half.
///
/// **Both were pulled back after the first look at it**, and the report is
/// worth keeping because it is a general trap: taken to the letter, the
/// scotopic argument produces a grey picture, and a grey picture is not what
/// anybody means by "night" in a game whose whole look is saturated. Every lit
/// surface came out washed, and the warm bands that survived read grey-orange
/// rather than as themselves. The rule this settled on is that the *shift* is
/// real and its **strength** is a matter of taste: the hue of what the light
/// falls on has to survive, because that hue is the game's.
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

/// What survives of the **fill** at the bottom of the night.
///
/// The smallest number here and the one that does the most work. At map 0
/// midnight it takes the fill from 29/60/84 to 5/11/18 — a surface facing away
/// from the moon goes from "lit blue" to very nearly black, which is what puts
/// contrast back into a night the table draws flat.
const FILL_FLOOR: f32 = 0.22;

/// …and what survives of the **moon**, which is deliberately almost four times
/// it. See the module note: dimming both equally is a flat dark picture, and
/// the *ratio* between these two is the contrast in the frame — 8:1 on the blue
/// channel at map 0 midnight, against the table's own 2:1.
const MOON_FLOOR: f32 = 0.85;

/// What survives of the sky dome, zenith first.
///
/// Rising toward the horizon so the last of the light sits where the sun went
/// down, which is where every night sky keeps it. On map 0 at midnight the
/// zenith band is already `0/0/0` and this changes nothing; it is for the maps
/// whose night sky is not.
const SKY_FLOOR: [f32; SKY_STOPS] = [0.25, 0.30, 0.36, 0.42, 0.50, 0.55];

/// What survives of the **sheen's** halo colour — `Light.dbc`'s band 9, which
/// `atmosphere.wgsl`'s `sun_sheen` adds on top of shiny ground.
///
/// The lowest floor of the four, because the band is a warm near-white
/// (`255/247/222` on map 0) and a cream highlight down a road at midnight is
/// the single most obviously wrong thing the ungraded night draws.
const SHEEN_FLOOR: f32 = 0.22;

/// How far the fog's far distance is pulled in, at the bottom of the night.
///
/// 500 yards becomes 275 on map 0. Nothing about this is a performance
/// measure — the terrain streamer keeps its 3x3 either way — it is that air you
/// cannot see through is most of what makes a dark place feel enclosed.
const FOG_END_PULL: f32 = 0.55;

/// …and its near distance, pulled harder, so the fade starts close and has a
/// long way to run rather than switching on at the edge of vision.
const FOG_START_PULL: f32 = 0.30;

/// **Grade an atmosphere into the night, in place.**
///
/// `n` is [`NightTuning::applied_to`]'s result. At `n == 0` this is the identity
/// — not approximately, exactly — which is the property that lets the option
/// default to on without moving a single daylight measurement.
pub fn deepen(atmosphere: &mut Atmosphere, n: f32) {
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

/// **The mist, as the five numbers the shader wants.**
///
/// See `shaders/atmosphere.wgsl`'s `night_air` for what each one does to a
/// fragment. They are here rather than there because they are a function of the
/// night factor, which is a CPU quantity, and because the packing below is the
/// half that can be tested.
#[derive(Clone, Copy, PartialEq, Debug, ShaderType)]
pub struct NightAir {
    /// The night factor itself, and the shader's own early-out: at zero
    /// `night_air` returns its argument before touching anything else, which is
    /// one uniform-coherent compare per fragment in daylight.
    pub night: f32,
    /// Density at [`Self::deck`], per yard. At full night this gives an optical
    /// depth of about 1.1 over the graded 275-yard fog distance, so roughly a
    /// third of a distant fragment survives.
    pub density: f32,
    /// The height over which that density falls by `1/e`, in yards. Small
    /// enough that a hilltop stands out of the mist and a valley floor is
    /// buried in it, which is the whole reason the integral is worth taking.
    pub falloff: f32,
    /// How hard the mist glows toward the moon.
    pub glow: f32,
    /// **Where the deck sits, in world Z.**
    ///
    /// The player's own altitude, re-read whenever the light resolves — every
    /// four yards of travel, see `sky::RESOLVE_CELL`. A *fixed* height was the
    /// obvious alternative and it does not work: this game's ground runs from
    /// below −500 in Thousand Needles to above +700 inside Ironforge, so any one
    /// sea level either drowns half the world or is invisible in the other half.
    /// A deck that follows the ground is not what real air does, and it is the
    /// only version of this that is visible everywhere.
    pub deck: f32,
}

/// Density at the deck at full night, per yard. See [`NightAir::density`].
///
/// **Halved after the first look.** The mist mixes every fragment toward the
/// horizon band, so density is *directly* how washed out the picture is: at
/// 0.004 a surface thirty yards away had already lost a tenth of itself to the
/// air and one at a hundred yards a third, which over a whole frame reads as a
/// haze laid over everything rather than as mist standing in the low ground.
const DENSITY: f32 = 0.0022;

/// The `1/e` height of that density, in yards. See [`NightAir::falloff`].
const FALLOFF: f32 = 30.0;

/// How much of the moon's own colour the mist adds back, looking at it.
///
/// **Also pulled back, and its exponent tightened with it** (see
/// `atmosphere.wgsl`'s `NIGHT_GLOW_POWER`). The two together decide how much of
/// the sky is inside the halo, and at 0.6 over an exponent of 8 the answer was
/// most of it — which at any hour the table still lights warm is a broad orange
/// wash rather than a moon.
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

    /// **The five numbers, as the two fog fields they ride in.**
    ///
    /// `DistanceFog::directional_light_color` and `::directional_light_exponent`
    /// are Bevy's own light-dispersion controls and this client does not use
    /// them: `atmosphere.wgsl`'s `fogged` hands `linear_fog` a scattering vector
    /// of `vec3(0.0)`, so `scattering_adjusted_fog_color` multiplies the colour
    /// by zero whatever is in it, and `linear_fog` never reads the exponent at
    /// all. Under `FogFalloff::Linear` they are five floats already bound at
    /// group 0 for every world material, which is exactly what was needed and
    /// which no bind group of this crate's own could have been cheaper than.
    ///
    /// The round trip is exact: `bevy_pbr`'s fog extraction is
    /// `LinearRgba::from(color).to_vec4()` with no clamp, so a
    /// `Color::LinearRgba` built here arrives in the shader unchanged.
    ///
    /// **If a later Bevy starts reading either field for something else, this
    /// is the one place to look** — the symptom would be a glow around the sun
    /// in daylight rather than an error.
    pub fn pack(&self) -> (Color, f32) {
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

pub struct NightPlugin;

impl Plugin for NightPlugin {
    /// **The shader half of this module is not registered here**, and that is
    /// deliberate: `shaders/night.wgsl` is loaded by [`crate::render::sky`]
    /// beside the two libraries it sits in a dependency chain with. See the
    /// note there — a library that `atmosphere.wgsl` imports must not be behind
    /// a plugin somebody might remove.
    fn build(&self, app: &mut App) {
        app.init_resource::<NightTuning>()
            .init_resource::<Night>()
            // **After the light has been resolved**, which is what writes
            // `Night` — see `sky::resolve`. Without the ordering the present
            // pass is graded for last frame's atmosphere, which at a dusk
            // transition is a visible pulse rather than a lag.
            .add_systems(Update, grade_present.after(crate::render::sky::apply));
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report.run_if(crate::ui::report::watched));
    }
}

/// Hand the full-screen quad the night factor.
///
/// The present pass is the only place in this client that sees a **finished**
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

/// **How deep the night is**, on the HUD — which is otherwise unanswerable by
/// looking, since the whole point of the grade is a picture that is *supposed*
/// to be dark.
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

    /// **Noon is the identity, exactly**, which is the property the default-on
    /// setting rests on: every daylight measurement in this project still holds
    /// with the option in place.
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

    /// …and midnight is the whole of it.
    #[test]
    fn midnight_is_the_full_grade() {
        let mut atmosphere = midnight();
        assert_eq!(factor(&atmosphere), 1.0);
        deepen(&mut atmosphere, 1.0);
        let byte = |c: [f32; 3]| c.map(|v| (v * 255.0).round() as i32);
        // The measurement the module note is about: the fill all but goes, the
        // moon largely stays, and the gap between them is the shape in the
        // picture.
        assert_eq!(byte(atmosphere.ambient), [5, 11, 18]);
        assert_eq!(byte(atmosphere.diffuse), [60, 92, 133]);
        // A moonlit surface is still legible; one facing away is not.
        let lit = byte(atmosphere.ambient)[2] + byte(atmosphere.diffuse)[2];
        assert!(lit > 100, "the moonlit side must stay readable, got {lit}");
        assert!(byte(atmosphere.ambient).iter().all(|c| *c < 24));
    }

    /// **…and the other half: the full-screen grade follows the factor down.**
    ///
    /// `sky::resolve`'s own test pins that leaving the world puts
    /// [`Night::factor`] back to zero. This pins the thing a *player* sees,
    /// which is one system further on and was the whole of the report: the
    /// present pass grades every pixel of `WorldFrame`, and the glue scene is
    /// drawn through the world camera into that same image — so a factor left
    /// standing is a login screen left crushed, cooled and vignetted.
    ///
    /// Written as a wiring test rather than reasoned about because the two are
    /// joined only by an `is_changed` guard and an `.after`, and neither fails
    /// loudly when it stops being true.
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

        // …and the world ends.
        app.world_mut().resource_mut::<Night>().factor = 0.0;
        app.update();
        assert_eq!(
            graded(&app),
            0.0,
            "the character screen must be drawn ungraded"
        );
    }

    /// **The screens before the world are never graded**, which is half of why
    /// the option is allowed to default to on.
    ///
    /// `Atmosphere::PLACEHOLDER` is what `sky::resolve` leaves standing when
    /// there is no session — before the first login, and again the moment one
    /// ends — so the login screen, character select and the Dark Portal behind
    /// them are lit by it. Its factor has to be exactly zero or all three read
    /// as a faded night, which is precisely the report this rule was written
    /// for. It is asserted here rather than reasoned about because the
    /// placeholder is the mean of nineteen daylight rows and a table edit could
    /// move it under the ramp without anything else noticing.
    #[test]
    fn the_glue_screens_are_never_night() {
        assert_eq!(factor(&Atmosphere::PLACEHOLDER), 0.0);
        let mut lit = Atmosphere::PLACEHOLDER;
        let n = NightTuning::default().applied_to(&lit);
        deepen(&mut lit, n);
        assert_eq!(lit.ambient, Atmosphere::PLACEHOLDER.ambient);
        assert_eq!(lit.diffuse, Atmosphere::PLACEHOLDER.diffuse);
        // …and with nothing burning, since the lamps are scaled by the same
        // number — see `render::lamps`.
        assert_eq!(NightAir::new(factor(&Atmosphere::PLACEHOLDER), 0.0).density, 0.0);
    }

    /// **21:00 is dusk and not night**, which is the hour the ramp was placed
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

    /// **The five numbers survive the fog fields they ride in**, which is the
    /// one thing about the transport that could break silently — see
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

    /// Off means off, and a strength of zero means off too — a flag that
    /// disabled the grade but left the factor non-zero would leave the shader's
    /// branch live for nothing.
    #[test]
    fn the_flag_parses_the_three_forms() {
        let midnight = midnight();
        assert_eq!(NightTuning::parse(None).applied_to(&midnight), 1.0);
        assert_eq!(NightTuning::parse(Some("off")).applied_to(&midnight), 0.0);
        assert_eq!(NightTuning::parse(Some("0")).applied_to(&midnight), 0.0);
        assert_eq!(NightTuning::parse(Some("0.5")).applied_to(&midnight), 0.5);
        assert_eq!(NightTuning::parse(Some("on")).applied_to(&midnight), 1.0);
        // Out of range warns and leaves the default, rather than grading by 4.
        assert_eq!(NightTuning::parse(Some("4")).applied_to(&midnight), 1.0);
        assert_eq!(NightTuning::parse(Some("dark")).applied_to(&midnight), 1.0);
        // …and the switch is honoured whatever the light says.
        let off = NightTuning {
            enabled: false,
            ..NightTuning::default()
        };
        assert_eq!(off.applied_to(&midnight), 0.0);
    }
}
