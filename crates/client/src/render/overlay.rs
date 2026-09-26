//! What is drawn about the world rather than in it: one switch per
//! visualisation, and nothing else.
//!
//! [`crate::render::tuning`] is this module's counterpart, and the two divide
//! the debug panel's picture between them. That module is a set of
//! subtractions: fifteen layers of the world, each of which can be taken
//! away. This module is a set of additions: six things that are not in the
//! world at all and are drawn over it to answer a question the finished frame
//! cannot.
//!
//! ## Why an addition is a different instrument from a subtraction
//!
//! A subtraction answers which layer a thing belongs to: turn the doodads off
//! and the dark shape on the hill is a building. It cannot answer anything
//! about a surface that is drawn correctly and is nonetheless wrong: a mesh
//! with a ten-thousand-triangle lawn behind one blade of grass, a hull that
//! does not match the building it is inside, an entity facing east while its
//! body points north. Those failures render plausibly, and each of them is
//! visible as soon as something is drawn on top of it.
//!
//! ## Which overlays are Bevy's and which are this project's
//!
//! Wireframe, bounding boxes, frusta and skinned-mesh bounds are engine
//! features this client had not turned on. Each costs a resource write when a
//! checkbox moves and nothing the rest of the time. Wireframe depends on the
//! GPU: it needs `POLYGON_MODE_LINE` and `IMMEDIATES`, which every desktop
//! Vulkan/DX12/Metal adapter has and WebGL has not. `WireframePlugin` declines
//! to install itself with a warning rather than failing, so the checkbox is
//! disabled and says why rather than being a switch that does nothing.
//! Nothing is requested of the adapter for it: `WgpuSettings::priority`
//! defaults to `Functionality`, which enables every feature the adapter has,
//! so the pair is already present on the backend this client asks for.
//!
//! The other two are this project's own, and both exist for a report:
//!
//! * The collision hulls. "I fell through the floor" and "the building did
//!   not draw" are different failures that look identical, and the HUD's
//!   `N solid, N solid triangles` line says only that hulls exist somewhere.
//!   Drawing the hulls under the character's feet says whether the one it
//!   stands on is the shape of the thing it stands on. Walls are coloured
//!   apart from floors, because
//!   [`vale_assets::Collider::triangle_is_wall`] is the rule the mover
//!   walks by.
//! * The facing spike. A strafing body is turned off its aim
//!   (`world::facing`), a mount's landing is chosen off its gait, and in both
//!   the drawn model and the number driving it can disagree silently. One
//!   line out of each unit's nose makes the disagreement visible.
//!
//! ## The collision overlay is bounded
//!
//! The collision overlay is a walk over hulls and their triangles, and there
//! are ~3,800 hulls in a loaded world, some of which are a cathedral. It asks
//! [`vale_assets::CollisionWorld::hulls_near`] for a radius around
//! [`WorldFocus`] and nothing else, and it caps the triangles it draws in one
//! frame. The session writes the focus from the character's position every
//! frame, and a host with no session writes it itself. A debug overlay that
//! halves the frame rate is one nobody leaves on long enough to see the thing
//! it was drawn for.
//!
//! ## What it costs when it is off
//!
//! 0.104 ms a frame at 30,000 meshes, measured. `WireframePlugin` is
//! registered unconditionally (a plugin cannot be added later), and the one
//! part of it that is not free with the switch off is
//! `check_wireframe_entities_needing_specialization`, an
//! `Or<(Changed<Mesh3d>, AssetChanged<Mesh3d>, …)>` sweep over every mesh in
//! the world every frame. Timed against the same 30,000-entity world with an
//! empty schedule: 0.116 ms against 0.013 ms. Everything else it adds queries
//! on `Mesh3dWireframe`, which no entity carries until the switch is on, so
//! those archetypes are empty and cost nothing.
//!
//! Against this client's ~14 ms frame that is 0.7%, inside the ±2.5 ms
//! run-to-run wander every other measurement here is quoted with. It is
//! recorded because it is the one cost in this module that is paid whether or
//! not anybody is looking, and because the number is proportional to the mesh
//! count: if this renderer's world grows by an order of magnitude, this is
//! the line to re-measure.
//!
//! `VALE_NO_WIREFRAME=1` removes it, on the same terms as
//! `VALE_NO_PARTICLES` and `VALE_NO_INTERFACE`: two runs differing in
//! one variable is how this project prices a pass, and a cost paid with the
//! switch off has to stay subtractable. The checkbox then says why it is
//! disabled, as it does for a GPU that cannot draw lines.
//!
//! ## The module compiles out
//!
//! The whole module is behind the `diagnostics` feature, with the window that
//! drives it. [`crate::render::tuning::WorldTuning`] is a dozen bools read by
//! systems that are already running and is not gated; every system here
//! exists only to draw an instrument, and `WireframePlugin` brings a render
//! phase, a material and a specialisation cache with it. See
//! [`crate::ui::debug`].

