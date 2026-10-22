//! The shading brush: the vertex colours painted onto the ground, `MCCV`.

use crate::adt::{heights, AdtFile, Region};
use vale_assets::world::adt::CHUNK_SIZE;
use super::Edit;
use super::brush::{Falloff, Shape};
use super::paint::Working;

/// What a shading stroke does to the vertices under it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shade {
    /// Toward the brush's own colour.
    Paint,
    /// …toward white, which in `MCCV`'s scale is *twice* the light rather than
    /// all of it — see [`vale_assets::world::adt::decode_colours`].
    Lighten,
    /// …and toward black.
    Darken,
    /// Toward the mean of what is under the brush, which is what takes a hard
    /// edge out without deciding what the colour should be.
    Smooth,
    /// Back to neutral, which for a chunk that ends up wholly neutral means
    /// **taking the region away** rather than filling it with 0x7F — see
    /// [`crate::adt::colours::clear`].
    Clear,
}

impl Shade {
    /// What to call it in an interface.
    pub fn name(self) -> &'static str {
        match self {
            Shade::Paint => "Paint",
            Shade::Lighten => "Lighten",
            Shade::Darken => "Darken",
            Shade::Smooth => "Smooth",
            Shade::Clear => "Clear",
        }
    }
}

/// A brush that paints `MCCV`.
///
/// ## It shares the height brush's footprint and nothing else
///
/// [`Shape`], [`Falloff`] and the core are the same three controls over the same
/// two numbers, and they are *called* rather than restated: a shading brush that
/// had its own idea of what a square footprint is would be a second answer to a
/// question [`Shape::distance`] already answers. What differs is everything past
/// the weight — the value moved is four bytes rather than one float, and it is
/// clamped to a range with a meaningful midpoint.
///
/// ## A stroke cannot work in the file's own numbers
///
/// The same fault the texture brush has and for the same reason, one region
/// along: a byte is 256 levels, so a step of a third of a level is a step that
/// rounds away and never happens. At the strengths a person actually paints
/// shading at — a slow darkening under a tree — every frame's step is a fraction
/// of a byte. So a stroke accumulates in [`Working`]'s `f32` copy and the file is
/// written from it, quantised once on the way out. See [`Working`], where the
/// argument is made at length about alpha.
///
/// ## Alpha is painted and is not read by anything here
///
/// `CImVector`'s fourth byte has no meaning this renderer uses — the shading is
/// the `rgb` product and nothing samples the alpha — so it is written neutral
/// and left there. It is kept in the record rather than dropped because the
/// region has a fixed length either way and a later version of this does have a
/// use for it (`MCLV`'s lighting pass), and a file written with a garbage byte
/// in it is a file that has to be rewritten then.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shading {
    pub radius: f32,
    /// How much of the distance to the target is closed per second at full
    /// weight, 0..1-ish. The same shape
    /// [`Mode::Flatten`](super::brush::Mode::Flatten)'s strength has.
    pub strength: f32,
    pub falloff: Falloff,
    pub core: f32,
    pub shape: Shape,
    pub mode: Shade,
    /// What [`Shade::Paint`] paints toward, as the multiplier a person chose:
    /// 1.0 is neutral, 0 is black, 2 is the brightest the byte can say.
    pub colour: [f32; 3],
}

impl Default for Shading {
    fn default() -> Shading {
        Shading {
            radius: 20.0,
            // A quarter of the way to the target per second at full weight:
            // slow enough that a person can stop where they meant to, which is
            // the whole difficulty of painting light.
            strength: 0.25,
            falloff: Falloff::Smooth,
            core: 0.0,
            shape: Shape::Circle,
            mode: Shade::Paint,
            // Neutral, so a brush that has not been given a colour paints
            // nothing rather than painting mid-grey over the world.
            colour: [1.0, 1.0, 1.0],
        }
    }
}

/// The largest multiplier `MCCV` can state: 255 / 127.
pub const SHADE_MAX: f32 = 255.0 / crate::adt::colours::NEUTRAL as f32;

