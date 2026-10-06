//! The game's interface drawn as meshes: the draw list the egui painter reads,
//! built into batched 2d geometry on the present camera instead of being
//! re-emitted through egui every frame.
//!
//! ```text
//! material.rs  one Material2d: a texture, vertex colours, and the game's own
//!              blend modes, which egui does not have
//! textures.rs  the art as bevy Images, through the same decode and the same
//!              byte-space alpha compensation the egui painter uses
//! fonts.rs     the four typefaces rasterised glyph by glyph into atlas pages
//! text.rs      where each glyph goes (wrap, rows, justify); pure and tested
//! build.rs     every Item kind as vertices: the kind handlers, the batcher,
//!              the clipper
//! floats.rs    the floating combat text, from the floater state the egui
//!              painter reads, in its own z band under the interface
//! loading.rs   the loading screen's three quads, over everything the game
//!              draws
//! ```
//!
//! ## Why the interface is not drawn through egui
//!
//! On 2026-09-02 the egui paint pipeline measured ~1.2 ms a frame of serial
//! main-thread work, because the walk's shapes were re-emitted and
//! re-tessellated every frame for a picture that changes at 30 Hz. This module
//! rebuilds meshes only when the walk ran, plus the minimap when the player
//! moved, so a frame between ticks costs nothing.
//!
//! ## Where the batches draw
//!
//! On the present camera's 2d phase. Every batch is a `Mesh2d` in
//! `Transparent2d`, keyed by z in draw-list order, over the present quad (an
//! opaque draw at z 0) and under egui's own pass, so the F4 window, the HUD
//! and the floating combat text paint over the interface.
//!
//! ## Choosing the painter
//!
//! This painter is the default. It was checked against the egui painter's
//! picture on the world, the panels, the glue screens, combat text and a live
//! fight. `VALE_UI_PAINTER=egui` selects the egui painter, kept for one round
//! for side-by-side comparison. Deleting it, and moving bevy_egui behind the
//! `diagnostics` feature, is the next step. The walk, the widgets, the events
//! and the mouse are the same under either painter.

pub mod build;
pub mod floats;
pub mod fonts;
pub mod loading;
pub mod material;
pub mod text;
pub mod textures;

use bevy::asset::embedded_asset;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::sprite_render::{Material2dPlugin, MeshMaterial2d};
use bevy::window::PrimaryWindow;
use std::collections::{HashMap, HashSet};

use material::{Blend, InterfaceMaterial};

/// Whether the mesh painter is on. Read once and cached; this plugin and the
/// egui painters' emission checks both call it, so they cannot disagree.
///
/// On unless `VALE_UI_PAINTER=egui`.
pub fn active() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        !std::env::var("VALE_UI_PAINTER").is_ok_and(|v| v.eq_ignore_ascii_case("egui"))
    })
}

/// One batch's entity, kept across rebuilds.
///
/// A rebuild rewrites the slot's mesh asset under the same handle and does not
/// respawn the entity, so the entity, its pipeline and its bind groups are
/// kept. A despawn-and-respawn rebuild blinked the whole interface for one
/// frame per tick (a 30 Hz flicker), because a newly spawned `Mesh2d` was not
/// specialized until the next frame.
struct Slot {
    entity: Entity,
    mesh: Handle<Mesh>,
    image: AssetId<Image>,
    blend: Blend,
    z: f32,
    /// A hash of the geometry last written. A rebuild that produces the same
    /// batch skips the upload; on an active frame most batches are unchanged.
    print: u64,
}