use bevy::pbr::wireframe::{WireframeConfig, WireframePlugin};
use super::focus::WorldFocus;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;

use crate::render::axes;
use crate::world::session::{Solids, WorldEntity};

/// What is drawn over the world. Every field defaults to off, so an
/// untouched client draws the game and nothing else.
///
/// `Clone` and `PartialEq` are derived for the reason
/// [`crate::render::tuning::WorldTuning`] derives them: `ResMut`'s `DerefMut`
/// marks a resource changed whether or not the value moved, and [`apply`]
/// below is guarded by `is_changed()`. The panel edits a copy and compares.
#[derive(Resource, Clone, PartialEq, Debug)]
pub struct DebugOverlay {
    /// Every mesh in the world, drawn again as its edges on top of itself.
    ///
    /// It answers the one question no count answers: how much geometry is
    /// there? A terrain chunk, an 830k-triangle city and a merged lawn all
    /// report one mesh and one draw call, and only this says which of them is
    /// dense.
    pub wireframe: bool,
    /// Its line width in screen pixels. 1.0 is a hairline, which disappears on
    /// a busy frame; 2.0 survives a screenshot.
    pub wireframe_width: f32,
    /// Every `Aabb` in the scene, as a box. Bevy's own
    /// `AabbGizmoConfigGroup::draw_all`.
    ///
    /// This is what the culler sees, which is not always the mesh: a bad or
    /// stale `Aabb` is a model that vanishes when viewed from one side, and
    /// nothing else in the frame would say so.
    pub aabbs: bool,
    /// Every `Frustum` in the scene, as its six planes.
    ///
    /// Three cameras in this client are not the world's (the glue scene, the
    /// portraits and the present blit), and nothing else says which camera a
    /// view is on.
    pub frustums: bool,
    /// The bounds each skinned mesh's joints contribute: the animated half of
    /// the same question. A pose that reaches outside them is a character
    /// that pops out of existence mid-swing.
    pub skinned_bounds: bool,
    /// The solid triangles standing near the world's focus, floors in one
    /// colour and walls in the other. See the module note.
    pub collision: bool,
    /// What each diagnostic box stands in for, written over it: the display
    /// id, the path and why it did not draw. See [`crate::ui::boxes`].
    ///
    /// The seventh visualisation and the only one that is not `Gizmos`: gizmos
    /// have no text, so it is egui at a projected world position. It is on
    /// this tab because that is where a person looks for it.
    pub boxes: bool,
    /// How far around the focus to draw them, in yards.
    pub collision_radius: f32,
    /// The most triangles one frame draws before the overlay stops and
    /// reports itself capped. Defaults to [`MAX_COLLISION_TRIANGLES`]. A host
    /// that raises the radius raises this with it, since the cap is what
    /// keeps a wide radius from drawing half a building.
    pub collision_triangles: usize,
    /// A spike out of every unit's nose, up its own facing, plus a stalk at its
    /// origin.
    pub facing: bool,
}

impl Default for DebugOverlay {
    fn default() -> Self {
        DebugOverlay {
            wireframe: false,
            wireframe_width: 1.0,
            aabbs: false,
            frustums: false,
            skinned_bounds: false,
            collision: false,
            boxes: false,
            collision_radius: 25.0,
            collision_triangles: MAX_COLLISION_TRIANGLES,
            facing: false,
        }
    }
}

/// One row of [`OVERLAYS`]: what it is called, what question it answers, and
/// the flag it sets. A named type because the triple is otherwise the widest
/// thing in the file and clippy reports that it reads badly inline.
pub type Overlay = (&'static str, &'static str, fn(&mut DebugOverlay) -> &mut bool);

