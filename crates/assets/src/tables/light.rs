//! `Light.dbc` -> `LightParams.dbc` -> `LightIntBand.dbc` + `LightFloatBand.dbc`
//! — **the world's light: the sun, the ambient, the sky and the fog, and the
//! colour of water, which is one of the same eighteen bands.**
//!
//! Everything global about how the world *reads* is in this chain, indexed by
//! map and by time of day. Nothing else in the game states any of it: a terrain
//! tile carries normals but no light, a WMO carries baked interior colours but
//! no sun, and no packet on the wire mentions either.
//!
//! This module was opened for water and the water half is written up below; the
//! rest of the chain is [`Atmosphere`], and the two share a row lookup, an
//! interpolator and a fallback.
//!
//! ## The water half, which is why the module exists at all
//!
//! It was opened for one reason, and it is a measurement: the four
//! liquid flipbooks carry no colour at all. `lake_a.1.blp` decodes to a peak
//! channel of **41 of 255 across the whole image**, grey, alpha-weighted mean
//! 7/7/7; `ocean_h` peaks at 82 and is the same shape. They are luminance masks
//! — foam and glints — and a surface drawn as `texel.rgb` blended over the canal
//! bed is a *black sheet at a third opacity*, which is what Stormwind's canals
//! were after the depth fix and is indistinguishable from a dirty shadow. Lava
//! and slime, which are the same code path, decode to 175/22/0 and 68/132/18 and
//! were visible the whole time — that contrast is the measurement.
//!
//! So the colour is the client's, per zone and per time of day, and 1.12 ships
//! it in the light chain:
//!
//! ```text
//! Light.dbc         id, mapId, x, z, y, falloffStart, falloffEnd, params[5]
//! LightParams.dbc   id, highlightSky, skyboxId, cloudType, glow,
//!                   waterShallowAlpha, waterDeepAlpha,
//!                   oceanShallowAlpha, oceanDeepAlpha
//! LightIntBand.dbc  id, entryCount, time[16], colour[16]
//! ```
//!
//! `LightIntBand` holds **18 bands per `LightParams`**, which is pinned by the
//! row counts rather than assumed: 426 `LightParams` x 18 = 7,668 `LightIntBand`
//! rows exactly, and band *b* of params *p* is row `(p - 1) * 18 + b + 1`.
//!
//! **The colour is packed red-first (`0x00RRGGBB`), and that is a measurement,
//! not a convention taken on trust.** Read the other way round, map 0's noon sun
//! (band 9) comes out pale blue and its sky top (band 2) comes out brown; read
//! red-first they are gold `228/217/179` and blue `37/44/56`. Two bands nobody
//! is arguing about, both only right one way.
//!
//! **Bands 13..16 are the water colours, and they are `close`/`far`, not
//! `shallow`/`deep`.** That distinction cost a wrong check: a shallow/deep pair
//! must darken, and surveyed across every default light the game ships, the
//! second of each pair is *brighter* in 15 of 19 — band 15 means `79/85/91` and
//! band 16 means `83/79/66`. They are the near and the far colour of the surface,
//! which is why the far one hazes toward the fog instead. Depth does not enter
//! the colour at all:
//!
//! ```text
//! band 13  ocean close   map 0 at noon  (97,130,183)   mean over 19 lights  76/118/145
//! band 14  ocean far                    (17, 75, 89)                       101/113/114
//! band 15  river close                  ( 0, 29, 41)                        79/ 85/ 91
//! band 16  river far                    (79, 93, 20)                        83/ 79/ 66
//! ```
//!
//! **Depth drives the alpha, and only the alpha** — between the two floats
//! `LightParams` states for this liquid. Map 0's four are 0.5 / 1.0 / 0.75 / 1.0:
//! a river bank is half transparent, the middle of the canal is solid, and both
//! numbers are the game's rather than this client's. That is the correction to
//! last round, which read `MLIQ`'s depth byte as an 8-bit alpha directly — right
//! that the shore should fade, wrong that a full canal should sit at a third
//! opacity over a colourless texture.

use crate::tables::dbc::Dbc;

/// The four liquids, as [`crate::world::wmo::Liquid`] resolves them.
use crate::world::wmo::Liquid;

/// How many int bands each `LightParams` row owns. See the module note: this is
/// `LightIntBand.record_count / LightParams.record_count` in 1.12 and is checked
/// against it at load, rather than trusted.
const BANDS_PER_PARAMS: u32 = 18;

/// How many *float* bands each `LightParams` row owns, checked the same way:
/// `LightFloatBand.record_count / LightParams.record_count`. 1.12 ships 2,556
/// against 426 params rows, which is 6 — a different number from the int bands'
/// 18, and assuming they matched would read one light's fog as another's.
const FLOAT_BANDS_PER_PARAMS: u32 = 6;

/// The eighteen int bands, of which this client reads the first eight and the
/// four water ones.
///
/// **`LightIntBand` names nothing — the band is its row's position — so every
/// one of these is pinned by measurement, printed by `vale light`.** Picking
/// the wrong index yields a *plausible* colour rather than a failure, which is
/// the trap this whole area keeps setting; each name below records what
/// separates it from its neighbours.
mod band {
    /// The sun: the colour of the single directional light the world is lit by.
    ///
    /// **Pinned by the clock rather than by one reading.** Map 0 holds
    /// `97/130/162` through the small hours — cool, dim, moonlight — and swings
    /// to `255/103/0` at 06:00 and `255/136/0` at noon. Nothing else in the
    /// table does that: [`AMBIENT`] below it never goes warm at all, and the sky
    /// bands move without ever leaving blue. A saturated orange looks wrong
    /// written down and is not: it arrives multiplied by `N·L` and added to a
    /// cool fill, which is exactly where this game's blue shadows come from.
    pub const DIFFUSE: u32 = 0;
    /// The fill, which is what every surface facing away from the sun is lit by.
    /// Cool where the sun is warm, and **darker than band 0 in 19 of 19
    /// lights** — a fill brighter than its own key does not exist.
    pub const AMBIENT: u32 = 1;
    /// The sky dome, zenith first, and **the pin is that the zenith is the
    /// darkest of the five**: map 0 at noon runs `0/31/73` at the top, then
    /// `58/162/207`, `153/220/245`, `175/218/224` and `180/180/180` — a deep
    /// blue overhead going pale at eye level, which is a sky lit from beyond the
    /// horizon and is what this game looks like. Read upside down it is a sky
    /// lit from directly above the observer, which nothing in the game is.
    ///
    /// The brightening is not monotonic all the way and the check must not ask
    /// it to be: 5 is a hair under 4 at map 0's noon, and over the 19 defaults
    /// [`SKY_SMOG`] is darker than the band above it in 18 and [`SKY_FOG`] in
    /// 17. Those last two are haze sitting *under* the sky rather than more of
    /// it.
    pub const SKY_TOP: u32 = 2;
    pub const SKY_MIDDLE: u32 = 3;
    pub const SKY_BAND_1: u32 = 4;
    pub const SKY_BAND_2: u32 = 5;
    pub const SKY_SMOG: u32 = 6;
    /// **The bottom of the dome and the colour the world fades into, one band
    /// used twice** — which is not an economy, it is the mechanism. Distant
    /// terrain disappears because it is fogged to exactly the colour the sky is
    /// drawn at the horizon behind it; any other value leaves a visible line
    /// where the ground ends.
    pub const SKY_FOG: u32 = 7;
    pub const OCEAN_CLOSE: u32 = 13;
    pub const OCEAN_FAR: u32 = 14;
    pub const RIVER_CLOSE: u32 = 15;
    pub const RIVER_FAR: u32 = 16;
    /// **The four water bands the shipped minimaps were rendered with**, one
    /// row up from the four above, read as a *shallow* and a *deep* colour
    /// mixed by `MCLQ`'s own depth byte.
    ///
    /// **Measured against the shipped pictures**, which are the retail
    /// renderer's own output and so the comparison the note on
    /// [`SHADOW`](Self::SHADOW) asks for. `vale bake <Map> <x> <y> minimap`
    /// reads the shipped picture back where the tile's `MCLQ` says the water
    /// is, binned by the depth byte at each pixel, and the mix that fits every
    /// band on every tile tried is
    /// `mix(band[shallow], band[deep], byte / 255)` over the ground at the
    /// float bands' own opacity ramp:
    ///
    /// * Menethil's ocean (`Azeroth_33_39`, bytes under 32) reads `41/73/66`
    ///   against band 14 of the Wetlands light, `28/73/78`, over the sea
    ///   floor at 0.77;
    /// * the open sea west of it (`Azeroth_30_39`, byte 255 over 57,696
    ///   pixels) reads `0/29/41`, which is band 15 of map 0's default light
    ///   exactly, and the bytes between read the mix — `7/40/50` at 160..254
    ///   against `7/40/50` predicted;
    /// * Elwynn's lake (`Azeroth_32_49`) reads `88/89/25` at bytes under 32
    ///   against band 16, `79/93/20`, over yellow-green ground, and
    ///   `73/87/45` at 96..159 against the mix toward band 17, `51/82/85`,
    ///   predicted `72/89/46`.
    ///
    /// So band 13 is not a water colour, and the four the client draws with
    /// are each one row low. **The client's own water is left on the four
    /// above** — that is a renderer change with a screen to check it on; these
    /// exist so the minimap can be drawn
    /// from the measured rows now. [`LightTables::liquid_by_depth_at`] reads
    /// them.
    pub const OCEAN_SHALLOW: u32 = 14;
    pub const OCEAN_DEEP: u32 = 15;
    pub const RIVER_SHALLOW: u32 = 16;
    pub const RIVER_DEEP: u32 = 17;
    /// **Unidentified, and read only so that `vale light` can keep printing
    /// it.** Nothing in the renderer consumes this band.
    ///
    /// The measurements stand and are worth keeping: it is **dimmer than the
    /// fill in 7 of the 7 open-sky lights** (`vale light` counts it beside
    /// the other four shape checks, and it misses in 3 of the 12 underground
    /// ones exactly as the sky checks do), it is cool where the sun is warm —
    /// map 0 at noon reads `51/82/85` against a fill of `104/130/154` and a sun
    /// of `255/136/0` — and it closes on the fill as the sun goes out, being
    /// half the fill's total at noon and nine tenths of it at midnight.
    ///
    /// **What was wrong was the conclusion drawn from them**, which was that
    /// this is the colour ground in a baked `MCSH` shadow is lit by instead of
    /// the sun. `Shaders\Pixel\terrain1.bls` — the client's own terrain
    /// fragment program, quoted in full in the renderer's `atmosphere.wgsl` —
    /// applies `MCSH` as a flat `shadow * 0.3 + 0.7` scalar over the whole
    /// modulated result. There is no shadow colour in the client's terrain path
    /// at all, so a band with the right *shape* for one was still the wrong
    /// answer.
    ///
    /// **The open question is now what row 17 actually is, and there is a
    /// candidate: river-far.** The canonical `LightIntBand` layout puts ocean
    /// at 14/15 and river at 16/17, one row above where the four water
    /// constants above sit — so either those four are shifted down by one and
    /// every water colour this client draws is off by a band, or the canonical
    /// layout is wrong here. Settling it wants the two mappings compared
    /// against the retail client on the same server, since both produce
    /// plausible water. Until then the four above are what `vale water`
    /// measured and this stays named for the shape it has rather than for a
    /// meaning nothing has established.
    pub const SHADOW: u32 = 17;
    /// The disc of the sun itself — 77/77/77 on map 0 at noon, 71/71/71 over
    /// the 19 defaults. **Read and carried, not yet drawn**: what consumes it
    /// is the sprite `Textures\sunCenter.blp`, which needs a *position over
    /// the day* that nothing in this chain states. See [`celestial`].
    pub const SUN_DISC: u32 = 8;
    /// Its halo — 255/247/222 on map 0 at noon, 228/217/179 over the defaults:
    /// the warm glow around the disc, `Textures\sunGlare.blp`. Carried on the
    /// same terms as [`SUN_DISC`].
    pub const SUN_HALO: u32 = 9;
}

