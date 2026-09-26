//! The floor effects, painted **onto the ground** instead of through it.
//!
//! A spell's ground art is authored as a flat rectangle at the model's own
//! `z = 0` — Arcane Explosion's three spirals, Battle Shout's crescents, the
//! paladin auras' rings — because the model has no idea what it is standing on.
//! Drawn as free geometry that is exactly what it looks like: a plane held at
//! the caster's feet, buried by the first slope that rises under it and floating
//! over the first that falls away. This module re-renders those quads as a
//! **surface-conforming mesh**: the same texture, the same UVs, the same
//! animated slide and spin, sampled onto whatever the ground actually is.
//!
//! ## Which half of this is the game's own behaviour, and which is not
//!
//! **The mechanism is the client's.** 1.12 has a ground-decal projector and uses
//! it for two things: the **selection ring** under a target and the **unit blob
//! shadow**, with a no-receiver gate skipping the draw outright. Both project onto the drawn surfaces and drape steps and
//! ledges, which is why a selection circle in the real client follows the hill a
//! creature is standing on. **Both of them drape now.** The selection ring is a
//! caller of this projector; the blob shadow takes the same heightfield sample
//! into its own merged mesh rather than one entity per unit — see
//! [`crate::render::shadows`], whose one-mesh-one-draw argument outranks sharing
//! the code here, and which reads the reference's own no-receiver gate
//! differently for a reason it states.
//!
//! **Applying it to the spell quads is not.** Nothing in the 1.12 spell-visual
//! chain conforms anything; the real client draws those batches through the
//! ordinary M2 pipeline and lets the depth test bury them. So this is a
//! deliberate improvement on the reference rather than a reconstruction of it,
//! and it is written down here rather than left to be rediscovered later as a
//! discrepancy.
//!
//! **And the projection itself is a heightfield sample, not a triangle clip.**
//! The reference gathers the receiving triangles and clips each to the decal's
//! box, so its output is exactly coplanar with what is on screen. This samples a
//! grid instead — `(GRID + 1)²` nodes over the posed quad, each asking the same
//! two questions the mover asks about the ground
//! ([`crate::world::session::ActiveSession::terrain_heights`] and
//! [`vale_assets::CollisionWorld::floor_batch`]). That is the right trade
//! *here* because this client's terrain is a heightfield with no triangles at
//! all — the mover, the camera and this all ask it the same way — and because a
//! grid finer than the ground's own 4.17-yard vertex spacing reproduces its
//! triangles exactly. What it cannot do is drape a vertical face: a step gets a
//! stretched ramp where the reference gets a smeared texel column, and a sample
//! that falls outside the decal's vertical slab is cut rather than faded.
//! Nothing in the shipped ground-quad population is authored to need either.
//!
//! ## What a decal is made of
//!
//! One [`GroundDecal`] entity per ground quad of one attached model, spawned by
//! `world::entities::effects::hang_model` in place of the mesh child that batch
//! would otherwise have been. It is a **root** entity, like an emitter and for
//! the same reason — its mesh is in world space and a parent's transform must
//! not compose into it — so it is tied to its owner by liveness
//! ([`retire_decals`]) rather than by the hierarchy.
//!
//! It keeps the batch's own material, so the blend mode, the texture and the
//! animated `M2Color` tint are the file's; it carries `EntityPart` and a
//! `MeshTag` so `animate_attachment` writes that tint into it exactly as it does
//! for an ordinary part.

use crate::axes;
use crate::render::models::M2Material;
use crate::render::nothing;
use crate::world::entities::EntityPart;
use crate::world::session::{ActiveSession, Session, Solids};
use vale_assets::world::m2::GroundQuad;
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};

/// Cells per side of the sampled grid, so `(GRID + 1)²` height queries per
/// rebuild.
///
/// Eight is chosen against the *ground's* own resolution rather than against the
/// decal's: an `MCNK` cell is 4.17 yards across and the effect quads run 2–12
/// yards, so eight cells is at or past the point where the sample reproduces the
/// terrain triangle it sits on and a finer grid only interpolates something
/// already exact.
const GRID: usize = 8;

