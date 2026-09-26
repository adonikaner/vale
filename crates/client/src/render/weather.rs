//! **The rain, the snow and the sand** — `SMSG_WEATHER` as a picture, and the
//! first thing drawn in this client's sky that the server *says* rather than
//! the tables state.
//!
//! ## What the reference draws, and which half of that is read
//!
//! The reference's weather is three
//! emitters and one state machine, and the state machine is transcribed in
//! [`vale_protocol::play::weather`]: a grade in 0..1, a ramp at ten seconds
//! per unit of change, and a **density** of `max(0, (grade - 0.25) * 4/3)`
//! handed to whichever emitter the type names. The emitters are:
//!
//! ```text
//! rain   textures\Weather\RainDrop01.blp     16x128   Shaders\Vertex\rain.bls
//!        textures\Weather\SnowMist01.blp     256x256  (the mist under it)
//!        textures\Weather\RainDropSplash01.blp 256x256 (the splash, unread)
//! snow   textures\Weather\SnowFlake01.blp    64x64    Shaders\Vertex\snowpoint.bls
//!        textures\Weather\SnowMist01.blp              (the same mist)
//! sand   textures\Weather\SnowMist01.blp              Shaders\Vertex\sand.bls
//! ```
//!
//! and all three are built in the same box: a **44 x 44 x 25
//! yard** box around the camera, 128 mist particles, with per-type sizes —
//! rain's mist 1.2..5.0 yards, snow's 3.0..9.0 — falling at `-pi/2` with a 20°
//! spread. The drop and flake counts scale with
//! the density: `6500 * 0.66 * density` drops
//! and `1300 * 0.66 * density` flakes on the ordinary path (`35000` and
//! `14000` on the other branch of a byte this project has not identified, and
//! is not using).
//!
//! **What the three shaders say.** They are the renderer's specification and
//! they were read first:
//!
//! * `rain.bls` extrudes each drop along its own fall direction into a
//!   **streak** — `MAD R0.xyz, normal, length, position` — billboarded about
//!   that axis toward the eye (the cross products at `MUL R1.xyz, R2.zxyz,
//!   R3.yzxy`), fades it in and out over a life window carried in `texcoord`,
//!   and writes `fogcoord` from its distance to the camera, so **rain is
//!   fogged**.
//! * `snowpoint.bls` and `sand.bls` draw **point sprites** whose
//!   `result.pointsize` falls with distance (`MAD R0.z, dist, c24.x, c24.y`),
//!   fading over the same life window; sand is tinted by a colour uniform
//!   (`MUL result.color, c0[0], …`) and fades over the first fifth of its
//!   life (`c10.y` = 0.2).
//!
//! Bevy has no point sprites, so a flake and a grain are camera-facing quads
//! here, and a drop is the streak the rain shader builds — the same geometry
//! the reference's vertex programs evaluate, evaluated in a vertex program
//! of this client's own, `shaders/weather.wgsl`.
//!
//! **A point sprite is a size in pixels, and that is the whole difference
//! between a snowflake and a dinner plate.** `result.pointsize` is screen
//! space: a flake a yard from the eye and one thirty yards off are drawn at the
//! same number of pixels, less the linear falloff the `MAD` applies. A quad
//! whose size is a constant in *yards* is the opposite — it grows without limit
//! as it approaches the camera, and a 0.2-yard flake at half a yard covers a
//! quarter of the screen. So the sprite layers size their quads *at* the
//! distance they are being drawn at, which is the identity the shader states
//! rather than an approximation of it. Only the mist keeps a size in yards,
//! because the reference's own emitter states one for it.
//!
//! **A particle is anchored to the world and not to the camera.** The box is
//! centred on the eye, but that is where drops are *spawned*: `snowpoint.bls`
//! computes `position = v16 + normal * age` from a vertex attribute fixed at
//! spawn, and the camera enters only as the `-c4.xyz` that makes the maths
//! camera-relative. Carrying the pool camera-relative *without* undoing the
//! camera's own movement is a different picture entirely — the whole field
//! slides with the eye, so the rain reads as glued to the screen and every
//! camera orbit sweeps it across the world. Here a particle's position is a
//! world position — `seed + velocity × pace × s + W(t)`, see below — and what
//! is drawn is that position relative to the eye, wrapped into the box.
//!
//! **What is a reading rather than a measurement**, stated: the mist sizes,
//! box and count are the reference's, and so are the streak's two yards of
//! fall and the *form* of the point-size curve. The fall speed, the pixel
//! sizes the curve is scaled by, the streak's width and the near fade are
//! chosen — see [`Layer::for_kind`], where each number is labelled. A wrong one
//! renders as plausible weather rather than failing, which is why they are
//! named there and nowhere else.
//!
//! ## What is the reference's and what is this client's, in one place
//!
//! The box, the mist, the density curve, the ramp, the point-sprite form and
//! the streak-length rule are the reference's. Four
//! things here are **this client's own, by request**, the way `render::night`
//! is — asked for from the window, labelled, and sized so a clear sky is the
//! identity:
//!
//! * **The cap counts.** The reference's ordinary branch is 4,290 drops and
//!   858 flakes; "more snowfall particles" — asked four times — runs 2x the
//!   drops and 9x the flakes, still inside the client's own other branch
//!   (35,000 / 14,000 before the 0.66 scale). The asymmetry is the GPU's: a
//!   streak is two orders of magnitude more pixels than a flake, so the
//!   intensity lives in the flakes and the rain stays inside the fill budget.
//! * **The wind and the pace grow with the density** — [`Layer::wind_with_density`]
//!   and [`Layer::pace_with_density`], per layer: a light snow drifts down and
//!   a blizzard drives sideways. The reference's wind is a constant spread.
//! * **A streak is never thinner than a pixel and a half** — `weather.wgsl`'s
//!   `STREAK_MIN_HALF_PIXELS`, the streak's own form of the point sprite's
//!   `MAX result.pointsize, 1`: a centimetre of width is under a pixel from
//!   ten yards, and a sub-pixel streak rasterises to nothing on most frames,
//!   which is what "the rain does not look like much" was.
//! * **The far field** — [`FAR_BOX_HALF_XY`]: a pool of small flakes in a
//!   120 x 120 x 80 box, because the reference's near box leaves a snowy
//!   vista fifty yards out with nothing falling in it.
//! * **The visibility drop** — [`VISIBILITY_TAKEN`] and the deeper
//!   [`VISIBILITY_TAKEN_SNOW`], applied in `sky::resolve`: falling weather
//!   closes the fog with the density, over and above the storm row's own
//!   distance, and a cap blizzard leaves eighty-odd yards of world.
//! * **The shorter streak** — [`Layer::streak_fall`] against the measured
//!   [`STREAK_FALL_YARDS`].
//!
//! …and one thing that is the reference's behaviour restored rather than a
//! choice: **an indoor camera gets no particles** ([`WeatherState::indoors`]),
//! which is what kept it snowing on the Great Forge.
//!
//! ## Where the particles are computed, and what it costs
//!
//! **On the GPU, from a mesh built once per weather.** A pool is one quad per
//! particle at the layer's *full* count, with the particle's velocity, life,
//! phase, size and index in its vertex attributes and nothing per frame in
//! the mesh at all. `weather.wgsl`'s vertex stage places each corner from five
//! vec4s of parameters ([`WeatherParams`]) and the view:
//!
//! * a particle's world position is `seed + velocity × pace × s + W(t)`,
//!   where `seed` is hashed from its index and the *cycle* of its life it is
//!   in (so every rebirth lands anywhere in the box, which is the reference's
//!   respawn), `s` the seconds since that birth, `pace` the density's fall
//!   multiplier and `W(t)` the running integral of the shared wind, kept on
//!   the CPU as an accumulator and handed over wrapped to the box — exact,
//!   since the box is a torus;
//! * what is drawn is that position relative to the eye, wrapped into the box
//!   on every face, which is what makes the field stand still in the world
//!   while the camera moves through it;
//! * the density ramp is an `alive` count: a particle whose index is past it
//!   collapses to a point far under the camera, and so does one that is
//!   behind the eye by more than its own reach, inside its fade, a mist
//!   sheet the camera stands in, or under a roof.
//!
//! **What stays on the CPU** is small and mostly per second rather than per
//! frame: the ramp, the `alive` counts, the wind's integral, and the indoor
//! test — a quarter of a pool a frame against the loaded buildings' rooms,
//! evaluated at the same position the shader draws (the generator is shared
//! bit for bit) and written as one bit per particle into a storage buffer the
//! shader reads. Three material writes a frame, three transforms, and that
//! is all. It used to be three meshes simulated and rebuilt on the CPU every
//! frame — ten thousand quads' positions and colours refilled, then cloned
//! into the render world and re-uploaded — which measured at about a
//! millisecond of CPU whatever the culls saved, and is why the far field was
//! once trimmed; nothing here scales with the count any more.
//!
//! **One reading in the port, stated.** The fall since a particle's birth is
//! `velocity × pace_now × s` rather than the integral of a pace that moved
//! during the life; the pace only moves over a ten-second ramp and a life is
//! under eight seconds, so the difference is a drift of a few hundredths of
//! a yard a frame while the weather is thickening and nothing at all once it
//! has.
//!
//! The fill-rate half of the cost is unchanged and is why the mist runs at
//! half the reference's 128 with a longer near fade, and why the drops stayed
//! at 2x while the flakes went to 8x — a streak is two orders of magnitude
//! more pixels than a flake. `--without weather` subtracts all of it
//! (`tuning::SWITCHES`), and the world tab reports what is falling.

