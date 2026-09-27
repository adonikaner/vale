//! Building the app, in three steps a host can call separately.
//!
//! [`run`] is one host, and a binary that embeds this crate as a library is
//! another. Both need the same window settings, the same disabled plugin
//! families, the same six directory plugin groups and the same schedule
//! executors. Each of those is a measured decision with its reason written
//! beside it. Neither host needs the other's command line. The shared parts are
//! therefore three functions here, and [`run`] keeps only the flag handling.
//!
//! Keeping one copy matters because a copy in another binary goes stale the
//! next time one of these numbers is re-measured, and the notes beside them
//! record why each value was chosen.
//!
//! [`run`]: crate::run

use bevy::app::PluginGroupBuilder;
use bevy::prelude::*;
use bevy::render::settings::{Backends, InstanceFlags, RenderCreation, WgpuSettings};
use bevy::render::RenderPlugin;

use crate::{assets, glue, input, interface, lua, render, settings, sound, ui, world};
use crate::world::session;

/// What the window is called and how big it opens.
pub struct Host {
    pub title: String,
    /// `None` for bevy's own default.
    pub size: Option<(u32, u32)>,
}

impl Default for Host {
    fn default() -> Host {
        Host {
            title: "Vale".into(),
            size: None,
        }
    }
}

/// The default plugins, configured: the window, the backend, and the families
/// this client does not use.
pub fn plugins(host: &Host) -> PluginGroupBuilder {
    let plugins =
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: host.title.clone(),
                    // `--size`, or Bevy's own default — see [`WindowScript`].
                    resolution: match host.size {
                        Some((width, height)) => (width, height).into(),
                        None => default(),
                    },
                    // `VALE_FRAME_LATENCY=<1..3>` sets the swapchain's
                    // maximum frame latency, for measuring the acquire.
                    // `prepare_windows`' self time scales with the surface's
                    // pixels (0.29 ms at 1280x720, 1.59 at 3440x1440, traced,
                    // same scene) in every present mode. This hint is how many
                    // frames the presentation engine lets queue before the
                    // acquire blocks, and it is the only setting wgpu exposes
                    // over that cost. Unset is wgpu's default of 2.
                    desired_maximum_frame_latency: std::env::var("VALE_FRAME_LATENCY")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .and_then(std::num::NonZero::new),
                    ..default()
                }),
                ..default()
            })
            // The backend is chosen explicitly rather than defaulted; see
            // [`wgpu_settings`]. The frame is CPU-bound on the per-draw-call
            // encode by an order of magnitude, and two candidate causes of that
            // are both decided by this one setting.
            // `wow_mpq` logs every archive it opens at INFO: one line per
            // archive per chain, seventeen per borrow. The console write costs
            // more than the line is worth, and `ui::debug::check_archives`
            // already reports the archives mounting once, so `wow_mpq` is
            // filtered to WARN. `RUST_LOG` still overrides this filter, as it
            // overrides bevy's own default.
            .set(bevy::log::LogPlugin {
                filter: format!("{},wow_mpq=warn", bevy::log::DEFAULT_FILTER),
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(wgpu_settings())),
                ..default()
            })
            // Four default plugins this client never calls are disabled. The
            // frame is CPU-bound on fixed per-frame machinery (Tracy, 2026-08-25:
            // an empty scene is 8.1 ms with 1.1 ms of GPU), and every plugin in
            // the group adds per-frame systems and render-graph nodes whether
            // or not anything uses them. bevy_post_process and bevy_anti_alias
            // alone are ~40 system runs a frame plus the depth-of-field graph
            // nodes in `Core3d`. None of the four is referenced in this
            // workspace: no `AnimationPlayer` exists (skeletal animation is
            // this project's own `M2Skeleton::pose`), no `Scene` asset is
            // spawned, and no post-process effect is added to a camera. `Msaa`
            // (F5) lives in `bevy_render` and still works with these gone; the
            // camera's `Tonemapping::None` is a `bevy_core_pipeline` component
            // and is unaffected.
            //
            // `GltfPlugin` stays although nothing loads a glTF: `PbrPlugin`
            // unconditionally reads `GltfExtensionHandlers`
            // (`bevy_pbr-0.19.0/src/gltf.rs:26`), a resource only `GltfPlugin`
            // inserts, and the app panics on startup without it (tested). Its
            // per-frame cost is zero: it registers asset loaders and appears
            // nowhere in a frame trace.
            //
            // `GizmoPlugin` stays: the seven overlay visualisations in
            // `render::overlay` are drawn with `Gizmos`.
            // `bevy_ui`, `bevy_picking` and `bevy_text` are not disabled here;
            // they are disabled below, with the other rider families.
            .disable::<bevy::animation::AnimationPlugin>()
            .disable::<bevy::scene::ScenePlugin>()
            .disable::<bevy::post_process::PostProcessPlugin>()
            .disable::<bevy::anti_alias::AntiAliasPlugin>()
            .build();
    // The rider families are disabled unless `VALE_RIDERS=1` is set. The
    // variable is the A/B: one binary, interleaved runs, which is how every
    // subtraction here is priced. `SpritePickingPlugin` and the `text2d`
    // systems are added inside `SpritePlugin::build`, where a group `.disable`
    // cannot reach them (targeting them panics with "does not exist"), so the
    // whole of `SpritePlugin` is disabled and those two go with it. The order
    // of the list below follows from three panics in smoke runs: sprite
    // picking writes `PointerHits`, which `PickingPlugin` registers, so
    // `PickingPlugin` stays; `text2d` reads `Assets<Font>`, which `TextPlugin`
    // registers, and `TextPlugin` is disabled.
    let plugins = if std::env::var("VALE_RIDERS").is_err() {
        // The UI-related plugin families this client never uses. Its
        // interface is FrameXML painted through egui, its picking is
        // `interface::target`'s own ray, and nothing draws a `Node`, a
        // `Sprite` or a `Text`. bevy_egui's default features require the
        // first two; the workspace dependency turns those features off (see
        // Cargo.toml). The 2026-08 census: ~75 runs/frame across them, plus
        // gamepad (`bevy_gilrs`), which nothing here reads.
        plugins
            .disable::<bevy::ui::UiPlugin>()
            .disable::<bevy::ui_render::UiRenderPlugin>()
            .disable::<bevy::text::TextPlugin>()
            .disable::<bevy::gilrs::GilrsPlugin>()
            .disable::<bevy::sprite::SpritePlugin>()
            .disable::<bevy::picking::input::PointerInputPlugin>()
            .disable::<bevy::picking::InteractionPlugin>()
            .disable::<bevy::ui_widgets::popover::PopoverPlugin>()
            .disable::<bevy::ui_widgets::ButtonPlugin>()
            .disable::<bevy::ui_widgets::CheckboxPlugin>()
            .disable::<bevy::ui_widgets::ListBoxPlugin>()
            .disable::<bevy::ui_widgets::MenuPlugin>()
            .disable::<bevy::ui_widgets::RadioGroupPlugin>()
            .disable::<bevy::ui_widgets::ScrollAreaPlugin>()
            .disable::<bevy::ui_widgets::ScrollbarPlugin>()
            .disable::<bevy::ui_widgets::SliderPlugin>()
            .disable::<bevy::ui_widgets::EditableTextInputPlugin>()
            .disable::<bevy::input_focus::InputFocusPlugin>()
            .disable::<bevy::input_focus::InputDispatchPlugin>()
    } else {
        plugins
    };
    plugins
}