/// Everything the rebuild keeps between runs.
#[derive(Default)]
struct MeshState {
    fixed: Vec<Slot>,
    minimap: Vec<Slot>,
    /// The `<Model>` frames. A separate group because a flat model's triangles
    /// depend on the wall clock and on nothing in the item list. See
    /// `build::model_batches`.
    models: Vec<Slot>,
    /// The icon on the pointer: one quad, in a separate group because it moves
    /// at frame rate. See `build::carried_batch`.
    carried: Vec<Slot>,
    last_carried: Option<(String, Vec2)>,
    /// One material instance per `(texture, blend)`, kept across rebuilds so
    /// bind groups are kept and prepare does not recreate them.
    materials: HashMap<(AssetId<Image>, Blend), Handle<InterfaceMaterial>>,
    /// Which portrait each token resolved to at the last build. A portrait
    /// arriving changes no item, so it is tracked here.
    portraits: Vec<(String, Option<AssetId<Image>>)>,
    /// The item list the current meshes were built from. The walk produces a
    /// new list every tick whether or not anything changed, so `Drawn`'s
    /// change flag alone would rebuild thirty times a second at idle. The
    /// deep compare costs tens of microseconds against milliseconds for a
    /// rebuild, and at idle it is all this system does.
    ///
    /// Compared through [`same_fixed`] rather than with `==`: a `<Model>` item
    /// is drawn by its own group, so a change inside it does not make the
    /// fixed meshes stale.
    last_items: Vec<crate::lua::widgets::draw::Item>,
    last_screen: Vec2,
    /// The window's scale factor at the last build. It can change while the
    /// logical size does not (a window moved to a monitor at another DPI), and
    /// every glyph is rasterised against it, so a change makes the meshes
    /// stale.
    last_dpi: f32,
    /// The minimap's inputs at the last build. The minimap is the one group
    /// whose inputs are the world rather than the widget tree. Compared by
    /// value because `MinimapView` is rewritten every frame, so its change
    /// flag does not say whether the player moved.
    last_place: Option<(bool, (f32, f32), f32, bool, String)>,
    reported_models: HashSet<String>,
    /// Whether the first-build log line was written; the mesh counterpart of
    /// the egui painter's "first frame drawn" line.
    reported: bool,
}

pub struct UiMeshPlugin;

impl Plugin for UiMeshPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/interface.wgsl");
        app.add_plugins(Material2dPlugin::<InterfaceMaterial>::default())
            .init_resource::<textures::UiTextures>()
            .init_resource::<fonts::UiFonts>()
            // After the walk, so a tick's items are drawn in the frame they are
            // produced, with the same latency the egui emission has. The walk
            // runs inside bevy_egui's context pass (`framexml::paint` is
            // registered in `EguiPrimaryContextPass`), whose loop system is in
            // `EguiPostUpdateSet::EndPass`, so the edge is against that set.
            //
            // Before the frame's transform, visibility and specialization
            // sweeps. These systems spawn entities and swap material and
            // `Transform` components through `Commands`, and a `before` edge
            // makes bevy flush those commands ahead of the target. Propagation
            // then computes this frame's `GlobalTransform`, the visibility
            // sweeps see a new entity's bounds, and the specialization check
            // queues a swapped material in this frame. Without the edges each
            // of those landed one frame late: a click that reordered the list
            // blinked the whole interface for one frame, and a dropdown opened
            // over the world map was drawn for one frame under the overlays it
            // should cover.
            //
            // Before `AssetEventSystems`. These systems rewrite `Mesh` and
            // `Image` assets in place (batch geometry, glyph atlas pages,
            // textures). A write becomes an `AssetEvent` only when that
            // asset's `asset_events` system runs, and the render world
            // extracts an asset only on its event. Both systems take
            // `ResMut<Assets<Mesh>>`, so without the edge bevy may run them in
            // either order, and the order can differ from frame to frame. In
            // a frame where `asset_events` ran first, the new geometry reached
            // the GPU one frame after the material and z changes made through
            // `Commands`, so for one frame a batch was drawn with its previous
            // vertices and its new texture and z.
            .add_systems(
                PostUpdate,
                (rebuild, floats::paint, loading::paint)
                    .after(bevy_egui::EguiPostUpdateSet::EndPass)
                    .before(bevy::asset::AssetEventSystems)
                    .before(bevy::transform::TransformSystems::Propagate)
                    .before(bevy::camera::visibility::VisibilitySystems::CalculateBounds)
                    .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)
                    .before(
                        bevy::sprite_render::check_entities_needing_specialization::<
                            InterfaceMaterial,
                        >,
                    ),
            );
    }
}

/// The resources the rebuild reads and does not write, bundled because the
/// full parameter list exceeded Bevy's limit of sixteen.
#[derive(bevy::ecs::system::SystemParam)]
struct Reads<'w> {
    drawn: Res<'w, crate::ui::framexml::Drawn>,
    tuning: Res<'w, crate::render::tuning::WorldTuning>,
    models: Res<'w, crate::lua::widgets::model::UiModels>,
    portraits: Res<'w, crate::render::portraits::Portraits>,
    dolls: Res<'w, crate::render::paperdoll::Dolls>,
    place: Res<'w, crate::interface::minimap::MinimapView>,
    cursor: Res<'w, crate::interface::cursor::Cursor>,
    ui_scale: Res<'w, crate::ui::scale::InterfaceScale>,
    time: Res<'w, Time>,
}