use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::DVec3;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use vale_protocol::play::weather::{Weather, WeatherKind};

use super::models::{ModelCache, TextureState};

const RAIN_DROP: &str = r"textures\Weather\RainDrop01.blp";
const SNOW_FLAKE: &str = r"textures\Weather\SnowFlake01.blp";
const MIST: &str = r"textures\Weather\SnowMist01.blp";

/// The emitter box the reference builds every layer in — `44.0, 44.0, 25.0`
/// for each of the three. Half-extents here, centred on the camera.
const BOX_HALF_XY: f32 = 22.0;
const BOX_HALF_Z: f32 = 12.5;
/// The mist particle count, the reference's 128 for every box — **halved
/// below, and that is a GPU measurement, not taste.** Weather at cap was
/// billing ~2 ms of GPU, and the flakes cannot be it: seven thousand
/// dozen-pixel quads is a couple of million blended pixels, while every mist
/// sprite is *yards* across and half of 128 of them fill the frame several
/// times over. Fill rate is the whole cost of this pass, and the mist is the
/// whole of the fill rate.
const MIST_COUNT: usize = 128;

/// …so this is what is actually spawned at full density: the reference's count
/// at half strength, which with the alpha already toned down keeps the haze
/// and returns most of the 2 ms. A stated divergence, like the counts the
/// other way.
const MIST_DRAWN: usize = MIST_COUNT / 2;

/// The mist's own near fade, longer than the sprites' two yards: a nine-yard
/// sheet four yards from the eye covers most of the screen, and those
/// screenfuls were the worst of the fill bill. Faded by [`NEAR_FADE_YARDS`]'s
/// squared curve over this distance instead, and then actually *culled* by the
/// zero-alpha skip in [`build`], which is what turns the fade into saved
/// fill.
const MIST_NEAR_FADE_YARDS: f32 = 6.0;
/// The fall direction's spread, 0.349 rad = 20° — **the mist
/// emitter's**, which is the box it belongs to. Applying it to the
/// *drops* as well was a round of "diagonal strands of string": twenty degrees
/// at a random yaw makes streaks that lean both ways and cross, and crossing
/// is what string does and rain does not. Each layer carries its own spread
/// now ([`Layer::spread`]); the mist keeps this one.
const SPREAD: f32 = 0.349;

/// **The drop and flake counts at full density**:
/// a per-type constant times 0.66 times the density. The
/// constants are the ordinary branch's (6500 and
/// 1300); the other branch's 35000 and 14000 sit behind a byte this round did
/// not identify.
const RAIN_DROPS_FULL: usize = 6500 * 66 / 100;
const SNOW_FLAKES_FULL: usize = 1300 * 66 / 100;

/// **How long a rain streak is, and it is measured**: `rain.bls` builds the
/// trailing pair of corners from the drop's position `2 / -velocity.z` seconds
/// further along its fall — `RCP R0.x, -v18.z; MAD R0.w, c16.y, R0.x, v24.y`
/// with `c16.y = 2` — so the streak spans exactly **two yards of fall**,
/// whatever the speed. It is not a length in the drop's own direction: a
/// wind-blown drop's streak is longer along the slant and still two yards
/// tall.
pub const STREAK_FALL_YARDS: f32 = 2.0;

/// **Where a point sprite has shrunk to nothing**, the far end of
/// `snowpoint.bls`'s `MAD R0.z, dist, c24.x, c24.y` — the two constants are
/// program locals the shader text does not carry, so the *form* is the
/// measurement and this distance is chosen: the box's own far corner, which is
/// `sqrt(22^2 + 22^2 + 12.5^2)` rounded up. Past it nothing is drawn anyway.
const POINT_FADE_END: f32 = 34.0;

/// …and the floor under it, which is the shader's own: `MAX result.pointsize,
/// 1, …`. A flake at the far edge of the box is one pixel rather than nothing.
const POINT_MIN_PIXELS: f32 = 1.0;

/// **How near the camera a particle fades out**, in yards. This is a choice and
/// it has no counterpart in the reference: neither shader has a near fade, and the reference draws a
/// drop that is two inches from the eye at whatever size the projection gives
/// it. What that produces here is a white bar across the whole frame, because a
/// streak is two yards long and the box is centred on the eye — so the last
/// yards are faded instead, on a squared curve so the very close ones go
/// fastest. The reference's own answer to the same problem is a texture this
/// client does not draw yet (`RainDropSplash01.blp`).
const NEAR_FADE_YARDS: f32 = 2.0;

/// …and the streak's own, longer: a streak is yards of geometry where a
/// flake is a dozen pixels, so the distance inside which it dominates the
/// frame is larger. "The raindrops are too big" was mostly the drops inside
/// this radius.
const STREAK_NEAR_FADE_YARDS: f32 = 3.5;

/// **The far field's box, and it is this client's own — a stated divergence.**
/// The reference stops at its 44 x 44 x 25 near box, so a snowy vista fifty
/// yards out has no flakes in it at all and heavy snow reads as "snowing only
/// near me". Real snow compounds with distance: flakes at every depth pile up
/// into the haze that closes visibility. A second, sparse pool of small point
/// sprites lives in this larger box to supply the mid-distance; the visibility
/// drop itself is the fog's — see `sky::resolve` and
/// [`WeatherState::visibility`].
const FAR_BOX_HALF_XY: f32 = 60.0;
const FAR_BOX_HALF_Z: f32 = 40.0;
/// Where a far flake's point size has shrunk to its one-pixel floor — past the
/// box's own diagonal, so a far flake never quite vanishes inside it.
const FAR_POINT_FADE_END: f32 = 140.0;

/// **How much of the fog the heaviest precipitation takes**, as a fraction of
/// the storm row's own distance. This client's, by request, and physical in
/// direction: visibility in falling weather drops with the density of what is
/// in the air, which is exactly the number the density curve already is. At a
/// drizzle it is untouched, because the density floor is a quarter of the
/// grade. This is rain's; snow and sand take more — see
/// [`VISIBILITY_TAKEN_SNOW`].
pub const VISIBILITY_TAKEN: f32 = 0.45;

/// …and snow's (and a sandstorm's), deeper: a cap-grade snow is a blizzard,
/// and a blizzard is mostly the world ending eighty yards out. Dun Morogh's
/// storm fog is 278 yards; at cap this leaves 83 of them, against rain's 153.
pub const VISIBILITY_TAKEN_SNOW: f32 = 0.70;
/// …and the floor under that, so an instance whose fog is already close does
/// not squeeze to nothing.
pub const VISIBILITY_FLOOR_YARDS: f32 = 60.0;

/// **The grade at which the sky is fully the storm row's.** This client's
/// reading, by request, and physical in direction: clouds precede
/// precipitation, so a sky that is snowing at all is already overcast — a
/// linear blend left vmangos's "light snow" (grade 0.3) under 70% blue sky,
/// which the report called "snowing with a clear sky". Full overcast by the
/// server's own "medium" (0.6); the particles keep their own curve.
const STORM_SKY_FULL_GRADE: f32 = 0.6;

/// **The count curve's exponent** — the counts follow `density^0.5` rather
/// than the density itself, this client's own and by request ("in wow it
/// relies more heavily on the snowflakes"): the reference's linear curve puts
/// 57 flakes in the whole box at light snow, which reads as nothing at all.
/// The square root fills the low grades without moving the cap.
const COUNT_CURVE: f32 = 0.5;

/// **How much faster the rain falls at full density** — `1 + PACE * density`
/// multiplies every particle's own velocity. This client's own, by request
/// ("snowfall should speed up with intensity"): a drizzle drifts and a
/// downpour drives. Rain's; the snow carries a steeper one of its own in
/// [`Layer::for_kind`], and both are [`Layer::pace_with_density`].
const PACE_WITH_DENSITY: f32 = 0.8;

/// Every how many frames one particle's own indoor test is refreshed. The
/// particles are world-anchored and buildings do not move, so the answer is
/// nearly constant per particle; a quarter of the pool a frame keeps the cost
/// at a few hundred room tests while a fast drop can intrude at most a couple
/// of yards past a doorway before its flag catches up.
const INDOOR_REFRESH: usize = 4;

/// How fast the camera's own indoor state eases, in seconds edge to edge —
/// what keeps the fog from snapping at a doorway. See
/// [`WeatherState::visibility`].
const INDOOR_BLEND_SECONDS: f32 = 2.0;

