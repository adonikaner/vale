//! The editor's marks in the world, drawn as meshes rather than as lines.
//!
//! A tool describes what it wants this frame — a dome over a waypoint, an
//! arrow on a handle, a dashed ring on the ground — by calling [`Marks`], the
//! same way it would call `Gizmos`. [`show`] then poses one pooled entity per
//! shape, merges every line of one colour and one pass into one mesh, and
//! hides the rest. The lists are emptied every frame, so a tool that stops
//! drawing leaves nothing behind.
//!
//! ## Why the lines are merged and the shapes are not
//!
//! Lines were tubes too, one entity per piece, and a flight-path map or a
//! selected building's boxes made several thousand of them, most translucent
//! or ghosted and therefore sorted and drawn one at a time: the flight-path
//! tool ran at 51 ms a frame. Merged, the lines of one colour and one pass are
//! one draw, and the tool ran at 17 ms. The merged mesh is rebuilt only on a
//! frame whose lines differ from the last frame's. The comparison is a sum of
//! one hash per line, so it does not depend on the order the tools' systems
//! ran in, which changes from frame to frame.
//!
//! The shapes stay one entity each. A tool sizes most of them by the distance
//! to the camera, so a moving camera changes every one of them every frame,
//! and rebuilding a merged mesh of 2,000 domes took 180 ms in the editor's
//! unoptimised build, against moving 2,000 entities, which costs little. A
//! shape's entities are not touched on a frame whose poses for that shape and
//! colour are the last frame's, by the same sum of hashes.
//!
//! ## Lines are widened by the shader
//!
//! A line is a thin tube about the segment between its two ends, and the
//! vertex shader pushes each vertex out from the segment by its width in
//! pixels at that vertex's depth (`marks.wgsl`). Its geometry therefore does
//! not depend on the camera, and a moving camera rebuilds nothing for the
//! flight routes, the box edges and the ground outlines. Each end of a long
//! line is as wide as its own depth needs, so a line that runs away from the
//! camera keeps one width on the screen along its length.
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
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{ConeAnchor, Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::platform::collections::{HashMap, HashSet};
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

/// The smallest a marker is drawn, as a radius in pixels; see
/// [`Marks::visible_radius`].
const MARKER_PIXELS: f32 = 4.0;

/// How many sides a line's tube has. The shader keeps the tube a few pixels
/// across, where six sides look round.
const LINE_SIDES: usize = 6;

/// How many lines [`Marks::flat_ring`] and [`Marks::wide_ring`] draw a circle
/// with.
const RING_SEGMENTS: usize = 48;

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

/// One line: its ends, its radius as a multiple of [`LINE_PIXELS`], and
/// whether it rests on the ground (raised by its own radius at each end).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Line {
    a: Vec3,
    b: Vec3,
    width: f32,
    lift: bool,
}

#[derive(Debug, Clone, Copy)]
struct LineMark {
    line: Line,
    colour: [u8; 4],
    look: Look,
}

/// What the tools want drawn this frame. See the module comment.
#[derive(Resource, Default)]
pub struct Marks {
    list: Vec<Mark>,
    lines: Vec<LineMark>,
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

    /// The radius a line at `at` is drawn with: [`LINE_PIXELS`]. For a shape
    /// drawn to match the lines, such as an arrow's shaft.
    pub fn line_radius(&self, at: Vec3) -> f32 {
        self.pixel(at) * LINE_PIXELS
    }