/// Rebuild whatever moved: the whole list when the walk ran, the minimap when
/// the world under it did, the carried icon when the pointer did.
#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    reads: Reads,
    assets: Res<crate::assets::GameAssets>,
    mut textures: ResMut<textures::UiTextures>,
    mut ui_fonts: ResMut<fonts::UiFonts>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<InterfaceMaterial>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut state: Local<MeshState>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Interface);
    // One deref, so the fields below borrow disjointly.
    let state = &mut *state;
    let Reads { drawn, tuning, models, portraits, dolls, place, cursor, ui_scale, time } = reads;
    let Ok(window) = window.single() else {
        return;
    };
    let screen = Vec2::new(window.width(), window.height());
    if screen.x <= 0.0 || screen.y <= 0.0 {
        return;
    }

    let items = &drawn.items;
    if !tuning.interface || items.is_empty() {
        despawn(&mut commands, &mut state.fixed);
        despawn(&mut commands, &mut state.minimap);
        despawn(&mut commands, &mut state.models);
        despawn(&mut commands, &mut state.carried);
        state.last_items.clear();
        state.last_place = None;
        state.last_carried = None;
        return;
    }

    // A portrait arriving changes no item, so its arrival is watched by what
    // each token resolves to now against what it resolved to at the build.
    let faces: Vec<(String, Option<AssetId<Image>>)> = items
        .iter()
        .filter_map(|item| item.paint())
        .filter_map(|paint| paint.portrait.clone())
        .map(|token| {
            let id = portraits.image(&token).map(|handle| handle.id());
            (token, id)
        })
        .collect();

    let here = (
        place.in_world,
        place.position,
        place.facing,
        place.indoors,
        place.directory.clone(),
    );
    // The icon on the pointer, the one input here that moves at frame rate:
    // the pointer position, in the logical pixels the egui painter read from
    // its context.
    let holding: Option<(String, Vec2)> = cursor
        .held
        .as_ref()
        .and_then(|held| held.texture())
        .zip(window.cursor_position())
        .map(|(path, at)| (path.to_string(), at));

    let dpi = window.scale_factor();
    let fixed_stale = screen != state.last_screen
        || dpi != state.last_dpi
        || faces != state.portraits
        || !same_fixed(items, &state.last_items);
    let minimap_stale = fixed_stale || state.last_place.as_ref() != Some(&here);
    let carried_stale = fixed_stale || holding != state.last_carried;
    // Models are rebuilt on every frame that has one on the screen:
    // `build::model_batches` reads the wall clock and nothing in the item
    // list, so there is no staleness test to make. With no model on the screen
    // this costs the `any`.
    let models_stale = items
        .iter()
        .any(|item| matches!(item.content, crate::lua::widgets::draw::Content::Model(_)))
        || !state.models.is_empty();
    if !fixed_stale && !minimap_stale && !carried_stale && !models_stale {
        return;
    }
    state.portraits = faces;
    state.last_screen = screen;
    state.last_dpi = dpi;
    state.last_place = Some(here);

    let view = crate::lua::widgets::layout::Viewport::of(
        f64::from(screen.x),
        f64::from(screen.y),
        ui_scale.get(),
    );
    let mut painter = build::Painter {
        assets: &assets,
        images: &mut images,
        textures: &mut textures,
        fonts: &mut ui_fonts,
        models: &models,
        portraits: &portraits,
        dolls: &dolls,
        place: &place,
        view,
        dpi,
        now_ms: (time.elapsed_secs_f64() * 1000.0).max(0.0) as u32,
        reported_models: &mut state.reported_models,
    };

    if fixed_stale {
        let batches = build::item_batches(items, &mut painter);
        apply(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut state.materials,
            &mut state.fixed,
            batches,
            screen,
        );
        state.last_items = items.clone();
    }
    if minimap_stale {
        let batches = build::minimap_batches(items, &mut painter);
        apply(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut state.materials,
            &mut state.minimap,
            batches,
            screen,
        );
    }
    if models_stale {
        let batches = build::model_batches(items, &mut painter);
        apply(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut state.materials,
            &mut state.models,
            batches,
            screen,
        );
    }
    if carried_stale {
        let batches = match &holding {
            Some((path, at)) => build::carried_batch(&mut painter, path, *at),
            None => Vec::new(),
        };
        apply(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut state.materials,
            &mut state.carried,
            batches,
            screen,
        );
        state.last_carried = holding;
    }
    ui_fonts.flush(&mut images);
    if !state.reported {
        state.reported = true;
        info!(
            "interface mesh: first build — {} items into {} batches ({} minimap, {} model)",
            items.len(),
            state.fixed.len() + state.minimap.len() + state.models.len(),
            state.minimap.len(),
            state.models.len(),
        );
    }
}

