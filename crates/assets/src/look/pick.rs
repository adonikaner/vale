//! **What the mouse is actually on** — the volume a click has to hit to name a
//! unit.
//!
//! Nothing on the wire says this and no file states it as a rule: the server has
//! `UNIT_FIELD_BOUNDINGRADIUS`, which is a *footprint* it uses for reach and
//! which the selection ring is drawn at, and the M2 carries several boxes with
//! no note saying which one a pointer means. So it is the client's own rule,
//! and the answer is two stages that are nothing like each other.
//!
//! ## Stage one is a sphere, and it is the *sequence's* sphere
//!
//! The client walks the pickable objects and tests each with a ray against one
//! sphere, whose centre and radius come from — this is the whole find —
//! **the animation the unit is playing right now**: the bounding box and
//! radius of the sequence it is playing (an `M2Sequence` record in the M2's
//! animation block), or for a pickable with no animator, the collision box.
//!
//! Then `centre = (min + max) * 0.5`, radius =
//! the sequence's own `+0x18` past the box — which is `+0x3C` of the record —
//! and **if that radius is not positive the model's header box and radius at
//! `+0xb4`/`+0xcc` are used instead**. Both are transformed by the
//! object's matrix, the radius by the matrix's first-row length, and the test is
//! the ordinary ray/sphere: miss if the far root is behind the eye or the near
//! root is past the end of the ray.
//!
//! **The difference between the two boxes is not a detail.** `M2::bounds` is the
//! union over every sequence *and* every emitter a file has:
//! `Creature\Wisp\Wisp.m2` declares a symmetric 12.8-yard cube covering the dust
//! it sprays, and the wisp inside it is about a yard. A pick built on the header
//! box hovers that wisp from six yards away — which is the report this module
//! exists for, in its most extreme form.
//!
//! ## Stage two is the drawn triangles, and it is what makes the pick tight
//!
//! A sphere around a humanoid has a radius of about *half its height*, so stage
//! one on its own is looser than the box it replaces, not tighter. The reference
//! is tight because it does not stop there: the sphere hits are **sorted by
//! distance** and then each is tested, nearest first, against the model's own
//! **posed** triangles — the M2's skin view, walked by its texture units. So
//! what is clickable is the silhouette the unit is drawn in,
//! at the frame it is drawn in, and a click between a running creature's legs
//! misses it.
//!
//! [`hit_mesh`] is that walk, with the skinning left to the caller — see its own
//! note for why that is a closure and not a pose array.
//!
//! ## Two deviations, both stated
//!
//! * **Every drawn triangle is tested, not the reference's filtered subset.**
//!   The loop skips a texture unit whose flag byte has `0x8` set and, for
//!   pickable class 2 only, one whose render flags name a non-opaque blend.
//!   Which class a *unit* is, is not established — the client maps it from
//!   the object type id — so neither
//!   filter is applied here. Both would only ever *remove* triangles, so the
//!   error is one-sided and towards selecting, which is the direction the whole
//!   pick already errs in.
//! * **Every geoset is tested**, including the ones the wearer is not drawing.
//!   A character's hairstyles all sit inside its head and its sleeve variants
//!   inside its arms, so this is a fraction of a yard at the wrists in exchange
//!   for the pick not depending on the dressing.

use crate::world::m2::{M2, M2Sequence};

/// The stage-one volume, in **model** space: the file's own axes, origin at the
/// model's feet.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sphere {
    pub centre: [f32; 3],
    pub radius: f32,
}

/// The model's own header sphere — `+0xb4` and `+0xcc` — which is the fallback
/// every branch below ends in.
///
/// Held separately from [`sequence_sphere`] because the renderer has this and
/// not the `M2`: a model is parsed once on a loader thread and the pointer asks
/// about it sixty times a second for the rest of the session.
pub fn model_sphere(model: &M2) -> Sphere {
    Sphere {
        centre: centre_of(model.bounds),
        radius: model.bounding_radius,
    }
}

