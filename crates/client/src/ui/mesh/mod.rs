//! **The game's interface as meshes** — the same draw list the egui painter
//! reads, built into batched 2d geometry on the present camera instead of
//! being re-emitted through egui every frame.
//!
//! ```text
//! material.rs  one Material2d: a texture, vertex colours, and the game's own
//!              blend modes — the thing egui structurally could not give us
//! textures.rs  the art as bevy Images, through the same decode and the same
//!              byte-space alpha door the egui painter uses
//! fonts.rs     the four typefaces rasterised glyph-by-glyph into atlas pages
//! text.rs      where each glyph goes — wrap, rows, justify — pure and tested
//! build.rs     every Item kind as vertices: the kind handlers, the batcher,
//!              the clipper
//! floats.rs    the floating combat text, off the same floater state the egui
//!              twin reads, in its own z band under the interface
//! loading.rs   the loading screen's three quads, over everything the game
//!              draws
//! ```
//!
//! ## Why
//!
//! The 2026-09-02 frame-wall round measured the egui paint pipeline at
//! ~1.2 ms a frame of serial main-thread work — the walk's shapes re-emitted
//! and re-tessellated every frame for a picture that changes at 30 Hz. This
//! module rebuilds meshes **only when the walk ran** (plus the minimap, which
//! moves with the player), so the per-frame cost between ticks is nothing at
//! all.
//!
//! ## Where it draws
//!
//! On the present camera's 2d phase: every batch is a `Mesh2d` in
//! `Transparent2d`, z-keyed by draw-list order, over the present quad (an
//! opaque draw at z 0) and under egui's own pass — so the F4 window, the HUD
//! and the floating combat text keep painting over the interface exactly as
//! they do today.
//!
//! ## The switch
//!
//! **This painter is the default** as of the soak that closed the parity
//! round — the world, the panels, the glue screens, combat text and a live
//! fight, each checked against the egui painter's own picture.
//! `VALE_UI_PAINTER=egui` selects the old painter, kept for one round as
//! the side-by-side and the escape hatch; its deletion — and with it
//! bevy_egui's retreat behind the `diagnostics` feature — is the next step.
//! The walk, the widgets, the events and the mouse are untouched either way:
//! the painter is a leaf.

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

/// Whether the mesh painter is on — one read, cached, checked by both this
/// plugin and the egui painters' emission skips so the sides cannot disagree.
///
/// On unless `VALE_UI_PAINTER=egui` asks for the old painter back.
pub fn active() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        !std::env::var("VALE_UI_PAINTER").is_ok_and(|v| v.eq_ignore_ascii_case("egui"))
    })
}

/// One batch's standing entity.
///
/// **Rewritten in place, never respawned.** A freshly spawned `Mesh2d` misses
/// the frame's specialization pass (this system runs late in `PostUpdate`, by
/// design, after the walk), so a despawn-and-respawn rebuild blinked the whole
/// interface for one frame per tick — which on screen is a 30 Hz flicker. The
/// mesh **asset** is replaced under the same handle instead, and the entity,
/// its pipeline and its bind groups all stand.
struct Slot {
    entity: Entity,
    mesh: Handle<Mesh>,
    image: AssetId<Image>,
    blend: Blend,
    z: f32,
    /// A hash of the geometry last written, so a rebuild that reproduced the
    /// same batch skips the re-upload — on an active frame most batches are
    /// bystanders to whatever moved.
    print: u64,
}

/// Everything the rebuild keeps between runs.
#[derive(Default)]
struct MeshState {
    fixed: Vec<Slot>,
    minimap: Vec<Slot>,
    /// The `<Model>` frames — their own group because a flat model's triangles
    /// are a function of the wall clock and of nothing in the item list. See
    /// `build::model_batches`.
    models: Vec<Slot>,
    /// The icon on the pointer — one quad, its own group because it moves at
    /// frame rate. See `build::carried_batch`.
    carried: Vec<Slot>,
    last_carried: Option<(String, Vec2)>,
    /// One material instance per `(texture, blend)`, kept across rebuilds so
    /// bind groups survive and prepare does not churn.
    materials: HashMap<(AssetId<Image>, Blend), Handle<InterfaceMaterial>>,
    /// Which portrait each token resolved to at the last build — the arrival
    /// of a picture changes no item, so it is watched here.
    portraits: Vec<(String, Option<AssetId<Image>>)>,
    /// The list the standing meshes were built from. The walk produces a fresh
    /// list every tick whether or not anything moved, so `Drawn`'s change flag
    /// alone would rebuild thirty times a second at idle; the deep compare is
    /// tens of microseconds against a rebuild that is milliseconds, and at
    /// idle it is the whole of what this system does.
    ///
    /// Compared through [`same_fixed`] rather than with `==`: a `<Model>` item
    /// is drawn by its own pass, so what it says about itself cannot make the
    /// fixed meshes stale.
    last_items: Vec<crate::lua::widgets::draw::Item>,
    last_screen: Vec2,
    /// The window's scale factor at the last build. It moves without the
    /// logical size moving — a window dragged to a monitor at another DPI —
    /// and every glyph in the tree is rastered against it, so it is part of
    /// what makes the meshes stale.
    last_dpi: f32,
    /// What the minimap drew from — the one population whose inputs are the
    /// world rather than the widget tree, compared by value for the same
    /// reason `last_items` is: `MinimapView` is rewritten every frame and its
    /// change flag says nothing about whether the player moved.
    last_place: Option<(bool, (f32, f32), f32, bool, String)>,
    reported_models: HashSet<String>,
    /// The one first-build log, the mesh counterpart of the egui painter's
    /// "first frame drawn" line.
    reported: bool,
}

