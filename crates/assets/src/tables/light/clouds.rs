//! The cloud layer: a texture of value noise, thresholded by the light's cloud
//! density, coloured by three of the light's bands, and drawn on a shallow dome
//! over the camera.
//!
//! The 1.12.1 client ships no cloud texture. It generates one at run time, and
//! `LightParams.cloudType` is 0 in all 426 shipped rows, so every sky with
//! clouds uses this one generator. This module is that generator; the renderer
//! uploads [`CloudField::rgba`] and draws [`dome`].
//!
//! ## What a texel is
//!
//! A texel's coverage is the sum of four octaves of value noise, each octave
//! twice the frequency and half the amplitude of the one before, over a
//! lattice that repeats every 256 cells. The sum is mapped to a byte
//! (`sum * 64 + 128`), the light's cover is subtracted from it, and what
//! remains goes through a fixed exponential curve: the texel's alpha. See
//! [`CloudField::rebuild`].
//!
//! Its colour is `shade * band 11 + band 12`, where `shade` falls from 191/255
//! for a thin texel to 64/255 for a thick one, plus `band 10` times how far
//! the texel's surface faces the sun (the moon at night). The surface normal is
//! the slope of the first three octaves. See [`CloudLight`].
//!
//! ## How it changes over time
//!
//! The noise has a third axis that advances by 1/256 of a lattice cell each
//! time the whole texture has been regenerated. The client regenerates
//! [`ROWS_PER_TICK`] rows every [`TICK_SECONDS`], so at 128 rows the texture
//! is rebuilt every 0.4 seconds and a cloud changes shape over about 100
//! seconds. The texture does not scroll; the clouds change in place.
//!
//! ## Which parts are this client's own
//!
//! The client fills its permutation table and its 256 random values at run
//! time. This module builds both from a fixed seed, so the clouds are the
//! client's in shape and statistics, not texel for texel. The octave offsets in
//! [`OCTAVE_OFFSET`] are also this client's; they keep the four octaves from
//! sharing a lattice origin.

use super::celestial::{self, DayTrack};

/// The texture is this many texels across at each value of the
/// `SkyCloudLOD` CVar, which the client clamps to 0 or 1.
pub const SIZES: [usize; 2] = [128, 256];

/// How many octaves of noise a texel sums.
pub const OCTAVES: usize = 4;

/// Each octave's step per texel, in 1/256 of a lattice cell, for each
/// `SkyCloudLOD`. At either level of detail the four octaves span 8, 16, 32 and
/// 64 cells across the texture; the larger texture has the finer step.
const STEPS: [[u32; OCTAVES]; 2] = [[16, 32, 64, 128], [8, 16, 32, 64]];

/// How many rows one regeneration rewrites.
pub const ROWS_PER_TICK: usize = 32;

/// How long the client waits between two regenerations, in seconds.
pub const TICK_SECONDS: f32 = 0.1;

/// How a texel's noise sum becomes a byte: `sum * NOISE_GAIN + NOISE_BIAS`.
/// The four octaves sum to at most 1.875 either way, so the byte stays inside
/// 8..248.
const NOISE_GAIN: f32 = 64.0;
const NOISE_BIAS: f32 = 128.0;

/// The base of the exponential curve a texel's density goes through:
/// `255 - 255 * SHARPNESS^x`.
const SHARPNESS: f32 = 0.96;

/// The cover the curve's step is computed for. The client builds the curve
/// once, at start-up, while its cover still holds this default, and does not
/// rebuild it when the light's cover changes.
const CURVE_COVER: f32 = 0.6;

/// Added to each octave's lattice coordinates, in cells. This client's own;
/// see the module note.
const OCTAVE_OFFSET: [u32; OCTAVES] = [0, 61, 122, 183];

/// How high the light stands above the texture plane, in texels, in clear
/// weather, and how much a storm adds to it. A higher light lights every
/// texel more evenly, so a storm flattens the clouds' relief.
const LIGHT_HEIGHT: f32 = 64.0;
const LIGHT_HEIGHT_STORM: f32 = 192.0;

/// How much of the sunlit colour a full storm takes away.
const HIGHLIGHT_STORM_LOSS: f32 = 0.75;

/// Between these two day fractions the clouds are lit by the sun, and outside
/// them by the moon: 04:50 and 22:10.
pub const SUN_FROM: f32 = 0.201_388_9;
pub const SUN_UNTIL: f32 = 0.923_611_1;

