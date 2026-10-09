//! The editor's marks in the world, drawn as meshes rather than as lines.
//!
//! A tool describes what it wants this frame — a dome over a waypoint, an
//! arrow on a handle, a dashed ring on the ground — by calling [`Marks`], the
//! same way it would call `Gizmos`. [`show`] then poses one pooled mesh entity
//! per shape and hides the rest. The list is emptied every frame, so a tool
//! that stops drawing leaves nothing behind.
//!
//! ## Why meshes rather than gizmo lines
//!
//! A gizmo line has no thickness and no shading, so it cannot look solid, and
//! Bevy's gizmo groups offer one depth rule per group: depth-tested or always
//! in front. Every aimable instrument here was always in front, because a
//! handle stands inside its own model and a depth-tested handle disappeared
//! into it. That drew a trigger volume buried in a hillside over the hillside,
//! and a path behind a ridge over the ridge, with nothing to say which was in
//! front.
//!
//! A mesh writes and tests depth like the world does, so the ground hides what
//! is under it.
//!
//! ## Hidden parts of what a person aims at are drawn faint
//!
//! [`Look::Ghosted`] draws the mesh twice. The solid pass is depth-tested as
//! usual. The ghost pass uses the opposite depth test, so it draws only where
//! something is in front of the mark, flat and at [`GHOST`] of the colour's
//! opacity. The two passes never cover the same pixel, which is what a path
//! drawn twice as two gizmo groups got wrong: both were visible where nothing
//! hid the path, and the blend order of the two groups changed from frame to
//! frame, so the path flickered.
//!
//! A handle inside a building therefore still shows, faintly, and becomes solid
//! where it comes out of the walls. Picking is unaffected: every tool picks by
//! arithmetic on the pointer's ray, not by what is drawn.
//!
//! [`Look::Solid`] has no ghost. It is for marks that only say where something
//! is, such as every spawn in a forest: a mark behind a tree is not shown.
//!
//! ## Coordinates
//!
//! Bevy's, as for `Gizmos`: a tool converts with `render::axes::to_bevy`
//! first. Up is +Y.
//!
//! ## Colour
//!
//! A colour is what `Color::srgb` states. The shader decodes it, shades it and
//! writes the byte-space value the world target holds; see `marks.wgsl`.

use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{ConeAnchor, Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::transform::TransformSystems;
use vale_client::world::camera::WorldCamera;

/// The shader's address, as `embedded_asset!` registers it.
const SHADER: &str = "embedded://vale_ide/marks.wgsl";

/// The ghost pass's opacity, as a fraction of the mark's own.
pub const GHOST: f32 = 0.35;

/// The thinnest a ghost may be, so a translucent volume's buried half is
/// still seen.
const GHOST_FLOOR: f32 = 0.10;

/// A line's radius in pixels, so it is about three pixels wide wherever it
/// is. Narrower than this, a tube with no multisampling breaks up into
/// dashes of one pixel and none.
const LINE_PIXELS: f32 = 1.5;

/// The most pieces [`Marks::line`] cuts one line into.
const LINE_PIECES: usize = 48;

/// How many yards one pixel covers at a point, from the world camera's
/// projection.
///
/// Read from the projection rather than from the distance to the camera,
/// because the map view is orthographic: there the size of a pixel is the
/// visible area divided by the picture's height and does not depend on the
/// distance at all. Sized by distance, every line in the map view became
/// thinner than a pixel as the view zoomed out, and drew as a faint broken
/// thread.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Scale {
    /// Yards per pixel per yard of depth: `2 tan(fov / 2) / height`.
    Perspective(f32),
    /// Yards per pixel, everywhere.
    Orthographic(f32),
}

impl Default for Scale {
    /// A 60-degree view a thousand pixels tall, until the camera is read.
    fn default() -> Self {
        Scale::Perspective(2.0 * (30.0f32).to_radians().tan() / 1000.0)
    }
}

/// Whether the part of a mark behind something is drawn. See the module
/// comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Look {
    /// Hidden where something is in front of it.
    Solid,
    /// …and drawn faint there.
    Ghosted,
}

/// The meshes a mark is made of. Each is a unit shape posed by a transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Shape {
    /// Radius 1 about the origin.
    Sphere,
    /// The upper half of [`Self::Sphere`], closed underneath.
    Dome,
    /// Radius 1, from y = -0.5 to 0.5.
    Tube,
    /// Base radius 1 at y = 0, tip at y = 1.
    Cone,
    /// Side 1 about the origin.
    Cube,
}

