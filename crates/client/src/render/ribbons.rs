//! Ribbon trails: the streak a weapon leaves, the tail behind a fireball, the
//! streamer off a wisp.
//!
//! **A ribbon is the one thing in this renderer with a memory.** Every other
//! drawn thing is a function of the current frame — a mesh at a transform, a
//! particle at a position it integrated to. A trail is the *path* its emitter
//! took over the last half second, and nothing about the emitter's present
//! state contains it. That is why it cannot be a particle emitter with a long
//! tail, and why it needs its own ring of committed edges.
//!
//! The simulation is the 5875 client's:
//!
//! * the emitter's bone-local origin goes through the **live** bone matrix
//!   every frame to give the node;
//! * an **edge** — a vertex pair at `+height_above` and `-height_below` along
//!   the bone's own local **+Y** — is committed at `edges_per_second`;
//! * edges age out at `edge_lifetime` and sag at `2·gravity·dt` while they
//!   live;
//! * the ring draws as one strip, `u` sliding from 0 at the head to 1 at the
//!   tail across the atlas cell, so the texture's own transparent end is the
//!   fade.
//!
//! **The look tracks are sampled, not baked**, and that is the difference
//! between a trail and nothing at all: `Spells\HolySmite_Low_Chest.m2` keys its
//! slash's height `0 -> 0.167 -> 0` over its first 267 ms, so the "constant
//! property" shortcut the particle emitters get away with reads a permanent
//! zero and the slash never draws. `vale model` counts the population that
//! trap applies to — **34 of the 36 keyed-height ribbons start at zero**.
//!
//! Structurally this is `particles.rs` with a different kernel, deliberately:
//! same [`Anchor`], same owner-liveness retirement, same one-mesh-per-emitter
//! rebuilt in world space with the entity's `Transform` carrying only the
//! anchor the transparent phase sorts by, and the same `M2Material` particle
//! branch — a strip's tint rides `ATTRIBUTE_COLOR` exactly as a quad's
//! over-life colour does. Sharing the *shape* is what keeps a fix to one from
//! having to be found and re-made in the other.

use crate::axes;
use crate::render::models::{M2Material, M2Params, Materials, SceneLighting, M2_ALPHA_KEY};
use crate::render::nothing;
use crate::render::particles::{Anchor, ParticleClip};
use vale_assets::world::m2::{M2Ribbon, M2Track};
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::ViewVisibility;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use std::collections::VecDeque;
use std::sync::Arc;

/// A backstop on the ring, far above `rate x lifetime` for anything shipped —
/// the busiest trail in the game is 30 edges/s for half a second.
const MAX_EDGES: usize = 256;

/// One ribbon definition and the material its strip draws through.
pub struct RibbonDraw {
    pub def: M2Ribbon,
    pub material: Handle<M2Material>,
}

/// Every ribbon of one model, shared by all its dressings and placements —
/// the counterpart of `ParticleSet`.
pub struct RibbonSet {
    pub ribbons: Vec<RibbonDraw>,
    /// The same sequence-0 clip the emitters run on. A ribbon's keyed height,
    /// alpha and visibility are on the model's own timeline, so they need the
    /// same clock.
    pub clip: Option<ParticleClip>,
}