// **Bands 10..12 are deliberately not read**: three cloud layers (map 0 at noon
// holds 255/199/138, 43/105/132 and 0/0/0), each a colour for a scrolling cloud
// sheet. A gradient dome consumes none of them and no cloud sheet has been found
// in the archives under any obvious name — `vale sky` lists what is there.
//
// Written down rather than left blank because their absence is the *reason*
// there are no clouds in the sky, and the round that adds them starts here.

/// The six float bands. Same row arithmetic as the int bands against
/// [`FLOAT_BANDS_PER_PARAMS`], and the same "the row position is the name".
mod float_band {
    /// How far away the world is fogged out — **not in yards**; see
    /// [`YARDS_PER_UNIT`].
    pub const FOG_END: u32 = 0;
    /// Where the fog *starts*, as a fraction of [`FOG_END`], so the two together
    /// are a ramp and not a wall.
    ///
    /// **It goes negative, and that is the table talking rather than a bad
    /// read**: map 0 holds 0.25 at noon and 0 or less at dawn, and Alterac
    /// Valley is negative all day. A start behind the camera means fog from the
    /// first yard, which is a morning mist and a snowbound battleground — both
    /// of which that zone is famous for. Clamped at zero on the way out, since
    /// there is nothing nearer than the camera.
    pub const FOG_START_SCALER: u32 = 1;
    /// Bands 2..5 hold 1.00, 0.50..0.65, 0.95 and 1.00 across all 19 lights —
    /// three of them never varying at all. They are the cloud and glow
    /// parameters of a sky this client does not draw, and their flatness is the
    /// evidence that 1.12 does not use them either.
    pub const _UNUSED: u32 = 2;
}

/// **What each of the eighteen int bands is**, by position, for a caller that
/// has to label a band it did not pick — a panel listing all eighteen, or a
/// report printing one.
///
/// The names are the [`band`] module's own, which is where each one's
/// measurement is written; this is that knowledge as data so nothing outside
/// this crate restates it. A band this client does not read still has a name
/// here: a row with no label is a row somebody edits by accident.
pub const INT_BAND_NAMES: [&str; BANDS_PER_PARAMS as usize] = [
    "Sun",
    "Ambient",
    "Sky zenith",
    "Sky middle",
    "Sky band 1",
    "Sky band 2",
    "Sky smog",
    "Sky horizon and fog",
    "Sun disc",
    "Sun halo",
    "Cloud layer 1",
    "Cloud layer 2",
    "Cloud layer 3",
    "Ocean close",
    "Ocean far",
    "River close",
    "River far",
    "Band 17",
];

/// …and the six float bands, on the same terms.
pub const FLOAT_BAND_NAMES: [&str; FLOAT_BANDS_PER_PARAMS as usize] = [
    "Fog end",
    "Fog start scaler",
    "Float band 2",
    "Float band 3",
    "Float band 4",
    "Float band 5",
];

/// **What is known about each int band beyond its name, and whether this
/// client reads it.**
///
/// Beside [`INT_BAND_NAMES`] rather than folded into it because the two are
/// asked for in different places: a form wants the label, and a report wants
/// the sentence. Empty where the name says everything.
///
/// The names and the notes are here rather than in the caller so that there
/// is one copy: `vale light` kept a second copy of this list, and it went stale — its note on band 17 still stated the
/// shadow-colour reading that [`band::SHADOW`] records as disproved by
/// `terrain1.bls`.
pub const INT_BAND_NOTES: [&str; BANDS_PER_PARAMS as usize] = [
    "The single directional light the world is lit by.",
    "The ambient fill. Darker than the sun in all 19 default lights.",
    "",
    "",
    "",
    "",
    "Haze below the sky, separate from the sky gradient.",
    "The horizon colour and the fog colour: one band used for both.",
    "Read but not drawn: the renderer has no sun sprite yet.",
    "Read but not drawn: the renderer has no sun glare sprite yet.",
    "Not read: no cloud sheet has been found in the archives.",
    "Not read: no cloud sheet has been found in the archives.",
    "Not read: no cloud sheet has been found in the archives.",
    "",
    "",
    "",
    "",
    "Unidentified. Dimmer than the ambient band in all 7 open-sky lights and \
     cool where the sun is warm. It is not a shadow colour: `terrain1.bls` \
     applies MCSH as a flat scalar. It may be the far river colour; see \
     `band::SHADOW`.",
];

/// …and the same for the six float bands.
pub const FLOAT_BAND_NOTES: [&str; FLOAT_BANDS_PER_PARAMS as usize] = [
    "The distance at which the fog is complete, in 1/36 of a yard.",
    "Where the fog starts, as a fraction of the fog end. A negative value \
     starts the fog at the camera.",
    "Not read: flat across all 19 default lights.",
    "Not read: flat across all 19 default lights.",
    "Not read: flat across all 19 default lights.",
    "Not read: flat across all 19 default lights.",
];

/// **Which `LightIntBand` row holds band `band` of `LightParams` row `params`.**
///
/// The band tables carry no reference column: a band is found by where its row
/// sits. `(params - 1) * 18 + band + 1`, in record numbers — which is a 1-based
/// row id, the same number the id column carries, because the shipped tables
/// are dense and in order.
///
/// `None` for a band past the eighteen, or for params row 0, which no light
/// names.
pub fn int_band_row(params: u32, band: u32) -> Option<u32> {
    if params == 0 || band >= BANDS_PER_PARAMS {
        return None;
    }
    Some((params - 1) * BANDS_PER_PARAMS + band + 1)
}

/// …and the same for the six float bands, against
/// [`FLOAT_BANDS_PER_PARAMS`].
///
/// A separate function rather than an argument because the two counts differ —
/// 18 against 6 — and sharing one would read one light's fog as another's, which
/// is the failure this whole chain keeps setting up.
pub fn float_band_row(params: u32, band: u32) -> Option<u32> {
    if params == 0 || band >= FLOAT_BANDS_PER_PARAMS {
        return None;
    }
    Some((params - 1) * FLOAT_BANDS_PER_PARAMS + band + 1)
}

/// The inverse of [`int_band_row`]: which `(params, band)` a row id is.
///
/// For a caller that has a row in hand and has to say what it is.
pub fn int_band_of(row: u32) -> Option<(u32, u32)> {
    if row == 0 {
        return None;
    }
    Some(((row - 1) / BANDS_PER_PARAMS + 1, (row - 1) % BANDS_PER_PARAMS))
}

/// …and of [`float_band_row`].
pub fn float_band_of(row: u32) -> Option<(u32, u32)> {
    if row == 0 {
        return None;
    }
    Some((
        (row - 1) / FLOAT_BANDS_PER_PARAMS + 1,
        (row - 1) % FLOAT_BANDS_PER_PARAMS,
    ))
}

/// **How many of a band row's sixteen key slots are live**, and how to read the
/// tail.
///
/// The slots past `EntryCount` hold whatever was in the authoring tool's
/// memory — `0xCCCCCCCC` in 101,116 of `LightIntBand`'s unused time slots — so
/// a reader must stop at the count and a writer must leave them where they are.
pub const BAND_KEYS: usize = 16;

/// Where a band row's times start, and where its values start.
///
/// `id, entryCount, time[16], value[16]` — pinned by every one of the 17,535
/// live entries of `LightIntBand` falling inside 0..[`DAY`], which the values
/// do not.
pub const BAND_TIME_FIELD: usize = 2;
/// See [`BAND_TIME_FIELD`].
pub const BAND_VALUE_FIELD: usize = 2 + BAND_KEYS;

/// **The float table's distances are in 1/36 of a yard**, which is the same
/// unit `Light.dbc`'s own coordinates are stored in one table up — a convention
/// that belongs to this chain rather than to either file.
///
/// Pinned by what it makes of the nineteen lights, because the raw numbers name
/// no unit: map 0's `fogEnd` is 18,000, which is four times the width of the
/// continent as yards and **500 yards** divided by 36. Read that way the whole
/// table becomes a list anyone who has played the game can check — Scarlet
/// Monastery fogs out at 111 yards, Onyxia's Lair at 167, Dire Maul at 444, and
/// Alterac Valley, alone and outdoors, at 888. Read as yards they are all
/// between four and thirty times the size of the zone they belong to.
pub const YARDS_PER_UNIT: f32 = 1.0 / 36.0;

/// `LightParams` fields. 0..4 are the id, the sky flags and the glow; 5..8 are
/// the four water alphas, and they are the reason this table is read at all.
pub mod params_field {
    pub const WATER_SHALLOW_ALPHA: usize = 5;
    pub const WATER_DEEP_ALPHA: usize = 6;
    pub const OCEAN_SHALLOW_ALPHA: usize = 7;
    pub const OCEAN_DEEP_ALPHA: usize = 8;
}

/// `Light` fields: id, map, three coordinates, two falloff radii, then the five
/// `LightParams` ids — clear, clear-underwater, storm, storm-underwater, death.
///
/// **The three coordinates are in the same "internal representation" every
/// `MDDF` and `MODF` placement in the game is in** — `(x = westward, y = up,
/// z = northward)`, measured from the map's corner — only scaled by 36 like
/// every other distance in this chain. So they are undone by the function that
/// already exists for placements, [`crate::world::adt::placement_to_world`], whose own
/// note says where the convention comes from and what it was checked against.
///
/// That was not obvious and is the measurement this round bought; see
/// [`LightTables::positional`] and the note on [`WORLD_CORNER`].
pub mod light_field {
    pub const MAP: usize = 1;
    /// Internal x, westward from the corner — which yields the world's **y**.
    pub const INTERNAL_X: usize = 2;
    /// Internal y, which is the height, and the one column that is a plain
    /// distance rather than an offset from anything.
    pub const INTERNAL_Y: usize = 3;
    /// Internal z, northward from the corner — the world's **x**.
    pub const INTERNAL_Z: usize = 4;
    pub const FALLOFF_START: usize = 5;
    pub const FALLOFF_END: usize = 6;
    /// Fair weather above the surface — the ordinary one.
    pub const PARAMS_CLEAR: usize = 7;
    /// **…and the same weather seen from under the water**, which is the whole
    /// of what the reference does about being submerged: a second
    /// `LightParams` row per light, with its own eighteen bands and its own two
    /// fog distances. The dome band becomes the colour of the water, the fog
    /// end drops to a few dozen yards, and the sun is what is left of it.
    ///
    /// **Nothing about this is a screen tint or a post effect.** The client has
    /// neither; it changes which light row is in force and the world is drawn
    /// the same way it always was. See [`LightTables::atmosphere_in`].
    ///
    /// The last of the five — death — is the ghost world, which this client
    /// does not have. The two between are read: see [`PARAMS_STORM`].
    pub const PARAMS_CLEAR_UNDERWATER: usize = 8;
    /// **The same light with the weather up**, which is the whole of what the
    /// sky does when `SMSG_WEATHER` says it is raining.
    ///
    /// A storm row is a darker sun over a colder fill under a dome that has
    /// lost its blue, and its fog is pulled in to a fraction of the clear row's
    /// — `vale weather` prints both rows of every map side by side, which is
    /// where the numbers behind that sentence are. **Nothing else changes**:
    /// there is no darkening pass and no grey quad over the frame, exactly as
    /// there is none for being underwater.
    pub const PARAMS_STORM: usize = 9;
    /// …and the same again from under the water, since a lake in a downpour is
    /// two switches, not one. Falls back to the clear-underwater row and then
    /// to the clear one — see [`super::Weather::of`].
    pub const PARAMS_STORM_UNDERWATER: usize = 10;
}