const SHAPES: [Shape; 5] = [Shape::Sphere, Shape::Dome, Shape::Tube, Shape::Cone, Shape::Cube];

#[derive(Debug, Clone, Copy)]
struct Mark {
    shape: Shape,
    pose: Transform,
    colour: [u8; 4],
    look: Look,
}

/// What the tools want drawn this frame. See the module comment.
#[derive(Resource, Default)]
pub struct Marks {
    list: Vec<Mark>,
    /// The world camera's position and the way it looks, read at the start
    /// of the frame.
    eye: Vec3,
    forward: Vec3,
    scale: Scale,
}

impl Marks {
    /// The world camera's position, for sizes that follow the distance.
    pub fn eye(&self) -> Vec3 {
        self.eye
    }

    /// How many yards one pixel covers at `at`. See [`Scale`].
    pub fn pixel(&self, at: Vec3) -> f32 {
        match self.scale {
            // A point beside or behind the eye is given a yard of depth, so
            // nothing is drawn with no thickness at all.
            Scale::Perspective(per_depth) => (at - self.eye).dot(self.forward).max(1.0) * per_depth,
            Scale::Orthographic(per_pixel) => per_pixel,
        }
    }

    /// The radius a line at `at` is drawn with: [`LINE_PIXELS`].
    pub fn line_radius(&self, at: Vec3) -> f32 {
        self.pixel(at) * LINE_PIXELS
    }

    fn push(&mut self, shape: Shape, pose: Transform, colour: Color, look: Look) {
        self.list.push(Mark {
            shape,
            pose,
            colour: colour.to_srgba().to_u8_array(),
            look,
        });
    }

    pub fn sphere(&mut self, centre: Vec3, radius: f32, colour: Color, look: Look) {
        let pose = Transform::from_translation(centre).with_scale(Vec3::splat(radius));
        self.push(Shape::Sphere, pose, colour, look);
    }

    /// A half sphere standing on `base`, flat side down.
    pub fn dome(&mut self, base: Vec3, radius: f32, colour: Color, look: Look) {
        let pose = Transform::from_translation(base).with_scale(Vec3::splat(radius));
        self.push(Shape::Dome, pose, colour, look);
    }

    /// A round bar from `a` to `b`.
    pub fn tube(&mut self, a: Vec3, b: Vec3, radius: f32, colour: Color, look: Look) {
        let along = b - a;
        let length = along.length();
        if length < 1e-4 {
            return;
        }
        let pose = Transform {
            translation: (a + b) * 0.5,
            rotation: Quat::from_rotation_arc(Vec3::Y, along / length),
            scale: Vec3::new(radius, length, radius),
        };
        self.push(Shape::Tube, pose, colour, look);
    }

    /// A tube [`Self::line_radius`] thick.
    ///
    /// In perspective a long line is cut into pieces no longer than half
    /// their own depth, each as thick as its nearer end needs. One thickness
    /// for the whole of a flight path's leg made a leg that starts near the
    /// camera a thread at its far end, and a short leg beside that far end
    /// thick, so two lines at one depth were drawn at two widths.
    pub fn line(&mut self, a: Vec3, b: Vec3, colour: Color, look: Look) {
        let length = a.distance(b);
        if length < 1e-4 {
            return;
        }
        let at = |along: f32| a + (b - a) * (along / length);
        let mut from = 0.0;
        for piece in 0..LINE_PIECES {
            let start = at(from);
            let step = match self.scale {
                Scale::Orthographic(_) => length,
                Scale::Perspective(_) => ((start - self.eye).dot(self.forward) * 0.5).max(1.0),
            };
            let to = match piece + 1 == LINE_PIECES {
                true => length,
                false => (from + step).min(length),
            };
            let end = at(to);
            let radius = self.line_radius(nearest_on_segment(self.eye, start, end));
            self.tube(start, end, radius, colour, look);
            if to >= length {
                break;
            }
            from = to;
        }
    }

