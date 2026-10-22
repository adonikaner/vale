//! The paint brush: texture layers painted onto the ground, their blend maps
//! in `MCAL`, and what a stroke could not paint.

use crate::adt::{alpha, AdtFile};
use vale_assets::world::adt::{TextureLayer, ALPHA_LEN, ALPHA_SIDE, CHUNK_SIZE};
use super::{ChunkPaint, Edit};
use super::brush::{Falloff, Shape, wobble};

/// What one step of a paint stroke did, and what it could not do.
///
/// **A stroke reports its refusals rather than returning fewer edits**, and that
/// is not tidiness. A chunk already carrying [`alpha::MAX_LAYERS`] textures
/// cannot take another, and about half the chunks in the shipped tiles are
/// already at four — measured: 144 of 256 on `Azeroth_32_48`, 194 of 256 on
/// `Azeroth_34_51`, 120 of 256 on `Kalimdor_30_41`. A brush that simply painted
/// nothing there stops dead part-way across the ground with no error anywhere,
/// which is what it was reported as: *"you can be painting a texture and
/// suddenly run into a chunk where it just stops"*.
#[derive(Debug, Default)]
pub struct Painted {
    pub edits: Vec<Edit>,
    /// **Which** chunks under the brush were full and do not already carry this
    /// texture — not how many. The caller wants to put one of them in front of
    /// the person painting, and a count cannot say which.
    /// See [`alpha::Paint::remove_layer`], which is the way out.
    pub full: Vec<usize>,
    /// …and which had **no texture at all** and have just been given this one as
    /// their base.
    ///
    /// Worth reporting for the same reason as a refusal, and the opposite one: a
    /// base cannot be partial, so the whole chunk becomes that texture however
    /// little of it the brush covered. That is the only thing an empty chunk can
    /// be given and it is not what a brush stroke usually means, so the caller
    /// says so rather than letting it look like the brush overshot.
    pub based: Vec<usize>,
    /// The chunks an erasing stroke reached whose base is the texture. The
    /// base has no blend map, so there is nothing to erase.
    pub base: Vec<usize>,
    /// The chunks a stroke confined to existing layers reached that do not
    /// carry the texture. See [`PaintBrush::existing_only`].
    pub absent: Vec<usize>,
    /// The full chunks in which a layer that showed almost nothing was given
    /// to the texture. See [`PaintBrush::reuse_hidden`].
    pub reused: Vec<usize>,
}

impl Painted {
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
            && self.full.is_empty()
            && self.based.is_empty()
            && self.base.is_empty()
            && self.absent.is_empty()
            && self.reused.is_empty()
    }
}

/// A stroke's own copy of the chunks it is over, at full precision.
///
/// ## A stroke cannot work in the numbers the file holds
///
/// A 4-bit chunk stores sixteen levels, so its alphas are multiples of 17. Read
/// the value, move it a little, write it back, read it again — and any step
/// smaller than half a quantum is a step that did not happen. That is the fault
/// the snap-away rule in the round before this one was written to fix, and
/// snapping is the wrong fix: it makes **every** step a whole quantum, so the
/// smallest possible stroke advances a fifteenth of the range per frame and the
/// strength setting controls nothing at all. Measured from the window: a quarter
/// of a second to full opacity at any strength.
///
/// So the stroke keeps its own copy of the texels it is working on, in `f32`,
/// seeded from the file the first time it touches a chunk. Every step
/// accumulates there and the file is written from it — quantised once, on the
/// way out, instead of once per frame. A step of a thousandth of a unit is a
/// step that is still there after a thousand frames.
///
/// Held by the caller for the length of one stroke and cleared when the button
/// comes up: it is a *stroke's* state, and a copy kept past the stroke that made
/// it is a copy that disagrees with the file the moment anything else writes.
#[derive(Debug, Default)]
pub struct Working {
    /// Per chunk: the blend maps as this stroke found them and as it has left
    /// them, and how much of the stroke each texel has taken. See [`Wet`].
    chunks: std::collections::HashMap<usize, Wet>,
    /// Per chunk, per vertex: the `MCCV` multiplier with the fraction kept.
    ///
    /// A second map rather than a second `Working`, because the two are the same
    /// idea over the same lifetime and a tool holds one of these per stroke. The
    /// shapes differ — 145 vertices against a layer's 4,096 texels — so they
    /// cannot share a map, and nothing wants them to: no stroke paints both.
    shading: std::collections::HashMap<usize, Vec<[f32; 3]>>,
}