/// What is falling, ramped — the client side of [`Weather`].
#[derive(Resource, Default)]
pub struct WeatherState {
    /// The packet this ramp is running toward, and what it was running from.
    target: Option<Weather>,
    from_grade: f32,
    /// When the ramp started, in `Time::elapsed_secs`.
    started: f32,
    /// The grade in force this frame — the lerp the reference keeps.
    pub grade: f32,
    /// What kind the picture is of. Held separately from `target` because the
    /// kind switches the moment the packet lands while the grade
    /// ramps, which is what makes rain turn to snow on the wing.
    pub kind: WeatherKind,
    /// **The camera is inside a building's room**, by [`crate::render::wmos::Interior::holds`]
    /// — the same test every unit's lighting makes. What it gates is the
    /// **fog squeeze only**, through [`Self::indoor_blend`]: the particles are
    /// gated one by one instead ([`Pool`]'s mask), so the world outside
    /// a doorway keeps snowing in view while the room stays dry, and stepping
    /// out is continuous rather than 100%-clear-to-storm in one frame.
    pub indoors: bool,
    /// [`Self::indoors`] eased over [`INDOOR_BLEND_SECONDS`], 0 outdoors and
    /// 1 indoors, so the fog release is a walk through a doorway rather than
    /// a cut.
    indoor_blend: f32,
    /// A frame counter for the rotation the particle indoor tests run on.
    tick: u32,
    /// The three pools — the drops or flakes, the mist, the far field — as the
    /// records the meshes were built from and the two things about each that
    /// move: how many are alive and which stand under a roof.
    pools: [PoolState; 3],
    /// Which weather the pools and meshes were built for; `None` until the
    /// first, and again when the kind switches — a flake is not a slow
    /// raindrop, and a build for the new kind is what a kind switch is.
    built_for: Option<WeatherKind>,
    /// The running integral of [`wind_now`], in yards, since the pools were
    /// built. In `f64` because it grows for the whole of a storm; it reaches
    /// the shader wrapped to each pool's box, which under a torus is exact.
    wind_travelled: DVec3,
}

/// **`--weather <kind>[,<grade>]`: a packet the server never sent, for the
/// whole run.** The scripted twin of the server's own `.wchange`, and it
/// exists for the reason `--hour` does: a scripted shot cannot wait for a
/// zone to decide to snow, and a pass that cannot be photographed from a
/// script is a pass nobody prices twice. Read by [`follow_the_server`] in
/// place of the wire's packet, instant, so the ramp does not eat the shot's
/// settle window.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ForcedWeather(pub Weather);

impl ForcedWeather {
    /// `rain`, `snow`, `sand` (or `storm`) or `fine`, with an optional grade
    /// in 0..1 after a comma — `snow,0.6`. Anything else is `None`, and the
    /// caller warns rather than photographing the server's own sky.
    pub fn parse(text: &str) -> Option<ForcedWeather> {
        let (kind, grade) = match text.trim().split_once(',') {
            Some((kind, grade)) => (kind.trim(), grade.trim().parse::<f32>().ok()?),
            None => (text.trim(), 1.0),
        };
        let kind = match kind.to_ascii_lowercase().as_str() {
            "rain" => WeatherKind::Rain,
            "snow" => WeatherKind::Snow,
            "sand" | "storm" => WeatherKind::Storm,
            "fine" | "clear" => WeatherKind::Fine,
            _ => return None,
        };
        (0.0..=1.0).contains(&grade).then_some(ForcedWeather(Weather {
            kind,
            grade,
            sound: 0,
            instant: true,
        }))
    }
}

/// One pool's CPU-side half.
#[derive(Default)]
struct PoolState {
    particles: Vec<Particle>,
    /// How many of them are drawn: the density's count, walked toward a few
    /// a frame — see [`walk_alive`].
    alive: usize,
    /// One bit per particle: standing inside a building's room this frame.
    /// The shader's storage buffer, rewritten only when a bit moves.
    indoors: Vec<u32>,
}

impl PoolState {
    fn hidden(&self) -> usize {
        self.indoors.iter().map(|word| word.count_ones() as usize).sum()
    }
}

impl WeatherState {
    /// `max(0, (grade - 0.25) * 4/3)` — see [`Weather::density`]. Not gated on
    /// [`Self::indoors`]: the pools keep falling while the camera is under a
    /// roof, because the doorway is in view — what an indoor camera does not
    /// see is decided per particle.
    pub fn density(&self) -> f32 {
        if self.kind == WeatherKind::Fine {
            return 0.0;
        }
        Weather::density(self.grade)
    }

    /// **How far the light is toward its storm row** — the ramped grade over
    /// [`STORM_SKY_FULL_GRADE`], so the overcast is complete by the server's
    /// own "medium" — and the whole of what `render::sky` reads here. See
    /// [`vale_assets::tables::light::LightTables::atmosphere_in_storm`].
    ///
    /// The grade rather than the density on purpose, and *not* gated on the
    /// kind the way [`Self::density`] is. Two reasons, both about the ramp:
    /// the density's floor is a quarter of the grade, so a shower the reference
    /// draws no drops for would otherwise leave the sky as bright as a clear
    /// one; and the kind switches the instant a packet lands while the grade
    /// takes ten seconds to follow it, so gating on the kind would snap the
    /// whole world back to daylight in one frame at the end of a storm. Fine
    /// weather is grade 0 on the wire — vmangos' `Weather::ReGenerate` sends
    /// nothing else — so nothing is lost by trusting it here.
    pub fn storm(&self) -> f32 {
        (self.grade / STORM_SKY_FULL_GRADE).clamp(0.0, 1.0)
    }

    /// **What precipitation leaves of the fog distance**, 1.0 for a clear sky
    /// — this client's own visibility drop, by request; see
    /// [`VISIBILITY_TAKEN`]. On the *density* rather than the grade,
    /// deliberately and opposite to [`Self::storm`]: the sky greys with the
    /// weather machine's state, but visibility is taken by what is physically
    /// in the air, which is the particle count. Snow takes more than rain per
    /// unit of density, because a flake is a wall where a drop is a thread.
    /// …and released — smoothly — while the camera is indoors, because the
    /// squeeze is about what the *camera* is looking through and a blizzard's
    /// eighty-yard world would otherwise haze Ironforge's own halls. Eased
    /// over [`INDOOR_BLEND_SECONDS`] so the fog walks through the doorway
    /// with you instead of cutting.
    pub fn visibility(&self) -> f32 {
        let taken = match self.kind {
            WeatherKind::Snow | WeatherKind::Storm => VISIBILITY_TAKEN_SNOW,
            _ => VISIBILITY_TAKEN,
        };
        let outside = 1.0 - self.indoor_blend.clamp(0.0, 1.0);
        1.0 - taken * outside * self.density().clamp(0.0, 1.0)
    }
}

/// One falling thing, as the mesh was built from it. What moves — where it
/// is, how old it is — is the shader's, from these and the time.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Particle {
    /// Yards per second, its own fall with its own scatter.
    velocity: Vec3,
    /// Seconds of life.
    life: f32,
    /// Where in its first life it started, 0..1, so the fades are spread
    /// rather than synchronised.
    phase: f32,
    /// A per-particle size factor, so a layer is not a lattice.
    size: f32,
}

/// The one layer's numbers, per kind. **Every number here is a choice unless
/// its comment names it as the reference's.**
struct Layer {
    /// Yards per second, straight down before the spread is applied.
    fall: f32,
    /// Whether the sprite layer is drawn as a streak along its fall (rain) or
    /// as a point sprite facing the eye (snow, sand).
    streak: bool,
    /// A streak's half-width, in yards — rain only, and a choice.
    width: f32,
    /// A streak's length, in yards of fall. The reference's is
    /// [`STREAK_FALL_YARDS`] — two, measured — and rain's here is shorter, by
    /// request. The *rule* (length is yards of fall, not of travel) is kept.
    streak_fall: f32,
    /// The far field's population at full density — see [`FAR_BOX_HALF_XY`].
    /// Zero for a kind with no far field, which is every kind but snow: far
    /// raindrops are individually invisible in life too, and the rain's
    /// mid-distance is carried by the fog and the mist alone.
    far_count_full: usize,
    /// …and its sprite size in pixels, smaller than the near flakes'.
    far_size: (f32, f32),
    /// A point sprite's size in **pixels**, min..max, before the distance
    /// falloff — see [`Shape::Point`] and the module note. Not yards: that is
    /// the difference the screenshots this round was opened for were of.
    size: (f32, f32),
    /// The mist's size in **yards**, min..max — the reference's own pair, and
    /// the one layer that is sized in the world rather than on the screen.
    mist_size: (f32, f32),
    mist_fall: f32,
    /// The fall direction's per-particle scatter for the drops and the far
    /// field, radians off vertical at a random yaw. The mist always takes the
    /// measured [`SPREAD`]; rain's own is a few degrees, because near-parallel
    /// is most of what makes fast streaks read as rain.
    spread: f32,
    /// A shared horizontal wind, yards per second — what tilts the whole fall
    /// one way, where [`Layer::spread`] scatters each drop its own way. This client's
    /// own, by request ("they seem to have some sort of wind tilt"): every
    /// particle gets the same slowly-turning, lightly-gusting vector, so the
    /// snow leans together the way blown snow does. See [`wind_now`].
    wind: f32,
    /// **How much the wind grows with the density**: the wind this second is
    /// `wind * (1 + this * density)`, so a heavy fall leans further than a
    /// light one. This client's own, by request ("heavy snow in wind"),
    /// and free: one multiply on the CPU, nothing per particle.
    wind_with_density: f32,
    /// …and how much faster the fall runs, on the same shape — see
    /// [`PACE_WITH_DENSITY`], which is rain's value.
    pace_with_density: f32,
    count_full: usize,
    tint: [f32; 4],
}

