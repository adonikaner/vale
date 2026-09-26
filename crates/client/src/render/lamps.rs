//! **Things that burn light the world around them.**
//!
//! The second half of this branch's deviation, and the larger one: 1.12 has no
//! local lights at all. A torch, a brazier, a street lantern, a campfire and a
//! spell landing on the ground are all *drawn* by the reference — they are
//! additive particle quads over the top of the frame — and not one of them
//! changes the colour of a single texel beneath it. That does not show at the
//! table's own night, which lights every surface in the world to at least
//! 29/60/84; it is the first thing you notice once [`crate::render::night`]
//! takes that away.
//!
//! ## A lamp is an additive *emitter*, or an additive *batch*
//!
//! There is no list of model names here and there must never be one. A model
//! that glows says so in its own particle definition: `blend` 3 and 4 are the
//! two additive modes, which is what an emitter is authored as when its quads
//! are meant to *add* light to the frame rather than sit in front of it. Smoke,
//! dust, spray and every other alpha-blended puff is blend 2 and is not a lamp.
//!
//! That covers everything in the game that is **on fire**: a campfire, a
//! brazier, a forge, the flame in a hanging lantern, a missile in flight, the
//! effect standing on a unit that has just been cast at, and the torch a guard
//! is *carrying* — because an attachment's emitters are spawned the same way.
//! Every one of them is an `Emitter` in [`crate::render::particles`] and every
//! one already knows its own colour.
//!
//! **And it left every lamppost in the game dark**, which is the report this
//! module's second rule came out of. `vale model` on the Elwynn lamppost —
//! 24 of them on one Goldshire tile — says `no lights`, `no ribbon emitters`,
//! `0 particle emitters`, and then this:
//!
//! ```text
//! [3] tex GLOW32.BLP  blend 4 add-alpha  [unlit no-depth-write]  2 tris
//! ```
//!
//! A static light in 1.12 is **an additive unlit quad and nothing else** — no
//! light record, no emitter. So the second source is
//! [`vale_assets::world::glow`]: an additive `unlit` batch is a lamp, its
//! colour is its texture's own average, and its size is the quad's. That is
//! still a rule read off the data with no list of names in it, and between the
//! two of them a lit street and a lit campfire are the same subject.
//!
//! The two populations barely overlap: an **emitter** is what a fire is, a
//! **glow batch** is what a lamp is.
//!
//! **The colour is the flame's; the brightness is not.** A lamp's hue is the
//! brightest of its three over-life colour keys, normalised — so a fire is
//! orange and a mana effect is blue — and its *value* is the one setting on the
//! panel. The alternative is reading the emitter's authored brightness, which
//! varies across the game by more than an order of magnitude for reasons that
//! are about how a quad reads over a texture and not about how much light is in
//! the room.
//!
//! ## The budget is what makes it free
//!
//! Bevy clusters point lights on the CPU — a light is assigned to the
//! screen-space cells its sphere touches — and that work is linear in the
//! number of lights times the cells each covers. Stormwind at night has
//! hundreds of additive emitters in view, and all of them are candidates.
//!
//! So [`BUDGET`] of them are lit, nearest to the camera first, and the rest are
//! not. The pool is **fixed while it is up**: [`BUDGET`] entities, rewritten in
//! place every frame, with the unused tail at zero intensity. Nothing is
//! spawned or despawned from frame to frame and no entity ever changes
//! archetype, which is the difference between this and the obvious version that
//! inserts a `PointLight` on whichever emitters are nearest.
//!
//! **And in daylight the pool does not exist.** This is the half that had to be
//! fixed after the first version: twenty-four `PointLight` entities are
//! twenty-four *clusterable objects*, and Bevy extracts, assigns and uploads
//! them every frame whether their intensity is zero or not — so a client at
//! noon, or one run with `--night off`, was paying for a feature that was doing
//! nothing. The pool is raised when the night first asks for a lamp and dropped
//! when it stops, which happens at dusk, at dawn and at a logout rather than
//! per frame. What is left on a day frame is three resource reads and a
//! compare; see [`light_the_flames`], whose first branch is that whole cost.

use bevy::prelude::*;

use crate::render::night::{Night, NightTuning};
use crate::render::particles::Emitter;

/// How many lamps may be lit at once.
///
/// Twenty-four, which is a choice about the CPU's clustering pass rather than
/// about the fragment shader — see the module note. It is enough for a lit
/// street and not enough for a lit city, and the ones that go are the far ones,
/// which is the right way round: a lamp forty yards away contributes less than
/// a byte to any texel by the time [`REACH_MAX`] has fallen off.
pub const BUDGET: usize = 24;