    /// [`Self::line`] lying on the ground: raised by its own radius at each
    /// end, so the tube rests on the surface rather than half inside it.
    ///
    /// A tube centred on the ground is half buried, and where the height a
    /// tool samples differs from the drawn mesh by more than that, all of it
    /// is, and that part is drawn as a ghost: an outline faint in some places
    /// and solid in others.
    pub fn ground_line(&mut self, a: Vec3, b: Vec3, colour: Color, look: Look) {
        let (lift_a, lift_b) = (self.line_radius(a), self.line_radius(b));
        self.line(a + Vec3::Y * lift_a, b + Vec3::Y * lift_b, colour, look);
    }

    /// [`Self::line`] through every point in turn.
    pub fn line_strip(&mut self, points: impl IntoIterator<Item = Vec3>, colour: Color, look: Look) {
        let mut last: Option<Vec3> = None;
        for point in points {
            if let Some(from) = last {
                self.line(from, point, colour, look);
            }
            last = Some(point);
        }
    }

    /// A cone with its base on `base` and its point at `tip`.
    pub fn cone(&mut self, base: Vec3, tip: Vec3, radius: f32, colour: Color, look: Look) {
        let along = tip - base;
        let length = along.length();
        if length < 1e-4 {
            return;
        }
        let pose = Transform {
            translation: base,
            rotation: Quat::from_rotation_arc(Vec3::Y, along / length),
            scale: Vec3::new(radius, length, radius),
        };
        self.push(Shape::Cone, pose, colour, look);
    }

    /// A shaft of `radius` with a head three times as wide, a fifth of the
    /// length long.
    pub fn arrow(&mut self, from: Vec3, to: Vec3, radius: f32, colour: Color, look: Look) {
        let along = to - from;
        let length = along.length();
        if length < 1e-4 {
            return;
        }
        let head = (length * 0.22).min(radius * 9.0);
        let neck = to - along / length * head;
        self.tube(from, neck, radius, colour, look);
        self.cone(neck, to, radius * 3.0, colour, look);
    }

    /// A solid box: a unit cube posed by `pose`.
    pub fn cuboid(&mut self, pose: Transform, colour: Color, look: Look) {
        self.push(Shape::Cube, pose, colour, look);
    }

    /// The twelve edges of a unit cube posed by `pose`, as lines.
    pub fn box_edges(&mut self, pose: Transform, colour: Color, look: Look) {
        let corner = |i: usize| {
            pose.transform_point(Vec3::new(
                if i & 1 == 0 { -0.5 } else { 0.5 },
                if i & 2 == 0 { -0.5 } else { 0.5 },
                if i & 4 == 0 { -0.5 } else { 0.5 },
            ))
        };
        for (a, b) in CUBE_EDGES {
            self.line(corner(a), corner(b), colour, look);
        }
    }

    /// A circle of `segments` tubes about `normal`, of `thickness` radius.
    pub fn ring(
        &mut self,
        centre: Vec3,
        normal: Vec3,
        radius: f32,
        thickness: f32,
        colour: Color,
        look: Look,
    ) {
        let segments = 48;
        let turn = Quat::from_rotation_arc(Vec3::Y, normal.normalize_or(Vec3::Y));
        let at = |i: usize| {
            let angle = i as f32 / segments as f32 * std::f32::consts::TAU;
            centre + turn * Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius)
        };
        for i in 0..segments {
            self.tube(at(i), at(i + 1), thickness, colour, look);
        }
    }

    /// A flat ring lying on the ground, a [`Self::line`] thick.
    pub fn flat_ring(&mut self, centre: Vec3, radius: f32, colour: Color, look: Look) {
        let thickness = self.line_radius(centre);
        self.ring(centre, Vec3::Y, radius, thickness, colour, look);
    }
}

/// Pairs of [`Marks::box_edges`]' corner numbers: bit 0 is x, bit 1 y, bit 2 z.
const CUBE_EDGES: [(usize, usize); 12] = [
    (0, 1), (2, 3), (4, 5), (6, 7),
    (0, 2), (1, 3), (4, 6), (5, 7),
    (0, 4), (1, 5), (2, 6), (3, 7),
];

/// The point on `a`–`b` nearest `p`.
fn nearest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let along = b - a;
    let length = along.length_squared();
    if length < 1e-8 {
        return a;
    }
    a + along * ((p - a).dot(along) / length).clamp(0.0, 1.0)
}