/// Which of the five weather columns a light is being asked for.
///
/// Four of the five: the two the camera decides — above the water and under it
/// — crossed with the two the server decides, which is what `SMSG_WEATHER`
/// says. The fifth is death and this client has no ghost world. See
/// [`light_field::PARAMS_CLEAR_UNDERWATER`] and [`light_field::PARAMS_STORM`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Weather {
    #[default]
    Clear,
    Underwater,
    Storm,
    StormUnderwater,
}

impl Weather {
    /// The clear or the storm face of this weather, which is the pair a
    /// **blend** runs between: rain does not switch the light over, it moves it
    /// — see [`LightTables::atmosphere_in_storm`].
    pub fn dry(self) -> (Weather, Weather) {
        match self {
            Weather::Clear | Weather::Storm => (Weather::Clear, Weather::Storm),
            Weather::Underwater | Weather::StormUnderwater => {
                (Weather::Underwater, Weather::StormUnderwater)
            }
        }
    }

    /// Pick this weather's id out of a light row's four.
    ///
    /// `[clear, clear-underwater, storm, storm-underwater]`, in the column
    /// order `Light.dbc` states them.
    fn of(self, ids: [u32; 4]) -> u32 {
        match self {
            Weather::Clear => ids[0],
            // **A row that names no underwater light falls back to its own
            // clear one**, which is the honest degradation rather than a
            // guessed tint: the shipped rows do fill this column, and a
            // hand-made or patched one that does not should look like the
            // surface rather than like nothing.
            Weather::Underwater if ids[1] != 0 => ids[1],
            Weather::Underwater => ids[0],
            // …and the same rule one column along. A light with no storm row
            // is a zone whose sky does not change when it rains, which is what
            // the reference draws for it too — not a zone this client invents a
            // darkening for.
            Weather::Storm if ids[2] != 0 => ids[2],
            Weather::Storm => Weather::Clear.of(ids),
            Weather::StormUnderwater if ids[3] != 0 => ids[3],
            Weather::StormUnderwater => Weather::Underwater.of(ids),
        }
    }
}

/// **Where `Light.dbc` measures its positions from** — the corner of the 64x64
/// ADT grid, which is [`crate::world::adt::MAP_ORIGIN`] and is re-exported here only
/// so this module's own note has something to point at.
///
/// **Measured, and against the server rather than against a convention.** The
/// unit was already pinned by the float bands ([`YARDS_PER_UNIT`]); what was
/// open was the origin and which column was which, and neither is stated
/// anywhere. Four readings of the three floats were scored by taking every
/// positional row on maps 0 and 1 and asking how near the nearest named place
/// in vmangos' own `game_tele` table falls — **186 of 355** rows land with a
/// named tele point inside their own falloff sphere under this one, against
/// 66, 73 and 76 for the three alternatives.
///
/// The count is the weaker half of that; the **names** are the measurement.
/// Under this reading light 3 lands 20 yards from `SwampOfSorrows`, light 5
/// 42 from `DeadwindPass`, light 39 — a 152..229 yard sphere — 71 from
/// `BootyBay`, lights 51 and 52 — 63..90 yards each — 52 and 54 from
/// `DwarvenDistrict`, light 62 19 from `PlaguewoodTower` and light 61 **7**
/// from `RavenholdtManor`. A wrong corner or a transposed pair does not put
/// forty small spheres on top of forty named landmarks.
///
/// And the reading it arrived at is not a new one: it is `placement_to_world`
/// exactly, which vmangos' own `convertPositionToInternalRep` pins. Two
/// unrelated methods, one answer — which is worth more than either, because
/// the corroborating one is *source* rather than a fit.
pub use crate::world::adt::MAP_ORIGIN as WORLD_CORNER;

/// A day is 2,880 half-minutes, which is the unit `LightIntBand`'s times are in.
pub const DAY: u32 = 2880;

/// **Noon, and it is a placeholder standing in for a clock.**
///
/// The light bands are a function of time of day and this client has no day/night
/// cycle — `SMSG_LOGIN_SETTIMESPEED` carries the server's clock and is not read
/// yet. Noon is the framing every screenshot in this repo was taken at, so it is
/// the honest fixed point; when the clock arrives this constant becomes an
/// argument and nothing else changes, because [`LightTables::liquid`] already
/// interpolates.
pub const NOON: u32 = DAY / 2;

/// **Where the sun stands** — a unit vector pointing from the world toward the
/// sun, in the world's own axes (+X north, +Y west, +Z up).
///
/// `Light.dbc` states the sun's colour at every hour and its direction at
/// none. What 1.12 *does* ship is the answer already
/// applied: every terrain chunk's `MCSH` is the shadow baked for a specific
/// sun, so the displacement between the tall doodads and the shadows they cast
/// is the azimuth, and height against reach is the elevation. `vale sun` is
/// the instrument, and it prints its measurement against this constant so the
/// two cannot drift silently.
///
/// Measured: azimuth 40 degrees (from +X north toward +Y west), elevation 40
/// degrees. Five tiles discriminate — Westfall's two say 39.0 and 38.0 deg
/// (18%/87% of predicted shadow present against 6%/45% for the opposite
/// azimuth), Durotar's says 49.5 deg at 70% against 12%, Duskwood 32.0 and the
/// southern Barrens 43.5 with weaker contrast — and the elevations of the
/// clean sparse tiles land 38..42. The old guessed angle stood at azimuth 225,
/// which put the lit side of every wall against the baked shadow beside it.
///
/// **The client agrees on the bearing and disagrees on the height.** It aims
/// its own lighting sun on a constant azimuth of 225° — the direction light *travels*,
/// so a sun at 40°'s neighbour, 45° — and sweeps its elevation 20°..37° over
/// the day ([`celestial::LIGHT_POLAR`]). So the fixed direction here is right
/// about where the sun is and about ten degrees low-slung against the client's
/// own noon.
///
/// **This is the bakes' sun, not the renderer's.** A Direct3D trace of the
/// reference measured the direction it sets on its one `D3DLIGHT9` — `(-0.6625,
/// -0.6625, -0.3497)` at 17:50, azimuth 225° and 20.5° up, which is the track
/// to half a degree — so the renderer aims its light by
/// [`celestial::light_toward`] and follows the hour. What stays on this
/// constant is everything that has to agree with the shipped `MCSH` and the
/// shipped minimaps: `vale sun`, the shadow bake and the minimap bake.
pub const SUN_TOWARD: [f32; 3] = [0.587, 0.492, 0.643];

/// **What the game draws *in* the sky, and when** — the stars, the sun's disc
/// and the moon's, on the client's own day-fraction curves.
///
/// Nothing in `Light.dbc` says any of this: the table states colours at an hour
/// and has no opinion about whether the star dome is up. The 1.12 client hard-
/// codes it, and this module is those constants as the client has them rather
/// than reasoned about — which matters, because a
/// star field faded on a plausible curve looks exactly like one faded on the
/// right curve until the hour it does not.
///
/// **How the curves work.** The star dome (`Environments\Stars\stars.mdl`),
/// the sun and moon sprites (`sunCenter.blp`, `moon.blp`, `moon02.blp`) and
/// the two glares each have a little four-key table of `(day fraction, value)`
/// pairs, and one evaluator they share: clamp `t` to 0..1,
/// find the first key at or after it, wrap from the last key round to the
/// first, and interpolate **linearly**. Each frame the client evaluates the
/// star dome's track at the hour and stores `value * 254 + 1` as a byte, and
/// does not draw the dome at all below 2. So the fade is a byte and a star
/// dome at under ~0.4% is simply skipped.
///
/// **Where they stand over the day.** The per-frame
/// celestial update evaluates a *polar* track and an *azimuth* track per
/// body with the same evaluator, turns the pair into a unit vector
/// with `(sin phi cos theta, sin phi sin theta, cos phi)`, scales it by
/// [`CELESTIAL_RADIUS`] and adds the camera's position. Three bodies come out of
/// it — [`SUN`], [`MOON`] and [`BLUE_MOON`] — and each carries a third track
/// that scales its sprite. The same shape again serves the
/// *lighting* sun, which is a different vector and is [`LIGHT_POLAR`].
pub mod celestial {
    /// **How far from the camera the sun and the moons are drawn**, in yards.
    ///
    /// Twelve. Not a mistake and not a unit confusion: the client draws its sky
    /// objects in a pass that is behind the world whatever their distance, so
    /// the radius only ever sets how the sprite's own size in yards turns into
    /// an angle on screen. Anything else about it would be invisible.
    pub const CELESTIAL_RADIUS: f32 = 12.0;

    /// The client's own π, so that a track reads as the fraction the client
    /// multiplies rather than as a decimal nobody can check against it.
    const PI: f32 = std::f32::consts::PI;

    /// One of the client's little day curves: `(day fraction, value)` pairs,
    /// ascending, evaluated with wrap-around.
    ///
    /// The x axis is a **fraction of a day**, which is what the client's own
    /// constants are in — 0.2291667 is 05:30 — rather than the half-minutes
    /// [`super::DAY`] counts. [`DayTrack::at`] takes the half-minutes and does
    /// the division, so nothing outside this module has to know.
    pub struct DayTrack {
        pub keys: &'static [(f32, f32)],
    }

    impl DayTrack {
        /// The curve's value at an hour, in half-minutes past midnight.
        ///
        /// The client's evaluator verbatim, wrap included: a time before the first key or
        /// after the last interpolates across midnight between the last and the
        /// first, which is exactly the case that matters — every one of these
        /// tracks is *about* the night.
        pub fn at(&self, half_minutes: u32) -> f32 {
            self.at_fraction((half_minutes % super::DAY) as f32 / super::DAY as f32)
        }

        /// The same, at a day fraction directly.
        ///
        /// Public because one body's clock is **not** the day: [`BLUE_MOON`]
        /// runs on a [`BLUE_MOON_PERIOD_DAYS`]-day cycle, so its tracks are
        /// sampled at a fraction that has already been through that division
        /// and cannot be expressed as half-minutes past midnight.
        pub fn at_fraction(&self, t: f32) -> f32 {
            if self.keys.is_empty() {
                return 0.0;
            }
            let t = t.clamp(0.0, 1.0);
            let last = self.keys.len() - 1;
            // The first key at or after `t`; past the end wraps to the first.
            let next = self.keys.iter().position(|(at, _)| t <= *at).unwrap_or(0);
            let prev = if next == 0 { last } else { next - 1 };
            let mut span = self.keys[next].0 - self.keys[prev].0;
            // The client's own epsilon: two keys at the same instant are a step,
            // and dividing by that span is a division by zero.
            if span.abs() < 0.001 {
                return self.keys[prev].1;
            }
            if span < 0.0 {
                span += 1.0;
            }
            let mut into = t - self.keys[prev].0;
            if into < 0.0 {
                into += 1.0;
            }
            let f = (into / span).clamp(0.0, 1.0);
            self.keys[prev].1 + (self.keys[next].1 - self.keys[prev].1) * f
        }
    }