/// How close two candidates have to be before the dimmer one is dropped, in
/// yards.
///
/// **A fire is usually three emitters**, not one: a flame, a glow and a shower
/// of embers, all additive and all standing at the same point. Without this,
/// one campfire spends three of the twenty-four slots on three copies of the
/// same light and a street loses two lamps at the far end for it.
const MERGE_RADIUS: f32 = 2.5;

/// The reach of the smallest lamp, in yards, before the panel's multiplier.
///
/// **All three of these came down after the first look, and the strength went
/// up to meet them.** A lamp reaching thirty yards at a falloff that is flat
/// near the source is not a lamp, it is a second ambient with a position: it
/// lit a whole clearing evenly and the picture lost its contrast. A bright core
/// over a short reach is what reads as a torch — the same light budget spent
/// where it can be seen against the dark rather than spread until it cannot.
const REACH_MIN: f32 = 8.0;

/// …and of the largest.
const REACH_MAX: f32 = 20.0;

/// How many yards of reach each yard of particle size is worth. A candle and a
/// bonfire are the same rule with different numbers in it.
const REACH_PER_SIZE: f32 = 2.0;

/// What a lamp adds to a surface it is standing on, in the light table's own
/// 0..1 band space, before the panel's multiplier and the night factor.
///
/// **Above one, and the clamp is what that means.** A lamp is a term of the
/// same sum the sun and the fill are terms of (see `atmosphere.wgsl`'s
/// `daylight_srgb_lamplit`), and that sum is clamped to 1 in the file's own
/// space — the game's own clamp, not a safety net (see `daylight_srgb`).
///
/// So a strength over 1 does not overflow anything: it saturates the **warm**
/// channel of a flame first and the others after, which is exactly what the
/// clamp exists to produce and exactly what a texel a yard from a torch should
/// do. What it buys is a core that is unmistakably a light source with the
/// flame's own colour around it, against a floor that falls off inside eight
/// yards. At 0.9 with a thirty-yard reach there was no core and no falloff —
/// just a large warm patch, which is the "washed out" half of the report.
const STRENGTH: f32 = 1.35;

/// **How much better a challenger has to be before it takes a lit lamp's
/// slot**, as a fraction.
///
/// This is the whole of the fix for "lights flicker on and off in towns". With
/// a hundred candidates and twenty-four slots, the pair sitting at ranks 24 and
/// 25 swap every time the camera moves a step — and a swap is a light going out
/// and another coming on. Boosting an incumbent's score by this before the sort
/// turns that jitter into a set that only changes when something has genuinely
/// become more important.
///
/// A quarter, which is wide: at 10% a walk down a street still churns, and the
/// cost of being generous is that a newly-relevant lamp waits a few yards. That
/// trade is the right way round — a lamp that arrives late is not noticeable
/// and a lamp that blinks is the report.
const INCUMBENT_MARGIN: f32 = 0.25;

/// How long a lamp takes to reach full strength, and to go out, in seconds,
/// when nothing on the panel says otherwise —
/// [`crate::render::night::NightTuning::lamp_fade`] is the live one.
///
/// The second half of the churn fix. Stickiness stops lamps *trading places*;
/// this stops the remaining changes from being **steps**.
///
/// **It was 0.25 and that was too short by more than the number suggests**,
/// because the old fade was also the wrong *shape* — see [`Slot::level`]. A
/// quarter of a second spent almost entirely in its first sixth is a pop with a
/// tail, which is what *"the light appears and then vanishes in a way that looks
/// buggy"* is. With a curve that starts and ends at rest, six tenths is gentle
/// at both ends and still inside the time it takes to walk past a lamppost.
const FADE_SECONDS: f32 = 0.6;

/// What Bevy divides a `PointLight`'s intensity by on its way to the shader.
///
/// `bevy_pbr`'s light extraction is `intensity / (4 * PI)` and the cluster
/// buffer then holds `colour * that`. Multiplying by it here means the number
/// the shader reads is exactly the number [`LampLight::strength`] states, with
/// no unit conversion hidden between the two — see [`LampLight::light`].
const BEVY_INTENSITY_UNIT: f32 = 4.0 * std::f32::consts::PI;

/// **One light a model carries, in its own space** — the static half of this
/// module, resolved once when the model loads and shared by every placement of
/// it. See [`vale_assets::world::glow`] for which batch this is and
/// `render::models::loader` for where the colour comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelGlow {
    /// Where it hangs, in the model's own space and **Bevy's axes** — converted
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
            // **The quad's own size decides the reach**, which is the only
            // thing in the file that says how much light this is meant to be: a
            // candle's glow is a few inches across and a forge's is yards. The
            // multiplier is large because the quad is small — a lamppost's is
            // under a yard — and the clamp is the emitters' own, so a street
            // lamp and a campfire land in the same range.
            reach: (REACH_MIN + GLOW_REACH_PER_YARD * glow.extent)
                .clamp(REACH_MIN, REACH_MAX),
        }
    }
}

