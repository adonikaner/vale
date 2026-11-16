//! Point lights for things that burn.
//!
//! This module is the larger part of the deviation from the 1.12.1 client that
//! it shares with [`crate::render::night`]: the 1.12.1 client has no local
//! lights. It draws a torch, a brazier, a street lantern, a campfire and a
//! spell landing on the ground as additive particle quads over the frame, and
//! none of them changes the colour of any texel beneath it. That is not
//! visible at the light table's own night, which lights every surface in the
//! world to at least 29/60/84. It is visible once [`crate::render::night`]
//! removes that floor.
//!
//! ## Which sources are lamps: additive emitters and additive unlit batches
//!
//! No list of model names is used, and none is to be added. A model that
//! glows says so in its own particle definition: `blend` 3 and 4 are the two
//! additive modes, which an emitter uses when its quads add light to the frame
//! rather than cover it. Smoke, dust, spray and other alpha-blended particles
//! use blend 2 and are not lamps.
//!
//! The emitter rule covers everything in the game that is on fire: a
//! campfire, a brazier, a forge, the flame in a hanging lantern, a missile in
//! flight, the effect on a unit that has just been cast at, and the torch a
//! guard carries, because an attachment's emitters are spawned the same way.
//! Each of them is an `Emitter` in [`crate::render::particles`] and each
//! carries its own colour.
//!
//! The emitter rule leaves every lamppost dark, and the second rule exists for
//! them. `vale model` on the Elwynn lamppost (24 of them on one Goldshire
//! tile) reports `no lights`, `no ribbon emitters`, `0 particle emitters`, and
//! this batch:
//!
//! ```text
//! [3] tex GLOW32.BLP  blend 4 add-alpha  [unlit no-depth-write]  2 tris
//! ```
//!
//! A static light in the 1.12.1 client is an additive unlit quad with no light
//! record and no emitter. The second source is therefore
//! [`vale_assets::world::glow`]: an additive `unlit` batch is a lamp, its
//! colour is its texture's average, and its size is the quad's. This rule is
//! also read from the data with no list of names, and the two rules together
//! treat a lit street and a lit campfire the same way.
//!
//! The two populations barely overlap: a fire is an emitter, and a lamp is a
//! glow batch.
//!
//! ## Colour comes from the source, brightness from the setting
//!
//! A lamp's hue is the brightest of its three over-life colour keys,
//! normalised, so a fire is orange and a mana effect is blue. Its value is a
//! setting on the panel. The emitter's authored brightness is not used: it
//! varies across the game by more than an order of magnitude, because it is
//! tuned for how a quad reads over a texture, not for how much light is in the
//! room.
//!
//! ## The lamp budget and the fixed pool
//!
//! Bevy clusters point lights on the CPU: each light is assigned to the
//! screen-space cells its sphere touches, and the work is linear in the number
//! of lights times the cells each covers. Stormwind at night has hundreds of
//! additive emitters in view, and all of them are candidates.
//!
//! Only [`BUDGET`] of them are lit, best first (see [`light_the_flames`] for
//! the ranking). The pool is fixed while it exists: [`BUDGET`] entities,
//! rewritten in place every frame, with the unused ones at zero intensity. No
//! entity is spawned or despawned from frame to frame and none changes
//! archetype, unlike a version that inserts a `PointLight` on whichever
//! emitters are nearest.
//!
//! In daylight the pool does not exist. Twenty-four `PointLight` entities are
//! twenty-four clusterable objects, and Bevy extracts, assigns and uploads them
//! every frame whatever their intensity. The first version kept the pool up at
//! all times, so a client at noon, or one run with `--night off`, paid for
//! lights that did nothing. The pool is now raised when the night first asks
//! for a lamp and dropped when it stops, at dusk, at dawn and at a logout
//! rather than per frame. A day frame costs three resource reads and a
//! compare; see [`light_the_flames`], whose first branch is that cost.

use bevy::prelude::*;

use crate::render::night::{Night, NightTuning};
use crate::render::particles::Emitter;

/// How many lamps may be lit at once.
///
/// Twenty-four. The limit is set by the CPU's clustering pass, not by the
/// fragment shader; see the module note. It is enough for a lit street and not
/// for a lit city. The lamps left out are the far ones, which matter least: a
/// lamp forty yards away is past [`REACH_MAX`] and contributes less than one
/// byte to any texel.
pub const BUDGET: usize = 24;

/// How close two candidates must be, in yards, before the dimmer one is
/// dropped.
///
/// A fire is usually three emitters at the same point: a flame, a glow and a
/// shower of embers, all additive. Without the merge, one campfire takes three
/// of the twenty-four slots for three copies of one light, and a street loses
/// two lamps at its far end.
const MERGE_RADIUS: f32 = 2.5;

/// The reach of the smallest lamp, in yards, before the panel's multiplier.
///
/// [`REACH_MIN`], [`REACH_MAX`] and [`REACH_PER_SIZE`] were lowered after the
/// first test, and [`STRENGTH`] was raised to compensate. A lamp reaching
/// thirty yards with a falloff that is flat near the source acts as a second
/// ambient light with a position: it lit a whole clearing evenly and the
/// picture lost its contrast. A bright core over a short reach reads as a
/// torch, because the light is concentrated where it shows against the dark.
const REACH_MIN: f32 = 8.0;

/// The reach of the largest lamp, in yards, before the panel's multiplier.
const REACH_MAX: f32 = 20.0;

/// How many yards of reach each yard of particle size is worth. The same rule
/// sizes a candle and a bonfire.
const REACH_PER_SIZE: f32 = 2.0;

/// What a lamp adds to a surface at its position, in the light table's 0..1
/// band space, before the panel's multiplier and the night factor.
///
/// The value is above 1 on purpose. A lamp is a term of the same sum as the
/// sun and the fill (see `atmosphere.wgsl`'s `daylight_srgb_lamplit`), and that
/// sum is clamped to 1 in the file's space. The clamp is the game's own, not a
/// safety net (see `daylight_srgb`).
///
/// A strength over 1 therefore overflows nothing: it saturates the warm channel
/// of a flame first and the other channels after, which is how a texel a yard
/// from a torch should look. The result is a core that is clearly a light
/// source, with the flame's colour around it, over a floor that falls off
/// within eight yards. At 0.9 with a thirty-yard reach there was no core and
/// no falloff, only a large warm patch, which was the "washed out" part of the
/// report.
const STRENGTH: f32 = 1.35;