/// Every overlay, as `(label, what it answers, accessor)`, for the panel that
/// draws them.
///
/// A list rather than six hand-written checkbox rows, for the reason
/// `render::tuning::SWITCHES` is one: an overlay added here and not
/// to the panel is an overlay nobody can reach, and the test below stops two
/// entries sharing one field.
pub const OVERLAYS: [Overlay; 7] = [
    (
        "error boxes",
        "what each grey box should have been, and why it is not drawn",
        |o| &mut o.boxes,
    ),
    (
        "wireframe",
        "every mesh's edges, which shows how much geometry is there",
        |o| &mut o.wireframe,
    ),
    (
        "bounding boxes",
        "the bounds the culler tests, which are not always where the mesh is",
        |o| &mut o.aabbs,
    ),
    (
        "camera frusta",
        "the four cameras in this client, and which one a view is on",
        |o| &mut o.frustums,
    ),
    (
        "skinned bounds",
        "the bounds of a skinned mesh: how far a pose is allowed to reach",
        |o| &mut o.skinned_bounds,
    ),
    (
        "collision",
        "the solid triangles underfoot: floors green, walls red",
        |o| &mut o.collision,
    ),
    (
        "unit facing",
        "a spike along each unit's heading, which strafing turns away from its aim",
        |o| &mut o.facing,
    ),
];

impl DebugOverlay {
    /// Everything off except the overlays `list` names: `--overlay wireframe`.
    ///
    /// This is the scripted form of the six checkboxes, and it exists for the
    /// reason [`crate::render::tuning::WorldTuning::without`] does one module
    /// over: looking at the screen costs a login and 25 seconds, and an
    /// instrument that can only be turned on by a person at the keyboard never
    /// appears beside a number in a log. It is also the only way a scripted
    /// `--shot` can photograph a wireframe.
    ///
    /// It is spelled as the on list rather than the off list, the opposite of
    /// `--without`, for the reason that one is spelled the other way round:
    /// these all default to off, so `--overlay collision` must not silently
    /// turn the other five on.
    ///
    /// A name that matches nothing is warned about and ignored. A misspelled
    /// overlay that quietly photographs the plain world is the one failure
    /// this must not have. Spaces are optional, so `bounding boxes` and
    /// `boundingboxes` both reach the same switch.
    pub fn with(list: &str) -> DebugOverlay {
        let mut overlay = DebugOverlay::default();
        let key = |s: &str| s.trim().to_ascii_lowercase().replace(' ', "");
        for name in list.split(',').filter(|s| !s.trim().is_empty()) {
            match OVERLAYS.iter().find(|(label, _, _)| key(label) == key(name)) {
                Some((_, _, field)) => *field(&mut overlay) = true,
                None => warn!(
                    "--overlay: no overlay called {name:?}; the six are {}",
                    OVERLAYS.map(|(label, _, _)| label).join(", ")
                ),
            }
        }
        overlay
    }
}

/// The default for [`DebugOverlay::collision_triangles`]: the most triangles
/// the collision overlay draws in one frame.
///
/// A cathedral's hull is tens of thousands of triangles and the overlay is a
/// gizmo line per edge, rebuilt every frame, so without a cap the instrument
/// that exists to explain a frame's cost becomes that cost. Ten thousand at
/// three lines each is inside a frame's budget and far more than
/// [`DebugOverlay::collision_radius`]'s default produces; when the cap is
/// reached, the panel says so rather than quietly drawing half a building.
pub const MAX_COLLISION_TRIANGLES: usize = 10_000;

/// How many triangles the last collision draw wanted, and how many it drew.
///
/// Published as a resource rather than logged because the cap above is a
/// silent truncation that otherwise reads as "this is all the collision there
/// is". The panel prints it beside the switch.
#[derive(Resource, Default)]
pub struct CollisionDrawn {
    pub hulls: usize,
    pub triangles: usize,
    pub capped: bool,
}

/// Whether a wireframe can be drawn in this run, and if not, why.
///
/// Resolved at startup from the real device rather than assumed, because
/// `WireframePlugin` declines silently on an adapter without
/// `POLYGON_MODE_LINE`: it logs one warning at startup and then the config
/// resource has no readers. A checkbox that ticks and does nothing would
/// report success for a switch that failed, so the panel disables it and says
/// which of the two reasons applies.
#[derive(Resource, Default, PartialEq, Eq, Debug)]
pub enum WireframeSupported {
    /// The plugin is in and the device has the features.
    Available,
    /// The plugin is in and the device lacks the features.
    #[default]
    NoGpuSupport,
    /// The plugin was left out of the run; see [`SUPPRESS`].
    SuppressedByEnv,
}

impl WireframeSupported {
    pub fn available(&self) -> bool {
        matches!(self, WireframeSupported::Available)
    }