    /// The radius a marker of `radius` yards at `at` is drawn with: its own,
    /// or [`MARKER_PIXELS`] pixels where that is larger.
    ///
    /// A marker is the size of the thing it marks, so near ones sit in the
    /// world at their true size rather than growing with distance; the floor
    /// keeps a far one as a dot rather than letting it vanish, which matters
    /// for flight-path nodes seen from across a zone and in the map view.
    pub fn visible_radius(&self, at: Vec3, radius: f32) -> f32 {
        radius.max(self.pixel(at) * MARKER_PIXELS)
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

    fn push_line(&mut self, line: Line, colour: Color, look: Look) {
        if line.a.distance_squared(line.b) < 1e-8 {
            return;
        }
        self.lines.push(LineMark {
            line,
            colour: colour.to_srgba().to_u8_array(),
            look,
        });
    }

    /// A line [`LINE_PIXELS`] pixels in radius wherever it is; see the module
    /// comment.
    pub fn line(&mut self, a: Vec3, b: Vec3, colour: Color, look: Look) {
        self.wide_line(a, b, 1.0, colour, look);
    }

    /// [`Self::line`] with `width` times its radius.
    pub fn wide_line(&mut self, a: Vec3, b: Vec3, width: f32, colour: Color, look: Look) {
        self.push_line(Line { a, b, width, lift: false }, colour, look);
    }

    /// [`Self::line`] lying on the ground: raised by its own radius at each
    /// end, so the tube rests on the surface rather than half inside it.
    ///
    /// A tube centred on the ground is half buried, and where the height a
    /// tool samples differs from the drawn mesh by more than that, all of it
    /// is, and that part is drawn as a ghost: an outline faint in some places
    /// and solid in others.
    pub fn ground_line(&mut self, a: Vec3, b: Vec3, colour: Color, look: Look) {
        self.wide_ground_line(a, b, 1.0, colour, look);
    }

    /// [`Self::ground_line`] with `width` times its radius.
    pub fn wide_ground_line(&mut self, a: Vec3, b: Vec3, width: f32, colour: Color, look: Look) {
        self.push_line(Line { a, b, width, lift: true }, colour, look);
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

    /// A circle of [`Self::wide_line`]s about `normal`.
    pub fn wide_ring(
        &mut self,
        centre: Vec3,
        normal: Vec3,
        radius: f32,
        width: f32,
        colour: Color,
        look: Look,
    ) {
        let turn = Quat::from_rotation_arc(Vec3::Y, normal.normalize_or(Vec3::Y));
        let at = |i: usize| {
            let angle = i as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
            centre + turn * Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius)
        };
        for i in 0..RING_SEGMENTS {
            self.wide_line(at(i), at(i + 1), width, colour, look);
        }
    }

    /// A flat circle of [`Self::line`]s about a vertical axis.
    pub fn flat_ring(&mut self, centre: Vec3, radius: f32, colour: Color, look: Look) {
        self.wide_ring(centre, Vec3::Y, radius, 1.0, colour, look);
    }
}

/// Pairs of [`Marks::box_edges`]' corner numbers: bit 0 is x, bit 1 y, bit 2 z.
const CUBE_EDGES: [(usize, usize); 12] = [
    (0, 1), (2, 3), (4, 5), (6, 7),
    (0, 2), (1, 3), (4, 6), (5, 7),
    (0, 4), (1, 5), (2, 6), (3, 7),
];

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
    /// The shader's own vertex stage, which widens a line; see the module
    /// comment.
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }

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

/// How many frames a key keeps entities it has not needed, before [`show`]
/// despawns them. A tool chosen again within this keeps its entities rather
/// than spawning them afresh.
const KEEP_FRAMES: u32 = 300;

/// One key's pooled entities: the first `shown` are visible, the rest hidden.
#[derive(Default)]
struct Slots {
    entities: Vec<Entity>,
    shown: usize,
    /// The [`signature`] of the poses the entities were last given.
    posed: (u64, usize),
    /// The most marks the key has drawn in one frame since the count of
    /// `frames` began.
    most: usize,
    frames: u32,
}

/// One colour and one pass: one material, and one merged mesh of lines.
type MaterialKey = ([u8; 4], bool);

/// The entity that draws one [`MaterialKey`]'s lines as one mesh, and the
/// [`signature`] of the lines it was last built from.
struct Lines {
    entity: Entity,
    mesh: Handle<Mesh>,
    built: (u64, usize),
}