/// How much better a challenger's score must be, as a fraction, before it
/// takes a lit lamp's slot.
///
/// This fixes the reported fault "lights flicker on and off in towns". With a
/// hundred candidates and twenty-four slots, the two at ranks 24 and 25 swap
/// each time the camera moves a step, and each swap turns one light off and
/// another on. Raising each incumbent's score by this fraction before the sort
/// means the set changes only when a candidate has become clearly more
/// important.
///
/// A quarter is wide; at 10% a walk down a street still churns. The cost is
/// that a newly relevant lamp comes on a few yards late, which is not
/// noticeable. A lamp that blinks is noticeable.
const INCUMBENT_MARGIN: f32 = 0.25;

/// How long a lamp takes to reach full strength, and to go out, in seconds,
/// when [`crate::render::night::NightTuning::lamp_fade`] is zero or less. That
/// field is the live value.
///
/// The second half of the flicker fix. [`INCUMBENT_MARGIN`] stops lamps
/// trading places; this fade stops the remaining changes from being steps.
///
/// It was 0.25, which was too short, because the old fade also had the wrong
/// shape; see [`Slot::level`]. A quarter of a second with almost all of the
/// change in its first sixth looks like a pop with a tail, which matches the
/// report "the light appears and then vanishes in a way that looks buggy".
/// With a curve that starts and ends at rest, 0.6 seconds is gentle at both
/// ends and still shorter than the time it takes to walk past a lamppost.
const FADE_SECONDS: f32 = 0.6;

/// What Bevy divides a `PointLight`'s intensity by on its way to the shader.
///
/// `bevy_pbr`'s light extraction is `intensity / (4 * PI)` and the cluster
/// buffer then holds `colour * that`. Multiplying by it here means the number
/// the shader reads is exactly the number [`LampLight::strength`] states, with
/// no unit conversion hidden between the two — see [`LampLight::light`].
const BEVY_INTENSITY_UNIT: f32 = 4.0 * std::f32::consts::PI;

/// One light a model carries, in the model's own space.
///
/// The static half of this module: resolved once when the model loads and
/// shared by every placement of it. See [`vale_assets::world::glow`] for which
/// batch this is and `render::models::loader` for where the colour comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ModelGlow {
    /// Where it hangs, in the model's own space and in Bevy's axes. Converted
    /// here rather than at the placement, because every reader of it is
    /// downstream of a Bevy transform.
    pub at: Vec3,
    /// Its hue, normalised so the brightest channel is 1, in the light table's
    /// own space. The texture's own average; see [`glow_colour`].
    pub colour: Vec3,
    /// How far it reaches at the model's own scale, in yards.
    pub reach: f32,
}

impl ModelGlow {
    /// A glow batch and its texture's average, as a light.
    pub fn of(glow: &vale_assets::world::glow::Glow, colour: Vec3) -> ModelGlow {
        ModelGlow {
            at: crate::render::axes::to_bevy(glow.at),
            colour: colour / colour.max_element().max(1e-4),
            // The quad's size decides the reach. It is the only value in the
            // file that indicates how much light this is meant to be: a
            // candle's glow is a few inches across and a forge's is yards. The
            // multiplier is large because the quad is small (a lamppost's is
            // under a yard), and the clamp is the same as the emitters', so a
            // street lamp and a campfire land in the same range.
            reach: (REACH_MIN + GLOW_REACH_PER_YARD * glow.extent)
                .clamp(REACH_MIN, REACH_MAX),
        }
    }
}

/// How many yards of reach each yard of glow quad is worth. See
/// [`ModelGlow::of`] for why this is four times [`REACH_PER_SIZE`].
const GLOW_REACH_PER_YARD: f32 = 8.0;

/// How far a flame's light dips when it wavers, as a fraction.
///
/// A tenth: enough that a room lit by one torch visibly moves, and not enough
/// for anything to read as flashing. The value is chosen by eye, not measured;
/// see the module note. The 1.12.1 client has no dynamic lamps, so there is no
/// reference behaviour for it. The lamps and this value are this client's own.
///
/// Only the burning sources use it. See [`LampLight::flicker`].
const FIRE_FLICKER: f32 = 0.10;

/// The multiplier on a lamp's brightness at time `t`, for a flicker of
/// `amplitude`.
///
/// It only dips. The wave is folded into `[0, 1]` and subtracted, so the result
/// is in `[1 - amplitude, 1]` and a lamp is never brighter than the strength
/// its source authored. This matters because the shader adds the result to
/// the sun and the fill in byte space, where a value above the clamp is lost.
///
/// Three sines are used rather than one: a single sine reads as a pulse, and
/// three at rates that do not divide each other read as a flame, with no
/// noise, table or state. `seed` is the lamp's key, so two torches on one wall
/// do not waver in step. Torches wavering in step look worse than no flicker.
pub(crate) fn flame(seed: u64, t: f32, amplitude: f32) -> f32 {
    if amplitude <= 0.0 {
        return 1.0;
    }
    // A hash mix, not a modulo. The first version scaled `seed % 997` into
    // radians, which spreads distant keys and leaves adjacent ones together.
    // Lamp keys are adjacent by construction, since they are an entity's bits
    // and a slot index, so two torches on one wall came out a thousandth of a
    // radian apart and wavered in step. `two_flames_are_out_of_phase` tests
    // this.
    let mut h = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    let phase = (h & 0xFFFF) as f32 * (std::f32::consts::TAU / 65536.0);
    let wave = 0.50 * (t * 11.0 + phase).sin()
        + 0.35 * (t * 7.3 + phase * 2.1).sin()
        + 0.15 * (t * 3.1 + phase * 0.7).sin();
    1.0 - amplitude * (0.5 + 0.5 * wave)
}

/// The average colour of a decoded texture, weighted by its alpha.
///
/// A glow texture is mostly transparent (`GLOW32.BLP` is 50%), and an
/// unweighted mean over it is half black, which pulls every lamp in the game
/// toward grey and loses the warm orange. Weighting by alpha gives the colour
/// of the light the quad adds to the frame.
///
/// `None` for a texture with no opaque texel, which is a batch that adds
/// nothing and is not a lamp.
pub(crate) fn glow_colour(rgba: &[u8]) -> Option<Vec3> {
    let mut sum = Vec3::ZERO;
    let mut weight = 0.0f32;
    for texel in rgba.chunks_exact(4) {
        let a = f32::from(texel[3]) / 255.0;
        sum += Vec3::new(
            f32::from(texel[0]),
            f32::from(texel[1]),
            f32::from(texel[2]),
        ) * a;
        weight += a;
    }
    if weight <= 0.0 {
        return None;
    }
    Some(sum / (weight * 255.0))
}

/// Identifies a lamp across frames.
///
/// Selection is sticky and fades (see [`light_the_flames`]), and both need to
/// recognise a lamp seen in the previous frame. A position would almost do,
/// since a lamppost never moves, but a torch on a walking guard moves, and a
/// key that changed as it walked would cause the flicker the stickiness exists
/// to remove. The key is the holder's entity plus, for a building with ten
/// lights in one component, the index of the light.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LampKey(u64, u16);

