//! **Which of a model's batches is a light**, and where it hangs.
//!
//! ## What a light is in 1.12, measured
//!
//! It is not a light record. `vale model` on the Elwynn lamppost —
//! `World\Azeroth\Elwynn\PassiveDoodads\LampPost\LampPost.m2`, 24 of them on one
//! Goldshire tile — prints:
//!
//! ```text
//! [2] tex ELWYNNLANTERN01.BLP  blend 0 opaque     [unlit]                12 tris
//! [3] tex GLOW32.BLP           blend 4 add-alpha  [unlit no-depth-write]  2 tris
//! no lights
//! no ribbon emitters
//! 0 particle emitters
//! ```
//!
//! **No `M2Light`, no emitter, no ribbon.** Every static light in this game —
//! the lampposts, the wall sconces, the hanging lanterns, a window's own glow —
//! is a small **additive, unlit quad** that the client draws over the frame, and
//! nothing else. The reference lights nothing with it, because 1.12 has no local
//! lights at all; the quad *is* the whole of what a lamp does there.
//!
//! That is why [`crate::look`]'s first rule for
//! `crate::render::lamps` — "a lamp is an additive particle emitter" — lit a
//! campfire and left a street of lanterns dark. An emitter is what a **fire**
//! is. This is what a **lamp** is, and they are different populations with
//! almost no overlap.
//!
//! ## The rule, and what it deliberately does not say
//!
//! A batch is a glow when it is **`unlit`** (material flag 0x01, the file's own
//! statement that this surface is not lit by anything, which is what a source
//! is) and its colour is **not driven by an animation track**.
//!
//! ### Why `unlit` alone, and not `unlit` *and* additive
//!
//! The first draft required `blend` 3 or 4 as well, off the Elwynn lamppost
//! quoted above — and that lamppost is the *only* construction in the game with
//! an additive unlit batch. A census over four tiles (`vale models`, which
//! prints it) says a lamp is authored three different ways, and the additive
//! halo is the rarest:
//!
//! ```text
//! GeneralLantern01     blend 0 opaque     [unlit]                 16 tris  ELWYNNLANTERN01
//! Candle01             blend 2 alpha      [unlit no-depth-write]   2 tris  GLOWWHITE32
//! WestfallLampPost02   blend 2 alpha      [unlit no-depth-write]   2 tris  GLOWORANGE32
//! LampPost (Elwynn)    blend 4 add-alpha  [unlit no-depth-write]   2 tris  GLOW32
//! ```
//!
//! So the additive test was not narrowing the rule to lamps — it was narrowing
//! it to *one zone's* lamp. Westfall's lampposts, every candle, every
//! chandelier, both hanging lanterns and the general lantern reported as dark
//! all have an unlit emissive batch and none of them has an additive one. The
//! blend mode says how a surface **composites**; `unlit` says whether it
//! **emits**, and only the second is the question being asked.
//!
//! ### …and why the colour track is the other half
//!
//! Widening to `unlit` alone takes in one population that is not a lamp:
//! Duskwood's **glowing eyes** — `DUSKWOODEYES01.BLP`, two triangles, unlit,
//! carried by `DuskwoodSpookyBush01` and `DuskwoodTreeCanopy02`. They are
//! genuinely self-illuminated and structurally identical to a glow quad, and
//! **size does not separate them**: measured, the eyes are 0.29–0.75y and the
//! lamps 0.16–1.01y, straddling each other. A threshold there would have been a
//! number picked to fit two models.
//!
//! What does separate them is on the batch already: the eyes carry
//! `tint(colour Some(n))`, an **animated colour track**, and not one of the
//! lamps does. That is not a coincidence about eyes — it is the definition of
//! what this module may answer. A [`Glow`] is resolved **once**, at spawn, and
//! stands for as long as the tile is loaded (see the module note on the centre,
//! and `render::lamps::StaticLamps`). A batch whose colour is driven by a track
//! has no single colour to resolve, so a static lamp cannot represent it
//! honestly whatever it turns out to be. Excluding it is correctness, not
//! filtering.
//!
//! An animated **alpha** track is fine and is kept: `WestfallLampPost02`'s
//! flame batch carries `tint(colour None, alpha Some(1))`, and it is the colour
//! that is read here.
//!
//! There is no list of model names here and there must never be one, for the
//! reason [`crate::render::lamps`]' emitter rule has none: a model that glows
//! says so in its own material flags, and a rule read off the data covers the
//! ones nobody has looked at.
//!
//! **Nothing here decides that a glow is a light.** That is a deviation from
//! the game and it belongs to the renderer — see `render::lamps`. What is here is the reading: *this batch adds light to
//! the frame, it is this big, and it hangs here*. The colour is not here either,
//! because it is the texture's and a texture is bytes this module does not have;
//! [`Glow::texture`] names which one, and the caller that decoded it supplies
//! the average. See `render::models::loader`.
//!
//! ## Why the centre and not the bone
//!
//! A glow quad usually rides a billboarded bone, and resolving that bone would
//! be the more general answer. It is not the more *useful* one: every model this
//! rule fires on is static scenery, whose skeleton is at bind pose for ever, and
//! the batch's own vertices are already in model space. Taking the centroid
//! costs one pass over the batch's indices and cannot disagree with what is
//! drawn — which resolving a bone can, and did, for the four cases where the
//! quad is offset from its bone's pivot.