/// One mark material: a colour, and which pass.
#[derive(Asset, AsBindGroup, TypePath, Clone)]
#[bind_group_data(MarkKey)]
pub struct MarkMaterial {
    /// `Color::srgb`'s numbers and the opacity.
    #[uniform(0)]
    colour: Vec4,
    ghost: bool,
}

/// What decides the pipeline: the pass, and whether the colour is opaque.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MarkKey {
    ghost: bool,
    translucent: bool,
}

impl From<&MarkMaterial> for MarkKey {
    fn from(material: &MarkMaterial) -> Self {
        MarkKey {
            ghost: material.ghost,
            translucent: material.colour.w < 1.0,
        }
    }
}

impl Material for MarkMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    /// Opaque marks write depth, so one mark hides another as solids do. A
    /// translucent mark or a ghost is blended and writes none.
    fn alpha_mode(&self) -> AlphaMode {
        match self.ghost || self.colour.w < 1.0 {
            true => AlphaMode::Blend,
            false => AlphaMode::Opaque,
        }
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let key = key.bind_group_data;
        // A translucent volume is seen from inside as well: a camera inside a
        // trigger's sphere still sees the sphere.
        if key.translucent && !key.ghost {
            descriptor.primitive.cull_mode = None;
        }
        if key.ghost {
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader_defs.push("GHOST".into());
            }
            // Depth is reversed: nearer is larger. `Less` passes where what
            // is already drawn is nearer than the mark, which is exactly the
            // part of it something hides.
            if let Some(depth) = descriptor.depth_stencil.as_mut() {
                depth.depth_write_enabled = Some(false);
                depth.depth_compare = Some(CompareFunction::Less);
            }
        }
        Ok(())
    }
}

/// Which pooled entities draw one shape in one material.
type PoolKey = (Shape, [u8; 4], bool);

/// The meshes, the materials and the entities [`show`] poses.
#[derive(Resource, Default)]
struct Pool {
    meshes: HashMap<Shape, Handle<Mesh>>,
    materials: HashMap<([u8; 4], bool), Handle<MarkMaterial>>,
    entities: HashMap<PoolKey, Vec<Entity>>,
    /// This frame's poses per key, kept to reuse the allocations.
    wanted: HashMap<PoolKey, Vec<Transform>>,
}

/// Marks a pooled entity.
#[derive(Component)]
struct Pooled;

pub struct MarksPlugin;

impl Plugin for MarksPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "marks.wgsl");
        app.add_plugins(MaterialPlugin::<MarkMaterial>::default())
            .init_resource::<Marks>()
            .init_resource::<Pool>()
            .add_systems(PreUpdate, follow_eye)
            .add_systems(PostUpdate, show.before(TransformSystems::Propagate));
    }
}

fn follow_eye(
    mut marks: ResMut<Marks>,
    camera: Query<(&GlobalTransform, &Camera, &Projection), With<WorldCamera>>,
) {
    let Ok((eye, camera, projection)) = camera.single() else {
        return;
    };
    marks.eye = eye.translation();
    marks.forward = eye.forward().into();
    let height = camera
        .physical_viewport_size()
        .map(|size| size.y.max(1) as f32)
        .unwrap_or(1000.0);
    marks.scale = match projection {
        Projection::Orthographic(ortho) => Scale::Orthographic(ortho.area.height() / height),
        Projection::Perspective(perspective) => {
            Scale::Perspective(2.0 * (perspective.fov * 0.5).tan() / height)
        }
        // A projection this crate does not make: keep the last.
        _ => marks.scale,
    };
}