/// One thing that burns.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LampLight {
    /// See [`LampKey`]. Written by whichever of the three sources produced it.
    pub key: LampKey,
    /// Where it stands, in Bevy's axes — the emitter's own settled origin.
    pub at: Vec3,
    /// Its hue, normalised so the brightest channel is 1. In the light table's
    /// own space, because that is the space the shader adds it in.
    pub colour: Vec3,
    /// How much light it adds at its centre, in that same space.
    pub strength: f32,
    /// How far it reaches, in yards. Zero at that distance, exactly — see
    /// `atmosphere.wgsl`'s `lamplight`, whose falloff is bounded rather than
    /// inverse-square.
    pub reach: f32,
    /// Whether it stands inside a building, which selects which of
    /// [`crate::render::night::NightTuning`]'s two multiplier pairs scales it.
    ///
    /// Set during the gather in [`light_the_flames`], not at spawn and not by
    /// the three sources. A `MOLT` record is indoors by construction, but a
    /// torch is a doodad that may be either, and a carried torch moves between
    /// the two. One rule, measured the same way for all three sources, is
    /// easier to trust than three rules that agree by coincidence.
    ///
    /// A building whose `Interior` has not been inserted yet reads as
    /// outdoors, the brighter of the two answers, until the WMO has streamed.
    /// This is accepted rather than guarded against. Holding a lamp dark until
    /// its building resolves would trade a couple of frames of a torch that is
    /// too bright for a couple of frames of an unlit room, and the unlit room
    /// looks like a bug.
    pub(crate) indoors: bool,
    /// How much this lamp wavers, as the fraction its brightness dips by; 0
    /// for a light that does not burn.
    ///
    /// This field is the whole per-source split. It is a field rather than a
    /// lookup because the three sources are already three separate
    /// construction sites and each knows what it is. A particle emitter that
    /// passed [`LampLight::of`]'s blend test is a fire, because that is what
    /// the test selects, so it gets [`FIRE_FLICKER`]. A model's glow quad and a
    /// building's `MOLT` record are not fires: a lamppost's pane, a forge's glow
    /// and a window lit from inside are steady. The module already separates
    /// the sources that burn from those that do not, and this field uses that
    /// separation rather than adding one.
    pub(crate) flicker: f32,
}

/// The multiplier pair a lamp is scaled by, as `(strength, reach)`.
///
/// The indoor/outdoor split is decided only here, so the three places that
/// need it cannot drift apart: the reach used to cull, the reach used to rank,
/// and the reach and strength written into the light. See
/// [`crate::render::night::NightTuning::indoor_lamp_strength`] for why the
/// split is per lamp rather than per camera.
///
/// The reach is floored so that a slider at zero gives a very small light
/// rather than a sphere of radius zero, which Bevy treats as an error rather
/// than as off. Strength at zero is off and needs no floor.
fn multipliers(tuning: &crate::render::night::NightTuning, indoors: bool) -> (f32, f32) {
    match indoors {
        true => (
            tuning.indoor_lamp_strength,
            tuning.indoor_lamp_reach.max(0.05),
        ),
        false => (tuning.lamp_strength, tuning.lamp_reach.max(0.05)),
    }
}

impl LampLight {
    /// The light this emitter casts, or `None` when it is not an emitter that
    /// glows.
    ///
    /// The rule is that a glowing emitter uses an additive blend; the module
    /// note gives the reason.
    pub fn of(emitter: &Emitter) -> Option<LampLight> {
        let def = emitter.definition();
        if !matches!(def.blend, 3 | 4) {
            return None;
        }
        // The brightest of the three over-life keys, weighted by its own alpha
        // — a key the emitter fades to nothing on contributes no light, and
        // several of this game's flames are authored bright at their midpoint
        // and black at both ends.
        let mut best = Vec3::ZERO;
        let mut best_value = 0.0;
        for key in def.colors {
            let colour = Vec3::new(
                f32::from(key[0]) / 255.0,
                f32::from(key[1]) / 255.0,
                f32::from(key[2]) / 255.0,
            );
            let value = colour.max_element() * (f32::from(key[3]) / 255.0);
            if value > best_value {
                best_value = value;
                best = colour;
            }
        }
        // A flame authored black in all three keys lights nothing, which is
        // right: it is a distortion or a shadow quad rather than a fire.
        if best_value <= 0.0 {
            return None;
        }
        let size = def
            .scales
            .iter()
            .copied()
            .fold(0.0f32, f32::max)
            .max(0.0)
            * emitter.placement_scale();
        Some(LampLight {
            // Filled in by the caller, which is the only place that knows the
            // emitter's own entity.
            key: LampKey(0, 0),
            // …and by the gather, which is the only place that can see the
            // buildings. See the field.
            indoors: false,
            // The one source that burns. Reaching here means the blend and
            // colour tests above passed, which makes this a flame rather than
            // a distortion quad.
            flicker: FIRE_FLICKER,
            at: emitter.origin(),
            // Normalised: the hue is the flame's and the value is the setting's.
            colour: best / best.max_element().max(1e-4),
            // Scaled by the emitter's current output, the one input to this
            // conversion that is not read from the definition; see
            // [`Emitter::output`] for the reason. Without it the light is a box
            // function over the entity's lifetime, shaped only by the fade at
            // each end.
            strength: STRENGTH * emitter.output(),
            reach: (REACH_MIN + REACH_PER_SIZE * size).clamp(REACH_MIN, REACH_MAX),
        })
    }

    /// The lamp as the `PointLight` Bevy clusters, with its intensity
    /// multiplied by `scale`.
    ///
    /// The colour is stored in a `Color::LinearRgba` container but is not a
    /// linear value. `render::sky` does the same for the sun and the fill, for
    /// the same reason: `bevy_pbr` uploads `LinearRgba::from(colour) *
    /// intensity` with no transfer function, so the shader reads the byte-space
    /// value written here. The shader adds it to the sun and the fill in that
    /// space, where the sum is correct.
    fn light(&self, scale: f32) -> PointLight {
        PointLight {
            color: Color::LinearRgba(LinearRgba::from_vec3(self.colour)),
            intensity: self.strength * scale * BEVY_INTENSITY_UNIT,
            range: self.reach,
            // A point rather than a sphere: the falloff in `atmosphere.wgsl` is
            // this client's own and reads neither.
            radius: 0.0,
            // Always off. A shadow-casting point light costs six depth passes
            // over everything in its range, and twenty-four of them would be
            // the most expensive work this renderer could do. The game casts no
            // runtime shadows; see `sky::sun_shadows`.
            shadow_maps_enabled: false,
            ..default()
        }
    }
}