/// The archives, the install config, and the six directory plugin groups.
///
/// Call it after `add_plugins` on the group [`plugins`] returns, because the
/// groups here expect bevy's own to be in the app already.
pub fn core(app: &mut App, gamedata_dir: String, config: vale_config::Config) {
    // Two resources that the plugins' systems require and a command-line flag
    // sets. [`crate::run`] overwrites them from its flags. They are inserted
    // here at their unset values because a host without those flags would
    // otherwise panic on the first frame with
    // "Parameter `Res<HoverProbe>` failed validation: Resource does not exist".
    // A resource a plugin needs belongs with the plugin, not with the command
    // line that tunes it.
    app.insert_resource(crate::HoverProbe(None))
        .insert_resource(lua::host::StartupScript::new(None));
    app
        .insert_resource(assets::GameAssets::new(gamedata_dir).with_root(config.root.clone()))
        .insert_resource(session::ClientConfig(config))
        // One plugin per directory. Each directory's `mod.rs` lists what is in
        // it and states the orderings that matter, so adding a pass does not
        // edit this list. It replaced a fourteen-entry list with the ordering
        // notes inline, which every new pass had to edit.
        .add_plugins((
            world::WorldPlugins,
            render::RenderPlugins,
            // Before the state groups below, which call into the interpreter
            // on the first frame a key is pressed and would otherwise ask for a
            // non-send resource inserted later in the same `build`.
            lua::LuaPlugins,
            input::InputPlugins,
            // Reads `ClientConfig`, inserted above, in `build`.
            settings::SettingsPlugins,
            interface::InterfacePlugins,
            glue::GluePlugins,
            world::StatePlugins,
            // After the state groups, whose area tracking and action state it
            // listens to; see `sound/mod.rs` for the ordering it states.
            sound::SoundPlugins,
            ui::UiPlugins,
        ));
}