/// How far above the sampled ground the decal is drawn, in yards.
///
/// The same argument as the blob shadow's `SHADOW_BIAS`, which is the client's
/// own `shadowBias` CVar by another name: this mesh is coplanar with the ground
/// by construction and coplanar geometry z-fights. Smaller than the blob's,
/// because that one is a single flat quad *approximating* a slope and this one
/// follows it — so this has to clear float noise and nothing else.
const LIFT: f32 = 0.03;

/// How far from a decal's own plane the ground may be and still receive it, as a
/// multiple of the quad's larger half-extent.
///
/// A ring cast at the foot of a cliff must not climb it, and one cast on a
/// rooftop must not pour over the edge and down the wall. Past this the sample
/// is dropped and the cells around it are not emitted.
const SLAB: f32 = 2.0;

/// One flat ground quad of one attached model, re-projected onto the ground.
#[derive(Component)]
pub struct GroundDecal {
    /// The attachment root this belongs to. Gone means [`retire_decals`]
    /// despawns this too — the emitters' rule, for the emitters' reason.
    owner: Entity,
    /// The joint whose pose animates the quad — the effect model's own bone, so
    /// the authored slide, spin and scale all survive. `None` for a boneless
    /// model, which rides the owner's own frame.
    joint: Option<Entity>,
    /// The corners in Bevy model space (`y = 0`), bilinear rectangle order —
    /// the file's own numbers, with **nothing** folded into them: everything
    /// between model space and the world is in the carrier's frame.
    corners: [Vec3; 4],
    /// The authored UV at each corner, parallel to [`Self::corners`].
    uvs: [[f32; 2]; 4],
    /// The posed corners the current mesh was built from. NaN-seeded, so the
    /// first pass always projects.
    cached: [Vec3; 4],
    /// How many hulls the collision world held when it was built — a static
    /// pose still has to re-project when a tile streams in under it.
    cached_hulls: usize,
}

impl GroundDecal {
    /// **Whose this quad is** — the attachment root its life is tied to.
    /// Read-only, for the reason `Emitter::owner` gives.
    pub fn owner(&self) -> Entity {
        self.owner
    }
}

/// Build the entity for one ground quad. Nothing here is a child of anything;
/// the tie to the model that owns it is [`GroundDecal::owner`].
///
/// `joint` is the entity carrying the quad's own bone, resolved by the caller
/// against the attached model's joint list — a bone the model does not have
/// falls back to `None`, which rides the attachment root, the same single frame
/// a boneless effect rides.
///
/// **The kit's scale is not applied here**, and folding it in as well is a decal
/// drawn at the square of it. Both carriers already have it: an attachment root's
/// local transform is `bone x offset x scale` ([`crate::world::entities::animate`])
/// and a joint is that composed with the effect's own pose, so the scale is in
/// the frame this poses the corners through whichever of the two it lands on. It
/// is invisible on the shipped population's commonest value — 1.00, which is
/// what Arcane Explosion and every effect traced so far carries — which is
/// exactly why it is written down rather than left to be found by eye later.
pub fn spawn_ground_decal(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    owner: Entity,
    joint: Option<Entity>,
    quad: &GroundQuad,
    material: Handle<M2Material>,
) -> Entity {
    // Model space becomes Bevy's here, exactly as a batch's vertices do.
    let corners = quad.corners.map(axes::to_bevy);
    commands
        .spawn((
            GroundDecal {
                owner,
                joint,
                corners,
                uvs: quad.uvs,
                cached: [Vec3::splat(f32::NAN); 4],
                cached_hulls: usize::MAX,
            },
            Mesh3d(meshes.add(empty_mesh())),
            MeshMaterial3d(material),
            Transform::default(),
            // Hidden until the first projection lands: a decal cast in mid-air
            // receives nothing at all, and the reference skips the whole draw
            // for exactly that.
            Visibility::Hidden,
            // The batch's animated colour, on the same terms as any other part —
            // `animate_attachment` writes the tag through this marker. Zero is
            // a transparent black, so the spawner writes the real value the way
            // it does for a mesh child.
            EntityPart,
            bevy::mesh::MeshTag(0),
            // Rewritten with the projection: the mesh changes shape every frame,
            // so a mesh-derived box would describe the last one.
            Aabb::from_min_max(Vec3::ZERO, Vec3::ZERO),
        ))
        .id()
}