impl Layer {
    fn for_kind(kind: WeatherKind) -> Option<Layer> {
        Some(match kind {
            WeatherKind::Fine => return None,
            WeatherKind::Rain => Layer {
                // A raindrop's terminal velocity is ~9 m/s; this is three
                // times that because the streak is what is seen rather than
                // the drop, it is a choice, and both 18 and 30 were reported
                // from the window as too slow ("slow motion"). The streak's
                // *length* is not a choice — see [`STREAK_FALL_YARDS`].
                fall: 36.0,
                // Three and a half degrees: rain falls nearly parallel, and
                // the mist's twenty made crossing strands.
                spread: 0.06,
                streak: true,
                // Shortened from the reference's measured two yards, by
                // request three times — at thirty-odd yards a second a short
                // streak reads as speed where a long one reads as slow-motion
                // string. Under a yard is a raindrop's dash; back up from
                // three quarters when the width got its pixel floor, since a
                // streak that is finally drawn can afford to be seen.
                streak_fall: 0.9,
                // A centimetre either side of the drop's line. The reference's
                // is the pair of per-corner constants `c10[A0.x]`, which are
                // program locals and not in the shader text. Sized against the
                // two reports it sits between: wide enough that 0.4-grade rain
                // (a fifth of the drops) still reads at all, thin enough that
                // a downpour is not white bars.
                width: 0.012,
                // Unused for a streak: rain is sized in the world, because
                // `rain.bls` is geometry rather than a point sprite.
                size: (0.0, 0.0),
                // The rain mist is 1.2..5.0.
                mist_size: (1.2, 5.0),
                mist_fall: 1.5,
                // Four yards a second against a thirty-yard fall: a rain that
                // leans about seven degrees, all one way — ten at a downpour.
                wind: 4.0,
                wind_with_density: 0.5,
                pace_with_density: PACE_WITH_DENSITY,
                // Twice the reference's ordinary branch. It stood at 2.5x for
                // a round and came back down when weather was measured at
                // ~2 ms of GPU: a streak is two orders of magnitude more
                // pixels than a flake, so rain is where the fill budget
                // actually goes, and the flakes — which are what the requests
                // kept asking more of — are where the intensity stays.
                count_full: RAIN_DROPS_FULL * 2,
                far_count_full: 0,
                far_size: (0.0, 0.0),
                // Chosen, between two reports: 0.55 made a downpour white bars
                // and 0.38 made a 0.4-grade shower barely legible. The near
                // fade is what now keeps a downpour from washing out, so the
                // alpha can carry the light rain. Raised again with the pixel
                // floor on the width ("the raindrops don't look like much"):
                // the floor is what made them visible at all, and the tint
                // is a shade brighter and bluer over it.
                tint: [0.85, 0.9, 1.0, 0.62],
            },
            WeatherKind::Snow => Layer {
                // Raised three times by request — "falling more aggressively",
                // then "faster, heavy snow in wind" — and scaled further by
                // `pace_with_density` at the heavy end: 2.6 at a flurry, 6.8
                // at a blizzard.
                fall: 2.6,
                // Flakes flutter: the mist's own twenty degrees reads right
                // for snow, which has no line to keep straight.
                spread: SPREAD,
                streak: false,
                width: 0.0,
                streak_fall: 0.0,
                // **Pixels**, and chosen: `snowpoint.bls` scales its curve by
                // `c24.z`, a program local. A flake is a 64x64 texture with a
                // lot of transparent border, so the painted part of this is
                // roughly half of it.
                size: (9.0, 20.0),
                // The snow mist is 3.0..9.0.
                mist_size: (3.0, 9.0),
                mist_fall: 0.6,
                // …against snow's fall, a lean past thirty degrees at the
                // gusts' top at a flurry — and at a blizzard the wind has
                // grown by `wind_with_density` to 5.3 against a 6.8 fall,
                // which is snow going by at nearly forty degrees.
                wind: 2.4,
                wind_with_density: 1.2,
                pace_with_density: 1.6,
                // Nine times the reference's 858, raised four times by
                // request — "more snowfall particles is what I've been
                // getting at", then "a little more" — and still under the
                // client's other branch (14,000 before the scale). A tenth
                // more quads on a pass whose cost is the mist's fill.
                count_full: SNOW_FLAKES_FULL * 9,
                // The far field is snow's alone — see [`FAR_BOX_HALF_XY`].
                // Trimmed from 3,500 when the pass was measured: every quad
                // in these pools is cloned and uploaded every frame whatever
                // the culls save, so the far field — a third of the vertex
                // buffer for the subtlest layer — is where a cut costs least.
                far_count_full: 3200,
                far_size: (4.0, 9.0),
                tint: [1.0, 1.0, 1.0, 0.95],
            },
            WeatherKind::Storm => Layer {
                // A sandstorm is the mist alone, driven sideways, and tinted:
                // `sand.bls` multiplies the vertex colour by `c0[0]`, the
                // storm's own colour uniform, which this reads as sand.
                fall: 0.0,
                streak: false,
                width: 0.0,
                spread: SPREAD,
                streak_fall: 0.0,
                size: (0.0, 0.0),
                mist_size: (3.0, 9.0),
                mist_fall: 0.4,
                // A sandstorm is nothing but wind.
                wind: 6.0,
                wind_with_density: 0.5,
                pace_with_density: 0.0,
                count_full: 0,
                far_count_full: 0,
                far_size: (0.0, 0.0),
                tint: [0.76, 0.62, 0.40, 0.5],
            },
        })
    }
}

/// Which of the three standing meshes an entity is.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Pool {
    /// The drops or the flakes: the near box, the loud layer.
    Drops,
    /// The mist under them — the reference's own second layer.
    Mist,
    /// The far flakes — this client's own; see [`FAR_BOX_HALF_XY`].
    Far,
}

impl Pool {
    const ALL: [Pool; 3] = [Pool::Drops, Pool::Mist, Pool::Far];

    fn index(self) -> usize {
        match self {
            Pool::Drops => 0,
            Pool::Mist => 1,
            Pool::Far => 2,
        }
    }

    /// The box this pool lives in: half-extents across and up.
    fn halves(self) -> (f32, f32) {
        match self {
            Pool::Far => (FAR_BOX_HALF_XY, FAR_BOX_HALF_Z),
            _ => (BOX_HALF_XY, BOX_HALF_Z),
        }
    }
}

/// The three standing meshes.
#[derive(Component)]
struct WeatherLayer {
    pool: Pool,
    material: Handle<WeatherMaterial>,
}

#[derive(Resource, Default)]
struct WeatherMeshes {
    spawned: bool,
}

/// How many `vec4<u32>` the indoor mask is: 9,216 bits, over the largest
/// pool (rain's 8,580 drops). A uniform rather than a storage buffer because
/// a kilobyte a pool a frame is nothing and it rides the upload the
/// parameters already make.
pub const MASK_VEC4S: usize = 72;

/// **The shader's parameters**, one set per pool. Every number here is
/// written by [`draw`]; the layout is `weather.wgsl`'s.
#[derive(Clone, Copy, ShaderType, Debug, PartialEq)]
pub struct WeatherParams {
    /// half_xy, half_z, alive, pace.
    pub half: Vec4,
    /// The wind's integral wrapped to this box (x, 0, z); w = the time.
    pub wind: Vec4,
    /// The wind this second — the streak's slant; w = the point size's fade end.
    pub gust: Vec4,
    pub tint: Vec4,
    /// kind (0 world, 1 point, 2 streak); a streak's half-width or a point
    /// sprite's floor in pixels; a streak's yards of fall; the near fade
    /// distance.
    pub shape: Vec4,
    /// One bit per particle: standing inside a building's room, so it falls
    /// unseen. Word `i` of the mask is component `i % 4` of element `i / 4`.
    pub mask: [UVec4; MASK_VEC4S],
}

impl Default for WeatherParams {
    fn default() -> Self {
        WeatherParams {
            half: Vec4::ZERO,
            wind: Vec4::ZERO,
            gust: Vec4::ZERO,
            tint: Vec4::ZERO,
            shape: Vec4::ZERO,
            mask: [UVec4::ZERO; MASK_VEC4S],
        }
    }
}

/// The mask's words packed into the uniform's vec4s.
fn pack_mask(words: &[u32]) -> [UVec4; MASK_VEC4S] {
    let mut mask = [UVec4::ZERO; MASK_VEC4S];
    for (i, word) in words.iter().enumerate().take(MASK_VEC4S * 4) {
        mask[i / 4][i % 4] = *word;
    }
    mask
}