/// How strongly band 10 lights the clouds over the day.
///
/// The keys are the client's, in the client's order, and they are not
/// ascending: the last two (21:20 and 22:00) lie before the sixth (22:10).
/// [`DayTrack::at`] takes the first key at or after the hour, as the client
/// does, so the last two only matter as the key before midnight. The result:
/// full until 04:00, out by 04:40, back from 04:50 to 05:30, full until 21:30,
/// out by 22:10, and full again from 22:10, when the moon takes over.
pub const HIGHLIGHT: DayTrack = DayTrack {
    keys: &[
        (0.166_666_7, 1.0), // 04:00
        (0.194_444_4, 0.0), // 04:40
        (0.201_388_9, 0.0), // 04:50
        (0.229_166_7, 1.0), // 05:30
        (0.895_833_3, 1.0), // 21:30
        (0.923_611_1, 0.0), // 22:10
        (0.888_888_9, 0.0), // 21:20
        (0.916_666_7, 1.0), // 22:00
    ],
};

/// What lights the cloud layer this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloudLight {
    /// How much of the sky is covered, 0 to 1: `LightFloatBand` band 3.
    /// 0 is no clouds at all.
    pub density: f32,
    /// Band 10, added where a texel faces the sun or the moon. sRGB, 0..1.
    pub highlight: [f32; 3],
    /// Band 11, scaled by how thin the texel is.
    pub shade: [f32; 3],
    /// Band 12, added to every texel.
    pub base: [f32; 3],
    /// The weather's grade, 0 for clear and 1 for the heaviest storm.
    pub storm: f32,
    /// The hour, in half-minutes past midnight.
    pub half_minutes: u32,
}

impl CloudLight {
    /// No clouds.
    pub const NONE: CloudLight = CloudLight {
        density: 0.0,
        highlight: [0.0; 3],
        shade: [0.0; 3],
        base: [0.0; 3],
        storm: 0.0,
        half_minutes: super::NOON,
    };

    /// The direction toward whatever lights the clouds at this hour, in the
    /// world's axes: the sun between [`SUN_FROM`] and [`SUN_UNTIL`], the moon
    /// (not the blue moon) otherwise.
    pub fn toward(&self) -> [f32; 3] {
        let t = celestial::day_fraction(self.half_minutes);
        if (SUN_FROM..=SUN_UNTIL).contains(&t) {
            celestial::SUN.direction(t)
        } else {
            celestial::MOON.direction(t)
        }
    }
}

/// The cloud texture, and the state that regenerates it a band of rows at a
/// time.
pub struct CloudField {
    size: usize,
    /// `log2(size) - 7`: the slope of the first three octaves is scaled by
    /// `1 << this`, so the normals are the same at either level of detail.
    slope_shift: u32,
    steps: [u32; OCTAVES],
    /// One byte per texel: the alpha.
    density: Vec<u8>,
    /// One slope per texel, x then y.
    normals: Vec<[f32; 2]>,
    /// The sum of the first three octaves along the row above the one being
    /// generated, which the y slope is measured against.
    above: Vec<f32>,
    rgba: Vec<[u8; 4]>,
    /// The next row to regenerate.
    row: usize,
    /// The noise's third axis, in 1/256 of a cell.
    morph: u32,
    /// Seconds until the next regeneration.
    wait: f32,
    /// Whether the whole texture has been generated once.
    built: bool,
    curve: [u8; 256],
    lattice: Lattice,
}

impl CloudField {
    /// An empty field at a `SkyCloudLOD` level of detail; values above 1 are
    /// read as 1, as the client clamps the CVar.
    pub fn new(lod: u32) -> CloudField {
        let lod = lod.min(1) as usize;
        let size = SIZES[lod];
        CloudField {
            size,
            slope_shift: size.trailing_zeros() - 7,
            steps: STEPS[lod],
            density: vec![0; size * size],
            normals: vec![[0.0; 2]; size * size],
            above: vec![0.0; size],
            rgba: vec![[0; 4]; size * size],
            row: 0,
            morph: 0,
            wait: 0.0,
            built: false,
            curve: curve(),
            lattice: Lattice::new(),
        }
    }

    /// Texels across, which is also rows down.
    pub fn size(&self) -> usize {
        self.size
    }

    /// The texture, row by row, as RGBA bytes. Row `r` is texture coordinate
    /// `v = r / size`; see [`dome`].
    pub fn rgba(&self) -> &[[u8; 4]] {
        &self.rgba
    }