    /// Why the checkbox is greyed out, for the panel to print. Empty when it
    /// is not.
    pub fn why(&self) -> &'static str {
        match self {
            WireframeSupported::Available => "",
            WireframeSupported::NoGpuSupport => "this gpu has no POLYGON_MODE_LINE",
            WireframeSupported::SuppressedByEnv => "left out by VALE_NO_WIREFRAME",
        }
    }
}

/// The environment variable that leaves `WireframePlugin` out of the run.
///
/// See the module note: the plugin costs 0.104 ms a frame at 30,000 meshes
/// with the switch off, and a cost paid by a switch nobody has touched has to
/// remain subtractable. Nothing else in this client reads it.
pub const SUPPRESS: &str = "VALE_NO_WIREFRAME";

pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugOverlay>()
            .init_resource::<CollisionDrawn>()
            .init_resource::<WireframeSupported>()
            .add_systems(Startup, probe_wireframe)
            .add_systems(
                Update,
                (
                    apply,
                    // Both are `Gizmos` writers and neither reads the other,
                    // so they are unordered on purpose: the immediate-mode
                    // buffer is per-system and the two never touch the same
                    // line.
                    draw_collision.run_if(|o: Res<DebugOverlay>| o.collision),
                    draw_facing.run_if(|o: Res<DebugOverlay>| o.facing),
                ),
            );
        // Nothing is asked of the adapter for this. See the module note:
        // `WgpuSettings::priority` is `Functionality`, so the device already
        // carries every feature it supports, and this plugin's own `finish`
        // checks for the two it needs and stands down with a warning if they
        // are absent.
        //
        // The one way to leave the plugin out exists because it is not free
        // even with the switch off; see [`SUPPRESS`].
        if std::env::var_os(SUPPRESS).is_none() {
            app.add_plugins(WireframePlugin::default());
        }
    }
}

/// Ask the device what it can do, once.
fn probe_wireframe(device: Option<Res<RenderDevice>>, mut supported: ResMut<WireframeSupported>) {
    if std::env::var_os(SUPPRESS).is_some() {
        *supported = WireframeSupported::SuppressedByEnv;
        return;
    }
    // The same two features `WireframePlugin::finish` requires. Named here
    // rather than inferred from whether the plugin installed, because the
    // plugin leaves no resource behind to look for.
    let wanted = bevy::render::settings::WgpuFeatures::POLYGON_MODE_LINE
        | bevy::render::settings::WgpuFeatures::IMMEDIATES;
    *supported = if device.is_some_and(|d| d.features().contains(wanted)) {
        WireframeSupported::Available
    } else {
        WireframeSupported::NoGpuSupport
    };
}

/// Push the switches into the engine's own config resources.
///
/// Guarded by `is_changed()`, which matters for the gizmo groups:
/// `GizmoConfigStore::config_mut` hands out a `&mut` that marks the store
/// changed, and the store's own change detection re-uploads every gizmo
/// group's buffers. Writing it every frame for three bools nobody moved would
/// add a per-frame cost with no switch behind it.
fn apply(
    overlay: Res<DebugOverlay>,
    // `Option`, because `VALE_NO_WIREFRAME` leaves the plugin that inserts
    // this out of the run; see [`SUPPRESS`]. The gizmo store is always there,
    // since `GizmoPlugin` is part of `DefaultPlugins`.
    wireframe: Option<ResMut<WireframeConfig>>,
    mut gizmos: ResMut<GizmoConfigStore>,
) {
    if !overlay.is_changed() {
        return;
    }
    if let Some(mut wireframe) = wireframe {
        wireframe.global = overlay.wireframe;
        wireframe.default_line_width = overlay.wireframe_width;
        // A colour rather than the default, which is black: this world is
        // mostly dark ground and a black wireframe over it is invisible where
        // the geometry question is hardest.
        wireframe.default_color = Color::srgb(0.35, 0.9, 1.0);
    }

    gizmos
        .config_mut::<bevy::gizmos::aabb::AabbGizmoConfigGroup>()
        .1
        .draw_all = overlay.aabbs;
    gizmos
        .config_mut::<bevy::gizmos::frustum::FrustumGizmoConfigGroup>()
        .1
        .draw_all = overlay.frustums;
    gizmos
        .config_mut::<bevy::gizmos::skinned_mesh_bounds::SkinnedMeshBoundsGizmoConfigGroup>()
        .1
        .draw_all = overlay.skinned_bounds;
}