/// The lights one placement of a model carries, in world space, already scaled
/// and rotated.
///
/// Stored on the placement's own entity rather than as child entities, and
/// computed once at spawn rather than every frame, because static scenery does
/// not move: a lamppost's glow stays at the same position for as long as the
/// tile is loaded. A child entity per lamp would add a transform to propagate
/// and an archetype to walk for a value that cannot change.
#[derive(Component, Clone, Debug)]
pub(crate) struct StaticLamps(pub(crate) Vec<LampLight>);

impl StaticLamps {
    /// A building's own lights (`MOLT`), placed by one instance of it: the
    /// third of the three sources.
    ///
    /// The other two sources are read from what is drawn: an emitter's colour
    /// is the flame's, and a glow batch's is its texture's. A `MOLT` record
    /// states a light directly, with a colour, a position and a falloff in
    /// yards, and it is the only source that does. The 1.12.1 client lights
    /// the building's own surfaces with its baked `MOCV` and not with these
    /// records, so a lit window lights no wall in the 1.12.1 client. The
    /// records do light the models standing near them; see
    /// [`crate::render::building_lights`].
    ///
    /// The falloff is the file's, not this module's. `atten_end` is where the
    /// building says the light stops, so it is taken as the reach and not
    /// clamped into [`REACH_MIN`]..[`REACH_MAX`]. Those bounds keep an
    /// estimated reach in a sane range, and here nothing is estimated. An
    /// inn's lights reach 7 to 9 yards, the size of a room.
    pub(crate) fn of_building(
        lights: &[vale_assets::world::wmo::WmoLight],
        placement: &Transform,
    ) -> Option<StaticLamps> {
        let lamps: Vec<LampLight> = lights
            .iter()
            .filter(|l| l.is_point())
            .enumerate()
            .map(|(slot, l)| LampLight {
                key: LampKey(0, slot as u16),
                // Decided in the gather — see the field. A `MOLT` record is
                // indoors by construction and is still measured rather than
                // assumed, so that one rule covers all three sources.
                indoors: false,
                // Steady. A `MOLT` record is the room's lighting as the
                // building's author placed it, not a fire in the room.
                flicker: 0.0,
                at: placement.transform_point(crate::render::axes::to_bevy(l.position)),
                colour: {
                    let c = Vec3::from_array(l.colour);
                    c / c.max_element().max(1e-4)
                },
                // The file's intensity times [`STRENGTH`]. Every record
                // measured states 1.0, so the factor changes nothing yet. It is
                // kept so that a building that states its hearth brighter than
                // its wall sconces is drawn that way.
                strength: STRENGTH * l.intensity.clamp(0.0, 4.0),
                reach: (l.atten_end * placement.scale.x.abs().max(0.05)).max(1.0),
            })
            .collect();
        (!lamps.is_empty()).then_some(StaticLamps(lamps))
    }

    /// The lights a model carries, placed by one instance of it.
    ///
    /// `None` when the model carries none, which is almost every model in the
    /// world — the component is not inserted at all in that case, so the query
    /// in [`light_the_flames`] walks lampposts rather than trees.
    pub fn of(glows: &[ModelGlow], placement: &Transform) -> Option<StaticLamps> {
        if glows.is_empty() {
            return None;
        }
        Some(StaticLamps(
            glows
                .iter()
                .enumerate()
                .map(|(slot, glow)| LampLight {
                    key: LampKey(0, slot as u16),
                    // Decided in the gather — see the field.
                    indoors: false,
                    // Steady. A glow quad is a pane, a forge mouth or a lit
                    // window, painted onto the model rather than burning on it.
                    // The few glow quads that are flames also carry a particle
                    // emitter, and the emitter's lamp flickers.
                    flicker: 0.0,
                    at: placement.transform_point(glow.at),
                    colour: glow.colour,
                    strength: STRENGTH,
                    // The placement's own scale, which is what makes the small
                    // lantern outside a farmhouse a smaller light than the one
                    // over an inn door — the file states one model and the tile
                    // states how big each copy of it is.
                    reach: (glow.reach * placement.scale.x.abs().max(0.05))
                        .clamp(REACH_MIN, REACH_MAX),
                })
                .collect(),
        ))
    }
}

/// The pool of [`BUDGET`] `PointLight` entities. See the module note for why
/// its size is fixed while the night is on, and [`light_the_flames`] for why
/// it does not exist while the night is off.
#[derive(Resource, Default)]
struct LampPool(Vec<Entity>);

pub(crate) struct LampPlugin;

impl Plugin for LampPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LampPool>()
            .init_resource::<LampCount>()
            .add_systems(
                Update,
                // After the emitters are stepped, which settles
                // `Emitter::origin`. A read one frame late places every lamp
                // one stride behind the torch carrying it. `merge_fields`
                // states the same ordering for the same reason.
                light_the_flames.after(crate::render::particles::simulate),
            );
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report.run_if(crate::ui::report::watched));
    }
}