/// This module's work, so that something which *spawns* a decal can order
/// itself in front of the projection rather than in front of a named system.
///
/// It exists because there is a spawner outside this file now
/// ([`crate::render::selection`]) and a decal spawned after the projection has
/// run is a frame of an unprojected mesh at the world origin — under the map.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct DecalSet;

pub struct DecalPlugin;

impl Plugin for DecalPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                retire_decals,
                // **After the joints are posed**, said rather than inherited —
                // the same ordering the emitters take and for the same reason:
                // a frame-late joint drags the spiral a stride behind the caster
                // walking out of it.
                project_decals
                    .after(retire_decals)
                    .after(crate::world::entities::animate)
                    .after(crate::world::entities::animate_areas),
            )
                .in_set(DecalSet),
        );
    }
}

/// Despawn decals whose owner is gone — the emitters' rule.
fn retire_decals(mut commands: Commands, decals: Query<(Entity, &GroundDecal)>, owners: Query<()>) {
    for (entity, decal) in &decals {
        if owners.get(decal.owner).is_err() {
            commands.entity(entity).despawn();
        }
    }
}

/// Re-project every decal whose quad moved.
///
/// The gate is the **posed corners**, which capture the whole of what the pose
/// does — a slide, a spin, a scale — so a static ring under a static caster
/// costs one four-vector compare a frame. It is `!=` on floats deliberately: the
/// question is "did the pose write the same numbers", not "are these close".
///
fn project_decals(
    session: Res<Session>,
    solids: Res<Solids>,
    mut meshes: ResMut<Assets<Mesh>>,
    frames: Query<&GlobalTransform, Without<GroundDecal>>,
    mut decals: Query<(
        &mut GroundDecal,
        &Mesh3d,
        &mut Transform,
        &mut Visibility,
        &mut Aabb,
    )>,
    mut scratch: Local<Scratch>,
) {
    if decals.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // The hull count stands in for "has the world under me changed": a tile
    // streaming in or out is the only thing that moves the ground under a decal
    // that is not moving itself.
    let hulls = solids.0.counts().0;
    for (mut decal, mesh, mut transform, mut visibility, mut aabb) in &mut decals {
        let carrier = decal.joint.unwrap_or(decal.owner);
        // The joint's `GlobalTransform` is written by `animate`, which may not
        // have reached an attachment spawned this very frame.
        let Ok(frame) = frames.get(carrier).copied() else {
            continue;
        };
        let affine = frame.affine();
        let posed = decal.corners.map(|c| affine.transform_point3(c));
        if posed == decal.cached && hulls == decal.cached_hulls {
            continue;
        }
        decal.cached = posed;
        decal.cached_hulls = hulls;

        let Some(mut mesh) = meshes.get_mut(&mesh.0) else {
            continue;
        };
        match project(&posed, &decal.uvs, active, &solids.0, &mut scratch, &mut mesh) {
            Some((centre, half)) => {
                *transform = Transform::from_translation(centre);
                *visibility = Visibility::Inherited;
                *aabb = Aabb::from_min_max(-half, half);
            }
            // Nothing under it: the reference's own no-receiver gate.
            None => *visibility = Visibility::Hidden,
        }
    }
}