/// Build a model's ribbon set, interning one material per trail.
///
/// The texture is the model's own, as an emitter's is. A ribbon that names no
/// texture is **dropped** rather than given the magenta placeholder: a
/// placeholder emitter is a visible magenta spray that says *look here*, but a
/// placeholder trail is a magenta band welded to a weapon for as long as it is
/// carried, and the file is telling us it has nothing to draw.
pub fn build_set(
    defs: &[M2Ribbon],
    clip: Option<ParticleClip>,
    kinds: &[u32],
    own: &[Handle<Image>],
    materials: &mut Materials,
) -> Option<Arc<RibbonSet>> {
    if defs.is_empty() {
        return None;
    }
    let ribbons: Vec<RibbonDraw> = defs
        .iter()
        .filter(|def| def.edges_per_second > 0.0 && def.peak_height() > 0.0)
        .filter_map(|def| {
            let slot = def.texture? as usize;
            let texture = own
                .get(slot)
                .filter(|_| kinds.get(slot) == Some(&0))
                .cloned()?;
            // The client's own per-blend fog policy, the same table the
            // emitters take: an additive trail fogs toward black, so a distant
            // one fades instead of painting a fog-coloured band.
            let mode = match def.blend {
                3 | 4 => 2.0,
                _ => 1.0,
            };
            let material = materials.intern(M2Material {
                params: M2Params {
                    ambient: Vec4::ZERO,
                    alpha_cutoff: crate::render::models::alpha_cut(def.blend, M2_ALPHA_KEY),
                    unlit: 1.0,
                    vertex_lit: 0.0,
                    liquid: 0.0,
                    liquid_close: Vec4::ZERO,
                    liquid_far: Vec4::ZERO,
                    uv_row0: Vec4::ZERO,
                    uv_row1: Vec4::ZERO,
                    // Unlit, like the emitters this rides beside.
                    // Solid: `body.x` is the opacity and 1.0 is "as authored".
            body: Vec4::X,
                    // A ribbon has no environment map — see
                    // `M2Material::overlay_a`.
                    overlay: Vec4::ZERO,
                    scene_ambient: SceneLighting::NONE.ambient,
                    scene_lamps: SceneLighting::NONE.lamps,
                    particle: Vec4::new(mode, 0.0, 0.0, 0.0),
                },
                // **No layers, and the base's own handle in both slots** —
                // see `M2Material::overlay_a`: a binding cannot be empty, and
                // bevy ref-counts bindless resources by id, so naming a handle
                // the material already holds takes no extra slot in the slab.
                overlay_a: texture.clone(),
                overlay_b: texture.clone(),
                texture,
                blend: def.blend,
                two_sided: def.two_sided,
                // A strip is a flat band hanging in the air: writing depth from
                // it would occlude whatever is behind its own transparent end.
                no_depth_write: true,
                // Only the ground foliage sways; see `M2Material::wind`.
                wind: false,
            });
            Some(RibbonDraw {
                def: def.clone(),
                material,
            })
        })
        .collect();
    if ribbons.is_empty() {
        return None;
    }
    Some(Arc::new(RibbonSet { ribbons, clip }))
}

/// One committed edge: the vertex pair across the node, world space, and when
/// it was laid down.
struct Edge {
    top: Vec3,
    bottom: Vec3,
    born: f32,
}

/// One placed trail's live state.
#[derive(Component)]
pub struct Ribbon {
    set: Arc<RibbonSet>,
    index: usize,
    /// Whose existence this trail is tied to. Gone means [`retire_ribbons`]
    /// despawns this too — the same liveness rule the emitters follow, and for
    /// the same reason: a trail entity lives at the world root and does not
    /// despawn with the thing that laid it.
    owner: Entity,
    anchor: Anchor,
    /// The placement's uniform scale. A ribbon's widths are model-space yards,
    /// so a model drawn at half size trails at half width.
    scale: f32,
    /// Newest at the back. The **live head** is not in here — it is appended at
    /// mesh time from the node's current position, so the strip stays welded to
    /// the emitter between commits rather than lagging by up to a whole
    /// commit interval.
    edges: VecDeque<Edge>,
    accumulator: f32,
    /// Seconds since spawn: the clip clock the keyed tracks sample against.
    age: f32,
}

impl Ribbon {
    /// **Whose this trail is** — the entity its life is tied to. Read-only,
    /// for the reason `Emitter::owner` gives: a trail lives at the world root,
    /// and this is the only join back to the thing that laid it.
    pub fn owner(&self) -> Entity {
        self.owner
    }
}

/// Spawn one trail entity per ribbon of a model.
///
/// `anchors` says where each ribbon's node comes from, exactly as it does for
/// the emitters — a joint of the model's own skeleton where it has one, the
/// owner's frame otherwise.
pub fn spawn_ribbons(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    set: &Arc<RibbonSet>,
    owner: Entity,
    anchors: impl Fn(usize, &M2Ribbon) -> Anchor,
    scale: f32,
) -> Vec<Entity> {
    // One switch for the whole family: a subtraction run must
    // take the trails out with the clouds or it measures neither.
    if std::env::var_os("VALE_NO_PARTICLES").is_some() {
        return Vec::new();
    }
    set.ribbons
        .iter()
        .enumerate()
        .map(|(index, ribbon)| {
            commands
                .spawn((
                    Ribbon {
                        set: Arc::clone(set),
                        index,
                        owner,
                        anchor: anchors(index, &ribbon.def),
                        scale,
                        edges: VecDeque::new(),
                        accumulator: 0.0,
                        age: 0.0,
                    },
                    Mesh3d(meshes.add(empty_strip())),
                    MeshMaterial3d(ribbon.material.clone()),
                    Transform::default(),
                    // Hidden until there are two edges to span, so an empty
                    // strip is never queued.
                    Visibility::Hidden,
                    Aabb::from_min_max(Vec3::ZERO, Vec3::ZERO),
                ))
                .id()
        })
        .collect()
}