/// One pool's material: its parameters (the indoor mask among them) and its
/// sprite.
///
/// Alpha-blended (blend 2), unlit, **fogged** — `rain.bls` writes `fogcoord`,
/// so a drop forty yards off is as grey as the hill behind it — two-sided,
/// and never writing depth, since a streak in front of a tree must not cut a
/// hole in it.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct WeatherMaterial {
    #[uniform(0)]
    pub params: WeatherParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
}

impl Material for WeatherMaterial {
    fn vertex_shader() -> ShaderRef {
        super::shader::WEATHER.into()
    }

    fn fragment_shader() -> ShaderRef {
        super::shader::WEATHER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // A streak turned about its fall faces the eye from one side or the
        // other as the drop passes; a flake's square is whichever way it was
        // built.
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/weather.wgsl");
        app.init_resource::<WeatherState>()
            .init_resource::<WeatherMeshes>()
            .add_plugins(MaterialPlugin::<WeatherMaterial>::default())
            .add_systems(
                Update,
                (
                    follow_the_server,
                    // Centred on the camera, so after it has been placed — the
                    // same anchor `sky::follow_camera` takes, and stated for the
                    // same reason.
                    draw.after(crate::world::camera::place),
                )
                    .chain(),
            );
        // The HUD line, which is not part of drawing the weather — see
        // `ui::debug` for what the `diagnostics` feature removes.
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report.run_if(crate::ui::report::watched));
    }
}

/// **Take each packet as the reference's `Set` takes it**: the kind at once, the
/// grade as a ramp from wherever the last ramp had reached.
pub(crate) fn follow_the_server(
    status: Res<crate::world::session::WorldStatus>,
    forced: Option<Res<ForcedWeather>>,
    time: Res<Time>,
    rig: Res<crate::world::camera::CameraRig>,
    interiors: Query<&crate::render::wmos::Interior>,
    mut state: ResMut<WeatherState>,
) {
    let now = time.elapsed_secs();
    // The wire's packet, or the one `--weather` stands in for it.
    let packet = match forced.as_deref() {
        Some(forced) => Some(forced.0),
        None => status.weather,
    };
    // A new packet, or the first. The comparison is on the whole value, which
    // is what the reference does too: the same weather restated starts no
    // new ramp.
    if packet != state.target {
        let from = state.grade;
        state.target = packet;
        state.from_grade = from;
        state.started = now;
        let kind = packet.map_or(WeatherKind::Fine, |weather| weather.kind);
        // **A kind switch empties the pools.** They are otherwise reused
        // across packets — which is right for a grade change — but a flake is
        // not a slow raindrop: teleporting from a snow zone into a rain zone
        // turned the standing flakes into streaks still falling at snow's
        // speed, and the recycle path then re-derived each rebirth's speed
        // from the old velocity, so the slow-motion rain outlived every one of
        // them. The pools are rebuilt for the new kind and the alive count
        // walks up again at `walk_alive`'s own pace, which reads as the new
        // weather arriving.
        if kind != state.kind {
            state.kind = kind;
            state.built_for = None;
        }
        if let Some(weather) = packet {
            if weather.instant {
                state.grade = weather.grade;
                state.from_grade = weather.grade;
            }
        }
    }
    // The ramp: `|to - from| * 10` seconds, a plain lerp of the two.
    let to = state.target.map_or(0.0, |w| w.grade);
    let duration = Weather::ramp_seconds(state.from_grade, to);
    let t = if duration > 0.0 {
        ((now - state.started) / duration).clamp(0.0, 1.0)
    } else {
        1.0
    };
    state.grade = state.from_grade + (to - state.from_grade) * t;
    // …and a sky that has ramped down to nothing is fine, so the last of the
    // snow does not hang in a "snow at grade 0" state forever.
    if state.grade <= 0.0 && to <= 0.0 {
        state.kind = WeatherKind::Fine;
    }
    // **Under a roof, nothing falls** — see [`WeatherState::indoors`]. The eye
    // rather than the character, because the weather is drawn around the
    // camera; `CameraRig::eye` is already in the world's own axes, which is
    // what `Interior::holds` takes. Asked only while there is weather to gate,
    // so a clear day costs none of the per-building box tests.
    let indoors = state.kind != WeatherKind::Fine && {
        let eye = rig.eye().to_array();
        interiors.iter().any(|interior| interior.holds(eye))
    };
    if state.indoors != indoors {
        state.indoors = indoors;
    }
    // …and the eased copy the fog reads, walked rather than cut — see
    // [`WeatherState::visibility`].
    let toward = if indoors { 1.0 } else { 0.0 };
    let step = time.delta_secs() / INDOOR_BLEND_SECONDS;
    if state.indoor_blend != toward {
        state.indoor_blend = if state.indoor_blend < toward {
            (state.indoor_blend + step).min(toward)
        } else {
            (state.indoor_blend - step).max(toward)
        };
    }
}

/// The world tab's line, after the sky's (`sky::SKY` is 10).
///
/// **A system of its own, behind the feature**, like every other pass's report
/// — see [`crate::ui::report`]. It was a tail on [`follow_the_server`] until
/// the `--no-default-features` build was run against it, where
/// `crate::ui::report` does not exist at all and the pass that runs the weather
/// took the whole build down with it.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(12);

#[cfg(feature = "diagnostics")]
fn report(state: Res<WeatherState>, mut hud: ResMut<crate::ui::report::HudReport>) {
    let target = state.target.map_or(0.0, |weather| weather.grade);
    let line = match state.kind {
        WeatherKind::Fine => "clear".to_string(),
        kind => {
            let hidden: usize = state.pools.iter().map(PoolState::hidden).sum();
            let falling = state.pools[Pool::Drops.index()].alive + state.pools[Pool::Far.index()].alive;
            format!(
                "{kind:?} grade {:.2} -> {target:.2}, density {:.2}, {falling} falling, {hidden} under a roof{}",
                state.grade,
                state.density(),
                if state.indoors { " (so is the camera)" } else { "" },
            )
        }
    };
    hud.set(crate::ui::report::Section::World, SLOT, "weather", line);
}