/// The four buffers the projection reuses across decals and across frames —
/// one allocation for the life of the app rather than four per rebuild.
#[derive(Default)]
struct Scratch {
    xy: Vec<(f32, f32)>,
    terrain: Vec<Option<f32>>,
    building: Vec<Option<f32>>,
    /// The two joined — what [`build`] takes, so that the half of the
    /// projection which needs no world can be driven by a test.
    surface: Vec<Option<f32>>,
}

/// Sample the ground under one posed quad and write the conforming mesh.
///
/// Returns the mesh's world-space centre — the entity's translation, and
/// therefore its sort anchor — and the half-extent of the geometry around it, or
/// `None` when nothing under it received the decal.
fn project(
    posed: &[Vec3; 4],
    uvs: &[[f32; 2]; 4],
    active: &ActiveSession,
    solids: &vale_assets::CollisionWorld,
    scratch: &mut Scratch,
    mesh: &mut Mesh,
) -> Option<(Vec3, Vec3)> {
    let map_id = active.map_id;
    let nodes = (GRID + 1) * (GRID + 1);
    let centre = (posed[0] + posed[1] + posed[2] + posed[3]) * 0.25;
    // The quad's two edge vectors, horizontally: what the slab is scaled by, and
    // the degeneracy test. A ring whose first animation frame is at scale zero
    // fits nothing and must not be drawn as a point.
    let ex = (posed[1] - posed[0] + posed[3] - posed[2]) * 0.5;
    let ez = (posed[2] - posed[0] + posed[3] - posed[1]) * 0.5;
    let half = Vec2::new(ex.x, ex.z)
        .length()
        .max(Vec2::new(ez.x, ez.z).length())
        * 0.5;
    if half < 1e-3 {
        return None;
    }
    let slab = SLAB * half;

    // The sample positions, in the world coordinates the two lookups take.
    scratch.xy.clear();
    scratch.xy.reserve(nodes);
    for j in 0..=GRID {
        for i in 0..=GRID {
            let (s, t) = (i as f32 / GRID as f32, j as f32 / GRID as f32);
            let w = axes::to_wow(bilerp3(posed, s, t));
            scratch.xy.push((w[0], w[1]));
        }
    }
    // **The same two questions the mover asks, asked once for the whole grid.**
    // See `world::session::Standing::floor`, which is where the join is written
    // for the character. It is repeated rather than shared because the *rule* is
    // not the same one: a mover wants what it can stand on and asks with a
    // ceiling a step above its feet, and a decal wants the visible surface under
    // a plane it is already lying on.
    let ceiling = axes::to_wow(centre)[2] + slab;
    active.terrain_heights(map_id, &scratch.xy, &mut scratch.terrain);
    solids.floor_batch(map_id, &scratch.xy, ceiling, &mut scratch.building);
    // The higher of the two surfaces, exactly as the mover takes it: a
    // building's floor over the ground it was built on.
    scratch.surface.clear();
    scratch.surface.extend((0..nodes).map(|n| {
        match (scratch.building[n], scratch.terrain[n]) {
            (Some(b), Some(g)) => Some(b.max(g)),
            (b, g) => b.or(g),
        }
    }));

    build(posed, uvs, &scratch.surface, slab, centre, mesh)
}