/// How many yards of reach each yard of *glow quad* is worth. See
/// [`ModelGlow::of`] for why this is four times [`REACH_PER_SIZE`].
const GLOW_REACH_PER_YARD: f32 = 8.0;

/// **How far a flame's light dips when it wavers**, as a fraction.
///
/// A tenth: enough that a room lit by one torch is visibly alive and not enough
/// that anything reads as flashing. It is a **look and not a measurement**, and
/// says so — see the module note. 1.12 has no dynamic lamps at all, so there is
/// no reference behaviour to be faithful to here; the lamps are this client's
/// own and so is this.
///
/// Only the *burning* sources take it. See [`LampLight::flicker`].
const FIRE_FLICKER: f32 = 0.10;

/// …and what a lamp's brightness is multiplied by at time `t`.
///
/// **It only ever dips.** The wave is folded into `[0, 1]` and subtracted, so
/// the result is `[1 - amplitude, 1]` and a lamp is never brighter than the
/// strength its source authored — which matters because the shader adds this to
/// the sun and the fill in byte space, where an overshoot has nowhere to go.
///
/// Three sines rather than one: a single sine reads as a pulse, and three at
/// rates that do not divide each other read as a flame without needing noise,
/// a table or any state. `seed` is the lamp's own key, so two torches on one
/// wall do not waver in step — which is the failure that would make this look
/// worse than no flicker at all.
pub fn flame(seed: u64, t: f32, amplitude: f32) -> f32 {
    if amplitude <= 0.0 {
        return 1.0;
    }
    // **A real mix, not a modulo.** The first version was `seed % 997` scaled
    // into radians, which spreads *distant* keys and leaves adjacent ones on
    // top of each other — and lamp keys are adjacent by construction, since
    // they are an entity's bits and a slot index. Two torches on one wall came
    // out a thousandth of a radian apart, which is in step. `two_flames_are_
    // _out_of_phase` is the test that caught it.
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

/// **The average colour of a decoded texture, weighted by its own alpha.**
///
/// A glow texture is mostly transparent — `GLOW32.BLP` is 50% — and an
/// unweighted mean over it is half black, which drags every lamp in the game
/// toward grey and loses the warm orange that is the whole point. Weighting by
/// alpha asks the question that matters: *of the light this quad actually adds
/// to the frame, what colour is it?*
///
/// `None` for a texture with no opaque texel in it at all, which is a batch
/// that adds nothing and is not a lamp.
pub fn glow_colour(rgba: &[u8]) -> Option<Vec3> {
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

/// **Which lamp this is, across frames.**
///
/// Selection is now sticky and fading (see [`light_the_flames`]), and both need
/// to recognise a lamp they saw last frame. A position would nearly do — a
/// lamppost never moves — but a torch on a walking guard does, and a key that
/// changed as it walked would make exactly the flicker the stickiness exists to
/// remove. So it is the *holder's* entity plus, for a building with ten lights
/// in one component, which of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LampKey(u64, u16);

/// One thing that burns.
#[derive(Clone, Copy, Debug)]
pub struct LampLight {
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
    /// **Whether it is standing inside a building**, which picks which of
    /// [`crate::render::night::NightTuning`]'s two multiplier pairs it is
    /// scaled by.
    ///
    /// Filled during the gather in [`light_the_flames`] rather than at spawn,
    /// and never by the three sources themselves — a `MOLT` record really is
    /// indoors by construction, but a torch is a doodad that may be either and
    /// a carried one moves between them. One rule, measured the same way for
    /// all three, is cheaper to trust than three rules that agree by
    /// coincidence.
    ///
    /// **A building whose `Interior` has not been inserted yet reads as
    /// outdoors**, which is the brighter of the two answers and lasts as long
    /// as the WMO takes to stream. Stated rather than guarded because the
    /// alternative — holding a lamp dark until its building resolves — trades
    /// a couple of frames of a torch being too bright for a couple of frames of
    /// a room being unlit, and the second is the one that reads as a bug.
    pub indoors: bool,
    /// **How much this lamp wavers**, as a fraction its brightness dips by —
    /// 0 for a light that does not burn.
    ///
    /// The whole of the per-source split, and it is a field rather than a
    /// lookup because the three sources are already three separate
    /// construction sites and each of them knows what it is. A particle
    /// emitter that passed [`LampLight::of`]'s blend test **is a fire** — that
    /// is what the test is — so it gets [`FIRE_FLICKER`]. A model's glow quad
    /// and a building's `MOLT` record are not: a lamppost's pane, a forge's
    /// glow and a window lit from inside are steady by construction, and the
    /// module note already called those two "the ones that are not burning".
    ///
    /// So this does not add a distinction; it spends one the module already
    /// makes.
    pub flicker: f32,
}

/// **Which pair of multipliers a lamp is scaled by**, as `(strength, reach)`.
///
/// The whole of the indoor/outdoor split lives here, so that the three places
/// that need it cannot drift: the reach used to cull, the reach used to rank,
/// and the reach and strength written into the light. See
/// [`crate::render::night::NightTuning::indoor_lamp_strength`], which carries
/// the argument for splitting per lamp rather than per camera.
///
/// The reach is floored so that a slider at zero is a very small light rather
/// than a sphere of radius zero, which Bevy treats as an error rather than as
/// "off"; strength at zero is genuinely off and needs no floor.
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
    /// **What this emitter would light the world with**, or `None` when it is
    /// not the sort of emitter that glows.
    ///
    /// The rule is one line and the module note is the argument for it: an
    /// additive blend is what a glowing emitter is authored as.
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
            // **The one source that burns.** Reaching here means the blend and
            // colour tests above passed, which is what makes this a flame
            // rather than a distortion quad.
            flicker: FIRE_FLICKER,
            at: emitter.origin(),
            // Normalised: the hue is the flame's and the value is the setting's.
            colour: best / best.max_element().max(1e-4),
            // **Scaled by what the emitter is actually emitting**, which is the
            // one thing in this conversion that is not read off the definition
            // — see [`Emitter::output`], which carries the argument. Without it
            // the light is a box function over the entity's lifetime and the
            // fade at each end is the only shape it has.
            strength: STRENGTH * emitter.output(),
            reach: (REACH_MIN + REACH_PER_SIZE * size).clamp(REACH_MIN, REACH_MAX),
        })
    }

    /// …as the `PointLight` Bevy will cluster.
    ///
    /// **The colour is a `Color::LinearRgba` container and not a claim**, which
    /// is the same device `render::sky` uses for the sun and the fill and for
    /// the same reason: `bevy_pbr` uploads `LinearRgba::from(colour) *
    /// intensity` with no transfer function anywhere in it, so what the shader
    /// reads is the byte-space value written here — and the shader adds it to
    /// the sun and the fill in that space, where a sum survives.
    fn light(&self, scale: f32) -> PointLight {
        PointLight {
            color: Color::LinearRgba(LinearRgba::from_vec3(self.colour)),
            intensity: self.strength * scale * BEVY_INTENSITY_UNIT,
            range: self.reach,
            // A point rather than a sphere: the falloff in `atmosphere.wgsl` is
            // this client's own and reads neither.
            radius: 0.0,
            // **Never.** A shadow-casting point light is six depth passes over
            // everything in its range, and twenty-four of them is the most
            // expensive thing this renderer could be asked to do. The game
            // casts no runtime shadows at all; see `sky::sun_shadows`.
            shadow_maps_enabled: false,
            ..default()
        }
    }
}

