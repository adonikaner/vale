//! **A flat `<Model>` widget's contents, as triangles an interface can paint.**
//!
//! `Interface\FrameXML\` declares fifteen `<Model>` elements and one of them is
//! on every action button in the game: `CooldownFrameTemplate` (`Cooldown.xml`)
//! holds `Interface\Cooldown\UI-Cooldown-Indicator.mdx`, and it is what draws
//! the sweeping clock over an icon. The 1.12 client renders it the way it
//! renders any model — a scene, a camera, a projection — and this client does
//! not have a per-widget 3D pass; what it does have is a painter that can lay a
//! textured mesh into a rectangle.
//!
//! **Which is enough, because the model is flat.** Measured on the cooldown
//! indicator: 20 vertices, 5 batches, `bounds z = 0` throughout, four of the
//! batches being axis-aligned quads in model XY ([`crate::world::m2::M2::ground_quad`]
//! names the same shape) and the fifth a slightly larger quad 0.012 above them.
//! A model like that has no depth to lose, so evaluating it on the CPU and
//! handing the painter triangles produces the same picture a camera would — and
//! it costs one mesh per cooldown rather than one render pass.
//!
//! ## What this decides and what it does not
//!
//! By this project's own rule ("could this be decided with no renderer
//! running?"), everything here is an `assets` question: which triangles, at
//! which texture coordinates, in which colour, at time *t*. What the *painter*
//! decides is where the rectangle is on the screen and how a blend mode is
//! approximated.
//!
//! ## The two rules that are readings rather than measurements
//!
//! * **The fit.** The model is mapped so that its own drawn XY bounding box
//!   fills the frame's rectangle. The real client projects it through a camera
//!   whose framing this project has not read, and `<Model scale="0.75">` is
//!   stated against *that* framing rather than against a rectangle — so the
//!   scale attribute is deliberately **not** applied here, because the fit has
//!   already normalised the size and multiplying by it again would inset the
//!   swirl inside its own icon. Stated rather than hidden: it is the one number
//!   in the markup this module ignores.
//! * **The axes.** Model `+X` is drawn to the right and model `+Y` upwards.
//!   That is not arbitrary — it is what makes the cooldown sweep **clockwise
//!   from twelve o'clock**: the four quadrant batches clear in the order
//!   (right, upper), (right, lower), (left, lower), (left, upper), one per
//!   quarter of the animation, which is a clock hand under this mapping and a
//!   widdershins one under any flip.
//!
//! ## A model with no vertices draws its emitters
//!
//! `Interface\Buttons\UI-AutoCastButton.mdx`, the autocast border on a pet
//! action button, is four particle emitters and nothing else. [`is_flat`]
//! accepts it and [`flatten`] hands back its sprites, evaluated on the CPU the
//! same way — see [`sprites`], which states which of its rules are readings.

use crate::world::m2::{M2, M2Sequence};

/// One vertex of a flattened model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiVertex {
    /// Where it lands **in the frame's own rectangle**, `0..1` from the left
    /// and `0..1` from the *bottom* — the game's own y-up convention, which the
    /// painter flips once at the end like every other rectangle it draws.
    pub x: f32,
    pub y: f32,
    /// Texture coordinates, already through the batch's texture matrix at this
    /// moment. May fall outside `0..1`: the sampler wraps, and the cooldown
    /// depends on it doing so.
    pub u: f32,
    pub v: f32,
    /// A per-vertex colour multiplied into the batch's, white for geometry.
    /// A sprite batch carries its over-life colour here, so an emitter's
    /// whole cloud is one batch rather than one per sprite.
    pub tint: [f32; 4],
}

/// One batch of a flattened model: a texture, a blend, a colour and a mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct UiBatch {
    /// The model's own texture path, as the file spells it.
    pub texture: String,
    /// `M2Batch::blend` — 3 and 4 are the additive pair, which a painter with
    /// one blend mode has to approximate.
    pub blend: u16,
    /// The animated tint and opacity, already sampled: `rgba`, multiplied into
    /// whatever the painter is drawing with.
    pub colour: [f32; 4],
    pub vertices: Vec<UiVertex>,
    /// Triangle list into [`UiBatch::vertices`].
    pub indices: Vec<u16>,
}