/// One chunk's share of a stroke, at full precision.
///
/// The maps are worked out from `start` and `cover` rather than stepped
/// frame by frame. See [`PaintBrush`], where the reason is.
#[derive(Debug)]
struct Wet {
    /// Per layer, per texel: alpha in 0..255 as the stroke found it.
    start: Vec<Vec<f32>>,
    /// …and as the stroke has left it, with the fraction kept.
    now: Vec<Vec<f32>>,
    /// Per texel: how much of the stroke it has taken, 0 to 1.
    cover: Vec<f32>,
}

impl Wet {
    fn seeded(maps: &[Vec<u8>]) -> Wet {
        let start = as_working(maps);
        Wet {
            now: start.clone(),
            start,
            cover: vec![0.0; ALPHA_LEN],
        }
    }

    /// Give the chunk a layer the stroke has just added, transparent.
    fn grow_to(&mut self, layers: usize) {
        while self.start.len() < layers {
            self.start.push(vec![0.0; ALPHA_LEN]);
            self.now.push(vec![0.0; ALPHA_LEN]);
        }
    }
}

impl Working {
    /// Forget everything. Called when the button comes up.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.shading.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty() && self.shading.is_empty()
    }

    /// This stroke's copy of one chunk's shading, seeded from `read` the first
    /// time it is asked for.
    pub fn shading(
        &mut self,
        chunk: usize,
        read: impl FnOnce() -> Vec<[f32; 3]>,
    ) -> &mut Vec<[f32; 3]> {
        self.shading.entry(chunk).or_insert_with(read)
    }
}