/// **The lights one placement of a model stands there holding**, in world
/// space, already scaled and rotated.
///
/// On the placement's own entity rather than as child entities, and computed
/// once at spawn rather than every frame, because **static scenery never
/// moves**: a lamppost's glow is at the same three numbers for as long as the
/// tile is loaded. A child entity per lamp would be a transform to propagate
/// and an archetype to walk for something that cannot change.
#[derive(Component, Clone, Debug)]
pub struct StaticLamps(pub Vec<LampLight>);

impl StaticLamps {
    /// **A building's own lights**, placed by one instance of it — `MOLT`, and
    /// the third of the three sources.
    ///
    /// The two above are readings of a *drawing*: an emitter's colour is the
    /// flame's, a glow batch's is its texture's. This one is the game stating a
    /// light outright — colour, position, and a falloff in yards — and it is
    /// the only source that does. 1.12 reads none of it: a WMO is lit by its
    /// baked `MOCV` and these records go unused, which is why a lit window
    /// lights nothing in the reference client either.
    ///
    /// **The falloff is the file's and not this module's.** `atten_end` is
    /// where the building says the light stops, so it is taken as the reach
    /// rather than clamped into [`REACH_MIN`]..[`REACH_MAX`] — those bounds
    /// exist to keep a *guess* in a sane range and there is nothing to guess
    /// here. An inn's are 7 to 9 yards, which is a room.
    pub fn of_building(
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
                // **Steady.** A `MOLT` record is the room's own lighting as the
                // building's author placed it, not a fire standing in it.
                flicker: 0.0,
                at: placement.transform_point(crate::render::axes::to_bevy(l.position)),
                colour: {
                    let c = Vec3::from_array(l.colour);
                    c / c.max_element().max(1e-4)
                },
                // **The file's own intensity, times ours.** Every record
                // measured states 1.0, so this is a hook rather than a
                // variation — but a building that says its hearth is brighter
                // than its wall sconces should be believed.
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
                    // **Steady too.** A glow quad is a pane, a forge mouth or a
                    // lit window — painted onto the model rather than burning
                    // on it. The handful that really are flames carry a
                    // particle emitter as well, and that copy flickers.
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

/// The pool. See the module note on why it never grows or shrinks **while the
/// night is on**, and [`light_the_flames`] for why it does not exist at all
/// while it is off.
#[derive(Resource, Default)]
struct LampPool(Vec<Entity>);

pub struct LampPlugin;

impl Plugin for LampPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LampPool>()
            .init_resource::<LampCount>()
            .add_systems(
                Update,
                // **After the emitters have been stepped**, which is what
                // settles `Emitter::origin` — a frame-late read hangs every
                // lamp a stride behind the torch carrying it, which is the
                // same ordering `merge_fields` states on itself and for the
                // same reason.
                light_the_flames.after(crate::render::particles::simulate),
            );
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report.run_if(crate::ui::report::watched));
    }
}