/// Whether two item lists produce the same fixed meshes.
///
/// A `<Model>` item is drawn by `build::model_batches` and contributes only
/// its index to the fixed batches, so a change inside it is not a reason to
/// rebuild the rest of the interface. `Scene::elapsed` changes every tick of
/// every running cooldown; compared with `==`, the whole interface was rebuilt
/// thirty times a second while any ability was on cooldown.
///
/// The index still matters, so this compares the lists position by position
/// rather than filtering models out: a model appearing or disappearing shifts
/// the z of every later item.
fn same_fixed(
    items: &[crate::lua::widgets::draw::Item],
    last: &[crate::lua::widgets::draw::Item],
) -> bool {
    use crate::lua::widgets::draw::Content;
    items.len() == last.len()
        && items.iter().zip(last.iter()).all(|(a, b)| match (&a.content, &b.content) {
            (Content::Model(_), Content::Model(_)) => true,
            _ => a == b,
        })
}

fn despawn(commands: &mut Commands, slots: &mut Vec<Slot>) {
    for slot in slots.drain(..) {
        commands.entity(slot.entity).despawn();
    }
}

/// Fit the slots to this build's batches: rewrite each mesh under its own
/// handle, spawn an entity only for a batch with no slot, and change a slot's
/// material or z only when it differs. See [`Slot`] for why entities are not
/// respawned.
#[allow(clippy::too_many_arguments)]
fn apply(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<InterfaceMaterial>,
    cache: &mut HashMap<(AssetId<Image>, Blend), Handle<InterfaceMaterial>>,
    slots: &mut Vec<Slot>,
    batches: Vec<build::Batch>,
    screen: Vec2,
) {
    let wanted = batches.len();
    for (index, batch) in batches.into_iter().enumerate() {
        let material = cache
            .entry((batch.image.id(), batch.blend))
            .or_insert_with(|| {
                materials.add(InterfaceMaterial {
                    texture: batch.image.clone(),
                    blend: batch.blend,
                })
            })
            .clone();
        let print = fingerprint(&batch, screen);
        match slots.get_mut(index) {
            Some(slot) => {
                if slot.print != print {
                    slot.print = print;
                    // The slot holds the handle, so the id's generation is
                    // live and this cannot fail. It warns rather than unwraps,
                    // so a later change to slot lifetime produces a stale
                    // batch and a warning instead of a panic.
                    if meshes.insert(slot.mesh.id(), to_screen_mesh(&batch, screen)).is_err() {
                        warn!("interface mesh: a batch mesh handle went stale");
                    }
                }
                if slot.image != batch.image.id() || slot.blend != batch.blend {
                    slot.image = batch.image.id();
                    slot.blend = batch.blend;
                    commands.entity(slot.entity).insert(MeshMaterial2d(material));
                }
                if slot.z != batch.z {
                    slot.z = batch.z;
                    // `Transparent2d` keeps each entity's phase item between
                    // frames and computes its sort key from the z only when
                    // the entity is queued, which happens when `Mesh2d` or
                    // the material changed. A `Transform` change alone leaves
                    // the old sort key in place. Re-inserting the same
                    // `Mesh2d` marks it changed, so the entity is queued again
                    // this frame with the new z.
                    commands.entity(slot.entity).insert((
                        Transform::from_xyz(0.0, 0.0, batch.z),
                        bevy::mesh::Mesh2d(slot.mesh.clone()),
                    ));
                }
            }
            None => {
                let mesh = meshes.add(to_screen_mesh(&batch, screen));
                let entity = commands
                    .spawn((
                        bevy::mesh::Mesh2d(mesh.clone()),
                        MeshMaterial2d(material),
                        Transform::from_xyz(0.0, 0.0, batch.z),
                        // The present camera's layer. That camera is the only
                        // 2D camera in the app and it sees only this layer, so
                        // a batch spawned without it is not drawn. See
                        // [`crate::render::present::PRESENT_LAYER`] and the
                        // test `the_interface_is_on_the_layer_that_draws_it`.
                        RenderLayers::layer(crate::render::present::PRESENT_LAYER),
                        Name::new("interface batch"),
                    ))
                    .id();
                slots.push(Slot {
                    entity,
                    mesh,
                    image: batch.image.id(),
                    blend: batch.blend,
                    z: batch.z,
                    print,
                });
            }
        }
    }
    for slot in slots.drain(wanted..) {
        commands.entity(slot.entity).despawn();
    }
}