    /// Generate every row now.
    pub fn rebuild(&mut self, light: &CloudLight) {
        self.row = 0;
        while self.row < self.size {
            self.generate_rows(light);
        }
        self.row = 0;
        self.built = true;
    }

    /// Advance the clock by `seconds`, and regenerate the next band of rows if
    /// a tick is due. Returns the rows rewritten, or `None` when nothing was.
    ///
    /// The first call builds the whole texture and returns every row. After
    /// that one band of [`ROWS_PER_TICK`] rows is rewritten every
    /// [`TICK_SECONDS`], and the noise's third axis moves on once the last band
    /// has been rewritten.
    pub fn advance(&mut self, seconds: f32, light: &CloudLight) -> Option<std::ops::Range<usize>> {
        if !self.built {
            self.rebuild(light);
            return Some(0..self.size);
        }
        self.wait -= seconds;
        if self.wait > 0.0 {
            return None;
        }
        self.wait = TICK_SECONDS;
        let first = self.row;
        self.generate_rows(light);
        let rows = first..self.row;
        if self.row >= self.size {
            self.row = 0;
            self.morph = self.morph.wrapping_add(1) & 0xFFFF;
        }
        Some(rows)
    }

    /// Rewrite [`ROWS_PER_TICK`] rows from [`Self::row`]: density and slope,
    /// then colour.
    fn generate_rows(&mut self, light: &CloudLight) {
        let end = (self.row + ROWS_PER_TICK).min(self.size);
        // The light's cover as the byte subtracted from every texel's noise.
        let cover = ((1.0 - light.density.clamp(0.0, 1.0)) * 255.0) as i32;
        let slope = (1u32 << self.slope_shift) as f32;
        for row in self.row..end {
            // The left neighbour of column 0 is taken as zero.
            let mut left = 0.0f32;
            for col in 0..self.size {
                let mut sum = 0.0f32;
                let mut amplitude = 1.0f32;
                for octave in 0..OCTAVES {
                    let step = self.steps[octave];
                    let offset = OCTAVE_OFFSET[octave] << 8;
                    let x = col as u32 * step + offset;
                    let y = row as u32 * step + offset;
                    sum += amplitude * self.lattice.sample(x, y, self.morph);
                    amplitude *= 0.5;
                    if octave == 2 {
                        let at = row * self.size + col;
                        self.normals[at] = [(left - sum) * slope, (self.above[col] - sum) * slope];
                        self.above[col] = sum;
                        left = sum;
                    }
                }
                let byte = (sum * NOISE_GAIN + NOISE_BIAS).floor().clamp(0.0, 255.0) as i32;
                self.density[row * self.size + col] = match byte - cover {
                    over if over >= 0 => self.curve[over.min(255) as usize],
                    _ => 0,
                };
            }
        }
        self.colour_rows(self.row..end, light);
        self.row = end;
    }

    /// Colour the given rows from their density and slope.
    fn colour_rows(&mut self, rows: std::ops::Range<usize>, light: &CloudLight) {
        let size = self.size as f32;
        let [u, v] = texel_toward(light.toward());
        let (sun_x, sun_y) = (u * size, v * size);
        let height = LIGHT_HEIGHT + LIGHT_HEIGHT_STORM * light.storm.clamp(0.0, 1.0);
        let t = celestial::day_fraction(light.half_minutes);
        let lit = HIGHLIGHT.at_fraction(t) * (1.0 - HIGHLIGHT_STORM_LOSS * light.storm.clamp(0.0, 1.0));
        for row in rows {
            for col in 0..self.size {
                let at = row * self.size + col;
                let density = self.density[at];
                if density == 0 {
                    // A clear texel keeps its left neighbour's colour, so that
                    // filtering at the edge of a cloud blends toward the
                    // cloud's colour rather than toward black.
                    let rgb = if col > 0 { self.rgba[at - 1] } else { [0; 4] };
                    self.rgba[at] = [rgb[0], rgb[1], rgb[2], 0];
                    continue;
                }
                let shade = f32::from(((255 - density) >> 1) + 64) / 255.0;
                let mut rgb = [0.0f32; 3];
                for k in 0..3 {
                    rgb[k] = light.shade[k] * shade + light.base[k];
                }
                let to_light = [sun_x - col as f32, sun_y - row as f32, height];
                let [nx, ny] = self.normals[at];
                let normal = [nx, ny, 1.0];
                let dot: f32 = (0..3).map(|k| to_light[k] * normal[k]).sum();
                let lengths = length(to_light) * length(normal);
                if lengths > 0.0 {
                    let facing = dot / lengths;
                    if facing > 0.0 {
                        for k in 0..3 {
                            rgb[k] += light.highlight[k] * facing * lit;
                        }
                    }
                }
                let byte = |c: f32| (c.min(1.0) * 255.0).floor().max(0.0) as u8;
                self.rgba[at] = [byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), density];
            }
        }
    }
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// `255 - 255 * SHARPNESS^(i * step)` for each byte `i` past the cover, with
/// the step `(255 - cover) / 256` of the cover the client starts with.
fn curve() -> [u8; 256] {
    let cover = ((1.0 - CURVE_COVER) * 255.0) as u32;
    let step = (255 - cover) as f32 / 256.0;
    let mut out = [0u8; 256];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = (255.0 - 255.0 * SHARPNESS.powf(i as f32 * step)) as u8;
    }
    out
}