pub struct UiMeshPlugin;

impl Plugin for UiMeshPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/interface.wgsl");
        app.add_plugins(Material2dPlugin::<InterfaceMaterial>::default())
            .init_resource::<textures::UiTextures>()
            .init_resource::<fonts::UiFonts>()
            // After the walk, so a tick's items are drawn the frame they are
            // produced — the same latency the egui emission has. The walk runs
            // inside bevy_egui's context pass (`framexml::paint` is registered
            // in `EguiPrimaryContextPass`), whose loop system sits in
            // `EguiPostUpdateSet::EndPass` — so the ordering is against the
            // set, stated rather than inherited.
            //
            // **…and before the frame's own sweeps, which is a flicker fix and
            // not tidiness.** These systems spawn entities and swap material
            // and `Transform` components through `Commands`; a `before` edge
            // makes bevy flush those commands ahead of the target, so
            // propagation computes this frame's `GlobalTransform` (a batch
            // whose z moved sorts right *now*, not next frame), the visibility
            // sweeps see a fresh spawn's bounds, and the specialization check
            // marks a swapped material for this frame's queue. Without the
            // edges every one of those landed a frame late: a click that
            // reordered the list blinked the whole interface for one frame,
            // and a dropdown opened over the world map spent a frame under
            // the overlays it should cover.
            .add_systems(
                PostUpdate,
                (rebuild, floats::paint, loading::paint)
                    .after(bevy_egui::EguiPostUpdateSet::EndPass)
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

/// What the rebuild reads and never writes, bundled — the bare parameter list
/// was past Bevy's sixteen.
#[derive(bevy::ecs::system::SystemParam)]
struct Reads<'w> {
    drawn: Res<'w, crate::ui::framexml::Drawn>,
    tuning: Res<'w, crate::render::tuning::WorldTuning>,
    models: Res<'w, crate::lua::widgets::model::UiModels>,
    portraits: Res<'w, crate::render::portraits::Portraits>,
    dolls: Res<'w, crate::render::paperdoll::Dolls>,
    place: Res<'w, crate::game::place::minimap::MinimapView>,
    cursor: Res<'w, crate::game::combat::cursor::Cursor>,
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
    // The icon on the pointer, which is the one input here that moves at
    // frame rate — the pointer position, in the same logical pixels the egui
    // painter read off its context.
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
    // **Every frame there is a model on the screen**, which is what an
    // animation is: `build::model_batches` reads the wall clock and nothing in
    // the item list, so there is no staleness test to make. An empty screen
    // costs the `any` and stops.
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

/// **Are these two item lists the same as far as the *fixed* meshes go?**
///
/// `==` on the whole list would do, and did — but a `<Model>` item is drawn by
/// `build::model_batches` and contributes nothing to the fixed batches except
/// its index, so anything it says about itself moving is not a reason to
/// rebuild the rest of the interface. `Scene::elapsed` moves every tick of
/// every running cooldown, and under the old compare that was the whole screen
/// rebuilt thirty times a second whenever an ability was on cooldown.
///
/// The index still matters, which is why this walks in step rather than
/// filtering: a model appearing or disappearing shifts every z below it.
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

/// Fit the standing entities to this build's batches: rewrite each mesh under
/// its own handle, respawn nothing that already exists, and only touch a
/// slot's material or z when they actually moved — see [`Slot`], which is the
/// flicker argument.
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
                    // live and this cannot fail today; a line rather than an
                    // unwrap, so a future change to slot lifetime degrades to
                    // a stale batch and a warning instead of a panic.
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
                    commands
                        .entity(slot.entity)
                        .insert(Transform::from_xyz(0.0, 0.0, batch.z));
                }
            }
            None => {
                let mesh = meshes.add(to_screen_mesh(&batch, screen));
                let entity = commands
                    .spawn((
                        bevy::mesh::Mesh2d(mesh.clone()),
                        MeshMaterial2d(material),
                        Transform::from_xyz(0.0, 0.0, batch.z),
                        // **The present camera's layer, and it is not
                        // decoration.** That camera is the only 2D one in the
                        // app and it sees exactly this layer, so a batch
                        // spawned without it is a batch nothing draws. See
                        // [`crate::render::present::PRESENT_LAYER`], and
                        // `the_interface_is_on_the_layer_that_draws_it` below,
                        // which is the test that would have caught it.
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

/// One batch's geometry as a number, screen size folded in — the size is part
/// of the pixel-to-camera mapping, so the same pixels at a new window are a
/// different mesh. A collision draws one stale batch for one tick; the hasher
/// is 64-bit SipHash and the input is the batch's own bytes.
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

/// Window pixels (y down) into the present camera's world — a `Fixed { 1, 1 }`
/// orthographic projection, so the viewport is the unit square about the
/// origin, y up. See `render::present`, whose full-screen quad set the space.
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

    /// **The painter draws onto the layer the present camera sees.**
    ///
    /// The one property in this file that nothing else can check and that
    /// nothing in it makes obvious. The present camera is the only 2D camera in
    /// the app; a batch on any other layer is a batch no camera draws, and the
    /// symptom is the entire interface silently absent while every count, every
    /// log line and every audit reports success — the frames load, the scripts
    /// run, the loading screen goes up and comes down on time, and the window
    /// shows the world and nothing over it.
    ///
    /// That is not hypothetical: it is what happened when the present camera
    /// was given a layer of its own to stop it re-drawing the world's gizmos,
    /// and the painter was left spawning onto layer 0.
    #[test]
    fn the_interface_is_on_the_layer_that_draws_it() {
        let present = RenderLayers::layer(crate::render::present::PRESENT_LAYER);
        let batch = RenderLayers::layer(crate::render::present::PRESENT_LAYER);
        assert!(
            present.intersects(&batch),
            "the interface painter spawns onto a layer the present camera cannot see"
        );
        // …and the world's layer is still not one of them, which is the
        // property the layer was introduced for. The two together are the whole
        // constraint: over the world, and not a second copy of it.
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

    /// **A `<Model>` moving is not a reason to rebuild the interface**, and a
    /// model appearing is.
    ///
    /// The first half is the cooldown swirl: `Scene::elapsed` moves every tick
    /// of every running cooldown, and under a whole-list `==` that was the
    /// entire screen re-tessellated thirty times a second whenever an ability
    /// was on cooldown. The second is the reason [`super::same_fixed`] walks in
    /// step rather than filtering the models out — the z of every batch is its
    /// item's index, so a model that was not there before shifts everything
    /// under it.
    #[test]
    fn a_models_own_clock_does_not_make_the_fixed_meshes_stale() {
        let a = vec![a_bar(), a_model(0.0)];
        assert!(super::same_fixed(&a, &[a_bar(), a_model(1.5)]), "only the clock moved");

        // …and a model whose *frame* moved is still not the fixed pass's
        // business: the fixed pass draws nothing at all for one.
        let mut faded = a_model(0.0);
        faded.alpha = 0.5;
        assert!(super::same_fixed(&a, &[a_bar(), faded]));

        // A model arriving, leaving or swapping places with something else is.
        assert!(!super::same_fixed(&a, &[a_bar()]));
        assert!(!super::same_fixed(&a, &[a_model(0.0), a_bar()]));
        assert!(!super::same_fixed(&a, &[a_bar(), a_bar()]));

        // …and everything that is not a model still compares by value, which
        // is the whole of what this test is protecting.
        let mut fuller = a_bar();
        if let Content::Bar(bar) = &mut fuller.content {
            bar.fraction = 0.75;
        }
        assert!(!super::same_fixed(&a, &[fuller, a_model(0.0)]));
    }

    /// The URI the material returns is the path the macro registers — the
    /// same assertion every embedded shader in this crate carries, from a file
    /// whose `file!()` parent is the embedder's.
    #[test]
    fn the_shader_uri_is_the_path_the_macro_registers() {
        assert_eq!(
            super::material::SHADER,
            crate::embedded_shader_uri!("interface.wgsl")
        );
    }
}