/// The mesh a trail starts with. Every attribute the material's pipeline
/// variant reads is present from the first frame, so the variant never changes
/// shape underneath it — and, as with the emitters, this is
/// `MAIN_WORLD | RENDER_WORLD` because it is rewritten every frame and a mesh
/// that has been *taken* by the render world panics on `insert_attribute`.
///
/// **It holds the degenerate quad rather than nothing at all**, and a trail is
/// the one of the four per-frame passes where that is guaranteed to matter: a
/// ribbon is spawned with no edges, so `simulate_ribbons` takes its `spans < 2`
/// exit on the first frame of every trail in the game and the mesh reaches the
/// allocator exactly as it was created. See [`crate::render::nothing`] for what
/// an empty one costs.
fn empty_strip() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; nothing::VERTICES]);
    nothing::nothing_drawn(&mut mesh);
    mesh
}

/// Despawn trails whose owner is gone.
///
/// **Immediately, and that is a deliberate difference from the reference.**
/// The 5875 client keeps the model alive while its emitters drain, so a
/// fireball's tail finishes fading after the fireball is gone. Draining here
/// would mean a trail outliving the entity it hangs off — and this renderer
/// despawns an owner at the moment its effect is reaped, so the tail would
/// hang in the air where the model used to be. The visible cost is the last
/// tenth of a second of a trail that was about to end anyway; the visible cost
/// of the other choice is a streak left standing in an empty field, which is
/// the same failure `retire_emitters` exists to prevent.
fn retire_ribbons(mut commands: Commands, ribbons: Query<(Entity, &Ribbon)>, owners: Query<()>) {
    for (entity, ribbon) in &ribbons {
        if owners.get(ribbon.owner).is_err() {
            commands.entity(entity).despawn();
        }
    }
}