/// The colour of a floor: a triangle the mover can stand on.
const FLOOR: Color = Color::srgb(0.35, 0.85, 0.35);
/// The colour of a wall. The split is the rule the mover walks by; see
/// [`vale_assets::Collider::triangle_is_wall`].
const WALL: Color = Color::srgb(0.95, 0.4, 0.35);

/// Draw the solid triangles standing near the world's focus.
///
/// The centre is [`WorldFocus`], not the character: the session writes the
/// focus from the character's position every frame, so in a session the two
/// are the same point, and a host drawing a map with no session writes the
/// focus itself and gets the overlay round whatever it is looking at. When
/// this read `WorldStatus` instead, the overlay drew nothing without a
/// character.
///
/// The hulls are stored in WoW's own axes and each vertex is converted on its
/// way to the gizmo. A conversion applied to the query instead would put the
/// overlay a rotation away from the thing it is drawn over, which is the
/// failure this overlay exists to find.
fn draw_collision(
    mut gizmos: Gizmos,
    overlay: Res<DebugOverlay>,
    solids: Res<Solids>,
    focus: Res<WorldFocus>,
    mut drawn: ResMut<CollisionDrawn>,
) {
    if !focus.present {
        return;
    }
    let centre = [focus.position.x, focus.position.y, focus.position.z];
    let radius = overlay.collision_radius;
    let cap = overlay.collision_triangles;
    let (mut hulls, mut triangles, mut capped) = (0usize, 0usize, false);
    solids.0.hulls_near(focus.map_id, centre, radius, |hull| {
        hulls += 1;
        for index in 0..hull.triangle_count() {
            if triangles >= cap {
                capped = true;
                return;
            }
            let tri = hull.triangle(index);
            // Reject per triangle as well as per hull: one hull can be a whole
            // building, and the radius is about what is underfoot.
            if !near(tri, centre, radius) {
                continue;
            }
            triangles += 1;
            let colour = if hull.triangle_is_wall(index) { WALL } else { FLOOR };
            let [a, b, c] = tri.map(axes::to_bevy);
            gizmos.line(a, b, colour);
            gizmos.line(b, c, colour);
            gizmos.line(c, a, colour);
        }
    });
    *drawn = CollisionDrawn { hulls, triangles, capped };
}

/// Whether any corner of the triangle is inside the query sphere's box.
///
/// A box rather than a sphere, and a corner rather than the whole triangle:
/// this is a display filter, not a collision test, and its one failure, a
/// triangle whose corners are all just outside a radius its middle crosses,
/// is one missing line at the edge of the overlay.
fn near(tri: [[f32; 3]; 3], centre: [f32; 3], radius: f32) -> bool {
    tri.iter()
        .any(|v| (0..3).all(|axis| (v[axis] - centre[axis]).abs() <= radius))
}

