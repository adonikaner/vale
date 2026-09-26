//! Draw calls, counted in the render world after the phases are formed.
//!
//! The HUD also reports how much geometry is in the world (batches), how much of
//! it the camera can see (visible meshes), and how many distinct materials are
//! behind it ([`crate::render::models::MaterialPool`]). None of those is a draw
//! call. Interning materials collapses batch sets, and multi-draw indirect
//! turns a batch set into one call, so the draw-call count sits between the
//! visible-mesh count and the material count.
//!
//! The CLI measured the effect of material interning as "3,042 unmergeable draw
//! calls became 186", based on a reading of Bevy's source. This module checks
//! that on a running frame: it counts the bins and batch sets in the render
//! world after they are formed and hands the number back to the HUD.
//!
//! ## Why the count depends on the machine's preprocessing mode
//!
//! A binned phase holds its work in three places, and which one it uses depends
//! on the machine, not on this client:
//!
//! * `multidrawable_meshes`: one batch set per entry, drawn with a single
//!   multi-draw indirect call however many different meshes are inside it.
//!   Material interning reduces this case.
//! * `batchable_meshes`: one entry per (batch set, mesh), one instanced draw
//!   each. Instances of one mesh still merge; two different meshes never do.
//! * `unbatchable_meshes`: one draw per entity.
//!
//! `BinnedRenderPhaseType::mesh` picks the first only when
//! `GpuPreprocessingSupport::max_supported_mode` is `Culling`, which needs
//! compute, `INDIRECT_FIRST_INSTANCE`, `IMMEDIATES`, 12 storage textures and 10
//! storage buffers per stage, and a backend that is not GL. On a machine that
//! falls short, every batch of `stormwind.wmo` is its own mesh and therefore its
//! own bin: 3,042 draw calls, with `MaterialPool` interning having no effect.
//! The material count, the batch count and the visible-mesh count do not show
//! this, so the mode is reported beside the draw-call count.

use bevy::core_pipeline::core_3d::{AlphaMask3d, Opaque3d, Transparent3d};
use bevy::prelude::*;
use bevy::render::batching::gpu_preprocessing::{GpuPreprocessingMode, GpuPreprocessingSupport};
use bevy::render::render_phase::{BinnedPhaseItem, ViewBinnedRenderPhases, ViewSortedRenderPhases};
use bevy::render::{Render, RenderApp, RenderSystems};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// What the last frame cost the GPU to be told about.
///
/// The counters are shared between the two worlds, not copied. The system that
/// fills them runs in the render world and the HUD reads them from the main
/// one. Bevy's worlds exchange data by extraction, which runs main -> render,
/// the opposite direction to this measurement. The resource is therefore an
/// `Arc` of atomics, inserted into both worlds as clones of one allocation, so
/// a store on the render side is visible to a load on the main side with no
/// schedule between them. [`the_two_worlds_share_one_set_of_counters`] tests
/// this: a `#[derive(Clone)]` that deep-copied would leave the HUD reporting
/// zero draw calls, which is indistinguishable from a renderer that draws
/// nothing.
#[derive(Resource, Clone, Default)]
pub struct DrawCalls(Arc<Counters>);

#[derive(Default)]
pub struct Counters {
    opaque: AtomicUsize,
    alpha_mask: AtomicUsize,
    /// Phase *items*, not calls — see [`DrawCalls::transparent`].
    transparent: AtomicUsize,
    /// 0 none, 1 preprocessing only, 2 culling. See [`DrawCalls::multidraw`].
    mode: AtomicUsize,
}

impl DrawCalls {
    /// The opaque phase: the terrain, the buildings and everything blend mode 0.
    pub fn opaque(&self) -> usize {
        self.0.opaque.load(Ordering::Relaxed)
    }

    /// The alpha-masked phase: blend mode 1, which is most of the game's foliage
    /// and every window lattice and railing.
    pub fn alpha_mask(&self) -> usize {
        self.0.alpha_mask.load(Ordering::Relaxed)
    }