    /// **The star dome's opacity over the day**.
    ///
    /// Full from midnight, holding until **03:00**, out by **04:30**, dark all
    /// day, and fading back in from **22:30** to midnight. The four times are
    /// built in the client as two anchors either side of a pair of offsets —
    /// `0.2291667` (05:30) minus 2.5 h and 1 h, and `0.8958333` (21:30) plus
    /// the same two — which is why they are not round numbers.
    ///
    /// The consequence worth stating before anyone judges a screenshot by it:
    /// at 22:36 this reads **0.07**, so the real client's own sky is very
    /// nearly starless at that hour too.
    pub const STARS: DayTrack = DayTrack {
        keys: &[
            (0.125, 1.0),      // 03:00
            (0.1875, 0.0),     // 04:30
            (0.9375, 0.0),     // 22:30
            (1.0, 1.0),        // 24:00
        ],
    };

    /// The sun's glare: dark before **06:30**, full from
    /// **07:30** to **19:30**, out again by **21:00**. Carried for the round
    /// that draws the disc; see this module's note on why that is not this one.
    pub const SUN_GLARE: DayTrack = DayTrack {
        keys: &[
            (0.2708333, 0.0),  // 06:30
            (0.3125, 1.0),     // 07:30
            (0.8125, 1.0),     // 19:30
            (0.875, 0.0),      // 21:00
        ],
    };

    /// The moon's, and it is the sun's turned inside out: full
    /// at **02:00**, gone by **03:15**, dark all day, back at **23:59**.
    pub const MOON_GLARE: DayTrack = DayTrack {
        keys: &[
            (0.0833333, 1.0),  // 02:00
            (0.1354167, 0.0),  // 03:15
            (0.9479167, 0.0),  // 22:45
            (0.9993056, 1.0),  // 23:59
        ],
    };

    /// **Below this the client does not draw the star dome at all**, and it is
    /// a byte rather than a fraction because that is how the client stores it:
    /// it computes `value * 254 + 1` into a byte and skips the draw on
    /// anything under 2.
    ///
    /// Kept because it is the difference between a dome that is merely
    /// invisible and one that costs nothing — at noon this is every frame.
    pub const MIN_STAR_BYTE: u32 = 2;

    /// The star dome's opacity as the byte the client keeps it in, or `None`
    /// when it is under [`MIN_STAR_BYTE`] and the dome is skipped.
    pub fn star_byte(half_minutes: u32) -> Option<u32> {
        let byte = (STARS.at(half_minutes) * 254.0 + 1.0) as u32;
        (byte >= MIN_STAR_BYTE).then_some(byte)
    }

    /// The star dome itself. `LightSkybox.dbc` never names it — it is the sky
    /// every zone gets that names no skybox of its own, and the client loads it
    /// by this path (as `.mdx`; the archives hold the `.m2`).
    pub const STAR_MODEL: &str = r"Environments\Stars\Stars.m2";

    // ------------------------------------------- the three things that move --

    /// **One thing standing in the sky**: where it is, how big its sprite is,
    /// and what that sprite is.
    ///
    /// The three the client has are [`SUN`], [`MOON`] and [`BLUE_MOON`], and
    /// they are the same six fields with different keys — which is the whole
    /// reason this is a struct rather than nine loose constants. The client
    /// runs the identical update over each of them.
    pub struct Body {
        /// The **polar** angle from straight up, in radians: 0 is the zenith and
        /// π/2 the horizon, so anything over π/2 is below it. This is the
        /// client's own convention, not an elevation — see [`Body::direction`].
        pub polar: DayTrack,
        /// The **azimuth**, in radians, measured from world +X (north) toward
        /// +Y (west). Two of the three never move in it.
        pub azimuth: DayTrack,
        /// What the sprite's size is multiplied by over the day. Both moving
        /// bodies swell near the horizon, which is the client doing by hand what
        /// the eye does by itself.
        pub scale: DayTrack,
        /// And the constant it is multiplied by on top, which is what makes the
        /// moon bigger than the sun.
        pub base_scale: f32,
        /// The sprite.
        pub texture: &'static str,
    }

    impl Body {
        /// Where the body stands, as a unit vector in the **world's own axes**
        /// (+X north, +Y west, +Z up) — the same frame [`super::SUN_TOWARD`] is
        /// in, and the same one every placement in the game uses.
        ///
        /// The client's own formula: `(sin φ cos θ, sin φ sin θ, cos φ)`. The client
        /// then normalises the result and multiplies by
        /// [`CELESTIAL_RADIUS`] — the normalise is a no-op on a vector built
        /// this way and is not reproduced.
        pub fn direction(&self, t: f32) -> [f32; 3] {
            let polar = self.polar.at_fraction(t);
            let azimuth = self.azimuth.at_fraction(t);
            [
                polar.sin() * azimuth.cos(),
                polar.sin() * azimuth.sin(),
                polar.cos(),
            ]
        }

        /// How high above the horizon it stands, in degrees — negative below.
        ///
        /// Not the client's own quantity (it works in the polar angle
        /// throughout) but the one every check and every HUD line wants, and
        /// deriving it in one place is what stops `90 - φ` being written three
        /// times with one of them the wrong way round.
        pub fn elevation(&self, t: f32) -> f32 {
            90.0 - self.polar.at_fraction(t).to_degrees()
        }

        /// The sprite's size multiplier at `t`.
        pub fn size(&self, t: f32) -> f32 {
            self.base_scale * self.scale.at_fraction(t)
        }
    }

    /// A day fraction from half-minutes past midnight, which is what
    /// [`Body`]'s three methods take.
    pub fn day_fraction(half_minutes: u32) -> f32 {
        (half_minutes % super::DAY) as f32 / super::DAY as f32
    }

    /// **The sun**, from the client's tracks: polar (five keys), azimuth
    /// (three) and scale (four).
    ///
    /// It rises at **05:30** and sets at **21:30**, and in between it climbs to
    /// within five degrees of the zenith at noon — the three keys at 11:55,
    /// 12:00 and 12:05 are a *plateau* rather than a spike, which is what stops
    /// it snapping through the top of the sky. Outside that stretch the
    /// evaluator wraps between the last key and the first, both of which are
    /// 100°, so it sits **ten degrees under the horizon all night** rather than
    /// being hidden: the client draws it and lets the ground occlude it.
    ///
    /// **Its azimuth never changes.** It goes up and comes back down on one
    /// bearing, 45° — north-west in the world's axes — which is the same
    /// bearing [`LIGHT_AZIMUTH`] points the lighting sun from, and within five
    /// degrees of the 40° [`super::SUN_TOWARD`] was measured at off the ground's
    /// own `MCSH` bakes. Three unrelated methods, one answer.
    pub const SUN: Body = Body {
        polar: DayTrack {
            keys: &[
                (0.2291667, PI * 0.5555556), // 05:30, 100° — ten under the horizon
                (0.4965278, PI * 0.0277778), // 11:55,   5°
                (0.5, PI * 0.0277778),       // 12:00,   5°
                (0.5034722, PI * 0.0277778), // 12:05,   5°
                (0.8958333, PI * 0.5555556), // 21:30, 100°
            ],
        },
        azimuth: DayTrack {
            keys: &[
                (0.2291667, PI * 0.25), // 05:30, 45°
                (0.5, PI * 0.25),       // 12:00, 45°
                (0.8958333, PI * 0.25), // 21:30, 45°
            ],
        },
        scale: DayTrack {
            keys: &[
                (0.25, 2.0),     // 06:00 — twice the size at the horizon
                (0.28125, 1.0),  // 06:45
                (0.84375, 1.0),  // 20:15
                (0.875, 2.0),    // 21:00
            ],
        },
        base_scale: 1.0,
        texture: r"Textures\sunCenter.blp",
    };

    /// **The moon** — the sun's shape inverted.
    ///
    /// Up from **22:00** to **04:00**, highest at midnight at 55° and under the
    /// horizon through the working day. Its azimuth is the sun's, 45°, and it is
    /// **1.75 times the size** — the one place the two bodies differ by a
    /// constant rather than by their keys.
    pub const MOON: Body = Body {
        polar: DayTrack {
            keys: &[
                (0.0, PI * 0.1944444),         // 00:00, 35°
                (0.0034722, PI * 0.1944444),   // 00:05, 35°
                (0.1666667, PI * 0.5555556),   // 04:00, 100°
                (0.9166667, PI * 0.5555556),   // 22:00, 100°
                (0.9965278, PI * 0.1944444),   // 23:55, 35°
            ],
        },
        azimuth: DayTrack {
            keys: &[
                (0.0, PI * 0.25),       // 00:00, 45°
                (0.1666667, PI * 0.25), // 04:00, 45°
                (0.9166667, PI * 0.25), // 22:00, 45°
            ],
        },
        scale: DayTrack {
            keys: &[
                (0.0416667, 1.0),  // 01:00
                (0.1666667, 1.5),  // 04:00 — half again at the horizon
                (0.9166667, 1.5),  // 22:00
                (0.9993056, 1.0),  // 23:59
            ],
        },
        base_scale: 1.75,
        texture: r"Textures\moon.blp",
    };

    /// **The second moon**, and it is a different body rather than a phase of
    /// the first — `Textures\moon02.blp`, on its own azimuth track.
    ///
    /// It climbs the same arc as [`MOON`] but **drifts across the sky as it
    /// goes**, 135° to 165°, which makes it the only thing in 1.12's sky whose
    /// bearing moves at all. And its clock is not the day: the client samples
    /// its tracks at `frac((day + t) / 1.7)`, so it is a day and a half out of
    /// step with everything else — see [`BLUE_MOON_PERIOD_DAYS`] and
    /// [`blue_moon_fraction`].
    pub const BLUE_MOON: Body = Body {
        polar: DayTrack {
            keys: &[
                (0.0, PI * 0.1944444),        // 35°
                (0.0034722, PI * 0.1944444),  // 35°
                (0.1666667, PI * 0.5555556),  // 100°
                (0.9166667, PI * 0.5555556),  // 100°
                (0.9965278, PI * 0.1944444),  // 35°
            ],
        },
        azimuth: DayTrack {
            keys: &[
                (0.0, PI * 0.75),             // 135°
                (0.1666667, PI * 0.8333333),  // 150°
                (0.9166667, PI * 0.9166667),  // 165°
            ],
        },
        scale: DayTrack {
            keys: &[
                (0.0416667, 1.0),
                (0.1666667, 1.5),
                (0.9166667, 1.5),
                (0.9993056, 1.0),
            ],
        },
        base_scale: 1.0,
        texture: r"Textures\moon02.blp",
    };

    /// How long [`BLUE_MOON`]'s own cycle is, in days — 1.7.
    pub const BLUE_MOON_PERIOD_DAYS: f32 = 1.7;

    /// [`BLUE_MOON`]'s clock: where it is in its own cycle, given how many whole
    /// days have passed and how far through the current one it is.
    ///
    /// The client does this in 16.16 fixed point to keep the fraction exact
    /// across a long session; `f32` is reproduced here because a moon a
    /// thousandth of a cycle out is a moon nobody can measure. The day counter
    /// is the client's own, which this client does not have a
    /// source for yet — passing 0 puts the blue moon at the start of its cycle
    /// and it still moves correctly over a day, which is the visible half.
    pub fn blue_moon_fraction(days: u32, half_minutes: u32) -> f32 {
        let t = days as f32 + day_fraction(half_minutes);
        let cycles = t / BLUE_MOON_PERIOD_DAYS;
        cycles - cycles.floor()
    }