/// Place each node, commit and expire edges, and rebuild the strips.
fn simulate_ribbons(
    time: Res<Time>,
    // The trails ride the particle switch, because they are the same subject to
    // anyone looking at the screen and they already share the env kill-switch.
    // Folded in for the same reason `render::particles` folds it in: this
    // system owns their `Visibility`.
    tuning: Res<crate::render::tuning::WorldTuning>,
    mut meshes: ResMut<Assets<Mesh>>,
    frames: Query<&GlobalTransform, Without<Ribbon>>,
    mut ribbons: Query<(
        &mut Ribbon,
        &Mesh3d,
        &mut Transform,
        &mut Visibility,
        &ViewVisibility,
        &mut Aabb,
    )>,
) {
    if !tuning.particles {
        for (_, _, _, mut visibility, _, _) in &mut ribbons {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
        }
        return;
    }
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    let now = time.elapsed_secs();
    let now_ms = (time.elapsed_secs_f64() * 1000.0) as u32;

    for (mut ribbon, mesh3d, mut transform, mut visibility, seen, mut aabb) in &mut ribbons {
        let ribbon = &mut *ribbon;
        let set = Arc::clone(&ribbon.set);
        let Some(draw) = set.ribbons.get(ribbon.index) else {
            continue;
        };
        let def = &draw.def;
        // A joint that vanished mid-frame (a rebuild in progress) holds the
        // ring where it is for a frame rather than laying an edge at the
        // origin, which would draw a band from the model to the world's centre.
        let frame = match &ribbon.anchor {
            Anchor::Fixed(placement) => placement.compute_affine(),
            Anchor::Joint(joint) => match frames.get(*joint) {
                Ok(gt) => gt.affine(),
                Err(_) => continue,
            },
            Anchor::Owner => match frames.get(ribbon.owner) {
                Ok(gt) => gt.affine(),
                Err(_) => continue,
            },
        };

        ribbon.age += dt;
        let clip_ms = clip_time(&set.clip, ribbon.age);
        let sample = |track: &Option<M2Track>, fallback: f32| match (track, &set.clip) {
            (None, _) => fallback,
            (Some(t), _) if t.times.is_empty() => fallback,
            (Some(t), Some(c)) => {
                t.sample(c.start + clip_ms, c.start, c.end, &c.global_sequences, now_ms)[0]
            }
            (Some(t), None) => t.first(),
        };

        // The ON/OFF gate. A dark trail stops committing and lets what it has
        // age out, which is what a thrown weapon wants: the same file is the
        // dagger in the hand and the dagger in flight, and only the second
        // sequence lights the trail.
        let lit = sample(&def.visibility, 1.0) > 0.5;
        let above = sample(&def.height_above, 0.0).max(0.0) * ribbon.scale;
        let below = sample(&def.height_below, 0.0).max(0.0) * ribbon.scale;

        // **The cross-section is the carrying bone's own local +Y**, captured
        // fresh from the live matrix every frame — the reference multiplies
        // only that row by the two heights. It is what makes a sword's trail lie in the plane
        // of the blade instead of standing across it.
        let node = frame.transform_point3(axes::to_bevy(def.position));
        let axis = frame
            .transform_vector3(axes::to_bevy([0.0, 1.0, 0.0]))
            .normalize_or(Vec3::Y);

        while ribbon
            .edges
            .front()
            .is_some_and(|e| now - e.born >= def.edge_lifetime)
        {
            ribbon.edges.pop_front();
        }
        if def.gravity != 0.0 {
            let sag = 2.0 * def.gravity * dt;
            for e in ribbon.edges.iter_mut() {
                e.top.y -= sag;
                e.bottom.y -= sag;
            }
        }
        if lit {
            ribbon.accumulator += def.edges_per_second * dt;
            if ribbon.accumulator >= 1.0 {
                ribbon.accumulator = ribbon.accumulator.fract();
                if ribbon.edges.len() < MAX_EDGES {
                    ribbon.edges.push_back(Edge {
                        top: node + axis * above,
                        bottom: node - axis * below,
                        born: now,
                    });
                }
            }
        } else {
            // Reset rather than let it run: a gate that opens again should lay
            // its first edge at once, not a fraction of an interval late.
            ribbon.accumulator = 0.0;
        }

        let head = lit.then_some((node + axis * above, node - axis * below));
        let spans = ribbon.edges.len() + usize::from(head.is_some());
        let wanted = if spans >= 2 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
        if spans < 2 {
            continue;
        }

        // The anchor the transparent phase sorts by, exactly as an emitter's
        // is: the head while the trail is being laid, the newest surviving
        // edge while it drains.
        let sort_at = match head {
            Some((top, bottom)) => (top + bottom) * 0.5,
            None => {
                let e = ribbon.edges.back().expect("spans >= 2 implies an edge");
                (e.top + e.bottom) * 0.5
            }
        };
        if transform.translation != sort_at {
            transform.translation = sort_at;
        }

        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for e in &ribbon.edges {
            lo = lo.min(e.top.min(e.bottom));
            hi = hi.max(e.top.max(e.bottom));
        }
        if let Some((top, bottom)) = head {
            lo = lo.min(top.min(bottom));
            hi = hi.max(top.max(bottom));
        }
        let grown = Aabb::from_min_max(lo - sort_at, hi - sort_at);
        if (Vec3::from(aabb.center) - Vec3::from(grown.center)).length_squared() > 0.0625
            || (Vec3::from(aabb.half_extents) - Vec3::from(grown.half_extents)).length_squared()
                > 0.0625
        {
            *aabb = grown;
        }

        // Same gate the clouds take: the ring above advanced whether or not
        // anyone can see it, but the geometry build and the asset re-upload
        // only happen for a strip whose draw survived last frame's cull.
        if !seen.get() {
            continue;
        }
        let rgb = match &def.color {
            Some(t) if !t.times.is_empty() => match &set.clip {
                Some(c) => {
                    let v = t.sample(c.start + clip_ms, c.start, c.end, &c.global_sequences, now_ms);
                    [v[0], v[1], v[2]]
                }
                None => {
                    let v = &t.values;
                    [
                        v.first().copied().unwrap_or(1.0),
                        v.get(1).copied().unwrap_or(1.0),
                        v.get(2).copied().unwrap_or(1.0),
                    ]
                }
            },
            _ => [1.0; 3],
        };
        let rgba = [rgb[0], rgb[1], rgb[2], sample(&def.alpha, 1.0).max(0.0)];

        let (rows, cols) = (def.tile_rows.max(1), def.tile_cols.max(1));
        let cell = def.tex_slot.min(rows * cols - 1);
        let (u0, u1) = (
            f32::from(cell % cols) / f32::from(cols),
            f32::from(cell % cols + 1) / f32::from(cols),
        );
        let (v0, v1) = (
            f32::from(cell / cols) / f32::from(rows),
            f32::from(cell / cols + 1) / f32::from(rows),
        );

        let mut positions = Vec::with_capacity(spans * 2);
        let mut uvs = Vec::with_capacity(spans * 2);
        let mut push = |top: Vec3, bottom: Vec3, age01: f32| {
            positions.push((top - sort_at).to_array());
            positions.push((bottom - sort_at).to_array());
            let u = u0 + (u1 - u0) * age01;
            uvs.push([u, v0]);
            uvs.push([u, v1]);
        };
        if let Some((top, bottom)) = head {
            push(top, bottom, 0.0);
        }
        for e in ribbon.edges.iter().rev() {
            push(
                e.top,
                e.bottom,
                ((now - e.born) / def.edge_lifetime).clamp(0.0, 1.0),
            );
        }
        let mut indices = Vec::with_capacity((spans - 1) * 6);
        for k in 0..(spans - 1) as u32 {
            let b = k * 2;
            indices.extend_from_slice(&[b, b + 1, b + 2, b + 1, b + 3, b + 2]);
        }
        if let Some(mut mesh) = meshes.get_mut(&mesh3d.0) {
            let count = positions.len();
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; count]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![rgba; count]);
            mesh.insert_indices(Indices::U32(indices));
        }
    }
}