/// The meshes, the materials and the entities [`show`] keeps.
#[derive(Resource, Default)]
struct Pool {
    meshes: HashMap<Shape, Handle<Mesh>>,
    materials: HashMap<MaterialKey, Handle<MarkMaterial>>,
    entities: HashMap<PoolKey, Slots>,
    lines: HashMap<MaterialKey, Lines>,
    /// This frame's poses and lines per key, kept to reuse the allocations.
    wanted: HashMap<PoolKey, Vec<Transform>>,
    wanted_lines: HashMap<MaterialKey, Vec<Line>>,
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

/// Pose one pooled entity per shape, merge the lines into one mesh per colour
/// and pass, hide the rest, then empty the lists.
///
/// Nothing is drawn while a playtest is running, whatever a tool asked for:
/// the tools that draw marks do not each check the playtest. The pool is
/// despawned when one starts, so a playtest keeps no editor entities and this
/// does nothing until the editor is back.
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
    for lines in pool.wanted_lines.values_mut() {
        lines.clear();
    }
    if !state.editing() {
        marks.list.clear();
        marks.lines.clear();
        for (_, slots) in pool.entities.drain() {
            for entity in slots.entities {
                commands.entity(entity).despawn();
            }
        }
        for (_, lines) in pool.lines.drain() {
            commands.entity(lines.entity).despawn();
        }
        return;
    }
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
    for mark in marks.lines.drain(..) {
        pool.wanted_lines.entry((mark.colour, false)).or_default().push(mark.line);
        if mark.look == Look::Ghosted {
            pool.wanted_lines
                .entry((ghost_of(mark.colour), true))
                .or_default()
                .push(mark.line);
        }
    }

    if pool.meshes.is_empty() {
        for shape in SHAPES {
            pool.meshes.insert(shape, meshes.add(mesh_of(shape)));
        }
    }
    let mut material = |key: MaterialKey| {
        pool.materials
            .entry(key)
            .or_insert_with(|| {
                let (colour, ghost) = key;
                let [r, g, b, a] = colour.map(|c| c as f32 / 255.0);
                materials.add(MarkMaterial {
                    colour: Vec4::new(r, g, b, a),
                    ghost,
                })
            })
            .clone()
    };

    // Every key with marks this frame, and every key that has entities, so a
    // key with nothing this frame hides them.
    let keys: Vec<PoolKey> = pool
        .wanted
        .iter()
        .filter(|(_, poses)| !poses.is_empty())
        .map(|(key, _)| *key)
        .chain(
            pool.entities
                .keys()
                .filter(|key| pool.wanted.get(*key).is_none_or(Vec::is_empty))
                .copied(),
        )
        .collect();
    for key in keys {
        let poses = pool.wanted.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        let slots = pool.entities.entry(key).or_default();
        // The same poses as last frame, in whatever order, leave the entities
        // as they are: one shape in one material, they are interchangeable.
        let posed = signature(poses.iter().map(pose_hash));
        let unchanged = posed == slots.posed && slots.shown == poses.len();
        slots.posed = posed;
        for (i, pose) in poses.iter().enumerate().filter(|_| !unchanged) {
            match slots.entities.get(i) {
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
                    let entity = commands
                        .spawn((
                            Pooled,
                            Mesh3d(pool.meshes[&shape].clone()),
                            MeshMaterial3d(material((colour, ghost))),
                            *pose,
                            Visibility::Visible,
                            NotShadowCaster,
                            NotShadowReceiver,
                        ))
                        .id();
                    slots.entities.push(entity);
                }
            }
        }
        // Only the entities shown last frame and not wanted now. The rest
        // were hidden on an earlier frame, and a tool that once drew
        // thousands of lines left that many: looking each one up every frame
        // cost every tool chosen after it.
        let shown = slots.shown.min(slots.entities.len());
        for &entity in slots.entities.iter().take(shown).skip(poses.len()) {
            if let Ok((_, mut visibility)) = entities.get_mut(entity) {
                if *visibility != Visibility::Hidden {
                    *visibility = Visibility::Hidden;
                }
            }
        }
        slots.shown = poses.len();