    /// **The lighting sun's own polar track** — the same shape, for a vector
    /// that is not any of the three above.
    ///
    /// 127° at midnight and at noon, 110° at 06:00 and 18:00, on a constant
    /// azimuth of [`LIGHT_AZIMUTH`]. Read as the direction light *travels* that
    /// is a sun standing **20° to 37° above the horizon, never setting** —
    /// night is done entirely with colour in this game.
    ///
    /// **Wired, through [`light_toward`].** It was read and left unwired for
    /// several rounds on the argument that a light swinging 17° over a day
    /// against shadows baked for one angle is a worse frame than a fixed one
    /// that agrees with them. A Direct3D trace of the reference then measured
    /// its light travelling `(-0.6625, -0.6625, -0.3497)` at 17:50 — this track
    /// to half a degree, over the same baked shadows — so the argument was
    /// about a frame the reference does not draw. [`super::SUN_TOWARD`] is
    /// what the bakes are checked against and nothing else.
    pub const LIGHT_POLAR: DayTrack = DayTrack {
        keys: &[
            (0.0, PI * 0.7055556),  // 00:00, 127°
            (0.25, PI * 0.6111111), // 06:00, 110°
            (0.5, PI * 0.7055556),  // 12:00, 127°
            (0.75, PI * 0.6111111), // 18:00, 110°
        ],
    };

    /// And its azimuth: π × 1.25, a constant **225°**. That is
    /// the direction the light travels, so the sun it implies is at 45° — the
    /// same bearing [`SUN`] rises and sets on.
    pub const LIGHT_AZIMUTH: f32 = PI * 1.25;

    /// **Where the lighting sun stands at an hour**, as a unit vector *toward*
    /// it in the world's own axes (+X north, +Y west, +Z up) — the frame
    /// [`super::SUN_TOWARD`] is in and the one a `DirectionalLight` is aimed
    /// from.
    ///
    /// The client's vector is the direction light *travels*, `(sin φ cos θ,
    /// sin φ sin θ, cos φ)` on [`LIGHT_POLAR`] and [`LIGHT_AZIMUTH`]; this is
    /// its negation. At 17:50 it is `(0.662, 0.662, 0.350)`, and the reference
    /// sets `(-0.6625, -0.6625, -0.3497)` on its `D3DLIGHT9` at that hour —
    /// which is the reason this exists.
    pub fn light_toward(half_minutes: u32) -> [f32; 3] {
        let polar = LIGHT_POLAR.at(half_minutes);
        let azimuth = LIGHT_AZIMUTH;
        [
            -(polar.sin() * azimuth.cos()),
            -(polar.sin() * azimuth.sin()),
            -polar.cos(),
        ]
    }
}

/// How many stops the sky dome has: bands 2..7, zenith to horizon.
pub const SKY_STOPS: usize = 6;

/// **Where each of those six stops sits on the dome**, as the sine of the
/// altitude above the horizon — 1.0 straight up, 0.0 at eye level, negative
/// below.
///
/// **This is the one number in the light chain that is not out of a file, and it
/// is written here rather than in the renderer so that it is one list to
/// correct.** `LightIntBand` gives six colours and no angles; the real client
/// draws them on a dome whose ring heights are generated in code, and no string,
/// table or constant in the client is known to name them. So the *ordering*
/// is the table's and the *spacing* is this client's.
///
/// What pins the two ends is not taste:
///
/// * the last stop **is** [`Atmosphere::fog`] — one band read twice (see
///   [`band::SKY_FOG`]) — so it has to sit at altitude 0, where the fogged-out
///   terrain meets it. Any other height puts a seam along the horizon, which is
///   the exact artefact the shared band exists to prevent.
/// * the first is the zenith and is the darkest of the six, which is what
///   [`band::SKY_TOP`] is pinned by.
///
/// The four between are bunched toward the horizon because the table's own
/// values say they belong there: [`band::SKY_SMOG`] is darker than the band
/// above it in 18 of the 19 default lights and [`band::SKY_FOG`] in 17, which is
/// haze lying *under* the sky rather than more of it — and haze is a thin wedge
/// over the horizon, not the top third of the sky.
pub const SKY_ALTITUDES: [f32; SKY_STOPS] = [1.00, 0.50, 0.22, 0.09, 0.03, 0.00];

/// Which int band each of those stops reads, zenith first.
///
/// Named one by one rather than counted off from [`band::SKY_TOP`], so that the
/// dome's order is a list to read against the table and not arithmetic to
/// trust. One copy, because two functions build a dome now and a pair that
/// disagreed would put a zone's smog where its zenith should be.
pub const SKY_BANDS: [u32; SKY_STOPS] = [
    band::SKY_TOP,
    band::SKY_MIDDLE,
    band::SKY_BAND_1,
    band::SKY_BAND_2,
    band::SKY_SMOG,
    band::SKY_FOG,
];

/// **Everything global about how the world looks, at one place and one time.**
///
/// The sun, the fill, the dome and the distance the world fades out at — all of
/// it out of the game's own tables rather than chosen here, which is the whole
/// point: a value picked to look right is a value that is wrong in the next
/// zone, and this game has 19 of them shipping different daylight.
///
/// Colours are 0..1, and they are **sRGB as the file states them**. Decoding to
/// linear is the renderer's business and is where the exposure question lives;
/// this module reports what the table says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Atmosphere {
    /// The sun: the colour of the one directional light the world is lit by.
    pub diffuse: [f32; 3],
    /// The fill, which is what everything facing away from the sun is lit by.
    /// Without it a north wall is black, which no screenshot of this game has.
    pub ambient: [f32; 3],
    /// The dome, zenith first and the horizon last — see [`band::SKY_TOP`].
    pub sky: [[f32; 3]; SKY_STOPS],
    /// Band 17, whatever it is — see [`band::SHADOW`], where its identity is an
    /// open question. Printed by `vale light`; **no longer read by the
    /// renderer**, since `MCSH` turned out to be a flat scalar with no colour.
    pub shadow: [f32; 3],
    /// The sun's own disc, band 8 — see [`band::SUN_DISC`]. Carried and not
    /// drawn: the sprite it colours needs a position over the day that this
    /// chain does not state. `vale sky` prints it.
    pub sun_disc: [f32; 3],
    /// Its halo, band 9 — [`band::SUN_HALO`], on the same terms.
    pub sun_halo: [f32; 3],
    /// Where the fog reaches full strength, in yards.
    pub fog_end: f32,
    /// Where it begins, in yards. Always `<= fog_end`; see
    /// [`float_band::FOG_START_SCALER`] for why the file stores the ratio.
    pub fog_start: f32,
}

impl Atmosphere {
    /// Fog distance for a light whose float band is missing or zero.
    ///
    /// **361 yards, the median `fogEnd` over the 19 default lights** rather
    /// than a round number: a client that has to guess should guess what the
    /// game usually says. The rows run 111 yards (Scarlet Monastery) to 888
    /// (Alterac Valley).
    pub const DEFAULT_FOG_END: f32 = 361.0;
    /// And where it starts, as a fraction of the end — the same median, 0.10.
    /// The game's own scalers run −0.30 to 0.40; see
    /// [`float_band::FOG_START_SCALER`] for what a negative one means.
    pub const DEFAULT_FOG_START_SCALER: f32 = 0.10;

    /// **What the world is lit by when the light chain says nothing at all** —
    /// no `Light.dbc`, or a chain naming no default for any map.
    ///
    /// Every one of these is the **mean of that band over the 19 default lights
    /// at noon**, which makes it the game's own average daylight rather than a
    /// taste. Deliberately not white and deliberately not garish, which is the
    /// opposite of what [`LiquidLight::UNLIT`] does two types up: a liquid is
    /// one surface and a loud one is a diagnostic, where this is the whole
    /// screen and a loud one hides every other thing wrong with the frame.
    /// See [`LightTables::atmosphere`].
    pub const PLACEHOLDER: Atmosphere = Atmosphere {
        diffuse: [0.80, 0.60, 0.47],
        ambient: [0.30, 0.32, 0.40],
        sky: [
            [0.14, 0.17, 0.22],
            [0.34, 0.48, 0.50],
            [0.51, 0.62, 0.58],
            [0.56, 0.70, 0.65],
            [0.49, 0.47, 0.49],
            [0.25, 0.30, 0.35],
        ],
        shadow: [0.21, 0.26, 0.27],
        sun_disc: [0.28, 0.28, 0.28],
        sun_halo: [0.89, 0.85, 0.70],
        fog_end: Atmosphere::DEFAULT_FOG_END,
        fog_start: Atmosphere::DEFAULT_FOG_END * Atmosphere::DEFAULT_FOG_START_SCALER,
    };

    /// The last stop of the dome, which is **also** the colour distant geometry
    /// is fogged to. One band read twice, deliberately — see
    /// [`band::SKY_FOG`]. Anything else puts a seam on the horizon.
    pub fn fog(&self) -> [f32; 3] {
        self.sky[SKY_STOPS - 1]
    }

    /// `self` moved `t` of the way toward `other` — every field of it.
    ///
    /// **Blended as whole atmospheres rather than band by band**, which is the
    /// cheaper *and* the safer of the two: a positional light contributes one
    /// `LightParams` row, so mixing the results of two rows is exactly mixing
    /// the two rows, and doing it here means the fog distances and the six sky
    /// stops cannot be left behind when a band is added. In the file's own
    /// space, like every other sum in this chain — see the module note.
    pub fn mix(&self, other: &Atmosphere, t: f32) -> Atmosphere {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        let lerp3 = |a: [f32; 3], b: [f32; 3]| {
            [lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])]
        };
        let mut sky = [[0.0; 3]; SKY_STOPS];
        for (stop, (a, b)) in sky.iter_mut().zip(self.sky.iter().zip(other.sky.iter())) {
            *stop = lerp3(*a, *b);
        }
        Atmosphere {
            diffuse: lerp3(self.diffuse, other.diffuse),
            ambient: lerp3(self.ambient, other.ambient),
            sky,
            shadow: lerp3(self.shadow, other.shadow),
            sun_disc: lerp3(self.sun_disc, other.sun_disc),
            sun_halo: lerp3(self.sun_halo, other.sun_halo),
            fog_end: lerp(self.fog_end, other.fog_end),
            fog_start: lerp(self.fog_start, other.fog_start),
        }
    }
}

/// One liquid's colour, and the two ends of its depth-driven opacity.
///
/// RGB is 0..1 and already *lit* — these come out of the light system, not out of
/// a texture, so nothing multiplies them by the sun.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiquidLight {
    /// The near colour, which is what a canal under the camera is drawn in.
    pub close: [f32; 3],
    /// The far colour. **Carried and not yet used**: mixing toward it needs a
    /// distance from the camera, which is a per-fragment term the liquid material
    /// does not have, and the surfaces this client draws are all within a few
    /// dozen yards. It is here so the field is read once rather than discovered
    /// again when the terrain's own `MCLQ` arrives and oceans reach the horizon.
    pub far: [f32; 3],
    /// Opacity at `MLIQ` depth 0 — the bank, where the bottom shows through.
    pub shallow_alpha: f32,
    /// Opacity at depth 255.
    pub deep_alpha: f32,
}

impl LiquidLight {
    /// What a liquid with no light table is drawn as.
    ///
    /// **Opaque white, which is wrong and conspicuous.** The alternative was to
    /// fall back to the texture's own near-black colour, and that is the bug this
    /// module exists to fix — a missing table must not reproduce an *invisible*
    /// failure. See [`LightTables`]' note on degradation.
    pub const UNLIT: LiquidLight = LiquidLight {
        close: [1.0, 1.0, 1.0],
        far: [1.0, 1.0, 1.0],
        shallow_alpha: 1.0,
        deep_alpha: 1.0,
    };