/// Seconds of trail age onto the clip's millisecond clock — the emitters'
/// rule, shared because it is the same clock.
fn clip_time(clip: &Option<ParticleClip>, age: f32) -> u32 {
    let ms = (age * 1000.0) as u32;
    match clip {
        Some(c) => {
            let span = c.end.saturating_sub(c.start).max(1);
            if c.loops {
                ms % span
            } else {
                ms.min(span)
            }
        }
        None => ms,
    }
}

pub struct RibbonPlugin;

impl Plugin for RibbonPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                retire_ribbons,
                // **Stated, not inherited** — the same ordering the emitters
                // take and for the same reason: a node read before the joints
                // are posed lays this frame's edge at last frame's bone, which
                // on a swinging weapon is the whole width of the swing.
                simulate_ribbons
                    .after(retire_ribbons)
                    .after(crate::world::camera::place)
                    .after(crate::world::entities::animate)
                    .after(crate::render::doodads::stream_doodads),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(start: u32, end: u32, loops: bool) -> Option<ParticleClip> {
        Some(ParticleClip {
            start,
            end,
            loops,
            global_sequences: Arc::from(&[][..]),
        })
    }

    /// A one-shot clip **clamps** rather than wraps, which is what makes a
    /// slash finish: a trail whose height ramps back to zero at 267 ms must
    /// stay at zero, not restart the flare every 1.1 seconds.
    #[test]
    fn a_one_shot_clip_clamps_and_a_looping_one_wraps() {
        assert_eq!(clip_time(&clip(3300, 4433, false), 2.0), 1133);
        assert_eq!(clip_time(&clip(3300, 4433, false), 0.5), 500);
        assert_eq!(clip_time(&clip(0, 1000, true), 2.5), 500);
    }

    /// **A trail never hands Bevy a zero-vertex mesh**, and it is the pass where
    /// that is certain rather than occasional: a ribbon is spawned with no edges
    /// at all, so `simulate_ribbons` takes its `spans < 2` exit on the first
    /// frame of every trail in the game and the mesh reaches the allocator
    /// exactly as [`empty_strip`] built it. An empty one there is two
    /// `Use-after-free` lines per trail — see [`crate::render::nothing`].
    ///
    /// The `COLOR` count is asserted beside the vertex count because it is
    /// written here rather than by the shared writer, and `count_vertices`
    /// silently takes the *shortest* attribute array.
    #[test]
    fn a_new_trail_never_hands_bevy_an_empty_mesh() {
        let mesh = empty_strip();
        assert_eq!(mesh.count_vertices(), nothing::VERTICES);
        assert!(mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR));
        assert_eq!(
            mesh.attribute(Mesh::ATTRIBUTE_COLOR).map(|c| c.len()),
            Some(nothing::VERTICES)
        );
    }

    /// The trap the whole module is arranged around: a keyed height authored
    /// from zero. `peak_height` is what the spawn filter asks, because
    /// `values[0]` would reject exactly the ribbons whose width is the
    /// animated part.
    #[test]
    fn a_slash_authored_from_zero_still_has_a_peak() {
        let mut def = ribbon_def();
        def.height_above = Some(M2Track {
            interpolation: 1,
            global_sequence: -1,
            times: vec![0, 200, 400],
            values: vec![0.0, 0.167, 0.0],
            dim: 1,
        });
        assert_eq!(def.height_above.as_ref().unwrap().values[0], 0.0);
        assert!((def.peak_height() - 0.167).abs() < 1e-6);
    }

    fn ribbon_def() -> M2Ribbon {
        M2Ribbon {
            bone: 0,
            position: [0.0; 3],
            texture: Some(0),
            blend: 4,
            two_sided: true,
            color: None,
            alpha: None,
            height_above: None,
            height_below: None,
            edges_per_second: 30.0,
            edge_lifetime: 0.5,
            gravity: 0.0,
            tile_rows: 1,
            tile_cols: 1,
            tex_slot: 0,
            visibility: None,
        }
    }
}