        // Despawn what the key has not needed for `KEEP_FRAMES`, so the
        // world does not keep every entity the busiest tool ever asked for.
        slots.most = slots.most.max(poses.len());
        slots.frames += 1;
        if slots.frames >= KEEP_FRAMES {
            for entity in slots.entities.drain(slots.most..) {
                commands.entity(entity).despawn();
            }
            slots.most = poses.len();
            slots.frames = 0;
        }
        if slots.entities.is_empty() {
            pool.entities.remove(&key);
        }
    }

    let keys: Vec<MaterialKey> =
        pool.wanted_lines.keys().chain(pool.lines.keys()).copied().collect::<HashSet<_>>().into_iter().collect();
    for key in keys {
        let wanted = pool.wanted_lines.entry(key).or_default();
        if wanted.is_empty() {
            if let Some(lines) = pool.lines.get(&key) {
                if let Ok((_, mut visibility)) = entities.get_mut(lines.entity) {
                    if *visibility != Visibility::Hidden {
                        *visibility = Visibility::Hidden;
                    }
                }
            }
            continue;
        }
        let lines = pool.lines.entry(key).or_insert_with(|| {
            let mesh = meshes.add(empty_mesh());
            let entity = commands
                .spawn((
                    Pooled,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material(key)),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    NotShadowCaster,
                    NotShadowReceiver,
                    // The merged mesh spans everything a tool draws, and its
                    // bounds change with every rebuild.
                    NoFrustumCulling,
                ))
                .id();
            Lines {
                entity,
                mesh,
                built: (0, 0),
            }
        });
        let now = signature(wanted.iter().map(line_hash));
        if lines.built != now {
            let centre = wanted.iter().map(|line| (line.a + line.b) * 0.5).sum::<Vec3>()
                / wanted.len() as f32;
            if meshes.insert(lines.mesh.id(), merge_lines(wanted, centre)).is_ok() {
                // A replaced mesh is queued again only when its `Mesh3d` is
                // inserted again, as the client's interface painter does.
                commands.entity(lines.entity).insert(Mesh3d(lines.mesh.clone()));
            }
            // At the lines' centre, so a translucent mesh sorts by where its
            // lines are rather than by the world's origin. A new entity is
            // spawned by a command and is not in the query until the next
            // frame, so its transform goes through a command as well.
            match entities.get_mut(lines.entity) {
                Ok((mut transform, _)) => *transform = Transform::from_translation(centre),
                Err(_) => {
                    commands
                        .entity(lines.entity)
                        .insert(Transform::from_translation(centre));
                }
            }
            lines.built = now;
        }
        match entities.get_mut(lines.entity) {
            Ok((_, mut visibility)) => {
                if *visibility != Visibility::Visible {
                    *visibility = Visibility::Visible;
                }
            }
            Err(_) => {
                commands.entity(lines.entity).insert(Visibility::Visible);
            }
        }
    }
}

/// A summary of a list of marks that does not depend on their order: the
/// wrapping sum of one hash per mark, and the count. Two lists with the same
/// signature are taken to be the same marks.
fn signature(hashes: impl Iterator<Item = u64>) -> (u64, usize) {
    hashes.fold((0u64, 0usize), |(sum, count), hash| (sum.wrapping_add(hash), count + 1))
}

/// A hash over the bits of a run of numbers.
fn hash_of(words: impl IntoIterator<Item = f32>) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for word in words {
        hash = (hash ^ u64::from(word.to_bits())).wrapping_mul(0x0100_0000_01b3);
        hash ^= hash >> 29;
    }
    hash
}

/// One pose's hash, for [`signature`].
fn pose_hash(pose: &Transform) -> u64 {
    hash_of(
        pose.translation
            .to_array()
            .into_iter()
            .chain(pose.rotation.to_array())
            .chain(pose.scale.to_array()),
    )
}

/// One line's hash, for [`signature`]. A raised line's width is fed negated,
/// so raising a line changes its hash.
fn line_hash(line: &Line) -> u64 {
    let width = match line.lift {
        true => -line.width,
        false => line.width,
    };
    hash_of(line.a.to_array().into_iter().chain(line.b.to_array()).chain([width]))
}