/// The schedule executors, set from seven separate measurements.
///
/// Each one carries the interleaved A/B it came from and the environment
/// variable that re-runs it. They are here rather than in a host because the
/// numbers depend on this client's schedule population, not on which host
/// built the app. They are kept together so they can be re-measured as a set:
/// the camera schedule went from a 0.2 ms loss to a 0.45 ms win when its
/// contents changed.
pub fn executors(app: &mut App) {
    // The five fixed-timestep schedules run on the single-threaded executor.
    // Nothing in this client uses `FixedUpdate`; the world's own cadences are
    // the interface clock and the session thread. With the multithreaded
    // executor, bevy started worker tasks for five near-empty schedules every
    // fixed step: ~0.25 ms/frame of task-pool overhead (Tracy,
    // `RunFixedMainLoop`). The single-threaded executor runs the same systems
    // without that start-up cost; anything a plugin adds there still runs,
    // sequentially.
    {
        use bevy::ecs::schedule::SingleThreadedExecutor;
        app.edit_schedule(bevy::app::FixedFirst, |s| {
            s.set_executor(SingleThreadedExecutor::new());
        });
        app.edit_schedule(bevy::app::FixedPreUpdate, |s| {
            s.set_executor(SingleThreadedExecutor::new());
        });
        app.edit_schedule(bevy::app::FixedUpdate, |s| {
            s.set_executor(SingleThreadedExecutor::new());
        });
        app.edit_schedule(bevy::app::FixedPostUpdate, |s| {
            s.set_executor(SingleThreadedExecutor::new());
        });
        app.edit_schedule(bevy::app::FixedLast, |s| {
            s.set_executor(SingleThreadedExecutor::new());
        });
        // The main schedules keep the multithreaded executor. The A/B (same
        // binary, an env switch, alternating runs) found sequential
        // Update/PostUpdate no different on the mean, with wider variance:
        // the heavy chains are already serialized by data conflicts, and the
        // remaining overlap roughly pays for the task-pool overhead. Re-run
        // the experiment on other hardware before relying on either result.

        // The render sub-app's `Render` schedule showed no difference either.
        // Measured 2026-08-25, interleaved pairs on one binary:
        // single-threaded 8.36/9.06 against multithreaded 8.78/8.86, pairs
        // overlapping. The Tracy self-times suggested a gain (the render app's
        // ~490 prepare runs are ~3.8 ms of work on ~3.6 ms of wall, so its
        // parallelism buys almost nothing), but serialising them costs what
        // the ~2.8 µs/run executor bookkeeping saves. Bevy's default is kept;
        // `VALE_RENDER_ST=1` re-runs the experiment on other hardware
        // without a rebuild.
        if std::env::var("VALE_RENDER_ST").is_ok() {
            if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
                render_app.world_mut().resource_scope(
                    |_, mut schedules: Mut<bevy::ecs::schedule::Schedules>| {
                        if let Some(s) = schedules.get_mut(bevy::render::Render) {
                            s.set_executor(SingleThreadedExecutor::new());
                        }
                    },
                );
            }
        }

        // The two camera schedules run single-threaded, which saves a
        // measured 0.45 ms. The tracing plan records a consistent 0.2 ms
        // regression, but that was measured with the post-process and
        // anti-alias nodes still in the schedule. With those plugins disabled,
        // `Core3d` is a handful of chained node systems the multithreaded
        // executor cannot overlap, so its only effect is handing each link to
        // a worker and parking between links. Interleaved on one binary,
        // 2026-08-25: single-threaded 8.37/8.32 against multithreaded
        // 8.61/8.98, both pairs separated. `VALE_CAMERA_MT=1` restores
        // bevy's default for re-measuring.
        if std::env::var("VALE_CAMERA_MT").is_err() {
            if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
                render_app.world_mut().resource_scope(
                    |_, mut schedules: Mut<bevy::ecs::schedule::Schedules>| {
                        use bevy::core_pipeline::schedule::{Core2d, Core3d};
                        if let Some(s) = schedules.get_mut(Core3d) {
                            s.set_executor(SingleThreadedExecutor::new());
                        }
                        if let Some(s) = schedules.get_mut(Core2d) {
                            s.set_executor(SingleThreadedExecutor::new());
                        }
                    },
                );
            }
        }

        // `VALE_MAIN_ST=1` runs the same experiment on the main app's
        // three big schedules. It matters when the main app is on the critical
        // path (its three phases sum past the render thread), as the Goldshire
        // round measured: then the ~2.8 µs/run executor bookkeeping over
        // several hundred main-world systems is no longer hidden in the slack.
        // The camera experiment above went from a 0.2 ms loss to a 0.45 ms win
        // when the schedule's contents changed, so this is an environment
        // switch rather than a fixed setting. Run it interleaved, in the scene
        // in question, and re-measure when the schedule population changes.
        if std::env::var("VALE_MAIN_ST").is_ok() {
            app.world_mut().resource_scope(
                |_, mut schedules: Mut<bevy::ecs::schedule::Schedules>| {
                    use bevy::app::{PostUpdate, PreUpdate, Update};
                    if let Some(s) = schedules.get_mut(PreUpdate) {
                        s.set_executor(SingleThreadedExecutor::new());
                    }
                    if let Some(s) = schedules.get_mut(Update) {
                        s.set_executor(SingleThreadedExecutor::new());
                    }
                    if let Some(s) = schedules.get_mut(PostUpdate) {
                        s.set_executor(SingleThreadedExecutor::new());
                    }
                },
            );
        }

        // `ExtractSchedule` runs single-threaded. It is the one schedule whose
        // executor bookkeeping is always on the main thread's critical path:
        // extract runs with the main world locked, and the main app cannot
        // start its next frame until it finishes, so every microsecond of
        // worker hand-off there is a microsecond of `PhaseOutsideMain`. Tracy
        // (wall-a, 2026-09-02) put the schedule's own self time at ~0.7 ms for
        // extract systems whose bodies are a few microseconds each.
        // Interleaved, three pairs, unsynced, Elwynn 2026-09-02:
        // single-threaded 4.60/5.96/6.13 median against multithreaded
        // 4.76/5.97/6.52. All three pairs point the same way, ~0.2 ms on the
        // mean, p95 mixed. The gain is small; it is kept because it also
        // removes worker wake-up from the one place the main thread waits.
        // `VALE_EXTRACT_MT=1` restores bevy's default for re-measuring. As
        // with the camera schedules, the result may change when the schedule
        // population does.
        if std::env::var("VALE_EXTRACT_MT").is_err() {
            if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
                render_app.world_mut().resource_scope(
                    |_, mut schedules: Mut<bevy::ecs::schedule::Schedules>| {
                        if let Some(s) = schedules.get_mut(bevy::render::ExtractSchedule) {
                            s.set_executor(SingleThreadedExecutor::new());
                        }
                    },
                );
            }
        }
    }
}