/// A texture brush.
///
/// ## Painting a texture is making it the topmost thing at a texel
///
/// A chunk's layers are drawn in order over an opaque base: layer 1 over the
/// base at its own alpha, layer 2 over that, layer 3 over that. So the fraction
/// of a texel that ends up showing layer *k* is `a_k` times what the layers
/// **above** it leave — and making texture *k* visible means two things and not
/// one: raising `a_k`, and lowering every `a_j` for `j > k`. A brush that raised
/// only its own layer paints nothing at all under a layer that is already opaque
/// there, which reads as "the brush does not work on this half of the chunk".
///
/// Layers below *k* are left alone. They are hidden by *k* wherever it is
/// opaque, so moving them changes nothing visible and would destroy the paint a
/// person would see again if they took *k* away.
///
/// ## …and it is what puts a fourth texture on a chunk
///
/// A chunk that does not carry the texture yet is given it as a **new topmost
/// layer**, transparent everywhere, and then painted. That is the one thing this
/// brush does that changes the length of a region, and the one that a caller
/// with the chunk drawn has to notice — see [`Edit::changes_the_texture_set`].
/// A chunk already carrying [`alpha::MAX_LAYERS`] cannot take another and is
/// skipped rather than having one of its layers thrown away.
///
/// ## A stroke paints up to its footprint and no further
///
/// Each texel takes a share of the stroke, `cover`, which rises toward the
/// footprint's weight there at [`Self::strength`] and never past it. The maps
/// are the stroke's starting maps blended toward the texture by that share.
/// So a held brush settles on its own profile: full in the core, the falloff's
/// fraction at the edge.
///
/// The first version moved every texel toward full opacity and used the weight
/// only as the speed. A texel near the rim got there slowly, but it got there:
/// holding the brush, or passing over the same ground twice, painted out to the
/// whole radius. WoWEdit's brush keeps the paint about its inner ring with a
/// large outer one, and was reported as the more precise of the two. A new
/// stroke starts from the maps the last one left, so strokes still build on
/// each other.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintBrush {
    /// Yards.
    pub radius: f32,
    /// How much of the radius is held at **full strength** before the falloff
    /// starts, as a fraction — see [`Falloff::over`]. The height brush's own, for
    /// the same reason: how hard the middle of a stroke bites is a separate want
    /// from which curve it fades along.
    pub core: f32,
    /// The footprint — see [`Shape`]. The same three the height brush has, for
    /// the same reason: a road is painted with a square and a clearing with a
    /// circle, and neither wants the other.
    pub shape: Shape,
    /// A fraction of the remaining distance a second, so a held brush converges
    /// on its footprint rather than stepping to it. How far it goes is the
    /// footprint's; see the section above.
    pub strength: f32,
    pub falloff: Falloff,
    /// **The archive path of the tileset, not an `MTEX` index.**
    ///
    /// `MTEX` is per tile, so a stroke that crosses a border has two different
    /// numbers for one texture and a brush holding a number could only paint one
    /// side of it. Naming it here also puts [`AdtFile::name_texture`] *inside*
    /// the stroke, after the chunk's paint has been captured, which is what makes
    /// an undo take the appended name back out again.
    pub texture: String,
    /// **What a layer of this texture grows**: the `MCLY` `effectId` written
    /// on a layer this brush adds, a `GroundEffectTexture` row — see
    /// `vale_assets::tables::foliage`. Zero grows nothing, and zero is what
    /// every layer this brush wrote carried before the field existed, so
    /// painting grass over dirt painted grass and planted none. The caller
    /// sets it when the texture is chosen, from what the shipped ground pairs
    /// that texture with.
    pub effect_id: u32,
    /// How visible the texture is where a held stroke ends up, 0 to 1. At 1
    /// the stroke converges on the texture alone. At 0.6 it converges on the
    /// texture at 60% over what is under it, from either side: ground already
    /// more opaque than that is brought down to it.
    pub opacity: f32,
    /// Take the texture away instead of putting it down: its layer moves
    /// toward transparent and nothing else moves, so what is under it shows
    /// again. Erasing never adds a layer. The base cannot be erased, since it
    /// has no blend map; see [`Painted::base`].
    pub erase: bool,
    /// Paint only chunks that already carry the texture. A chunk without it
    /// is left alone and reported in [`Painted::absent`], so a stroke changes
    /// blends and never the set of textures a chunk names.
    pub existing_only: bool,
    /// When a chunk is full, give the texture the layer that shows least,
    /// provided it shows less than [`REUSE_UNDER`] of the chunk. See
    /// [`Painted::reused`].
    pub reuse_hidden: bool,
    /// How much of the ground under the brush is painted, 0 to 1. At 1 the
    /// stroke is solid. Below 1 it paints in patches, the fraction of the
    /// footprint that [`speckle`] puts under this value, which is how one
    /// texture is broken up into another.
    pub density: f32,
    /// How many yards a patch is across, for a [`Self::density`] under 1.
    pub grain: f32,
}

/// The most of a chunk a layer may show and still be given to another
/// texture by [`PaintBrush::reuse_hidden`]: two hundredths. Below it the layer
/// is a few texels at a border, and replacing it changes nothing a person
/// would find.
pub const REUSE_UNDER: f32 = 0.02;

/// A value from 0 to 1 that depends on where a texel is and on nothing else,
/// varying over `grain` yards: [`wobble`] moved into that range.
///
/// A stroke at a density under 1 paints the texels where this is under the
/// density. It is a function of the place for the reason `wobble` is: a held
/// stroke has to deepen the same patches and not paint new ones every frame.
pub fn speckle(x: f32, y: f32, grain: f32) -> f32 {
    wobble(x, y, grain.max(0.25)) * 0.5 + 0.5
}

impl Default for PaintBrush {
    fn default() -> PaintBrush {
        PaintBrush {
            radius: 12.0,
            // A third of the radius at full strength, with the falloff over
            // the rest: WoWEdit's inner ring, about where it draws one. Since
            // a stroke paints no further than its footprint, a brush with no
            // core is solid at one point only.
            core: 0.35,
            shape: Shape::Circle,
            strength: 3.0,
            // Falling from the moment it leaves the core and nearly gone well
            // before the rim. Smooth is flat where it leaves the core, so a
            // band outside the inner ring painted at nearly full strength and
            // the brush read as heavy beside WoWEdit's.
            falloff: Falloff::Sharp,
            texture: String::new(),
            effect_id: 0,
            opacity: 1.0,
            erase: false,
            existing_only: false,
            reuse_hidden: false,
            density: 1.0,
            // About four texels of a blend map, which is the smallest patch
            // that still reads as a patch after the map's own filtering.
            grain: 2.0,
        }
    }
}