/// Pose one pooled entity per mark and hide the rest, then empty the list.
///
/// Nothing is drawn while a playtest is running, whatever a tool asked for:
/// the tools that draw marks do not each check the playtest.
#[allow(clippy::too_many_arguments)]
fn show(
    mut marks: ResMut<Marks>,
    mut pool: ResMut<Pool>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<MarkMaterial>>,
    state: Res<crate::playtest::Playtest>,
    mut entities: Query<(&mut Transform, &mut Visibility), With<Pooled>>,
) {
    let pool = &mut *pool;
    for poses in pool.wanted.values_mut() {
        poses.clear();
    }
    if state.editing() {
        for mark in marks.list.drain(..) {
            pool.wanted
                .entry((mark.shape, mark.colour, false))
                .or_default()
                .push(mark.pose);
            if mark.look == Look::Ghosted {
                pool.wanted
                    .entry((mark.shape, ghost_of(mark.colour), true))
                    .or_default()
                    .push(mark.pose);
            }
        }
    }
    marks.list.clear();

    if pool.meshes.is_empty() {
        for shape in SHAPES {
            pool.meshes.insert(shape, meshes.add(mesh_of(shape)));
        }
    }

    // Every key ever used, so a key with nothing this frame hides its entities.
    let keys: Vec<PoolKey> = pool.wanted.keys().chain(pool.entities.keys()).copied().collect();
    for key in keys {
        let poses = pool.wanted.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        let have = pool.entities.entry(key).or_default();
        for (i, pose) in poses.iter().enumerate() {
            match have.get(i) {
                Some(&entity) => {
                    if let Ok((mut transform, mut visibility)) = entities.get_mut(entity) {
                        if *transform != *pose {
                            *transform = *pose;
                        }
                        if *visibility != Visibility::Visible {
                            *visibility = Visibility::Visible;
                        }
                    }
                }
                None => {
                    let (shape, colour, ghost) = key;
                    let material = pool
                        .materials
                        .entry((colour, ghost))
                        .or_insert_with(|| {
                            let [r, g, b, a] = colour.map(|c| c as f32 / 255.0);
                            materials.add(MarkMaterial {
                                colour: Vec4::new(r, g, b, a),
                                ghost,
                            })
                        })
                        .clone();
                    let entity = commands
                        .spawn((
                            Pooled,
                            Mesh3d(pool.meshes[&shape].clone()),
                            MeshMaterial3d(material),
                            *pose,
                            Visibility::Visible,
                            NotShadowCaster,
                            NotShadowReceiver,
                        ))
                        .id();
                    have.push(entity);
                }
            }
        }
        for &entity in have.iter().skip(poses.len()) {
            if let Ok((_, mut visibility)) = entities.get_mut(entity) {
                if *visibility != Visibility::Hidden {
                    *visibility = Visibility::Hidden;
                }
            }
        }
    }
}

/// The ghost pass's colour for a mark's colour.
fn ghost_of(colour: [u8; 4]) -> [u8; 4] {
    let alpha = (colour[3] as f32 / 255.0 * GHOST).max(GHOST_FLOOR);
    [colour[0], colour[1], colour[2], (alpha * 255.0).round() as u8]
}

fn mesh_of(shape: Shape) -> Mesh {
    match shape {
        Shape::Sphere => Sphere::new(1.0).mesh().uv(24, 16),
        Shape::Dome => dome(24, 8),
        Shape::Tube => Cylinder::new(1.0, 1.0).mesh().resolution(10).build(),
        Shape::Cone => Cone {
            radius: 1.0,
            height: 1.0,
        }
        .mesh()
        .anchor(ConeAnchor::Base)
        .resolution(20)
        .build(),
        Shape::Cube => Cuboid::new(1.0, 1.0, 1.0).mesh().build(),
    }
}