    /// The colour and opacity at a given `MLIQ` depth byte.
    ///
    /// `self` moved `t` of the way toward `other`. See [`Atmosphere::mix`],
    /// which this is the water half of and which says why it is done on the
    /// result rather than on the bands.
    pub fn mix(&self, other: &LiquidLight, t: f32) -> LiquidLight {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        let lerp3 = |a: [f32; 3], b: [f32; 3]| {
            [lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])]
        };
        LiquidLight {
            close: lerp3(self.close, other.close),
            far: lerp3(self.far, other.far),
            shallow_alpha: lerp(self.shallow_alpha, other.shallow_alpha),
            deep_alpha: lerp(self.deep_alpha, other.deep_alpha),
        }
    }

    /// Used by `vale water` to report the ramp; the renderer does the same mix
    /// in the fragment shader off the vertex alpha, so one draw covers a canal
    /// that shallows at its banks.
    pub fn at_depth(&self, depth: u8) -> [f32; 4] {
        let t = depth as f32 / 255.0;
        [
            self.close[0],
            self.close[1],
            self.close[2],
            self.shallow_alpha + (self.deep_alpha - self.shallow_alpha) * t,
        ]
    }
}

/// One of `Light.dbc`'s **positional** rows: a sphere of different daylight
/// laid over the map's default.
///
/// This is how the game gives a zone its own look — Duskwood's grey, Ashenvale's
/// teal water, the warm lamplight of a city — and there are 80 of them on map 0
/// and 126 on map 1 against one default each. A row is not a zone boundary and
/// does not try to be: the spheres cover about a tenth of map 0's tiles, and
/// everywhere else really is the default, which is what the real client shows
/// outside them too.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionalLight {
    pub id: u32,
    pub map: u32,
    /// The centre, in the world's own axes and in yards. See [`WORLD_CORNER`].
    pub at: [f32; 3],
    /// The same three floats exactly as the file states them — kept so that
    /// `vale light` can score the *other* readings of them against the
    /// tiles each map ships. A decode this project cannot re-derive from the
    /// stored result is a decode nothing can check.
    pub raw: [f32; 3],
    /// Inside this radius the row applies at full strength.
    pub falloff_start: f32,
    /// And outside this one, not at all.
    pub falloff_end: f32,
    /// The `LightParams` row it names in clear weather.
    pub params: u32,
    /// …and the one it names for a camera **under the water**, which is a
    /// different row with its own eighteen bands — see
    /// [`light_field::PARAMS_CLEAR_UNDERWATER`]. Zero for a row that states
    /// none, which [`Weather::of`] reads as "use the clear one".
    pub params_underwater: u32,
    /// …and the two the weather names, on the same terms — see
    /// [`light_field::PARAMS_STORM`].
    pub params_storm: u32,
    pub params_storm_underwater: u32,
}

impl PositionalLight {
    /// The row's four `LightParams` ids in `Light.dbc`'s own column order, which
    /// is what [`Weather::of`] indexes.
    pub fn params_of(&self, weather: Weather) -> u32 {
        weather.of([
            self.params,
            self.params_underwater,
            self.params_storm,
            self.params_storm_underwater,
        ])
    }
}

impl PositionalLight {
    /// How strongly this row applies at `at` — 1.0 inside `falloff_start`,
    /// falling linearly to 0.0 at `falloff_end`, and 0.0 beyond it.
    ///
    /// **The distance is three-dimensional, which is the literal reading of one
    /// radius and two of the rows are the check on it**: Ironforge's light sits
    /// at height 436 and Ironforge at 502, inside a 999..1214 yard sphere, and
    /// Un'Goro's pair are 218 yards *below* sea level. A horizontal-only
    /// distance would light the sky above Ironforge by the mountain's light.
    ///
    /// **The ramp itself is interpretation.** The two radii are the file's and
    /// the fact that the inner one is smaller is the file's; that the blend
    /// between them is linear is this client's reading of "falloff", and
    /// nothing in the client has been checked to confirm the curve. What
    /// is not at risk either way is the two ends, which is where a light
    /// visibly is or is not.
    pub fn weight(&self, at: [f32; 3]) -> f32 {
        let d = ((self.at[0] - at[0]).powi(2)
            + (self.at[1] - at[1]).powi(2)
            + (self.at[2] - at[2]).powi(2))
        .sqrt();
        if d <= self.falloff_start {
            return 1.0;
        }
        if d >= self.falloff_end {
            return 0.0;
        }
        // Only reachable with `falloff_start < d < falloff_end`, so the span is
        // positive; the guard is for a row that states the two the same way
        // round it should not.
        let span = self.falloff_end - self.falloff_start;
        if span <= 0.0 {
            return 1.0;
        }
        (self.falloff_end - d) / span
    }
}

/// The light chain, loaded once.
///
/// **Optional, and its absence is a documented degradation** like every other
/// table but the three that resolve a model: without it every liquid draws
/// [`LiquidLight::UNLIT`], which is a flat white sheet — conspicuous rather than
/// invisible, deliberately, because the failure this replaces was invisible.
pub struct LightTables {
    /// Per map: the id of the `LightParams` row the map's *default* light names.
    ///
    /// A map's default is the row at the origin with both falloff radii zero —
    /// vanilla ships one per map — and every other row refines it inside a
    /// sphere. Those are [`Self::positional`].
    /// `[clear, clear-underwater, storm, storm-underwater]` — see [`Weather`].
    defaults: std::collections::HashMap<u32, [u32; 4]>,
    /// Every row that is not a default: a sphere of its own daylight over the
    /// map's, which is where a zone's own look lives.
    ///
    /// **Sorted by `falloff_end` descending at load, and the order is the blend
    /// order** — see [`LightTables::lights_at`]. Sorting here rather than at
    /// every query is not only for the cost: it makes "the smaller sphere is
    /// the more specific one" a property of the stored table, in one place,
    /// instead of a rule three callers have to remember.
    positional: Vec<PositionalLight>,
    /// `LightParams` rows by id: the four water alphas.
    params: std::collections::HashMap<u32, [f32; 4]>,
    /// `LightIntBand` rows by id: `(times, colours)`, already trimmed to the
    /// entry count the row declares.
    bands: std::collections::HashMap<u32, (Vec<u32>, Vec<u32>)>,
    /// `LightFloatBand` rows by id, same shape with floats. **May be empty**,
    /// which costs the fog distances and nothing else — see
    /// [`Atmosphere::DEFAULT_FOG_END`].
    float_bands: std::collections::HashMap<u32, (Vec<u32>, Vec<f32>)>,
}