impl PaintBrush {
    /// Apply one step of the stroke at a world position, and return the edits it
    /// made.
    ///
    /// `seconds` is how long since the last step, exactly as
    /// [`Brush::stroke`](super::brush::Brush::stroke)'s is and for the same
    /// reason: a brush covers ground at the same rate whatever the frame rate
    /// is.
    pub fn stroke(
        &self,
        tile: &mut AdtFile,
        working: &mut Working,
        at: [f32; 2],
        seconds: f32,
    ) -> Painted {
        // The brush's own footprint: its shape, its falloff curve and its
        // core, around the point.
        let weight = |x: f32, y: f32| {
            let distance = self.shape.distance(x - at[0], y - at[1]);
            self.falloff.over(distance / self.radius, self.core)
        };
        let chunks = self.chunks_under(tile, at);
        self.paint_where(tile, working, &chunks, seconds, weight)
    }

    /// **Paint with a footprint the caller describes**: `weight` gives each
    /// point in world x and y a share of the stroke, 0 to 1, and `chunks` are
    /// the chunks it can be above 0 in. [`Self::stroke`] is this with the
    /// brush's own circle; a road is this with the distance from its centre
    /// line. The radius, shape, falloff and core are not read here.
    ///
    /// With `seconds` at 1 and a strength of 1, one call moves every texel
    /// all the way to its weight's share of the opacity, which is a footprint
    /// written once rather than rubbed in.
    pub fn paint_where(
        &self,
        tile: &mut AdtFile,
        working: &mut Working,
        chunks: &[usize],
        seconds: f32,
        weight: impl Fn(f32, f32) -> f32,
    ) -> Painted {
        let mut painted = Painted::default();
        if self.texture.is_empty() {
            return painted;
        }
        for &index in chunks {
            let before = ChunkPaint::capture(tile, index);
            match self.paint_chunk(tile, working, index, seconds, &weight) {
                Step::Painted => {}
                Step::Nothing => continue,
                Step::Based => painted.based.push(index),
                Step::Reused => painted.reused.push(index),
                Step::Full => {
                    painted.full.push(index);
                    continue;
                }
                Step::Base => {
                    painted.base.push(index);
                    continue;
                }
                Step::Absent => {
                    painted.absent.push(index);
                    continue;
                }
            }
            let after = ChunkPaint::capture(tile, index);
            if after == before {
                continue;
            }
            painted.edits.push(Edit::Paint {
                chunk: index,
                before: Box::new(before),
                after: Box::new(after),
            });
        }
        painted
    }