use super::m2::M2;

/// One light a model carries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glow {
    /// Where it hangs, in the model's own space and axes — the centroid of the
    /// batch's own vertices. See the module note on why this is not the bone.
    pub at: [f32; 3],
    /// How big the quad is: half the longest distance between any two of its
    /// vertices, in model yards. A candle's is a few inches and a bonfire's is
    /// several yards, and that ratio is the only thing in the file that says
    /// how much light this is meant to be.
    pub extent: f32,
    /// Which of [`M2::textures`] colours it, or `None` when the lookup does not
    /// resolve — the same `None` [`super::m2::M2Batch::texture`] carries, and
    /// the same reason to drop the batch rather than guess.
    pub texture: Option<u32>,
}

/// **Whether a batch's colour is driven by an animation track**, which is what
/// puts it out of this module's reach — see the module note, under *why the
/// colour track is the other half*. An animated *alpha* does not count: it is
/// the colour that is read here.
fn colour_is_animated(batch: &super::m2::M2Batch) -> bool {
    batch.tint.is_some_and(|tint| tint.color.is_some())
}

/// The lights this model carries, in the order its batches are declared.
///
/// Empty for almost everything — a wall, a tree, a wolf. On the models it does
/// fire for it is one or two: the Elwynn lamppost has both a lit pane and a
/// halo over it, a few inches apart, and `render::lamps`' own `MERGE_RADIUS`
/// folds that pair back into one candidate rather than spending two of
/// twenty-four slots on one lamppost.
pub fn glows(m2: &M2) -> Vec<Glow> {
    m2.batches
        .iter()
        .filter(|b| b.unlit && b.index_count >= 3 && !colour_is_animated(b))
        .filter_map(|b| {
            let start = b.index_start as usize;
            let end = start + b.index_count as usize;
            let indices = m2.indices.get(start..end)?;
            // The batch's own vertices, deduplicated by nothing: a quad's four
            // corners appear twice each across its two triangles and the
            // centroid is unmoved by the repetition, which is why this does not
            // bother to build a set.
            let mut sum = [0.0f64; 3];
            let mut count = 0.0f64;
            let mut corners: Vec<[f32; 3]> = Vec::new();
            for &i in indices {
                let Some(v) = m2.positions.get(i as usize).copied() else {
                    continue;
                };
                for k in 0..3 {
                    sum[k] += f64::from(v[k]);
                }
                count += 1.0;
                if !corners.iter().any(|c| *c == v) {
                    corners.push(v);
                }
            }
            if count == 0.0 {
                return None;
            }
            let at = [
                (sum[0] / count) as f32,
                (sum[1] / count) as f32,
                (sum[2] / count) as f32,
            ];
            let mut widest = 0.0f32;
            for (i, a) in corners.iter().enumerate() {
                for b in &corners[i + 1..] {
                    let d = (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
                    widest = widest.max(d);
                }
            }
            Some(Glow {
                at,
                extent: widest * 0.5,
                texture: b.texture,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::m2::M2Batch;

    fn quad(blend: u16, unlit: bool, at: [f32; 3], side: f32) -> M2 {
        let mut m2 = M2::default();
        let half = side * 0.5;
        for (dx, dy) in [(-half, -half), (half, -half), (half, half), (-half, half)] {
            m2.positions.push([at[0] + dx, at[1] + dy, at[2]]);
        }
        m2.indices = vec![0, 1, 2, 0, 2, 3];
        m2.batches.push(M2Batch {
            geoset: 0,
            index_start: 0,
            index_count: 6,
            texture: Some(0),
            blend,
            unlit,
            two_sided: false,
            no_depth_write: true,
            tint: None,
            uv: None,
        });
        m2
    }

    /// **The rule, both ways round.** An unlit batch is a light whatever it
    /// composites as; a lit one is not, and that is the whole of the reading —
    /// see the module note for the four lamps it was measured off.
    ///
    /// **The blend mode is checked to be irrelevant rather than checked for.**
    /// It used to be half the rule, and that was the bug: `blend` 3 and 4 are
    /// the Elwynn lamppost's construction and *nobody else's*, so the rule fired
    /// on one zone's lamppost and left every candle, chandelier, hanging lantern
    /// and Westfall lamppost in the game dark. `unlit` is the flag that means
    /// "emits"; how the surface composites is a separate question.
    #[test]
    fn a_light_is_unlit_whatever_it_composites_as() {
        // Every blend the game uses, all of them lights when unlit — the four
        // real constructions are 0 (a pane), 2 (a halo) and 4 (an additive
        // halo), and the others cost nothing to allow.
        for blend in [0u16, 1, 2, 3, 4, 5, 6] {
            assert_eq!(
                glows(&quad(blend, true, [0.0, 0.0, 4.0], 1.0)).len(),
                1,
                "blend {blend}, unlit, is a light"
            );
            // Lit: a surface something else shines on, not a source. This is
            // the whole of what the rule excludes on the blend axis, which is
            // to say nothing.
            assert!(
                glows(&quad(blend, false, [0.0, 0.0, 4.0], 1.0)).is_empty(),
                "blend {blend}, lit, is not"
            );
        }
    }

    /// **A batch whose colour is on a track is not a static lamp**, which is
    /// what keeps Duskwood's glowing eyes out — see the module note.
    ///
    /// Stated as a property of *this module's answer* rather than as a fact
    /// about eyes: a [`Glow`] is resolved once and stands for the life of the
    /// tile, so a colour that moves has no value to resolve. An animated
    /// **alpha** is a different track and is kept, because it is the colour
    /// that is read here — `WestfallLampPost02`'s flame batch carries exactly
    /// that and is a light.
    #[test]
    fn an_animated_colour_is_not_a_static_lamp_but_an_animated_alpha_is() {
        use crate::world::m2::BatchTint;

        let mut animated = quad(2, true, [0.0, 0.0, 4.0], 1.0);
        animated.batches[0].tint = Some(BatchTint {
            color: Some(0),
            transparency: None,
        });
        assert!(glows(&animated).is_empty(), "a colour track is not resolvable");

        let mut flickering = quad(2, true, [0.0, 0.0, 4.0], 1.0);
        flickering.batches[0].tint = Some(BatchTint {
            color: None,
            transparency: Some(1),
        });
        assert_eq!(
            glows(&flickering).len(),
            1,
            "an alpha track leaves the colour resolvable"
        );
    }

    /// Where it hangs and how big it is — the two numbers the renderer turns
    /// into a position and a reach.
    #[test]
    fn a_glow_is_measured_where_it_is_drawn() {
        let found = glows(&quad(4, true, [3.0, -2.0, 4.5], 2.0));
        let glow = found.first().expect("one glow");
        // The centroid of a square is its centre, whatever order the two
        // triangles walk it in.
        assert!((glow.at[0] - 3.0).abs() < 1e-4, "{:?}", glow.at);
        assert!((glow.at[1] + 2.0).abs() < 1e-4, "{:?}", glow.at);
        assert!((glow.at[2] - 4.5).abs() < 1e-4, "{:?}", glow.at);
        // …and the extent is half the diagonal, not half the side: a quad two
        // yards across measures 2.83 corner to corner.
        assert!((glow.extent - 2.0_f32.sqrt()).abs() < 1e-4, "{}", glow.extent);
        assert_eq!(glow.texture, Some(0));
    }

    /// A batch with no triangles in it is not a light, however it is flagged —
    /// the guard that keeps a degenerate declaration out of the light budget.
    #[test]
    fn a_batch_with_no_geometry_is_not_a_light() {
        let mut m2 = quad(4, true, [0.0, 0.0, 0.0], 1.0);
        m2.batches[0].index_count = 0;
        assert!(glows(&m2).is_empty());
        // …and one whose indices point past the end of the buffer, which is the
        // shape a damaged tail produces. See `ChunkReader`'s own note.
        let mut past = quad(4, true, [0.0, 0.0, 0.0], 1.0);
        past.batches[0].index_start = 900;
        assert!(glows(&past).is_empty());
    }
}