impl LightTables {
    /// Parse the chain. Any of the first three failing gives `None`, which the
    /// caller turns into [`LiquidLight::UNLIT`] everywhere.
    ///
    /// **`float_band` is the fourth and is allowed to be missing**, on different
    /// terms from the other three: it carries two distances, and a client with
    /// no fog draws the world it already draws. The other three carry every
    /// colour, and without them there is nothing to draw it in.
    pub fn parse(
        light: &[u8],
        params: &[u8],
        int_band: &[u8],
        float_band: &[u8],
    ) -> Option<LightTables> {
        let light = Dbc::parse(light).ok()?;
        let params_dbc = Dbc::parse(params).ok()?;
        let bands_dbc = Dbc::parse(int_band).ok()?;

        // The 18 is checked against the files rather than trusted: if a chain
        // ever ships a different band count the row arithmetic below would read
        // the sky colour of the next light as water and produce a *plausible*
        // wrong colour, which is the failure mode this whole area keeps hitting.
        if params_dbc.record_count == 0
            || bands_dbc.record_count != params_dbc.record_count * BANDS_PER_PARAMS as usize
        {
            return None;
        }

        let mut defaults = std::collections::HashMap::new();
        let mut positional = Vec::new();
        for record in 0..light.record_count {
            let map = light.u32_at(record, light_field::MAP)?;
            let Some(params) = light.u32_at(record, light_field::PARAMS_CLEAR) else {
                continue;
            };
            let params_underwater = light
                .u32_at(record, light_field::PARAMS_CLEAR_UNDERWATER)
                .unwrap_or(0);
            let params_storm = light
                .u32_at(record, light_field::PARAMS_STORM)
                .unwrap_or(0);
            let params_storm_underwater = light
                .u32_at(record, light_field::PARAMS_STORM_UNDERWATER)
                .unwrap_or(0);
            let end = light.f32_at(record, light_field::FALLOFF_END).unwrap_or(0.0);
            // The default light is the one whose falloff covers nothing, which is
            // how the client marks "everywhere on this map".
            if end == 0.0 {
                defaults.entry(map).or_insert([
                    params,
                    params_underwater,
                    params_storm,
                    params_storm_underwater,
                ]);
                continue;
            }
            let coord = |field| light.f32_at(record, field).unwrap_or(0.0) * YARDS_PER_UNIT;
            positional.push(PositionalLight {
                id: light.u32_at(record, 0).unwrap_or(0),
                map,
                // Scaled out of the file's 1/36 yards first, then handed to the
                // same conversion every placement in the game goes through —
                // see `light_field`. The height passes through it untouched,
                // which is what the rows say too: they run −234 to +763 yards,
                // where a corner-relative one would put every light 17 km up.
                at: crate::world::adt::placement_to_world([
                    coord(light_field::INTERNAL_X),
                    coord(light_field::INTERNAL_Y),
                    coord(light_field::INTERNAL_Z),
                ]),
                raw: [
                    light.f32_at(record, light_field::INTERNAL_X).unwrap_or(0.0),
                    light.f32_at(record, light_field::INTERNAL_Y).unwrap_or(0.0),
                    light.f32_at(record, light_field::INTERNAL_Z).unwrap_or(0.0),
                ],
                falloff_start: light
                    .f32_at(record, light_field::FALLOFF_START)
                    .unwrap_or(0.0)
                    * YARDS_PER_UNIT,
                falloff_end: end * YARDS_PER_UNIT,
                params,
                params_underwater,
                params_storm,
                params_storm_underwater,
            });
        }
        // Widest first: see the field's own note, and `lights_at` for what the
        // order then means.
        positional.sort_by(|a, b| {
            b.falloff_end
                .partial_cmp(&a.falloff_end)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut params = std::collections::HashMap::new();
        for record in 0..params_dbc.record_count {
            let Some(id) = params_dbc.u32_at(record, 0) else {
                continue;
            };
            params.insert(
                id,
                [
                    params_dbc
                        .f32_at(record, params_field::WATER_SHALLOW_ALPHA)
                        .unwrap_or(1.0),
                    params_dbc
                        .f32_at(record, params_field::WATER_DEEP_ALPHA)
                        .unwrap_or(1.0),
                    params_dbc
                        .f32_at(record, params_field::OCEAN_SHALLOW_ALPHA)
                        .unwrap_or(1.0),
                    params_dbc
                        .f32_at(record, params_field::OCEAN_DEEP_ALPHA)
                        .unwrap_or(1.0),
                ],
            );
        }

        let bands = read_band_table(&bands_dbc, |dbc, record, field| dbc.u32_at(record, field));

        // The float table is optional and is checked on its own terms: a row
        // count that is not 6 per params means the arithmetic would read one
        // light's fog as another's, so it is dropped whole rather than read
        // crooked. Dropping it costs the two distances and nothing else.
        let float_bands = match Dbc::parse(float_band) {
            Ok(dbc) if dbc.record_count == params_dbc.record_count * FLOAT_BANDS_PER_PARAMS as usize => {
                read_band_table(&dbc, |dbc, record, field| dbc.f32_at(record, field))
            }
            _ => std::collections::HashMap::new(),
        };

        Some(LightTables {
            defaults,
            positional,
            params,
            bands,
            float_bands,
        })
    }

    /// The colour and opacity ramp for one liquid on one map, at `time`
    /// half-minutes past midnight.
    ///
    /// **`None` means "draw it from its own texture", and magma and slime always
    /// answer that** — not for want of a band, but because theirs are the two
    /// flipbooks that *do* carry colour (peak 255 and 174, against water's 41)
    /// and their own `alphaDepth=0` already says they are opaque. Tinting them
    /// would be inventing a decision the files have already made, and tinting
    /// them *white*, which is what the fallback is, would blow a lava pool out.
    pub fn liquid(&self, map: u32, kind: Liquid, time: u32) -> Option<LiquidLight> {
        let Some(&params_id) = self.defaults.get(&map) else {
            // Deliberately *not* `params_for` — a map with no light of its own
            // gets conspicuous white water rather than map 0's, which is the
            // documented split between this fallback and `atmosphere`'s.
            return match kind {
                Liquid::Magma | Liquid::Slime => None,
                _ => Some(LiquidLight::UNLIT),
            };
        };
        // The water's own colour is the *surface* light's: a canal is drawn from
        // outside it, and what changes when the camera goes under is the
        // atmosphere rather than the liquid material.
        self.liquid_of(Weather::Clear.of(params_id), kind, time, Self::liquid_bands(kind)?)
    }

    /// The same, refined by wherever `at` is on that map — the positional half
    /// of the chain, which is where a zone's own water lives: Un'Goro's green,
    /// Ashenvale's teal, Ironforge's lava-lit pools. See [`Self::lights_at`].
    ///
    /// A position no sphere covers answers exactly what [`Self::liquid`] does,
    /// which is most of every map and is the colour the real client uses out
    /// there too.
    pub fn liquid_at(
        &self,
        map: u32,
        at: [f32; 3],
        kind: Liquid,
        time: u32,
    ) -> Option<LiquidLight> {
        let mut light = self.liquid(map, kind, time)?;
        let bands = Self::liquid_bands(kind)?;
        for (params, weight) in self.lights_at(map, at) {
            if let Some(refined) = self.liquid_over(&light, params, kind, time, bands) {
                light = light.mix(&refined, weight);
            }
        }
        Some(light)
    }

    /// **The same liquid read as a shallow and a deep colour**, off the four
    /// bands one row up — see [`band::OCEAN_SHALLOW`], where the measurement
    /// is. `close` is the colour at depth byte 0 and `far` the colour at 255;
    /// the two alphas are the float bands' own, as in [`Self::liquid_at`].
    ///
    /// What the shipped minimaps were rendered with, and not what the client
    /// draws its surfaces with; the two are the same chain read one row apart.
    pub fn liquid_by_depth_at(
        &self,
        map: u32,
        at: [f32; 3],
        kind: Liquid,
        time: u32,
    ) -> Option<LiquidLight> {
        let bands = match kind {
            Liquid::Water => (band::RIVER_SHALLOW, band::RIVER_DEEP),
            Liquid::Ocean => (band::OCEAN_SHALLOW, band::OCEAN_DEEP),
            Liquid::Magma | Liquid::Slime => return None,
        };
        let Some(&params_id) = self.defaults.get(&map) else {
            return Some(LiquidLight::UNLIT);
        };
        let mut light = self.liquid_of(Weather::Clear.of(params_id), kind, time, bands)?;
        for (params, weight) in self.lights_at(map, at) {
            if let Some(refined) = self.liquid_over(&light, params, kind, time, bands) {
                light = light.mix(&refined, weight);
            }
        }
        Some(light)
    }

    /// One `LightParams` row's liquid **laid over `base`**: every band the row
    /// states, and `base`'s where it states nothing. See [`Self::band`] for why
    /// that is the ordinary case rather than a fault.
    fn liquid_over(
        &self,
        base: &LiquidLight,
        params_id: u32,
        kind: Liquid,
        time: u32,
        (close_band, far_band): (u32, u32),
    ) -> Option<LiquidLight> {
        let (shallow, deep) = Self::alpha_fields(kind)?;
        let alphas = self.params.get(&params_id);
        let alpha = |field: usize, under: f32| {
            alphas
                .map(|a| a[field - params_field::WATER_SHALLOW_ALPHA])
                .unwrap_or(under)
        };
        Some(LiquidLight {
            close: self.band(params_id, close_band, time).unwrap_or(base.close),
            far: self.band(params_id, far_band, time).unwrap_or(base.far),
            shallow_alpha: alpha(shallow, base.shallow_alpha),
            deep_alpha: alpha(deep, base.deep_alpha),
        })
    }

    /// Which two bands a liquid's surface is drawn from, or `None` for the two
    /// that are drawn from their own texture.
    fn liquid_bands(kind: Liquid) -> Option<(u32, u32)> {
        match kind {
            Liquid::Water => Some((band::RIVER_CLOSE, band::RIVER_FAR)),
            Liquid::Ocean => Some((band::OCEAN_CLOSE, band::OCEAN_FAR)),
            Liquid::Magma | Liquid::Slime => None,
        }
    }

    /// …and which two alpha fields.
    fn alpha_fields(kind: Liquid) -> Option<(usize, usize)> {
        match kind {
            Liquid::Water => Some((
                params_field::WATER_SHALLOW_ALPHA,
                params_field::WATER_DEEP_ALPHA,
            )),
            Liquid::Ocean => Some((
                params_field::OCEAN_SHALLOW_ALPHA,
                params_field::OCEAN_DEEP_ALPHA,
            )),
            Liquid::Magma | Liquid::Slime => None,
        }
    }

    /// One `LightParams` row's liquid, which is what both of the above are made
    /// of, from the two bands given.
    fn liquid_of(
        &self,
        params_id: u32,
        kind: Liquid,
        time: u32,
        (close_band, far_band): (u32, u32),
    ) -> Option<LiquidLight> {
        let (shallow_alpha, deep_alpha) = Self::alpha_fields(kind)?;
        // The alpha fields are stored in the order this array was built in.
        let alpha_index = |field: usize| field - params_field::WATER_SHALLOW_ALPHA;

        let alphas = self
            .params
            .get(&params_id)
            .copied()
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);

        Some(LiquidLight {
            close: self.band_colour(params_id, close_band, time),
            far: self.band_colour(params_id, far_band, time),
            shallow_alpha: alphas[alpha_index(shallow_alpha)],
            deep_alpha: alphas[alpha_index(deep_alpha)],
        })
    }

    /// The world's light on one map at one time: the sun, the fill, the dome
    /// and the two fog distances. See [`Atmosphere`].
    ///
    /// **A map with no `Light` row of its own is given map 0's, and that is a
    /// different fallback from the one [`LightTables::liquid`] takes** for the
    /// same missing row. The reason they differ is what the fallback is *for*:
    /// white water is a marker on one surface, in a place the player may never
    /// walk, and it is loud exactly where it needs to be. A white sky is the
    /// whole screen — it does not report a missing row, it reports a broken
    /// client, and it would hide every other thing wrong with the frame. So
    /// this one degrades to the game's own daylight and the *count* is what
    /// reports it: `vale light` names every map taking the fallback.
    pub fn atmosphere(&self, map: u32, time: u32) -> Atmosphere {
        self.atmosphere_weather(map, time, Weather::Clear)
    }

    /// The same, in one of the two weathers this client can be in — see
    /// [`Weather`] and [`Self::atmosphere_in`].
    pub fn atmosphere_weather(&self, map: u32, time: u32, weather: Weather) -> Atmosphere {
        match self.params_for_weather(map, weather) {
            Some(params_id) => self.atmosphere_of(params_id, time),
            None => Atmosphere::PLACEHOLDER,
        }
    }

    /// The same, refined by wherever `at` is on that map.
    ///
    /// **This is the difference between a continent lit one way and a world of
    /// zones.** Map 0 ships one default light and eighty spheres over it, and
    /// until they were read every one of Stormwind, Duskwood, Westfall,
    /// Stranglethorn and Ironforge was drawn in Elwynn's daylight. Goldshire
    /// itself is covered by no sphere at all, which is why the one place every
    /// screenshot in this repo was taken looked right while the rest did not.
    ///
    /// See [`Self::lights_at`] for the blend and for which half of it is
    /// measurement.
    pub fn atmosphere_at(&self, map: u32, at: [f32; 3], time: u32) -> Atmosphere {
        self.atmosphere_in(map, at, time, Weather::Clear)
    }

    /// …and the same again with the **weather** chosen, which is the whole of
    /// what this client does about a camera under the water.
    ///
    /// `Light.dbc` states five `LightParams` ids per row and the second of them
    /// is the same light seen from beneath the surface: its dome band is the
    /// water's colour, its fog closes to a few dozen yards, and its sun is what
    /// is left of one. Nothing else changes — there is no post pass, no tinted
    /// quad and no second shader, and the reference has none of those either.
    /// A zone's own sphere is switched with it, so diving in Un'Goro's lake and
    /// diving in the sea are different colours for the same reason they are
    /// different colours above the surface.
    ///
    /// **What decides *whether* the camera is under is not this module's** — it
    /// is a liquid surface against an eye position, and it lives beside the
    /// mover that already asks the same question about the character's feet.
    pub fn atmosphere_in(
        &self,
        map: u32,
        at: [f32; 3],
        time: u32,
        weather: Weather,
    ) -> Atmosphere {
        let mut sky = self.atmosphere_weather(map, time, weather);
        for (params, weight) in self.lights_in(map, at, weather) {
            let over = self.atmosphere_over(&sky, params, time);
            sky = sky.mix(&over, weight);
        }
        sky
    }

    /// …and **with the weather up**: the same place lit by its clear row and by
    /// its storm row, mixed by how hard it is coming down.
    ///
    /// `storm` is the weather machine's grade, 0 for a clear sky and 1 for the
    /// hardest the server ever sends. At 0 this is exactly [`Self::atmosphere_in`]
    /// — the identity, tested — so a clear day is not paying for this and is not
    /// changed by it.
    ///
    /// **Which half is which.** That `Light.dbc` carries a storm row per light,
    /// that it is column 2 of the five, and what is in it — a darker sun, a
    /// colder fill, a dome that has lost its blue and a fog pulled in to a
    /// fraction of the clear one's — is the table, printed by `vale weather`.
    /// That the two are **mixed** rather than switched, and that the mix is
    /// linear in the grade, is this client's reading: the client has not
    /// been checked for which of the grade and the density it blends on, or whether
    /// it blends at all rather than crossfading over a fixed time. What is not
    /// open is the direction — a client that ignores the column draws Elwynn's
    /// noon behind a downpour, which is the picture this was opened for.
    ///
    /// The grade rather than the density (`max(0, (g - 0.25) * 4/3)`) on
    /// purpose: the density is the *particle* count and its floor is a quarter,
    /// so a light shower that the reference draws no drops for would otherwise
    /// leave the sky exactly as bright as a clear one — and a sky that is
    /// already grey before the first drop falls is what the ramp is for.
    pub fn atmosphere_in_storm(
        &self,
        map: u32,
        at: [f32; 3],
        time: u32,
        weather: Weather,
        storm: f32,
    ) -> Atmosphere {
        let (dry, wet) = weather.dry();
        let clear = self.atmosphere_in(map, at, time, dry);
        let storm = storm.clamp(0.0, 1.0);
        if storm <= 0.0 {
            return clear;
        }
        clear.mix(&self.atmosphere_in(map, at, time, wet), storm)
    }

    /// One `LightParams` row's atmosphere **laid over `base`**: every band the
    /// row states, and `base`'s where it states nothing.
    ///
    /// See [`Self::band`] — a row leaving a band empty is the ordinary case,
    /// and taking [`Self::atmosphere_of`]'s white for those would repaint the
    /// zone's sky with the marker that means "the chain is broken".
    fn atmosphere_over(&self, base: &Atmosphere, params_id: u32, time: u32) -> Atmosphere {
        let mut sky = base.sky;
        for (stop, band) in sky.iter_mut().zip(SKY_BANDS) {
            if let Some(colour) = self.band(params_id, band, time) {
                *stop = colour;
            }
        }
        // The fog is a pair and it moves as one: a row stating an end and no
        // scaler would otherwise take the *base*'s ratio of a different
        // distance, which is a zone's own fog starting somewhere neither light
        // asked for. Either both come from the row or neither does.
        let (fog_start, fog_end) = match self.band_value(params_id, float_band::FOG_END, time) {
            Some(end) if end > 0.0 => {
                let end = end * YARDS_PER_UNIT;
                let scaler = self
                    .band_value(params_id, float_band::FOG_START_SCALER, time)
                    .unwrap_or(Atmosphere::DEFAULT_FOG_START_SCALER);
                (end * scaler.clamp(0.0, 1.0), end)
            }
            _ => (base.fog_start, base.fog_end),
        };
        Atmosphere {
            diffuse: self.band(params_id, band::DIFFUSE, time).unwrap_or(base.diffuse),
            ambient: self.band(params_id, band::AMBIENT, time).unwrap_or(base.ambient),
            sky,
            shadow: self.band(params_id, band::SHADOW, time).unwrap_or(base.shadow),
            sun_disc: self
                .band(params_id, band::SUN_DISC, time)
                .unwrap_or(base.sun_disc),
            sun_halo: self
                .band(params_id, band::SUN_HALO, time)
                .unwrap_or(base.sun_halo),
            fog_end,
            fog_start,
        }
    }

    /// Every positional row covering `at` on `map`, with how strongly it
    /// applies, **in the order they are to be laid over the map's default**.
    ///
    /// The order is widest sphere first, so a smaller light applied later wins
    /// over a larger one it sits inside. That case is real and it is not rare:
    /// Orgrimmar stands inside Durotar's 1023..1547 yard light *and* its own
    /// 460..503 yard one, both at full weight, naming different `LightParams`
    /// rows — so some rule has to break the tie, and "the more specific sphere"
    /// is the only one of the two that can be right in both directions.
    ///
    /// **Which half is which:** that these rows exist, where they are, how
    /// large they are and which params they name is all measured (see
    /// [`WORLD_CORNER`]). That overlapping rows compose by mixing inner over
    /// outer, and that the falloff between the two radii is linear, is this
    /// client's reading and has not been checked against the client. What it cannot
    /// get wrong is a point covered by one sphere, which is the ordinary case.
    pub fn lights_at(&self, map: u32, at: [f32; 3]) -> Vec<(u32, f32)> {
        self.lights_in(map, at, Weather::Clear)
    }

    /// …and in a chosen weather — the same spheres in the same order, each
    /// naming its own second row. See [`Self::atmosphere_in`].
    pub fn lights_in(&self, map: u32, at: [f32; 3], weather: Weather) -> Vec<(u32, f32)> {
        self.positional
            .iter()
            .filter(|light| light.map == map)
            .filter_map(|light| match light.weight(at) {
                w if w > 0.0 => Some((light.params_of(weather), w)),
                _ => None,
            })
            .collect()
    }

    /// Every positional row on a map, widest first — for `vale light`, which
    /// checks them against the tiles the map actually has.
    pub fn positional(&self, map: u32) -> impl Iterator<Item = &PositionalLight> {
        self.positional.iter().filter(move |light| light.map == map)
    }

    /// One `LightParams` row's whole atmosphere, which is what both of the
    /// above are made of.
    fn atmosphere_of(&self, params_id: u32, time: u32) -> Atmosphere {
        let mut sky = [[0.0; 3]; SKY_STOPS];
        for (colour, band) in sky.iter_mut().zip(SKY_BANDS) {
            *colour = self.band_colour(params_id, band, time);
        }
        // Both distances default together: a fog end with no scaler would put
        // the start at the camera, which is a world seen through milk.
        let (fog_end, scaler) = match self.band_value(params_id, float_band::FOG_END, time) {
            Some(end) if end > 0.0 => (
                end * YARDS_PER_UNIT,
                self.band_value(params_id, float_band::FOG_START_SCALER, time)
                    .unwrap_or(Atmosphere::DEFAULT_FOG_START_SCALER),
            ),
            _ => (
                Atmosphere::DEFAULT_FOG_END,
                Atmosphere::DEFAULT_FOG_START_SCALER,
            ),
        };
        Atmosphere {
            diffuse: self.band_colour(params_id, band::DIFFUSE, time),
            ambient: self.band_colour(params_id, band::AMBIENT, time),
            sky,
            shadow: self.band_colour(params_id, band::SHADOW, time),
            sun_disc: self.band_colour(params_id, band::SUN_DISC, time),
            sun_halo: self.band_colour(params_id, band::SUN_HALO, time),
            fog_end,
            fog_start: fog_end * scaler.clamp(0.0, 1.0),
        }
    }

    /// Which `LightParams` row a map's light comes from, falling back to map 0's
    /// — see [`LightTables::atmosphere`]. `None` only if map 0 is missing too,
    /// which means the chain loaded but named no default light at all.
    pub fn params_for(&self, map: u32) -> Option<u32> {
        self.params_for_weather(map, Weather::Clear)
    }

    /// …in a chosen weather — see [`Weather`].
    pub fn params_for_weather(&self, map: u32, weather: Weather) -> Option<u32> {
        self.defaults
            .get(&map)
            .or_else(|| self.defaults.get(&0))
            .map(|ids| weather.of(*ids))
    }

    /// Whether this map names a light row of its own, rather than borrowing map
    /// 0's. For `vale light`, which counts them.
    pub fn has_own_light(&self, map: u32) -> bool {
        self.defaults.contains_key(&map)
    }

    /// Every `LightParams` id the chain names as some map's default, and one
    /// band of it — for `vale water`'s band survey, which is what pins the
    /// four water bands out of the eighteen. See the module note.
    pub fn default_params(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.defaults.values().map(|pair| pair[0]).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// One band of one `LightParams` row, for the same survey. `None` when the
    /// arithmetic lands outside the table.
    /// **`None` means the row states nothing about this band**, which is the
    /// ordinary case and not a fault: `LightIntBand` rows carry an entry count
    /// and **only 181 of the 426 `LightParams` rows fill all eighteen**. Most
    /// state fourteen to seventeen and leave the rest to whatever they are laid
    /// over — Duskwood's light says nothing about river water, Ironforge's
    /// nothing about the ocean.
    ///
    /// That distinction did not exist while the only caller was a map's own
    /// default, because a default states everything; it appeared the moment a
    /// positional row was read over one, and it appeared as **white water in
    /// Duskwood** — [`Self::band_colour`]'s conspicuous marker standing in for
    /// a band that was never missing, only silent.
    pub fn band(&self, params_id: u32, band: u32, time: u32) -> Option<[f32; 3]> {
        let row = params_id.checked_sub(1)? * BANDS_PER_PARAMS + band + 1;
        let (times, colours) = self.bands.get(&row)?;
        let (i, j, t) = straddle(times, colours.len(), time)?;
        let (a, b) = (unpack(colours[i]), unpack(colours[j]));
        Some([
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ])
    }

    /// One int band's colour at one time. White for a band the row does not
    /// state, which is the same conspicuous fallback [`LiquidLight::UNLIT`] is.
    ///
    /// **Only for a map's *default* light**, which states every band — so the
    /// white really does mean "this chain is broken". A positional row goes
    /// through [`Self::band`] and keeps what it is laid over instead.
    fn band_colour(&self, params_id: u32, band: u32, time: u32) -> [f32; 3] {
        self.band(params_id, band, time).unwrap_or([1.0, 1.0, 1.0])
    }

    /// One float band's value at one time. `None` for a row the arithmetic
    /// misses — which is the ordinary case, because the float table is optional.
    ///
    /// Public because `vale light` surveys the six of them the same way the
    /// eighteen were surveyed; that is how [`float_band`]'s two are named.
    pub fn band_value(&self, params_id: u32, band: u32, time: u32) -> Option<f32> {
        let row = params_id.checked_sub(1)? * FLOAT_BANDS_PER_PARAMS + band + 1;
        let (times, values) = self.float_bands.get(&row)?;
        let (i, j, t) = straddle(times, values.len(), time)?;
        Some(values[i] + (values[j] - values[i]) * t)
    }
}

/// The two entries of a band that straddle `time`, and how far between them it
/// is — **wrapping across midnight**, because a band is a cycle and a time past
/// the last entry belongs between it and the first.
///
/// One copy for both tables. It was written for the int bands and the float
/// bands need it identically; two copies of a wrap this fiddly is two chances to
/// get dusk lasting until midnight in one of them and not the other.
fn straddle(times: &[u32], values: usize, time: u32) -> Option<(usize, usize, f32)> {
    if times.is_empty() || values != times.len() {
        return None;
    }
    if times.len() == 1 {
        return Some((0, 0, 0.0));
    }
    let time = time % DAY;
    // The last entry to start at or before `time`; if none does, `time` is
    // before the first, so the pair is (last, first) across midnight.
    let (i, j, from, span) = match times.iter().rposition(|&t| t <= time) {
        Some(i) if i + 1 < times.len() => (i, i + 1, times[i], times[i + 1] - times[i]),
        Some(i) => (i, 0, times[i], DAY - times[i] + times[0]),
        None => {
            let last = times.len() - 1;
            (last, 0, times[last], DAY - times[last] + times[0])
        }
    };
    let elapsed = if time >= from {
        time - from
    } else {
        DAY - from + time
    };
    let t = if span == 0 {
        0.0
    } else {
        (elapsed as f32 / span as f32).clamp(0.0, 1.0)
    };
    Some((i, j, t))
}

/// One `LightIntBand` / `LightFloatBand` table, by row id.
///
/// Both files are `id, entryCount, time[16], value[16]` and differ only in how
/// the sixteen values are read, so the trimming and the 16-entry cap are stated
/// once. `read` is the cell accessor — `u32_at` for the colours, `f32_at` for
/// the distances.
fn read_band_table<T>(
    dbc: &Dbc,
    read: impl Fn(&Dbc, usize, usize) -> Option<T>,
) -> std::collections::HashMap<u32, (Vec<u32>, Vec<T>)> {
    let mut rows = std::collections::HashMap::new();
    for record in 0..dbc.record_count {
        let Some(id) = dbc.u32_at(record, 0) else {
            continue;
        };
        let count = dbc.u32_at(record, 1).unwrap_or(0).min(16) as usize;
        let times: Vec<u32> = (0..count)
            .filter_map(|i| dbc.u32_at(record, 2 + i))
            .collect();
        let values: Vec<T> = (0..count)
            .filter_map(|i| read(dbc, record, 18 + i))
            .collect();
        rows.insert(id, (times, values));
    }
    rows
}

/// A packed `LightIntBand` colour: `0x00RRGGBB`.
///
/// Read off the files rather than assumed — map 0's ocean-shallow band at noon is
/// `0x6182F7`, which is (97, 130, 247) as red-first and (247, 130, 97) the other
/// way round. Ocean water is the first of those.
fn unpack(colour: u32) -> [f32; 3] {
    [
        ((colour >> 16) & 0xFF) as f32 / 255.0,
        ((colour >> 8) & 0xFF) as f32 / 255.0,
        (colour & 0xFF) as f32 / 255.0,
    ]
}

#[cfg(test)]
mod tests;