/// The broad-phase sphere for a model playing `sequence`, or for one playing
/// nothing at all.
///
/// `sequence` is `None` for a model with no skeleton — a chest, a door, a
/// signpost — which is the reference's class-3 case in everything but which box
/// it reads: it has no animator to ask, so the header's is all there is.
///
/// **A sequence that states a zero radius falls back to the model's own**, which
/// is the reference's own fallback and is not rare — 279 of the bestiary's 10,790
/// sequences leave the field at zero (`vale pick`), and reading it literally
/// would make a unit playing one of them unclickable.
pub fn sequence_sphere(model: Sphere, sequence: Option<&M2Sequence>) -> Sphere {
    match sequence {
        Some(seq) if seq.radius > 0.0 => Sphere {
            centre: centre_of(seq.bounds),
            radius: seq.radius,
        },
        _ => model,
    }
}

fn centre_of(bounds: [[f32; 3]; 2]) -> [f32; 3] {
    [
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    ]
}

/// Ray against a sphere: how far along `direction` the ray first touches it, or
/// `None`.
///
/// `direction` is a **unit** vector and `max` is how far the ray reaches, both
/// because that is the shape the reference works in — it normalises the segment
/// once and keeps its length — and because the answer is then a distance in
/// yards rather than a parameter of whatever length the caller's ray happened to
/// have.
///
/// A ray starting *inside* the sphere answers 0 rather than missing
/// (the near root is clamped up to zero), so a pointer over a unit
/// the camera is standing inside still picks it.
pub fn ray_sphere(
    origin: [f32; 3],
    direction: [f32; 3],
    max: f32,
    centre: [f32; 3],
    radius: f32,
) -> Option<f32> {
    let to_centre = sub(centre, origin);
    let along = dot(to_centre, direction);
    // The closest approach, squared, against the radius squared: the classic
    // form, and the one the reference uses rather than a quadratic.
    let closest = sub(scale(direction, along), to_centre);
    let gap = dot(closest, closest);
    let r2 = radius * radius;
    if gap > r2 {
        return None;
    }
    let half_chord = (r2 - gap).sqrt();
    // Behind the eye entirely.
    if along + half_chord < 0.0 {
        return None;
    }
    let near = along - half_chord;
    if near >= max {
        return None;
    }
    Some(near.max(0.0))
}

/// Ray against one triangle — Möller–Trumbore, **two-sided**.
///
/// Two-sided because an M2 batch may be (material flag `0x04`, set on most
/// foliage and on a fair amount of creature trim) and because a back face is
/// still a face of the silhouette: culling here would put a hole in the pick
/// wherever a model's skin is one-sided and seen from inside, which is most
/// capes.
pub fn ray_triangle(
    origin: [f32; 3],
    direction: [f32; 3],
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
) -> Option<f32> {
    const EPSILON: f32 = 1e-7;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let p = cross(direction, ac);
    let det = dot(ab, p);
    if det.abs() < EPSILON {
        return None;
    }
    let inv = 1.0 / det;
    let t = sub(origin, a);
    let u = dot(t, p) * inv;
    if !(-EPSILON..=1.0 + EPSILON).contains(&u) {
        return None;
    }
    let q = cross(t, ab);
    let v = dot(direction, q) * inv;
    if v < -EPSILON || u + v > 1.0 + EPSILON {
        return None;
    }
    let distance = dot(ac, q) * inv;
    (distance >= 0.0).then_some(distance)
}

/// The model's drawn mesh, kept for the narrow phase: positions in **model**
/// space, the skin that moves them, and the triangles over them.
///
/// A copy of four of [`M2`]'s arrays rather than a reference to it, because the
/// renderer drops the `M2` the moment its meshes are uploaded and the narrow
/// phase needs the vertices on the CPU. One of these per model *path* — what a
/// creature is wearing does not change its silhouette enough to matter, and the
/// same argument the collision hull is shared under applies here.
#[derive(Debug, Clone, Default)]
pub struct PickMesh {
    pub positions: Vec<[f32; 3]>,
    /// Four bone weights per vertex, 0..255. Empty for a model with no
    /// skeleton, where [`Self::skinned`] answers the bind position.
    pub weights: Vec<[u8; 4]>,
    pub bones: Vec<[u8; 4]>,
    /// Already through the view's vertex lookup, so these index
    /// [`Self::positions`] directly.
    pub indices: Vec<u16>,
}