/// **Keep the three pools' parameters current**, and rebuild them when the
/// weather changes kind.
#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<super::tuning::WorldTuning>,
    camera: Query<&Transform, With<crate::world::camera::WorldCamera>>,
    mut state: ResMut<WeatherState>,
    mut held: ResMut<WeatherMeshes>,
    mut cache: ResMut<ModelCache>,
    mut materials: ResMut<Assets<WeatherMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut layers: Query<
        (&WeatherLayer, &mut Transform, &mut Mesh3d, &mut Visibility),
        Without<crate::world::camera::WorldCamera>,
    >,
    // The loaded buildings' room boxes, for hiding the particles that fall
    // inside one — see [`PoolState::indoors`].
    interiors: Query<&crate::render::wmos::Interior>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Particles);
    let Some(eye) = camera.iter().next() else { return };
    let density = if tuning.weather { state.density() } else { 0.0 };
    let layer = Layer::for_kind(state.kind).filter(|_| density > 0.0);

    // Nothing falling: hide the meshes, and do no other work — which is every
    // frame of every clear day.
    let Some(layer) = layer else {
        if state.pools.iter().any(|pool| pool.alive > 0) {
            for pool in &mut state.pools {
                pool.alive = 0;
            }
            for (_, _, _, mut visibility) in &mut layers {
                *visibility = Visibility::Hidden;
            }
        }
        return;
    };

    // The three textures, through the model cache's own door so they are
    // resident and evicted like every other texture in the world.
    let sprite = match state.kind {
        WeatherKind::Rain => RAIN_DROP,
        _ => SNOW_FLAKE,
    };
    let (
        TextureState::Ready(sprite_image),
        TextureState::Ready(mist_image),
        TextureState::Ready(flake_image),
    ) = (
        cache.texture(sprite),
        cache.texture(MIST),
        cache.texture(SNOW_FLAKE),
    ) else {
        return;
    };
    if !held.spawned {
        held.spawned = true;
        for (pool, name) in [
            (Pool::Drops, "weather drops"),
            (Pool::Mist, "weather mist"),
            (Pool::Far, "weather far"),
        ] {
            let material = materials.add(WeatherMaterial {
                params: WeatherParams::default(),
                texture: mist_image.clone(),
            });
            commands.spawn((
                WeatherLayer { pool, material: material.clone() },
                Mesh3d(meshes.add(empty())),
                MeshMaterial3d(material),
                Transform::from_translation(eye.translation),
                // **A box of its own, because nothing computes one**: the mesh
                // is render-world only, so Bevy never sees its vertices, and
                // an entity with no `Aabb` is what the GPU occlusion culler
                // rejects outright — three pools that were prepared, queued
                // and never drawn. The far field's box, which holds all three.
                bevy::camera::primitives::Aabb::from_min_max(
                    Vec3::new(-FAR_BOX_HALF_XY, -FAR_BOX_HALF_Z, -FAR_BOX_HALF_XY),
                    Vec3::new(FAR_BOX_HALF_XY, FAR_BOX_HALF_Z, FAR_BOX_HALF_XY),
                ),
                NoFrustumCulling,
                bevy::light::NotShadowCaster,
                Visibility::Hidden,
                Name::new(name),
            ));
        }
        // Spawned this frame; the entities exist next frame.
        return;
    }

    let now = time.elapsed_secs();
    let dt = time.delta_secs().min(0.1);
    let kind = state.kind;

    // **A kind switch, or the first weather, builds the pools and the
    // meshes** — once, at the layer's full count. The ramp is `alive`.
    if state.built_for != Some(kind) {
        state.built_for = Some(kind);
        state.wind_travelled = DVec3::ZERO;
        let mut rng = now_bits(now);
        for pool in Pool::ALL {
            let (fall, sizes, spread, count) = match pool {
                Pool::Drops => (layer.fall, layer.size, layer.spread, layer.count_full),
                Pool::Mist => (layer.mist_fall, layer.mist_size, SPREAD, MIST_DRAWN),
                Pool::Far => (layer.fall, layer.far_size, layer.spread, layer.far_count_full),
            };
            let count = count.min(MASK_VEC4S * 128);
            let particles: Vec<Particle> =
                (0..count).map(|_| spawn(&mut rng, fall, sizes, spread)).collect();
            let words = (count / 32 + 1).max(1);
            let slot = &mut state.pools[pool.index()];
            slot.alive = 0;
            slot.indoors = vec![0; words];
            slot.particles = particles;
        }
        for (which, _, mut mesh, _) in &mut layers {
            let particles = &state.pools[which.pool.index()].particles;
            mesh.0 = meshes.add(build_mesh(particles, which.pool.halves()));
            if let Some(mut material) = materials.get_mut(&which.material) {
                material.texture = match which.pool {
                    Pool::Mist => mist_image.clone(),
                    Pool::Far => flake_image.clone(),
                    Pool::Drops => sprite_image.clone(),
                };
            }
        }
    }

    // **The wind's integral**, and the pace the fall drives at — both grown
    // with the density by the layer's own two numbers, so a heavier fall is
    // faster *and* leans further. See [`Layer::wind_with_density`].
    let wind = wind_now(layer.wind * (1.0 + layer.wind_with_density * density), now);
    state.wind_travelled += wind.as_dvec3() * dt as f64;
    let pace = 1.0 + layer.pace_with_density * density;

    // **Walk the alive counts toward the density**, a few per frame, which is
    // what makes a ramp look like rain thickening rather than switching on.
    // The counts follow the density through [`COUNT_CURVE`]'s square root, so
    // a light snow is a visible snow — the cap is untouched, since 1 to any
    // power is 1.
    let filling = density.powf(COUNT_CURVE);
    let wants = [
        (layer.count_full as f32 * filling) as usize,
        (MIST_DRAWN as f32 * filling.max(0.25)) as usize,
        (layer.far_count_full as f32 * filling) as usize,
    ];
    for (pool, want) in state.pools.iter_mut().zip(wants) {
        let want = want.min(pool.particles.len());
        pool.alive = walk_alive(pool.alive, want);
    }

    // **The buildings that could possibly hold a particle**, once per frame:
    // open country is the ordinary case and it pays one cheap box test per
    // loaded building and nothing per particle.
    let nearby: Vec<&crate::render::wmos::Interior> = interiors
        .iter()
        .filter(|interior| {
            interior.near(
                crate::render::axes::to_wow(eye.translation),
                FAR_BOX_HALF_XY * 1.5,
            )
        })
        .collect();
    state.tick = state.tick.wrapping_add(1);
    let tick = state.tick as usize;
    let WeatherState { pools, wind_travelled, .. } = &mut *state;

    for (which, mut transform, _, mut visibility) in &mut layers {
        transform.translation = eye.translation;
        *visibility = Visibility::Inherited;
        let pool = &mut pools[which.pool.index()];
        let (half_xy, half_z) = which.pool.halves();
        let wind_wrapped = Vec3::new(
            wrap(wind_travelled.x as f32, half_xy),
            0.0,
            wrap(wind_travelled.z as f32, half_xy),
        );

        // **Under a roof it falls unseen** — its own test, in rotation.
        // World-anchored particles against buildings that do not move, so a
        // quarter of the pool a frame is enough; see [`INDOOR_REFRESH`]. The
        // position tested is the one the shader draws: [`particle_rel`] is
        // the vertex program's arithmetic, over the same generator.
        if nearby.is_empty() {
            pool.indoors.iter_mut().for_each(|word| *word = 0);
        } else {
            for (index, p) in pool.particles.iter().enumerate().take(pool.alive) {
                if (index + tick) % INDOOR_REFRESH != 0 {
                    continue;
                }
                let (rel, _) = particle_rel(
                    p,
                    index as u32,
                    now,
                    pace,
                    wind_wrapped,
                    eye.translation,
                    (half_xy, half_z),
                );
                let world = crate::render::axes::to_wow(eye.translation + rel);
                // A mist sprite has *extent*: its centre standing a wall's
                // thickness outside a room still drapes its sheet through it,
                // so its test widens the boxes by its own radius. A flake is a
                // dozen pixels and its centre is the whole truth.
                let margin = if which.pool == Pool::Mist { p.size } else { 0.0 };
                let inside = nearby.iter().any(|interior| interior.holds_within(world, margin));
                let (word, bit) = (index / 32, 1u32 << (index % 32));
                if inside {
                    pool.indoors[word] |= bit;
                } else {
                    pool.indoors[word] &= !bit;
                }
            }
        }

        // Which pool, its colour, and its shape — the mist is the only layer
        // the reference sizes in the world.
        let tint = layer.tint;
        let (tint, shape) = match which.pool {
            Pool::Drops if layer.streak => (
                tint,
                Vec4::new(2.0, layer.width, layer.streak_fall, STREAK_NEAR_FADE_YARDS),
            ),
            Pool::Drops => (tint, Vec4::new(1.0, POINT_MIN_PIXELS, 0.0, NEAR_FADE_YARDS)),
            // Toned down a notch from 0.35 by request — the flakes carry the
            // storm now and the mist is the haze under them.
            Pool::Mist => (
                [tint[0], tint[1], tint[2], tint[3] * 0.28],
                Vec4::new(0.0, 0.0, 0.0, MIST_NEAR_FADE_YARDS),
            ),
            // The far flakes are a little dimmer than the near ones, which is
            // what distance does to a flake against the haze.
            Pool::Far => (
                [tint[0], tint[1], tint[2], tint[3] * 0.65],
                Vec4::new(1.0, POINT_MIN_PIXELS, 0.0, NEAR_FADE_YARDS),
            ),
        };
        let fade_end = match which.pool {
            Pool::Far => FAR_POINT_FADE_END,
            _ => POINT_FADE_END,
        };
        let params = WeatherParams {
            half: Vec4::new(half_xy, half_z, pool.alive as f32, pace),
            wind: Vec4::new(wind_wrapped.x, 0.0, wind_wrapped.z, now),
            gust: Vec4::new(wind.x, wind.y, wind.z, fade_end),
            tint: Vec4::from_array(tint),
            shape,
            mask: pack_mask(&pool.indoors),
        };
        // `get_mut` marks the asset changed, which is what re-uploads the
        // uniform: five vec4s and the mask, once a frame per pool.
        if let Some(mut material) = materials.get_mut(&which.material) {
            material.params = params;
        }
    }
}

/// Grow or shrink an alive count toward `want`, by at most a sixtieth of the
/// larger of the two a frame, so the ten-second ramp is visible as a
/// thickening.
fn walk_alive(alive: usize, want: usize) -> usize {
    let step = (want.max(alive) / 60).max(4);
    if alive < want {
        (alive + step).min(want)
    } else if alive > want {
        alive - (alive - want).min(step)
    } else {
        alive
    }
}

/// A particle's own fall direction spread by up to `spread` and a random size
/// in `size`, with a random phase so the fades are spread rather than
/// synchronised. Its position is the shader's — see [`particle_rel`].
fn spawn(rng: &mut u32, fall: f32, size: (f32, f32), spread: f32) -> Particle {
    let yaw = rand01(rng) * std::f32::consts::TAU;
    let tilt = rand01(rng) * spread;
    let drift = if fall > 0.0 { fall * tilt.sin() } else { 1.0 + rand01(rng) };
    let velocity = Vec3::new(yaw.cos() * drift, -fall * tilt.cos(), yaw.sin() * drift);
    let life = if fall > 0.0 { (2.0 * BOX_HALF_Z / fall).max(0.5) } else { 4.0 + rand01(rng) * 4.0 };
    Particle {
        velocity,
        life,
        phase: rand01(rng),
        size: size.0 + (size.1 - size.0) * rand01(rng),
    }
}

/// **The shader's own generator, bit for bit** — `weather.wgsl`'s `pcg`. The
/// CPU's room test lands on the position that is drawn because this is the
/// hash the seed comes out of on both sides.
fn pcg(x: u32) -> u32 {
    let h = x.wrapping_mul(747796405).wrapping_add(2891336453);
    let w = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277803737);
    (w >> 22) ^ w
}

fn unit(x: u32) -> f32 {
    (pcg(x) & 0xff_ffff) as f32 / 16_777_216.0
}