    /// One chunk's share of a step.
    fn paint_chunk(
        &self,
        tile: &mut AdtFile,
        working: &mut Working,
        index: usize,
        seconds: f32,
        weight: &dyn Fn(f32, f32) -> f32,
    ) -> Step {
        let Some(chunk) = tile.chunk(index) else {
            return Step::Nothing;
        };
        let origin = chunk.head().position();
        // Each texel's share of the step, worked out first: a chunk the
        // footprint does not reach at all is left before anything below
        // gives it a layer, makes the texture its base or reports it full.
        let weights: Vec<f32> = (0..ALPHA_LEN)
            .map(|texel| {
                let [x, y] = alpha::texel_position(origin, texel % ALPHA_SIDE, texel / ALPHA_SIDE);
                weight(x, y).clamp(0.0, 1.0)
            })
            .collect();
        if weights.iter().all(|&w| w == 0.0) {
            return Step::Nothing;
        }
        let mut paint = alpha::paint(chunk);

        // **A chunk with no `MCLY` at all takes this texture as its base.**
        //
        // Not a corner case: `development` has 543 such chunks and one whole
        // tile of them, and this returned `Nothing` on every one — so a blank
        // tile could never be given its first texture, which is exactly how it
        // was reported.
        //
        // The base is the one layer that has no blend map — it is the opaque
        // thing every other layer is painted *over* — so there is no partial
        // version of it and the falloff has nothing to act on. The whole chunk
        // becomes this texture. That is the only thing an empty chunk can be
        // given, and [`Painted::based`] is how the caller says so.
        if paint.is_empty() {
            // Neither an eraser nor a stroke confined to existing layers has
            // anything to do with a chunk that has no layers.
            if self.erase || self.existing_only {
                return match self.erase {
                    true => Step::Nothing,
                    false => Step::Absent,
                };
            }
            paint.layers.push(TextureLayer {
                texture_id: tile.name_texture(&self.texture),
                flags: 0,
                alpha_offset: 0,
                effect_id: self.effect_id,
            });
            paint.maps.push(vec![255u8; ALPHA_LEN]);
            if let Some(chunk) = tile.chunk_mut(index) {
                alpha::set_paint(chunk, &paint);
            }
            // The working copy is dropped with it: the chunk it was seeded from
            // had no layers, and the next step of this stroke re-seeds from a
            // chunk that has one.
            working.chunks.remove(&index);
            return Step::Based;
        }

        // **The texture is named only once it is known the chunk can take it.**
        // Appending to `MTEX` for a chunk that is already full would leave the
        // tile carrying a tileset nothing draws.
        let named = tile
            .texture_names()
            .iter()
            .position(|name| name.eq_ignore_ascii_case(&self.texture))
            .map(|at| at as u32);
        let mut reused = false;
        let target = match named.and_then(|id| paint.layer_of(id)) {
            Some(layer) => layer,
            // An eraser takes a texture away and has none to take here.
            None if self.erase => return Step::Nothing,
            None if self.existing_only => return Step::Absent,
            None if !paint.has_room() => {
                // A full chunk. The layer that shows least is given to this
                // texture when it shows almost nothing; otherwise the stroke
                // is refused here, as it always was.
                let hidden = match self.reuse_hidden {
                    true => least_shown(&paint).filter(|&(_, shown)| shown < REUSE_UNDER),
                    false => None,
                };
                let Some((layer, _)) = hidden else {
                    return Step::Full;
                };
                paint.layers[layer].texture_id = tile.name_texture(&self.texture);
                paint.layers[layer].effect_id = self.effect_id;
                paint.maps[layer] = vec![0u8; ALPHA_LEN];
                // The stroke's own copy of that layer was the old texture's.
                if let Some(wet) = working.chunks.get_mut(&index) {
                    for maps in [&mut wet.start, &mut wet.now] {
                        if let Some(map) = maps.get_mut(layer) {
                            map.fill(0.0);
                        }
                    }
                }
                reused = true;
                layer
            }
            None => {
                paint.layers.push(TextureLayer {
                    texture_id: tile.name_texture(&self.texture),
                    flags: 0,
                    alpha_offset: 0,
                    effect_id: self.effect_id,
                });
                paint.maps.push(vec![0u8; ALPHA_LEN]);
                paint.len() - 1
            }
        };
        // The base has no blend map. Painting it clears what is over it, which
        // is the same picture; erasing it has no meaning.
        if self.erase && target == 0 {
            return Step::Base;
        }

        // **The stroke's own copy, at full precision** — see [`Working`], which
        // is where the whole argument is. Seeded from the file the first time
        // this stroke touches this chunk, and grown when the step above has just
        // given the chunk a layer it did not have.
        let wet = working
            .chunks
            .entry(index)
            .or_insert_with(|| Wet::seeded(&paint.maps));
        wet.grow_to(paint.maps.len());

        // Clamped so a long frame cannot overshoot and oscillate, which is
        // the same guard `Brush::moved` puts on its two converging modes.
        let rate = (self.strength * seconds).clamp(0.0, 1.0);
        let opacity = self.opacity.clamp(0.0, 1.0);
        for texel in 0..ALPHA_LEN {
            let (tx, ty) = (texel % ALPHA_SIDE, texel / ALPHA_SIDE);
            let [x, y] = alpha::texel_position(origin, tx, ty);
            // **In the footprint's own shape.** A texel is 0.52 yards, so a
            // square brush here has a genuinely straight edge rather than a
            // stepped one.
            let weight = weights[texel];
            if weight == 0.0 {
                continue;
            }
            // A density under 1 paints in patches: the texels where the
            // place's own value is under the density, with a soft edge a
            // twelfth of the range wide so a patch does not end on a texel.
            let patch = match self.density < 1.0 {
                true => ((self.density - speckle(x, y, self.grain)) * 12.0 + 0.5).clamp(0.0, 1.0),
                false => 1.0,
            };
            // The texel's share of the stroke rises toward the footprint here
            // and never past it; see the section on [`PaintBrush`].
            let reach = weight * patch;
            let cover = &mut wet.cover[texel];
            if reach <= *cover {
                continue;
            }
            *cover += (reach - *cover) * rate;
            let cover = *cover;
            if self.erase {
                // The texture's own layer toward transparent, and nothing
                // else: what is under it shows again.
                wet.now[target][texel] = wet.start[target][texel] * (1.0 - cover);
                continue;
            }
            // The target layer toward the opacity asked for, from either
            // side…
            let from = wet.start[target][texel];
            wet.now[target][texel] = from + (255.0 * opacity - from) * cover;
            // …and everything painted over it toward transparent, which is the
            // half a brush that "does not work" is missing. Scaled by the
            // opacity, so a faint stroke also uncovers the texture faintly.
            for above in target + 1..wet.now.len() {
                wet.now[above][texel] = wet.start[above][texel] * (1.0 - cover * opacity);
            }
        }

        // …and the file, written from it. Quantised **once, here**, rather than
        // once per frame: what the chunk can hold is sixteen levels or 256, and
        // `encode_alpha_maps` is what applies that on the way out.
        let mut moved = false;
        for (layer, map) in paint.maps.iter_mut().enumerate() {
            let Some(source) = wet.now.get(layer) else { continue };
            for (texel, slot) in map.iter_mut().enumerate() {
                let value = source[texel].clamp(0.0, 255.0).round() as u8;
                if *slot != value {
                    *slot = value;
                    moved = true;
                }
            }
        }
        if !moved && !reused {
            return Step::Nothing;
        }
        // **The base layer is the one that cannot be blended.** If the brush was
        // asked for a texture that is layer 0 there is nothing to raise — it is
        // already under everything — so what the loop above did was clear the
        // layers over it, which is the same picture and the right one.
        if let Some(chunk) = tile.chunk_mut(index) {
            alpha::set_paint(chunk, &paint);
        }
        match reused {
            true => Step::Reused,
            false => Step::Painted,
        }
    }