impl Shading {
    /// Apply one step of the stroke, and return the edits it made.
    ///
    /// `working` is the stroke's own `f32` copy — see [`Working`]. `level` is
    /// what [`Shade::Smooth`] converges on, handed in for the reason the height
    /// brush's is: a mean taken per chunk is a different colour in every chunk
    /// and leaves a step at every 33-yard boundary.
    pub fn stroke(
        &self,
        tile: &mut AdtFile,
        at: [f32; 2],
        seconds: f32,
        working: &mut Working,
        level: Option<[f32; 3]>,
    ) -> Vec<Edit> {
        let mut edits = Vec::new();
        for index in self.chunks_under(tile, at) {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            let before = crate::adt::colours::capture(tile, index);
            let live = working.shading(index, || read_shading(tile, index));
            let mut moved = false;
            for i in 0..vale_assets::world::adt::HEIGHTS_PER_CHUNK {
                let (dx, dy) = heights::vertex_offset(i);
                let (x, y) = (origin[0] - dx, origin[1] - dy);
                let w = self
                    .falloff
                    .over(self.shape.distance(x - at[0], y - at[1]) / self.radius, self.core);
                if w == 0.0 {
                    continue;
                }
                let step = (self.strength * w * seconds).min(1.0);
                let target = match self.mode {
                    Shade::Paint => self.colour,
                    Shade::Lighten => [SHADE_MAX; 3],
                    Shade::Darken => [0.0; 3],
                    Shade::Clear => [1.0; 3],
                    Shade::Smooth => match level {
                        Some(level) => level,
                        None => continue,
                    },
                };
                for k in 0..3 {
                    let was = live[i][k];
                    let now = (was + (target[k] - was) * step).clamp(0.0, SHADE_MAX);
                    if now != was {
                        live[i][k] = now;
                        moved = true;
                    }
                }
            }
            if !moved {
                continue;
            }
            let quantised = quantise(live);
            let Some(chunk) = tile.chunk_mut(index) else {
                continue;
            };
            crate::adt::colours::set_all(&mut *chunk, &quantised);
            // **A chunk painted back to neutral loses the region.** Anything
            // else leaves 580 bytes behind on a tile that was shipped without
            // them, which is the difference between a file somebody edited and
            // a file somebody edited *and then undid*.
            if crate::adt::colours::is_neutral(chunk) {
                crate::adt::colours::clear(chunk);
            }
            let after = crate::adt::colours::capture(tile, index);
            if after != before {
                edits.push(Edit::Colours {
                    chunk: index,
                    before,
                    after,
                });
            }
        }
        edits
    }

    /// **The two halves of the mean shade under this brush, in one tile**: the
    /// weighted sum and the total weight.
    ///
    /// Kept apart for the reason `Brush::weighed` keeps its apart — the caller
    /// adds up several tiles before dividing, because a brush can span two
    /// files and [`Shade::Smooth`] has to pull both toward one colour.
    pub fn weighed(&self, tile: &AdtFile, at: [f32; 2]) -> ([f64; 3], f64) {
        let mut sum = [0.0f64; 3];
        let mut weight = 0.0f64;
        for index in self.chunks_under(tile, at) {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            let live = read_shading(tile, index);
            for i in 0..vale_assets::world::adt::HEIGHTS_PER_CHUNK {
                let (dx, dy) = heights::vertex_offset(i);
                let (x, y) = (origin[0] - dx, origin[1] - dy);
                let w = self
                    .falloff
                    .over(self.shape.distance(x - at[0], y - at[1]) / self.radius, self.core);
                if w == 0.0 {
                    continue;
                }
                for k in 0..3 {
                    sum[k] += f64::from(live[i][k]) * f64::from(w);
                }
                weight += f64::from(w);
            }
        }
        (sum, weight)
    }

    /// The chunks whose square the brush's bounding box reaches.
    fn chunks_under(&self, tile: &AdtFile, at: [f32; 2]) -> Vec<usize> {
        (0..tile.chunks.len())
            .filter(|&i| {
                let Some(chunk) = tile.chunk(i) else {
                    return false;
                };
                let origin = chunk.head().position();
                let near =
                    |o: f32, p: f32| p >= o - CHUNK_SIZE - self.radius && p <= o + self.radius;
                near(origin[0], at[0]) && near(origin[1], at[1])
            })
            .collect()
    }
}

/// One chunk's shading as multipliers, neutral where it has no region.
fn read_shading(tile: &AdtFile, index: usize) -> Vec<[f32; 3]> {
    let neutral = vec![[1.0f32; 3]; vale_assets::world::adt::HEIGHTS_PER_CHUNK];
    let Some(chunk) = tile.chunk(index) else {
        return neutral;
    };
    match chunk.region(Region::Colours) {
        None => neutral,
        Some(mccv) => vale_assets::world::adt::decode_colours(&mccv.data)
            .into_iter()
            .map(|c| [c[0], c[1], c[2]])
            .collect(),
    }
}

/// …and back into the file's bytes, once, on the way out.
///
/// The alpha is written neutral — see [`Shading`], where the reason it is
/// written at all is.
fn quantise(live: &[[f32; 3]]) -> Vec<crate::adt::colours::Colour> {
    live.iter()
        .map(|c| {
            let byte = |v: f32| {
                (v * crate::adt::colours::NEUTRAL as f32)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            [byte(c[0]), byte(c[1]), byte(c[2]), crate::adt::colours::NEUTRAL]
        })
        .collect()
}