/// A stalk at every unit's origin and a spike up its own facing.
fn draw_facing(mut gizmos: Gizmos, units: Query<(&Transform, &WorldEntity)>) {
    for (transform, entity) in &units {
        let base = transform.translation;
        // The stalk first: it marks the origin, and an entity sunk into the
        // ground or floating over it is the commonest placement fault.
        gizmos.line(base, base + Vec3::Y * 3.0, Color::srgb(0.4, 0.6, 1.0));
        // The heading is the model's own forward rather than the number that
        // was meant to produce it: a strafe turns the body off the aim, so
        // drawing the aim would hide the disagreement this overlay exists to
        // show.
        let forward = transform.rotation * Vec3::NEG_Z;
        let colour = if entity.moving {
            Color::srgb(1.0, 0.85, 0.2)
        } else {
            Color::srgb(0.7, 0.7, 0.7)
        };
        gizmos.arrow(base + Vec3::Y * 1.0, base + Vec3::Y * 1.0 + forward * 2.5, colour);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every overlay is reachable from the panel and every one starts off,
    /// which is the opposite default from `WorldTuning`, on purpose: a
    /// subtraction left on hides part of the world, and an addition left on
    /// draws over it. Both read as "the world looks wrong" two sessions
    /// later, and the panel names whichever is set.
    #[test]
    fn every_overlay_is_on_the_panel_and_defaults_off() {
        let mut overlay = DebugOverlay::default();
        for (label, _, field) in OVERLAYS {
            assert!(!*field(&mut overlay), "{label} must default off");
        }
        // Six distinct fields rather than one read six times: a copied line in
        // `OVERLAYS` gives two labels one flag, and the panel would look right
        // while one of the two did nothing.
        for (index, (_, _, field)) in OVERLAYS.iter().enumerate() {
            *field(&mut overlay) = true;
            let on = OVERLAYS.iter().filter(|(_, _, f)| *f(&mut overlay)).count();
            assert_eq!(on, index + 1, "overlay {index} shares a field with another");
        }
    }

    /// The checkbox reaches the engine's own resources. This is the one thing
    /// about this module that could silently do nothing: `WireframeConfig`
    /// and the three gizmo groups are Bevy's, written from here and read
    /// elsewhere, so a switch wired to the wrong field would tick and change
    /// nothing.
    #[test]
    fn the_switches_reach_the_engines_own_config() {
        use bevy::gizmos::config::GizmoConfig;

        let mut app = App::new();
        // The groups are inserted into the store by hand rather than through
        // `init_gizmo_group`, which also schedules Bevy's own mesh-building
        // systems and drags the whole asset server in behind them. What is
        // under test is `apply`, and the store is all it touches.
        let mut store = GizmoConfigStore::default();
        store.insert(GizmoConfig::default(), bevy::gizmos::aabb::AabbGizmoConfigGroup::default());
        store.insert(
            GizmoConfig::default(),
            bevy::gizmos::frustum::FrustumGizmoConfigGroup::default(),
        );
        store.insert(
            GizmoConfig::default(),
            bevy::gizmos::skinned_mesh_bounds::SkinnedMeshBoundsGizmoConfigGroup::default(),
        );
        app.init_resource::<DebugOverlay>()
            .init_resource::<WireframeConfig>()
            .insert_resource(store)
            .add_systems(Update, apply);

        // The defaults, applied on the first run: everything off.
        app.update();
        assert!(!app.world().resource::<WireframeConfig>().global);

        {
            let mut overlay = app.world_mut().resource_mut::<DebugOverlay>();
            overlay.wireframe = true;
            overlay.wireframe_width = 2.5;
            overlay.aabbs = true;
            overlay.frustums = true;
            overlay.skinned_bounds = true;
        }
        app.update();

        let wireframe = app.world().resource::<WireframeConfig>();
        assert!(wireframe.global);
        assert!((wireframe.default_line_width - 2.5).abs() < 1e-6);

        let mut store = app.world_mut().resource_mut::<GizmoConfigStore>();
        assert!(store.config_mut::<bevy::gizmos::aabb::AabbGizmoConfigGroup>().1.draw_all);
        assert!(store.config_mut::<bevy::gizmos::frustum::FrustumGizmoConfigGroup>().1.draw_all);
        assert!(
            store
                .config_mut::<bevy::gizmos::skinned_mesh_bounds::SkinnedMeshBoundsGizmoConfigGroup>()
                .1
                .draw_all
        );
    }

    /// `--overlay` turns on what it names and nothing else, and a name that
    /// matches nothing leaves the world unadorned rather than guessing, which
    /// is the rule `--without` follows in the other direction.
    #[test]
    fn the_overlay_argument_turns_on_only_what_it_names() {
        let plain = DebugOverlay::with("");
        assert_eq!(plain, DebugOverlay::default());

        let one = DebugOverlay::with("wireframe");
        assert!(one.wireframe);
        assert!(!one.collision, "the other five stay off");

        // Spaces optional, case-insensitive, several at once.
        let several = DebugOverlay::with("boundingboxes, unit facing");
        assert!(several.aabbs && several.facing);
        assert!(!several.wireframe);

        // A misspelling is ignored rather than guessed at.
        assert_eq!(DebugOverlay::with("wirefame"), DebugOverlay::default());
    }

    /// The display filter admits a triangle with a corner inside the box and
    /// refuses one wholly outside it, including one that is close in two axes
    /// and a hundred yards up, which is the case a 2D test would let through
    /// and which is most of a city.
    #[test]
    fn the_collision_filter_is_a_box_on_all_three_axes() {
        let centre = [0.0, 0.0, 0.0];
        let under = [[1.0, 1.0, -0.5], [2.0, 1.0, -0.5], [2.0, 2.0, -0.5]];
        assert!(near(under, centre, 5.0));
        let far_east = [[50.0, 0.0, 0.0], [51.0, 0.0, 0.0], [51.0, 1.0, 0.0]];
        assert!(!near(far_east, centre, 5.0));
        let overhead = [[1.0, 1.0, 90.0], [2.0, 1.0, 90.0], [2.0, 2.0, 90.0]];
        assert!(!near(overhead, centre, 5.0), "a roof is not underfoot");
    }
}