impl PickMesh {
    /// Everything the file draws, whatever geoset it belongs to — see the
    /// module note's second deviation.
    pub fn of(model: &M2) -> Self {
        PickMesh {
            positions: model.positions.clone(),
            weights: model.bone_weights.clone(),
            bones: model.bone_indices.clone(),
            indices: model.indices.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.indices.len() < 3
    }

    /// One vertex moved by a pose, in model space.
    ///
    /// The same blend [`M2::skin_position`] does, and it is here as well so that
    /// a caller holding a [`PickMesh`] and no `M2` — which is every caller in
    /// the renderer — can pose it.
    pub fn skinned(&self, vertex: usize, pose: &[[f32; 12]]) -> [f32; 3] {
        let p = self.positions[vertex];
        let (Some(weights), Some(bones)) = (self.weights.get(vertex), self.bones.get(vertex)) else {
            return p;
        };
        let mut out = [0.0f32; 3];
        let mut total = 0.0f32;
        for k in 0..4 {
            let w = f32::from(weights[k]) / 255.0;
            if w == 0.0 {
                continue;
            }
            let Some(m) = pose.get(bones[k] as usize) else {
                continue;
            };
            total += w;
            for r in 0..3 {
                out[r] += w
                    * (m[r * 4] * p[0] + m[r * 4 + 1] * p[1] + m[r * 4 + 2] * p[2] + m[r * 4 + 3]);
            }
        }
        if total == 0.0 { p } else { out }
    }
}

/// **The narrow phase**: the nearest triangle of `mesh` the ray crosses, in
/// whatever space `place` answers in.
///
/// `place` is a *closure* rather than a pose array, and that is the one design
/// decision in this module. The renderer already holds every bone of every
/// visible unit as a world-space joint — written by the same code the GPU skins
/// from — so re-posing here to work in model space would be both slower and, far
/// worse, **a second answer to "what pose is this unit in"**: the drawn one
/// composes a cross-fade, a counter-twist and a masked second track, and a pick
/// that reproduced two of those three would be tight against a silhouette
/// nobody can see. The CLI and the tests hand in [`PickMesh::skinned`] and work
/// in model space; the renderer hands in the joints and works in world space.
/// One traversal, one triangle test, and no way for the two to drift.
///
/// `origin`, `direction` and the answer are in that same space. `direction` need
/// not be normalised, but if it is not, the distance is in units of its length.
pub fn hit_mesh(
    mesh: &PickMesh,
    place: impl Fn(usize) -> [f32; 3],
    origin: [f32; 3],
    direction: [f32; 3],
) -> Option<f32> {
    // Positions are memoised across the triangle walk: a vertex is shared by six
    // triangles on average, and skinning it once per triangle would be six times
    // the work in the one part of this that is not already cheap.
    let mut cache: Vec<Option<[f32; 3]>> = vec![None; mesh.positions.len()];
    let mut nearest: Option<f32> = None;
    for triangle in mesh.indices.chunks_exact(3) {
        let mut corner = [[0.0f32; 3]; 3];
        let mut ok = true;
        for (slot, &index) in corner.iter_mut().zip(triangle) {
            let index = usize::from(index);
            if index >= cache.len() {
                // A batch whose indices run past the vertex array — these are
                // twenty-year-old files, and the draw path already tolerates it.
                ok = false;
                break;
            }
            *slot = *cache[index].get_or_insert_with(|| place(index));
        }
        if !ok {
            continue;
        }
        if let Some(t) = ray_triangle(origin, direction, corner[0], corner[1], corner[2]) {
            nearest = Some(nearest.map_or(t, |best: f32| best.min(t)));
        }
    }
    nearest
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: [f32; 3]) -> [f32; 3] {
        let len = dot(v, v).sqrt();
        scale(v, 1.0 / len)
    }