/// [`BUDGET`] lights, dark, the first time the night asks for any.
///
/// **Not at `Startup`, which is where this was and which was a standing cost
/// for a feature that was off.** Twenty-four `PointLight` entities are twenty-
/// four clusterable objects: Bevy extracts them, assigns each to the
/// screen-space cells its sphere touches and uploads the buffer, every frame,
/// whether their intensity is zero or not. A client at noon — or with
/// `--night off`, or one that has never seen a dark place — was paying that for
/// nothing.
///
/// The pool is still fixed *while it is up*: the churn this avoids is
/// per-frame, and building or dropping it happens at dusk, at dawn and at a
/// logout. See [`light_the_flames`], which owns both edges.
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

/// Choose this frame's lamps and write them into the pool.
/// **One of [`BUDGET`] slots**: which lamp is in it, and how far it has come up.
///
/// The slot is what makes the set *stable*. A lamp keeps the slot it was given
/// for as long as it is wanted, so nothing moves because the sort order moved;
/// and a lamp that stops being wanted keeps it while it fades out, so nothing
/// is ever switched off in one frame. See [`INCUMBENT_MARGIN`] and
/// [`FADE_SECONDS`], which are the two halves of it.
#[derive(Clone, Copy)]
struct Slot {
    lamp: Option<LampLight>,
    /// **How far through its fade the slot is**, 0 dark and 1 full, advancing
    /// at a constant rate — see [`Slot::level`], which is the value actually
    /// written and is this shaped.
    phase: f32,
}

