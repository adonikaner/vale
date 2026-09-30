//! The strip a weapon leaves behind it while a melee ability plays.
//!
//! A kit's weapon trail ([`vale_assets::tables::spell::WeaponTrail`]) is armed
//! on a unit by `crate::world::entities`, which spawns one [`WeaponTrailStrip`]
//! per weapon model the unit carries when its next animation starts. This
//! module records the weapon's two points every frame, runs the opacity clock
//! ([`vale_assets::look::weapon_trail::TrailClock`], where the rule is), and
//! rebuilds the strip's mesh.
//!
//! The strip is untextured and alpha blended, and every vertex carries the
//! kit's colour and the pair's opacity. It shares the shape of
//! [`crate::render::ribbons`]: a world-space mesh rebuilt every frame on a root
//! entity whose `Transform` holds only the point the transparent phase sorts
//! by, retired when the weapon it follows is gone.

use crate::render::models::{M2Material, M2Params, Materials, SceneLighting};
use crate::render::nothing;
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::ViewVisibility;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::VecDeque;
use vale_assets::look::weapon_trail::{TrailClock, TrailPoints, RING_PAIRS};
use vale_assets::tables::spell::WeaponTrail;

/// The one material every strip draws through: a white texel, alpha blended,
/// unlit, two-sided and not writing depth. The colour is per vertex.
#[derive(Resource)]
pub struct TrailMaterial(pub Handle<M2Material>);

/// Where a strip reads the weapon's two points from: the weapon model's own
/// joints when it has a skeleton, or the attachment's root when it does not.
/// The points are in model space and every joint's matrix maps model space
/// to the world, so either is applied to the point as it stands.
#[derive(Clone, Copy)]
pub struct TrailAnchors {
    pub bottom: Entity,
    pub top: Entity,
}

/// One weapon's trail.
#[derive(Component)]
pub struct WeaponTrailStrip {
    /// The attachment root of the weapon. When it no longer exists the strip
    /// is despawned: a weapon put away, or a unit rebuilt or gone.
    owner: Entity,
    anchors: TrailAnchors,
    points: TrailPoints,
    /// Linear 0..1 of the kit's colour bytes; see the note in `m2.wgsl` on
    /// `ATTRIBUTE_COLOR`, which multiplies in byte space.
    colour: [f32; 3],
    clock: TrailClock,
    /// Recorded pairs, `(bottom, top)`, oldest at the front.
    ring: VecDeque<(Vec3, Vec3)>,
}

impl WeaponTrailStrip {
    /// The weapon this strip follows.
    pub fn owner(&self) -> Entity {
        self.owner
    }
}

/// Spawns one strip on the weapon whose attachment root is `owner`.
pub fn spawn_strip(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: &TrailMaterial,
    owner: Entity,
    anchors: TrailAnchors,
    points: TrailPoints,
    trail: &WeaponTrail,
    now_ms: u64,
) -> Entity {
    commands
        .spawn((
            WeaponTrailStrip {
                owner,
                anchors,
                points,
                colour: trail.colour.map(|c| f32::from(c) / 255.0),
                clock: TrailClock::new(trail.alpha, trail.duration_ms, now_ms),
                ring: VecDeque::with_capacity(RING_PAIRS),
            },
            Mesh3d(meshes.add(empty_strip())),
            MeshMaterial3d(material.0.clone()),
            Transform::default(),
            Visibility::Hidden,
            Aabb::from_min_max(Vec3::ZERO, Vec3::ZERO),
        ))
        .id()
}

/// The mesh a strip starts with; see [`crate::render::nothing`] for why it is
/// never empty.
fn empty_strip() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; nothing::VERTICES]);
    nothing::nothing_drawn(&mut mesh);
    mesh
}

fn build_material(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: Materials,
) {
    let white = images.add(Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[255, 255, 255, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ));
    let material = materials.intern(M2Material {
        params: M2Params {
            ambient: Vec4::ZERO,
            // The translucent modes' alpha-test reference: a fully
            // transparent fragment is discarded and nothing else.
            alpha_cutoff: crate::render::models::alpha_cut(2, crate::render::models::M2_ALPHA_KEY),
            unlit: 1.0,
            vertex_lit: 0.0,
            liquid: 0.0,
            liquid_close: Vec4::ZERO,
            liquid_far: Vec4::ZERO,
            uv_row0: Vec4::ZERO,
            uv_row1: Vec4::ZERO,
            body: Vec4::X,
            overlay: Vec4::ZERO,
            scene_ambient: SceneLighting::NONE.ambient,
            scene_lamps: SceneLighting::NONE.lamps,
            // The particle branch, which takes its colour and opacity from
            // the vertex, fogged as an ordinary blended draw.
            particle: Vec4::new(1.0, 0.0, 0.0, 0.0),
        },
        overlay_a: white.clone(),
        overlay_b: white.clone(),
        uv_table: crate::render::models::UV_TABLE,
        texture: white,
        blend: 2,
        two_sided: true,
        no_depth_write: true,
        wind: false,
    });
    commands.insert_resource(TrailMaterial(material));
}