/// Where a particle is born for the `cycle`th life of its index: anywhere in
/// the box, which is the reference's respawn.
fn seed_for(index: u32, cycle: u32, halves: (f32, f32)) -> Vec3 {
    let k = index.wrapping_mul(3).wrapping_add(cycle.wrapping_mul(0x9E37_79B1));
    Vec3::new(
        (unit(k) * 2.0 - 1.0) * halves.0,
        (unit(k.wrapping_add(1)) * 2.0 - 1.0) * halves.1,
        (unit(k.wrapping_add(2)) * 2.0 - 1.0) * halves.0,
    )
}

/// **Where the shader draws a particle this frame**, relative to the eye, and
/// how far through its life it is — the vertex program's arithmetic, on the
/// CPU, for the room test. `wind` is the wind's integral already wrapped to
/// this pool's box.
fn particle_rel(
    p: &Particle,
    index: u32,
    now: f32,
    pace: f32,
    wind: Vec3,
    eye: Vec3,
    halves: (f32, f32),
) -> (Vec3, f32) {
    let cycles = (now + p.phase * p.life) / p.life;
    let cycle = cycles.floor();
    let age = cycles - cycle;
    let seed = seed_for(index, cycle as u32, halves);
    let raw = seed + p.velocity * (pace * age * p.life) + wind - eye;
    let rel = Vec3::new(wrap(raw.x, halves.0), wrap(raw.y, halves.1), wrap(raw.z, halves.0));
    (rel, age)
}

/// Into `[-half, half)`, however far out the value is — one camera delta can be
/// a teleport's worth.
fn wrap(v: f32, half: f32) -> f32 {
    (v + half).rem_euclid(2.0 * half) - half
}

/// **The wind this second**, shared by every particle of every layer: a
/// direction that drifts right round over about seven minutes and a strength
/// that gusts between three quarters and full over ten-second swells. Both
/// numbers are chosen; what is deliberate about the *shape* is that it is one
/// vector for the whole field — per-particle randomness is [`SPREAD`]'s job,
/// and a wind that every flake shares is what reads as weather rather than
/// noise.
fn wind_now(speed: f32, t: f32) -> Vec3 {
    let yaw = t * 0.015;
    let gust = 0.75 + 0.25 * (t * 0.6).sin();
    Vec3::new(yaw.cos(), 0.0, yaw.sin()) * (speed * gust)
}