    /// A sphere two yards ahead is hit at its **near** face, not at its centre
    /// and not at its far side.
    #[test]
    fn a_ray_meets_a_sphere_at_its_near_face() {
        let hit = ray_sphere([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 100.0, [2.0, 0.0, 0.0], 0.5);
        assert!((hit.expect("a hit") - 1.5).abs() < 1e-5, "{hit:?}");
        // Half a yard off-axis, which is exactly tangent, still counts; a yard
        // off does not.
        assert!(ray_sphere([0.0, 0.4, 0.0], [1.0, 0.0, 0.0], 100.0, [2.0, 0.0, 0.0], 0.5).is_some());
        assert!(ray_sphere([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], 100.0, [2.0, 0.0, 0.0], 0.5).is_none());
    }

    /// **Three ways a sphere in the right place is still a miss**, each of which
    /// the reference tests separately and each of which reads as a unit you can
    /// click from somewhere you should not be able to.
    #[test]
    fn a_sphere_behind_the_eye_or_past_the_end_of_the_ray_is_a_miss() {
        let centre = [-3.0, 0.0, 0.0];
        // Behind: the whole chord is at a negative distance.
        assert!(ray_sphere([0.0; 3], [1.0, 0.0, 0.0], 100.0, centre, 0.5).is_none());
        // Past the end: `max` is what bounds the segment, and the reference
        // compares the *near* root against it.
        assert!(ray_sphere([0.0; 3], [1.0, 0.0, 0.0], 1.0, [2.0, 0.0, 0.0], 0.5).is_none());
        assert!(ray_sphere([0.0; 3], [1.0, 0.0, 0.0], 2.0, [2.0, 0.0, 0.0], 0.5).is_some());
        // Inside: clamped to zero rather than answering the negative root, so a
        // camera standing in a unit still picks it.
        assert_eq!(
            ray_sphere([0.0; 3], [1.0, 0.0, 0.0], 100.0, [0.0; 3], 5.0),
            Some(0.0)
        );
    }

    /// The triangle test hits inside and misses outside, **from either side**.
    #[test]
    fn a_ray_crosses_a_triangle_from_either_face() {
        let (a, b, c) = ([0.0, -1.0, -1.0], [0.0, 1.0, -1.0], [0.0, 0.0, 1.0]);
        let front = ray_triangle([-2.0, 0.0, 0.0], [1.0, 0.0, 0.0], a, b, c);
        assert!((front.expect("a hit") - 2.0).abs() < 1e-4, "{front:?}");
        // The same triangle from behind: two-sided, see the function's note.
        let back = ray_triangle([2.0, 0.0, 0.0], [-1.0, 0.0, 0.0], a, b, c);
        assert!((back.expect("a hit") - 2.0).abs() < 1e-4, "{back:?}");
        // Outside the edges, and behind the origin.
        assert!(ray_triangle([-2.0, 5.0, 0.0], [1.0, 0.0, 0.0], a, b, c).is_none());
        assert!(ray_triangle([2.0, 0.0, 0.0], [1.0, 0.0, 0.0], a, b, c).is_none());
    }

    /// **The narrow phase is a hole punch, and that is the whole point of it.**
    ///
    /// Two quads a yard apart with a yard of nothing between them — a pair of
    /// legs seen from the front. The broad-phase sphere covers both and the gap;
    /// the mesh test hits the quads and misses the gap, which is what "a click
    /// between a running creature's legs misses it" means.
    #[test]
    fn the_mesh_test_misses_the_gap_the_sphere_covers() {
        let mut mesh = PickMesh::default();
        for side in [-1.5f32, 0.5] {
            let base = mesh.positions.len() as u16;
            mesh.positions.extend([
                [0.0, side, 0.0],
                [0.0, side + 1.0, 0.0],
                [0.0, side + 1.0, 2.0],
                [0.0, side, 2.0],
            ]);
            mesh.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        let along = [1.0, 0.0, 0.0];
        let at = |y: f32| hit_mesh(&mesh, |i| mesh.positions[i], [-4.0, y, 1.0], along);
        // Through the left leg, then the right.
        assert!((at(-1.0).expect("left") - 4.0).abs() < 1e-4);
        assert!((at(1.0).expect("right") - 4.0).abs() < 1e-4);
        // …and between them, where a box or a sphere would both have answered.
        assert_eq!(at(0.0), None);
        assert!(ray_sphere([-4.0, 0.0, 1.0], along, 100.0, [0.0, 0.0, 1.0], 2.0).is_some());
    }

    /// The mesh test answers the **nearest** crossing, not the first one in
    /// index order — otherwise a unit's far side wins whenever the file happens
    /// to have authored it first, and the sort that orders candidates is
    /// meaningless.
    #[test]
    fn the_mesh_test_answers_the_nearest_crossing() {
        let mut mesh = PickMesh::default();
        // The far wall is authored first, deliberately.
        for x in [6.0f32, 2.0] {
            let base = mesh.positions.len() as u16;
            mesh.positions.extend([
                [x, -1.0, -1.0],
                [x, 1.0, -1.0],
                [x, 1.0, 1.0],
                [x, -1.0, 1.0],
            ]);
            mesh.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        let hit = hit_mesh(&mesh, |i| mesh.positions[i], [0.0; 3], [1.0, 0.0, 0.0]);
        assert!((hit.expect("a hit") - 2.0).abs() < 1e-4, "{hit:?}");
    }

    /// **A pose moves the pick with the model.** The narrow phase is only worth
    /// having if it follows the animation, so this asserts the join rather than
    /// the arithmetic: the same ray misses the bind pose and hits the posed one.
    #[test]
    fn the_mesh_test_follows_the_pose() {
        let mut mesh = PickMesh::default();
        mesh.positions
            .extend([[0.0, -1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 2.0]]);
        mesh.weights.extend([[255, 0, 0, 0]; 3]);
        mesh.bones.extend([[0, 0, 0, 0]; 3]);
        mesh.indices.extend([0u16, 1, 2]);

        let identity = [[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]];
        // The same bone lifted four yards — the translation is the last term of
        // each row, so this is `+4` on z and nothing else.
        let moved = [[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 4.0]];

        // A ray at head height, across the triangle's own plane.
        let ray = |pose: &[[f32; 12]]| {
            hit_mesh(
                &mesh,
                |i| mesh.skinned(i, pose),
                [-3.0, 0.0, 5.0],
                unit([1.0, 0.0, 0.0]),
            )
        };
        assert_eq!(ray(&identity), None, "the bind pose is not under the ray");
        assert!(ray(&moved).is_some(), "the posed triangle is");
    }

    /// The sequence's sphere wins, and **a sequence that states nothing falls
    /// back to the model's** — the branch that keeps a file with empty sequence
    /// boxes clickable at all.
    #[test]
    fn a_sequence_states_its_own_sphere_and_zero_means_the_models() {
        let mut model = M2::default();
        model.bounds = [[-8.0, -8.0, -8.0], [8.0, 8.0, 8.0]];
        model.bounding_radius = 13.8;

        let mut seq = M2Sequence {
            id: 0,
            variation: 0,
            start: 0,
            end: 1000,
            move_speed: 0.0,
            flags: 0,
            probability: 0x7fff,
            bounds: [[-0.5, -0.5, 0.0], [0.5, 0.5, 2.0]],
            radius: 1.2,
        };
        let header = model_sphere(&model);
        assert_eq!(
            sequence_sphere(header, Some(&seq)),
            Sphere {
                centre: [0.0, 0.0, 1.0],
                radius: 1.2
            }
        );
        seq.radius = 0.0;
        let fallback = sequence_sphere(header, Some(&seq));
        assert_eq!(fallback.radius, 13.8);
        assert_eq!(fallback.centre, [0.0, 0.0, 0.0]);
        // …and a model with no skeleton at all takes the same branch.
        assert_eq!(sequence_sphere(header, None), fallback);
    }
}