impl UiBatch {
    /// **Nothing to draw.** A batch whose colour has faded to nothing — which
    /// is every one of the cooldown's five before its animation starts and
    /// after it ends — is dropped rather than painted transparent.
    pub fn is_invisible(&self) -> bool {
        self.colour[3] <= 0.0 || self.vertices.is_empty() || self.indices.is_empty()
    }
}

/// Whether this model can be drawn as a flat mesh at all.
///
/// **Every drawn vertex within [`FLAT_EPS`] of one plane in Z.** The cooldown
/// indicator passes (its two planes are 0 and 0.012 apart, both inside the
/// tolerance below); a character portrait does not, and answering `false` for
/// one is what keeps this from drawing a paper doll as a squashed silhouette.
pub fn is_flat(model: &M2) -> bool {
    if model.positions.is_empty() {
        // A model with no geometry at all is drawable only if it has emitters
        // to draw — see [`sprites`].
        return !model.particles.is_empty();
    }
    let (lo, hi) = model.positions.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
        (lo.min(p[2]), hi.max(p[2]))
    });
    lo <= hi && hi - lo <= FLAT_EPS
}

/// How much depth a "flat" model may have, in model units.
///
/// The cooldown's flash quad sits 0.012 above its clock quads on a model whose
/// whole extent is 0.038, so a tolerance tighter than a third of the extent
/// would reject it. Absolute rather than relative because a UI model is
/// authored at whatever scale it likes and this is only asking "is there any
/// depth here to lose".
const FLAT_EPS: f32 = 0.05;

/// **Flatten a model into triangles for a frame's rectangle**, at `elapsed_ms`
/// of `sequence`.
///
/// `now_ms` drives any global sequence, exactly as
/// [`crate::world::m2::M2TextureAnims::matrix_in`] takes it. Batches come back in the
/// model's own order, which is the order they must be painted in — the cooldown
/// depends on it only to put its finish flash over the clock.
pub fn flatten(
    model: &M2,
    sequence: Option<&M2Sequence>,
    elapsed_ms: u32,
    now_ms: u32,
) -> Vec<UiBatch> {
    // The fit: the drawn vertices' box, or, for a model with no vertices, the
    // box its emitters sweep — see [`sprites`].
    let Some(bounds) = drawn_bounds(model).or_else(|| orbit_bounds(model, sequence)) else {
        return Vec::new();
    };
    let ([min_x, min_y], [max_x, max_y]) = bounds;
    let span_x = (max_x - min_x).max(f32::EPSILON);
    let span_y = (max_y - min_y).max(f32::EPSILON);

    let mut out = Vec::new();
    for batch in &model.batches {
        let Some(texture) = batch
            .texture
            .and_then(|i| model.textures.get(i as usize))
            .map(|t| t.file_name.clone())
            .filter(|f| !f.is_empty())
        else {
            continue;
        };
        let colour = match batch.tint {
            Some(tint) => model.tints.sample_in(tint, sequence, elapsed_ms, now_ms),
            None => [1.0; 4],
        };
        let uv = match batch.uv {
            Some(index) => model.uv_anims.matrix_in(index, sequence, elapsed_ms, now_ms),
            None => crate::world::m2::UV_IDENTITY,
        };

        let range = batch.index_start as usize
            ..(batch.index_start as usize + batch.index_count as usize).min(model.indices.len());
        if range.is_empty() {
            continue;
        }
        // One vertex per *use*, rather than the model's own indexing: a batch
        // is a handful of triangles and the copy is cheaper than carrying a
        // remap, which would also have to be per batch because the texture
        // matrix differs between them.
        let mut vertices = Vec::with_capacity(range.len());
        for &index in &model.indices[range] {
            let index = index as usize;
            let (Some(position), Some(texcoord)) =
                (model.positions.get(index), model.uvs.get(index))
            else {
                continue;
            };
            let (u, v) = (texcoord[0], texcoord[1]);
            vertices.push(UiVertex {
                x: (position[0] - min_x) / span_x,
                y: (position[1] - min_y) / span_y,
                u: uv[0] * u + uv[1] * v + uv[2],
                v: uv[3] * u + uv[4] * v + uv[5],
                tint: [1.0; 4],
            });
        }
        let indices = (0..vertices.len() as u16).collect::<Vec<_>>();
        out.push(UiBatch {
            texture,
            blend: batch.blend,
            colour,
            vertices,
            indices,
        });
    }
    out.extend(sprites(model, sequence, bounds, now_ms));
    out
}