/// Turn a posed quad and the ground heights under its grid into the conforming
/// mesh — **the half of the projection that answers with no session, no
/// collision world and no window**, and therefore the half that is tested.
///
/// `surface` is one entry per grid node in the same row-major order the sampling
/// walks, holding the height of whatever received the decal there, or `None`
/// where nothing did.
fn build(
    posed: &[Vec3; 4],
    uvs: &[[f32; 2]; 4],
    surface: &[Option<f32>],
    slab: f32,
    centre: Vec3,
    mesh: &mut Mesh,
) -> Option<(Vec3, Vec3)> {
    let nodes = (GRID + 1) * (GRID + 1);
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(nodes);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(nodes);
    let mut out_uvs: Vec<[f32; 2]> = Vec::with_capacity(nodes);
    let mut received = vec![false; nodes];
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for j in 0..=GRID {
        for i in 0..=GRID {
            let n = j * (GRID + 1) + i;
            let (s, t) = (i as f32 / GRID as f32, j as f32 / GRID as f32);
            let plane = bilerp3(posed, s, t);
            let y = match surface[n] {
                Some(z) if (z + LIFT - plane.y).abs() <= slab => {
                    received[n] = true;
                    z + LIFT
                }
                // Off the map, or a surface outside the slab: keep the authored
                // plane so the grid stays a rectangle, and emit no cell here.
                _ => plane.y,
            };
            let p = Vec3::new(plane.x, y, plane.z) - centre;
            lo = lo.min(p);
            hi = hi.max(p);
            positions.push(p.to_array());
            // Flat up. These are `unlit` additive draws — every ground quad in
            // the shipped population is — so the normal is never read; it is
            // written because Bevy's vertex layout expects one.
            normals.push([0.0, 1.0, 0.0]);
            out_uvs.push(bilerp2(uvs, s, t));
        }
    }

    let mut indices: Vec<u32> = Vec::with_capacity(GRID * GRID * 6);
    for j in 0..GRID {
        for i in 0..GRID {
            let a = j * (GRID + 1) + i;
            let (b, c, d) = (a + 1, a + GRID + 1, a + GRID + 2);
            // A cell is emitted only when all four of its corners found ground:
            // a cell straddling the slab's edge would be a triangle standing on
            // its side, which is worse than the gap.
            if !(received[a] && received[b] && received[c] && received[d]) {
                continue;
            }
            // **The winding is `(a, b, c)` and not `(a, c, b)`, and getting it
            // the other way round drew every decal in the game face-down.**
            //
            // The grid's `(s, t)` runs along the quad's authored model-space
            // `+x`/`+y`, and [`axes::to_bevy`] maps those to Bevy's `-z`/`-x` —
            // negating *two* of the three axes, which reverses orientation in
            // the horizontal plane. So the parameterisation this walks is
            // left-handed seen from above, and the obvious winding puts the face
            // normal at `-y`. None of these batches is `two_sided` (`vale
            // model`: `[unlit no-depth-write]` and nothing else), so a
            // back-facing grid is culled outright — no error, no warning, and a
            // spell that simply has no art on the floor.
            //
            // [`the_grid_faces_up`] is the check, and it is a check rather than
            // a comment because the failure is invisible from every direction a
            // player can look from.
            for v in [a, b, c, b, d, c] {
                indices.push(v as u32);
            }
        }
    }
    if indices.is_empty() {
        return None;
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, out_uvs);
    mesh.insert_indices(Indices::U32(indices));
    Some((centre, (hi - lo).abs().max(Vec3::splat(0.01))))
}

/// Bilinear over the quad's four corners at `(s, t)`, in the rectangle order
/// [`GroundQuad::corners`] states.
fn bilerp3(corners: &[Vec3; 4], s: f32, t: f32) -> Vec3 {
    corners[0]
        .lerp(corners[1], s)
        .lerp(corners[2].lerp(corners[3], s), t)
}

/// …and over the authored UVs, parallel to it.
fn bilerp2(uvs: &[[f32; 2]; 4], s: f32, t: f32) -> [f32; 2] {
    let lerp = |a: [f32; 2], b: [f32; 2], k: f32| [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k];
    lerp(lerp(uvs[0], uvs[1], s), lerp(uvs[2], uvs[3], s), t)
}