    /// Translucent phase items, which is an upper bound on its draw calls.
    ///
    /// A sorted phase merges neighbouring items that share a mesh and a material
    /// at `Prepare` time, after this is read. It is counted anyway because
    /// `vale wmos` reports zero translucent batches in Stormwind, so a value
    /// well above zero here is a finding in itself.
    pub fn transparent(&self) -> usize {
        self.0.transparent.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> usize {
        self.opaque() + self.alpha_mask() + self.transparent()
    }

    /// Whether this machine collapses different meshes into one call at all.
    ///
    /// When this is false, every other number here means something different,
    /// and material interning has no effect: a bin, not a batch set, is a draw
    /// call, and a bin is one mesh. See the module note.
    pub fn multidraw(&self) -> bool {
        self.0.mode.load(Ordering::Relaxed) == 2
    }

    /// The preprocessing mode, for the HUD.
    pub fn mode(&self) -> &'static str {
        match self.0.mode.load(Ordering::Relaxed) {
            2 => "multidraw",
            1 => "gpu preprocessing, no multidraw",
            _ => "cpu preprocessing",
        }
    }
}

pub struct DrawCallPlugin;

impl Plugin for DrawCallPlugin {
    fn build(&self, app: &mut App) {
        let counts = DrawCalls::default();
        app.insert_resource(counts.clone());
        // The render app exists by the time a plugin's `build` runs (RenderPlugin
        // is in `DefaultPlugins` and this is added after it), but a headless test
        // app has no render world at all. The counters are still readable there
        // and stay zero.
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.insert_resource(counts).add_systems(
                Render,
                // After the bins are filled and swept, before anything reads them
                // to build buffers. Later would also work for the binned phases,
                // which are retained across the frame; earlier would count a bin
                // that this frame's visibility has already emptied.
                count_draw_calls
                    .after(RenderSystems::PhaseSort)
                    .before(RenderSystems::Prepare),
            );
            // `VALE_DUMP_BINS=1` logs a breakdown of the opaque phase by
            // pipeline and slab, three hundred frames in (past the load, before
            // the shutter). The count above says how many multidraw calls the
            // frame makes; this says what splits them into that many batch
            // sets, which is where a batch-set reduction starts. It is an
            // instrument, so it is gated: a run without the variable pays one
            // `bool` check.
            if std::env::var("VALE_DUMP_BINS").is_ok() {
                render.add_systems(
                    Render,
                    dump_opaque_bins
                        .after(RenderSystems::PhaseSort)
                        .before(RenderSystems::Prepare),
                );
            }
        }
    }
}