/// A 256-cell lattice of random values in -1..1, repeating in all three axes,
/// interpolated with a raised cosine.
struct Lattice {
    permutation: [u8; 256],
    values: [f32; 256],
    /// `(1 - cos(pi * i / 256)) / 2`: the weight of the far corner at a
    /// fraction of `i / 256` of a cell.
    fade: [f32; 256],
}

impl Lattice {
    fn new() -> Lattice {
        // A fixed seed, so that two runs draw the same clouds.
        let mut state: u32 = 0x1234_5678;
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state >> 8
        };
        let mut permutation: [u8; 256] = std::array::from_fn(|i| i as u8);
        for i in (1..256).rev() {
            let j = (next() as usize) % (i + 1);
            permutation.swap(i, j);
        }
        let values = std::array::from_fn(|_| 1.0 - 2.0 * (next() & 0x7FFF) as f32 / 32_768.0);
        let fade = std::array::from_fn(|i| {
            (1.0 - (std::f32::consts::PI * i as f32 / 256.0).cos()) * 0.5
        });
        Lattice {
            permutation,
            values,
            fade,
        }
    }

    fn at(&self, x: u32, y: u32, z: u32) -> f32 {
        let p = |i: u32| u32::from(self.permutation[(i & 255) as usize]);
        self.values[p(x + p(y + p(z))) as usize]
    }

    /// The value at a point given in 1/256 of a cell on each axis.
    fn sample(&self, x: u32, y: u32, z: u32) -> f32 {
        let (xi, yi, zi) = (x >> 8, y >> 8, z >> 8);
        let fx = self.fade[(x & 255) as usize];
        let fy = self.fade[(y & 255) as usize];
        let fz = self.fade[(z & 255) as usize];
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let plane = |z: u32| {
            let near = lerp(self.at(xi, yi, z), self.at(xi + 1, yi, z), fx);
            let far = lerp(self.at(xi, yi + 1, z), self.at(xi + 1, yi + 1, z), fx);
            lerp(near, far, fy)
        };
        lerp(plane(zi), plane(zi + 1), fz)
    }
}

/// The dome the texture is drawn on.
///
/// A cap of a unit sphere whose centre is [`DROP`] below the camera, cut at a
/// polar angle of 45 degrees, so its rim lies on the horizon plane. Twelve
/// rings of [`SEGMENTS`] vertices; ring 0 is the zenith. The texture is mapped
/// as a disc: ring `r` lies on the circle of radius `r / 22` around the
/// texture's centre, so the rim ring touches the texture's edge.
pub mod dome {
    /// Each ring's polar angle about the dome's centre, as a fraction of pi.
    pub const RING_POLAR: [f32; 12] = [
        0.0, 0.025, 0.05, 0.075, 0.1, 0.125, 0.15, 0.175, 0.205, 0.23, 0.245, 0.25,
    ];

    /// Each ring's vertex alpha: opaque to ring 8, half at ring 9, clear at the
    /// last two, so the clouds fade out above the horizon.
    pub const RING_ALPHA: [u8; 12] = [255, 255, 255, 255, 255, 255, 255, 255, 255, 128, 0, 0];

    /// Vertices per ring.
    pub const SEGMENTS: usize = 16;

    /// How far below the camera the dome's centre is: `cos(pi / 4)`, which
    /// puts the rim at height zero.
    pub const DROP: f32 = std::f32::consts::FRAC_1_SQRT_2;