/// One mesh of every line in `list`, relative to `centre`.
///
/// Each line is a [`LINE_SIDES`]-sided tube of no thickness: every vertex
/// lies on the segment, at one of the two ends, and the tangent slot carries
/// the direction the shader pushes it out in (`xyz`) and the line's width,
/// negated for a line raised onto the ground (`w`). The normal is the same
/// direction. The tube has no caps, which at a few pixels across cannot be
/// seen.
fn merge_lines(list: &[Line], centre: Vec3) -> Mesh {
    let count = list.len() * LINE_SIDES * 2;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(count);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(count);
    let mut offsets: Vec<[f32; 4]> = Vec::with_capacity(count);
    let mut triangles: Vec<u32> = Vec::with_capacity(list.len() * LINE_SIDES * 6);
    let around: [(f32, f32); LINE_SIDES] = std::array::from_fn(|side| {
        let angle = side as f32 / LINE_SIDES as f32 * std::f32::consts::TAU;
        (angle.cos(), angle.sin())
    });
    for line in list {
        let along = (line.b - line.a).normalize_or(Vec3::Y);
        // `u`, `v` and `along` are right-handed, so the triangles below wind
        // counter-clockwise seen from outside the tube.
        let u = along.any_orthonormal_vector();
        let v = along.cross(u);
        let w = match line.lift {
            true => -line.width,
            false => line.width,
        };
        let base = positions.len() as u32;
        for end in [line.a - centre, line.b - centre] {
            for &(cos, sin) in &around {
                let out = u * cos + v * sin;
                positions.push(end.to_array());
                normals.push(out.to_array());
                offsets.push([out.x, out.y, out.z, w]);
            }
        }
        let sides = LINE_SIDES as u32;
        for side in 0..sides {
            let next = (side + 1) % sides;
            let (a0, a1) = (base + side, base + next);
            let (b0, b1) = (base + sides + side, base + sides + next);
            triangles.extend_from_slice(&[a0, a1, b0, a1, b1, b0]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_indices(Indices::U32(triangles))
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, offsets)
}

/// A mesh with no triangles, for an entity before its first lines.
fn empty_mesh() -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_indices(Indices::U32(Vec::new()))
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new())
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

    /// In perspective a mark sized to match the lines is as large as its depth
    /// needs, and in the map view's orthographic projection the same
    /// everywhere.
    #[test]
    fn a_line_radius_is_a_few_pixels_in_either_projection() {
        let perspective = looking_down_z(Scale::Perspective(0.001));
        let far = perspective.line_radius(Vec3::new(0.0, 0.0, -1000.0));
        assert!((far - 1000.0 * 0.001 * LINE_PIXELS).abs() < 1e-4);
        // Behind the eye is a yard of depth, not none.
        assert!(perspective.line_radius(Vec3::new(0.0, 0.0, 5.0)) > 0.0);

        let ortho = looking_down_z(Scale::Orthographic(2.0));
        assert_eq!(ortho.line_radius(Vec3::new(0.0, 0.0, -3.0)), 2.0 * LINE_PIXELS);
        assert_eq!(ortho.line_radius(Vec3::new(50.0, 0.0, -900.0)), 2.0 * LINE_PIXELS);
    }

    /// A line is one mark between its two ends, whatever the camera: the
    /// shader widens it, so its geometry does not move with the camera.
    #[test]
    fn a_line_is_one_mark_that_does_not_depend_on_the_camera() {
        let (a, b) = (Vec3::new(0.0, 0.0, -2.0), Vec3::new(0.0, 0.0, -2000.0));
        let mut near = looking_down_z(Scale::Perspective(0.001));
        near.line(a, b, Color::WHITE, Look::Solid);
        let mut far = looking_down_z(Scale::Orthographic(5.0));
        far.eye = Vec3::new(0.0, 900.0, 0.0);
        far.line(a, b, Color::WHITE, Look::Solid);
        assert_eq!(near.lines.len(), 1);
        assert!(near.list.is_empty());
        assert_eq!(near.lines[0].line, far.lines[0].line);
        assert_eq!(near.lines[0].line, Line { a, b, width: 1.0, lift: false });
        near.wide_ground_line(a, b, 1.5, Color::WHITE, Look::Solid);
        assert_eq!(near.lines[1].line, Line { a, b, width: 1.5, lift: true });
        // A line of no length is not drawn.
        near.line(a, a, Color::WHITE, Look::Solid);
        assert_eq!(near.lines.len(), 2);
    }

    /// A line's mesh has its vertices on the segment, pushes them out at right
    /// angles to it, carries its width and whether it is raised, and winds its
    /// triangles outward.
    #[test]
    fn a_line_mesh_lies_on_its_segment_and_faces_outward() {
        let (a, b) = (Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 6.0, 3.0));
        let mesh = merge_lines(&[Line { a, b, width: 1.5, lift: true }], Vec3::ZERO);
        let positions: Vec<Vec3> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) => v.iter().map(|p| Vec3::from_array(*p)).collect(),
            _ => panic!("positions"),
        };
        let offsets: Vec<[f32; 4]> = match mesh.attribute(Mesh::ATTRIBUTE_TANGENT) {
            Some(bevy::mesh::VertexAttributeValues::Float32x4(v)) => v.clone(),
            _ => panic!("offsets"),
        };
        assert_eq!(positions.len(), LINE_SIDES * 2);
        let along = (b - a).normalize();
        for (p, o) in positions.iter().zip(&offsets) {
            assert!(p.distance(a) < 1e-5 || p.distance(b) < 1e-5);
            let out = Vec3::new(o[0], o[1], o[2]);
            assert!((out.length() - 1.0).abs() < 1e-5);
            assert!(out.dot(along).abs() < 1e-5);
            assert_eq!(o[3], -1.5, "a raised line's width is negated");
        }
        // Each triangle's normal, from its winding, points the way its
        // vertices are pushed.
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("indices");
        };
        for triangle in indices.chunks(3) {
            let [i, j, k] = [triangle[0], triangle[1], triangle[2]].map(|n| n as usize);
            let out = |n: usize| Vec3::new(offsets[n][0], offsets[n][1], offsets[n][2]);
            // The vertices pushed out by one unit, as the shader would.
            let (p, q, r) = (positions[i] + out(i), positions[j] + out(j), positions[k] + out(k));
            let face = (q - p).cross(r - p);
            assert!(face.dot(out(i) + out(j) + out(k)) > 0.0, "{triangle:?}");
        }
    }

    /// The change test ignores the order the marks arrived in and notices a
    /// moved, widened or raised line, and a moved pose.
    #[test]
    fn a_signature_ignores_order_and_notices_a_change() {
        let first = Line { a: Vec3::ZERO, b: Vec3::X, width: 1.0, lift: false };
        let second = Line { a: Vec3::new(1.0, 2.0, 3.0), b: Vec3::Y, width: 1.0, lift: false };
        let of = |lines: &[Line]| signature(lines.iter().map(line_hash));
        assert_eq!(of(&[first, second]), of(&[second, first]));
        let moved = Line { a: Vec3::new(1.0, 2.0, 3.5), ..second };
        let wider = Line { width: 1.6, ..second };
        let raised = Line { lift: true, ..second };
        for changed in [moved, wider, raised] {
            assert_ne!(of(&[first, second]), of(&[first, changed]));
        }
        assert_ne!(of(&[first]), of(&[first, first]));

        let poses = [Transform::from_xyz(1.0, 2.0, 3.0), Transform::from_xyz(4.0, 5.0, 6.0)];
        let of = |poses: &[Transform]| signature(poses.iter().map(pose_hash));
        assert_eq!(of(&poses), of(&[poses[1], poses[0]]));
        assert_ne!(of(&poses), of(&[poses[0], Transform::from_xyz(4.0, 5.0, 6.5)]));
    }
}