/// The GPU backend this client asks for, and the
/// validation flag that is the reason for asking.
///
/// Before `app::wgpu_settings`, nothing in this client named a backend, and the
/// default was not the one this project assumed. `wgpu_core::Instance::new`
/// registers its backends in a fixed order (Vulkan, Metal, Dx12, Gles, with the
/// comment "The ordering in this list implies prioritization and needs to be
/// preserved"), and `request_adapter` sorts the collected adapters by device
/// type with a stable sort, so among discrete GPUs the Vulkan adapter stays
/// first. Bevy's `Backends::all()` therefore selects Vulkan on Windows, and
/// always has: every frame number this project had recorded was taken on
/// Vulkan, while the README said D3D12.
///
/// `all()` also leaves DX12 in the requested list, and one setting keys off
/// that list rather than off the adapter: `InstanceFlags::VALIDATION_INDIRECT_CALL`
/// is kept whenever DX12 is in the backends and removed otherwise ("wgpu
/// executes additional necessary logic during validation passes for the DX12
/// backend, so the flag should stay for DX12. Removing this flag improves
/// performance", `bevy_render::settings`, under `not(debug_assertions)`).
/// `InstanceFlags::from_build_config` returns that flag on its own for a build
/// with `debug-assertions` off, which every dependency here is
/// (`[profile.dev.package."*"]`).
///
/// So `all()` meant Vulkan with DX12's rule applied: every indirect call was
/// validated, and under multidraw every binned draw is an indirect call. Naming
/// a single backend settles the flag either way.
///
/// The client therefore asks for Vulkan: the backend that was already selected,
/// without the DX12 rule. The adapter is the same one `all()` selected, so the
/// change differs from every earlier frame number in this repository in one
/// variable.
///
/// DX12 is not usable on the development machine: requesting it outright
/// errored. The cause was not diagnosed and is not recorded as a limitation of
/// the backend. DX12 is still reachable as `WGPU_BACKEND=dx12`, which also
/// restores the flag, since bevy calls that validation necessary logic there.
///
/// | Run | Backend | Indirect validation |
/// |---|---|---|
/// | every frame number in this repo before this change | Vulkan | on |
/// | current | Vulkan | off |
/// | `WGPU_VALIDATION_INDIRECT_CALL=1` | Vulkan | on |
///
/// The first two rows differ only in the flag, which is the measurement; the
/// third is the same A/B without a rebuild. Bevy logs the adapter it chose at
/// INFO (`AdapterInfo { .., backend: Vulkan }`). Read it rather than assume it:
/// an assumption put the wrong backend in the first draft of this comment.
///
/// This is a hypothesis with a named experiment, not a measurement. What is
/// measured is the ~30 µs per draw-call encode, and that it is spent inside wgpu
/// rather than in this workspace; a release build changes neither. Whether the
/// indirect validation is where that time goes is what the A/B answers.
fn wgpu_settings() -> WgpuSettings {
    let mut settings = WgpuSettings::default();
    // `WgpuSettings::default` has already read `WGPU_BACKEND`, so an explicit
    // choice is left alone: an A/B must differ in one setting only. macOS is
    // excluded because it has no Vulkan without MoltenVK, and its default,
    // Metal, is the correct backend there.
    if Backends::from_env().is_none() && !cfg!(target_os = "macos") {
        settings.backends = Some(Backends::VULKAN);
    }
    // Bevy's own rule, re-applied against the backend list this function
    // requests. It is conditional because the flag is needed on DX12: an
    // unvalidated indirect call there is the reason bevy keeps it. Requesting
    // DX12 therefore restores bevy's default behaviour.
    if !settings.backends.is_some_and(|b| b.contains(Backends::DX12)) {
        settings.instance_flags.remove(InstanceFlags::VALIDATION_INDIRECT_CALL);
        // `WGPU_VALIDATION_INDIRECT_CALL=1` still overrides this; it is the A/B.
        settings.instance_flags = settings.instance_flags.with_env();
    }
    settings
}