/// What a decal is spawned holding: a mesh with the right attributes and no
/// triangles that reach the rasteriser.
///
/// It has to *have* the attributes from the start. Bevy chooses the pipeline
/// from the mesh's vertex layout, so a mesh that gains an attribute on its
/// second frame is a different pipeline from the one its material was
/// specialised against on its first.
///
/// **"No triangles" is the degenerate quad and not an empty array**, for the
/// reason [`crate::render::nothing`] gives — and a decal reaches the allocator
/// in this state routinely, not just on its first frame: [`build`] returns
/// `None` without touching the mesh when nothing under the quad received it, so
/// a decal cast in mid-air keeps whatever it was spawned with for its whole
/// life.
fn empty_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    nothing::nothing_drawn(&mut mesh);
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bilinear map has to hit the authored corners exactly, or the texture
    /// slides off the quad it was authored for.
    #[test]
    fn the_bilinear_map_hits_the_corners() {
        let uvs = [[0.0, 0.0], [0.25, 0.0], [0.0, 1.0], [0.25, 1.0]];
        assert_eq!(bilerp2(&uvs, 0.0, 0.0), [0.0, 0.0]);
        assert_eq!(bilerp2(&uvs, 1.0, 0.0), [0.25, 0.0]);
        assert_eq!(bilerp2(&uvs, 0.0, 1.0), [0.0, 1.0]);
        assert_eq!(bilerp2(&uvs, 1.0, 1.0), [0.25, 1.0]);
        assert_eq!(bilerp2(&uvs, 0.5, 0.5), [0.125, 0.5]);
    }

    /// A spun, translated quad's corners still land on the map's corners — the
    /// property that makes the grid follow the bone rather than the world axes.
    #[test]
    fn a_spun_quad_keeps_its_corners() {
        let spin = Quat::from_rotation_y(0.9);
        let corners = [
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(-1.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 1.0),
        ]
        .map(|c| spin * c + Vec3::new(5.0, 3.0, -7.0));
        assert!((bilerp3(&corners, 0.0, 0.0) - corners[0]).length() < 1e-5);
        assert!((bilerp3(&corners, 1.0, 0.0) - corners[1]).length() < 1e-5);
        assert!((bilerp3(&corners, 0.0, 1.0) - corners[2]).length() < 1e-5);
        assert!((bilerp3(&corners, 1.0, 1.0) - corners[3]).length() < 1e-5);
        let mid = (corners[0] + corners[1] + corners[2] + corners[3]) * 0.25;
        assert!((bilerp3(&corners, 0.5, 0.5) - mid).length() < 1e-5);
    }

    /// A quad exactly as `M2::ground_quad` hands one over — the authored
    /// model-space rectangle through [`axes::to_bevy`], which is the only
    /// ordering this module ever sees.
    fn authored_quad() -> [Vec3; 4] {
        [[-2.0, -2.0, 0.0], [2.0, -2.0, 0.0], [-2.0, 2.0, 0.0], [2.0, 2.0, 0.0]]
            .map(axes::to_bevy)
    }

    /// **Every triangle the grid emits faces up**, and this is the test the
    /// round that wrote this module did not have.
    ///
    /// The obvious winding over a `(s, t)` grid is `(a, c, b)`, and it is wrong
    /// here: `to_bevy` negates two of the three axes, so the authored
    /// parameterisation is orientation-reversed in the horizontal plane and that
    /// winding puts the face normal at `-y`. No ground-quad batch in the game is
    /// `two_sided`, so a back-facing grid is culled outright — every spell's
    /// floor art missing, with no error anywhere and nothing on screen to say
    /// which of the dozen things between the file and the pixel had failed.
    #[test]
    fn the_grid_faces_up() {
        let posed = authored_quad();
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        // Flat ground at the quad's own height, so every node receives.
        let surface = vec![Some(0.0); (GRID + 1) * (GRID + 1)];
        let mut mesh = empty_mesh();
        let centre = (posed[0] + posed[1] + posed[2] + posed[3]) * 0.25;
        assert!(build(&posed, &uvs, &surface, 4.0, centre, &mut mesh).is_some());

        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions");
        };
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("no indices");
        };
        assert_eq!(indices.len(), GRID * GRID * 6, "every cell received");
        for triangle in indices.chunks_exact(3) {
            let p = |k: usize| Vec3::from(positions[triangle[k] as usize]);
            let normal = (p(1) - p(0)).cross(p(2) - p(0));
            assert!(
                normal.y > 0.0,
                "triangle {triangle:?} faces {normal:?}, which is culled from above",
            );
        }
    }

    /// A node with nothing under it takes no cell with it, and its **neighbours**
    /// go too — a cell needs all four corners on the ground or it is a triangle
    /// standing on its side at the edge of the slab.
    #[test]
    fn a_node_with_no_receiver_drops_its_cells() {
        let posed = authored_quad();
        let uvs = [[0.0; 2]; 4];
        let mut surface = vec![Some(0.0); (GRID + 1) * (GRID + 1)];
        // One interior node, so all four of the cells around it go.
        surface[(GRID + 1) * 2 + 2] = None;
        let mut mesh = empty_mesh();
        let centre = (posed[0] + posed[1] + posed[2] + posed[3]) * 0.25;
        assert!(build(&posed, &uvs, &surface, 4.0, centre, &mut mesh).is_some());
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("no indices");
        };
        assert_eq!(indices.len(), (GRID * GRID - 4) * 6);
    }

    /// Nothing under any of it is the reference's own no-receiver gate: no mesh,
    /// and the caller hides the decal rather than drawing a plane in the air.
    #[test]
    fn nothing_under_it_projects_nothing() {
        let posed = authored_quad();
        let mut mesh = empty_mesh();
        let centre = (posed[0] + posed[1] + posed[2] + posed[3]) * 0.25;
        assert!(build(&posed, &[[0.0; 2]; 4], &vec![None; (GRID + 1) * (GRID + 1)], 4.0, centre, &mut mesh)
            .is_none());
    }

    /// The spawned mesh must already carry every attribute the projection will
    /// write, or the first real frame recompiles the pipeline.
    #[test]
    fn the_empty_mesh_has_the_attributes_the_projection_writes() {
        let mesh = empty_mesh();
        for attribute in [
            Mesh::ATTRIBUTE_POSITION,
            Mesh::ATTRIBUTE_NORMAL,
            Mesh::ATTRIBUTE_UV_0,
        ] {
            assert!(mesh.attribute(attribute.id).is_some(), "{:?}", attribute.id);
        }
        // …and it must not be *empty*, which is a different requirement with a
        // different reason: see [`crate::render::nothing`]. A decal that never
        // finds ground keeps this mesh for its whole life, so a zero-vertex one
        // here is two use-after-free lines per spell cast.
        assert_eq!(mesh.count_vertices(), nothing::VERTICES);
        assert_eq!(mesh.indices().map(bevy::render::mesh::Indices::len), Some(6));
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;

    /// **The plugin's systems have to be able to coexist in one schedule.**
    ///
    /// Cheap insurance against exactly the class of failure that took the debug
    /// panel down at startup: Bevy rejects a system holding one resource both
    /// ways and takes the process with it (`B0002`), and a conflicting pair of
    /// queries does the same. Neither is visible at compile time and neither
    /// shows up in any headless check, because `--audit` returns before the
    /// render app is built at all.
    ///
    /// It runs an update with no session, which is the early-return path and
    /// the one every frame of the login screen takes.
    #[test]
    fn the_pass_builds_and_runs_with_nothing_in_the_world() {
        let mut app = App::new();
        app.add_plugins((
            bevy::asset::AssetPlugin::default(),
            bevy::transform::TransformPlugin,
        ));
        app.init_asset::<Mesh>();
        app.init_resource::<Session>();
        app.init_resource::<Solids>();
        // The two orderings `DecalPlugin` states are on systems owned by other
        // directories, so this registers the pair by hand rather than pulling
        // in the whole entity pass.
        app.add_systems(Update, (retire_decals, project_decals));
        app.update();
    }
}