/// Print one aggregation of the opaque phase: batch sets by pipeline, with the
/// distinct material-slab and mesh-slab counts that split them. See the
/// registration above; `VALE_DUMP_BINS=1` only.
fn dump_opaque_bins(
    mut frames: Local<u32>,
    opaque: Option<Res<ViewBinnedRenderPhases<Opaque3d>>>,
    pipelines: Option<Res<bevy::render::render_resource::PipelineCache>>,
    allocators: Option<Res<bevy::pbr::MaterialBindGroupAllocators>>,
    ) {
    *frames += 1;
    // Twice: mid-load and settled, because the question is whether the slab
    // spread is a property of the load burst or of the steady state.
    if *frames != 300 && *frames != 1500 {
        return;
    }
    if let Some(allocators) = &allocators {
        for (ty, allocator) in allocators.iter() {
            info!(
                "bins: allocator {ty:?} — {} slabs, {} live allocations, {} bytes",
                allocator.slab_count(),
                allocator.allocations(),
                allocator.slabs_size()
            );
        }
    }
    let Some(opaque) = opaque else { return };
    use std::collections::BTreeMap;
    // pipeline id -> (batch sets, material slabs, vertex slabs)
    let mut by_pipeline: BTreeMap<
        u32,
        (usize, std::collections::BTreeSet<Option<u32>>, std::collections::BTreeSet<u32>),
    > = BTreeMap::new();
    let mut batchable = 0;
    let mut unbatchable = 0;
    for phase in opaque.values() {
        for key in phase.multidrawable_meshes.keys() {
            let entry = by_pipeline.entry(key.pipeline.id() as u32).or_default();
            entry.0 += 1;
            entry.1.insert(key.material_bind_group_index);
            entry.2.insert(key.slabs.vertex_slab_id.id.get());
        }
        batchable += phase.batchable_meshes.len();
        unbatchable += phase.unbatchable_meshes.len();
    }
    info!("bins: opaque — {} pipelines, batchable {batchable}, unbatchable {unbatchable}", by_pipeline.len());
    for (pipeline, (sets, materials, slabs)) in &by_pipeline {
        let label = pipelines
            .as_ref()
            .and_then(|cache| {
                cache
                    .get_render_pipeline_descriptor(
                        bevy::render::render_resource::CachedRenderPipelineId::new(*pipeline as usize),
                    )
                    .label
                    .as_deref()
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "?".into());
        info!(
            "bins:   pipeline {pipeline} ({label}) — {sets} batch sets over {} material slab(s) x {} mesh slab(s)",
            materials.len(),
            slabs.len()
        );
    }
}

/// One binned phase's draw calls, across every view that has one.
///
/// Summed over views rather than reported per camera because the question this
/// answers is what the *frame* costs, and the shadow phase — which would be the
/// other view — is a different phase type and is not drawn here at all
/// (the sun's `shadow_maps_enabled` follows `RenderTuning::sun_shadows`, F10,
/// off by default; see `render::sky`).
fn binned_draws<BPI: BinnedPhaseItem>(phases: &ViewBinnedRenderPhases<BPI>) -> usize {
    phases
        .values()
        .map(|phase| {
            // One multi-draw indirect call per batch set, whatever is inside it;
            // one instanced draw per bin that could not be multidrawn; and one
            // draw each for the entities that could not be batched at all.
            phase.multidrawable_meshes.len()
                + phase.batchable_meshes.len()
                + phase
                    .unbatchable_meshes
                    .values()
                    .map(|bin| bin.entities.len())
                    .sum::<usize>()
                + phase
                    .non_mesh_items
                    .values()
                    .map(|bin| bin.entities.len())
                    .sum::<usize>()
        })
        .sum()
}

/// Read this frame's phases and publish the counts.
///
/// Every phase resource is optional: they are inserted by the core 3D plugin,
/// and a run configured without one reports zero rather than panicking a render
/// world that is otherwise working.
#[allow(clippy::too_many_arguments)]
fn count_draw_calls(
    counts: Res<DrawCalls>,
    support: Option<Res<GpuPreprocessingSupport>>,
    opaque: Option<Res<ViewBinnedRenderPhases<Opaque3d>>>,
    alpha_mask: Option<Res<ViewBinnedRenderPhases<AlphaMask3d>>>,
    transparent: Option<Res<ViewSortedRenderPhases<Transparent3d>>>,
) {
    let store = |slot: &AtomicUsize, value: usize| slot.store(value, Ordering::Relaxed);

    store(
        &counts.0.opaque,
        opaque.map(|p| binned_draws(&p)).unwrap_or(0),
    );
    store(
        &counts.0.alpha_mask,
        alpha_mask.map(|p| binned_draws(&p)).unwrap_or(0),
    );
    store(
        &counts.0.transparent,
        transparent
            .map(|p| p.values().map(|phase| phase.items.len()).sum())
            .unwrap_or(0),
    );
    store(
        &counts.0.mode,
        match support.map(|s| s.max_supported_mode) {
            Some(GpuPreprocessingMode::Culling) => 2,
            Some(GpuPreprocessingMode::PreprocessingOnly) => 1,
            _ => 0,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The counters are written in the render world and read in the main one,
    /// which works only because `Clone` shares the allocation rather than
    /// copying it. A `#[derive(Clone)]` over plain `AtomicUsize` fields would
    /// compile, clone by value, and leave the HUD reporting zero draw calls for
    /// a renderer that is drawing the whole world, with nothing else indicating
    /// a fault.
    #[test]
    fn the_two_worlds_share_one_set_of_counters() {
        let main_world = DrawCalls::default();
        let render_world = main_world.clone();

        render_world.0.opaque.store(186, Ordering::Relaxed);
        render_world.0.alpha_mask.store(12, Ordering::Relaxed);
        render_world.0.transparent.store(3, Ordering::Relaxed);
        render_world.0.mode.store(2, Ordering::Relaxed);

        assert_eq!(main_world.opaque(), 186);
        assert_eq!(main_world.total(), 186 + 12 + 3);
        assert!(main_world.multidraw(), "the mode crosses too, not just the counts");
        assert_eq!(main_world.mode(), "multidraw");
    }

    /// The three modes are named rather than numbered on the HUD, and the
    /// distinction that matters is `multidraw` against everything else: in the
    /// other two a bin is a draw call, so the same scene costs an order of
    /// magnitude more and no other number on the HUD moves.
    #[test]
    fn only_culling_mode_is_multidraw() {
        let counts = DrawCalls::default();
        for (value, name, multidraw) in [
            (0usize, "cpu preprocessing", false),
            (1, "gpu preprocessing, no multidraw", false),
            (2, "multidraw", true),
        ] {
            counts.0.mode.store(value, Ordering::Relaxed);
            assert_eq!(counts.mode(), name);
            assert_eq!(counts.multidraw(), multidraw);
        }
    }
}