/// A hash of one batch's geometry and the screen size. The size is part of the
/// pixel-to-camera mapping, so the same pixels at another window size are a
/// different mesh. A collision draws one stale batch for one tick; the hasher
/// is 64-bit SipHash over the batch's own bytes.
fn fingerprint(batch: &build::Batch, screen: Vec2) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    screen.x.to_bits().hash(&mut hasher);
    screen.y.to_bits().hash(&mut hasher);
    for p in &batch.positions {
        p[0].to_bits().hash(&mut hasher);
        p[1].to_bits().hash(&mut hasher);
    }
    for uv in &batch.uvs {
        uv[0].to_bits().hash(&mut hasher);
        uv[1].to_bits().hash(&mut hasher);
    }
    for colour in &batch.colours {
        for channel in colour {
            channel.to_bits().hash(&mut hasher);
        }
    }
    batch.indices.hash(&mut hasher);
    hasher.finish()
}

/// Window pixels (y down) into the present camera's world. The camera has a
/// `Fixed { 1, 1 }` orthographic projection, so the viewport is the unit
/// square about the origin, y up. See `render::present`, whose full-screen
/// quad uses the same space.
fn to_screen_mesh(batch: &build::Batch, screen: Vec2) -> Mesh {
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    let positions: Vec<[f32; 3]> = batch
        .positions
        .iter()
        .map(|p| [p[0] / screen.x - 0.5, 0.5 - p[1] / screen.y, 0.0])
        .collect();
    let normals = vec![[0.0f32, 0.0, 1.0]; positions.len()];
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, batch.uvs.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, batch.colours.clone());
    mesh.insert_indices(bevy::mesh::Indices::U32(batch.indices.clone()));
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::widgets::draw::{Content, Item, Order};
    use crate::lua::widgets::layout::Rect;
    use crate::lua::widgets::model::Scene;
    use crate::lua::widgets::statusbar::Bar;

    /// The painter draws onto the layer the present camera sees.
    ///
    /// The present camera is the only 2D camera in the app, so a batch on any
    /// other layer is not drawn. The interface is then absent from the window
    /// while every count, log line and audit reports success. This happened
    /// when the present camera was moved to its own layer, so that it stopped
    /// drawing the world's gizmos a second time, and the painter still spawned
    /// onto layer 0.
    #[test]
    fn the_interface_is_on_the_layer_that_draws_it() {
        let present = RenderLayers::layer(crate::render::present::PRESENT_LAYER);
        let batch = RenderLayers::layer(crate::render::present::PRESENT_LAYER);
        assert!(
            present.intersects(&batch),
            "the interface painter spawns onto a layer the present camera cannot see"
        );
        // The present camera does not see the world's layer, which is what its
        // own layer is for: it draws over the world and not a second copy of it.
        assert!(!present.intersects(&RenderLayers::layer(0)));
    }

    fn at(content: Content) -> Item {
        Item {
            rect: Rect { left: 0.0, bottom: 0.0, width: 30.0, height: 30.0 },
            content,
            alpha: 1.0,
            order: Order { strata: 0, level: 0, layer: 0, text: false, sequence: 0 },
            clip: None,
        }
    }

    fn a_bar() -> Item {
        at(Content::Bar(Bar {
            fraction: 0.5,
            texture: None,
            colour: [1.0; 4],
            layer: 0,
            vertical: false,
        }))
    }

    fn a_model(elapsed: f64) -> Item {
        at(Content::Model(Scene {
            frame: "PetActionButton1AutoCast".to_string(),
            file: r"Interface\Buttons\UI-AutoCastButton.mdx".to_string(),
            sequence: 0,
            camera: 0,
            fog: None,
            facing: 0.0,
            character_facing: 0.0,
            scale: 1.0,
            unit: None,
            rotation: 0.0,
            elapsed,
        }))
    }

    /// A change inside a `<Model>` item does not make the fixed meshes stale;
    /// a model appearing or disappearing does.
    ///
    /// `Scene::elapsed` changes every tick of every running cooldown, and with
    /// a whole-list `==` the whole interface was rebuilt thirty times a second
    /// while an ability was on cooldown. [`super::same_fixed`] compares position
    /// by position rather than filtering models out because each batch's z is
    /// its item's index, so a new model shifts the z of every later item.
    #[test]
    fn a_models_own_clock_does_not_make_the_fixed_meshes_stale() {
        let a = vec![a_bar(), a_model(0.0)];
        assert!(super::same_fixed(&a, &[a_bar(), a_model(1.5)]), "only the clock moved");

        // A change to the model item's own fields, here its alpha, does not
        // either: the fixed group draws nothing for a model.
        let mut faded = a_model(0.0);
        faded.alpha = 0.5;
        assert!(super::same_fixed(&a, &[a_bar(), faded]));

        // A model arriving, leaving or swapping places with something else is.
        assert!(!super::same_fixed(&a, &[a_bar()]));
        assert!(!super::same_fixed(&a, &[a_model(0.0), a_bar()]));
        assert!(!super::same_fixed(&a, &[a_bar(), a_bar()]));

        // An item that is not a model still compares by value.
        let mut fuller = a_bar();
        if let Content::Bar(bar) = &mut fuller.content {
            bar.fraction = 0.75;
        }
        assert!(!super::same_fixed(&a, &[fuller, a_model(0.0)]));
    }

    /// A slot whose z changed has its `Mesh2d` marked changed, and a slot whose
    /// z did not change does not.
    ///
    /// Bevy's 2d phase computes a batch's sort key when the entity is queued
    /// and queues it again only when `Mesh2d` or the material changed. If a z
    /// change touched only `Transform`, the batch kept its old place in the
    /// draw order: a static border could sort under a health bar whose
    /// geometry, and so whose sort key, had been updated since.
    #[test]
    fn a_batch_whose_z_moved_is_marked_for_queueing_again() {
        use bevy::ecs::world::CommandQueue;

        let batch = |z: f32| build::Batch {
            image: Handle::default(),
            blend: Blend::Alpha,
            z,
            positions: vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]],
            uvs: vec![[0.0; 2]; 3],
            colours: vec![[1.0; 4]; 3],
            indices: vec![0, 1, 2],
        };
        let mut world = World::new();
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<InterfaceMaterial>::default();
        let mut cache = HashMap::new();
        let mut slots = Vec::new();
        let screen = Vec2::new(100.0, 100.0);
        let mut run = |world: &mut World, slots: &mut Vec<Slot>, z: f32| {
            let mut queue = CommandQueue::default();
            let mut commands = Commands::new(&mut queue, world);
            apply(
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut cache,
                slots,
                vec![batch(z)],
                screen,
            );
            queue.apply(world);
        };

        run(&mut world, &mut slots, 1.0);
        let entity = slots[0].entity;
        world.clear_trackers();

        run(&mut world, &mut slots, 1.0);
        let mesh = world.entity(entity).get_ref::<bevy::mesh::Mesh2d>().unwrap();
        assert!(!mesh.is_changed(), "nothing moved, so nothing is queued again");
        world.clear_trackers();

        run(&mut world, &mut slots, 1.002);
        assert_eq!(slots.len(), 1, "the slot is reused, not respawned");
        let entity_ref = world.entity(entity);
        assert_eq!(entity_ref.get::<Transform>().unwrap().translation.z, 1.002);
        assert!(
            entity_ref.get_ref::<bevy::mesh::Mesh2d>().unwrap().is_changed(),
            "a moved z must mark Mesh2d changed, or the phase keeps the old sort key"
        );
    }

    /// The URI the material returns is the path the macro registers. Every
    /// embedded shader in this crate has this test, written in a file whose
    /// `file!()` parent is the embedder's.
    #[test]
    fn the_shader_uri_is_the_path_the_macro_registers() {
        assert_eq!(
            super::material::SHADER,
            crate::embedded_shader_uri!("interface.wgsl")
        );
    }
}