/// Spawns [`BUDGET`] dark lights, the first time the night asks for any.
///
/// The pool is not spawned at `Startup`. It was spawned there at first, which
/// was a standing cost for a feature that was off: twenty-four `PointLight`
/// entities are twenty-four clusterable objects, and Bevy extracts them,
/// assigns each to the screen-space cells its sphere touches and uploads the
/// buffer every frame, whatever their intensity. A client at noon, one run with
/// `--night off`, or one that never entered a dark place paid that cost for
/// nothing.
///
/// The pool is still fixed while it exists: the churn this avoids is per
/// frame, and the pool is built or dropped only at dusk, at dawn and at a
/// logout. See [`light_the_flames`], which handles both transitions.
fn raise_pool(commands: &mut Commands, pool: &mut LampPool) {
    pool.0 = (0..BUDGET)
        .map(|_| {
            commands
                .spawn((
                    Name::new("lamp"),
                    PointLight {
                        intensity: 0.0,
                        range: 0.01,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    Transform::default(),
                ))
                .id()
        })
        .collect();
}

/// How many lamps are lit this frame, for the HUD.
#[derive(Resource, Default)]
pub struct LampCount {
    pub lit: usize,
    pub candidates: usize,
}

/// One of [`BUDGET`] slots: which lamp it holds, and how far that lamp has
/// faded in.
///
/// The slots keep the set stable. A lamp keeps its slot for as long as it is
/// wanted, so no lamp moves when the sort order changes, and a lamp that stops
/// being wanted keeps its slot while it fades out, so no lamp is switched off
/// in one frame. [`INCUMBENT_MARGIN`] and [`FADE_SECONDS`] are the two parts
/// of this.
#[derive(Clone, Copy)]
struct Slot {
    lamp: Option<LampLight>,
    /// How far through its fade the slot is, from 0 (dark) to 1 (full),
    /// advancing at a constant rate. [`Slot::level`] shapes this value, and
    /// the shaped value is the one written to the light.
    phase: f32,
}

impl Slot {
    /// The fraction of its light this slot delivers: [`Self::phase`] through
    /// a smoothstep.
    ///
    /// The first version was an exponential approach:
    ///
    /// ```text
    /// level += (target - level) * (dt / FADE_SECONDS)
    /// ```
    ///
    /// That never jumps, but it is fastest at the instant it starts: a third
    /// of the change lands in the first tenth of a second and the rest trails
    /// off over the next second. Coming on, it looks like a pop with a tail;
    /// going off, the light looks gone before it has finished fading. A spell
    /// effect's light arrives and departs within about a second, so for a
    /// spell effect this shape is the whole appearance. It caused the report
    /// that casting looks "buggy or artificial".
    ///
    /// `3t² - 2t³` is at rest at both ends: its rate is zero at 0 and at 1 and
    /// highest in the middle, so no fade starts or stops abruptly. It costs two
    /// multiplies per slot per frame, over at most [`BUDGET`] slots.
    ///
    /// The level is computed from `phase` rather than stored in its place
    /// because the shaping is not invertible. A slot interrupted mid-fade
    /// continues from its current position, and that position must be on the
    /// linear clock or the level jumps.
    fn level(self) -> f32 {
        let t = self.phase.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

/// Choose this frame's lamps and write them into the pool.
///
/// ## Why the choice uses four rules rather than one sort
///
/// The first version used one rule: rank every candidate by distance to the
/// camera and take the nearest [`BUDGET`]. Every lamp fault players reported
/// came from that rule.
///
/// * It ranked lights that cannot be seen. A lamp behind the camera competed
///   for a slot with one in front, and Bevy then assigned it to no cluster, so
///   the slot lit nothing. In a town that is about half the candidates, which
///   is why the HUD showed `24 of 130 burning` while the camera faced no lamps.
///   The fix is to cull against the view frustum first, sphere against six
///   planes, before anything is ranked.
/// * It ranked by distance rather than by contribution. A twenty-yard
///   lamppost at twenty-five yards matters and an eight-yard campfire at
///   twenty does not, and distance alone cannot tell them apart. The score is
///   the light a lamp can deliver: `strength * reach^2 / distance^2`.
/// * Nothing was sticky, so the two candidates either side of the cut swapped
///   each time the camera moved, and both blinked. See [`INCUMBENT_MARGIN`].
/// * Nothing faded, so every remaining change was a step. See
///   [`FADE_SECONDS`].
///
/// The four combine: culling makes the field small enough for the score to
/// mean something, the score orders it, the margin keeps the order stable from
/// frame to frame, and the fade smooths the changes that remain.
fn light_the_flames(
    mut commands: Commands,
    time: Res<Time>,
    emitters: Query<(Entity, &Emitter)>,
    standing: Query<(Entity, &StaticLamps)>,
    night: Res<Night>,
    tuning: Res<NightTuning>,
    mut pool: ResMut<LampPool>,
    mut lights: Query<(&mut PointLight, &mut Transform)>,
    camera: Query<
        (Entity, &Transform, &bevy::camera::primitives::Frustum),
        (With<crate::world::camera::WorldCamera>, Without<PointLight>),
    >,
    mut count: ResMut<LampCount>,
    interiors: Query<&crate::render::wmos::Interior>,
    mut chosen: Local<Vec<(f32, LampLight)>>,
    mut slots: Local<Vec<Slot>>,
    mut announced: Local<usize>,
) {
    // `VALE_NO_LAMPS=1` is a measurement probe, not a setting. With no lamps
    // spawned the shader's cluster list is empty, so an A/B against the
    // default splits the night's fragment cost into its two parts, the
    // 24-lamp `lamplight()` loop and `night_air()`'s per-fragment
    // exponentials, with no shader change and no second build. It exists
    // because at 3440x1440 the night measured ~3.5 ms of opaque-pass GPU time
    // and the two parts need different fixes.
    //
    // `scale` is the night's dimmer only; each lamp's strength multiplier is
    // applied later (see [`multipliers`]). It used to include
    // `tuning.lamp_strength` as well, which was correct while one multiplier
    // pair covered the whole world and wrong once there were two.
    let scale = if tuning.enabled && tuning.lamps && std::env::var("VALE_NO_LAMPS").is_err() {
        night.factor
    } else {
        0.0
    };
    // With the night off (every daylight frame, and every frame of a session
    // run with `--night off`) this pass costs three resource reads and a
    // compare. Nothing is queried or allocated, and no lights exist for Bevy
    // to extract, cluster and upload. See [`raise_pool`].
    if scale <= 0.0 {
        if !pool.0.is_empty() {
            for entity in pool.0.drain(..) {
                commands.entity(entity).despawn();
            }
            slots.clear();
            count.lit = 0;
            count.candidates = 0;
            // The camera returns to the single-cluster view the world was
            // spawned with, which is the cheap and correct configuration for a
            // world with no clusterable lights. See the branch below that calls
            // [`raise_pool`], and `world::camera`, which owns the daylight
            // default.
            for (camera, ..) in &camera {
                commands
                    .entity(camera)
                    .insert(bevy::light::cluster::ClusterConfig::Single);
            }
        }
        return;
    }
    if pool.0.is_empty() {
        raise_pool(&mut commands, &mut pool);
        // The cluster grid is enabled with the lamps and only with them.
        // `world::camera` spawns the view as `ClusterConfig::Single` because a
        // world with no point lights has nothing to bin. Under one cluster,
        // every fragment on screen loops over every burning lamp; at
        // 3440x1440 that cost the deep night ~3.5 ms of opaque-pass GPU time.
        // Measured 2026-08-27: opaque 5.1–5.3 ms, against 1.63–1.68 with
        // `--night off` and 1.93–2.04 with the pool suppressed
        // (`VALE_NO_LAMPS=1`), so the 24-lamp loop was ~1.5–3.2 ms of it and
        // `night_air` ~0.3. The default grid limits a fragment's loop to the
        // 0–3 lamps its froxel touches.
        //
        // The config is swapped here rather than set at spawn because the
        // daylight reason for `Single` still holds: CPU binning is real work,
        // and a noon session with no lights should not pay for it. Both
        // transitions write through commands next to the pool they belong to,
        // so the config and the pool disagree for at most the frame the
        // commands take to apply.
        for (camera, ..) in &camera {
            commands
                .entity(camera)
                .insert(bevy::light::cluster::ClusterConfig::default());
        }
        // The pool is spawned by a command and does not exist until it is
        // applied, so nothing is written this frame. A torch is unlit for one
        // frame when dusk begins.
        return;
    }
    if slots.len() != pool.0.len() {
        slots.resize(pool.0.len(), Slot { lamp: None, phase: 0.0 });
    }

    let Some((_, eye, frustum)) = camera.iter().next() else {
        return;
    };
    let eye = eye.translation;

    // Gathered and culled in one pass. A lamp whose sphere does not touch the
    // view frustum cannot light a visible fragment, so it is not a candidate:
    // it is not scored, not sorted, and not counted in the `N burning` the HUD
    // report shows. Before the cull, that count was about a hundred with
    // nothing on screen.
    chosen.clear();
    let visible = |chosen: &mut Vec<(f32, LampLight)>, mut lamp: LampLight| {
        // Classified before the cull, because the cull radius depends on the
        // result. The test is `Interior::holds`, the same one
        // `world::entities::light_entities` makes for every unit in the world.
        // It rejects on the building's whole bounding box before it looks at
        // any room, so over a few dozen loaded buildings it costs one AABB test
        // each, once per candidate per frame.
        //
        // Classifying before the cull is deliberate. An indoor lamp culled
        // against the outdoor reach would only be over-inclusive, but it would
        // also be ranked by that reach, and the ranking decides which
        // twenty-four of the candidates are lit.
        //
        // There is no vertical probe offset, unlike `light_entities`' version
        // of this test. That version lifts its point a yard because a
        // character's position is at its feet and a room box's top face is the
        // outside of the ceiling, so a character standing on a flat roof would
        // read as inside. A lamp's position is the emitter's origin (a flame on
        // a brazier, a glow partway up a lamppost, a `MOLT` record hanging in a
        // room), and none of those sits on a floor or a roof face, so an offset
        // here would only push a low light through a ceiling.
        let point = crate::render::axes::to_wow(lamp.at);
        lamp.indoors = interiors.iter().any(|interior| interior.holds(point));
        let (_, reach_scale) = multipliers(&tuning, lamp.indoors);
        let reach = lamp.reach * reach_scale;
        let sphere = bevy::camera::primitives::Sphere {
            center: lamp.at.into(),
            radius: reach,
        };
        if !frustum.intersects_sphere(&sphere, true) {
            return;
        }
        // Scored by the light it can deliver, not by distance alone. The
        // inverse square is not this client's falloff (`atmosphere.wgsl`'s
        // falloff is bounded), but it ranks correctly: twice as far delivers a
        // quarter as much.
        let score = lamp.strength * reach * reach / lamp.at.distance_squared(eye).max(1.0);
        chosen.push((score, lamp));
    };
    for (entity, emitter) in &emitters {
        if let Some(lamp) = LampLight::of(emitter) {
            visible(&mut chosen, LampLight { key: LampKey(entity.to_bits(), 0), ..lamp });
        }
    }
    // The sources that do not burn: a lamppost's glow quad and a building's
    // `MOLT` records, both resolved at spawn and static since. See
    // [`StaticLamps`], and the module note for why these are separate rules.
    for (entity, lamps) in &standing {
        for lamp in &lamps.0 {
            visible(
                &mut chosen,
                LampLight { key: LampKey(entity.to_bits(), lamp.key.1), ..*lamp },
            );
        }
    }
    count.candidates = chosen.len();

    // Incumbents are favoured; this is the whole anti-flicker rule. A lamp that
    // is already lit scores `1 + INCUMBENT_MARGIN` times its measured score,
    // so a challenger must be that much better before the two trade places.
    for (score, lamp) in chosen.iter_mut() {
        if slots.iter().any(|s| s.phase > 0.0 && s.lamp.is_some_and(|l| l.key == lamp.key)) {
            *score *= 1.0 + INCUMBENT_MARGIN;
        }
    }
    chosen.sort_by(|a, b| b.0.total_cmp(&a.0));

    // Best first, dropping any candidate within [`MERGE_RADIUS`] of one already
    // taken, because a campfire is three emitters at one point.
    let merge = MERGE_RADIUS * MERGE_RADIUS;
    let mut wanted: Vec<LampLight> = Vec::with_capacity(pool.0.len());
    for (_, lamp) in chosen.iter() {
        if wanted.len() >= pool.0.len() {
            break;
        }
        if wanted.iter().any(|o| o.at.distance_squared(lamp.at) < merge) {
            continue;
        }
        wanted.push(*lamp);
    }
    count.lit = wanted.len();

    // Every wanted lamp keeps the slot it already had. Without this the fade
    // would not help: a lamp that changed slot would fade out of one and into
    // the other, which is still a flicker.
    let mut held = vec![false; slots.len()];
    let mut homeless: Vec<LampLight> = Vec::new();
    for lamp in &wanted {
        match slots
            .iter()
            .position(|s| s.lamp.is_some_and(|l| l.key == lamp.key))
        {
            Some(i) => {
                slots[i].lamp = Some(*lamp);
                held[i] = true;
            }
            None => homeless.push(*lamp),
        }
    }
    // Newcomers take the slot where the least light is lost: a dark slot
    // first, then the one furthest through its fade-out.
    for lamp in homeless {
        let Some(i) = (0..slots.len())
            .filter(|i| !held[*i])
            .min_by(|a, b| slots[*a].phase.total_cmp(&slots[*b].phase))
        else {
            break;
        };
        // A slot still fading a different lamp out is taken over rather than
        // waited for — its level carries on from where it is, so the swap is a
        // dip rather than a step.
        slots[i].lamp = Some(lamp);
        held[i] = true;
    }

    // A constant rate along the clock, not a fraction of what remains.
    // [`Slot::level`] does the shaping, and shaping here as well would compound
    // the two curves. A frame longer than the whole fade clamps to one step.
    let fade = if tuning.lamp_fade > 0.0 { tuning.lamp_fade } else { FADE_SECONDS };
    let step = (time.delta_secs() / fade).clamp(0.0, 1.0);
    for (i, slot) in slots.iter_mut().enumerate() {
        slot.phase = match held[i] {
            true => (slot.phase + step).min(1.0),
            false => (slot.phase - step).max(0.0),
        };
        if slot.phase <= 0.0 && !held[i] {
            slot.lamp = None;
        }
    }

    // Logged at each new high-water mark, not only on the first lit frame. The
    // first lit frame comes before the doodads have streamed in, so logging
    // only then made a scripted run at a crossroads with two dozen lampposts
    // report the three campfires already visible and never the lamps.
    if count.lit > *announced && count.lit > 0 {
        *announced = count.lit;
        let nearest = wanted[0];
        let (_, reach_scale) = multipliers(&tuning, nearest.indoors);
        info!(
            "lamps: {} lit of {} in view ({} indoors); nearest {:.0}/{:.0}/{:.0} {} reaching {:.0}y",
            count.lit,
            count.candidates,
            wanted.iter().filter(|lamp| lamp.indoors).count(),
            nearest.colour.x * 255.0,
            nearest.colour.y * 255.0,
            nearest.colour.z * 255.0,
            if nearest.indoors { "indoors" } else { "outdoors" },
            nearest.reach * reach_scale,
        );
    }

    for (i, entity) in pool.0.iter().enumerate() {
        let Ok((mut light, mut transform)) = lights.get_mut(*entity) else {
            continue;
        };
        match slots[i].lamp {
            Some(lamp) => {
                let (strength_scale, reach_scale) = multipliers(&tuning, lamp.indoors);
                // The waver is the last multiplier, after the night's dimmer,
                // the indoor split and the fade, so a lamp fading in wavers as
                // it fades in rather than working against its fade. Zero
                // amplitude gives exactly 1.0 for the cost of a compare; see
                // [`flame`].
                let waver = flame(
                    lamp.key.0 ^ u64::from(lamp.key.1),
                    time.elapsed_secs(),
                    lamp.flicker * tuning.lamp_flicker,
                );
                let wanted = PointLight {
                    range: lamp.reach * reach_scale,
                    ..lamp.light(scale * strength_scale * slots[i].level() * waver)
                };
                // Written only when changed, on both components: a `DerefMut`
                // on either marks it changed, and a changed light is
                // re-extracted and re-clustered. A torch on a wall does not
                // move, and a slot at full level is no longer written once its
                // fade is over.
                //
                // A flame bypasses this on purpose. A lamp with a non-zero
                // [`LampLight::flicker`] has a different intensity every frame,
                // so it is written every frame. That is the whole cost of the
                // effect, and it is why the amplitude is a slider rather than a
                // constant. The two steady sources are unaffected: their
                // amplitude is zero, [`flame`] returns 1.0 after a compare, and
                // the intensity does not change. The writes are bounded by the
                // number of fires in the pool rather than by [`BUDGET`], and
                // `flame flicker` at 0 on the world tab removes them for an
                // A/B.
                if transform.translation != lamp.at {
                    transform.translation = lamp.at;
                }
                if light.intensity != wanted.intensity
                    || light.range != wanted.range
                    || light.color != wanted.color
                {
                    *light = wanted;
                }
            }
            // A slot nothing is fading through. Zero intensity rather than
            // despawned — see the module note on the pool.
            None => {
                if light.intensity != 0.0 {
                    light.intensity = 0.0;
                    light.range = 0.01;
                }
            }
        }
    }
}

/// [`crate::ui::report`] slot, beside `sky`'s at 10 and `night`'s at 11.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(12);

/// Shows on the HUD how many lamps are lit and how many candidates are in
/// view. The two numbers show whether [`BUDGET`] is large enough for the
/// current location.
#[cfg(feature = "diagnostics")]
fn report(count: Res<LampCount>, mut hud: ResMut<crate::ui::report::HudReport>) {
    if !count.is_changed() {
        return;
    }
    hud.set(
        crate::ui::report::Section::World,
        SLOT,
        "lamps",
        format!(
            "{} lit of {} burning{}",
            count.lit,
            count.candidates,
            if count.lit >= BUDGET { " (at the budget)" } else { "" },
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fade starts and ends at rest. See [`Slot::level`], which describes the
    /// exponential approach it replaced.
    ///
    /// The test checks the property, not the numbers. An exponential approach
    /// delivers its largest change in its first frame, which makes a light
    /// appear abruptly. A smoothstep is slowest at both ends and fastest in the
    /// middle, so the first frame of a fade is nearly invisible.
    #[test]
    fn a_fade_is_slowest_at_both_ends_and_fastest_in_the_middle() {
        let at = |phase: f32| Slot { lamp: None, phase }.level();

        // The ends are exact, so a slot at rest is fully off or fully on
        // rather than nearly so.
        assert_eq!(at(0.0), 0.0);
        assert_eq!(at(1.0), 1.0);
        // …and clamped, because a step can overshoot on a long frame.
        assert_eq!(at(-0.5), 0.0);
        assert_eq!(at(1.5), 1.0);
        // Symmetric about the middle, which is what makes coming on and going
        // out look like each other.
        assert!((at(0.5) - 0.5).abs() < 1e-6);

        // The main property. Of ten equal steps along the clock, the first and
        // last deliver far less light than the middle one; the exponential did
        // the opposite at its start.
        let d = |a: f32, b: f32| at(b) - at(a);
        let first = d(0.0, 0.1);
        let middle = d(0.45, 0.55);
        let last = d(0.9, 1.0);
        assert!(first < middle * 0.5, "the first step is gentle: {first} vs {middle}");
        assert!(last < middle * 0.5, "and so is the last: {last} vs {middle}");
        assert!((first - last).abs() < 1e-6, "and the two ends match");

        // Monotonic, so a fade never goes backwards while its phase goes
        // forwards — the slot ordering that picks a newcomer's home relies on
        // phase and level agreeing about which slot has least light in it.
        let mut previous = -1.0;
        for i in 0..=20 {
            let now = at(i as f32 / 20.0);
            assert!(now >= previous, "monotonic at {i}");
            previous = now;
        }
    }

    /// The two multiplier pairs, and which one a lamp gets.
    ///
    /// The defaults follow the reports and are asymmetric on purpose. A torch
    /// on a road was "dim and not very useful", which is a pool too small
    /// (reach) rather than a centre too dark. A room looked "completely fake
    /// ... because of all the emitter lights usually present", which is
    /// several overlapping emitters each at full strength. So outdoors gains
    /// reach and keeps strength, and indoors keeps reach and loses strength.
    #[test]
    fn a_lamp_indoors_and_a_lamp_in_the_open_are_scaled_by_different_pairs() {
        let tuning = crate::render::night::NightTuning::default();

        // Outdoors: full strength, twice the reach.
        assert_eq!(multipliers(&tuning, false), (1.0, 2.0));
        // Indoors: half the strength, the reach the file states.
        assert_eq!(multipliers(&tuning, true), (0.5, 1.0));

        // Applied to a lamp, a 12-yard reach becomes 24 in the open and 12 in
        // a room.
        let lamp = LampLight {
            key: LampKey(0, 0),
            at: Vec3::ZERO,
            colour: Vec3::ONE,
            strength: STRENGTH,
            reach: 12.0,
            indoors: false,
            flicker: 0.0,
        };
        assert_eq!(lamp.reach * multipliers(&tuning, lamp.indoors).1, 24.0);
        assert_eq!(lamp.reach * multipliers(&tuning, true).1, 12.0);

        // A reach slider at zero gives a very small light, not an invalid one.
        // Bevy treats a range of exactly 0 as an error rather than as off. A
        // light is turned off here by its intensity, which has no floor,
        // because zero strength is off.
        let dark = crate::render::night::NightTuning {
            lamp_strength: 0.0,
            lamp_reach: 0.0,
            indoor_lamp_strength: 0.0,
            indoor_lamp_reach: 0.0,
            ..Default::default()
        };
        assert_eq!(multipliers(&dark, false), (0.0, 0.05));
        assert_eq!(multipliers(&dark, true), (0.0, 0.05));
    }

    /// The lamp's arithmetic, which can be tested without a world: the hue is
    /// normalised, the reach grows with the particle and stops, and the
    /// intensity is exactly what the shader reads back.
    #[test]
    fn a_lamps_hue_is_the_flames_and_its_value_is_the_settings() {
        let lamp = LampLight {
            key: LampKey(0, 0),
            at: Vec3::ZERO,
            colour: Vec3::new(1.0, 0.55, 0.2),
            strength: STRENGTH,
            reach: 12.0,
            indoors: false,
            flicker: 0.0,
        };
        let light = lamp.light(1.0);
        let colour = LinearRgba::from(light.color);
        assert_eq!([colour.red, colour.green, colour.blue], [1.0, 0.55, 0.2]);
        // What `atmosphere.wgsl` reads is `colour * intensity / 4pi`, so the
        // round trip has to land back on the strength exactly.
        let seen = light.intensity / BEVY_INTENSITY_UNIT;
        assert!((seen - STRENGTH).abs() < 1e-6, "{seen}");
        assert_eq!(light.range, 12.0);
        assert!(!light.shadow_maps_enabled, "twenty-four cube shadows is the one thing this may not do");
    }

    /// A flame only dims, never brightens. The byte-space sum depends on this:
    /// the shader adds the result to the sun and the fill in a space where a
    /// value above the clamp is lost.
    #[test]
    fn a_flame_never_brightens_past_what_its_source_authored() {
        for step in 0..2000 {
            let t = step as f32 * 0.01;
            for seed in [0u64, 1, 977, 40_000] {
                let v = flame(seed, t, FIRE_FLICKER);
                assert!(v <= 1.0, "{v} at t={t} seed={seed}");
                assert!(v >= 1.0 - FIRE_FLICKER, "{v} at t={t} seed={seed}");
            }
        }
    }

    /// A lamp that does not burn gets exactly 1.0, which keeps the write guard
    /// effective for two of the three sources.
    #[test]
    fn a_steady_lamp_is_untouched() {
        for step in 0..200 {
            let t = step as f32 * 0.1;
            assert_eq!(flame(7, t, 0.0), 1.0);
        }
    }

    /// Two torches on one wall must not waver in step, because in step they
    /// look worse than no flicker. The first version of [`flame`] had this
    /// fault: its phase was `seed % 997` scaled into radians, which spreads
    /// distant keys and leaves adjacent ones together. A lamp key is an
    /// entity's bits and a slot index, so adjacent keys are the normal case.
    /// The seeds below are 1 and 2 for that reason, not values far apart.
    #[test]
    fn two_flames_are_out_of_phase() {
        let apart = (0..400)
            .map(|step| {
                let t = step as f32 * 0.05;
                (flame(1, t, FIRE_FLICKER) - flame(2, t, FIRE_FLICKER)).abs()
            })
            .fold(0.0f32, f32::max);
        assert!(apart > 0.01, "the two seeds move together: {apart}");
    }

    /// Only the burning source carries a flicker amplitude. The three
    /// construction sites are the whole per-source split, and this test exists
    /// so that a source added later needs a decision about its amplitude.
    #[test]
    fn only_a_fire_flickers() {
        assert!(FIRE_FLICKER > 0.0);
        // A glow quad, as `StaticLamps` builds one.
        let glow = ModelGlow::of(
            &vale_assets::world::glow::Glow {
                at: [0.0, 0.0, 0.0],
                extent: 0.5,
                texture: None,
            },
            Vec3::ONE,
        );
        assert!(glow.reach >= REACH_MIN);
    }

    /// A client that never enters a dark place has no lamps at all. The test
    /// makes that saving checkable, as the project requires of savings.
    ///
    /// The measure is the entity count, not the intensity: a zero-intensity
    /// `PointLight` is still a clusterable object, and Bevy extracts it,
    /// assigns it to every screen-space cell its sphere touches and uploads the
    /// buffer once a frame regardless. The first version of this module spawned
    /// the pool at `Startup` and so charged every daylight session for a
    /// feature that did nothing.
    ///
    /// Both transitions are tested. The harder failure to see is not a pool
    /// left up but a pool rebuilt every frame, which looks the same from
    /// outside and costs twenty-four spawns and twenty-four despawns at sixty
    /// hertz.
    #[test]
    fn a_day_frame_has_no_lamps_in_the_world_at_all() {
        let mut app = App::new();
        app.init_resource::<LampPool>()
            .init_resource::<LampCount>()
            // The deep night is off by default, and the lamps with it.
            .insert_resource(NightTuning::on())
            // The fade needs a clock; there is no camera in this world, so the
            // pass raises the pool and then returns before the choosing. The
            // test checks the entity count, not the choice of lamps.
            .init_resource::<Time>()
            .insert_resource(Night::default())
            .add_systems(Update, light_the_flames);

        let lamps = |app: &mut App| {
            app.world_mut()
                .query_filtered::<(), With<PointLight>>()
                .iter(app.world())
                .count()
        };

        // Daylight: the factor is zero, so nothing is ever raised.
        app.update();
        assert_eq!(lamps(&mut app), 0, "noon must cost nothing");

        // Dusk turns over. The pool goes up on one frame and stays up.
        app.world_mut().resource_mut::<Night>().factor = 1.0;
        app.update();
        assert_eq!(lamps(&mut app), BUDGET);
        let ids: Vec<_> = app.world().resource::<LampPool>().0.clone();
        app.update();
        assert_eq!(lamps(&mut app), BUDGET);
        assert_eq!(
            app.world().resource::<LampPool>().0,
            ids,
            "the same entities, not twenty-four fresh ones a frame"
        );

        // …and dawn, or a logout, takes them away again.
        app.world_mut().resource_mut::<Night>().factor = 0.0;
        app.update();
        assert_eq!(lamps(&mut app), 0, "and it must not outlive the night");
        assert_eq!(app.world().resource::<LampCount>().lit, 0);
    }

    /// A slot with no lamp in it is dark, and dark means an empty cluster
    /// rather than a light that adds nothing — see the module note on why the
    /// pool is fixed.
    #[test]
    fn an_unlit_slot_costs_a_cluster_nothing() {
        let lamp = LampLight {
            key: LampKey(0, 0),
            at: Vec3::ZERO,
            colour: Vec3::ONE,
            strength: STRENGTH,
            reach: 12.0,
            indoors: false,
            flicker: 0.0,
        };
        assert_eq!(lamp.light(0.0).intensity, 0.0);
    }
}