fn retire_strips(
    mut commands: Commands,
    strips: Query<(Entity, &WeaponTrailStrip)>,
    owners: Query<()>,
) {
    for (entity, strip) in &strips {
        if owners.get(strip.owner).is_err() {
            commands.entity(entity).despawn();
        }
    }
}

/// Records each weapon's two points, advances the clocks and rebuilds the
/// strips.
fn simulate_strips(
    mut commands: Commands,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    frames: Query<&GlobalTransform, Without<WeaponTrailStrip>>,
    mut strips: Query<(
        Entity,
        &mut WeaponTrailStrip,
        &Mesh3d,
        &mut Transform,
        &mut Visibility,
        &ViewVisibility,
        &mut Aabb,
    )>,
) {
    let now_ms = (time.elapsed_secs_f64() * 1000.0) as u64;
    for (entity, mut strip, mesh3d, mut transform, mut visibility, seen, mut aabb) in &mut strips {
        let strip = &mut *strip;
        let Some(frame) = strip.clock.advance(now_ms) else {
            commands.entity(entity).despawn();
            continue;
        };
        if frame.lay {
            // A joint missing for a frame (a rebuild in progress) records
            // nothing rather than a pair at the world origin.
            let (Ok(bottom), Ok(top)) = (
                frames.get(strip.anchors.bottom),
                frames.get(strip.anchors.top),
            ) else {
                continue;
            };
            let bottom = bottom.transform_point(crate::axes::to_bevy(strip.points.bottom.position));
            let top = top.transform_point(crate::axes::to_bevy(strip.points.top.position));
            if strip.ring.len() == RING_PAIRS {
                strip.ring.pop_front();
            }
            strip.ring.push_back((bottom, top));
        }

        let drawn = frame.pairs.min(strip.ring.len());
        let wanted = if drawn >= 2 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
        if drawn < 2 {
            continue;
        }
        let pairs = strip.ring.range(strip.ring.len() - drawn..);

        let (newest_bottom, newest_top) = *strip.ring.back().expect("drawn >= 2");
        let sort_at = (newest_bottom + newest_top) * 0.5;
        if transform.translation != sort_at {
            transform.translation = sort_at;
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for (bottom, top) in pairs.clone() {
            lo = lo.min(bottom.min(*top));
            hi = hi.max(bottom.max(*top));
        }
        *aabb = Aabb::from_min_max(lo - sort_at, hi - sort_at);

        // The clock above runs whether or not the strip is on screen; the
        // mesh is rebuilt only for one that passed last frame's cull.
        if !seen.get() {
            continue;
        }
        let mut positions = Vec::with_capacity(drawn * 2);
        let mut colours = Vec::with_capacity(drawn * 2);
        let [r, g, b] = strip.colour;
        for (i, (bottom, top)) in pairs.enumerate() {
            let alpha = f32::from(frame.pair_alpha(i)) / 255.0;
            positions.push((*bottom - sort_at).to_array());
            positions.push((*top - sort_at).to_array());
            colours.push([r, g, b, alpha]);
            colours.push([r, g, b, alpha]);
        }
        let mut indices = Vec::with_capacity((drawn - 1) * 6);
        for k in 0..(drawn - 1) as u32 {
            let b = k * 2;
            indices.extend_from_slice(&[b, b + 1, b + 2, b + 1, b + 3, b + 2]);
        }
        if let Some(mut mesh) = meshes.get_mut(&mesh3d.0) {
            let count = positions.len();
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; count]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; count]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
            mesh.insert_indices(Indices::U32(indices));
        }
    }
}

pub struct WeaponTrailPlugin;

impl Plugin for WeaponTrailPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, build_material).add_systems(
            Update,
            (
                retire_strips,
                // After the weapon's joints are posed for this frame, or the
                // pair is recorded where the blade was a frame ago.
                simulate_strips
                    .after(retire_strips)
                    .after(crate::world::entities::animate),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// See `ribbons`' test of the same name: a strip is spawned with no pairs
    /// and must not hand Bevy an empty mesh on its first frame.
    #[test]
    fn a_new_strip_never_hands_bevy_an_empty_mesh() {
        let mesh = empty_strip();
        assert_eq!(mesh.count_vertices(), nothing::VERTICES);
        assert_eq!(
            mesh.attribute(Mesh::ATTRIBUTE_COLOR).map(|c| c.len()),
            Some(nothing::VERTICES)
        );
    }
}