impl Slot {
    /// **What fraction of its light this slot is delivering**, which is
    /// [`Self::phase`] through a smoothstep.
    ///
    /// The shape is the point. The first version of this was an exponential
    /// approach —
    ///
    /// ```text
    /// level += (target - level) * (dt / FADE_SECONDS)
    /// ```
    ///
    /// — which is smooth in the sense that it never jumps, and is **fastest at
    /// the instant it starts**: a third of the whole change lands in the first
    /// tenth of a second and the rest trails off over the next second. Coming
    /// *on* that reads as a pop with a tail, and going off it reads as a light
    /// that is already gone before it has finished leaving. Against a spell
    /// effect, which is a light that arrives and departs within a second or so,
    /// it is the whole of the appearance — the report was that casting looks
    /// "buggy or artificial", and this is why.
    ///
    /// `3t² - 2t³` is at **rest at both ends**: zero rate at 0 and at 1, fastest
    /// in the middle. Nothing starts or stops abruptly because nothing starts or
    /// stops at speed. It costs two multiplies a slot a frame, over at most
    /// [`BUDGET`] of them.
    ///
    /// Kept separate from `phase` rather than shaping in place because the
    /// shaping is not invertible: a slot interrupted mid-fade carries on from
    /// where it *is*, and that has to be a position on the linear clock or the
    /// carry-over is not continuous.
    fn level(self) -> f32 {
        let t = self.phase.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

/// Choose this frame's lamps and write them into the pool.
///
/// ## Why the choosing is four rules and not a sort
///
/// The first version was one: rank every candidate by distance to the camera
/// and take the nearest [`BUDGET`]. Every fault a player reported came out of
/// that one line.
///
/// * **It ranked lights that cannot be seen.** A lamp behind the camera
///   competed for a slot with one in front — and Bevy then assigned it to no
///   cluster, so the slot was spent and lit nothing. In a town that is roughly
///   half the field, which is where `24 of 130 burning` while looking at
///   nothing came from. So: **cull against the view frustum first**, sphere
///   against six planes, before anything is ranked.
/// * **It ranked by distance rather than by contribution.** A twenty-yard lamp
///   post at twenty-five yards matters and an eight-yard campfire at twenty does
///   not, and distance alone cannot tell them apart. So the score is the light
///   a lamp can actually deliver: `strength * reach^2 / distance^2`.
/// * **Nothing was sticky**, so the two candidates either side of the cut swapped
///   every time the camera moved and the pair blinked. See
///   [`INCUMBENT_MARGIN`].
/// * **Nothing faded**, so every change that did survive was a step. See
///   [`FADE_SECONDS`].
///
/// The four compose: culling makes the field small enough that the score means
/// something, the score makes the order meaningful, the margin keeps the order
/// from mattering frame to frame, and the fade covers what is left.
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
    // **`VALE_NO_LAMPS=1` is a measurement probe**, not a setting: with no
    // lamps spawned the shader's cluster list is empty, so an A/B against the
    // default splits the night's fragment cost into its two halves — the
    // 24-lamp `lamplight()` loop against `night_air()`'s per-fragment
    // exponentials — with no shader change and no second binary. It exists
    // because the 3440x1440 round measured the night at ~3.5 ms of opaque-pass
    // GPU and the two halves want different fixes.
    // **The night's own dimmer only**, with each lamp's own strength
    // multiplier applied later — see [`multipliers`]. It used to carry
    // `tuning.lamp_strength` as well, which was correct while there was one
    // pair for the whole world and is the thing that had to move once there
    // were two.
    let scale = if tuning.enabled && tuning.lamps && std::env::var("VALE_NO_LAMPS").is_err() {
        night.factor
    } else {
        0.0
    };
    // **The whole of what this pass costs when the night is off**, which is
    // every frame of every daylight session and every frame of every session
    // run with `--night off`: three resource reads and a compare. Nothing is
    // queried, nothing is allocated, and — the part that is not obvious — there
    // are no lights in the world for Bevy to extract, cluster and upload
    // either. See [`raise_pool`].
    if scale <= 0.0 {
        if !pool.0.is_empty() {
            for entity in pool.0.drain(..) {
                commands.entity(entity).despawn();
            }
            slots.clear();
            count.lit = 0;
            count.candidates = 0;
            // …and the camera goes back to the one-cluster view the world was
            // spawned with, which is the cheap and correct shape for a world
            // holding zero clusterable lights. See [`raise_pool`]'s half of
            // this and `world::camera`, which owns the daylight default.
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
        // **The cluster grid comes up with the lamps, and only with them.**
        // `world::camera` spawns the view as `ClusterConfig::Single` because a
        // world with no point lights has nothing to bin — but under one
        // cluster **every fragment on screen walks every burning lamp**, and
        // at 3440x1440 that priced the deep night at ~3.5 ms of opaque-pass
        // GPU: measured 2026-08-27, opaque 5.1–5.3 ms against 1.63–1.68 with
        // `--night off`, and 1.93–2.04 with the pool suppressed
        // (`VALE_NO_LAMPS=1`), so the 24-lamp loop was ~1.5–3.2 ms of it
        // and `night_air` ~0.3. The default grid puts a fragment's loop at
        // the 0–3 lamps its froxel touches instead.
        //
        // Swapped here rather than fixed at spawn because the *daylight* half
        // of the old decision still holds: the CPU binning is real work, and a
        // noon session with zero lights should not pay it. Both edges write
        // through commands beside the pool they belong to, so the config and
        // the population cannot disagree for more than the frame the commands
        // take.
        for (camera, ..) in &camera {
            commands
                .entity(camera)
                .insert(bevy::light::cluster::ClusterConfig::default());
        }
        // The pool is spawned by a command and does not exist until it is
        // applied, so there is nothing to write into this frame. One frame of
        // an unlit torch at the moment dusk turns over.
        return;
    }
    if slots.len() != pool.0.len() {
        slots.resize(pool.0.len(), Slot { lamp: None, phase: 0.0 });
    }

    let Some((_, eye, frustum)) = camera.iter().next() else {
        return;
    };
    let eye = eye.translation;

    // **Gathered and culled in one pass.** A lamp whose sphere does not touch
    // the view frustum cannot light a visible fragment, so it is not a
    // candidate at all — it is not scored, not sorted, and not counted as one
    // of the `N burning` the report shows, which is the number that used to be
    // a hundred while nothing was on screen.
    chosen.clear();
    let visible = |chosen: &mut Vec<(f32, LampLight)>, mut lamp: LampLight| {
        // **Classified before it is culled, because the cull radius depends on
        // the answer.** The test is `Interior::holds`, the same one
        // `world::entities::light_entities` makes for every unit in the world,
        // and it rejects on the building's whole bounding box before it looks
        // at a single room — so the cost over a few dozen loaded buildings is
        // an AABB test each and it is made once per candidate per frame.
        //
        // Doing it here rather than after the cull is deliberate: an indoor
        // lamp culled against the *outdoor* reach would merely be
        // over-inclusive, but it would also be **ranked** by that reach, and
        // the ranking is what decides which twenty-four of the candidates are
        // lit at all.
        //
        // **No vertical probe offset, unlike `light_entities`' version of this
        // test.** That one lifts its point a yard because a character's
        // position is at their *feet* and a room box's top face is the outside
        // of the ceiling, so somebody standing on a flat roof would read as
        // inside. A lamp's position is the emitter's own origin — a flame on a
        // brazier, a glow partway up a lamppost, a `MOLT` record hanging in a
        // room — and none of those sits on a floor or a roof face, so an
        // offset here would only push a low light through a ceiling.
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
        // **The light it can deliver**, not how near it is. The inverse square
        // is the shape of a falloff rather than this client's own — see
        // `atmosphere.wgsl`, whose falloff is bounded — but for *ranking* it is
        // the right question: twice as far is a quarter as much.
        let score = lamp.strength * reach * reach / lamp.at.distance_squared(eye).max(1.0);
        chosen.push((score, lamp));
    };
    for (entity, emitter) in &emitters {
        if let Some(lamp) = LampLight::of(emitter) {
            visible(&mut chosen, LampLight { key: LampKey(entity.to_bits(), 0), ..lamp });
        }
    }
    // …and the ones that are not burning: a lamppost's glow quad and a
    // building's own `MOLT` records, both resolved at spawn and standing still
    // ever since. See [`StaticLamps`], and the module note for why these are
    // separate rules rather than the same one.
    for (entity, lamps) in &standing {
        for lamp in &lamps.0 {
            visible(
                &mut chosen,
                LampLight { key: LampKey(entity.to_bits(), lamp.key.1), ..*lamp },
            );
        }
    }
    count.candidates = chosen.len();

    // **The incumbents get their thumb on the scale**, which is the whole of
    // the anti-flicker rule: a lamp that is already lit is worth
    // `1 + INCUMBENT_MARGIN` of what it measures, so a challenger has to be
    // that much better before the two trade places.
    for (score, lamp) in chosen.iter_mut() {
        if slots.iter().any(|s| s.phase > 0.0 && s.lamp.is_some_and(|l| l.key == lamp.key)) {
            *score *= 1.0 + INCUMBENT_MARGIN;
        }
    }
    chosen.sort_by(|a, b| b.0.total_cmp(&a.0));

    // Best first, dropping any candidate standing on top of one already taken —
    // see [`MERGE_RADIUS`], which is about a campfire being three emitters
    // rather than about tidiness.
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

    // **Keep every wanted lamp in the slot it already had.** This is what makes
    // the fade mean anything: a lamp that moved slot would fade out of one and
    // into another, which is a flicker with extra steps.
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
    // …and put the newcomers wherever the least light is being lost: a dark
    // slot first, then the one furthest through its fade-out.
    for lamp in homeless {
        let Some(i) = (0..slots.len())
            .filter(|i| !held[*i])
            .min_by(|a, b| slots[*a].phase.total_cmp(&slots[*b].phase))
        else {
            break;
        };
        // A slot still fading a *different* lamp out is taken over rather than
        // waited for — its level carries on from where it is, so the swap is a
        // dip rather than a step.
        slots[i].lamp = Some(lamp);
        held[i] = true;
    }

    // **A constant rate along the clock**, not a fraction of what is left —
    // the shaping is [`Slot::level`]'s and doing it here as well would compound
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

    // **A new high-water mark, not the first frame.** The first draft logged
    // once, the first time anything was lit — and that frame is *before the
    // doodads have streamed in*, so a scripted run at a crossroads with two
    // dozen lampposts on it reported the three campfires it could already see
    // and never mentioned the lamps at all. An instrument that answers early is
    // worse than one that does not answer.
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
                // **The waver, and it is the last multiplier on the chain** —
                // after the night's dimmer, the indoor split and the fade, so
                // a lamp coming up wavers *as* it comes up rather than fighting
                // its own fade. Zero amplitude is exactly 1.0 and costs a
                // compare; see [`flame`].
                let waver = flame(
                    lamp.key.0 ^ u64::from(lamp.key.1),
                    time.elapsed_secs(),
                    lamp.flicker * tuning.lamp_flicker,
                );
                let wanted = PointLight {
                    range: lamp.reach * reach_scale,
                    ..lamp.light(scale * strength_scale * slots[i].level() * waver)
                };
                // **Written only when it moved**, on both components: a
                // `DerefMut` on either marks it changed, and a changed light
                // is a re-extraction and a re-cluster. A torch on a wall does
                // not move, and a slot at full level stops being written at
                // all once its fade is over.
                //
                // **A flame defeats that, and deliberately.** A lamp with a
                // non-zero [`LampLight::flicker`] has a different intensity
                // every frame, so it takes the write every frame — which is the
                // whole cost of the effect and is why the amplitude is a
                // *slider* rather than a constant. The two steady sources keep
                // the old behaviour exactly: their amplitude is zero, [`flame`]
                // returns 1.0 on a compare, and the intensity does not move. So
                // the churn is bounded by the number of *fires* in the pool
                // rather than by [`BUDGET`], and `flame flicker` at 0 on the
                // world tab restores the old frame for an A/B.
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

/// **How many things are burning, and how many of them are lighting anything**
/// — the two numbers the budget is about, and the pair that says whether
/// [`BUDGET`] is the right size for the place you are standing in.
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

    /// **The lamp's own arithmetic**, which is the half of this module that can
    /// be tested without a world: the hue is normalised, the reach grows with
    /// the particle and stops, and the intensity is exactly what the shader
    /// reads back.
    /// **The two pairs, and which one a lamp gets.**
    ///
    /// The defaults are the reported ones and they are asymmetric on purpose:
    /// a torch on a road was *"dim and not very useful"*, which is its pool
    /// being too small rather than its centre too dark — reach — and a room was
    /// *"completely fake ... because of all the emitter lights usually
    /// present"*, which is several overlapping emitters each at full strength.
    /// So outdoors gains reach and keeps strength, and indoors keeps reach and
    /// loses strength.
    /// **A fade starts and ends at rest**, which is the whole of the shape
    /// change — see [`Slot::level`], where the exponential it replaced is.
    ///
    /// The property, not the numbers: an exponential approach delivers its
    /// largest change in its first frame, and that is what a light "appearing
    /// abruptly" is. A smoothstep is slowest at both ends and fastest in the
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

        // **The property that matters.** Ten equal steps along the clock: the
        // first and last deliver far less light than the middle one, which is
        // the opposite of what the exponential did at its start.
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

    #[test]
    fn a_lamp_indoors_and_a_lamp_in_the_open_are_scaled_by_different_pairs() {
        let tuning = crate::render::night::NightTuning::default();

        // Outdoors: full strength, twice the reach.
        assert_eq!(multipliers(&tuning, false), (1.0, 2.0));
        // Indoors: half the strength, the reach the file states.
        assert_eq!(multipliers(&tuning, true), (0.5, 1.0));

        // …and they really are the two ends of one lever, so a lamp's own
        // 12-yard reach comes out at 24 in the open and 12 in a room.
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

        // **A reach slider at zero is a very small light, not a broken one.**
        // Bevy treats a range of exactly 0 as an error rather than as "off",
        // and the way a light is turned off here is its intensity — which has
        // no floor, because zero strength is genuinely off.
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

    /// **A flame only ever dips**, which is the property the byte-space sum
    /// depends on: the shader adds this to the sun and the fill in a space
    /// where an overshoot has nowhere to go.
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

    /// …and a lamp that does not burn is left exactly alone, which is what
    /// keeps the write guard doing its job for two of the three sources.
    #[test]
    fn a_steady_lamp_is_untouched() {
        for step in 0..200 {
            let t = step as f32 * 0.1;
            assert_eq!(flame(7, t, 0.0), 1.0);
        }
    }

    /// **Two torches on one wall must not waver in step**, which is the failure
    /// that would read worse than no flicker at all — and which the first
    /// version of [`flame`] had. Its phase was `seed % 997` scaled into
    /// radians, which spreads distant keys and leaves adjacent ones on top of
    /// each other; a lamp key is an entity's bits and a slot index, so
    /// adjacent is the normal case. The seeds below are 1 and 2 for that
    /// reason rather than something comfortably far apart.
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

    /// **Only the burning source carries an amplitude.** The three
    /// construction sites are the whole of the per-source split, and this is
    /// what stops a later one being added without a decision about it.
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

    /// **A client that never sees a dark place has no lamps in it at all** —
    /// the saving, made checkable, which is this project's rule about savings.
    ///
    /// The number that matters is not the intensity, it is the **entity count**:
    /// a zero-intensity `PointLight` is still a clusterable object, and Bevy
    /// extracts it, assigns it to every screen-space cell its sphere touches and
    /// uploads the buffer once a frame regardless. The first version of this
    /// module spawned the pool at `Startup` and therefore charged every daylight
    /// session for a feature that was doing nothing.
    ///
    /// Both edges are here, because the interesting failure is not the one that
    /// leaves the pool up — it is the one that rebuilds it every frame, which
    /// looks identical from the outside and costs twenty-four spawns and
    /// twenty-four despawns at sixty hertz.
    #[test]
    fn a_day_frame_has_no_lamps_in_the_world_at_all() {
        let mut app = App::new();
        app.init_resource::<LampPool>()
            .init_resource::<LampCount>()
            .init_resource::<NightTuning>()
            // The fade needs a clock; there is no camera in this world, so the
            // pass raises the pool and then returns before the choosing. That
            // is exactly what is under test: the *entity count*, not the pick.
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