/// The upper half of a unit sphere and a disc closing it underneath.
fn dome(sectors: u32, stacks: u32) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for stack in 0..=stacks {
        let up = stack as f32 / stacks as f32 * std::f32::consts::FRAC_PI_2;
        for sector in 0..=sectors {
            let around = sector as f32 / sectors as f32 * std::f32::consts::TAU;
            let p = Vec3::new(up.cos() * around.cos(), up.sin(), up.cos() * around.sin());
            positions.push(p.to_array());
            normals.push(p.to_array());
        }
    }
    let row = sectors + 1;
    for stack in 0..stacks {
        for sector in 0..sectors {
            let a = stack * row + sector;
            let b = a + row;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    let middle = positions.len() as u32;
    positions.push([0.0, 0.0, 0.0]);
    normals.push([0.0, -1.0, 0.0]);
    for sector in 0..=sectors {
        let around = sector as f32 / sectors as f32 * std::f32::consts::TAU;
        positions.push([around.cos(), 0.0, around.sin()]);
        normals.push([0.0, -1.0, 0.0]);
    }
    for sector in 0..sectors {
        let a = middle + 1 + sector;
        indices.extend_from_slice(&[middle, a, a + 1]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_indices(Indices::U32(indices))
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ghost is fainter than its mark and never invisible.
    #[test]
    fn a_ghost_is_faint_but_visible() {
        assert_eq!(ghost_of([255, 0, 0, 255])[3], (GHOST * 255.0).round() as u8);
        assert_eq!(ghost_of([255, 0, 0, 20])[3], (GHOST_FLOOR * 255.0).round() as u8);
        assert_eq!(&ghost_of([10, 20, 30, 255])[..3], &[10, 20, 30]);
    }

    /// A tube is posed so its two ends land on the two points it was asked
    /// for, whichever way it points.
    #[test]
    fn a_tube_runs_between_its_ends() {
        let mut marks = Marks::default();
        let (a, b) = (Vec3::new(1.0, 2.0, 3.0), Vec3::new(-4.0, 0.5, 7.0));
        marks.tube(a, b, 0.1, Color::WHITE, Look::Solid);
        marks.tube(b, a, 0.1, Color::WHITE, Look::Solid);
        for mark in &marks.list {
            let bottom = mark.pose.transform_point(Vec3::new(0.0, -0.5, 0.0));
            let top = mark.pose.transform_point(Vec3::new(0.0, 0.5, 0.0));
            let ends = [bottom, top];
            assert!(ends.iter().any(|e| e.distance(a) < 1e-4), "{ends:?}");
            assert!(ends.iter().any(|e| e.distance(b) < 1e-4), "{ends:?}");
        }
    }

    fn looking_down_z(scale: Scale) -> Marks {
        Marks {
            forward: Vec3::NEG_Z,
            scale,
            ..Marks::default()
        }
    }

    /// In perspective a line is as thick as its depth needs, and in the map
    /// view's orthographic projection the same everywhere.
    #[test]
    fn a_line_is_a_few_pixels_wide_in_either_projection() {
        let near = nearest_on_segment(Vec3::ZERO, Vec3::new(-10.0, 5.0, 0.0), Vec3::new(10.0, 5.0, 0.0));
        assert!(near.distance(Vec3::new(0.0, 5.0, 0.0)) < 1e-4);

        let perspective = looking_down_z(Scale::Perspective(0.001));
        let far = perspective.line_radius(Vec3::new(0.0, 0.0, -1000.0));
        assert!((far - 1000.0 * 0.001 * LINE_PIXELS).abs() < 1e-4);
        // Behind the eye is a yard of depth, not none.
        assert!(perspective.line_radius(Vec3::new(0.0, 0.0, 5.0)) > 0.0);

        let ortho = looking_down_z(Scale::Orthographic(2.0));
        assert_eq!(ortho.line_radius(Vec3::new(0.0, 0.0, -3.0)), 2.0 * LINE_PIXELS);
        assert_eq!(ortho.line_radius(Vec3::new(50.0, 0.0, -900.0)), 2.0 * LINE_PIXELS);
    }

    /// A long line in perspective is cut into pieces that meet end to end and
    /// grow thicker with depth; in orthographic it is one piece.
    #[test]
    fn a_long_line_is_cut_into_pieces_that_thicken_with_depth() {
        let mut marks = looking_down_z(Scale::Perspective(0.001));
        let (a, b) = (Vec3::new(0.0, 0.0, -2.0), Vec3::new(0.0, 0.0, -2000.0));
        marks.line(a, b, Color::WHITE, Look::Solid);
        assert!(marks.list.len() > 4 && marks.list.len() <= LINE_PIECES, "{}", marks.list.len());
        let ends: Vec<(Vec3, Vec3)> = marks
            .list
            .iter()
            .map(|m| {
                let p = [-0.5, 0.5].map(|y| m.pose.transform_point(Vec3::new(0.0, y, 0.0)));
                // Ordered from the eye outward.
                match p[0].z > p[1].z {
                    true => (p[0], p[1]),
                    false => (p[1], p[0]),
                }
            })
            .collect();
        assert!(ends[0].0.distance(a) < 1e-3);
        assert!(ends.last().unwrap().1.distance(b) < 1e-2);
        for pair in ends.windows(2) {
            assert!(pair[0].1.distance(pair[1].0) < 1e-2, "{pair:?}");
        }
        let radii: Vec<f32> = marks.list.iter().map(|m| m.pose.scale.x).collect();
        assert!(radii.windows(2).all(|r| r[1] >= r[0]), "{radii:?}");

        let mut ortho = looking_down_z(Scale::Orthographic(1.0));
        ortho.line(a, b, Color::WHITE, Look::Solid);
        assert_eq!(ortho.list.len(), 1);
    }
}