    /// One vertex: a position on the unit dome in the world's axes (+X north,
    /// +Y west, +Z up), a texture coordinate, and an alpha.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Vertex {
        pub position: [f32; 3],
        pub uv: [f32; 2],
        pub alpha: f32,
    }

    /// Every vertex, ring by ring.
    pub fn vertices() -> Vec<Vertex> {
        let mut out = Vec::with_capacity(RING_POLAR.len() * SEGMENTS);
        let last = (RING_POLAR.len() - 1) as f32;
        for (ring, (&polar, &alpha)) in RING_POLAR.iter().zip(RING_ALPHA.iter()).enumerate() {
            let phi = polar * std::f32::consts::PI;
            let radius = ring as f32 / last * 0.5;
            for segment in 0..SEGMENTS {
                let theta = segment as f32 * std::f32::consts::TAU / SEGMENTS as f32;
                out.push(Vertex {
                    position: [
                        phi.sin() * theta.sin(),
                        phi.sin() * theta.cos(),
                        phi.cos() - DROP,
                    ],
                    uv: [0.5 + radius * theta.sin(), 0.5 + radius * theta.cos()],
                    alpha: f32::from(alpha) / 255.0,
                });
            }
        }
        out
    }

    /// The triangles, as indices into [`vertices`].
    pub fn indices() -> Vec<u32> {
        let mut out = Vec::new();
        let ring = |r: usize, s: usize| (r * SEGMENTS + s % SEGMENTS) as u32;
        for r in 0..RING_POLAR.len() - 1 {
            for s in 0..SEGMENTS {
                let (a, b) = (ring(r, s), ring(r, s + 1));
                let (c, d) = (ring(r + 1, s), ring(r + 1, s + 1));
                out.extend([a, c, b, b, c, d]);
            }
        }
        out
    }
}