/// **One quad per particle, built once.** The position attribute is each
/// particle's first seed — **not** what the shader draws, which it computes,
/// but the box Bevy's own bounds pass reads off a mesh whenever its handle
/// moves: with every position at the origin that box is a point at the eye,
/// which is behind the near plane, and the entity was frustum-culled with
/// the pool full. The normal carries the velocity, the uv which corner the
/// vertex is, and the colour the life, the phase, the size and the index.
/// Render-world only: nothing on the CPU reads it back and nothing ever
/// changes it.
fn build_mesh(particles: &[Particle], halves: (f32, f32)) -> Mesh {
    // **Never empty.** A mesh with no vertices is a zero-byte allocation in
    // the render world's slab allocator, which it reports as a use-after-free
    // on every frame it is drawn; a pool with nothing in it (rain's far
    // field) carries one particle that `alive == 0` collapses.
    let placeholder = [Particle { velocity: Vec3::ZERO, life: 1.0, phase: 0.0, size: 0.0 }];
    let particles = if particles.is_empty() { &placeholder[..] } else { particles };
    let n = particles.len();
    let mut positions = Vec::with_capacity(n * 4);
    let mut normals = Vec::with_capacity(n * 4);
    let mut uvs = Vec::with_capacity(n * 4);
    let mut colors = Vec::with_capacity(n * 4);
    let mut indices = Vec::with_capacity(n * 6);
    for (index, p) in particles.iter().enumerate() {
        let base = (index * 4) as u32;
        let seed = seed_for(index as u32, 0, halves).to_array();
        for corner in [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]] {
            positions.push(seed);
            normals.push(p.velocity.to_array());
            uvs.push(corner);
            colors.push([p.life, p.phase, p.size, index as f32]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A mesh with nothing drawn in it, for the frame between spawning and the
/// first build.
fn empty() -> Mesh {
    build_mesh(&[], (BOX_HALF_XY, BOX_HALF_Z))
}

/// The particle pass's own generator, so the two agree on what "random" is.
fn rand01(rng: &mut u32) -> f32 {
    crate::render::particles::rand01(rng)
}

fn now_bits(secs: f32) -> u32 {
    (secs * 1000.0) as u32 | 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A clear sky costs nothing and draws nothing**: no layer, and a density
    /// of zero whatever the grade says — a grade with no kind is the state
    /// after a ramp down.
    #[test]
    fn fine_weather_has_no_layer_and_no_density() {
        assert!(Layer::for_kind(WeatherKind::Fine).is_none());
        let state = WeatherState { grade: 1.0, kind: WeatherKind::Fine, ..Default::default() };
        assert_eq!(state.density(), 0.0);
        let state = WeatherState { grade: 1.0, kind: WeatherKind::Rain, ..Default::default() };
        assert!((state.density() - 1.0).abs() < 1e-6);
    }

    /// **Under a roof the fog eases back but the world keeps snowing** — the
    /// doorway report. The camera flag gates only the squeeze, through the
    /// eased blend; what an indoor camera does not see is decided per
    /// particle, and the sky mix is not gated at all.
    #[test]
    fn indoors_the_fog_eases_back_but_the_world_keeps_snowing() {
        let mut state = WeatherState { grade: 1.0, kind: WeatherKind::Snow, ..Default::default() };
        assert!(state.density() > 0.99);
        assert!((state.visibility() - (1.0 - VISIBILITY_TAKEN_SNOW)).abs() < 1e-6);
        state.indoors = true;
        state.indoor_blend = 1.0;
        assert!(state.density() > 0.99, "the pools keep falling for the doorway");
        assert_eq!(state.visibility(), 1.0, "…but the camera's fog is its own");
        state.indoor_blend = 0.5;
        let half = state.visibility();
        assert!(half > 1.0 - VISIBILITY_TAKEN_SNOW && half < 1.0, "eased, not cut: {half}");
        assert!(state.storm() > 0.99, "the light mix is not gated");
    }

    /// **The sky is fully overcast by the server's own "medium"** — the grade
    /// the mix runs on reaches 1 at [`STORM_SKY_FULL_GRADE`], so light snow is
    /// under a clouded sky rather than 30% of one.
    #[test]
    fn the_sky_commits_to_overcast_by_medium_weather() {
        let at = |grade| WeatherState { grade, kind: WeatherKind::Snow, ..Default::default() }.storm();
        assert_eq!(at(0.0), 0.0);
        assert!((at(0.3) - 0.5).abs() < 1e-6, "half way at light snow");
        assert_eq!(at(0.6), 1.0, "fully the storm row at medium");
        assert_eq!(at(0.9), 1.0);
    }

    /// **The wind is one vector for the whole field**: it turns and gusts with
    /// time, never points up or down, and never exceeds the layer's speed —
    /// per-particle scatter is [`SPREAD`]'s job.
    #[test]
    fn the_wind_is_shared_and_stays_level() {
        let a = wind_now(1.4, 10.0);
        let b = wind_now(1.4, 10.0);
        assert_eq!(a, b, "one wind per instant");
        for t in [0.0, 7.0, 100.0, 400.0] {
            let w = wind_now(1.4, t);
            assert_eq!(w.y, 0.0, "level");
            assert!(w.length() <= 1.4 + 1e-4, "gusts cap at the layer's speed");
            assert!(w.length() >= 1.4 * 0.5 - 1e-4, "and never die entirely");
        }
        let early = wind_now(1.4, 0.0);
        let later = wind_now(1.4, 100.0);
        assert!(early.angle_between(later).abs() > 0.5, "the direction walks");
    }

    /// **The visibility drop rides the density, not the grade** — a drizzle
    /// under the density floor takes none of the fog, the cap takes the
    /// kind's own fraction, and a cap blizzard closes in deeper than a cap
    /// downpour.
    #[test]
    fn visibility_drops_with_the_density_and_not_before_it() {
        let at = |grade, kind| WeatherState { grade, kind, ..Default::default() }.visibility();
        assert_eq!(at(0.0, WeatherKind::Snow), 1.0);
        assert_eq!(at(0.25, WeatherKind::Snow), 1.0, "under the floor nothing is in the air");
        assert!(at(0.6, WeatherKind::Snow) < at(0.3, WeatherKind::Snow));
        assert!((at(1.0, WeatherKind::Snow) - (1.0 - VISIBILITY_TAKEN_SNOW)).abs() < 1e-6);
        assert!((at(1.0, WeatherKind::Rain) - (1.0 - VISIBILITY_TAKEN)).abs() < 1e-6);
        assert!(
            at(1.0, WeatherKind::Snow) < at(1.0, WeatherKind::Rain),
            "a blizzard is a wall where a downpour is threads",
        );
    }

    /// **The far field is snow's alone**, and a far seed fills its own larger
    /// box — the mid-distance the near box leaves empty.
    #[test]
    fn snow_has_a_far_field_and_rain_does_not() {
        assert!(Layer::for_kind(WeatherKind::Snow).unwrap().far_count_full > 0);
        assert_eq!(Layer::for_kind(WeatherKind::Rain).unwrap().far_count_full, 0);
        assert_eq!(Layer::for_kind(WeatherKind::Storm).unwrap().far_count_full, 0);

        let far = (FAR_BOX_HALF_XY, FAR_BOX_HALF_Z);
        let seeds: Vec<Vec3> = (0..400).map(|i| seed_for(i, 0, far)).collect();
        assert!(
            seeds.iter().any(|s| s.x.abs() > BOX_HALF_XY || s.z.abs() > BOX_HALF_XY),
            "seeds reach beyond the near box",
        );
        for s in &seeds {
            assert!(s.x.abs() <= FAR_BOX_HALF_XY && s.y.abs() <= FAR_BOX_HALF_Z);
        }
    }

    /// The alive count walks toward the density a few a frame and never past
    /// it, up and down.
    #[test]
    fn the_alive_count_walks_toward_the_density() {
        let mut alive = 0;
        for _ in 0..400 {
            alive = walk_alive(alive, 200);
        }
        assert_eq!(alive, 200);
        let one_step = walk_alive(0, 200);
        assert!(one_step >= 4 && one_step < 200, "a few a frame: {one_step}");
        for _ in 0..400 {
            alive = walk_alive(alive, 50);
        }
        assert_eq!(alive, 50);
    }

    /// Rain's scatter is a few degrees where the mist keeps the measured
    /// twenty, which is the difference between strands and rain.
    #[test]
    fn rain_is_nearly_parallel_and_the_mist_keeps_its_spread() {
        let layer = Layer::for_kind(WeatherKind::Rain).unwrap();
        assert!(layer.spread < 0.1, "rain: {}", layer.spread);
        let mut rng = 11u32;
        for _ in 0..200 {
            let p = spawn(&mut rng, layer.fall, layer.size, layer.spread);
            let lean = (p.velocity.x.hypot(p.velocity.z) / -p.velocity.y).atan();
            assert!(lean <= layer.spread + 1e-4, "a drop leaned {lean} rad");
            assert!(p.velocity.y < 0.0, "rain falls");
            assert!(p.phase >= 0.0 && p.phase < 1.0);
        }
        let mist = spawn(&mut rng, layer.mist_fall, layer.mist_size, SPREAD);
        assert!(mist.size >= 1.2 && mist.size <= 5.0, "the reference's mist sizes");
    }

    /// **The seed is a hash, and it is the same hash the shader has**: the
    /// same index and cycle give the same point, a different cycle a
    /// different one, and every point is inside the box.
    #[test]
    fn a_seed_is_deterministic_per_cycle_and_inside_the_box() {
        let halves = (BOX_HALF_XY, BOX_HALF_Z);
        assert_eq!(seed_for(7, 3, halves), seed_for(7, 3, halves));
        assert_ne!(seed_for(7, 3, halves), seed_for(7, 4, halves), "a rebirth lands elsewhere");
        assert_ne!(seed_for(7, 3, halves), seed_for(8, 3, halves));
        for index in 0..500 {
            for cycle in 0..3 {
                let s = seed_for(index, cycle, halves);
                assert!(s.x.abs() <= BOX_HALF_XY && s.z.abs() <= BOX_HALF_XY && s.y.abs() <= BOX_HALF_Z);
            }
        }
        // The generator's first values, pinned so the shader's copy can be
        // checked against them by hand.
        assert_eq!(pcg(0), 0x7bb2fe2);
        assert_eq!(pcg(1), 0xa8beea3c);
    }

    /// **A particle stays where it is when the camera moves**, and re-enters
    /// on the opposite face when it leaves one: the drawn position is the
    /// world position wrapped into the box about the eye.
    #[test]
    fn a_particle_stays_where_it_is_when_the_camera_moves() {
        let p = Particle { velocity: Vec3::new(0.0, -2.0, 0.0), life: 10.0, phase: 0.0, size: 1.0 };
        let halves = (BOX_HALF_XY, BOX_HALF_Z);
        let eye = Vec3::new(100.0, 50.0, -30.0);
        let (rel, age) = particle_rel(&p, 3, 1.0, 1.0, Vec3::ZERO, eye, halves);
        assert!(age > 0.0 && age < 1.0);
        let world = eye + rel;
        // Step the eye a yard: the world position is the same point unless it
        // crossed a face, in which case it moved by exactly a box.
        let moved = eye + Vec3::new(1.0, 0.0, 0.0);
        let (rel2, _) = particle_rel(&p, 3, 1.0, 1.0, Vec3::ZERO, moved, halves);
        let world2 = moved + rel2;
        let delta = world2 - world;
        assert!(
            delta.length() < 1e-3 || (delta.x.abs() - 2.0 * BOX_HALF_XY).abs() < 1e-3,
            "the field slid with the camera: {delta}",
        );
        assert!(rel.x.abs() <= BOX_HALF_XY && rel.y.abs() <= BOX_HALF_Z && rel.z.abs() <= BOX_HALF_XY);
        // …and a second later it has fallen two yards, within its cycle.
        let (later, _) = particle_rel(&p, 3, 2.0, 1.0, Vec3::ZERO, eye, halves);
        let fell = (eye + later) - world;
        assert!((fell.y + 2.0).abs() < 1e-3 || (fell.y + 2.0).abs() > 2.0 * BOX_HALF_Z - 1e-3, "{fell}");
    }

    /// **The point sprite's floor reaches the shader as a parameter** — `MAX
    /// result.pointsize, 1` — in the slot a streak uses for its width, and
    /// the shader reads that slot rather than carrying a literal of its own.
    #[test]
    fn the_shader_floors_a_point_sprite_from_the_parameter() {
        let shader = include_str!("shaders/weather.wgsl");
        assert!(shader.contains("clamp(1.0 - distance / params.gust.w, 0.0, 1.0), params.shape.y)"));
        assert_eq!(POINT_MIN_PIXELS, 1.0, "`MAX result.pointsize, 1`");
    }

    /// `--weather`'s spellings: a kind with an optional grade, and nothing
    /// else, since a misspelling that quietly photographed the server's sky
    /// is the one failure the instrument is not allowed to have.
    #[test]
    fn a_forced_weather_parses_a_kind_and_a_grade() {
        let snow = ForcedWeather::parse("snow").expect("snow");
        assert_eq!(snow.0.kind, WeatherKind::Snow);
        assert_eq!(snow.0.grade, 1.0);
        assert!(snow.0.instant, "no ramp to wait out");
        let light = ForcedWeather::parse("Rain, 0.4").expect("rain");
        assert_eq!((light.0.kind, light.0.grade), (WeatherKind::Rain, 0.4));
        assert_eq!(ForcedWeather::parse("sand").unwrap().0.kind, WeatherKind::Storm);
        assert_eq!(ForcedWeather::parse("fine").unwrap().0.kind, WeatherKind::Fine);
        for bad in ["", "hail", "snow,2", "snow,x", ",0.5"] {
            assert_eq!(ForcedWeather::parse(bad), None, "{bad:?}");
        }
    }

    /// The mask packs its words four to a vec4, in order, and every pool the
    /// layer table names fits in it.
    #[test]
    fn the_mask_packs_in_order_and_holds_the_largest_pool() {
        let mask = pack_mask(&[1, 2, 3, 4, 5]);
        assert_eq!(mask[0], UVec4::new(1, 2, 3, 4));
        assert_eq!(mask[1], UVec4::new(5, 0, 0, 0));
        for kind in [WeatherKind::Rain, WeatherKind::Snow, WeatherKind::Storm] {
            let layer = Layer::for_kind(kind).unwrap();
            for count in [layer.count_full, layer.far_count_full, MIST_DRAWN] {
                assert!(count <= MASK_VEC4S * 128, "{kind:?}: {count} particles overflow the mask");
            }
        }
    }

    /// The mesh is four vertices a particle carrying its record, and never
    /// empty of attributes — the pipeline's layout needs all four.
    #[test]
    fn the_mesh_carries_the_record_and_the_corners() {
        let p = Particle { velocity: Vec3::new(0.5, -3.0, 0.0), life: 2.0, phase: 0.25, size: 7.0 };
        let mesh = build_mesh(&[p, p], (BOX_HALF_XY, BOX_HALF_Z));
        assert_eq!(mesh.count_vertices(), 8);
        // The positions span the box, for the bounds pass — see `build_mesh`.
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).expect("positions");
        assert_eq!(positions.len(), 8);
        let colors = mesh.attribute(Mesh::ATTRIBUTE_COLOR).expect("the record");
        assert_eq!(colors.len(), 8, "one record per vertex");
        // …and an empty pool still has a quad, since a mesh with no
        // vertices is a zero-byte allocation the render world rejects.
        let empty = empty();
        assert_eq!(empty.count_vertices(), 4);
        assert!(empty.attribute(Mesh::ATTRIBUTE_UV_0).is_some());
    }
}