/// **A particle-only model's contents** — the autocast border.
///
/// `Interface\Buttons\UI-AutoCastButton.mdx` is the `<Model>` under every pet
/// action button (`PetActionBarFrame.xml`, `$parentAutoCast`), shown while the
/// spell auto-casts. It has no vertices: four emitters, one per root bone, and
/// the bones walk a 0.02-unit square a quarter turn apart over the one
/// two-second sequence. Each emitter births 300 `GlowStar.blp` sprites a second
/// that live one second, half-size 0.005 shrinking through 0.0015 to 0.001,
/// coloured yellow (249, 223, 49) through cream to transparent white, blended
/// additively (blend 4). Measured off the file; the numbers are in the test.
///
/// ## The rules, and which are readings
///
/// * **The clock is `now_ms`.** The frame's own `elapsed` only moves under
///   `AdvanceTime`, and nothing in the shipped Lua calls it on this frame; the
///   reference animates a model frame without Lua. Sequence 0 is looped,
///   which is its own flag.
/// * **A sprite is born where its emitter was at its birth time and stays
///   there.** The emitters carry neither the model-space flag (0x10) nor a
///   follow response, so a birth is baked at the emitter's position, which is
///   the bone's translation at `birth mod sequence length` plus the emitter's
///   own offset. Births are at `k / rate` seconds on the absolute clock, so a
///   sprite keeps its place from frame to frame and the trail is the path the
///   emitter walked over the last lifespan.
/// * **Motion out of the plane is dropped.** The emitters spray along `+Z` at
///   2 units a second; in a flat drawing that is toward the viewer and moves
///   nothing on the page. A reading, stated: the reference's model-frame
///   camera was not traced, so how much a sprite grows as it approaches is
///   not known. Gravity and drag are both zero here.
/// * **The fit is the emitters' orbit box** — the XY box the emitter
///   positions sweep over the sequence — where a model with vertices uses its
///   drawn box. The stars then run along the frame's edges, which is what a
///   border is. Sprite size is scaled by the same fit, and a sprite may
///   extend past the rectangle.
/// * **The bone is sampled by its translation track alone.** Every bone here
///   is a root with a translation track and nothing else; a model whose
///   emitter bone has a parent or a rotation is drawn with the translation
///   only, which is a stated shortfall of this path rather than of the world's
///   particle pass.
/// * The over-life ramp is the same one `render::particles::over_life` runs:
///   two linear segments split at `mid_point`. Twinkle, spin, tails, atlases
///   and geometry models are not drawn; none of the interface's models use
///   them (this one's twinkle percent is 1, its spin 0.1 rad/s, its atlas
///   1x1).
pub fn sprites(
    model: &M2,
    sequence: Option<&M2Sequence>,
    bounds: ([f32; 2], [f32; 2]),
    now_ms: u32,
) -> Vec<UiBatch> {
    let ([min_x, min_y], [max_x, max_y]) = bounds;
    let span_x = (max_x - min_x).max(f32::EPSILON);
    let span_y = (max_y - min_y).max(f32::EPSILON);
    let mut out = Vec::new();
    for emitter in &model.particles {
        if emitter.geometry_model.is_some() {
            continue;
        }
        let Some(texture) = model
            .textures
            .get(emitter.texture as usize)
            .map(|t| t.file_name.clone())
            .filter(|f| !f.is_empty())
        else {
            continue;
        };
        let rate = first_key(&emitter.emission_rate);
        let life = first_key(&emitter.lifespan);
        if rate <= 0.0 || life <= 0.0 {
            continue;
        }
        // Bounded: a UI emitter at the shipped numbers is 300 live sprites.
        let live = ((rate * life).ceil() as usize).min(MAX_SPRITES_PER_EMITTER);
        let now = f64::from(now_ms) / 1000.0;
        // The newest birth at or before now, then back through the lifespan.
        let newest = (now * f64::from(rate)).floor() as i64;
        let mut vertices = Vec::with_capacity(live * 4);
        let mut indices = Vec::with_capacity(live * 6);
        // Oldest first, so the newest (brightest, largest) draws on top.
        for k in (0..live as i64).rev() {
            let birth = (newest - k) as f64 / f64::from(rate);
            if birth < 0.0 {
                continue;
            }
            let age = (now - birth) as f32;
            if age < 0.0 || age >= life {
                continue;
            }
            let (colour, half) = over_life(emitter, age / life);
            if colour[3] <= 0.0 || half <= 0.0 {
                continue;
            }
            let at = emitter_at(model, emitter, sequence, (birth * 1000.0) as u32, now_ms);
            let x = (at[0] - min_x) / span_x;
            let y = (at[1] - min_y) / span_y;
            let (hx, hy) = (half / span_x, half / span_y);
            let base = vertices.len() as u16;
            for (dx, dy, u, v) in [
                (-hx, -hy, 0.0, 1.0),
                (hx, -hy, 1.0, 1.0),
                (hx, hy, 1.0, 0.0),
                (-hx, hy, 0.0, 0.0),
            ] {
                vertices.push(UiVertex {
                    x: x + dx,
                    y: y + dy,
                    u,
                    v,
                    tint: colour,
                });
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        if vertices.is_empty() {
            continue;
        }
        // One batch per emitter: the sprite's own colour rides on its
        // vertices, and the batch colour is white.
        out.push(UiBatch {
            texture,
            blend: emitter.blend,
            colour: [1.0; 4],
            vertices,
            indices,
        });
    }
    out
}

/// The most live sprites one emitter may draw — the shipped rate times
/// lifespan is 300, and a file stating more is capped rather than trusted.
const MAX_SPRITES_PER_EMITTER: usize = 512;

/// The XY box the emitters sweep over the sequence — the fit for a model with
/// no vertices. Sampled at [`ORBIT_SAMPLES`] moments; `None` for no emitters.
fn orbit_bounds(model: &M2, sequence: Option<&M2Sequence>) -> Option<([f32; 2], [f32; 2])> {
    if model.particles.is_empty() {
        return None;
    }
    let length = sequence.map_or(0, |s| s.end.saturating_sub(s.start)).max(1);
    let mut min = [f32::MAX; 2];
    let mut max = [f32::MIN; 2];
    for emitter in &model.particles {
        for i in 0..ORBIT_SAMPLES {
            let t = (length as u64 * i as u64 / ORBIT_SAMPLES as u64) as u32;
            let at = emitter_at(model, emitter, sequence, t, 0);
            for axis in 0..2 {
                min[axis] = min[axis].min(at[axis]);
                max[axis] = max[axis].max(at[axis]);
            }
        }
    }
    Some((min, max))
}

const ORBIT_SAMPLES: usize = 64;

/// Where an emitter is at `t_ms` of the sequence: the bone's translation at
/// that moment, wrapped onto the sequence, plus the emitter's own offset.
fn emitter_at(
    model: &M2,
    emitter: &crate::world::m2::M2Particle,
    sequence: Option<&M2Sequence>,
    t_ms: u32,
    now_ms: u32,
) -> [f32; 3] {
    let mut at = emitter.position;
    let Some(skeleton) = model.skeleton.as_ref() else {
        return at;
    };
    let Some(bone) = skeleton.bones.get(emitter.bone as usize) else {
        return at;
    };
    let Some(track) = bone.translation.as_ref() else {
        return at;
    };
    let (start, end) = sequence.map_or((0, 0), |s| (s.start, s.end));
    let length = end.saturating_sub(start);
    let t = if length == 0 { start } else { start + t_ms % length };
    let offset = track.sample(t, start, end, &skeleton.global_sequences, now_ms);
    for axis in 0..3 {
        at[axis] += offset[axis];
    }
    at
}

/// The first key of a track, or 0 — every UI emitter's rate and lifespan are
/// one key.
fn first_key(track: &Option<crate::world::m2::M2Track>) -> f32 {
    track
        .as_ref()
        .and_then(|t| t.values.first().copied())
        .unwrap_or(0.0)
}

/// Colour and half-size at `u` of a sprite's life — two linear segments split
/// at `mid_point`, the same ramp the world's particle pass samples.
fn over_life(emitter: &crate::world::m2::M2Particle, u: f32) -> ([f32; 4], f32) {
    let u = u.clamp(0.0, 1.0);
    let mid = emitter.mid_point.clamp(1e-3, 1.0);
    let (a, b, t) = if u <= mid {
        (0, 1, u / mid)
    } else {
        (1, 2, (u - mid) / (1.0 - mid).max(1e-3))
    };
    let mut colour = [0.0f32; 4];
    for (c, out) in colour.iter_mut().enumerate() {
        let (ca, cb) = (f32::from(emitter.colors[a][c]), f32::from(emitter.colors[b][c]));
        *out = (ca + (cb - ca) * t) / 255.0;
    }
    let half = emitter.scales[a] + (emitter.scales[b] - emitter.scales[a]) * t;
    (colour, half)
}

/// The XY box the model's **drawn** vertices occupy — the fit's denominator.
///
/// Every vertex any batch indexes, rather than
/// [`crate::world::m2::M2::bounds`]: the header's box on the cooldown indicator is
/// `0..0.1` where the geometry is `0..0.038`, so fitting the header's would
/// draw the clock at a third of the size of the button it is over, in its
/// corner.
fn drawn_bounds(model: &M2) -> Option<([f32; 2], [f32; 2])> {
    let mut min = [f32::MAX; 2];
    let mut max = [f32::MIN; 2];
    let mut any = false;
    for batch in &model.batches {
        let range = batch.index_start as usize
            ..(batch.index_start as usize + batch.index_count as usize).min(model.indices.len());
        for &index in model.indices.get(range).unwrap_or(&[]) {
            let Some(position) = model.positions.get(index as usize) else {
                continue;
            };
            any = true;
            for axis in 0..2 {
                min[axis] = min[axis].min(position[axis]);
                max[axis] = max[axis].max(position[axis]);
            }
        }
    }
    any.then_some((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::m2::{BatchTint, M2Batch, M2Texture};

    /// A one-quad model in the cooldown's own shape: a unit square in model XY
    /// at `z = 0`, mapping the whole texture.
    fn quad() -> M2 {
        let mut model = M2::default();
        model.positions = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        model.uvs = vec![[0.0, 1.0], [1.0, 1.0], [0.0, 0.0], [1.0, 0.0]];
        model.indices = vec![0, 1, 2, 1, 3, 2];
        model.textures = vec![M2Texture {
            kind: 0,
            file_name: "Interface\\Cooldown\\cooldown.blp".to_string(),
        }];
        model.batches = vec![M2Batch {
            geoset: 0,
            index_start: 0,
            index_count: 6,
            texture: Some(0),
            blend: 2,
            unlit: true,
            two_sided: false,
            no_depth_write: true,
            tint: None,
            uv: None,
        }];
        model
    }

    /// **The fit is the model's own drawn box**, and the y stays up.
    #[test]
    fn a_flat_quad_fills_the_frames_rectangle() {
        let model = quad();
        assert!(is_flat(&model));
        let batches = flatten(&model, None, 0, 0);
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.texture, "Interface\\Cooldown\\cooldown.blp");
        assert_eq!(batch.vertices.len(), 6, "one vertex per index, per batch");
        // The corners of the unit square land on the corners of the rectangle.
        let corners: Vec<(f32, f32)> = batch.vertices.iter().map(|v| (v.x, v.y)).collect();
        assert!(corners.contains(&(0.0, 0.0)));
        assert!(corners.contains(&(1.0, 1.0)));
        // …and the texture coordinates are the file's own, untransformed.
        assert!(batch.vertices.iter().all(|v| (0.0..=1.0).contains(&v.u)));
    }

    /// **A model with depth is refused**, which is what stops a paper doll from
    /// being drawn as a flat silhouette by this path.
    #[test]
    fn a_model_with_depth_is_not_flat() {
        let mut model = quad();
        model.positions[3][2] = 1.0;
        assert!(!is_flat(&model));
    }

    /// A batch whose opacity track has faded to zero is invisible, which is
    /// every one of the cooldown's five outside its own animation.
    #[test]
    fn a_faded_batch_is_dropped_by_the_caller() {
        let mut batch = flatten(&quad(), None, 0, 0).remove(0);
        assert!(!batch.is_invisible());
        batch.colour[3] = 0.0;
        assert!(batch.is_invisible());
    }

    /// **The tint is sampled, not assumed**: a batch that names a colour track
    /// gets whatever it says at this moment.
    #[test]
    fn a_tinted_batch_carries_its_sampled_colour() {
        let mut model = quad();
        model.batches[0].tint = Some(BatchTint { color: None, transparency: None });
        let batch = &flatten(&model, None, 0, 0)[0];
        assert_eq!(batch.colour, [1.0; 4], "no tracks is fully opaque white");
    }

    /// **The autocast border, as the file states it**: no vertices, four
    /// emitters on four root bones, each bone's translation track walking a
    /// 0.02-unit square over the 2,000 ms sequence a quarter turn apart. The
    /// numbers are `UI-AutoCastButton.m2`'s own (bone 0's keys at 0, 500,
    /// 1000, 1500, 2000 ms; emitter 0's ramp), reduced to two emitters.
    fn autocast() -> M2 {
        use crate::world::m2::{M2Bone, M2Particle, M2Sequence, M2Skeleton, M2Track};
        let square = |phase: usize| -> M2Track {
            let corners = [[0.0, 0.0, 0.0], [0.0, 0.02, -0.01], [0.02, 0.02, 0.0], [0.02, 0.0, -0.01]];
            let mut values = Vec::new();
            for i in 0..5 {
                values.extend_from_slice(&corners[(i + phase) % 4]);
            }
            // Relative to the first corner, as a translation track is.
            let origin = corners[phase % 4];
            for (i, v) in values.iter_mut().enumerate() {
                *v -= origin[i % 3];
            }
            M2Track {
                interpolation: 1,
                global_sequence: -1,
                times: vec![0, 500, 1000, 1500, 2000],
                values,
                dim: 3,
                ..M2Track::default()
            }
        };
        let bone = |phase: usize| M2Bone {
            translation: Some(square(phase)),
            parent: -1,
            ..M2Bone::default()
        };
        let emitter = |bone: u16, position: [f32; 3]| M2Particle {
            flags: 0x1,
            position,
            bone,
            texture: 0,
            blend: 4,
            emitter_type: 3,
            emission_speed: Some(M2Track { times: vec![0], values: vec![2.0], dim: 1, ..M2Track::default() }),
            lifespan: Some(M2Track { times: vec![0], values: vec![1.0], dim: 1, ..M2Track::default() }),
            emission_rate: Some(M2Track { times: vec![0], values: vec![300.0], dim: 1, ..M2Track::default() }),
            mid_point: 0.5,
            colors: [[249, 223, 49, 255], [254, 241, 190, 255], [255, 255, 255, 0]],
            scales: [0.005, 0.0015, 0.001],
            rows: 1,
            columns: 1,
            ..M2Particle::default()
        };
        let mut model = M2::default();
        model.textures = vec![M2Texture {
            kind: 0,
            file_name: "Interface\\Buttons\\GlowStar.blp".to_string(),
        }];
        let mut skeleton = M2Skeleton::default();
        skeleton.bones = vec![bone(0), bone(1)];
        skeleton.sequences = vec![M2Sequence { start: 0, end: 2000, ..M2Sequence::default() }];
        model.skeleton = Some(skeleton);
        model.particles = vec![emitter(0, [0.0, 0.0, 0.0]), emitter(1, [0.0, 0.02, 0.0])];
        model
    }

    /// **A model with emitters and no vertices is flat**, and it draws: at any
    /// moment past the first lifespan each emitter has 300 live sprites, in
    /// one batch each, additively blended, textured with the star.
    #[test]
    fn a_particle_only_model_is_flat_and_draws_its_emitters() {
        let model = autocast();
        assert!(is_flat(&model));
        assert!(!is_flat(&M2::default()), "no vertices and no emitters is nothing");
        let sequence = model.skeleton.as_ref().and_then(|s| s.sequences.first());
        let batches = flatten(&model, sequence, 0, 3_000);
        assert_eq!(batches.len(), 2, "one batch per emitter");
        for batch in &batches {
            assert_eq!(batch.texture, "Interface\\Buttons\\GlowStar.blp");
            assert_eq!(batch.blend, 4);
            assert_eq!(batch.vertices.len(), 300 * 4);
            assert_eq!(batch.indices.len(), 300 * 6);
            assert!(!batch.is_invisible());
        }
    }

    /// **The fit is the orbit box**, so the sprites' centres lie within the
    /// frame's rectangle, and the newest is drawn last and brightest.
    #[test]
    fn sprites_sit_on_the_frames_edges_and_the_newest_is_on_top() {
        let model = autocast();
        let sequence = model.skeleton.as_ref().and_then(|s| s.sequences.first());
        let batch = &flatten(&model, sequence, 0, 5_000)[0];
        // Centres: the mean of each quad's four corners.
        let centres: Vec<(f32, f32)> = batch
            .vertices
            .chunks(4)
            .map(|q| {
                let x = q.iter().map(|v| v.x).sum::<f32>() / 4.0;
                let y = q.iter().map(|v| v.y).sum::<f32>() / 4.0;
                (x, y)
            })
            .collect();
        for (x, y) in &centres {
            assert!((-1e-3..=1.0 + 1e-3).contains(x), "{x}");
            assert!((-1e-3..=1.0 + 1e-3).contains(y), "{y}");
        }
        // A one-second trail over a two-second lap covers two of the four
        // edges: the centres are not all on one line.
        let xs: Vec<f32> = centres.iter().map(|c| c.0).collect();
        let ys: Vec<f32> = centres.iter().map(|c| c.1).collect();
        let spread = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread(&xs) > 0.5 || spread(&ys) > 0.5);
        // Oldest first: the last quad is the newest, at full alpha and the
        // birth size; the first is nearly dead.
        let last = &batch.vertices[batch.vertices.len() - 4..];
        let first = &batch.vertices[..4];
        assert!(last[0].tint[3] > 0.99, "{:?}", last[0].tint);
        assert!(first[0].tint[3] < 0.02, "{:?}", first[0].tint);
        assert!((last[1].x - last[0].x) > (first[1].x - first[0].x), "the newest is the largest");
    }

    /// **The clock is the wall clock and births are on a fixed grid**, so a
    /// sprite born at one frame is at the same place the next frame.
    #[test]
    fn a_sprite_keeps_its_place_between_frames() {
        let model = autocast();
        let sequence = model.skeleton.as_ref().and_then(|s| s.sequences.first());
        let a = flatten(&model, sequence, 0, 4_000);
        let b = flatten(&model, sequence, 0, 4_016);
        // The newest sprite of frame A is still alive in frame B, four births
        // further from the end (300/s over 16 ms is 4.8, floored).
        let newest_a = &a[0].vertices[a[0].vertices.len() - 4..];
        let same_in_b = &b[0].vertices[b[0].vertices.len() - 4 * 5..b[0].vertices.len() - 4 * 4];
        assert!((newest_a[0].x + (newest_a[1].x - newest_a[0].x) / 2.0
            - (same_in_b[0].x + (same_in_b[1].x - same_in_b[0].x) / 2.0))
            .abs()
            < 1e-4);
    }
}