/// Where a direction from the camera meets the dome, as a texture coordinate.
///
/// The ray is intersected with the dome's sphere, and the hit's polar angle
/// about the sphere's centre, capped at the rim's, sets its distance from the
/// texture's centre: half the texture's width at the rim. A direction below
/// the rim lands on the rim. The colouring uses this for the sun and the moon.
///
/// The mesh's own mapping in [`dome`] spaces its rings by index rather than by
/// angle, so the two agree exactly only in the eight evenly spaced inner rings.
/// The client draws with the same mismatch.
pub fn texel_toward(direction: [f32; 3]) -> [f32; 2] {
    let len = length(direction);
    if len <= 0.0 {
        return [0.5, 0.5];
    }
    let d = direction.map(|c| c / len);
    let drop = dome::DROP;
    // |t d - c| = 1 with c = (0, 0, -drop): t^2 + 2 t drop d.z + drop^2 - 1 = 0.
    let b = drop * d[2];
    let t = -b + (b * b + 1.0 - drop * drop).sqrt();
    let hit = d.map(|c| c * t);
    let rim = dome::RING_POLAR[dome::RING_POLAR.len() - 1] * std::f32::consts::PI;
    let polar = (hit[2] + drop).clamp(-1.0, 1.0).acos().min(rim);
    let radius = polar / rim * 0.5;
    let across = (hit[0] * hit[0] + hit[1] * hit[1]).sqrt();
    if across < 1e-5 {
        return [0.5, 0.5];
    }
    [0.5 + radius * hit[0] / across, 0.5 + radius * hit[1] / across]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(density: f32) -> CloudLight {
        CloudLight {
            density,
            highlight: [1.0, 0.8, 0.5],
            shade: [0.2, 0.4, 0.5],
            base: [0.0, 0.0, 0.0],
            storm: 0.0,
            half_minutes: super::super::NOON,
        }
    }

    fn covered(field: &CloudField) -> f32 {
        let opaque = field.rgba().iter().filter(|t| t[3] > 0).count();
        opaque as f32 / field.rgba().len() as f32
    }

    #[test]
    fn no_density_is_no_clouds_and_more_density_is_more_sky_covered() {
        let mut field = CloudField::new(0);
        field.rebuild(&light(0.0));
        assert_eq!(covered(&field), 0.0);
        field.rebuild(&light(0.5));
        let clear = covered(&field);
        field.rebuild(&light(0.95));
        let storm = covered(&field);
        assert!(clear > 0.05 && clear < 0.95, "clear sky covers {clear}");
        assert!(storm > clear, "storm {storm} against clear {clear}");
        assert!(storm > 0.9, "a 0.95 density covers almost the whole sky: {storm}");
    }

    #[test]
    fn the_curve_starts_at_zero_and_rises_toward_opaque() {
        let curve = curve();
        assert_eq!(curve[0], 0);
        for i in 1..256 {
            assert!(curve[i] >= curve[i - 1]);
        }
        assert!(curve[255] > 240, "{}", curve[255]);
    }

    #[test]
    fn one_tick_rewrites_one_band_of_rows_and_the_third_axis_moves_after_the_last() {
        let mut field = CloudField::new(0);
        let lit = light(0.5);
        assert_eq!(field.advance(0.0, &lit), Some(0..128), "the first call builds everything");
        assert_eq!(field.advance(0.05, &lit), Some(0..32), "the first tick is due at once");
        assert_eq!(field.advance(0.05, &lit), None, "half a tick later nothing is due");
        assert_eq!(field.advance(0.06, &lit), Some(32..64));
        assert_eq!(field.morph, 0);
        field.advance(0.1, &lit);
        field.advance(0.1, &lit);
        assert_eq!(field.morph, 1, "the fourth band ends the pass");
        assert_eq!(field.row, 0);
    }

    #[test]
    fn the_larger_level_of_detail_doubles_the_texture() {
        assert_eq!(CloudField::new(0).size(), 128);
        assert_eq!(CloudField::new(1).size(), 256);
        assert_eq!(CloudField::new(7).size(), 256, "the CVar is clamped to 1");
    }

    #[test]
    fn straight_up_is_the_centre_of_the_texture_and_the_horizon_its_edge() {
        assert_eq!(texel_toward([0.0, 0.0, 1.0]), [0.5, 0.5]);
        let [u, v] = texel_toward([1.0, 0.0, 0.0]);
        assert!((u - 1.0).abs() < 1e-4 && (v - 0.5).abs() < 1e-4, "{u} {v}");
        let [u, v] = texel_toward([0.0, 1.0, -0.5]);
        assert!((u - 0.5).abs() < 1e-4 && (v - 1.0).abs() < 1e-4, "below the rim lands on it: {u} {v}");
    }

    #[test]
    fn the_dome_is_twelve_rings_whose_rim_is_on_the_horizon_and_clear() {
        let vertices = dome::vertices();
        assert_eq!(vertices.len(), 12 * dome::SEGMENTS);
        let rim = &vertices[11 * dome::SEGMENTS];
        assert!(rim.position[2].abs() < 1e-5, "{:?}", rim.position);
        assert_eq!(rim.alpha, 0.0);
        assert_eq!(vertices[0].position, [0.0, 0.0, 1.0 - dome::DROP]);
        assert_eq!(dome::indices().len(), 11 * dome::SEGMENTS * 6);
    }

    #[test]
    fn the_sun_lights_the_clouds_by_day_and_the_moon_by_night() {
        let at = |hour: u32, minute: u32| CloudLight {
            half_minutes: (hour * 60 + minute) * 2,
            ..CloudLight::NONE
        };
        let noon = celestial::day_fraction(at(12, 0).half_minutes);
        assert_eq!(at(12, 0).toward(), celestial::SUN.direction(noon));
        let midnight = celestial::day_fraction(0);
        assert_eq!(at(0, 0).toward(), celestial::MOON.direction(midnight));
        let late = celestial::day_fraction(at(22, 30).half_minutes);
        assert_eq!(at(22, 30).toward(), celestial::MOON.direction(late));
    }

    #[test]
    fn the_highlight_is_out_around_dawn_and_after_sunset_and_full_otherwise() {
        let at = |hour: u32, minute: u32| HIGHLIGHT.at((hour * 60 + minute) * 2);
        assert_eq!(at(2, 0), 1.0);
        assert_eq!(at(4, 45), 0.0);
        assert!((at(5, 10) - 0.5).abs() < 0.01, "{}", at(5, 10));
        assert_eq!(at(12, 0), 1.0);
        assert!((at(21, 50) - 0.5).abs() < 0.01, "{}", at(21, 50));
        assert_eq!(at(23, 0), 1.0, "the moon lights them fully after 22:10");
    }

    #[test]
    fn a_clear_texel_keeps_its_neighbours_colour_with_no_alpha() {
        let mut field = CloudField::new(0);
        field.rebuild(&light(0.5));
        let rgba = field.rgba();
        let size = field.size();
        let clear = (1..size * size)
            .find(|&i| rgba[i][3] == 0 && i % size != 0 && rgba[i - 1][3] > 0)
            .expect("a clear texel after a cloudy one");
        assert_eq!(&rgba[clear][..3], &rgba[clear - 1][..3]);
    }
}