    /// The chunks whose square the brush's circle reaches. The same rule
    /// [`Brush::chunks_under`](super::brush::Brush::chunks_under) uses, kept
    /// beside its own brush rather than shared, because the two are free to
    /// grow apart — a paint brush has no reason to reach a neighbour the way a
    /// height brush does for shading.
    fn chunks_under(&self, tile: &AdtFile, at: [f32; 2]) -> Vec<usize> {
        (0..tile.chunks.len())
            .filter(|&i| {
                let Some(chunk) = tile.chunk(i) else {
                    return false;
                };
                let origin = chunk.head().position();
                let near = |o: f32, p: f32| p >= o - CHUNK_SIZE - self.radius && p <= o + self.radius;
                near(origin[0], at[0]) && near(origin[1], at[1])
            })
            .collect()
    }
}

/// The layer over the base that shows least of the chunk, with how much it
/// shows. `None` for a chunk with only a base.
fn least_shown(paint: &alpha::Paint) -> Option<(usize, f32)> {
    paint
        .coverage()
        .into_iter()
        .enumerate()
        .skip(1)
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// One chunk's blend maps as the stroke works on them.
fn as_working(maps: &[Vec<u8>]) -> Vec<Vec<f32>> {
    maps.iter()
        .map(|map| map.iter().map(|&alpha| f32::from(alpha)).collect())
        .collect()
}

/// What one chunk's share of a paint step came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Painted,
    /// The chunk had no layers and this texture is now its base — see
    /// [`Painted::based`].
    Based,
    /// The brush reached this chunk and changed nothing — outside the falloff,
    /// or already where it is going.
    Nothing,
    /// **It carries four textures and none of them is this one.** Reported
    /// rather than swallowed; see [`Painted`].
    Full,
    /// It was full, and a layer that showed almost nothing now carries this
    /// texture. See [`Painted::reused`].
    Reused,
    /// An eraser reached a chunk whose base is the texture. See
    /// [`Painted::base`].
    Base,
    /// The stroke is confined to existing layers and the chunk does not carry
    /// the texture. See [`Painted::absent`].
    Absent,
}
